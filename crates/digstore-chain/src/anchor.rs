//! High-level anchoring operations the CLI drives: mint an empty store, update
//! its root, and wait for confirmation — all over coinset. Ties together
//! key derivation, spend building/signing, lineage sync, and broadcast.

use crate::cat::{build_dig_payment_multi, dig_cats_multi};
use crate::coinset::ChainReads;
use crate::dig;
use crate::error::{ChainError, Result};
use crate::singleton::{
    build_mint_unsigned_multi, build_update_unsigned_multi, coins_with_keys_from_wallet,
    sync_datastore,
};
use crate::wallet::ScannedWallet;
use chia_protocol::Bytes32;
use datalayer_driver::{sign_coin_spends, SpendBundle};

#[derive(Clone, Debug)]
pub struct MintOutcome {
    pub launcher_id: Bytes32, // == store_id
    pub coin_id: Bytes32,     // eve singleton coin to poll for confirmation
    pub tx_id: Bytes32,       // SpendBundle::name() — conventional tx id of the mint
}

#[derive(Clone, Debug)]
pub struct UpdateOutcome {
    pub new_coin_id: Bytes32, // new singleton coin to poll
    pub tx_id: Bytes32,       // SpendBundle::name() — conventional tx id of the update
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmState {
    Confirmed { height: u32 },
    Pending,
}

#[async_trait::async_trait]
pub trait ChainAnchor: Send + Sync {
    /// Scan the HD wallet and return the aggregated state.
    async fn scan(&self, mnemonic: &str) -> Result<ScannedWallet>;
    /// Total spendable XCH (mojos) across all scanned addresses.
    async fn balance(&self, w: &ScannedWallet) -> Result<u64>;
    /// Total spendable DIG (base units) across all scanned addresses.
    async fn dig_balance(&self, w: &ScannedWallet) -> Result<u64>;
    /// Mint an empty (root = 0) owner-only store using the full scanned wallet;
    /// gathers XCH + DIG across ALL HD addresses and signs with all keys. Broadcasts.
    /// `label`/`description` are written into the CHIP-0035 singleton metadata.
    async fn mint_empty_store(
        &self,
        w: &ScannedWallet,
        label: Option<String>,
        description: Option<String>,
        fee: u64,
    ) -> Result<MintOutcome>;
    /// Sync the current singleton for `launcher_id`, build+broadcast a root update.
    /// Uses the full scanned wallet for fee coins and DIG across all HD addresses.
    /// `label`/`description` are RE-SENT (the update replaces metadata) so they persist.
    async fn update_root(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        label: Option<String>,
        description: Option<String>,
        w: &ScannedWallet,
        fee: u64,
    ) -> Result<UpdateOutcome>;
    /// Poll until `coin_id` is confirmed (present in a block) or `timeout_secs` elapses.
    async fn confirm(&self, coin_id: Bytes32, timeout_secs: u64) -> Result<ConfirmState>;
}

/// Production anchor over coinset.
pub struct CoinsetAnchor<C: ChainReads> {
    chain: C,
}

impl<C: ChainReads> CoinsetAnchor<C> {
    pub fn new(chain: C) -> Self {
        Self { chain }
    }
}

impl CoinsetAnchor<crate::coinset::Coinset> {
    pub fn mainnet() -> Self {
        Self::new(crate::coinset::Coinset::mainnet())
    }
}

#[async_trait::async_trait]
impl<C: ChainReads> ChainAnchor for CoinsetAnchor<C> {
    async fn scan(&self, mnemonic: &str) -> Result<ScannedWallet> {
        crate::wallet::scan_wallet(&self.chain as &dyn ChainReads, mnemonic).await
    }

    async fn balance(&self, w: &ScannedWallet) -> Result<u64> {
        Ok(w.xch_balance())
    }

    async fn dig_balance(&self, w: &ScannedWallet) -> Result<u64> {
        Ok(w.dig_balance())
    }

