//! `digstore deploy` — CI auto-deploy to an EXISTING store (a new capsule).
//!
//! This is the "deploy from GitHub Actions" entry point: on a fresh checkout
//! with no local `.dig`, advance the store's on-chain root and publish the new
//! capsule to DIGHub, like Vercel's Git integration. It NEVER mints (`init`
//! creates a store and spends 100 DIG; `deploy` only ADVANCES an existing one).
//!
//! Flow:
//!   1. Resolve config from `dig.toml` (overridden by flags/env).
//!   2. (optional) run the build command.
//!   3. Reconstruct the store's local `.dig` state with the supplied publisher
//!      deploy key + the current on-chain root (`store_ops::adopt_existing_store`).
//!   4. Stage the output dir, then run the SAME `commit -m --push` path the
//!      interactive CLI uses (on-chain root update + DIGHub push), non-interactively.
//!   5. Print the new capsule (`storeId:rootHash`) + dig:// URN + hub URL.
//!
//! Secrets it consumes (the Action injects these from repo secrets):
//!   - the funded wallet seed (`~/.dig`/`DIGSTORE_HOME` + `DIGSTORE_PASSPHRASE`)
//!     — signs the on-chain update and pays 100 DIG + fee per deploy;
//!   - the publisher deploy key (`--deploy-key` / `DIGSTORE_DEPLOY_KEY`) — signs
//!     the §21 head push so DIGHub accepts the capsule;
//!   - for a PRIVATE store, the secret salt (`--salt` / `DIGSTORE_STORE_SALT`).

use std::path::PathBuf;

use digstore_core::{Bytes32, SecretSalt, Visibility};

use crate::cli::{CommitArgs, DeployArgs};
use crate::context::CliContext;
use crate::dig_toml::DigToml;
use crate::error::CliError;
use crate::ops::{remote_ops, store_ops};
use crate::runtime::block_on;

/// Resolved deploy configuration (file < flag/env precedence already applied).
struct DeployConfig {
    store_id: Bytes32,
    output_dir: String,
    build_command: Option<String>,
    message: Option<String>,
    wait_timeout: u64,
    remote: Option<String>,
    #[allow(dead_code)]
    network: String,
}

/// `DIGSTORE_DEPLOY_KEY` / `--deploy-key` → the 32-byte publisher seed.
fn resolve_deploy_key(args: &DeployArgs) -> Result<[u8; 32], CliError> {
    let hex_str = args
        .deploy_key
        .clone()
        .or_else(|| std::env::var("DIGSTORE_DEPLOY_KEY").ok())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            CliError::InvalidArgument(
                "no deploy key: pass --deploy-key or set DIGSTORE_DEPLOY_KEY (run `digstore \
                 deploy-key export` on the machine that created the store)"
                    .into(),
            )
        })?;
    let bytes = hex::decode(hex_str.trim())
        .map_err(|_| CliError::InvalidArgument("deploy key must be 64-hex".into()))?;
    bytes
        .try_into()
        .map_err(|_| CliError::InvalidArgument("deploy key must be a 32-byte (64-hex) seed".into()))
}

/// Resolve the store visibility: PRIVATE iff a salt is provided (`--salt` /
/// `DIGSTORE_STORE_SALT`), else PUBLIC. A private store MUST be adopted with its
/// original salt or retrieval keys diverge.
fn resolve_visibility(args: &DeployArgs) -> Result<Visibility, CliError> {
    let salt_hex = args
        .salt
        .clone()
        .or_else(|| std::env::var("DIGSTORE_STORE_SALT").ok())
        .filter(|s| !s.trim().is_empty());
    match salt_hex {
        None => Ok(Visibility::Public),
        Some(h) => {
            let bytes = hex::decode(h.trim())
                .map_err(|_| CliError::InvalidArgument("salt must be 64-hex".into()))?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| CliError::InvalidArgument("salt must be a 32-byte (64-hex)".into()))?;
            Ok(Visibility::Private(SecretSalt(arr)))
        }
    }
}

