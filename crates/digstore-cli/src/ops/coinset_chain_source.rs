//! [`CoinsetChainSource`] — a coinset-backed adapter presenting digs' existing
//! [`ChainReads`] transport as the ecosystem-canonical
//! [`dig_chainsource_interface::ChainSource`] trait (#1349).
//!
//! ## Why this exists (a TRANSPORT adapter only)
//!
//! `dig_store::get_store_status` reads chain state through the one canonical
//! [`ChainSource`](dig_chainsource_interface::ChainSource) trait. digs already owns a hardened
//! Chia read transport ([`digstore_chain::coinset::Coinset`], with retry/timeout for coinset.org's
//! transient truncation, #84) behind its OWN [`ChainReads`](digstore_chain::coinset::ChainReads)
//! trait — a *different* trait. Rather than pull in a heavyweight peer-connecting provider just to
//! run one read subcommand, this adapter bridges the two: it maps digs' `ChainReads` results onto
//! `ChainSource`'s result shapes and preserves the fail-closed contract.
//!
//! It supplies ONLY transport reads. The money-critical singleton lineage walk that turns those
//! reads into a `StoreStatus` lives entirely inside `dig-store` (NC-9, already triple-gated) — this
//! adapter never interprets lineage, never decides Live/Melted, and holds no keys.
//!
//! ## Fail-closed mapping (the soundness crux, mirrored from the trait)
//!
//! - `Ok(None)` / an empty answer = the transport reliably reported genuine absence. Safe.
//! - `Err(_)` = the transport could NOT answer (network/parse). The consumer MUST fail closed; a
//!   transport failure is NEVER degraded to "absent". Every `ChainReads` error therefore maps to
//!   [`ChainSourceError::Transport`], never to `Ok(None)`.
//!
//! ## Scope: only what `get_store_status` calls
//!
//! `get_store_status` exercises exactly three reads — [`coin_spend`](ChainSource::coin_spend),
//! [`coin_record`](ChainSource::coin_record), and [`peak_height`](ChainSource::peak_height). The
//! remaining trait methods are not on that path; they return
//! [`ChainSourceError::Unsupported`] (fail closed) rather than a misleading empty answer, so any
//! future caller that reaches for them gets a loud "not provided by this adapter" instead of a
//! silent wrong absence.

use chia_protocol::{Bytes32, CoinSpend};
use dig_chainsource_interface::{ChainSource, ChainSourceError, CoinRecord};
use digstore_chain::coinset::{ChainReads, CoinInfo};

/// Adapts any digs [`ChainReads`] transport into the canonical [`ChainSource`] trait for
/// `dig_store::get_store_status`. See the module docs for the fail-closed contract.
///
/// Generic over the transport so the production [`Coinset`](digstore_chain::coinset::Coinset) and an
/// in-memory test double share one code path. Because `ChainReads` is async and `ChainSource` is
/// synchronous, the adapter owns a single-threaded Tokio runtime and drives each read to completion
/// on it; `get_store_status` runs on a plain sync call stack (no ambient runtime), so this never
/// nests a `block_on`.
pub struct CoinsetChainSource<C: ChainReads> {
    chain: C,
    runtime: tokio::runtime::Runtime,
}

impl<C: ChainReads> CoinsetChainSource<C> {
    /// Wraps `chain`, building the dedicated current-thread runtime used to drive its async reads.
    ///
    /// # Errors
    ///
    /// Returns a [`ChainSourceError::Transport`] if the Tokio runtime cannot be built.
    pub fn new(chain: C) -> Result<Self, ChainSourceError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ChainSourceError::Transport(format!("tokio runtime: {e}")))?;
        Ok(Self { chain, runtime })
    }
}

/// Maps digs' [`CoinInfo`] onto the canonical [`CoinRecord`], following the "None means not known"
/// convention: a zero confirmed height/timestamp is coinset's "unknown" sentinel, surfaced as
/// `None`, and a spent height is present only when the coin is actually spent.
fn to_coin_record(info: CoinInfo) -> CoinRecord {
    CoinRecord {
        coin: info.coin,
        confirmed_height: (info.confirmed_block_index != 0).then_some(info.confirmed_block_index),
        spent_height: info.spent.then_some(info.spent_block_index),
        timestamp: (info.timestamp != 0).then_some(info.timestamp),
        coinbase: info.coinbase,
    }
}

impl<C: ChainReads> ChainSource for CoinsetChainSource<C> {
    type Error = ChainSourceError;

    fn coin_record(&self, coin_id: Bytes32) -> Result<Option<CoinRecord>, Self::Error> {
        self.runtime
            .block_on(self.chain.coin_record(coin_id))
            .map(|opt| opt.map(to_coin_record))
            .map_err(|e| ChainSourceError::Transport(e.to_string()))
    }

