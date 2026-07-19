//! Capped, high-value-first XCH coin selection + consolidation for digstore's
//! on-chain money paths (`init` mint, `commit`/`deploy` root-advance).
//!
//! This is the digstore-side adapter over the **canonical dig-wallet-backend
//! primitive** (`dig_wallet_backend::engine::selection`). digstore NEVER hand-rolls
//! coin selection (SYSTEM.md §4.1 / Appendix B): the ordering (descending by amount,
//! tie-broken by coin id), the coin cap ([`COIN_CAP`]), and the `NeedsConsolidation`
//! vs `InsufficientFunds` distinction are the ecosystem-wide contract, expressed once
//! in dig-wallet-backend and reused here.
//!
//! ## Why a cap
//!
//! `init`/`commit`/`deploy` build and broadcast REAL spend bundles. A bundle drawing
//! too many input coins exceeds Chia's block/mempool cost ceiling and is rejected, so
//! a coin-fragmented wallet would be silently unable to publish. Selecting the largest
//! coins first, bounded at [`COIN_CAP`], keeps the bundle small; when the largest
//! [`COIN_CAP`] coins still cannot cover the spend the wallet must first CONSOLIDATE
//! (merge coins into fewer, larger ones) and retry — see
//! [`select_for_consolidation`].
//!
//! ## Fronting the builder, not replacing it
//!
//! The existing datalayer_driver / `chia-wallet-sdk` builders still CONSTRUCT the
//! bundle. This module only PRE-PICKS the coins: [`select_xch`] returns the exact
//! coins to spend (or a discriminated "needs consolidation" / "insufficient"
//! outcome), and the mint/update builders spend from precisely that set. Selection is
//! pure — no network, no signing.

use chia_protocol::Coin;
use dig_wallet_backend::engine::selection::{
    select_for_consolidation, select_for_spend, SelectionOutcome,
};

use crate::error::{ChainError, Result};

/// Default maximum number of coins a single digstore spend may consume (50).
///
/// digstore fixes its own cap so a `init`/`commit`/`deploy` bundle stays within Chia's
/// block/mempool cost ceiling — a smaller bound than dig-wallet-backend's default
/// ([`dig_wallet_backend::engine::selection::DEFAULT_COIN_CAP`]), which serves general
/// wallets. Passed explicitly to every selection call so the browser/JS spend layer and
/// every other digstore consumer agree on the exact boundary between "spendable" and
/// "needs consolidation".
pub const COIN_CAP: usize = 50;

/// Outcome of a capped, high-value-first XCH selection over digstore's native coins.
///
/// Mirrors dig-wallet-backend's [`SelectionOutcome`], re-expressed in digstore's own
/// error/variant vocabulary for the mint/update builders. The caller matches the
/// variant — `NeedsConsolidation` is never conflated with `InsufficientFunds`
/// (consolidation cannot create value).
#[derive(Debug, Clone)]
pub enum XchSelection {
    /// Coins reaching the target were found within the cap; spend exactly these.
    Selected {
        /// The selected coins, high-value-first (a subset of the input coins,
        /// preserving each coin's identity so the builder can spend it under its
        /// owning key).
        coins: Vec<Coin>,
        /// Total mojos of the selected coins.
        total: u64,
        /// Excess (`total - target`) — the change output.
        change: u64,
    },
    /// Enough total XCH exists, but reaching the target needs more than `cap` coins.
    /// Consolidate (merge coins) and retry.
    NeedsConsolidation {
        /// Total unspent XCH coins the wallet holds.
        available_coin_count: u32,
        /// Sum of all unspent XCH (always `>= required`).
        available_total: u64,
        /// The target that could not be reached within the cap, in mojos.
        required: u64,
        /// The coin-count cap in force.
        cap: usize,
    },
    /// The wallet's total XCH is below the target — genuinely insufficient funds.
    InsufficientFunds {
        /// Sum of all unspent XCH (always `< required`).
        available_total: u64,
        /// The target amount, in mojos.
        required: u64,
    },
}

