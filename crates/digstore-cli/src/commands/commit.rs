use chia_protocol::Bytes32;

use crate::cli::CommitArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_state::{AnchorState, AnchorStatus};
use crate::ops::{anchor_backend, anchor_ux, store_ops};
use crate::runtime::block_on;
use digstore_chain::anchor::ConfirmState;

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
    let (keys, anchor, mocked, fee) = anchor_backend::prepare_anchor(ui)?;

    // 3. Load the store's anchor state. Every store is anchored at init, so a
    //    missing anchor.toml is an error state, not a fresh-store case.
    let mut state = AnchorState::load(&ctx.dig_dir)?
        .ok_or_else(|| CliError::Chain("store is not anchored; run `digstore init`".into()))?;
    let launcher_id = parse_bytes32(&state.store_id, "store_id")?;
    let new_root_b32 = Bytes32::new(prepared.root.0);

    // 4. Submit the on-chain root update (or reuse an in-flight one).
    //    Idempotency: if a Pending update for THIS exact root was already
    //    submitted, do not re-submit — reuse its coin id and skip to confirm.
    let resume = state.status == AnchorStatus::Pending && state.last_root == new_root_hex;
    let coin_id = if resume {
        parse_bytes32(&state.coin_id, "coin_id")?
    } else {
        let upd = block_on(anchor.update_root(launcher_id, new_root_b32, &keys, fee))
            .and_then(|r| r.map_err(|e| CliError::UpdateFailed(e.to_string())))?;
        let coin_hex = hex::encode(upd.new_coin_id.as_ref());
        anchor_ux::report_submitted(ui, "update", &coin_hex, ui.json());

        // Persist a Pending record IMMEDIATELY so a subsequent timeout leaves a
        // resumable anchor.toml pointing at this in-flight update.
        state.status = AnchorStatus::Pending;
        state.last_root = new_root_hex.clone();
        state.coin_id = coin_hex;
        state.save(&ctx.dig_dir)?;
        upd.new_coin_id
    };

    // 5. Block until the update confirms (up to --wait-timeout).
    let confirmed =
        anchor_ux::confirm_with_ui(ui, anchor.as_ref(), coin_id, args.wait_timeout, ui.json())?;
    match confirmed {
        ConfirmState::Confirmed { .. } => {
            // Record the confirmation BEFORE finalizing local state.
            state.apply_confirm(&confirmed);
            state.save(&ctx.dig_dir)?;

            // Only NOW advance local history (roots.log + generation + module +
            // clear staging). The chain has the root; the local store catches up.
            let outcome = store_ops::finalize_commit(ctx, prepared)?;
            let coin_hex = hex::encode(coin_id.as_ref());

            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "root": outcome.roothash.to_hex(),
                    "module": outcome.output_path.display().to_string(),
                    "size": outcome.output_size,
                    "coin_id": coin_hex,
                    "anchor_status": "confirmed",
                    "mocked": mocked,
                }));
            } else {
                ui.success(format!("committed root {}", outcome.roothash.to_hex()));
                ui.line(format!(
                    "  module: {} ({} bytes)",
                    outcome.output_path.display(),
                    outcome.output_size
                ));
                ui.line(format!("  anchored on mainnet (coin {coin_hex})"));
                ui.hint("digstore push origin");
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

/// Parse a 32-byte hex id from `anchor.toml` into a `chia_protocol::Bytes32`.
fn parse_bytes32(hex_str: &str, field: &str) -> Result<Bytes32, CliError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| CliError::Chain(format!("anchor.toml {field} is not valid hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| CliError::Chain(format!("anchor.toml {field} is not 32 bytes")))?;
    Ok(Bytes32::new(arr))
}
