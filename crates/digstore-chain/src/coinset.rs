//! Coinset.org access behind a small trait so anchoring logic is testable
//! without a network. Real impl wraps `chia_sdk_coinset::CoinsetClient`.

use crate::error::{ChainError, Result};
use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use chia_sdk_coinset::ChiaRpcClient;

/// A confirmed coin record — the crate's mirror of coinset's `CoinRecord`.
///
/// Carries the full set of fields downstream parity features need:
/// `spent`/`spent_block_index` (spent-coin enumeration + tx history removals),
/// `confirmed_block_index` (confirmation polling + history adds), `timestamp`
/// (history ordering / human-readable dates), and `coinbase` (distinguishing
/// reward coins). Mapping to a crate-local struct keeps `chia_sdk_coinset` from
/// leaking into the public API.
#[derive(Clone, Debug)]
pub struct CoinInfo {
    pub coin: Coin,
    pub spent: bool,
    pub confirmed_block_index: u32,
    pub spent_block_index: u32,
    /// Unix timestamp of the block that confirmed the coin (0 if unknown).
    pub timestamp: u64,
    /// True if this coin is a block reward (farmer/pool coinbase).
    pub coinbase: bool,
}

/// The crate's confirmed-coin-record type. Alias of [`CoinInfo`] so callers and
/// the plan's `coin_records_by_puzzle_hash(...) -> Result<Vec<CoinRecord>>`
/// signature read naturally without exposing `chia_sdk_coinset::CoinRecord`.
pub type CoinRecord = CoinInfo;

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
    if is_not_found(&msg) {
        return Ok(None);
    }
    Err(ChainError::Chain(format!(
        "get_coin_record_by_name failed: {msg:?}"
    )))
}

/// coinset reports an absent coin/spend as `success = false` + a `"…not found"`
/// error. That is the NORMAL transient state while a freshly-pushed tx sits in
/// the mempool (no on-chain record / no solution yet), not a hard chain error.
fn is_not_found(msg: &str) -> bool {
    msg.to_lowercase().contains("not found")
}

/// Classify a `get_puzzle_and_solution` response into present / absent / error,
/// mirroring [`classify_coin_record`]: a `"…not found"` (the coin is not yet
/// on-chain / has no recorded solution) maps to `Ok(None)`; any other
/// `success = false` is a genuine RPC failure and is surfaced.
fn classify_coin_spend(
    success: bool,
    error: Option<String>,
    mapped: Option<CoinSpend>,
) -> Result<Option<CoinSpend>> {
    if success {
        return Ok(mapped);
    }
    let msg = error.unwrap_or_default();
    if is_not_found(&msg) {
        return Ok(None);
    }
    Err(ChainError::Chain(format!(
        "get_puzzle_and_solution failed: {msg:?}"
    )))
}

/// Builds the JSON body for a `get_fee_estimate` POST request.
///
/// Extracted so that the serialization logic can be unit-tested without a network.
fn build_fee_estimate_body(
    bundle: &SpendBundle,
    target_secs: u64,
    spend_count: usize,
) -> serde_json::Value {
    serde_json::json!({
        "spend_bundle": {
            "coin_spends": bundle.coin_spends.iter().map(|cs| {
                serde_json::json!({
                    "coin": {
                        "amount": cs.coin.amount,
                        "parent_coin_info": format!("0x{}", hex::encode(cs.coin.parent_coin_info.to_bytes())),
                        "puzzle_hash": format!("0x{}", hex::encode(cs.coin.puzzle_hash.to_bytes())),
                    },
                    "puzzle_reveal": format!("0x{}", hex::encode(cs.puzzle_reveal.to_vec())),
                    "solution": format!("0x{}", hex::encode(cs.solution.to_vec())),
                })
            }).collect::<Vec<serde_json::Value>>(),
            "aggregated_signature": format!("0x{}", hex::encode(bundle.aggregated_signature.to_bytes())),
        },
        "target_times": [target_secs],
        "spend_count": spend_count,
    })
}

