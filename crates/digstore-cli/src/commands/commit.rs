use chia_protocol::Bytes32;

use crate::cli::CommitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_state::{AnchorState, AnchorStatus};
use crate::ops::{anchor_backend, anchor_ux, store_ops};
use crate::runtime::block_on;
use digstore_chain::anchor::ConfirmState;
use digstore_chain::dig::{self, format_dig, format_xch};

/// `digstore commit` pushes the staged generation's new root to the store's
/// on-chain singleton via a Chia `update` and BLOCKS until confirmed BEFORE
/// finalizing the local generation. This is a HARD GATE: local history (roots.log,
/// generations, staging) never advances past the chain. The staged root is
/// computed first (fail-fast on empty staging, before any wallet/anchor work);
/// only after the update confirms is the generation persisted. A confirmation
/// timeout (or any confirm error) leaves staging + history untouched and a
/// resumable Pending `anchor.toml`, so a re-run reuses the in-flight update.
pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: CommitArgs) -> Result<(), CliError> {
    // 1. Compute the next root from staging. Persists NOTHING. Fails fast if
    //    nothing is staged — before any wallet/anchor work.
    let prepared = store_ops::stage_to_root(ctx)?;
    let new_root_hex = prepared.root.to_hex();

    // --dry-run: report the resulting version (root) + the exact DIG/XCH cost and
    // STOP — no seed unlock, no wallet scan, no on-chain update, no finalize.
    // Nothing is spent and nothing is published; this is a safe cost preview.
    if args.dry_run {
        return dry_run(ctx, ui, &prepared.root);
    }

    // 2. Anchor gate: unlock seed (NoSeed → exit 9), build the (mock or real)
    //    backend, warn if mocked, surface the fee.
    let (keys, mnemonic, anchor, mocked, fee) = anchor_backend::prepare_anchor(ui)?;

    // 3. Load the store's anchor state. Every store is anchored at init, so a
    //    missing anchor.toml is an error state, not a fresh-store case.
    let mut state = AnchorState::load(&ctx.dig_dir)?
        .ok_or_else(|| CliError::Chain("store is not anchored; run `digstore init`".into()))?;

    // A store whose INITIAL mint has not confirmed yet (pending init, no root
    // anchored) cannot accept a commit: there is no confirmed on-chain singleton
    // to update, so an `update` would fail at lineage sync. Refuse with a clear
    // next step rather than a confusing chain error. A pending COMMIT (an in-flight
    // root update) carries a non-empty `last_root` and is the resumable case
    // handled below — so ONLY an empty `last_root` indicates a pending init.
    if state.status == AnchorStatus::Pending && state.last_root.is_empty() {
        return Err(CliError::InvalidArgument(
            "the store's initial mint is not yet confirmed; run `digstore anchor` to confirm it before committing".into(),
        ));
    }

    let launcher_id = parse_bytes32(&state.store_id, "store_id")?;
    let new_root_b32 = Bytes32::new(prepared.root.0);

    // Preserve the on-chain project name/description across this update: `update_root` REPLACES the
    // singleton metadata, so re-send the label/description persisted in config.toml (set at init),
    // or they would be cleared on commit.
    let store_cfg = digstore_store::load_config(ctx.config_path()).ok();
    let label = store_cfg.as_ref().and_then(|c| c.label.clone());
    let description = store_cfg.as_ref().and_then(|c| c.description.clone());

    // 3b. Preflight balance for BOTH assets, with up-front cost disclosure. A
    //     commit pays COMMIT_DIG (100 DIG) embedded in the on-chain bundle PLUS the
    //     XCH fee. Block before the update if the wallet is short on EITHER asset;
    //     roots.log / staging are untouched on a block.
    let sp = ui.spinner("Scanning your wallet…");
    let w = block_on(anchor.scan(&mnemonic))??;
    let have_xch = block_on(anchor.balance(&w))??;
    let have_dig = block_on(anchor.dig_balance(&w))??;
    sp.finish();

    if !ui.json() {
        ui.line(format!(
            "⛓  Committing anchors a new root on Chia mainnet and costs {} DIG + up to {} XCH (fee).",
            format_dig(dig::COMMIT_DIG),
            format_xch(fee)
        ));
        ui.line(format!(
            "   you have {} DIG and {} XCH.",
            format_dig(have_dig),
            format_xch(have_xch)
        ));
    }

    if have_xch < fee {
        return Err(CliError::InsufficientFunds {
            need: fee,
            have: have_xch,
            address: digstore_chain::keys::owner_address(&keys),
            asset: "XCH".into(),
        });
    }
    if have_dig < dig::COMMIT_DIG {
        return Err(CliError::InsufficientFunds {
            need: dig::COMMIT_DIG,
            have: have_dig,
            address: digstore_chain::keys::owner_address(&keys),
            asset: "DIG".into(),
        });
    }

    // 4. Submit the on-chain root update (or reuse an in-flight one).
    //    Idempotency: if a Pending update for THIS exact root was already
    //    submitted, do not re-submit — reuse its coin id and skip to confirm.
    //    `--resubmit` overrides this to force a fresh update (for a stuck pending
    //    update that will not confirm); it spends DIG + an XCH fee again.
    let resume =
        !args.resubmit && state.status == AnchorStatus::Pending && state.last_root == new_root_hex;
    // #17: an optional WRITER DELEGATE key (`--writer-key` / `DIGSTORE_WRITER_KEY`)
    // authorizes the on-chain root advance instead of the owner master seed. When
    // set, the singleton spend is built with the writer's key (the store must carry
    // its delegated puzzle, pre-authorized by the owner via `updateStoreOwnership`);
    // the wallet still pays the 100 DIG + XCH fee. Absent => the owner path.
    let writer_keys = resolve_writer_keys(&args)?;
    if writer_keys.is_some() && !ui.json() {
        ui.line(
            "🔑 advancing the root with a WRITER DELEGATE key (deploy token), not the owner seed",
        );
    }

    let coin_id = if resume {
        parse_bytes32(&state.coin_id, "coin_id")?
    } else {
        let sp = ui.spinner("Building & signing the update…");
        let upd = block_on(async {
            match &writer_keys {
                Some(writer) => {
                    anchor
                        .update_root_writer(
                            launcher_id,
                            new_root_b32,
                            label.clone(),
                            description.clone(),
                            writer,
                            &w,
                            fee,
                        )
                        .await
                }
                None => {
                    anchor
                        .update_root(
                            launcher_id,
                            new_root_b32,
                            label.clone(),
                            description.clone(),
                            &w,
                            fee,
                        )
                        .await
                }
            }
        })
        .and_then(|r| r.map_err(|e| CliError::UpdateFailed(e.to_string())))?;
        sp.finish();
        let coin_hex = hex::encode(upd.new_coin_id.as_ref());
        anchor_ux::report_submitted(ui, "update", &coin_hex, ui.json());

        // Persist a Pending record IMMEDIATELY so a subsequent timeout leaves a
        // resumable anchor.toml pointing at this in-flight update.
        state.status = AnchorStatus::Pending;
        state.last_root = new_root_hex.clone();
        state.coin_id = coin_hex;
        state.last_tx_id = hex::encode(upd.tx_id.as_ref());
        state.save(&ctx.dig_dir)?;
        upd.new_coin_id
    };

    // 5. Block until the update confirms (up to --wait-timeout).
    let confirmed =
        anchor_ux::confirm_with_ui(ui, anchor.as_ref(), coin_id, args.wait_timeout, ui.json())?;
    match confirmed {
        ConfirmState::Confirmed { height } => {
            // Record the confirmation BEFORE finalizing local state.
            state.apply_confirm(&confirmed);
            state.save(&ctx.dig_dir)?;

            // Build the on-chain pointer to embed in the compiled module. The
            // launcher id IS the store id; the coin id is the confirmed update
            // coin. `coinset_url` is a transport hint read from the global config.
            let to_core = |b: Bytes32| {
                let mut a = [0u8; 32];
                a.copy_from_slice(b.as_ref());
                digstore_core::Bytes32(a)
            };
            let coinset_url = digstore_chain::config::dig_home()
                .and_then(|home| digstore_chain::config::GlobalConfig::load(&home))
                .map(|g| g.coinset_url)
                .unwrap_or_else(|_| digstore_chain::config::DEFAULT_COINSET_URL.to_string());
            let cs = digstore_core::datasection::ChainState {
                version: digstore_core::datasection::ChainState::VERSION,
                network: "mainnet".to_string(),
                launcher_id: to_core(launcher_id),
                coin_id: to_core(coin_id),
                confirmed_height: height,
                tx_id: state.last_tx_id.clone(), // best-effort; may be empty
                coinset_url,
            };

            // Only NOW advance local history (roots.log + generation + module +
            // clear staging). The chain has the root; the local store catches up.
            // The interactive `commit` embeds an empty metadata manifest (the dighub `compile`
            // path supplies the store's real manifest); a future CLI `--metadata` can thread one.
            let outcome = store_ops::finalize_commit(
                ctx,
                prepared,
                Some(cs),
                crate::ops::serve::empty_manifest(),
            )?;
            let coin_hex = hex::encode(coin_id.as_ref());

            // The capsule identity of this deployment: `storeId:rootHash`
            // (byte-identical to `digstore_core::Capsule::canonical()`). A store
            // is a sequence of capsules — one per commit/root advance.
            let capsule = format!("{}:{}", state.store_id, outcome.roothash.to_hex());

            if ui.json() {
                // Decide + perform the push in JSON mode too. `--push` is built for
                // CI, which runs with `--json`; if the push only happened in the
                // human branch, `commit --push --json` would silently NOT publish.
                // `do_push` only pushes when `--push` is set (json/non-TTY never
                // prompts), so json + no `--push` still pushes nothing — unchanged.
                let pushed = do_push(ctx, ui, &args);
                let mut obj = serde_json::json!({
                    "root": outcome.roothash.to_hex(),
                    "capsule": capsule,
                    "module": outcome.output_path.display().to_string(),
                    "size": outcome.output_size,
                    "coin_id": coin_hex,
                    "anchor_status": "confirmed",
                    "mocked": mocked,
                });
                if let Some(result) = pushed {
                    obj["pushed"] = serde_json::json!(result.is_ok());
                    match &result {
                        Ok(out) => obj["claimed"] = serde_json::json!(out.claimed),
                        Err(e) => obj["push_error"] = serde_json::json!(e.to_string()),
                    }
                }
                ui.emit_json(&obj);
            } else {
                // #14: lead with a plain-language success line. Keep the capsule id
                // (the ecosystem-vocabulary identifier the user shares) on a line
                // BELOW it, and demote the noisier protocol detail (module bytes /
                // on-chain coin) behind --verbose so the default surface stays clean.
                ui.success("Published a new version — it's live and permanent.");
                ui.line(format!("  capsule: {capsule}  (storeId:rootHash)"));
                if ctx.verbose {
                    ui.line(format!(
                        "  module: {} ({} bytes)",
                        outcome.output_path.display(),
                        outcome.output_size
                    ));
                    ui.line(format!("  anchored on mainnet (coin {coin_hex})"));
                }
                // Offer to publish this deployment to DIGHub. Never blocks/prompts in
                // --json/non-TTY runs (see `maybe_offer_push`).
                maybe_offer_push(ctx, ui, &args);
            }
            Ok(())
        }
        ConfirmState::Pending => {
            // Do NOT finalize: roots.log, generations, and staging are UNTOUCHED;
            // anchor.toml stays Pending (saved above) and resumable.
            if !ui.json() {
                ui.line(format!(
                    "⏳ update submitted (root {new_root_hex}) — not yet confirmed; it will confirm in the background. Re-run `digstore commit` to finish."
                ));
            }
            Err(CliError::ConfirmTimeout)
        }
    }
}