    fn coin_spend(&self, coin_id: Bytes32) -> Result<Option<CoinSpend>, Self::Error> {
        // digs' `coin_spend` needs the spend height, which the canonical trait doesn't take, so we
        // first read the coin record to learn it. This also encodes the correct absence semantics:
        // an unknown coin has no spend (`Ok(None)`), and an UNSPENT coin has no spend YET
        // (`Ok(None)`) — the latter is exactly how the lineage walk detects a live tip. A transport
        // failure on either read fails closed (`Err`), never a false `None`.
        let info = match self
            .runtime
            .block_on(self.chain.coin_record(coin_id))
            .map_err(|e| ChainSourceError::Transport(e.to_string()))?
        {
            Some(info) => info,
            None => return Ok(None),
        };
        if !info.spent {
            return Ok(None);
        }
        self.runtime
            .block_on(self.chain.coin_spend(coin_id, info.spent_block_index))
            .map_err(|e| ChainSourceError::Transport(e.to_string()))
    }

    fn peak_height(&self) -> Result<Option<u32>, Self::Error> {
        self.runtime
            .block_on(self.chain.peak_height())
            .map(Some)
            .map_err(|e| ChainSourceError::Transport(e.to_string()))
    }

    // --- Not on the `get_store_status` read path: fail closed, never a silent empty answer. ------

    fn coin_records_by_puzzle_hash(
        &self,
        _puzzle_hash: Bytes32,
        _include_spent: bool,
    ) -> Result<Vec<CoinRecord>, Self::Error> {
        Err(ChainSourceError::Unsupported("coin_records_by_puzzle_hash"))
    }

    fn coin_records_by_parent(
        &self,
        _parent_coin_id: Bytes32,
    ) -> Result<Vec<CoinRecord>, Self::Error> {
        Err(ChainSourceError::Unsupported("coin_records_by_parent"))
    }

    fn resolve_singleton_lineage(
        &self,
        _launcher_id: Bytes32,
    ) -> Result<Option<dig_chainsource_interface::SingletonLineage>, Self::Error> {
        Err(ChainSourceError::Unsupported("resolve_singleton_lineage"))
    }

