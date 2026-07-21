//! `dig-store store-status <store_id>` — report a store's aggregate on-chain status (#1349).
//!
//! This is a THIN consumer of the fail-closed [`dig_store::get_store_status`] aggregator: it
//! resolves a Chia read transport (coinset, with a user override), wraps it as a canonical
//! [`ChainSource`](dig_chainsource_interface::ChainSource) via
//! [`CoinsetChainSource`](crate::ops::coinset_chain_source::CoinsetChainSource), asks the library
//! for the store's status, and renders it. It NEVER re-implements the lineage walk or the
//! Live/Melted/NotFound decision — that money-critical logic is owned + triple-gated in `dig-store`.
//!
//! ## This is a raw Chia-chain read, not a dig-node content read
//!
//! Store status is read from Chia chain state (coin records + spends), so it deliberately does NOT
//! use the §5.3 `dig.local -> localhost -> rpc.dig.net` DIG-node content ladder (a dig-node does
//! not answer raw `coin_record`/`coin_spend` queries). The endpoint is resolved as:
//! `--coinset-url` > `$DIG_COINSET_URL` > the default coinset.org URL — giving the user a
//! first-class way to point at a custom Chia read endpoint.

use chia_protocol::Bytes32;
use dig_store::{get_store_status, StoreStatus, StoreStatusKind};

use crate::cli::StoreStatusArgs;
use crate::error::CliError;
use crate::ops::coinset_chain_source::CoinsetChainSource;
use crate::ui::Ui;

/// Environment variable that overrides the coinset read endpoint (below `--coinset-url`).
const COINSET_URL_ENV: &str = "DIG_COINSET_URL";

/// Runs `store-status`: resolve the endpoint, read the status via the library, render it.
pub fn run(ui: &Ui, args: StoreStatusArgs) -> Result<(), CliError> {
    let store_id = parse_store_id(&args.store_id)?;
    let coinset_url = resolve_coinset_url(args.coinset_url.as_deref());

    let chain = CoinsetChainSource::new(digstore_chain::coinset::Coinset::with_url(coinset_url))
        .map_err(|e| CliError::Network(format!("chain source: {e}")))?;

    let status = get_store_status(&chain, store_id, args.confirmation_target).map_err(|e| {
        // A library error means the chain could NOT be read reliably (fail closed) — it is NOT a
        // "store not found" (that is a successful `StoreStatusKind::NotFound` result).
        CliError::Chain(format!("could not read store status: {e}"))
    })?;

    if ui.json() {
        ui.emit_json(&status);
    } else {
        for (label, value) in human_rows(&status) {
            ui.line(format!("{label:<20} {value}"));
        }
    }
    Ok(())
}

/// Resolves the coinset read endpoint: `--coinset-url` wins, then `$DIG_COINSET_URL`, then the
/// crate default (coinset.org). The env var is read directly (not a clap `env=`) so the flag and
/// the env var stay independently observable, mirroring how `--node` is resolved elsewhere.
fn resolve_coinset_url(flag: Option<&str>) -> String {
    if let Some(url) = flag {
        return url.to_string();
    }
    if let Ok(url) = std::env::var(COINSET_URL_ENV) {
        if !url.trim().is_empty() {
            return url;
        }
    }
    digstore_chain::config::DEFAULT_COINSET_URL.to_string()
}

/// Parses a store id from a bare 32-byte hex string, an `0x`-prefixed hex string, or a
/// `urn:dig:chia:<store_id>[:<root>]` URN (the store-id body is taken).
fn parse_store_id(raw: &str) -> Result<Bytes32, CliError> {
    let trimmed = raw.trim();
    // Accept a full store/capsule URN by taking the store-id body.
    let body = trimmed
        .strip_prefix("urn:dig:chia:")
        .unwrap_or(trimmed)
        // A capsule URN pins a root after `:`; the store id is the part before it.
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches("0x");

    let bytes = hex::decode(body).map_err(|_| {
        CliError::InvalidArgument(format!(
            "store id must be 32-byte hex (64 hex chars), got: {raw}"
        ))
    })?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| {
        CliError::InvalidArgument(format!(
            "store id must be exactly 32 bytes (64 hex chars), got: {raw}"
        ))
    })?;
    Ok(Bytes32::new(arr))
}

