//! Coinset.org access behind a small trait so anchoring logic is testable
//! without a network. Real impl wraps `chia_sdk_coinset::CoinsetClient`.
//!
//! ## Transient-failure resilience (#84 — a live user was blocked minting on mainnet)
//!
//! coinset.org intermittently truncates/aborts an HTTP response body under load
//! (`reqwest` surfaces this as `error decoding response body`). It has NO built-in
//! retry or timeout. A single such hiccup used to abort `digstore doctor` (the fund
//! scan) and `digstore init` (the mint) — and, critically, one hiccup landed AFTER
//! the mint spend was already broadcast, aborting a flow whose XCH was already spent.
//!
//! Every coinset read here therefore goes through [`Coinset::call`], which wraps the
//! underlying RPC in retry-with-exponential-backoff + jitter + a per-attempt timeout
//! ([`retry_core`] / [`RetryConfig`]). Transient failures (truncated body, transport
//! errors, timeouts, 5xx, 429 — see [`TransientClass`]) are retried; a definitive
//! not-found or other terminal 4xx is NOT retried (it never becomes a hang). Reads are
//! issued SEQUENTIALLY (the wallet scan loops address-by-address), so concurrency is
//! already bounded to 1 and digstore never triggers coinset's parallel-fan-out volume
//! failure (#62); the retry+timeout handles the isolated transient truncation.

use crate::error::{ChainError, Result};
use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use chia_sdk_coinset::ChiaRpcClient;
use std::future::Future;
use std::time::Duration;

/// Chia's per-generator-byte CLVM cost (`ConsensusConstants::cost_per_byte`). The serialized coin
/// spends of a bundle contribute this much cost per byte BEFORE any execution/condition cost, so it
/// gives a cheap, decode-free lower bound on a bundle's total cost.
const COST_PER_BYTE: u64 = 12_000;

/// Serialized generator size (bytes) of a bundle's coin spends — the sum of every spend's puzzle
/// reveal + solution. A cheap size proxy that needs no CLVM decode.
fn bundle_generator_bytes(bundle: &SpendBundle) -> usize {
    bundle
        .coin_spends
        .iter()
        .map(|cs| cs.puzzle_reveal.as_slice().len() + cs.solution.as_slice().len())
        .sum()
}

/// Returns `Some(reason)` when `bundle` is DEFINITIVELY too large to ever be accepted: its generator
/// bytes ALONE (times [`COST_PER_BYTE`], before any execution/condition cost) meet or exceed the
/// per-block CLVM cost ceiling [`crate::collection::MAX_BLOCK_COST_CLVM`]. Such a bundle is a
/// terminal condition — no retry helps — so `push` refuses it up-front (#231). The message is
/// actionable and explicitly disclaims the misleading "coinset.org connectivity" reading.
fn oversize_reason(bundle: &SpendBundle) -> Option<String> {
    let bytes = bundle_generator_bytes(bundle);
    let byte_cost = (bytes as u64).saturating_mul(COST_PER_BYTE);
    if byte_cost >= crate::collection::MAX_BLOCK_COST_CLVM {
        Some(format!(
            "the spend bundle is {bytes} generator bytes ({} spends), whose byte cost alone \
             ({byte_cost}) exceeds Chia's per-block cost limit ({}) — this is a transaction SIZE \
             limit, NOT a coinset.org connectivity problem. Split the operation into smaller \
             batches (e.g. `collection mint --batch-size`).",
            bundle.coin_spends.len(),
            crate::collection::MAX_BLOCK_COST_CLVM,
        ))
    } else {
        None
    }
}

/// Retry policy for transient coinset RPC failures.
///
/// Defaults are tuned for an interactive CLI: recover quickly from the common
/// single-hiccup case while bounding the worst case (coinset fully down) so a
/// command never hangs indefinitely.
#[derive(Clone, Copy, Debug)]
pub struct RetryConfig {
    /// Total attempts (including the first). `1` disables retry.
    pub max_attempts: u32,
    /// Base backoff before the first retry; doubles each subsequent retry.
    pub base_delay: Duration,
    /// Upper bound on any single backoff sleep.
    pub max_delay: Duration,
    /// Per-attempt wall-clock budget; an attempt exceeding it is treated as a
    /// transient timeout and retried. Needed because the coinset client sets no
    /// reqwest timeout, so a hung connection would otherwise block forever.
    pub per_attempt_timeout: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
            per_attempt_timeout: Duration::from_secs(20),
        }
    }
}

/// Classifies an operation error as transient (worth retrying) or terminal.
trait TransientClass {
    fn is_transient(&self) -> bool;
}

