//! High-level anchoring operations the CLI drives: mint an empty store, update
//! its root, and wait for confirmation — all over coinset. Ties together
//! key derivation, spend building/signing, lineage sync, and broadcast.

use crate::coinset::ChainReads;
use crate::error::Result;
use crate::keys::WalletKeys;
use crate::singleton::{build_mint, build_update, sync_datastore};
use chia_protocol::Bytes32;

#[derive(Clone, Debug)]
pub struct MintOutcome {
    pub launcher_id: Bytes32, // == store_id
    pub coin_id: Bytes32,     // eve singleton coin to poll for confirmation
}

#[derive(Clone, Debug)]
pub struct UpdateOutcome {
    pub new_coin_id: Bytes32, // new singleton coin to poll
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmState {
    Confirmed { height: u32 },
    Pending,
}

#[async_trait::async_trait]
pub trait ChainAnchor: Send + Sync {
    /// Spendable XCH (mojos) at the wallet's owner puzzle hash.
    async fn balance(&self, keys: &WalletKeys) -> Result<u64>;
    /// Spendable DIG (base units) at the wallet's DIG CAT puzzle hash.
    async fn dig_balance(&self, keys: &WalletKeys) -> Result<u64>;
    /// Mint an empty (root = 0) owner-only store; broadcast. Returns ids.
    async fn mint_empty_store(&self, keys: &WalletKeys, fee: u64) -> Result<MintOutcome>;
    /// Sync the current singleton for `launcher_id`, build+broadcast a root update.
    async fn update_root(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        keys: &WalletKeys,
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
    async fn balance(&self, keys: &WalletKeys) -> Result<u64> {
        let coins = self.chain.unspent_coins(keys.owner_puzzle_hash).await?;
        Ok(coins.iter().map(|c| c.amount).sum())
    }

    async fn dig_balance(&self, keys: &WalletKeys) -> Result<u64> {
        crate::cat::dig_balance(&self.chain as &dyn ChainReads, keys.owner_puzzle_hash).await
    }

    async fn mint_empty_store(&self, keys: &WalletKeys, fee: u64) -> Result<MintOutcome> {
        let unspent = self.chain.unspent_coins(keys.owner_puzzle_hash).await?;
        let built = build_mint(keys, &unspent, Bytes32::default(), fee)?;
        // Read coin_id and launcher_id BEFORE moving `built.bundle` into push.
        let coin_id = built.datastore.coin.coin_id();
        let launcher_id = built.launcher_id;
        self.chain.push(built.bundle).await?;
        Ok(MintOutcome {
            launcher_id,
            coin_id,
        })
    }

    async fn update_root(
        &self,
        launcher_id: Bytes32,
        new_root: Bytes32,
        keys: &WalletKeys,
        fee: u64,
    ) -> Result<UpdateOutcome> {
        // &self.chain is &C; coerces to &dyn ChainReads because C: ChainReads.
        let store = sync_datastore(&self.chain as &dyn ChainReads, launcher_id).await?;
        let unspent = self.chain.unspent_coins(keys.owner_puzzle_hash).await?;
        let built = build_update(keys, store, new_root, &unspent, fee)?;
        // Read new_coin_id BEFORE moving `built.bundle` into push.
        let new_coin_id = built.new_coin_id;
        self.chain.push(built.bundle).await?;
        Ok(UpdateOutcome { new_coin_id })
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
    use crate::coinset::mock::MockChain;
    use crate::coinset::CoinInfo;
    use crate::keys::derive_wallet_keys;
    use chia_protocol::Coin;

    // Public BIP-39 test vector (NOT a real wallet).
    const ABANDON: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon art";

    // -----------------------------------------------------------------------
    // Test 1: balance sums unspent coins at the owner puzzle hash.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn balance_sums_coins_at_owner_ph() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let ph = keys.owner_puzzle_hash;
        mock.coins_by_ph.insert(
            ph,
            vec![
                Coin::new(Bytes32::default(), ph, 500_000),
                Coin::new(Bytes32::new([1u8; 32]), ph, 300_000),
            ],
        );
        let anchor = CoinsetAnchor::new(mock);
        let bal = anchor.balance(&keys).await.unwrap();
        assert_eq!(bal, 800_000);
    }

    // -----------------------------------------------------------------------
    // Test 1b: dig_balance sums DIG CAT coins at the owner's DIG CAT ph.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn dig_balance_sums_cat_coins_at_dig_ph() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let cat_ph = crate::cat::dig_cat_puzzle_hash(keys.owner_puzzle_hash);
        mock.coins_by_ph.insert(
            cat_ph,
            vec![
                Coin::new(Bytes32::default(), cat_ph, 60_000),
                Coin::new(Bytes32::new([1u8; 32]), cat_ph, 40_000),
            ],
        );
        let anchor = CoinsetAnchor::new(mock);
        assert_eq!(anchor.dig_balance(&keys).await.unwrap(), 100_000);
    }

    // -----------------------------------------------------------------------
    // Test 2: mint_empty_store builds a bundle and records exactly one push.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn mint_empty_store_pushes_one_bundle() {
        let keys = derive_wallet_keys(ABANDON).unwrap();
        let mut mock = MockChain::default();
        let ph = keys.owner_puzzle_hash;
        // Provide a synthetic funding coin with enough mojos (mint needs fee+1 minimum).
        let funding_coin = Coin::new(Bytes32::default(), ph, 1_000_000);
        mock.coins_by_ph.insert(ph, vec![funding_coin]);

        let anchor = CoinsetAnchor::new(mock);
        let outcome = anchor.mint_empty_store(&keys, 0).await.unwrap();

        // launcher_id must be non-default (it's a real hash from the mint).
        assert_ne!(
            outcome.launcher_id,
            Bytes32::default(),
            "launcher_id should be non-default"
        );

        // Exactly one SpendBundle must have been pushed.
        let pushed_count = anchor.chain.pushed.lock().unwrap().len();
        assert_eq!(pushed_count, 1, "expected exactly one pushed bundle");
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