/// `commit --dry-run`: report the resulting version (root) and the EXACT cost of
/// publishing it (100 DIG + the configured XCH fee) WITHOUT spending, anchoring,
/// or finalizing anything. The root is computed from staging exactly as a real
/// commit would (so the previewed root is the one a real commit produces); the
/// fee is read from the global config without unlocking the seed.
fn dry_run(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    root: &digstore_core::Bytes32,
) -> Result<(), CliError> {
    let store_id = ctx.find_store_id()?;
    let capsule = format!("{}:{}", store_id.to_hex(), root.to_hex());

    // The XCH fee is a global-config value; load it directly (no wallet/seed). On
    // any load failure, fall back to the default fee so the preview still works.
    let fee = digstore_chain::config::dig_home()
        .and_then(|home| digstore_chain::config::GlobalConfig::load(&home))
        .map(|g| g.fee)
        .unwrap_or_else(|_| digstore_chain::config::GlobalConfig::default().fee);

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "dry_run": true,
            "root": root.to_hex(),
            "capsule": capsule,
            "store_id": store_id.to_hex(),
            "cost_dig": dig::COMMIT_DIG,
            "cost_dig_display": format_dig(dig::COMMIT_DIG),
            "fee_xch_mojos": fee,
            "fee_xch_display": format_xch(fee),
            "spent": false,
        }));
    } else {
        ui.success(format!("dry run — would publish version {}", root.to_hex()));
        ui.line(format!("  capsule: {capsule}  (storeId:rootHash)"));
        ui.line(format!(
            "  cost: {} DIG + up to {} XCH (fee) — NOTHING spent",
            format_dig(dig::COMMIT_DIG),
            format_xch(fee)
        ));
        ui.hint("digstore commit -m \"<message>\"   # to actually publish");
    }
    Ok(())
}

