//! `digstore deploy-key export` — export the active store's publisher deploy key.
//!
//! The deploy key is the 32-byte seed behind the store's BLS PUBLISHER key
//! (`signing_key.bin`), generated at `init`. CI needs it to sign the §21 head
//! push so DIGHUb (which pinned this store's publisher pubkey at first push)
//! accepts a new capsule. It is the ONE piece of owner state that cannot be
//! reconstructed from the wallet seed, so it is exported once and stored as a CI
//! secret. It carries NO on-chain spend authority — only head-push authority —
//! but it is still a credential and must be handled like one.

use crate::cli::{DeployKeyAction, DeployKeyArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: DeployKeyArgs) -> Result<(), CliError> {
    match args.action {
        DeployKeyAction::Export { out } => export(ctx, ui, out),
    }
}

fn export(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    out: Option<std::path::PathBuf>,
) -> Result<(), CliError> {
    let seed = store_ops::read_signing_seed(ctx)?;
    let hex = hex::encode(seed);

    if let Some(path) = out {
        // Owner-only on disk (it is a credential), like the seed/session files.
        store_ops::write_secret_file(&path, hex.as_bytes())
            .map_err(|e| CliError::Other(e.into()))?;
        if ui.json() {
            ui.emit_json(&serde_json::json!({ "written": path.display().to_string() }));
        } else {
            ui.success(format!("deploy key written to {}", path.display()));
            ui.line("Store it as a CI secret (e.g. DIGSTORE_DEPLOY_KEY). It authorizes publishing");
            ui.line(format!(
                "capsules to {}; it has NO spend authority, but treat it like a credential.",
                crate::branding::DIGHUB
            ));
        }
        return Ok(());
    }

    if ui.json() {
        ui.emit_json(&serde_json::json!({ "deploy_key": hex }));
    } else {
        // Bare line so `KEY=$(digstore deploy-key export)` captures exactly the key.
        ui.line(hex);
    }
    Ok(())
}