fn resolve_config(ctx: &CliContext, args: &DeployArgs) -> Result<DeployConfig, CliError> {
    let file = DigToml::read(&ctx.op_dir)?;

    let store_id_hex = args.store_id.clone().or(file.store_id).ok_or_else(|| {
        CliError::InvalidArgument(
            "no store id: set `store-id` in dig.toml or pass --store-id".into(),
        )
    })?;
    let store_id = Bytes32::from_hex(store_id_hex.trim())
        .map_err(|_| CliError::InvalidArgument("store id must be 64-hex".into()))?;

    let output_dir = args
        .output_dir
        .clone()
        .or(file.output_dir)
        .unwrap_or_else(|| "dist".to_string());

    let build_command = args.build_command.clone().or(file.build_command);
    let message = args.message.clone().or(file.message);
    let wait_timeout = args.wait_timeout.or(file.wait_timeout).unwrap_or(300);
    let network = args
        .network
        .clone()
        .or(file.network)
        .unwrap_or_else(|| "mainnet".to_string());
    let remote = args.remote.clone().or(file.remote);

    Ok(DeployConfig {
        store_id,
        output_dir,
        build_command,
        message,
        wait_timeout,
        remote,
        network,
    })
}

/// Run the user's build command (if any) from the operating directory.
fn run_build(ui: &crate::ui::Ui, op_dir: &std::path::Path, cmd: &str) -> Result<(), CliError> {
    if !ui.json() {
        ui.line(format!("▶ build: {cmd}"));
    }
    // Use the platform shell so multi-step commands ("npm ci && npm run build") work.
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    let status = command
        .current_dir(op_dir)
        .status()
        .map_err(|e| CliError::Other(anyhow::anyhow!("spawn build command: {e}")))?;
    if !status.success() {
        return Err(CliError::Other(anyhow::anyhow!(
            "build command failed with status {status}"
        )));
    }
    Ok(())
}

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: DeployArgs) -> Result<(), CliError> {
    let cfg = resolve_config(ctx, &args)?;
    let deploy_seed = resolve_deploy_key(&args)?;
    let visibility = resolve_visibility(&args)?;

    // 1. Build (optional).
    if let Some(cmd) = &cfg.build_command {
        run_build(ui, &ctx.op_dir, cmd)?;
    }

    // The output dir to publish (relative to the operating directory).
    let output_dir = if PathBuf::from(&cfg.output_dir).is_absolute() {
        PathBuf::from(&cfg.output_dir)
    } else {
        ctx.op_dir.join(&cfg.output_dir)
    };
    if !output_dir.is_dir() {
        return Err(CliError::InvalidArgument(format!(
            "output directory '{}' does not exist (build it first, or set output-dir)",
            output_dir.display()
        )));
    }

    // 2. Resolve the store's CURRENT on-chain root — the head the new capsule
    //    fast-forwards from (honors DIGSTORE_ANCHOR_MOCK for offline tests).
    let sp = ui.spinner("Reading the store's on-chain root…");
    let tip = block_on(remote_ops::onchain_tip_root(&cfg.store_id))??;
    sp.finish();

    // 3. Reconstruct the store's local `.dig` state in this (fresh) workspace,
    //    so `add`/`commit` target the EXISTING store with the right publisher key.
    //    `deploy` runs as a store-scoped command, so `ctx.dig_dir` is the per-store
    //    dir; if it is already adopted/initialized this is idempotent-friendly:
    //    skip reconstruction and just advance it.
    if !ctx.config_path().exists() {
        store_ops::adopt_existing_store(ctx, cfg.store_id, &deploy_seed, visibility, tip, None)?;
    }

    // Point `origin` at the configured remote (the public DIGHub by default). The
    // store dir only exists after adoption, so set this here. `commit --push`
    // publishes to `origin`.
    if let Some(remote) = &cfg.remote {
        crate::config::add_remote(ctx, "origin", remote)?;
    }

    // 4. Stage the output dir. We point `add` at the output dir as its operating
    //    directory so resource keys are relative to it (the site root), then
    //    commit + push using the canonical commit path.
    let stage_ctx = CliContext {
        op_dir: output_dir.clone(),
        ..ctx.clone()
    };
    let staged = store_ops::add_files(&stage_ctx, &[], true, false, None)?;
    if staged.staged.is_empty() {
        return Err(CliError::InvalidArgument(format!(
            "nothing to deploy: '{}' is empty or unchanged",
            output_dir.display()
        )));
    }
    if !ui.json() {
        ui.line(format!(
            "✓ staged {} file(s) from {}",
            staged.staged.len(),
            output_dir.display()
        ));
    }

    // 5. Commit (on-chain root update) + push to DIGHub, non-interactively.
    //    This reuses the exact `commit -m --push` path the interactive CLI uses;
    //    `--push` publishes to the default `origin` remote without prompting.
    crate::commands::commit::run(
        ctx,
        ui,
        CommitArgs {
            message: cfg.message.clone(),
            wait_timeout: cfg.wait_timeout,
            resubmit: false,
            push: true,
            no_push: false,
            dry_run: false,
        },
    )
}
