//! Live [`ChainSource`] backed by the coinset.org Chia full-node RPC mirror
//! (`https://api.coinset.org`).
//!
//! This is the REAL chain source for residual #3 (`SECURITY.md`): it supplies
//! the current block header hash + height + timestamp used to anchor the
//! attestation freshness gate (§13.7/§16) to wall-clock time. It calls the
//! standard Chia RPC endpoints `POST /get_blockchain_state` (current peak) and
//! `POST /get_block_record_by_height` (on-chain confirmation + timestamp walk).
//!
//! It is best-effort with short, clear errors ([`ProverError::ChainRpc`]). The
//! mocked parsing tests live in `tests/coinset_parse.rs`; an HTTP-mocked and an
//! `#[ignore]`d live test live in `tests/coinset_live.rs`.
//!
//! NOTE: a real chain source alone does NOT make execution proofs unforgeable —
//! the proof backend is still the forgeable [`crate::mock::MockProver`] unless
//! the `risc0` feature is enabled (which needs the RISC0 toolchain). See
//! `SECURITY.md` residual #3.

use crate::chain::ChainSource;
use crate::error::{ProverError, Result};
use digstore_core::{Bytes32, ChiaBlockRef};
use serde::Deserialize;

/// A Chia `BlockRecord` (subset we need). `timestamp` is `None` on
/// non-transaction blocks; such blocks point at a previous transaction block.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRecord {
    pub header_hash: String,
    pub height: u32,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub prev_transaction_block_height: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainState {
    pub peak: Option<BlockRecord>,
}

/// Response of `POST /get_blockchain_state`.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainStateResponse {
    pub success: bool,
    #[serde(default)]
    pub blockchain_state: Option<BlockchainState>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response of `POST /get_block_record_by_height` and `/get_block_record`.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRecordResponse {
    pub success: bool,
    #[serde(default)]
    pub block_record: Option<BlockRecord>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A resolver that fetches a `BlockRecord` at a given height (used to walk down
/// to the previous transaction block when a record lacks a timestamp).
pub type BlockRecordResolver<'a> = dyn FnMut(u32) -> Result<BlockRecord> + 'a;

/// Decode a Chia `0x`-prefixed 32-byte hex string into [`Bytes32`].
fn parse_header_hash(s: &str) -> Result<Bytes32> {
    let hex = s.strip_prefix("0x").unwrap_or(s);
    Bytes32::from_hex(hex).map_err(|e| ProverError::ChainRpc(format!("bad header hash hex: {e:?}")))
}

/// Resolve a `BlockRecord` to a [`ChiaBlockRef`]. The header hash and height
/// come from `record`; if `record` has no timestamp it is a non-transaction
/// block, so we walk `prev_transaction_block_height` via `resolve` until a
/// timestamped block is found, and inherit that timestamp.
fn record_to_ref(
    record: BlockRecord,
    resolve: &mut BlockRecordResolver<'_>,
) -> Result<ChiaBlockRef> {
    let header_hash = parse_header_hash(&record.header_hash)?;
    let height = record.height;
    let timestamp = resolve_timestamp(&record, resolve)?;
    Ok(ChiaBlockRef {
        header_hash,
        height,
        timestamp,
    })
}

/// Walk down to the nearest previous transaction block to obtain a timestamp.
fn resolve_timestamp(record: &BlockRecord, resolve: &mut BlockRecordResolver<'_>) -> Result<u64> {
    if let Some(ts) = record.timestamp {
        return Ok(ts);
    }
    let mut next = record.prev_transaction_block_height;
    // Bound the walk to avoid pathological loops.
    for _ in 0..256 {
        let h = next.ok_or_else(|| {
            ProverError::ChainRpc(format!(
                "block at height {} has no timestamp and no prev_transaction_block_height",
                record.height
            ))
        })?;
        let prev = resolve(h)?;
        if let Some(ts) = prev.timestamp {
            return Ok(ts);
        }
        next = prev.prev_transaction_block_height;
    }
    Err(ProverError::ChainRpc(format!(
        "could not resolve a transaction block timestamp for height {}",
        record.height
    )))
}

/// Extract the peak [`ChiaBlockRef`] from a blockchain-state response. The peak
/// is a transaction block (carries its own timestamp), so no resolver is used.
pub fn parse_blockchain_state(resp: BlockchainStateResponse) -> Result<ChiaBlockRef> {
    if !resp.success {
        return Err(ProverError::ChainRpc(
            resp.error
                .unwrap_or_else(|| "get_blockchain_state failed".into()),
        ));
    }
    let peak = resp
        .blockchain_state
        .and_then(|s| s.peak)
        .ok_or_else(|| ProverError::ChainRpc("no peak in blockchain_state".into()))?;
    record_to_ref(peak, &mut |_h| {
        Err(ProverError::ChainRpc(
            "peak unexpectedly lacked a timestamp".into(),
        ))
    })
}

/// Extract a [`ChiaBlockRef`] from a block-record response, walking down to the
/// previous transaction block (via `resolve`) for the timestamp when needed.
pub fn parse_block_record_resolved(
    resp: BlockRecordResponse,
    resolve: &mut BlockRecordResolver<'_>,
) -> Result<ChiaBlockRef> {
    if !resp.success {
        return Err(ProverError::ChainRpc(
            resp.error
                .unwrap_or_else(|| "get_block_record failed".into()),
        ));
    }
    let rec = resp
        .block_record
        .ok_or_else(|| ProverError::ChainRpc("no block_record in response".into()))?;
    record_to_ref(rec, resolve)
}

/// Live [`ChainSource`] backed by the coinset.org Chia RPC mirror.
#[derive(Debug, Clone)]
pub struct CoinsetChainSource {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl Default for CoinsetChainSource {
    fn default() -> Self {
        Self::new("https://api.coinset.org")
    }
}

impl CoinsetChainSource {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::blocking::Client::new(),
        }
    }

    fn post_json(&self, endpoint: &str, body: String) -> Result<String> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint);
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .map_err(|e| ProverError::ChainRpc(format!("{endpoint} send: {e}")))?;
        resp.text()
            .map_err(|e| ProverError::ChainRpc(format!("{endpoint} body: {e}")))
    }

    fn fetch_record(&self, height: u32) -> Result<BlockRecord> {
        let body = self.post_json(
            "get_block_record_by_height",
            format!("{{\"height\": {height}}}"),
        )?;
        let resp: BlockRecordResponse = serde_json::from_str(&body)
            .map_err(|e| ProverError::ChainRpc(format!("parse record: {e}")))?;
        if !resp.success {
            return Err(ProverError::ChainRpc(
                resp.error
                    .unwrap_or_else(|| "get_block_record_by_height failed".into()),
            ));
        }
        resp.block_record
            .ok_or_else(|| ProverError::ChainRpc("no block_record in response".into()))
    }
}

