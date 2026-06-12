//! Coinset.org access behind a small trait so anchoring logic is testable
//! without a network. Real impl wraps `chia_sdk_coinset::CoinsetClient`.

use crate::error::{ChainError, Result};
use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use chia_sdk_coinset::ChiaRpcClient;

/// A confirmed coin record (subset of coinset's CoinRecord).
#[derive(Clone, Debug)]
pub struct CoinInfo {
    pub coin: Coin,
    pub spent: bool,
    pub confirmed_block_index: u32,
    pub spent_block_index: u32,
}

/// Classify a `get_coin_record_by_name` response into present / absent / error.
///
/// coinset returns `success = false` with a `"…not found"` error when the coin is
/// not (yet) on-chain — the NORMAL transient state while a freshly-pushed tx sits
/// in the mempool. That MUST be treated as "no record yet" so confirmation polling
/// keeps waiting, NOT as a hard chain error (otherwise `confirm` aborts on the very
/// first poll and a real mint/update can never confirm). Any other `success = false`
/// is a genuine RPC failure and is surfaced.
fn classify_coin_record(
    success: bool,
    error: Option<String>,
    mapped: Option<CoinInfo>,
) -> Result<Option<CoinInfo>> {
    if success {
        return Ok(mapped);
    }
    let msg = error.unwrap_or_default();
    if msg.to_lowercase().contains("not found") {
        return Ok(None);
    }
    Err(ChainError::Chain(format!(
        "get_coin_record_by_name failed: {msg:?}"
    )))
}

/// Minimal chain interface anchoring needs (reads + broadcast).
#[async_trait::async_trait]
pub trait ChainReads: Send + Sync {
    async fn unspent_coins(&self, puzzle_hash: Bytes32) -> Result<Vec<Coin>>;
    async fn coin_record(&self, name: Bytes32) -> Result<Option<CoinInfo>>;
    async fn coin_spend(&self, coin_id: Bytes32, spent_height: u32) -> Result<Option<CoinSpend>>;
    async fn peak_height(&self) -> Result<u32>;
    async fn push(&self, bundle: SpendBundle) -> Result<()>;
}

/// Production impl over coinset.org.
pub struct Coinset {
    client: chia_sdk_coinset::CoinsetClient,
}

impl Coinset {
    pub fn mainnet() -> Self {
        Self {
            client: chia_sdk_coinset::CoinsetClient::mainnet(),
        }
    }

    pub fn with_url(base_url: String) -> Self {
        Self {
            client: chia_sdk_coinset::CoinsetClient::new(base_url),
        }
    }
}