/// Parses `estimates[0]` from a `get_fee_estimate` JSON response.
///
/// Returns 0 on any failure (success=false, missing field, wrong type) — fail-open.
fn parse_fee_estimate_response(json: &serde_json::Value) -> u64 {
    if !json
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return 0;
    }
    json.get("estimates")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Map coinset's `CoinRecord` to the crate-local [`CoinInfo`]/[`CoinRecord`].
///
/// One place to translate so every query method (`coin_record`,
/// `coin_records_by_puzzle_hash`, …) stays consistent and `chia_sdk_coinset`'s
/// type never escapes into the public API.
fn map_coin_record(cr: chia_sdk_coinset::CoinRecord) -> CoinInfo {
    CoinInfo {
        coin: cr.coin,
        spent: cr.spent,
        confirmed_block_index: cr.confirmed_block_index,
        spent_block_index: cr.spent_block_index,
        timestamp: cr.timestamp,
        coinbase: cr.coinbase,
    }
}

/// Minimal chain interface anchoring needs (reads + broadcast).
#[async_trait::async_trait]
pub trait ChainReads: Send + Sync {
    async fn unspent_coins(&self, puzzle_hash: Bytes32) -> Result<Vec<Coin>>;

    /// Unspent coins that carry `hint` as a memo hint.
    ///
    /// Gateway query for hint-indexed discovery: the digstore owner-hint locates a
    /// user's stores (launcher coins), and CAT/NFT enumeration finds assets hinted to
    /// a wallet puzzle hash. Wraps coinset's `get_coin_records_by_hint` (confirmed
    /// available in chia-sdk-coinset 0.30) with `include_spent_coins = false`, then
    /// keeps only currently-unspent records.
    async fn unspent_coins_by_hint(&self, hint: Bytes32) -> Result<Vec<Coin>>;

    /// All coin records at `puzzle_hash`, optionally including already-spent coins.
    ///
    /// The foundation for tx history (adds + removes across an address) and
    /// spent-coin enumeration. Wraps coinset's `get_coin_records_by_puzzle_hash`;
    /// `include_spent` is forwarded as `include_spent_coins` so a single call can
    /// fetch the full lifetime of an address (`true`) or just its live coins
    /// (`false`). Returns the crate's [`CoinRecord`] (carrying spent/height/timestamp).
    async fn coin_records_by_puzzle_hash(
        &self,
        puzzle_hash: Bytes32,
        include_spent: bool,
    ) -> Result<Vec<CoinRecord>>;

    /// All coin records carrying `hint` as a memo hint, optionally including
    /// already-spent coins — the hint twin of [`coin_records_by_puzzle_hash`].
    ///
    /// Unlike [`unspent_coins_by_hint`](ChainReads::unspent_coins_by_hint) (which
    /// drops spent records), this returns the FULL hint history. It is the
    /// foundation for owner-independent on-chain indexing where the discovery anchor
    /// (e.g. an NFT's mint-time owner hint) may itself have been SPENT — the public
    /// collection index walks each such record's singleton lineage forward to the
    /// current unspent tip. Wraps coinset's `get_coin_records_by_hint` with
    /// `include_spent_coins = include_spent`.
    ///
    /// Default impl returns an empty vec so existing [`ChainReads`] impls that don't
    /// model hint history (test simulators, the offline CLI mock) compile unchanged;
    /// the production [`Coinset`] overrides it with the real coinset query. This keeps
    /// the trait append-only (a new method with a default, never a changed signature).
    async fn coin_records_by_hint(
        &self,
        _hint: Bytes32,
        _include_spent: bool,
    ) -> Result<Vec<CoinRecord>> {
        Ok(Vec::new())
    }

    /// All coin records whose `parent_coin_info` is in `parent_ids`, optionally including
    /// already-spent coins — the on-chain "children of these coins" query.
    ///
    /// This is the forward-lineage primitive the public collection index walks with: a
    /// singleton's NEXT generation is the (single) child of its current coin, so following a
    /// launcher → eve → … → tip is a sequence of `coin_records_by_parent_ids([current_coin_id])`
    /// lookups. Wraps coinset's `get_coin_records_by_parent_ids`.
    ///
    /// Default impl returns an empty vec (append-only trait extension), so impls that don't
    /// model child lookups (test simulators, the offline CLI mock) compile unchanged; the
    /// production [`Coinset`] overrides it, and the test [`mock`] models it from a parent index.
    async fn coin_records_by_parent_ids(
        &self,
        _parent_ids: &[Bytes32],
        _include_spent: bool,
    ) -> Result<Vec<CoinRecord>> {
        Ok(Vec::new())
    }