/// Select XCH coins to cover `target` mojos, high-value-first, capped at `cap` coins
/// (pass [`COIN_CAP`] for the default of 50).
///
/// Delegates to dig-wallet-backend's [`select_for_spend`] — which speaks in
/// `chia_protocol::Coin` natively — and re-expresses the result in digstore's
/// [`XchSelection`]. Pure — no network, no signing.
pub fn select_xch(coins: &[Coin], target: u64, cap: usize) -> Result<XchSelection> {
    Ok(match select_for_spend(coins, target, cap) {
        SelectionOutcome::Selected {
            coins,
            total,
            change,
        } => XchSelection::Selected {
            coins,
            total,
            change,
        },
        SelectionOutcome::NeedsConsolidation {
            available_coin_count,
            available_total,
            required,
            cap,
        } => XchSelection::NeedsConsolidation {
            available_coin_count,
            available_total,
            required,
            cap,
        },
        SelectionOutcome::InsufficientFunds {
            available_total,
            required,
        } => XchSelection::InsufficientFunds {
            available_total,
            required,
        },
    })
}

/// Select the XCH coins to spend to cover `target`, or fail with the discriminated
/// [`ChainError`] the mint/update builders + CLI act on.
///
/// This is the drop-in replacement for `datalayer_driver::select_coins` in every
/// XCH-funding spend builder — it fronts the datalayer_driver bundle construction
/// with the shared capped selector (cap [`COIN_CAP`]), so the coins the builder
/// consumes are always the largest, at most 50, high-value-first:
/// - `Ok(coins)` — spend exactly these.
/// - `Err(ChainError::NeedsConsolidation { .. })` — enough value, too many coins;
///   the CLI consolidates + retries.
/// - `Err(ChainError::Chain(..))` — a genuine XCH shortfall (clear message).
pub fn select_xch_coins(coins: &[Coin], target: u64) -> Result<Vec<Coin>> {
    match select_xch(coins, target, COIN_CAP)? {
        XchSelection::Selected { coins, .. } => Ok(coins),
        XchSelection::NeedsConsolidation {
            available_coin_count,
            available_total,
            required,
            cap,
        } => Err(ChainError::NeedsConsolidation {
            asset: "XCH".to_string(),
            available_coin_count,
            available_total,
            required,
            cap,
        }),
        XchSelection::InsufficientFunds {
            available_total,
            required,
        } => Err(ChainError::Chain(format!(
            "insufficient XCH: have {available_total} mojos, need {required}"
        ))),
    }
}