#[async_trait::async_trait]
impl ChainReads for Coinset {
    async fn unspent_coins(&self, puzzle_hash: Bytes32) -> Result<Vec<Coin>> {
        let resp = self
            .client
            .get_coin_records_by_puzzle_hashes(vec![puzzle_hash], None, None, Some(false))
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_records_by_puzzle_hashes: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_coin_records_by_puzzle_hashes failed: {:?}",
                resp.error
            )));
        }

        let coin_records = resp.coin_records.ok_or_else(|| {
            ChainError::Chain(
                "get_coin_records_by_puzzle_hashes: success=true but coin_records absent"
                    .to_string(),
            )
        })?;
        let coins = coin_records
            .into_iter()
            .filter(|cr| !cr.spent)
            .map(|cr| cr.coin)
            .collect();

        Ok(coins)
    }

    async fn coin_record(&self, name: Bytes32) -> Result<Option<CoinInfo>> {
        let resp = self
            .client
            .get_coin_record_by_name(name)
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_record_by_name: {e}")))?;

        let mapped = resp.coin_record.map(|cr| CoinInfo {
            coin: cr.coin,
            spent: cr.spent,
            confirmed_block_index: cr.confirmed_block_index,
            spent_block_index: cr.spent_block_index,
        });
        classify_coin_record(resp.success, resp.error, mapped)
    }

    async fn coin_spend(&self, coin_id: Bytes32, spent_height: u32) -> Result<Option<CoinSpend>> {
        let resp = self
            .client
            .get_puzzle_and_solution(coin_id, Some(spent_height))
            .await
            .map_err(|e| ChainError::Chain(format!("get_puzzle_and_solution: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_puzzle_and_solution failed: {:?}",
                resp.error
            )));
        }

        Ok(resp.coin_solution)
    }

    async fn peak_height(&self) -> Result<u32> {
        let resp = self
            .client
            .get_blockchain_state()
            .await
            .map_err(|e| ChainError::Chain(format!("get_blockchain_state: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_blockchain_state failed: {:?}",
                resp.error
            )));
        }

        let state = resp.blockchain_state.ok_or_else(|| {
            ChainError::Chain("get_blockchain_state: no blockchain_state in response".to_string())
        })?;

        Ok(state.peak.height)
    }

    async fn push(&self, bundle: SpendBundle) -> Result<()> {
        let resp = self
            .client
            .push_tx(bundle)
            .await
            .map_err(|e| ChainError::Chain(format!("push_tx: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "push_tx rejected: status={} error={:?}",
                resp.status, resp.error
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory mock for testing anchoring logic offline.
    #[derive(Default)]
    pub(crate) struct MockChain {
        pub coins_by_ph: HashMap<Bytes32, Vec<Coin>>,
        pub records: HashMap<Bytes32, CoinInfo>,
        pub spends: HashMap<Bytes32, CoinSpend>,
        pub peak: u32,
        pub pushed: Mutex<Vec<SpendBundle>>,
    }

    #[async_trait::async_trait]
    impl ChainReads for MockChain {
        async fn unspent_coins(&self, ph: Bytes32) -> Result<Vec<Coin>> {
            Ok(self.coins_by_ph.get(&ph).cloned().unwrap_or_default())
        }

        async fn coin_record(&self, name: Bytes32) -> Result<Option<CoinInfo>> {
            Ok(self.records.get(&name).cloned())
        }

        async fn coin_spend(&self, coin_id: Bytes32, _h: u32) -> Result<Option<CoinSpend>> {
            // mock returns the spend by coin_id only; spent_height is ignored
            Ok(self.spends.get(&coin_id).cloned())
        }

        async fn peak_height(&self) -> Result<u32> {
            Ok(self.peak)
        }

        async fn push(&self, bundle: SpendBundle) -> Result<()> {
            self.pushed
                .lock()
                .expect("MockChain pushed mutex poisoned")
                .push(bundle);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockChain;
    use super::*;

    #[tokio::test]
    async fn mock_unspent_returns_empty_for_unknown_ph() {
        let m = MockChain::default();
        let ph = Bytes32::default();
        let coins = m.unspent_coins(ph).await.unwrap();
        assert!(coins.is_empty());
    }

    #[tokio::test]
    async fn mock_unspent_and_push_roundtrip() {
        let mut m = MockChain::default();
        let ph = Bytes32::default();
        let parent = Bytes32::from([1u8; 32]);
        let coin = Coin::new(parent, ph, 1_000);
        m.coins_by_ph.insert(ph, vec![coin]);

        let found = m.unspent_coins(ph).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].amount, 1_000);

        // push records the bundle (empty bundle via aggregate)
        let bundle = SpendBundle::aggregate(&[]);
        m.push(bundle).await.unwrap();
        let pushed = m.pushed.lock().unwrap();
        assert_eq!(pushed.len(), 1);
    }

    #[tokio::test]
    async fn mock_peak_height() {
        let m = MockChain {
            peak: 6_515_821,
            ..Default::default()
        };
        assert_eq!(m.peak_height().await.unwrap(), 6_515_821);
    }

    #[tokio::test]
    async fn mock_coin_record_none_for_unknown() {
        let m = MockChain::default();
        let name = Bytes32::from([0xab; 32]);
        assert!(m.coin_record(name).await.unwrap().is_none());
    }

    // Regression: coinset reports a not-yet-confirmed (mempool) coin as
    // success=false + a "…not found" error. That MUST map to Ok(None) so
    // `confirm` keeps polling, not to a chain error that aborts confirmation on
    // the first poll. (Found by a real mainnet init: the mint broadcast fine but
    // confirmation died with `get_coin_record_by_name failed: ... not found`.)
    #[test]
    fn classify_not_found_is_pending_not_error() {
        let r = classify_coin_record(false, Some("Coin record 0xabc not found".into()), None);
        assert!(matches!(r, Ok(None)), "not-found must be Ok(None), got {r:?}");
    }

    #[test]
    fn classify_real_rpc_error_propagates() {
        let r = classify_coin_record(false, Some("internal server error".into()), None);
        assert!(matches!(r, Err(ChainError::Chain(_))));
    }

    #[test]
    fn classify_success_passes_record_through() {
        let info = CoinInfo {
            coin: Coin::new(Bytes32::default(), Bytes32::default(), 1),
            spent: false,
            confirmed_block_index: 100,
            spent_block_index: 0,
        };
        let got = classify_coin_record(true, None, Some(info)).unwrap();
        assert_eq!(got.map(|c| c.confirmed_block_index), Some(100));
        assert!(classify_coin_record(true, None, None).unwrap().is_none());
    }
}
