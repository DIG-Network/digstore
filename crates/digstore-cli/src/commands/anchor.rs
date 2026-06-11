use chia_protocol::Bytes32;

use crate::cli::{AnchorAction, AnchorArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_backend::{build_anchor, warn_if_mocked};
use crate::ops::anchor_state::{AnchorState, AnchorStatus};
use crate::ops::anchor_ux;
use digstore_chain::anchor::ConfirmState;

/// `digstore anchor` resumes a pending on-chain anchor; `digstore anchor status`
/// inspects it read-only.
///
/// SCOPE: this command confirms the on-chain coin recorded in `anchor.toml` and
/// flips that record's status. It is READ-ONLY with respect to the wallet seed —
/// `confirm` needs no keys — so it never unlocks the seed. If the pending state
/// came from a `commit` (a root update that timed out), confirming here updates
/// the anchor record but does NOT finalize the local generation; the user must
/// re-run `digstore commit` (which idempotently reuses the pending update and
/// then finalizes). `anchor` resume fully completes a pending `init` mint. We
/// keep this command focused on chain-state confirmation and never try to
/// finalize a local generation here.
pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: AnchorArgs) -> Result<(), CliError> {
    // Every store is anchored at init; a missing anchor.toml is an error state.
    let mut state = AnchorState::load(&ctx.dig_dir)?.ok_or_else(|| {
        CliError::Chain("store is not anchored; run `digstore init`".into())
    })?;

    // Read-only: do NOT unlock the seed — confirm/status need no wallet keys.
    let (anchor, mocked) = build_anchor();
    warn_if_mocked(ui, mocked);

    let coin_id = parse_coin_id(&state.coin_id)?;

    match args.action {
        Some(AnchorAction::Status) => status(ctx, ui, anchor.as_ref(), &state, coin_id, mocked),
        None => resume(ctx, ui, anchor.as_ref(), &mut state, coin_id, mocked, args.wait_timeout),
    }
}

/// Read-only inspect: print the persisted record plus a single live on-chain
/// check (`confirm(_, 0)` polls once, non-blocking). Always exits 0.
fn status(
    _ctx: &CliContext,
    ui: &crate::ui::Ui,
    anchor: &dyn digstore_chain::anchor::ChainAnchor,
    state: &AnchorState,
    coin_id: Bytes32,
    mocked: bool,
) -> Result<(), CliError> {
    // Single, non-blocking poll (timeout 0) of the recorded coin's live state.
    let live = crate::runtime::block_on(anchor.confirm(coin_id, 0))??;
    let (onchain_confirmed, onchain_height) = match live {
        ConfirmState::Confirmed { height } => (true, Some(height)),
        ConfirmState::Pending => (false, None),
    };

    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "network": state.network,
            "store_id": state.store_id,
            "coin_id": state.coin_id,
            "status": persisted_status(state.status),
            "last_root": state.last_root,
            "confirmed_height": state.confirmed_height,
            "onchain_confirmed": onchain_confirmed,
            "onchain_height": onchain_height,
            "mocked": mocked,
        }));
        return Ok(());
    }

    ui.line(format!("network:          {}", state.network));
    ui.line(format!("store_id:         {}", state.store_id));
    ui.line(format!("coin_id:          {}", state.coin_id));
    ui.line(format!("status:           {}", persisted_status(state.status)));
    let last_root = if state.last_root.is_empty() {
        "(none)"
    } else {
        &state.last_root
    };
    ui.line(format!("last_root:        {last_root}"));
    ui.line(format!("confirmed_height: {}", state.confirmed_height));
    if onchain_confirmed {
        ui.line(format!(
            "on-chain:         confirmed (height {})",
            onchain_height.unwrap_or(0)
        ));
    } else {
        ui.line("on-chain:         not yet confirmed");
    }
    Ok(())
}

/// Resume a pending anchor: if already confirmed, report and exit 0; otherwise
/// block (up to `wait_timeout`) for the recorded coin to confirm, flip the
/// record on success, or return `ConfirmTimeout` (exit 14) if still pending.
fn resume(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    anchor: &dyn digstore_chain::anchor::ChainAnchor,
    state: &mut AnchorState,
    coin_id: Bytes32,
    mocked: bool,
    wait_timeout: u64,
) -> Result<(), CliError> {
    if state.status == AnchorStatus::Confirmed {
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "store_id": state.store_id,
                "coin_id": state.coin_id,
                "status": "confirmed",
                "confirmed_height": state.confirmed_height,
                "mocked": mocked,
            }));
        } else {
            ui.success(format!(
                "already confirmed (height {})",
                state.confirmed_height
            ));
        }
        return Ok(());
    }

    // Pending → wait for the on-chain coin to confirm.
    let confirmed = anchor_ux::confirm_with_ui(ui, anchor, coin_id, wait_timeout, ui.json())?;
    match confirmed {
        ConfirmState::Confirmed { .. } => {
            state.apply_confirm(&confirmed);
            state.save(&ctx.dig_dir)?;
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "store_id": state.store_id,
                    "coin_id": state.coin_id,
                    "status": "confirmed",
                    "confirmed_height": state.confirmed_height,
                    "mocked": mocked,
                }));
            } else {
                ui.success(format!(
                    "anchor confirmed (height {})",
                    state.confirmed_height
                ));
            }
            Ok(())
        }
        ConfirmState::Pending => {
            // Leave anchor.toml pending and resumable.
            if !ui.json() {
                ui.line("still pending; try again later");
            }
            Err(CliError::ConfirmTimeout)
        }
    }
}

/// Lowercase persisted-status label matching `anchor.toml`'s serde encoding.
fn persisted_status(status: AnchorStatus) -> &'static str {
    match status {
        AnchorStatus::Pending => "pending",
        AnchorStatus::Confirmed => "confirmed",
    }
}

/// Parse the recorded coin id hex (`anchor.toml`) into a `chia_protocol::Bytes32`.
fn parse_coin_id(hex_str: &str) -> Result<Bytes32, CliError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| CliError::Chain(format!("anchor.toml coin_id is not valid hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| CliError::Chain("anchor.toml coin_id is not 32 bytes".to_string()))?;
    Ok(Bytes32::new(arr))
}