    async fn coin_record(&self, name: Bytes32) -> Result<Option<CoinInfo>>;
    async fn coin_spend(&self, coin_id: Bytes32, spent_height: u32) -> Result<Option<CoinSpend>>;
    async fn peak_height(&self) -> Result<u32>;
    async fn push(&self, bundle: SpendBundle) -> Result<()>;

    /// Submit `bundle` to the mempool. Named alias of [`push`](ChainReads::push)
    /// matching coinset's `push_tx`, so state-changing parity actions (send,
    /// make/take offer, NFT/DID ops) read against the primitive they mean.
    ///
    /// The default impl delegates to `push`; impls need not override it. The
    /// dig-wallet-side `DIG_WALLET_ALLOW_BROADCAST` gate remains the policy layer
    /// above this primitive.
    async fn push_tx(&self, bundle: SpendBundle) -> Result<()> {
        self.push(bundle).await
    }

    /// Estimate the fee (mojos) required to confirm `bundle` within `target_secs` seconds.
    ///
    /// Calls coinset's `get_fee_estimate` endpoint with `target_times = [target_secs]` and
    /// `spend_count = bundle.coin_spends.len()`.  Returns `estimates[0]` on success.
    ///
    /// **Fail-open**: any network error, non-success response, or parse failure returns
    /// `Ok(0)` — fee estimation must never block a mint or commit.
    async fn estimate_fee(&self, bundle: &SpendBundle, target_secs: u64) -> Result<u64>;
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

