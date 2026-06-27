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
    let coin_id = if resume {
        parse_bytes32(&state.coin_id, "coin_id")?
    } else {
        let sp = ui.spinner("Building & signing the update…");
        let upd = block_on(anchor.update_root(
            launcher_id,
            new_root_b32,
            label.clone(),
            description.clone(),
            &w,
            fee,
        ))
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
                ui.emit_json(&serde_json::json!({
                    "root": outcome.roothash.to_hex(),
                    "capsule": capsule,
                    "module": outcome.output_path.display().to_string(),
                    "size": outcome.output_size,
                    "coin_id": coin_hex,
                    "anchor_status": "confirmed",
                    "mocked": mocked,
                }));
            } else {
                ui.success(format!("committed root {}", outcome.roothash.to_hex()));
                ui.line(format!("  capsule: {capsule}  (storeId:rootHash)"));
                ui.line(format!(
                    "  module: {} ({} bytes)",
                    outcome.output_path.display(),
                    outcome.output_size
                ));
                ui.line(format!("  anchored on mainnet (coin {coin_hex})"));
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

/// After a deployment confirms (human-mode success branch only), decide whether to
/// publish it to DIGHub and, if so, run the existing push path. This is the ONLY
/// place commit pushes; it never duplicates the push logic — it calls
/// [`crate::commands::push::run`] with the default (`origin`) remote, exactly as
/// `digstore push origin` would.
///
/// Decision (mirrors the `ui.spinner`/`progress_bar` TTY/json gating so it never
/// blocks automation):
/// - `--no-push`: never push; keep the `digstore push origin` hint.
/// - `--push`: push without asking (works non-interactively too).
/// - interactive (TTY, not json): ask `Push this deployment to DIGHub now? [y/N]`
///   (default No); push on yes, keep the hint on no.
/// - non-interactive (no flag): do nothing but print the hint — never prompt/block.
///
/// A push failure here NEVER fails the commit: the deployment is already confirmed
/// on-chain and finalized locally. We surface a clear error line and the hint so the
/// user can retry with `digstore push origin`.
fn maybe_offer_push(ctx: &CliContext, ui: &crate::ui::Ui, args: &CommitArgs) {
    // Explicit opt-out, or a non-interactive run with no `--push`: just leave the hint.
    if args.no_push || (!args.push && !ui.can_prompt()) {
        ui.hint("digstore push origin");
        return;
    }

    // `--push` pushes unconditionally; otherwise ask (we are interactive here).
    let do_push = args.push || ui.confirm("Push this deployment to DIGHub now?", false);
    if !do_push {
        // User declined: keep the hint so they know how to publish later.
        ui.hint("digstore push origin");
        return;
    }

    // Reuse the existing push path — the same target as `digstore push origin`.
    // Its own (now-indeterminate) progress bar runs while it works; on success it
    // prints "pushed root … to origin".
    match crate::commands::push::run(
        ctx,
        ui,
        crate::cli::PushArgs {
            remote: "origin".to_string(),
        },
    ) {
        Ok(()) => {}
        Err(e) => {
            // Do NOT fail the (already-confirmed) commit: report and keep the hint.
            ui.error(&e);
            ui.hint("digstore push origin");
        }
    }
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
