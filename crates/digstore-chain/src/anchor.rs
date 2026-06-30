//! High-level anchoring operations the CLI drives: mint an empty store, update
//! its root, and wait for confirmation — all over coinset. Ties together
//! key derivation, spend building/signing, lineage sync, and broadcast.

use crate::cat::{build_dig_payment_multi, dig_cats_multi};
use crate::coinset::ChainReads;
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
    /// gathers XCH across ALL HD addresses and signs with all keys. Broadcasts.
    /// `label`/`description` are written into the CHIP-0035 singleton metadata.
    ///
    /// **Minting is FREE of $DIG (#111):** the mint only launches the singleton +
    /// the XCH network fee — NO DIG payment is attached. The DIG payment is paid only
    /// on a commit / root-advance (a capsule); see [`ChainAnchor::update_root`].
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
    /// `dig_amount` is the DIG (base units) paid to the treasury — caller-resolved
    /// (dynamic USD-pegged amount, or the [`crate::dig::COMMIT_DIG`] default).
    #[allow(clippy::too_many_arguments)]
    async fn update_root(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        label: Option<String>,
        description: Option<String>,
        w: &ScannedWallet,
        fee: u64,
        dig_amount: u64,
    ) -> Result<UpdateOutcome>;
    /// Advance `launcher_id`'s root signed by a WRITER DELEGATE key (#17 deploy
    /// token), NOT the owner master seed. `writer` is the writer-delegate keys
    /// derived from the deploy-token seed; the store MUST already carry that
    /// writer's delegated puzzle (the owner pre-authorized it via
    /// `updateStoreOwnership` — the hub Teams "Deployer" flow). The wallet `w`
    /// still funds the XCH fee + the DIG payment (atomic in the same bundle, signed
    /// alongside the writer-authorized singleton spend). `label`/`description` are
    /// re-sent (the update REPLACES metadata).
    #[allow(clippy::too_many_arguments)]
    async fn update_root_writer(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        label: Option<String>,
        description: Option<String>,
        writer: &crate::keys::WalletKeys,
        w: &ScannedWallet,
        fee: u64,
        dig_amount: u64,
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
        // Build + sign the mint bundle (singleton launch + XCH fee, no $DIG; with fee
        // auto-estimate), then push. Minting a store is free of $DIG (#111).
        let built =
            build_mint_store_bundle(&self.chain as &dyn ChainReads, w, label, description, fee)
                .await?;
        // Bundle hash == conventional tx id; capture it BEFORE the bundle moves.
        let tx_id = built.bundle.name();
        self.chain.push(built.bundle).await?;
        Ok(MintOutcome {
            launcher_id: built.launcher_id,
            coin_id: built.coin_id,
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
        dig_amount: u64,
    ) -> Result<UpdateOutcome> {
        let built = build_advance_store_bundle(
            &self.chain as &dyn ChainReads,
            launcher_id,
            new_root,
            label,
            description,
            w,
            fee,
            dig_amount,
        )
        .await?;
        // Bundle hash == conventional tx id; capture it BEFORE the bundle moves.
        let tx_id = built.bundle.name();
        self.chain.push(built.bundle).await?;
        Ok(UpdateOutcome {
            new_coin_id: built.new_coin_id,
            tx_id,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_root_writer(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        label: Option<String>,
        description: Option<String>,
        writer: &crate::keys::WalletKeys,
        w: &ScannedWallet,
        fee: u64,
        dig_amount: u64,
    ) -> Result<UpdateOutcome> {
        let built = build_advance_store_writer_bundle(
            &self.chain as &dyn ChainReads,
            launcher_id,
            new_root,
            label,
            description,
            writer,
            w,
            fee,
            dig_amount,
        )
        .await?;
        let tx_id = built.bundle.name();
        self.chain.push(built.bundle).await?;
        Ok(UpdateOutcome {
            new_coin_id: built.new_coin_id,
            tx_id,
        })
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

// ===========================================================================
// Build-only (no-push) bundle builders — shared by the CLI anchor (which then
// pushes) AND the in-process wallet (which applies its own broadcast gate).
//
// These hold the SOLE copy of the bundle-assembly + fee auto-estimate logic, so the
// CLI and the wallet can never drift. The MINT bundle is the singleton launch + XCH
// fee ONLY — minting is FREE of $DIG (#111). The COMMIT (root-advance) bundle holds
// the atomic "singleton update + DIG payment, signed together under one aggregated
// signature" logic — the per-capsule $DIG payment rides with the new capsule, never
// split across bundles. Each returns a fully-signed SpendBundle but does NOT broadcast
// — pushing is the caller's gated decision.
// ===========================================================================

/// A built + signed mint bundle (singleton launch + XCH fee, NO $DIG — #111), not yet
/// pushed.
pub struct MintStoreBundle {
    pub bundle: SpendBundle,
    pub launcher_id: Bytes32, // == store_id
    pub coin_id: Bytes32,     // eve singleton coin to poll for confirmation
}

/// A built + signed root-advance bundle (singleton update + DIG payment), not yet pushed.
pub struct AdvanceStoreBundle {
    pub bundle: SpendBundle,
    pub new_coin_id: Bytes32,
}

/// Auto-estimate the fee for `bundle` when `requested_fee == 0`, returning the fee to
/// actually use. Fail-open: if estimation yields 0 or the wallet can't cover it, keep
/// fee 0 (the empty-mempool case). An explicit non-zero `requested_fee` is honoured.
async fn resolve_fee(
    chain: &dyn ChainReads,
    bundle: &SpendBundle,
    w: &ScannedWallet,
    requested_fee: u64,
) -> u64 {
    if requested_fee != 0 {
        return requested_fee;
    }
    let est = chain.estimate_fee(bundle, 60).await.unwrap_or(0);
    if est > 0 && w.xch_balance() >= est {
        est
    } else {
        0
    }
}

/// Build + sign the mint bundle for a store with `root = 0`, gathering XCH across ALL
/// scanned HD addresses and signing with all keys. Auto-estimates the fee when `fee == 0`.
/// Does NOT push — the caller broadcasts (CLI) or gates it (wallet).
///
/// **Minting a store is FREE of $DIG (#111, SYSTEM.md → "DIG CAT payment"):** this
/// bundle is the CHIP-0035 singleton launch + the XCH network fee ONLY — it carries NO
/// DIG-CAT payment. The DIG payment is attached only on a commit / root-advance (a
/// capsule), in [`build_advance_store_bundle`].
pub async fn build_mint_store_bundle(
    chain: &dyn ChainReads,
    w: &ScannedWallet,
    label: Option<String>,
    description: Option<String>,
    fee: u64,
) -> Result<MintStoreBundle> {
    // Index 0 is the store owner and change destination.
    let change_ph = w.addrs[0].keys.owner_puzzle_hash;

    // Build + sign the mint bundle (singleton launch + XCH fee, no $DIG) for a given fee.
    let build = |effective_fee: u64| -> Result<MintStoreBundle> {
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
        // Minting carries NO DIG payment (#111) — the singleton launch + XCH fee only.
        let all = mint.coin_spends;
        let signature = sign_coin_spends(&all, &w.signing_keys(), false)
            .map_err(|e| ChainError::Chain(format!("sign mint bundle: {e}")))?;
        Ok(MintStoreBundle {
            bundle: SpendBundle::new(all, signature),
            launcher_id,
            coin_id,
        })
    };

    let first = build(fee)?;
    let effective = resolve_fee(chain, &first.bundle, w, fee).await;
    if effective == fee {
        Ok(first)
    } else {
        // Rebuild with the estimated fee; fall back to the fee-0 bundle on failure.
        build(effective).or(Ok(first))
    }
}

/// Build + sign the atomic owner-authorized root-advance+DIG bundle for `launcher_id`
/// → `new_root`, gathering XCH + DIG across ALL scanned addresses. Syncs the live
/// singleton, auto-estimates the fee when `fee == 0`. Does NOT push.
#[allow(clippy::too_many_arguments)]
pub async fn build_advance_store_bundle(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
    new_root: Bytes32,
    label: Option<String>,
    description: Option<String>,
    w: &ScannedWallet,
    fee: u64,
    dig_amount: u64,
) -> Result<AdvanceStoreBundle> {
    let change_ph = w.addrs[0].keys.owner_puzzle_hash;
    let owner_pk = w.addrs[0].keys.synthetic_pk;
    let store = sync_datastore(chain, launcher_id).await?;
    let cats = dig_cats_multi(chain, w).await?;

    let build = |effective_fee: u64| -> Result<AdvanceStoreBundle> {
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
            dig_amount,
            launcher_id,
        )?;
        // ATOMICITY: DIG payment + singleton update in ONE co-signed bundle (see
        // build_mint_store_bundle) — admitted all-or-nothing, never split.
        let mut all = update.coin_spends;
        all.extend(pay);
        // ENFORCE the per-capsule $DIG payment (#130): a commit/root-advance MUST
        // carry the treasury payment. Fail CLOSED before signing so a builder bug
        // can never emit a FREE root-advance.
        crate::dig::verify_commit_pays_dig_treasury(&all)?;
        let signature = sign_coin_spends(&all, &w.signing_keys(), false)
            .map_err(|e| ChainError::Chain(format!("sign combined update+DIG bundle: {e}")))?;
        Ok(AdvanceStoreBundle {
            bundle: SpendBundle::new(all, signature),
            new_coin_id,
        })
    };

    let first = build(fee)?;
    let effective = resolve_fee(chain, &first.bundle, w, fee).await;
    if effective == fee {
        Ok(first)
    } else {
        build(effective).or(Ok(first))
    }
}

/// Build + sign the atomic WRITER-DELEGATE-authorized root-advance+DIG bundle (#17): the
/// singleton update is authorized by `writer`'s key (NOT the owner seed) while the wallet
/// `w` funds the XCH fee + DIG payment. The store MUST already carry `writer`'s delegated
/// puzzle. Does NOT push.
#[allow(clippy::too_many_arguments)]
pub async fn build_advance_store_writer_bundle(
    chain: &dyn ChainReads,
    launcher_id: Bytes32,
    new_root: Bytes32,
    label: Option<String>,
    description: Option<String>,
    writer: &crate::keys::WalletKeys,
    w: &ScannedWallet,
    fee: u64,
    dig_amount: u64,
) -> Result<AdvanceStoreBundle> {
    // Index 0 is the wallet's fee/change address (the writer authorizes the singleton;
    // the wallet still pays the XCH fee + the DIG payment).
    let change_ph = w.addrs[0].keys.owner_puzzle_hash;
    let fee_keys = crate::keys::WalletKeys {
        synthetic_sk: w.addrs[0].keys.synthetic_sk.clone(),
        synthetic_pk: w.addrs[0].keys.synthetic_pk,
        owner_puzzle_hash: w.addrs[0].keys.owner_puzzle_hash,
    };
    let store = sync_datastore(chain, launcher_id).await?;
    let cats = dig_cats_multi(chain, w).await?;
    // The deploy token is index-0-funded in CI; fee draws from the change address.
    let fee_coins: Vec<chia_protocol::Coin> = w.addrs[0].xch.clone();

    let build = |effective_fee: u64| -> Result<AdvanceStoreBundle> {
        let update = crate::singleton::build_update_unsigned_writer(
            writer.synthetic_pk,
            store.clone(),
            new_root,
            label.clone(),
            description.clone(),
            &fee_keys,
            &fee_coins,
            effective_fee,
        )?;
        let new_coin_id = update.new_coin_id;
        let pay = build_dig_payment_multi(
            w.addrs.iter().map(|a| &a.keys),
            change_ph,
            &cats,
            dig_amount,
            launcher_id,
        )?;
        // ONE bundle, signed by BOTH the writer key (singleton) and ALL wallet keys
        // (fee + DIG). ATOMICITY as above — never split or pushed separately.
        let mut all = update.coin_spends;
        all.extend(pay);
        // ENFORCE the per-capsule $DIG payment (#130): fail CLOSED before signing
        // if the writer-authorized commit bundle does not pay the treasury.
        crate::dig::verify_commit_pays_dig_treasury(&all)?;
        let mut signers = w.signing_keys();
        signers.push(writer.synthetic_sk.clone());
        let signature = sign_coin_spends(&all, &signers, false)
            .map_err(|e| ChainError::Chain(format!("sign writer update+DIG bundle: {e}")))?;
        Ok(AdvanceStoreBundle {
            bundle: SpendBundle::new(all, signature),
            new_coin_id,
        })
    };

    let first = build(fee)?;
    let effective = resolve_fee(chain, &first.bundle, w, fee).await;
    if effective == fee {
        Ok(first)
    } else {
        build(effective).or(Ok(first))
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
    // Test 2 (#111): minting a store is FREE of $DIG. A wallet with XCH but NO DIG
    // mints SUCCESSFULLY — the mint bundle is the singleton launch + XCH fee only,
    // never a DIG payment. (Pre-#111 the mint embedded a DIG payment and blocked
    // without DIG; that coupling is removed — only a commit/capsule pays $DIG.)
    // The full on-chain validity of the bundle is verified LIVE by the controller.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn mint_empty_store_succeeds_without_dig() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let ph = keys.owner_puzzle_hash;
        // Plenty of XCH, but no DIG CAT coins at the DIG puzzle hash.
        let funding_coin = Coin::new(Bytes32::default(), ph, 1_000_000);
        mock.coins_by_ph.insert(ph, vec![funding_coin]);

        let anchor = CoinsetAnchor::new(mock);
        // Scan to get a ScannedWallet, then call mint with it.
        let w = anchor.scan(ABANDON).await.unwrap();
        let out = anchor
            .mint_empty_store(&w, None, None, 0)
            .await
            .expect("mint must succeed without any DIG (minting is free of $DIG)");
        assert_ne!(out.launcher_id, Bytes32::default());
        // The mint was pushed (the mock push records it) — minting needs no DIG.
        let pushed_count = anchor.chain.pushed.lock().unwrap().len();
        assert_eq!(pushed_count, 1, "the DIG-free mint is pushed");
    }

    // -----------------------------------------------------------------------
    // Test 2b: multi-address mint — XCH from multiple addresses, no DIG.
    // Verifies:
    //   - signing_keys() covers all kept addresses (multi-address signing),
    //   - coins_with_keys_from_wallet aggregates XCH coins across addresses,
    //   - the coin pool spans multiple distinct puzzle hashes,
    //   - mint_empty_store SUCCEEDS with no DIG seeded (#111: minting is free of
    //     $DIG), confirming the full multi-address XCH mint path is invoked.
    //
    // Note: full bundle validity (CLVM execution, signature check) requires real
    // chain data and is verified live in Task 5.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn mint_multi_address_signing_keys_cover_all_addresses() {
        // Seed XCH at index 0 and index 2 — no DIG anywhere (minting needs none).
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

        // mint_empty_store: no DIG coins anywhere, yet the mint SUCCEEDS (#111 —
        // minting is free of $DIG). Proves the whole multi-address XCH mint path is
        // wired and never gathers/pays DIG on the mint.
        let out = anchor
            .mint_empty_store(&w, None, None, 0)
            .await
            .expect("multi-address mint must succeed without any DIG");
        assert_ne!(out.launcher_id, Bytes32::default());
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
                timestamp: 0,
                coinbase: false,
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

    // -----------------------------------------------------------------------
    // #111: MINT is FREE of $DIG; only COMMIT (a capsule) pays.
    //
    // The digstore mirror of chip35's `mint_bundle_has_no_dig_payment_but_commit_does`
    // (chip35 `core/tests/dig_capsule.rs`). The MINT bundle MUST NOT contain a DIG-CAT
    // payment to the treasury (mint = launch the singleton + XCH fee only); a COMMIT's
    // DIG payment MUST. We prove this by a keyless byte-signal: the DIG payment's CAT
    // spend commits to the treasury INNER puzzle hash (it appears in the spend's
    // serialized `puzzle_reveal || solution`), while mint/update singleton spends never
    // reference the treasury. This needs no on-chain CLVM/lineage (matching chip35).
    // -----------------------------------------------------------------------

    // The treasury-payment byte-signal helper is now the production
    // `crate::dig::bundle_pays_dig_treasury` (#130) — used both here and by the
    // commit-validation gate `verify_commit_pays_dig_treasury`. The test imports it
    // rather than re-defining a local copy (DRY: one keyless signal).
    use crate::dig::bundle_pays_dig_treasury;

    /// MINT must NOT pay $DIG (minting a store is free of $DIG — only the XCH fee +
    /// the 1-mojo singleton ride in the mint bundle), while a COMMIT's DIG payment
    /// DOES pay the treasury. This is the digstore side of task #111, pinned at the
    /// chain builder. The mint side uses a real built+signed bundle over a MockChain
    /// with XCH but NO DIG (which now SUCCEEDS — mint no longer requires DIG); the
    /// commit-payment side builds the canonical [`crate::cat::build_dig_store_payment`]
    /// over a keyless synthetic DIG `Cat` (a full owner-signed commit bundle needs
    /// real CLVM lineage, validated live).
    #[tokio::test]
    async fn mint_bundle_has_no_dig_payment_but_commit_does() {
        use crate::cat::build_dig_store_payment;
        use chia_wallet_sdk::driver::{Cat, CatInfo};

        let keys = derive_wallet_keys(ABANDON).unwrap();
        let ph = keys.owner_puzzle_hash;

        // --- MINT: XCH only, NO DIG anywhere. The mint must build + sign without a
        //     DIG payment, and the resulting bundle must not pay the treasury. ---
        let mut mock = MockChain::default();
        mock.coins_by_ph
            .insert(ph, vec![Coin::new(Bytes32::default(), ph, 1_000_000)]);
        let anchor = CoinsetAnchor::new(mock);
        let w = block_on_local(anchor.scan(ABANDON));
        let w = w.unwrap();
        let mint = block_on_local(build_mint_store_bundle(
            &anchor.chain as &dyn ChainReads,
            &w,
            Some("My Store".into()),
            None,
            1_000,
        ))
        .expect("mint must succeed without any DIG (mint is free of $DIG)");
        assert!(
            !bundle_pays_dig_treasury(&mint.bundle.coin_spends),
            "MINT must NOT pay $DIG to the treasury — minting a store is free of $DIG"
        );

        // --- COMMIT payment: the per-capsule DIG payment DOES pay the treasury. ---
        // A keyless synthetic DIG Cat (asset_id == DIG_ASSET_ID); selection/condition
        // construction does not need a lineage proof.
        let dig = Cat::new(
            Coin::new(
                Bytes32::from([5u8; 32]),
                Bytes32::from([6u8; 32]),
                1_000_000,
            ),
            None,
            CatInfo::new(crate::dig::DIG_ASSET_ID, None, ph),
        );
        let store_id = mint.launcher_id;
        let pay = build_dig_store_payment(keys.synthetic_pk, vec![dig], store_id, 100_000)
            .expect("build_dig_store_payment");
        assert!(
            bundle_pays_dig_treasury(&pay),
            "COMMIT (capsule creation) MUST pay the per-capsule $DIG price to the treasury"
        );
    }

    /// Block a future on a fresh current-thread runtime (these tests run under
    /// `#[tokio::test]`; nesting a `block_on` inside would panic, so build a
    /// dedicated runtime on a scoped thread for the inner async builder calls).
    fn block_on_local<F: std::future::Future + Send>(fut: F) -> F::Output
    where
        F::Output: Send,
    {
        std::thread::scope(|s| {
            s.spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build runtime")
                    .block_on(fut)
            })
            .join()
            .expect("join")
        })
    }
}