/// The human-readable label/value rows for a [`StoreStatus`], in display order. A `None` optional
/// field renders as `-`. Pure (no I/O) so the rendering is unit-testable.
fn human_rows(status: &StoreStatus) -> Vec<(&'static str, String)> {
    let dash = || "-".to_string();
    let confirmations = match &status.confirmations {
        Some(c) => format!("{} / {}", c.have, c.target),
        None => dash(),
    };
    vec![
        ("Status", status_label(status.status).to_string()),
        ("Confirmations", confirmations),
        ("Store ID", status.store_id.clone()),
        (
            "Owner puzzle hash",
            status.owner_puzzle_hash.clone().unwrap_or_else(dash),
        ),
        ("Live root", status.live_root.clone().unwrap_or_else(dash)),
        (
            "Program hash",
            status.program_hash.clone().unwrap_or_else(dash),
        ),
        (
            "Head signature",
            status.head_signature.clone().unwrap_or_else(dash),
        ),
        ("Coin ID", status.coin_id.clone().unwrap_or_else(dash)),
        ("Verified", status.verified.to_string()),
        ("Generations", status.generation_count.to_string()),
    ]
}

/// The human label for a [`StoreStatusKind`].
fn status_label(kind: StoreStatusKind) -> &'static str {
    match kind {
        StoreStatusKind::Live => "live",
        StoreStatusKind::Melted => "melted",
        StoreStatusKind::NotFound => "not found",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dig_chainsource_interface::{ChainSourceError, MockChainSource};
    use dig_store::{Confirmations, DEFAULT_CONFIRMATION_TARGET};

    fn rows_map(status: &StoreStatus) -> std::collections::HashMap<&'static str, String> {
        human_rows(status).into_iter().collect()
    }

    fn hex64(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    // --- store-id parsing -------------------------------------------------------------------

    #[test]
    fn parses_bare_hex_store_id() {
        let id = parse_store_id(&hex64(0xab)).unwrap();
        assert_eq!(id, Bytes32::new([0xab; 32]));
    }

    #[test]
    fn parses_0x_prefixed_and_urn_forms() {
        let expected = Bytes32::new([0xcd; 32]);
        assert_eq!(
            parse_store_id(&format!("0x{}", hex64(0xcd))).unwrap(),
            expected
        );
        assert_eq!(
            parse_store_id(&format!("urn:dig:chia:{}", hex64(0xcd))).unwrap(),
            expected
        );
        // A capsule URN (store:root) resolves to the store-id body.
        assert_eq!(
            parse_store_id(&format!("urn:dig:chia:{}:{}", hex64(0xcd), hex64(0x11))).unwrap(),
            expected
        );
    }

    #[test]
    fn rejects_non_hex_and_wrong_length() {
        assert!(matches!(
            parse_store_id("zzzz").unwrap_err(),
            CliError::InvalidArgument(_)
        ));
        assert!(matches!(
            parse_store_id("abcd").unwrap_err(),
            CliError::InvalidArgument(_)
        ));
    }

    // --- coinset-url resolution -------------------------------------------------------------

    #[test]
    fn flag_beats_env_and_default() {
        assert_eq!(
            resolve_coinset_url(Some("https://flag.example")),
            "https://flag.example"
        );
    }

    #[test]
    fn falls_back_to_default_when_no_flag_or_env() {
        // Not asserting on the env var here (tests share a process); a None flag with no env set
        // must yield the crate default.
        if std::env::var(COINSET_URL_ENV).is_err() {
            assert_eq!(
                resolve_coinset_url(None),
                digstore_chain::config::DEFAULT_COINSET_URL
            );
        }
    }

    // --- rendering: Live / Melted / NotFound ------------------------------------------------

    fn live_status() -> StoreStatus {
        StoreStatus {
            status: StoreStatusKind::Live,
            store_id: hex64(0x01),
            confirmations: Some(Confirmations {
                have: 40,
                target: 32,
            }),
            owner_puzzle_hash: Some(hex64(0x02)),
            live_root: Some(hex64(0x03)),
            program_hash: Some(hex64(0x04)),
            head_signature: None,
            coin_id: Some(hex64(0x05)),
            verified: true,
            generation_count: 3,
        }
    }

    #[test]
    fn renders_live_status_fields() {
        let rows = rows_map(&live_status());
        assert_eq!(rows["Status"], "live");
        assert_eq!(rows["Confirmations"], "40 / 32");
        assert_eq!(rows["Store ID"], hex64(0x01));
        assert_eq!(rows["Owner puzzle hash"], hex64(0x02));
        assert_eq!(rows["Live root"], hex64(0x03));
        assert_eq!(rows["Program hash"], hex64(0x04));
        assert_eq!(rows["Head signature"], "-");
        assert_eq!(rows["Coin ID"], hex64(0x05));
        assert_eq!(rows["Verified"], "true");
        assert_eq!(rows["Generations"], "3");
    }

    #[test]
    fn renders_melted_status_with_dashes_and_generation_count() {
        let status = StoreStatus {
            status: StoreStatusKind::Melted,
            store_id: hex64(0x0a),
            confirmations: None,
            owner_puzzle_hash: None,
            live_root: None,
            program_hash: None,
            head_signature: None,
            coin_id: None,
            verified: false,
            generation_count: 7,
        };
        let rows = rows_map(&status);
        assert_eq!(rows["Status"], "melted");
        assert_eq!(rows["Confirmations"], "-");
        assert_eq!(rows["Live root"], "-");
        assert_eq!(rows["Coin ID"], "-");
        assert_eq!(rows["Verified"], "false");
        assert_eq!(rows["Generations"], "7");
    }

    #[test]
    fn renders_not_found_status() {
        let status = StoreStatus {
            status: StoreStatusKind::NotFound,
            store_id: hex64(0x0b),
            confirmations: None,
            owner_puzzle_hash: None,
            live_root: None,
            program_hash: None,
            head_signature: None,
            coin_id: None,
            verified: false,
            generation_count: 0,
        };
        let rows = rows_map(&status);
        assert_eq!(rows["Status"], "not found");
        assert_eq!(rows["Generations"], "0");
        assert_eq!(rows["Verified"], "false");
    }

    // --- JSON shape: StoreStatus serializes directly ----------------------------------------

    #[test]
    fn json_shape_matches_store_status_fields() {
        let value = serde_json::to_value(live_status()).unwrap();
        assert_eq!(value["status"], "live");
        assert_eq!(value["store_id"], hex64(0x01));
        assert_eq!(value["confirmations"]["have"], 40);
        assert_eq!(value["confirmations"]["target"], 32);
        assert_eq!(value["verified"], true);
        assert_eq!(value["generation_count"], 3);
        assert_eq!(value["head_signature"], serde_json::Value::Null);
    }

    // --- end-to-end through the library with the canonical mock source ----------------------

    #[test]
    fn empty_chain_reports_not_found() {
        // A launcher with no spend on chain → the library walk concludes NotFound. This exercises
        // the real `get_store_status` path against the canonical `MockChainSource`.
        let chain = MockChainSource::new().with_peak(1000);
        let status = get_store_status(
            &chain,
            Bytes32::new([0x42; 32]),
            DEFAULT_CONFIRMATION_TARGET,
        )
        .unwrap();
        assert_eq!(status.status, StoreStatusKind::NotFound);
        assert_eq!(status.generation_count, 0);
        assert!(!status.verified);
    }

    #[test]
    fn transport_failure_fails_closed_not_absent() {
        // A source that cannot answer must surface an error, NEVER a false NotFound.
        let chain = MockChainSource::new().fail_with(ChainSourceError::Timeout);
        let result = get_store_status(
            &chain,
            Bytes32::new([0x42; 32]),
            DEFAULT_CONFIRMATION_TARGET,
        );
        assert!(
            result.is_err(),
            "transport failure must not degrade to NotFound"
        );
    }
}