    fn block_timestamp(&self, _height: u32) -> Result<Option<u64>, Self::Error> {
        Err(ChainSourceError::Unsupported("block_timestamp"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chia_protocol::{Coin, SpendBundle};
    use digstore_chain::error::ChainError;
    use digstore_chain::Result as ChainResult;

    fn id(byte: u8) -> Bytes32 {
        Bytes32::new([byte; 32])
    }

    fn sample_coin() -> Coin {
        Coin::new(id(0xaa), id(0xbb), 1)
    }

    fn sample_spend() -> CoinSpend {
        CoinSpend::new(sample_coin(), vec![1u8, 2, 3].into(), vec![4u8, 5].into())
    }

    /// A hand-driven [`ChainReads`] double: each read returns a queued canned outcome so the
    /// adapter's mapping + fail-closed behaviour can be asserted without a network.
    #[derive(Default)]
    struct FakeChain {
        coin_record: Option<ChainResult<Option<CoinInfo>>>,
        coin_spend: Option<ChainResult<Option<CoinSpend>>>,
        peak: Option<ChainResult<u32>>,
    }

    #[async_trait]
    impl ChainReads for FakeChain {
        async fn unspent_coins(&self, _puzzle_hash: Bytes32) -> ChainResult<Vec<Coin>> {
            Ok(Vec::new())
        }
        async fn unspent_coins_by_hint(&self, _hint: Bytes32) -> ChainResult<Vec<Coin>> {
            Ok(Vec::new())
        }
        async fn coin_records_by_puzzle_hash(
            &self,
            _puzzle_hash: Bytes32,
            _include_spent: bool,
        ) -> ChainResult<Vec<digstore_chain::coinset::CoinRecord>> {
            Ok(Vec::new())
        }
        async fn coin_record(&self, _name: Bytes32) -> ChainResult<Option<CoinInfo>> {
            match self.coin_record.as_ref().expect("coin_record queued") {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(ChainError::Chain(e.to_string())),
            }
        }
        async fn coin_spend(
            &self,
            _coin_id: Bytes32,
            _spent_height: u32,
        ) -> ChainResult<Option<CoinSpend>> {
            match self.coin_spend.as_ref().expect("coin_spend queued") {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(ChainError::Chain(e.to_string())),
            }
        }
        async fn peak_height(&self) -> ChainResult<u32> {
            match self.peak.as_ref().expect("peak queued") {
                Ok(v) => Ok(*v),
                Err(e) => Err(ChainError::Chain(e.to_string())),
            }
        }
        async fn push(&self, _bundle: SpendBundle) -> ChainResult<()> {
            Ok(())
        }
        async fn estimate_fee(&self, _bundle: &SpendBundle, _target_secs: u64) -> ChainResult<u64> {
            Ok(0)
        }
    }

    fn adapter(chain: FakeChain) -> CoinsetChainSource<FakeChain> {
        CoinsetChainSource::new(chain).expect("runtime")
    }

    #[test]
    fn coin_record_maps_spent_coin_fields() {
        let info = CoinInfo {
            coin: sample_coin(),
            spent: true,
            confirmed_block_index: 100,
            spent_block_index: 150,
            timestamp: 1_700_000_000,
            coinbase: false,
        };
        let chain = FakeChain {
            coin_record: Some(Ok(Some(info))),
            ..Default::default()
        };
        let record = adapter(chain).coin_record(id(1)).unwrap().unwrap();
        assert_eq!(record.coin, sample_coin());
        assert_eq!(record.confirmed_height, Some(100));
        assert_eq!(record.spent_height, Some(150));
        assert_eq!(record.timestamp, Some(1_700_000_000));
        assert!(record.is_spent());
    }

    #[test]
    fn coin_record_maps_unspent_and_zero_sentinels_to_none() {
        let info = CoinInfo {
            coin: sample_coin(),
            spent: false,
            confirmed_block_index: 0,
            spent_block_index: 0,
            timestamp: 0,
            coinbase: false,
        };
        let chain = FakeChain {
            coin_record: Some(Ok(Some(info))),
            ..Default::default()
        };
        let record = adapter(chain).coin_record(id(1)).unwrap().unwrap();
        assert_eq!(record.confirmed_height, None);
        assert_eq!(record.spent_height, None);
        assert_eq!(record.timestamp, None);
        assert!(!record.is_spent());
    }

    #[test]
    fn coin_record_absence_is_ok_none() {
        let chain = FakeChain {
            coin_record: Some(Ok(None)),
            ..Default::default()
        };
        assert_eq!(adapter(chain).coin_record(id(1)).unwrap(), None);
    }

    #[test]
    fn coin_record_transport_failure_fails_closed() {
        let chain = FakeChain {
            coin_record: Some(Err(ChainError::Chain("boom".into()))),
            ..Default::default()
        };
        let err = adapter(chain).coin_record(id(1)).unwrap_err();
        assert!(matches!(err, ChainSourceError::Transport(_)));
    }

    #[test]
    fn coin_spend_unknown_coin_is_ok_none() {
        let chain = FakeChain {
            coin_record: Some(Ok(None)),
            ..Default::default()
        };
        assert_eq!(adapter(chain).coin_spend(id(1)).unwrap(), None);
    }

    #[test]
    fn coin_spend_unspent_coin_is_ok_none_without_fetching_spend() {
        // An unspent tip has no spend yet — the walk relies on this returning `Ok(None)` (Live),
        // and the underlying `coin_spend` must NOT be consulted (none is queued, so a call panics).
        let info = CoinInfo {
            coin: sample_coin(),
            spent: false,
            confirmed_block_index: 100,
            spent_block_index: 0,
            timestamp: 0,
            coinbase: false,
        };
        let chain = FakeChain {
            coin_record: Some(Ok(Some(info))),
            ..Default::default()
        };
        assert_eq!(adapter(chain).coin_spend(id(1)).unwrap(), None);
    }

    #[test]
    fn coin_spend_spent_coin_returns_the_spend() {
        let info = CoinInfo {
            coin: sample_coin(),
            spent: true,
            confirmed_block_index: 100,
            spent_block_index: 150,
            timestamp: 0,
            coinbase: false,
        };
        let chain = FakeChain {
            coin_record: Some(Ok(Some(info))),
            coin_spend: Some(Ok(Some(sample_spend()))),
            ..Default::default()
        };
        assert_eq!(
            adapter(chain).coin_spend(id(1)).unwrap(),
            Some(sample_spend())
        );
    }

    #[test]
    fn coin_spend_transport_failure_on_record_fails_closed() {
        let chain = FakeChain {
            coin_record: Some(Err(ChainError::Chain("boom".into()))),
            ..Default::default()
        };
        assert!(matches!(
            adapter(chain).coin_spend(id(1)).unwrap_err(),
            ChainSourceError::Transport(_)
        ));
    }

    #[test]
    fn peak_height_wraps_value_and_maps_error() {
        let ok = FakeChain {
            peak: Some(Ok(1234)),
            ..Default::default()
        };
        assert_eq!(adapter(ok).peak_height().unwrap(), Some(1234));

        let err = FakeChain {
            peak: Some(Err(ChainError::Chain("down".into()))),
            ..Default::default()
        };
        assert!(matches!(
            adapter(err).peak_height().unwrap_err(),
            ChainSourceError::Transport(_)
        ));
    }

    #[test]
    fn off_path_methods_fail_closed_as_unsupported() {
        let source = adapter(FakeChain::default());
        assert!(matches!(
            source
                .coin_records_by_puzzle_hash(id(1), false)
                .unwrap_err(),
            ChainSourceError::Unsupported(_)
        ));
        assert!(matches!(
            source.coin_records_by_parent(id(1)).unwrap_err(),
            ChainSourceError::Unsupported(_)
        ));
        assert!(matches!(
            source.resolve_singleton_lineage(id(1)).unwrap_err(),
            ChainSourceError::Unsupported(_)
        ));
        assert!(matches!(
            source.block_timestamp(1).unwrap_err(),
            ChainSourceError::Unsupported(_)
        ));
    }
}