impl TransientClass for reqwest::Error {
    fn is_transient(&self) -> bool {
        // A truncated/failed body decode ("error decoding response body" — the exact
        // #84 symptom), connect/request/body transport errors, and timeouts are all
        // transient coinset hiccups under load. A surfaced 5xx / 429 is transient too;
        // any other status (a real 4xx client error) is terminal.
        if self.is_timeout()
            || self.is_connect()
            || self.is_request()
            || self.is_body()
            || self.is_decode()
        {
            return true;
        }
        if let Some(status) = self.status() {
            return status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        }
        false
    }
}

/// Failure outcome of [`retry_core`]: the terminal/last operation error, or an
/// all-attempts-timed-out signal (no operation error to surface).
enum RetryFail<E> {
    Op(E),
    Timeout,
}

/// Compute the (jittered) backoff before the `attempt`-th retry (1-based on the
/// number of failures so far): exponential `base * 2^(attempt-1)`, capped at
/// `max_delay`, then full-jittered into `[delay/2, delay]` to avoid thundering-herd
/// synchronization across concurrent clients.
fn jittered_backoff(cfg: &RetryConfig, attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(16);
    let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
    let base_ms = cfg.base_delay.as_millis() as u64;
    let cap_ms = cfg.max_delay.as_millis() as u64;
    let exp_ms = base_ms.saturating_mul(factor).min(cap_ms);
    if exp_ms == 0 {
        return Duration::from_millis(0);
    }
    let half = exp_ms / 2;
    let span = exp_ms - half;
    let rnd = rand_u64() % (span + 1);
    Duration::from_millis(half + rnd)
}

/// A single random u64 for jitter. `getrandom` is a hard dependency; on its
/// (effectively impossible) failure we degrade to zero jitter rather than panic.
fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    if getrandom::getrandom(&mut b).is_ok() {
        u64::from_le_bytes(b)
    } else {
        0
    }
}

/// Run `op` with retry-on-transient + exponential backoff + jitter + a per-attempt
/// timeout. Returns the first success; retries transient failures and per-attempt
/// timeouts up to `cfg.max_attempts`; returns immediately on a terminal error.
///
/// Generic over the error + its [`TransientClass`] so the loop is unit-testable with
/// a synthetic error (the production callers use `reqwest::Error`).
async fn retry_core<T, E, F, Fut>(
    cfg: &RetryConfig,
    mut op: F,
) -> std::result::Result<T, RetryFail<E>>
where
    E: TransientClass,
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
{
    let max = cfg.max_attempts.max(1);
    let mut attempt = 1u32;
    loop {
        match tokio::time::timeout(cfg.per_attempt_timeout, op()).await {
            Ok(Ok(v)) => return Ok(v),
            Ok(Err(e)) => {
                if !e.is_transient() || attempt >= max {
                    return Err(RetryFail::Op(e));
                }
            }
            Err(_elapsed) => {
                if attempt >= max {
                    return Err(RetryFail::Timeout);
                }
            }
        }
        tokio::time::sleep(jittered_backoff(cfg, attempt)).await;
        attempt += 1;
    }
}

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
    retry: RetryConfig,
}

impl Coinset {
    pub fn mainnet() -> Self {
        Self {
            client: chia_sdk_coinset::CoinsetClient::mainnet(),
            retry: RetryConfig::default(),
        }
    }

    pub fn with_url(base_url: String) -> Self {
        Self {
            client: chia_sdk_coinset::CoinsetClient::new(base_url),
            retry: RetryConfig::default(),
        }
    }