impl ChainSource for CoinsetChainSource {
    fn get_peak(&self) -> Result<ChiaBlockRef> {
        let body = self.post_json("get_blockchain_state", "{}".into())?;
        let resp: BlockchainStateResponse = serde_json::from_str(&body)
            .map_err(|e| ProverError::ChainRpc(format!("parse state: {e}")))?;
        parse_blockchain_state(resp)
    }

    fn verify_block(&self, block: &ChiaBlockRef, freshness_window_secs: u64) -> Result<()> {
        // Confirm the block is on-chain: fetch the record at this height and
        // compare header hashes (walking down for a timestamp if needed).
        let body = self.post_json(
            "get_block_record_by_height",
            format!("{{\"height\": {}}}", block.height),
        )?;
        let resp: BlockRecordResponse = serde_json::from_str(&body)
            .map_err(|e| ProverError::ChainRpc(format!("parse record: {e}")))?;
        let on_chain = parse_block_record_resolved(resp, &mut |h| self.fetch_record(h))?;
        if on_chain.header_hash != block.header_hash {
            return Err(ProverError::BlockNotOnChain(block.header_hash.to_hex()));
        }
        // Freshness against the current peak's wall-clock timestamp.
        let now = self.get_peak()?.timestamp;
        if block.timestamp > now {
            return Err(ProverError::BlockInFuture(block.timestamp, now));
        }
        if now - block.timestamp > freshness_window_secs {
            return Err(ProverError::BlockTooOld {
                block_ts: block.timestamp,
                now,
                window: freshness_window_secs,
            });
        }
        Ok(())
    }
}