    async fn mint_empty_store(
        &self,
        w: &ScannedWallet,
        label: Option<String>,
        description: Option<String>,
        fee: u64,
    ) -> Result<MintOutcome> {
        // Index 0 is the store owner and change destination.
        let change_ph = w.addrs[0].keys.owner_puzzle_hash;

        // 2) UNSIGNED DIG payment: 100 DIG to the treasury, memo = launcher id.
        //    Gather DIG cats across ALL scanned addresses.
        //    (Must be done once before the build closure since it requires async.)
        let cats = dig_cats_multi(&self.chain as &dyn ChainReads, w).await?;

        // Helper: build + sign the combined mint+DIG bundle for a given fee.
        let build_mint_bundle = |effective_fee: u64| -> Result<(SpendBundle, Bytes32, Bytes32)> {
            // 1) UNSIGNED mint coin spends: gather XCH across ALL scanned addresses.
            let all_xch = coins_with_keys_from_wallet(w);
            let mint = build_mint_unsigned_multi(
                &all_xch,
                change_ph,
                Bytes32::default(),
                label.clone(),
                description.clone(),
                effective_fee,
            )?;
            let coin_id = mint.datastore.coin.coin_id();
            let launcher_id = mint.launcher_id;

            let pay = build_dig_payment_multi(
                w.addrs.iter().map(|a| &a.keys),
                change_ph,
                &cats,
                dig::INIT_DIG,
                launcher_id,
            )?;

            // 3) Combine into ONE bundle and sign atomically with ALL address keys.
            //    ATOMICITY: the DIG payment and the singleton spend ride in a single
            //    SpendBundle under one aggregated signature, so the mempool admits
            //    them all-or-nothing. This co-signing is the SOLE atomicity guarantee
            //    between the DIG payment and the anchor — these coin spends must never
            //    be split across bundles or pushed separately.
            let mut all = mint.coin_spends;
            all.extend(pay);
            let signature = sign_coin_spends(&all, &w.signing_keys(), false)
                .map_err(|e| ChainError::Chain(format!("sign combined mint+DIG bundle: {e}")))?;
            Ok((SpendBundle::new(all, signature), coin_id, launcher_id))
        };

        // Build the initial bundle (fee=0 if auto-estimating, or the caller's fee override).
        let (bundle, coin_id, launcher_id) = build_mint_bundle(fee)?;

        // Auto-estimate fee when the caller supplied fee=0.
        // Fail-open: if estimation returns 0 or wallet can't cover it, use the fee-0 bundle.
        let (final_bundle, coin_id, launcher_id) = if fee == 0 {
            let est = self.chain.estimate_fee(&bundle, 60).await.unwrap_or(0);
            if est > 0 && w.xch_balance() >= est {
                // Rebuild + re-sign with the estimated fee.
                match build_mint_bundle(est) {
                    Ok(rebuilt) => rebuilt,
                    Err(_) => (bundle, coin_id, launcher_id), // rebuild failed → fall back
                }
            } else {
                (bundle, coin_id, launcher_id) // empty mempool or insufficient XCH → fee 0
            }
        } else {
            (bundle, coin_id, launcher_id) // explicit fee override
        };

        // Bundle hash == conventional tx id; capture it BEFORE the bundle moves.
        let tx_id = final_bundle.name();
        self.chain.push(final_bundle).await?;
        Ok(MintOutcome {
            launcher_id,
            coin_id,
            tx_id,
        })
    }

