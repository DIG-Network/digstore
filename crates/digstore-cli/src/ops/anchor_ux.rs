//! Confirmation-reporting UX shared by `init`/`commit`/`anchor`.
//!
//! ## Reporting granularity (known limitation)
//!
//! The [`ChainAnchor::confirm`] trait method polls the chain *internally* and
//! only returns a terminal [`ConfirmState`] (`Confirmed`/`Pending`). It exposes
//! no intermediate progress, so this layer cannot surface staged sub-progress
//! (e.g. mempool → confirming N/M blocks). The helper therefore reports the
//! transition `submitted → (confirmed | pending)` only. This is an intentional
//! limitation of the current trait shape, not an oversight here.

use chia_protocol::Bytes32;
use digstore_chain::anchor::{ChainAnchor, ConfirmState};

use crate::error::CliError;
use crate::ui::Ui;

/// Wait for `coin_id` to confirm (up to `timeout_secs`) and report progress.
///
/// In human mode, prints a "waiting" notice before blocking, then a confirmed
/// or pending line. In `--json` mode (`json == true`) it prints nothing — the
/// caller emits the final JSON — and just returns the terminal state.
pub fn confirm_with_ui(
    ui: &Ui,
    anchor: &dyn ChainAnchor,
    coin_id: Bytes32,
    timeout_secs: u64,
    json: bool,
) -> Result<ConfirmState, CliError> {
    if !json {
        ui.line(format!(
            "⛓  Anchoring on Chia mainnet… waiting for confirmation (timeout {timeout_secs}s)"
        ));
    }

    // `block_on` returns `Result<chain_result, CliError>`; the inner is
    // `Result<ConfirmState, ChainError>` → map into `CliError` via `?`.
    let state = crate::runtime::block_on(anchor.confirm(coin_id, timeout_secs))??;

    if !json {
        match &state {
            ConfirmState::Confirmed { height } => {
                ui.success(format!("confirmed (height {height})"));
            }
            ConfirmState::Pending => {
                ui.line("⏳ pending — not yet confirmed; run `digstore anchor status`");
            }
        }
    }

    Ok(state)
}

/// Report that a spend/transaction was submitted to the chain. Human mode prints
/// `✓ submitted {kind} {id_hex}`; `--json` mode is a no-op (the caller's JSON
/// carries the id).
pub fn report_submitted(ui: &Ui, kind: &str, id_hex: &str, json: bool) {
    if !json {
        ui.success(format!("submitted {kind} {id_hex}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::anchor_backend::MockAnchor;

    fn test_ui(json: bool) -> Ui {
        Ui::resolve(
            crate::ui::ColorChoice::Never,
            json,
            false,
            false,
            false,
            false,
            false,
        )
    }

    #[test]
    fn confirm_with_ui_returns_confirmed() {
        let anchor = MockAnchor::default();
        let st = confirm_with_ui(&test_ui(false), &anchor, Bytes32::default(), 1, false).unwrap();
        assert_eq!(st, ConfirmState::Confirmed { height: 1 });
    }

    #[test]
    fn confirm_with_ui_returns_pending_in_json_mode() {
        let anchor = MockAnchor {
            confirm_pending: true,
            ..MockAnchor::default()
        };
        let st = confirm_with_ui(&test_ui(true), &anchor, Bytes32::default(), 1, true).unwrap();
        assert_eq!(st, ConfirmState::Pending);
    }
}