    async fn unspent_coins_by_hint(&self, hint: Bytes32) -> Result<Vec<Coin>> {
        let resp = self
            .client
            .get_coin_records_by_hint(hint, None, None, Some(false))
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_records_by_hint: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_coin_records_by_hint failed: {:?}",
                resp.error
            )));
        }

        let coin_records = resp.coin_records.ok_or_else(|| {
            ChainError::Chain(
                "get_coin_records_by_hint: success=true but coin_records absent".to_string(),
            )
        })?;
        // include_spent_coins=false already filters at the node, but guard anyway so
        // a node that ignores the flag can't surface spent coins as "unspent".
        let coins = coin_records
            .into_iter()
            .filter(|cr| !cr.spent)
            .map(|cr| cr.coin)
            .collect();

        Ok(coins)
    }

    async fn coin_records_by_puzzle_hash(
        &self,
        puzzle_hash: Bytes32,
        include_spent: bool,
    ) -> Result<Vec<CoinRecord>> {
        let resp = self
            .client
            .get_coin_records_by_puzzle_hash(puzzle_hash, None, None, Some(include_spent))
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_records_by_puzzle_hash: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_coin_records_by_puzzle_hash failed: {:?}",
                resp.error
            )));
        }

        let coin_records = resp.coin_records.ok_or_else(|| {
            ChainError::Chain(
                "get_coin_records_by_puzzle_hash: success=true but coin_records absent".to_string(),
            )
        })?;

        Ok(coin_records.into_iter().map(map_coin_record).collect())
    }

    async fn coin_records_by_hint(
        &self,
        hint: Bytes32,
        include_spent: bool,
    ) -> Result<Vec<CoinRecord>> {
        let resp = self
            .client
            .get_coin_records_by_hint(hint, None, None, Some(include_spent))
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_records_by_hint: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_coin_records_by_hint failed: {:?}",
                resp.error
            )));
        }

        let coin_records = resp.coin_records.ok_or_else(|| {
            ChainError::Chain(
                "get_coin_records_by_hint: success=true but coin_records absent".to_string(),
            )
        })?;

        Ok(coin_records.into_iter().map(map_coin_record).collect())
    }

    async fn coin_records_by_parent_ids(
        &self,
        parent_ids: &[Bytes32],
        include_spent: bool,
    ) -> Result<Vec<CoinRecord>> {
        let resp = self
            .client
            .get_coin_records_by_parent_ids(parent_ids.to_vec(), None, None, Some(include_spent))
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_records_by_parent_ids: {e}")))?;

        if !resp.success {
            return Err(ChainError::Chain(format!(
                "get_coin_records_by_parent_ids failed: {:?}",
                resp.error
            )));
        }

        let coin_records = resp.coin_records.ok_or_else(|| {
            ChainError::Chain(
                "get_coin_records_by_parent_ids: success=true but coin_records absent".to_string(),
            )
        })?;

        Ok(coin_records.into_iter().map(map_coin_record).collect())
    }

    async fn coin_record(&self, name: Bytes32) -> Result<Option<CoinInfo>> {
        let resp = self
            .client
            .get_coin_record_by_name(name)
            .await
            .map_err(|e| ChainError::Chain(format!("get_coin_record_by_name: {e}")))?;

        let mapped = resp.coin_record.map(map_coin_record);
        classify_coin_record(resp.success, resp.error, mapped)
    }

    async fn coin_spend(&self, coin_id: Bytes32, spent_height: u32) -> Result<Option<CoinSpend>> {
        let resp = self
            .client
            .get_puzzle_and_solution(coin_id, Some(spent_height))
            .await
            .map_err(|e| ChainError::Chain(format!("get_puzzle_and_solution: {e}")))?;

        classify_coin_spend(resp.success, resp.error, resp.coin_solution)
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

    async fn estimate_fee(&self, bundle: &SpendBundle, target_secs: u64) -> Result<u64> {
        // The chia-sdk-coinset CoinsetClient does not expose get_fee_estimate, so we
        // issue a raw POST using the same reqwest client pattern it uses internally.
        // Fail-open: any error returns Ok(0) so estimation never blocks a mint/commit.
        let url = format!("{}/get_fee_estimate", self.client.base_url());
        let spend_count = bundle.coin_spends.len();

        let body = build_fee_estimate_body(bundle, target_secs, spend_count);

        let http = reqwest::Client::new();
        let result: reqwest::Result<serde_json::Value> = async {
            let resp = http.post(&url).json(&body).send().await?;
            resp.json::<serde_json::Value>().await
        }
        .await;

        let json = match result {
            Ok(v) => v,
            Err(_) => return Ok(0), // network error → fail-open
        };

        Ok(parse_fee_estimate_response(&json))
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
        /// Hint-indexed coin records (seed both spent and unspent; the mock filters
        /// spent ones for `unspent_coins_by_hint`, mirroring the real impl).
        pub records_by_hint: HashMap<Bytes32, Vec<CoinRecord>>,
        /// Puzzle-hash-indexed coin records (spent + unspent) for
        /// `coin_records_by_puzzle_hash` — the basis for tx history.
        pub records_by_ph: HashMap<Bytes32, Vec<CoinRecord>>,
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

        async fn unspent_coins_by_hint(&self, hint: Bytes32) -> Result<Vec<Coin>> {
            Ok(self
                .records_by_hint
                .get(&hint)
                .map(|recs| recs.iter().filter(|r| !r.spent).map(|r| r.coin).collect())
                .unwrap_or_default())
        }

        async fn coin_records_by_puzzle_hash(
            &self,
            puzzle_hash: Bytes32,
            include_spent: bool,
        ) -> Result<Vec<CoinRecord>> {
            Ok(self
                .records_by_ph
                .get(&puzzle_hash)
                .map(|recs| {
                    recs.iter()
                        .filter(|r| include_spent || !r.spent)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default())
        }

        async fn coin_records_by_hint(
            &self,
            hint: Bytes32,
            include_spent: bool,
        ) -> Result<Vec<CoinRecord>> {
            // Mirror the real impl: same hint index, but honour include_spent so the
            // public collection index can see SPENT mint-time owner-hint records.
            Ok(self
                .records_by_hint
                .get(&hint)
                .map(|recs| {
                    recs.iter()
                        .filter(|r| include_spent || !r.spent)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default())
        }

        async fn coin_records_by_parent_ids(
            &self,
            parent_ids: &[Bytes32],
            include_spent: bool,
        ) -> Result<Vec<CoinRecord>> {
            // Derive children from the seeded `records` map: a child's `parent_coin_info`
            // is in `parent_ids`. This models coinset's get_coin_records_by_parent_ids for
            // the forward singleton-lineage walk without a separate index.
            Ok(self
                .records
                .values()
                .filter(|r| {
                    parent_ids.contains(&r.coin.parent_coin_info) && (include_spent || !r.spent)
                })
                .cloned()
                .collect())
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

        async fn estimate_fee(&self, _bundle: &SpendBundle, _target_secs: u64) -> Result<u64> {
            // Mock always returns 0 (fail-open / empty-mempool simulation).
            Ok(0)
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
        assert!(
            matches!(r, Ok(None)),
            "not-found must be Ok(None), got {r:?}"
        );
    }

    #[test]
    fn classify_real_rpc_error_propagates() {
        let r = classify_coin_record(false, Some("internal server error".into()), None);
        assert!(matches!(r, Err(ChainError::Chain(_))));
    }

    // Same regression as coin_record but for get_puzzle_and_solution: a coin not
    // yet on-chain (no recorded solution) comes back success=false + "…not found",
    // which MUST be Ok(None) (pending), not a chain error that aborts reconstruction.
    #[test]
    fn classify_coin_spend_not_found_is_pending_not_error() {
        let r = classify_coin_spend(false, Some("Coin spend 0xabc not found".into()), None);
        assert!(
            matches!(r, Ok(None)),
            "coin_spend not-found must be Ok(None), got {r:?}"
        );
    }

    #[test]
    fn classify_coin_spend_real_rpc_error_propagates() {
        let r = classify_coin_spend(false, Some("internal server error".into()), None);
        assert!(matches!(r, Err(ChainError::Chain(_))));
    }

    #[test]
    fn classify_success_passes_record_through() {
        let info = CoinInfo {
            coin: Coin::new(Bytes32::default(), Bytes32::default(), 1),
            spent: false,
            confirmed_block_index: 100,
            spent_block_index: 0,
            timestamp: 0,
            coinbase: false,
        };
        let got = classify_coin_record(true, None, Some(info)).unwrap();
        assert_eq!(got.map(|c| c.confirmed_block_index), Some(100));
        assert!(classify_coin_record(true, None, None).unwrap().is_none());
    }

    // -----------------------------------------------------------------------
    // Fee estimate parsing tests (no live network — pure logic).
    // -----------------------------------------------------------------------

    #[test]
    fn parse_fee_estimate_success_extracts_estimates_0() {
        let json = serde_json::json!({
            "success": true,
            "estimates": [12345678_u64, 99999999_u64],
            "current_fee_rate": 5,
        });
        assert_eq!(parse_fee_estimate_response(&json), 12_345_678);
    }

    #[test]
    fn parse_fee_estimate_success_false_returns_0() {
        let json = serde_json::json!({
            "success": false,
            "error": "node not synced",
            "estimates": [999_u64],
        });
        assert_eq!(parse_fee_estimate_response(&json), 0);
    }

    #[test]
    fn parse_fee_estimate_missing_success_returns_0() {
        let json = serde_json::json!({ "estimates": [100_u64] });
        assert_eq!(parse_fee_estimate_response(&json), 0);
    }

    #[test]
    fn parse_fee_estimate_empty_estimates_returns_0() {
        let json = serde_json::json!({ "success": true, "estimates": [] });
        assert_eq!(parse_fee_estimate_response(&json), 0);
    }

    #[test]
    fn parse_fee_estimate_missing_estimates_returns_0() {
        let json = serde_json::json!({ "success": true });
        assert_eq!(parse_fee_estimate_response(&json), 0);
    }

    #[tokio::test]
    async fn mock_chain_estimate_fee_returns_0() {
        let m = MockChain::default();
        let bundle = SpendBundle::aggregate(&[]);
        let est = m.estimate_fee(&bundle, 60).await.unwrap();
        assert_eq!(est, 0, "MockChain estimate_fee must be fail-open (0)");
    }

    // -----------------------------------------------------------------------
    // FOUNDATION: hint-indexed discovery query (unspent_coins_by_hint).
    // -----------------------------------------------------------------------

    /// Build a [`CoinRecord`] fixture with explicit spent state for seeding the mock.
    fn record(parent: [u8; 32], ph: Bytes32, amount: u64, spent: bool) -> CoinRecord {
        CoinRecord {
            coin: Coin::new(Bytes32::new(parent), ph, amount),
            spent,
            confirmed_block_index: 100,
            spent_block_index: if spent { 200 } else { 0 },
            timestamp: 1_700_000_000,
            coinbase: false,
        }
    }

    #[tokio::test]
    async fn unspent_coins_by_hint_empty_for_unknown_hint() {
        let m = MockChain::default();
        let coins = m
            .unspent_coins_by_hint(Bytes32::from([0xaa; 32]))
            .await
            .unwrap();
        assert!(coins.is_empty());
    }

    #[tokio::test]
    async fn unspent_coins_by_hint_returns_only_unspent() {
        let mut m = MockChain::default();
        let hint = Bytes32::from([0x11; 32]);
        let ph = Bytes32::from([0x22; 32]);
        // Two unspent + one spent record under the same hint; only the unspent
        // ones (a store launcher discovery scenario) must surface.
        m.records_by_hint.insert(
            hint,
            vec![
                record([1u8; 32], ph, 1, false),
                record([2u8; 32], ph, 2, true),
                record([3u8; 32], ph, 3, false),
            ],
        );
        let coins = m.unspent_coins_by_hint(hint).await.unwrap();
        assert_eq!(coins.len(), 2, "spent record must be filtered out");
        let mut amounts: Vec<u64> = coins.iter().map(|c| c.amount).collect();
        amounts.sort_unstable();
        assert_eq!(amounts, vec![1, 3]);
    }

    // -----------------------------------------------------------------------
    // FOUNDATION: tx-history / spent-enumeration query
    // (coin_records_by_puzzle_hash with include_spent toggle).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn coin_records_by_puzzle_hash_empty_for_unknown_ph() {
        let m = MockChain::default();
        let recs = m
            .coin_records_by_puzzle_hash(Bytes32::from([0xbb; 32]), true)
            .await
            .unwrap();
        assert!(recs.is_empty());
    }

    #[tokio::test]
    async fn coin_records_by_puzzle_hash_excludes_spent_when_flag_false() {
        let mut m = MockChain::default();
        let ph = Bytes32::from([0x33; 32]);
        m.records_by_ph.insert(
            ph,
            vec![
                record([1u8; 32], ph, 10, false),
                record([2u8; 32], ph, 20, true),
            ],
        );
        let live = m.coin_records_by_puzzle_hash(ph, false).await.unwrap();
        assert_eq!(live.len(), 1, "include_spent=false must drop spent coins");
        assert!(!live[0].spent);
        assert_eq!(live[0].coin.amount, 10);
    }

    #[tokio::test]
    async fn coin_records_by_puzzle_hash_includes_spent_when_flag_true() {
        let mut m = MockChain::default();
        let ph = Bytes32::from([0x44; 32]);
        m.records_by_ph.insert(
            ph,
            vec![
                record([1u8; 32], ph, 10, false),
                record([2u8; 32], ph, 20, true),
            ],
        );
        let all = m.coin_records_by_puzzle_hash(ph, true).await.unwrap();
        assert_eq!(all.len(), 2, "include_spent=true must keep spent coins");
        // The spent record must carry its spent_block_index + timestamp through —
        // tx history depends on those fields, not just the coin.
        let spent = all.iter().find(|r| r.spent).expect("spent record present");
        assert_eq!(spent.spent_block_index, 200);
        assert_eq!(spent.timestamp, 1_700_000_000);
    }

    // -----------------------------------------------------------------------
    // FOUNDATION: push_tx is the mempool-submit primitive every state-changing
    // parity action depends on. It is a named alias of `push` (default impl).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn push_tx_submits_bundle_to_mempool() {
        let m = MockChain::default();
        let bundle = SpendBundle::aggregate(&[]);
        m.push_tx(bundle).await.unwrap();
        let pushed = m.pushed.lock().unwrap();
        assert_eq!(pushed.len(), 1, "push_tx must record the submitted bundle");
    }
}