/// Decide whether a confirmed deployment should be pushed to DIGHub now, and if
/// so perform it via the shared push path ([`crate::commands::push::push_core`]
/// against the default `origin` remote — the same target as `digstore push
/// origin`). This is the ONE push decision, used by BOTH the human and the JSON
/// success branches so `commit --push` publishes regardless of output mode.
///
/// Decision (mirrors the TTY/json gating so it never blocks automation):
/// - `--no-push`: never push (returns `None`).
/// - `--push`: push without asking (works non-interactively / in `--json` too).
/// - interactive (TTY, not json): ask `Push this deployment to DIGHub now? [y/N]`
///   (default No); push on yes.
/// - non-interactive (no flag): do not push (returns `None`).
///
/// Returns `None` when no push was attempted, or `Some(result)` carrying the push
/// outcome. A push FAILURE never fails the commit (the deployment is already
/// confirmed on-chain and finalized locally) — the caller surfaces it.
fn do_push(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    args: &CommitArgs,
) -> Option<Result<crate::commands::push::PushOutcome, CliError>> {
    // Explicit opt-out, or a non-interactive run with no `--push`: do not push.
    if args.no_push || (!args.push && !ui.can_prompt()) {
        return None;
    }
    // `--push` pushes unconditionally; otherwise ask (we are interactive here).
    let push = args.push || ui.confirm("Push this deployment to DIGHub now?", false);
    if !push {
        return None;
    }
    Some(crate::commands::push::push_core(ctx, ui, "origin"))
}