    /// Override the retry policy (tuning / tests). Defaults to [`RetryConfig::default`].
    pub fn with_retry_config(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Issue one coinset RPC through the transient-failure retry wrapper.
    ///
    /// `op` re-issues the underlying request on each attempt (so retries hit a fresh
    /// connection after a truncated body). `label` names the RPC for error messages,
    /// preserving the pre-existing `"<method>: <error>"` shape callers/tests expect.
    async fn call<T, F, Fut>(&self, label: &str, op: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = reqwest::Result<T>>,
    {
        retry_core(&self.retry, op).await.map_err(|f| match f {
            RetryFail::Op(e) => ChainError::Chain(format!("{label}: {e}")),
            RetryFail::Timeout => ChainError::Chain(format!(
                "{label}: coinset did not respond after {} attempts",
                self.retry.max_attempts
            )),
        })
    }
}

#[async_trait::async_trait]
impl ChainReads for Coinset {
    async fn unspent_coins(&self, puzzle_hash: Bytes32) -> Result<Vec<Coin>> {
        let resp = self
            .call("get_coin_records_by_puzzle_hashes", || {
                self.client.get_coin_records_by_puzzle_hashes(
                    vec![puzzle_hash],
                    None,
                    None,
                    Some(false),
                )
            })
            .await?;

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
            .call("get_coin_records_by_hint", || {
                self.client
                    .get_coin_records_by_hint(hint, None, None, Some(false))
            })
            .await?;

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
            .call("get_coin_records_by_puzzle_hash", || {
                self.client.get_coin_records_by_puzzle_hash(
                    puzzle_hash,
                    None,
                    None,
                    Some(include_spent),
                )
            })
            .await?;

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
            .call("get_coin_records_by_hint", || {
                self.client
                    .get_coin_records_by_hint(hint, None, None, Some(include_spent))
            })
            .await?;

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
            .call("get_coin_records_by_parent_ids", || {
                self.client.get_coin_records_by_parent_ids(
                    parent_ids.to_vec(),
                    None,
                    None,
                    Some(include_spent),
                )
            })
            .await?;

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
            .call("get_coin_record_by_name", || {
                self.client.get_coin_record_by_name(name)
            })
            .await?;

        let mapped = resp.coin_record.map(map_coin_record);
        classify_coin_record(resp.success, resp.error, mapped)
    }

    async fn coin_spend(&self, coin_id: Bytes32, spent_height: u32) -> Result<Option<CoinSpend>> {
        let resp = self
            .call("get_puzzle_and_solution", || {
                self.client
                    .get_puzzle_and_solution(coin_id, Some(spent_height))
            })
            .await?;

        classify_coin_spend(resp.success, resp.error, resp.coin_solution)
    }

    async fn peak_height(&self) -> Result<u32> {
        let resp = self
            .call("get_blockchain_state", || {
                self.client.get_blockchain_state()
            })
            .await?;

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
        // Pre-flight oversize guard (#231): a bundle whose generator bytes alone would blow the
        // per-block CLVM cost limit is DEFINITIVELY too large — the full node rejects it and coinset
        // returns an aborted/non-JSON body that `reqwest` surfaces as "error decoding response body"
        // (a transient-looking symptom the #84 retry logic would otherwise retry + misreport as a
        // coinset.org connectivity problem). Detect it up-front and fail TERMINALLY with an
        // actionable message, before broadcasting or retrying.
        if let Some(reason) = oversize_reason(&bundle) {
            return Err(ChainError::BundleTooLarge(reason));
        }
        // Retrying a transient transport error on push is SAFE: the tx id is
        // deterministic, so re-submitting the identical bundle is idempotent at the
        // mempool (a duplicate is accepted / already-present, never a second spend).
        // The bundle is cloned per attempt because `push_tx` consumes it.
        let resp = self
            .call("push_tx", || self.client.push_tx(bundle.clone()))
            .await?;

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

    // -----------------------------------------------------------------------
    // #84: transient-failure retry wrapper. coinset.org intermittently returns
    // a truncated body ("error decoding response body") under load; a single
    // hiccup used to abort doctor's fund scan and init's mint (once even AFTER
    // the mint was broadcast). These prove the wrapper retries transient
    // failures, does NOT retry terminal ones (no hang), and bounds attempts.
    // -----------------------------------------------------------------------

    /// A synthetic op error so the retry loop is testable without a network.
    #[derive(Debug, PartialEq)]
    enum TestErr {
        Transient,
        Terminal,
    }
    impl TransientClass for TestErr {
        fn is_transient(&self) -> bool {
            matches!(self, TestErr::Transient)
        }
    }

    /// Near-zero delays so retry tests are fast + deterministic.
    fn fast_cfg(max_attempts: u32) -> RetryConfig {
        RetryConfig {
            max_attempts,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(1),
            per_attempt_timeout: Duration::from_secs(5),
        }
    }

    // Acceptance (a): a transient failure then success → the wrapper returns
    // success, not an abort.
    #[tokio::test]
    async fn retry_recovers_from_transient_then_succeeds() {
        let cfg = fast_cfg(5);
        let calls = std::cell::Cell::new(0u32);
        let out: std::result::Result<u32, RetryFail<TestErr>> = retry_core(&cfg, || {
            let n = calls.get();
            calls.set(n + 1);
            async move {
                if n == 0 {
                    Err(TestErr::Transient)
                } else {
                    Ok(42u32)
                }
            }
        })
        .await;
        assert!(
            matches!(out, Ok(42)),
            "should recover to success after retry"
        );
        assert_eq!(calls.get(), 2, "one failure + one success = two attempts");
    }

    // Acceptance (b): a terminal (definitive) error is NOT retried — it returns
    // immediately and never hangs looping.
    #[tokio::test]
    async fn retry_does_not_retry_terminal_error() {
        let cfg = fast_cfg(5);
        let calls = std::cell::Cell::new(0u32);
        let out: std::result::Result<u32, RetryFail<TestErr>> = retry_core(&cfg, || {
            calls.set(calls.get() + 1);
            async move { Err::<u32, _>(TestErr::Terminal) }
        })
        .await;
        assert!(matches!(out, Err(RetryFail::Op(TestErr::Terminal))));
        assert_eq!(calls.get(), 1, "terminal error must not be retried");
    }

    // A persistently transient failure is retried up to the bound, then gives up
    // (surfaces the last error) — bounded, never an infinite loop.
    #[tokio::test]
    async fn retry_exhausts_attempts_on_persistent_transient() {
        let cfg = fast_cfg(3);
        let calls = std::cell::Cell::new(0u32);
        let out: std::result::Result<u32, RetryFail<TestErr>> = retry_core(&cfg, || {
            calls.set(calls.get() + 1);
            async move { Err::<u32, _>(TestErr::Transient) }
        })
        .await;
        assert!(matches!(out, Err(RetryFail::Op(TestErr::Transient))));
        assert_eq!(calls.get(), 3, "exactly max_attempts tries");
    }

    // A per-attempt timeout is treated as transient and retried.
    #[tokio::test]
    async fn retry_recovers_from_per_attempt_timeout() {
        let cfg = RetryConfig {
            max_attempts: 3,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(1),
            per_attempt_timeout: Duration::from_millis(20),
        };
        let calls = std::cell::Cell::new(0u32);
        let out: std::result::Result<u32, RetryFail<TestErr>> = retry_core(&cfg, || {
            let n = calls.get();
            calls.set(n + 1);
            async move {
                if n == 0 {
                    // First attempt overruns the per-attempt budget → timeout → retry.
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
                Ok::<u32, TestErr>(7)
            }
        })
        .await;
        assert!(
            matches!(out, Ok(7)),
            "should retry after the first attempt times out"
        );
        assert_eq!(calls.get(), 2);
    }

    // Acceptance (a), end-to-end through the REAL reqwest path: a local server that
    // truncates the body on the first connection then returns valid JSON on the
    // second. `unspent_coins` must succeed (retry), not abort with
    // "error decoding response body".
    #[tokio::test]
    async fn coinset_retries_truncated_body_then_succeeds() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            // 1st connection: declare a large Content-Length, send a partial body,
            // then close → reqwest fails decoding the response body (transient).
            let (mut s1, _) = listener.accept().unwrap();
            let _ = s1.read(&mut buf);
            let _ = s1.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 200\r\nConnection: close\r\n\r\n{\"success\": tru",
            );
            drop(s1);
            // 2nd connection: a full, valid empty-result response.
            let (mut s2, _) = listener.accept().unwrap();
            let _ = s2.read(&mut buf);
            let body = b"{\"success\": true, \"coin_records\": []}";
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s2.write_all(hdr.as_bytes());
            let _ = s2.write_all(body);
            let _ = s2.flush();
        });

        let cs = Coinset::with_url(format!("http://{addr}")).with_retry_config(fast_cfg(5));
        let coins = cs
            .unspent_coins(Bytes32::default())
            .await
            .expect("retry must recover the truncated first response");
        assert!(coins.is_empty(), "valid second response parsed");
        handle.join().unwrap();
    }

    // Acceptance (b), end-to-end: a definitive not-found (`success:false` +
    // "...not found") is returned in ONE response and maps to Ok(None) — it is NOT
    // retried (the server only ever accepts a single connection) and never hangs.
    #[tokio::test]
    async fn coinset_not_found_is_terminal_single_call() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let (mut s, _) = listener.accept().unwrap();
            let _ = s.read(&mut buf);
            let body = b"{\"success\": false, \"error\": \"Coin record 0xabc not found\"}";
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
            // Do NOT accept a second connection: if the code retried a not-found, the
            // client would hang and the test would time out.
        });

        let cs = Coinset::with_url(format!("http://{addr}")).with_retry_config(fast_cfg(5));
        let rec = cs
            .coin_record(Bytes32::default())
            .await
            .expect("not-found must be Ok(None), not an error");
        assert!(rec.is_none(), "not-found maps to Ok(None)");
        handle.join().unwrap();
    }

    // Acceptance (c): post-submit confirmation survives a transient error. A
    // `CoinsetAnchor::confirm` poll hits a truncated body on the first connection,
    // then a valid coin record on the second → confirmation succeeds (Confirmed)
    // rather than aborting the flow whose spend is already on the wire.
    #[tokio::test]
    async fn confirm_survives_transient_then_confirms() {
        use crate::anchor::{ChainAnchor, CoinsetAnchor, ConfirmState};
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            // 1st poll: truncated body → transient.
            let (mut s1, _) = listener.accept().unwrap();
            let _ = s1.read(&mut buf);
            let _ = s1.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 300\r\nConnection: close\r\n\r\n{\"succ",
            );
            drop(s1);
            // 2nd poll: a valid coin record confirmed at height 12345.
            let (mut s2, _) = listener.accept().unwrap();
            let _ = s2.read(&mut buf);
            let body = concat!(
                "{\"success\": true, \"coin_record\": {",
                "\"coin\": {\"amount\": 1, \"parent_coin_info\": \"0x0000000000000000000000000000000000000000000000000000000000000000\", \"puzzle_hash\": \"0x0000000000000000000000000000000000000000000000000000000000000000\"},",
                "\"confirmed_block_index\": 12345, \"spent_block_index\": 0, \"spent\": false, \"coinbase\": false, \"timestamp\": 1700000000}}"
            );
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s2.write_all(hdr.as_bytes());
            let _ = s2.write_all(body.as_bytes());
            let _ = s2.flush();
        });

        let cs = Coinset::with_url(format!("http://{addr}")).with_retry_config(fast_cfg(5));
        let anchor = CoinsetAnchor::new(cs);
        // timeout_secs=1 → a single poll; the retry happens WITHIN that poll's
        // coin_record call, so confirmation still succeeds despite the transient.
        let state = anchor
            .confirm(Bytes32::default(), 1)
            .await
            .expect("confirm must survive a transient coinset error");
        assert_eq!(state, ConfirmState::Confirmed { height: 12345 });
        handle.join().unwrap();
    }

    // -----------------------------------------------------------------------
    // #231: oversized-bundle pre-flight guard. A bundle whose generator bytes
    // alone exceed the per-block cost limit is TERMINAL — `push` must refuse it
    // up-front (not broadcast, not retry, not misreport as a coinset hiccup).
    // -----------------------------------------------------------------------

    fn coin_spend_with_reveal_len(reveal_len: usize) -> CoinSpend {
        use chia_protocol::Program;
        CoinSpend::new(
            Coin::new(Bytes32::default(), Bytes32::default(), 0),
            Program::from(vec![0u8; reveal_len]),
            Program::from(vec![0u8; 8]),
        )
    }

    #[test]
    fn oversize_reason_flags_only_bundles_over_the_block_limit() {
        // A tiny bundle is fine.
        let small = SpendBundle::new(
            vec![coin_spend_with_reveal_len(500)],
            chia::bls::Signature::default(),
        );
        assert!(oversize_reason(&small).is_none());
        assert_eq!(bundle_generator_bytes(&small), 508);

        // A bundle whose generator bytes * COST_PER_BYTE >= MAX_BLOCK_COST_CLVM is terminal.
        // Threshold ≈ 11e9 / 12000 ≈ 916_667 bytes; one ~1 MB reveal clears it.
        let big = SpendBundle::new(
            vec![coin_spend_with_reveal_len(1_000_000)],
            chia::bls::Signature::default(),
        );
        let reason = oversize_reason(&big).expect("a ~1 MB bundle must be flagged oversized");
        assert!(
            reason.contains("NOT a coinset.org connectivity problem")
                && reason.contains("--batch-size"),
            "the oversize message must be actionable and disclaim a connectivity cause: {reason}"
        );
    }

    #[tokio::test]
    async fn push_refuses_oversized_bundle_terminally_without_network() {
        // Point at an unroutable address: if the guard fires first, no connection is attempted, so
        // the test returns instantly with the terminal BundleTooLarge (proving no retry / no network).
        let cs = Coinset::with_url("http://127.0.0.1:9".into()).with_retry_config(fast_cfg(5));
        let big = SpendBundle::new(
            vec![coin_spend_with_reveal_len(1_000_000)],
            chia::bls::Signature::default(),
        );
        let err = cs.push(big).await.unwrap_err();
        assert!(
            matches!(&err, ChainError::BundleTooLarge(m) if m.contains("SIZE limit")),
            "oversized push must be terminal BundleTooLarge, got: {err}"
        );
    }
}
