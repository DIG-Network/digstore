//! Shared XCH consolidation loop for the on-chain money commands (`init`, `commit`,
//! `deploy`).
//!
//! When a mint / root-advance spend build reports
//! [`ChainError::NeedsConsolidation`](digstore_chain::ChainError::NeedsConsolidation)
//! — the wallet holds enough XCH but is too coin-fragmented to build the bundle
//! within the 50-coin cap — this drives the coin-management contract (epic #410):
//! consolidate the coins, wait for the merge to confirm on-chain, re-scan, and retry
//! the original spend; repeat until it succeeds, the user declines, or a bounded
//! round limit. Consolidation spends a real XCH fee, so it runs ONLY with the user's
//! consent (`--consolidate`/`--yes`, or an interactive yes).

use digstore_chain::anchor::{ChainAnchor, ConfirmState};
use digstore_chain::wallet::ScannedWallet;

use crate::error::CliError;
use crate::runtime::block_on;
use crate::ui::Ui;

/// Which on-chain money operation is being retried — decides how a non-consolidation
/// spend error is surfaced (`MINT_FAILED` vs `UPDATE_FAILED`, preserving exit codes).
#[derive(Clone, Copy)]
pub enum SpendKind {
    Mint,
    Update,
}

/// Map a chain spend error into a [`CliError`], routing `NeedsConsolidation` to the
/// dedicated variant (so the loop intercepts it) and everything else to the op's
/// failure variant. This is what the mint/update attempt closure uses so the loop
/// can pattern-match `CliError::NeedsConsolidation`.
pub fn map_spend_error(e: digstore_chain::ChainError, kind: SpendKind) -> CliError {
    match e {
        digstore_chain::ChainError::NeedsConsolidation { .. } => CliError::from(e),
        other => match kind {
            SpendKind::Mint => CliError::MintFailed(other.to_string()),
            SpendKind::Update => CliError::UpdateFailed(other.to_string()),
        },
    }
}

/// Whether consolidation may proceed WITHOUT prompting: an explicit `--consolidate`
/// flag or the global `--yes`.
pub fn preapproved(ui: &Ui, consolidate_flag: bool) -> bool {
    consolidate_flag || ui.assume_yes()
}

/// A bounded guard on the retry loop. Each round merges up to 50 coins into one, so a
/// realistic wallet converges in a couple of rounds; this only prevents a pathological
/// infinite loop (e.g. a fee that erodes value faster than the merge concentrates it).
const MAX_ROUNDS: usize = 12;

/// Run `attempt` (a spend build+broadcast), consolidating + retrying on
/// `NeedsConsolidation`. Returns the successful outcome together with the (possibly
/// re-scanned) wallet.
///
/// - `consolidate_flag` — the resolved `--consolidate`/`--yes` opt-in. When
///   consolidation is needed and neither this nor an interactive yes is given, returns
///   `CliError::NeedsConsolidation` (never spends without consent).
/// - `attempt` — builds+broadcasts the spend for a given wallet; its errors must
///   already be mapped via [`map_spend_error`] so `NeedsConsolidation` is
///   distinguishable.
#[allow(clippy::too_many_arguments)]
pub fn with_consolidation<T>(
    ui: &Ui,
    anchor: &dyn ChainAnchor,
    mnemonic: &str,
    fee: u64,
    consolidate_flag: bool,
    wait_timeout: u64,
    mut wallet: ScannedWallet,
    mut attempt: impl FnMut(&ScannedWallet) -> Result<T, CliError>,
) -> Result<(T, ScannedWallet), CliError> {
    for _round in 0..=MAX_ROUNDS {
        match attempt(&wallet) {
            Ok(value) => return Ok((value, wallet)),
            Err(CliError::NeedsConsolidation {
                asset,
                coin_count,
                available,
                required,
                cap,
            }) => {
                let go = preapproved(ui, consolidate_flag)
                    || (ui.can_prompt()
                        && ui.confirm(
                            &format!(
                                "Your wallet has {coin_count} small {asset} coins — the largest \
                                 {cap} can't cover this spend. Combine them first (one extra XCH fee)?"
                            ),
                            false,
                        ));
                if !go {
                    // No consent: surface the actionable error (exit NEEDS_CONSOLIDATION).
                    return Err(CliError::NeedsConsolidation {
                        asset,
                        coin_count,
                        available,
                        required,
                        cap,
                    });
                }
                run_consolidation(ui, anchor, &wallet, fee, cap, wait_timeout, &asset)?;
                // Re-scan (coins changed) and retry the spend.
                wallet = block_on(anchor.scan(mnemonic))??;
            }
            Err(other) => return Err(other),
        }
    }
    Err(CliError::Chain(format!(
        "consolidation did not converge after {MAX_ROUNDS} rounds; try a manual consolidation"
    )))
}