/// Human-mode wrapper over [`do_push`]: pushes per the decision, then prints the
/// same `pushed root … to origin` success line `digstore push origin` would, or
/// surfaces the error + the retry hint. (The JSON branch folds the [`do_push`]
/// result into its single object instead of printing.)
fn maybe_offer_push(ctx: &CliContext, ui: &crate::ui::Ui, args: &CommitArgs) {
    match do_push(ctx, ui, args) {
        // No push attempted (opted out / declined / non-interactive default).
        None => ui.hint("digstore push origin"),
        Some(Ok(out)) => {
            ui.success(format!("pushed root {} to origin", out.root.to_hex()));
            if out.claimed {
                ui.line("linked to your dighub account (pending on-chain owner verification)");
            }
        }
        Some(Err(e)) => {
            // Do NOT fail the (already-confirmed) commit: report and keep the hint.
            ui.error(&e);
            ui.hint("digstore push origin");
        }
    }
}

/// Resolve the optional WRITER DELEGATE (deploy-token) key for the on-chain root
/// advance (#17): `--writer-key` (flag, hidden `--deploy-key` alias) >
/// `DIGSTORE_WRITER_KEY` (env). Returns `None` for the normal owner-signed path.
/// The key is a 64-hex 32-byte seed; it derives the writer's wallet synthetic key,
/// which authorizes a metadata-only update of a store that delegated to it.
fn resolve_writer_keys(
    args: &CommitArgs,
) -> Result<Option<digstore_chain::keys::WalletKeys>, CliError> {
    let hex_str = args
        .writer_key
        .clone()
        .or_else(|| std::env::var("DIGSTORE_WRITER_KEY").ok())
        .filter(|s| !s.trim().is_empty());
    let Some(hex_str) = hex_str else {
        return Ok(None);
    };
    let bytes = hex::decode(hex_str.trim())
        .map_err(|_| CliError::InvalidArgument("writer deploy key must be 64-hex".into()))?;
    let seed: [u8; 32] = bytes.try_into().map_err(|_| {
        CliError::InvalidArgument("writer deploy key must be a 32-byte (64-hex) seed".into())
    })?;
    Ok(Some(digstore_chain::keys::wallet_keys_from_seed(&seed)))
}

/// Parse a 32-byte hex id from `anchor.toml` into a `chia_protocol::Bytes32`.
fn parse_bytes32(hex_str: &str, field: &str) -> Result<Bytes32, CliError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| CliError::Chain(format!("anchor.toml {field} is not valid hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| CliError::Chain(format!("anchor.toml {field} is not 32 bytes")))?;
    Ok(Bytes32::new(arr))
}