    async fn update_root(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        label: Option<String>,
        description: Option<String>,
        w: &ScannedWallet,
        fee: u64,
    ) -> Result<UpdateOutcome> {
        // Index 0 is the store owner (the singleton was minted by its synthetic_pk)
        // and the change destination.
        let change_ph = w.addrs[0].keys.owner_puzzle_hash;
        let owner_pk = w.addrs[0].keys.synthetic_pk;

        // &self.chain is &C; coerces to &dyn ChainReads because C: ChainReads.
        let store = sync_datastore(&self.chain as &dyn ChainReads, launcher_id).await?;

        // 2) UNSIGNED DIG payment: 100 DIG (COMMIT_DIG) to the treasury, memo = store id.
        //    Gather DIG cats across ALL scanned addresses.
        //    (Must be done once before the build closure since it requires async.)
        let cats = dig_cats_multi(&self.chain as &dyn ChainReads, w).await?;

        // Helper: build + sign the combined update+DIG bundle for a given fee.
        let build_update_bundle = |effective_fee: u64| -> Result<(SpendBundle, Bytes32)> {
            // 1) UNSIGNED update coin spends (singleton + XCH fee) across all addresses.
            let all_xch = coins_with_keys_from_wallet(w);
            let update = build_update_unsigned_multi(
                owner_pk,
                store.clone(),
                new_root,
                label.clone(),
                description.clone(),
                &all_xch,
                effective_fee,
            )?;
            let new_coin_id = update.new_coin_id;

            let pay = build_dig_payment_multi(
                w.addrs.iter().map(|a| &a.keys),
                change_ph,
                &cats,
                dig::COMMIT_DIG,
                launcher_id,
            )?;

            // 3) Combine into ONE bundle and sign atomically with ALL address keys.
            //    ATOMICITY: the DIG payment and the singleton spend ride in a single
            //    SpendBundle under one aggregated signature, so the mempool admits
            //    them all-or-nothing. This co-signing is the SOLE atomicity guarantee
            //    between the DIG payment and the anchor — these coin spends must never
            //    be split across bundles or pushed separately.
            let mut all = update.coin_spends;
            all.extend(pay);
            let signature = sign_coin_spends(&all, &w.signing_keys(), false)
                .map_err(|e| ChainError::Chain(format!("sign combined update+DIG bundle: {e}")))?;
            Ok((SpendBundle::new(all, signature), new_coin_id))
        };

        // Build the initial bundle (fee=0 if auto-estimating, or the caller's fee override).
        let (bundle, new_coin_id) = build_update_bundle(fee)?;

        // Auto-estimate fee when the caller supplied fee=0.
        // Fail-open: if estimation returns 0 or wallet can't cover it, use the fee-0 bundle.
        let (final_bundle, new_coin_id) = if fee == 0 {
            let est = self.chain.estimate_fee(&bundle, 60).await.unwrap_or(0);
            if est > 0 && w.xch_balance() >= est {
                // Rebuild + re-sign with the estimated fee.
                match build_update_bundle(est) {
                    Ok(rebuilt) => rebuilt,
                    Err(_) => (bundle, new_coin_id), // rebuild failed → fall back
                }
            } else {
                (bundle, new_coin_id) // empty mempool or insufficient XCH → fee 0
            }
        } else {
            (bundle, new_coin_id) // explicit fee override
        };

        // Bundle hash == conventional tx id; capture it BEFORE the bundle moves.
        let tx_id = final_bundle.name();
        self.chain.push(final_bundle).await?;
        Ok(UpdateOutcome { new_coin_id, tx_id })
    }

