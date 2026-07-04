//! Integration tests for `digstore authorize-origin-as-writer` (#24), driven through the
//! INSTALLED-style compiled `digstore` binary against the seeded mock anchoring env
//! (`DIGSTORE_ANCHOR_MOCK`, see `common::seed_mock_env`).
//!
//! `authorize-origin-as-writer` is STORE-SCOPED like `anchor`/`revoke`/`deploy-key`: the CLI
//! dispatch resolves an existing workspace/store BEFORE the command body ever runs, so a
//! missing store (`NoStore`, exit 3) is the FIRST gate no matter what else is wrong with the
//! invocation. Once a store exists, the command's own order is: resolve + parse the pubkey
//! (explicit `--pubkey`, or well-known discovery), THEN load the store's on-chain identity,
//! THEN unlock the wallet and sync the on-chain singleton.
//!
//! The shared `assets::MockChainReads` backend (used by every Wave-B asset command) models
//! only a synthetic FUNDING coin — it has no existing minted DataStore singleton, so
//! `sync_datastore` cannot succeed under it. These tests therefore cover the command's OWN
//! responsibility end-to-end (dispatch gating, arg validation, wiring order, clean error
//! surfacing) up to that boundary. The actual on-chain delegation-merge behavior (a second
//! writer delegate never revoking the first) is proven on a REAL Chia simulator by
//! `digstore_chain::singleton::tests::owner_delegates_second_writer_without_revoking_the_first`.

mod common;
use common::{dig, tmp_dig, ABANDON_MNEMONIC};
use predicates::prelude::*;

/// A valid 96-hex BLS12-381 G1 pubkey (the ABANDON test wallet's index-0 synthetic key) to
/// pass via `--pubkey`, skipping well-known network discovery in tests that don't exercise it.
fn abandon_pubkey_hex() -> String {
    let keys = digstore_chain::keys::derive_wallet_keys(ABANDON_MNEMONIC).unwrap();
    hex::encode(keys.synthetic_pk.to_bytes())
}

/// Without an initialized store, even a well-formed `--pubkey` fails with the standard
/// "no store" error (exit 3) — the store-scoped dispatch gate runs before the command body.
#[test]
fn errors_when_store_is_not_initialized() {
    let dir = tmp_dig();
    let pubkey = abandon_pubkey_hex();
    dig(&dir)
        .args([
            "authorize-origin-as-writer",
            "hub.dig.net",
            "--pubkey",
            &pubkey,
        ])
        .assert()
        .failure()
        .code(3);
}

/// The store gate wins even over an otherwise-invalid `--pubkey`: a missing store is reported
/// (exit 3), not the pubkey's own format error — dispatch-level preconditions take precedence.
#[test]
fn store_gate_takes_precedence_over_a_malformed_pubkey() {
    let dir = tmp_dig();
    dig(&dir)
        .args([
            "authorize-origin-as-writer",
            "hub.dig.net",
            "--pubkey",
            "not-a-valid-pubkey",
        ])
        .assert()
        .failure()
        .code(3);
}

/// Once a store exists, a malformed `--pubkey` is rejected (exit 2, invalid-argument) BEFORE
/// any wallet/chain access.
#[test]
fn rejects_malformed_pubkey_after_store_init() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args([
            "authorize-origin-as-writer",
            "hub.dig.net",
            "--pubkey",
            "not-a-valid-pubkey",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid argument"));
}

/// The `--json` error shape for a bad pubkey matches the stable `{"ok":false,"error":{...}}`
/// contract (§6.2): a catalogued `code`, not prose-only.
#[test]
fn rejects_malformed_pubkey_json_error_shape() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let out = dig(&dir)
        .args([
            "--json",
            "authorize-origin-as-writer",
            "hub.dig.net",
            "--pubkey",
            "too-short",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert_eq!(v["error"]["code"], "INVALID_ARGUMENT");
}

/// Once a store exists, omitting `--pubkey` drives REAL well-known discovery. Against an
/// origin with nothing listening (a refused local connection), the fetch fails cleanly as a
/// network error (exit 6) rather than hanging or panicking.
#[test]
fn well_known_discovery_surfaces_a_clean_network_error_when_unreachable() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["authorize-origin-as-writer", "127.0.0.1:1"])
        .assert()
        .failure()
        .code(6);
}

/// With a real (mocked-mint) store and a valid `--pubkey`, the command reaches the on-chain
/// sync step and fails there with a clear chain error under the shared asset-command mock
/// (which models no existing minted singleton) — proving arg validation, store resolution,
/// and wallet unlock all wire correctly ahead of the chain call. See the module doc for why
/// the on-chain merge itself is proven elsewhere (the `digstore-chain` simulator test).
#[test]
fn reaches_chain_sync_after_store_init_and_fails_cleanly_under_mock() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pubkey = abandon_pubkey_hex();
    dig(&dir)
        .args([
            "authorize-origin-as-writer",
            "hub.dig.net",
            "--pubkey",
            &pubkey,
            "--dry-run",
        ])
        .assert()
        .failure()
        .code(13);
}
