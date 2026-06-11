use chia_protocol::Bytes32;

use crate::cli::{AnchorAction, AnchorArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::anchor_backend::{build_anchor, warn_if_mocked};
use crate::ops::anchor_state::{AnchorState, AnchorStatus};
use crate::ops::anchor_ux;
use crate::ops::store_ops;
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
    // `inspect` operates on the given module file only — it does not require
    // the store's anchor.toml, so dispatch it before that load.
    if let Some(AnchorAction::Inspect { ref module }) = args.action {
        return inspect(ui, module);
    }

    // Every store is anchored at init; a missing anchor.toml is an error state.
    let mut state = AnchorState::load(&ctx.dig_dir)?
        .ok_or_else(|| CliError::Chain("store is not anchored; run `digstore init`".into()))?;

    // Read-only: do NOT unlock the seed — confirm/status need no wallet keys.
    let (anchor, mocked) = build_anchor();
    warn_if_mocked(ui, mocked);

    let coin_id = parse_coin_id(&state.coin_id)?;

    match args.action {
        Some(AnchorAction::Status) => status(ctx, ui, anchor.as_ref(), &state, coin_id, mocked),
        // `Inspect` is handled above; this arm is unreachable but required by the compiler.
        Some(AnchorAction::Inspect { .. }) => unreachable!(),
        None => resume(
            ctx,
            ui,
            anchor.as_ref(),
            &mut state,
            coin_id,
            mocked,
            args.wait_timeout,
        ),
    }
}

/// Serialize a `ChainState` to a JSON value for `--json` output.
fn chain_state_json(cs: &digstore_core::datasection::ChainState) -> serde_json::Value {
    serde_json::json!({
        "network": cs.network,
        "launcher_id": cs.launcher_id.to_hex(),
        "coin_id": cs.coin_id.to_hex(),
        "confirmed_height": cs.confirmed_height,
        "tx_id": cs.tx_id,
        "coinset_url": cs.coinset_url,
    })
}

/// Best-effort: load the current module and decode its embedded `ChainState`.
/// Returns `None` on any error (no committed module, missing file, no pointer).
fn read_module_chain_state_for_store(
    ctx: &CliContext,
    store_id_hex: &str,
) -> Option<digstore_core::datasection::ChainState> {
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).ok()?;
    let path = store_ops::module_path_for(ctx, &store_id, None).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    store_ops::read_module_chain_state(&bytes).ok().flatten()
}

/// Read-only inspect: print the persisted record plus a single live on-chain
/// check (`confirm(_, 0)` polls once, non-blocking). Always exits 0.
fn status(
    ctx: &CliContext,
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

    // Best-effort: decode the embedded chain pointer from the current module.
    let module_cs = read_module_chain_state_for_store(ctx, &state.store_id);

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
            "module_chain_state": module_cs.as_ref().map(chain_state_json),
        }));
        return Ok(());
    }

    ui.line(format!("network:          {}", state.network));
    ui.line(format!("store_id:         {}", state.store_id));
    ui.line(format!("coin_id:          {}", state.coin_id));
    ui.line(format!(
        "status:           {}",
        persisted_status(state.status)
    ));
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
    if let Some(cs) = &module_cs {
        ui.line(format!("module network:   {}", cs.network));
        ui.line(format!("module launcher:  {}", cs.launcher_id.to_hex()));
        ui.line(format!("module coin:      {}", cs.coin_id.to_hex()));
        ui.line(format!("module height:    {}", cs.confirmed_height));
    }
    Ok(())
}

/// Decode and print the embedded chain pointer from a compiled `.dig` module.
fn inspect(ui: &crate::ui::Ui, module: &std::path::Path) -> Result<(), CliError> {
    let bytes =
        std::fs::read(module).map_err(|e| CliError::Other(anyhow::anyhow!("read module: {e}")))?;
    let cs = store_ops::read_module_chain_state(&bytes)?
        .ok_or_else(|| CliError::NotFound("module carries no chain state".into()))?;

    if ui.json() {
        ui.emit_json(&chain_state_json(&cs));
        return Ok(());
    }

    ui.line(format!("network:          {}", cs.network));
    ui.line(format!("launcher_id:      {}", cs.launcher_id.to_hex()));
    ui.line(format!("coin_id:          {}", cs.coin_id.to_hex()));
    ui.line(format!("confirmed_height: {}", cs.confirmed_height));
    ui.line(format!("tx_id:            {}", cs.tx_id));
    ui.line(format!("coinset_url:      {}", cs.coinset_url));
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

            // Determine whether the anchored root is ahead of the local head.
            // This happens when a `commit` timed out: the on-chain root update was
            // confirmed here but the local generation was never finalized.
            let local_head = store_ops::current_root(ctx)?
                .map(|r| r.to_hex())
                .unwrap_or_default();
            let needs_commit_finalize =
                !state.last_root.is_empty() && state.last_root != local_head;

            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "store_id": state.store_id,
                    "coin_id": state.coin_id,
                    "status": "confirmed",
                    "confirmed_height": state.confirmed_height,
                    "needs_commit_finalize": needs_commit_finalize,
                    "mocked": mocked,
                }));
            } else {
                ui.success(format!(
                    "anchor confirmed (height {})",
                    state.confirmed_height
                ));
                if needs_commit_finalize {
                    ui.hint(
                        "the anchored root is ahead of your local history; \
                         run `digstore commit` to finalize it",
                    );
                }
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