/// Select up to `cap` XCH coins to merge into a single coin during consolidation
/// (highest-value first, deterministic). Requires at least 2 coins.
///
/// Delegates to dig-wallet-backend's [`select_for_consolidation`]. Pure — no network,
/// no signing. The returned coins feed the consolidation bundle builder
/// ([`crate::send::build_xch_consolidation`]).
pub fn select_xch_for_consolidation(coins: &[Coin], cap: usize) -> Result<Vec<Coin>> {
    select_for_consolidation(coins, cap)
        .map_err(|e| ChainError::Chain(format!("consolidation selection: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chia_protocol::Bytes32;

    /// Build a coin with a distinct id per `seed` (distinct parent bytes) and `amount`.
    fn coin(amount: u64, seed: u8) -> Coin {
        Coin::new(
            Bytes32::new([seed; 32]),
            Bytes32::new([seed.wrapping_add(100); 32]),
            amount,
        )
    }

    fn amounts(coins: &[Coin]) -> Vec<u64> {
        coins.iter().map(|c| c.amount).collect()
    }

    #[test]
    fn coin_cap_default_is_50() {
        assert_eq!(COIN_CAP, 50);
    }

    /// Regression guard for the epic #998 migration: digstore-chain must NOT depend on
    /// the deprecated dig-l1-wallet — coin selection now comes from dig-wallet-backend.
    #[test]
    fn no_dig_l1_wallet_dependency() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            !manifest.contains("dig-l1-wallet"),
            "digstore-chain must not depend on the deprecated dig-l1-wallet"
        );
        assert!(
            manifest.contains("dig-wallet-backend"),
            "digstore-chain must depend on dig-wallet-backend"
        );
    }

    #[test]
    fn selects_high_value_first() {
        let coins = [coin(100, 1), coin(300, 2), coin(200, 3)];
        match select_xch(&coins, 400, COIN_CAP).unwrap() {
            XchSelection::Selected {
                coins,
                total,
                change,
            } => {
                // Largest first: 300 then 200 reaches 400; the 100 coin is untouched.
                assert_eq!(amounts(&coins), vec![300, 200]);
                assert_eq!(total, 500);
                assert_eq!(change, 100);
            }
            other => panic!("expected Selected, got {other:?}"),
        }
    }

    #[test]
    fn selected_coins_preserve_identity() {
        // The coins returned on `Selected` must be the SAME coins (by coin id) as the
        // inputs, so the builder can re-attach each coin's owning key.
        let inputs = [coin(500, 7), coin(400, 8)];
        match select_xch(&inputs, 600, COIN_CAP).unwrap() {
            XchSelection::Selected { coins, .. } => {
                let input_ids: std::collections::HashSet<Bytes32> =
                    inputs.iter().map(|c| c.coin_id()).collect();
                for c in &coins {
                    assert!(
                        input_ids.contains(&c.coin_id()),
                        "selected coin is an input"
                    );
                }
            }
            other => panic!("expected Selected, got {other:?}"),
        }
    }

    #[test]
    fn at_cap_boundary_is_selected() {
        // 50 coins of 1; cap 50; target 50 → all 50 reach it exactly (still within cap).
        let coins: Vec<Coin> = (0..50).map(|i| coin(1, i as u8)).collect();
        match select_xch(&coins, 50, 50).unwrap() {
            XchSelection::Selected { total, change, .. } => {
                assert_eq!(total, 50);
                assert_eq!(change, 0);
            }
            other => panic!("expected Selected at the cap boundary, got {other:?}"),
        }
    }

    #[test]
    fn one_over_cap_needs_consolidation() {
        // 51 coins of 1; cap 50; target 51 → total (51) is enough but the largest 50
        // sum to only 50 < 51, so the spend cannot be built within the cap.
        let coins: Vec<Coin> = (0..51).map(|i| coin(1, i as u8)).collect();
        match select_xch(&coins, 51, 50).unwrap() {
            XchSelection::NeedsConsolidation {
                available_coin_count,
                available_total,
                required,
                cap,
            } => {
                assert_eq!(available_coin_count, 51);
                assert_eq!(available_total, 51);
                assert_eq!(required, 51);
                assert_eq!(cap, 50);
            }
            other => panic!("expected NeedsConsolidation, got {other:?}"),
        }
    }

    #[test]
    fn genuine_shortfall_is_insufficient_not_consolidation() {
        // 51 coins of 1 (total 51); target 100 → no consolidation can reach it.
        let coins: Vec<Coin> = (0..51).map(|i| coin(1, i as u8)).collect();
        match select_xch(&coins, 100, 50).unwrap() {
            XchSelection::InsufficientFunds {
                available_total,
                required,
            } => {
                assert_eq!(available_total, 51);
                assert_eq!(required, 100);
            }
            other => panic!("expected InsufficientFunds, got {other:?}"),
        }
    }

    #[test]
    fn empty_wallet_is_insufficient() {
        match select_xch(&[], 10, COIN_CAP).unwrap() {
            XchSelection::InsufficientFunds {
                available_total,
                required,
            } => {
                assert_eq!(available_total, 0);
                assert_eq!(required, 10);
            }
            other => panic!("expected InsufficientFunds, got {other:?}"),
        }
    }

    #[test]
    fn consolidation_picks_largest_capped() {
        let coins = [coin(5, 1), coin(1, 2), coin(4, 3), coin(2, 4), coin(3, 5)];
        let picked = select_xch_for_consolidation(&coins, 3).unwrap();
        // Top 3 by value, descending.
        assert_eq!(amounts(&picked), vec![5, 4, 3]);
    }

    #[test]
    fn consolidation_requires_two_coins() {
        assert!(select_xch_for_consolidation(&[coin(10, 1)], 50).is_err());
        assert!(select_xch_for_consolidation(&[], 50).is_err());
    }
}