    /// Polls chain every 10 s until `coin_id` appears or the poll budget expires.
    /// Does NOT sleep after the final check, so `Pending` returns immediately
    /// when `timeout_secs` maps to a single poll (e.g. in unit tests).
    async fn confirm(&self, coin_id: Bytes32, timeout_secs: u64) -> Result<ConfirmState> {
        let deadline_polls = (timeout_secs / 10).max(1);
        for i in 0..deadline_polls {
            if let Some(rec) = self.chain.coin_record(coin_id).await? {
                return Ok(ConfirmState::Confirmed {
                    height: rec.confirmed_block_index,
                });
            }
            // Skip the sleep on the very last iteration so callers with a
            // budget of 1 poll (timeout_secs < 10) return immediately.
            if i + 1 < deadline_polls {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
        Ok(ConfirmState::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cat::dig_cat_puzzle_hash;
    use crate::coinset::mock::MockChain;
    use crate::coinset::CoinInfo;
    use crate::keys::{derive_indexed_keys, derive_wallet_keys};
    use chia_protocol::Coin;

    // Public BIP-39 test vector (NOT a real wallet).
    const ABANDON: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon art";

    // -----------------------------------------------------------------------
    // Test 1: balance returns the ScannedWallet's aggregate XCH balance.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn balance_sums_coins_at_owner_ph() {
        let mut mock = MockChain::default();
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let ph = keys.owner_puzzle_hash;
        mock.coins_by_ph.insert(
            ph,
            vec![
                Coin::new(Bytes32::default(), ph, 500_000),
                Coin::new(Bytes32::new([1u8; 32]), ph, 300_000),
            ],
        );
        // Scan to produce a ScannedWallet that has these coins at index 0.
        let anchor = CoinsetAnchor::new(mock);
        let w = anchor.scan(ABANDON).await.unwrap();
        let bal = anchor.balance(&w).await.unwrap();
        assert_eq!(bal, 800_000);
    }

    // -----------------------------------------------------------------------
    // Test 1b: dig_balance returns the ScannedWallet's aggregate DIG balance.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn dig_balance_sums_cat_coins_at_dig_ph() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let cat_ph = dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        let mut mock = MockChain::default();
        mock.coins_by_ph.insert(
            cat_ph,
            vec![
                Coin::new(Bytes32::default(), cat_ph, 60_000),
                Coin::new(Bytes32::new([1u8; 32]), cat_ph, 40_000),
            ],
        );
        let anchor = CoinsetAnchor::new(mock);
        let w = anchor.scan(ABANDON).await.unwrap();
        assert_eq!(anchor.dig_balance(&w).await.unwrap(), 100_000);
    }

    // -----------------------------------------------------------------------
    // Test 1c: balance and dig_balance aggregate across multiple HD indices.
    // Seeds XCH at index 0 + index 2, DIG at index 2; asserts the totals are
    // the cross-address sum. Mirrors balance_sums_coins_at_owner_ph but with
    // a multi-index mock wallet.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn balance_aggregates_across_hd_indices() {
        // Derive keys for indices 0 and 2.
        let indexed = derive_indexed_keys(ABANDON, 0..3).unwrap();
        let ph0 = indexed[0].owner_puzzle_hash;
        let ph2 = indexed[2].owner_puzzle_hash;
        let dig_ph2 = dig_cat_puzzle_hash(ph2);

        let mut mock = MockChain::default();
        // XCH at index 0: 500_000
        mock.coins_by_ph
            .insert(ph0, vec![Coin::new(Bytes32::default(), ph0, 500_000)]);
        // XCH at index 2: 300_000; DIG at index 2: 120_000
        mock.coins_by_ph
            .insert(ph2, vec![Coin::new(Bytes32::new([2u8; 32]), ph2, 300_000)]);
        mock.coins_by_ph.insert(
            dig_ph2,
            vec![Coin::new(Bytes32::new([3u8; 32]), dig_ph2, 120_000)],
        );

        let anchor = CoinsetAnchor::new(mock);
        let w = anchor.scan(ABANDON).await.unwrap();

        // Total XCH = 500_000 + 300_000 = 800_000
        assert_eq!(anchor.balance(&w).await.unwrap(), 800_000);
        // Total DIG = 120_000
        assert_eq!(anchor.dig_balance(&w).await.unwrap(), 120_000);
    }

    // -----------------------------------------------------------------------
    // Test 2: mint_empty_store now embeds a DIG payment in the SAME bundle, so a
    // wallet with XCH but NO DIG is blocked before any push (atomic: the mint
    // cannot ride without its DIG payment). This proves the DIG payment is wired
    // into the mint path. The happy path (a real DIG CAT reconstructed over
    // coinset + the combined signed bundle) is validated LIVE by the controller
    // and by the ignored `dig_cats_live_reconstruct` test — a valid DIG CAT
    // cannot be reconstructed offline without real CLVM lineage matching the
    // mainnet DIG asset id.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn mint_empty_store_blocks_without_dig() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let ph = keys.owner_puzzle_hash;
        // Plenty of XCH, but no DIG CAT coins at the DIG puzzle hash.
        let funding_coin = Coin::new(Bytes32::default(), ph, 1_000_000);
        mock.coins_by_ph.insert(ph, vec![funding_coin]);

        let anchor = CoinsetAnchor::new(mock);
        // Scan to get a ScannedWallet, then call mint with it.
        let w = anchor.scan(ABANDON).await.unwrap();
        let err = anchor
            .mint_empty_store(&w, None, None, 0)
            .await
            .unwrap_err();
        match err {
            crate::error::ChainError::Chain(msg) => {
                assert!(msg.contains("insufficient DIG"), "got: {msg}");
            }
            other => panic!("expected insufficient DIG, got {other:?}"),
        }

        // Nothing was pushed — the mint is atomic with its DIG payment.
        let pushed_count = anchor.chain.pushed.lock().unwrap().len();
        assert_eq!(
            pushed_count, 0,
            "expected no pushed bundle when DIG is short"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2b: multi-address mint — XCH from multiple addresses, no DIG.
    // Verifies:
    //   - signing_keys() covers all kept addresses (multi-address signing),
    //   - coins_with_keys_from_wallet aggregates XCH coins across addresses,
    //   - the coin pool spans multiple distinct puzzle hashes,
    //   - mint_empty_store fails with insufficient DIG (no DIG coins seeded),
    //     confirming the full multi-address XCH + DIG path is invoked.
    //
    // Note: full bundle validity (CLVM execution, signature check) requires real
    // chain data and is verified live in Task 5.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn mint_multi_address_signing_keys_cover_all_addresses() {
        // Seed XCH at index 0 and index 2 — no DIG anywhere.
        // (DIG coins in the mock would trigger dig_cats coin-record lookups that
        // fail on a bare MockChain; we test DIG gathering structurally here.)
        let indexed = derive_indexed_keys(ABANDON, 0..3).unwrap();
        let ph0 = indexed[0].owner_puzzle_hash;
        let ph2 = indexed[2].owner_puzzle_hash;

        let mut mock = MockChain::default();
        // XCH at index 0
        mock.coins_by_ph
            .insert(ph0, vec![Coin::new(Bytes32::default(), ph0, 5_000_000)]);
        // XCH at index 2
        mock.coins_by_ph.insert(
            ph2,
            vec![Coin::new(Bytes32::new([2u8; 32]), ph2, 3_000_000)],
        );

        let anchor = CoinsetAnchor::new(mock);
        let w = anchor.scan(ABANDON).await.unwrap();

        // Wallet must cover multiple addresses (indices 0 and 2 kept; index 1 empty).
        assert!(
            w.addrs.len() >= 2,
            "expected ≥2 addresses in scanned wallet"
        );
        // signing_keys covers all kept addresses.
        assert_eq!(w.signing_keys().len(), w.addrs.len());

        // The XCH coin pool spans multiple distinct puzzle hashes.
        let all_xch = crate::singleton::coins_with_keys_from_wallet(&w);
        let distinct_phs: std::collections::HashSet<Bytes32> =
            all_xch.iter().map(|c| c.owner_puzzle_hash).collect();
        assert!(
            distinct_phs.len() >= 2,
            "expected XCH coins from ≥2 puzzle hashes, got {distinct_phs:?}"
        );
        // All XCH coins are tagged with their address's synthetic_pk.
        for ck in &all_xch {
            assert_ne!(
                ck.synthetic_pk.to_bytes(),
                [0u8; 48],
                "synthetic_pk must be set"
            );
        }

        // mint_empty_store: no DIG coins anywhere → dig_cats_multi returns empty
        // → insufficient DIG. Proves the whole multi-address mint path is wired.
        let err = anchor
            .mint_empty_store(&w, None, None, 0)
            .await
            .unwrap_err();
        match err {
            crate::error::ChainError::Chain(msg) => {
                assert!(msg.contains("insufficient DIG"), "got: {msg}");
            }
            other => panic!("expected insufficient DIG error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Test 3a: confirm returns Confirmed when coin_record is present.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn confirm_returns_confirmed_when_record_present() {
        let mut mock = MockChain::default();
        let coin_id = Bytes32::new([42u8; 32]);
        mock.records.insert(
            coin_id,
            CoinInfo {
                coin: Coin::new(Bytes32::default(), Bytes32::default(), 0),
                spent: false,
                confirmed_block_index: 5_000,
                spent_block_index: 0,
            },
        );
        let anchor = CoinsetAnchor::new(mock);
        let state = anchor.confirm(coin_id, 60).await.unwrap();
        assert_eq!(state, ConfirmState::Confirmed { height: 5_000 });
    }

    // -----------------------------------------------------------------------
    // Test 3b: confirm returns Pending fast when no record and timeout_secs=1.
    // With timeout_secs=1: deadline_polls = max(1/10, 1) = 1.
    // The loop checks once, finds nothing, i+1 == deadline_polls → no sleep.
    // Returns Pending immediately (no 10 s hang).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn confirm_returns_pending_fast_with_no_record() {
        let mock = MockChain::default();
        let coin_id = Bytes32::new([99u8; 32]);
        let anchor = CoinsetAnchor::new(mock);
        let state = anchor.confirm(coin_id, 1).await.unwrap();
        assert_eq!(state, ConfirmState::Pending);
    }

    // update_root is NOT unit-tested here because it requires real singleton
    // lineage data for sync_datastore to walk. It is exercised by the live
    // integration test `build_update_live_no_broadcast` in singleton.rs.
}
