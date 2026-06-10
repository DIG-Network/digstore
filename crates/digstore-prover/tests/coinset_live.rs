//! `CoinsetChainSource` end-to-end tests against the coinset.org Chia RPC mirror.
//!
//! Two layers:
//!   * **HTTP-mocked** (default, runs in CI): an `httpmock` server returns a
//!     canned `get_blockchain_state` / `get_block_record_by_height` body, and we
//!     assert `CoinsetChainSource` parses it into the expected `ChiaBlockRef`
//!     and that `verify_block` accepts a fresh on-chain block.
//!   * **Live** (`#[ignore]`d): hits the real `https://api.coinset.org` peak.
//!     Opt in with `cargo test -p digstore-prover --test coinset_live -- --ignored`.
//!     Best-effort: skipped in CI and tolerant of transient network failure.

use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::{ChainSource, CoinsetChainSource};

const STATE_JSON: &str = include_str!("fixtures/get_blockchain_state.json");

/// A transaction-block record at the peak height so `verify_block`'s on-chain
/// header-hash check matches the peak and finds a timestamp without walking.
const PEAK_RECORD_JSON: &str = r#"{
  "block_record": {
    "header_hash": "0xb5f2a7c1d3e4f5061728394a5b6c7d8e9f00112233445566778899aabbccddee",
    "height": 5421337,
    "timestamp": 1717804800,
    "prev_transaction_block_height": 5421330
  },
  "success": true
}"#;

fn peak_header_hash() -> Bytes32 {
    Bytes32::from_hex("b5f2a7c1d3e4f5061728394a5b6c7d8e9f00112233445566778899aabbccddee").unwrap()
}

#[test]
fn mocked_get_peak_parses_canned_blockchain_state() {
    let server = httpmock::MockServer::start();
    let m = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/get_blockchain_state");
        then.status(200)
            .header("content-type", "application/json")
            .body(STATE_JSON);
    });

    let src = CoinsetChainSource::new(server.base_url());
    let block = src.get_peak().expect("get_peak parses the canned state");
    m.assert();

    assert_eq!(block.height, 5_421_337);
    assert_eq!(block.timestamp, 1_717_804_800);
    assert_eq!(block.header_hash, peak_header_hash());
}

#[test]
fn mocked_verify_block_accepts_fresh_on_chain_block() {
    let server = httpmock::MockServer::start();
    // verify_block fetches the record at the block height (on-chain check) ...
    let _rec = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/get_block_record_by_height");
        then.status(200)
            .header("content-type", "application/json")
            .body(PEAK_RECORD_JSON);
    });
    // ... and the peak (for "now") for the freshness comparison.
    let _state = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/get_blockchain_state");
        then.status(200)
            .header("content-type", "application/json")
            .body(STATE_JSON);
    });

    let src = CoinsetChainSource::new(server.base_url());
    let block = ChiaBlockRef {
        header_hash: peak_header_hash(),
        height: 5_421_337,
        timestamp: 1_717_804_800,
    };
    // now == block.timestamp (peak), so it is trivially within any window.
    src.verify_block(&block, 600)
        .expect("a fresh on-chain block verifies");
}

#[test]
fn mocked_verify_block_rejects_block_not_on_chain() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/get_block_record_by_height");
        then.status(200)
            .header("content-type", "application/json")
            .body(PEAK_RECORD_JSON);
    });

    let src = CoinsetChainSource::new(server.base_url());
    // Same height, DIFFERENT header hash => not the block that's actually on-chain.
    let forged = ChiaBlockRef {
        header_hash: Bytes32([0xAB; 32]),
        height: 5_421_337,
        timestamp: 1_717_804_800,
    };
    assert!(
        src.verify_block(&forged, 600).is_err(),
        "a header-hash mismatch at the height must be rejected"
    );
}

/// Live smoke test against the real coinset.org mirror. Best-effort: opt in with
/// `--ignored`. Asserts the peak has a plausible height/timestamp. Tolerant of
/// transient network failure so it never flakes CI (it is `#[ignore]`d anyway).
#[test]
#[ignore = "live network: hits https://api.coinset.org; run with --ignored"]
fn live_get_peak_from_coinset_org() {
    let src = CoinsetChainSource::default();
    match src.get_peak() {
        Ok(peak) => {
            assert!(peak.height > 5_000_000, "mainnet peak height is large");
            assert!(
                peak.timestamp > 1_600_000_000,
                "peak timestamp is after 2020"
            );
            assert_ne!(peak.header_hash, Bytes32([0u8; 32]), "non-zero header hash");
        }
        Err(e) => {
            eprintln!("[live] coinset.org get_peak failed (best-effort, skipping): {e}");
        }
    }
}