/// Build + broadcast ONE consolidation and block until the merged coin confirms.
fn run_consolidation(
    ui: &Ui,
    anchor: &dyn ChainAnchor,
    wallet: &ScannedWallet,
    fee: u64,
    cap: usize,
    wait_timeout: u64,
    asset: &str,
) -> Result<(), CliError> {
    if !ui.json() {
        ui.line(format!(
            "🧹 Consolidating your {asset} coins so this spend fits in one transaction…"
        ));
    }
    let outcome = block_on(anchor.consolidate_xch(wallet, fee, cap))??;
    // A consolidation must confirm before the retry can spend the merged coin.
    let state = block_on(anchor.confirm(outcome.output_coin_id, wait_timeout))??;
    if !matches!(state, ConfirmState::Confirmed { .. }) {
        return Err(CliError::ConfirmTimeout);
    }
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "event": "consolidated",
            "asset": asset,
            "merged_coins": outcome.input_count,
            "merged_mojos": outcome.merged,
            "output_coin_id": hex::encode(outcome.output_coin_id.as_ref()),
            "tx_id": hex::encode(outcome.tx_id.as_ref()),
        }));
    } else {
        ui.success(format!(
            "Merged {} {asset} coins into one ({} mojos).",
            outcome.input_count, outcome.merged
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::anchor_backend::MockAnchor;
    use std::sync::atomic::AtomicUsize;

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon art";

    /// A non-interactive UI for tests (no TTY → `can_prompt()` is false, so a
    /// declined-consolidation case never blocks on a prompt).
    fn ui() -> Ui {
        Ui::resolve(
            crate::ui::ColorChoice::Never,
            false,
            false,
            false,
            false,
            false,
        )
    }

    fn wallet() -> ScannedWallet {
        block_on(MockAnchor::default().scan(ABANDON)).unwrap().unwrap()
    }

    #[test]
    fn succeeds_first_try_when_not_fragmented() {
        // fragmented_rounds = 0 → the attempt succeeds immediately; no consolidation.
        let anchor = MockAnchor::default();
        let mut attempts = 0;
        let (v, _w) = with_consolidation(&ui(), &anchor, ABANDON, 0, false, 1, wallet(), |_w| {
            attempts += 1;
            Ok::<_, CliError>(42)
        })
        .unwrap();
        assert_eq!(v, 42);
        assert_eq!(attempts, 1, "no retry when not fragmented");
    }

    #[test]
    fn consolidates_then_retries_to_success() {
        // The mock reports NeedsConsolidation once, then succeeds; --consolidate given.
        let anchor = MockAnchor {
            fragmented_rounds: AtomicUsize::new(1),
            ..MockAnchor::default()
        };
        let mut attempts = 0;
        let (v, _w) = with_consolidation(&ui(), &anchor, ABANDON, 0, true, 1, wallet(), |_w| {
            attempts += 1;
            // Model the spend: the mock's fragmented counter drives the outcome; here
            // we mirror it by asking the mock to mint (which consumes a round).
            block_on(anchor.mint_empty_store(&wallet(), None, None, 0))
                .unwrap()
                .map(|_| 7)
                .map_err(|e| map_spend_error(e, SpendKind::Mint))
        })
        .unwrap();
        assert_eq!(v, 7);
        assert_eq!(attempts, 2, "one NeedsConsolidation round then a successful retry");
    }

    #[test]
    fn declined_consolidation_returns_needs_consolidation() {
        // Fragmented, but no --consolidate and non-interactive → NEEDS_CONSOLIDATION.
        let anchor = MockAnchor {
            fragmented_rounds: AtomicUsize::new(1),
            ..MockAnchor::default()
        };
        let err = with_consolidation(&ui(), &anchor, ABANDON, 0, false, 1, wallet(), |_w| {
            block_on(anchor.mint_empty_store(&wallet(), None, None, 0))
                .unwrap()
                .map(|_| 0u32)
                .map_err(|e| map_spend_error(e, SpendKind::Mint))
        })
        .unwrap_err();
        assert!(matches!(err, CliError::NeedsConsolidation { .. }));
        assert_eq!(err.exit_code(), 18);
    }
}
