//! Chain-verified clone (Phase B, SECURITY.md residual #6).
//!
//! `clone`/`pull` now verify the served root equals the store singleton's
//! CURRENT on-chain root (read via the launcher id embedded in the module's
//! `ChainState`), failing closed on mismatch or an unreachable chain. These
//! tests exercise that gate entirely offline through the `DIGSTORE_ANCHOR_MOCK`
//! seam (always set by `common::dig`) plus the per-command chain-root knobs:
//! - `DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT=<hex>` — the mocked on-chain root.
//! - `DIGSTORE_ANCHOR_MOCK_CHAIN_UNREACHABLE=1` — fail closed (chain unreachable).
//! - neither set — skip the comparison (so legacy remote tests stay green).

mod common;
use common::*;
use predicates::prelude::*;

/// Read the source store's host public key (48 bytes) from trusted_keys.json —
/// the exact mechanism used by `cli_remote_clone_push_pull.rs`.
fn host_pubkey(dir: &tempfile::TempDir) -> [u8; 48] {
    let text = std::fs::read_to_string(store_dir(dir).join("trusted_keys.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let hex = v[0]["public_key"].as_str().unwrap();
    let bytes = hex::decode(hex).unwrap();
    bytes.try_into().unwrap()
}

/// Publish a committed store to a `TestServer`; return (server, store_id, root).
/// The served module embeds a `ChainState` whose launcher id == the store id
/// (Phase A `commit` behavior), so the chain-root gate is active for a clone.
fn published_store() -> (TestServer, String, String) {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();
    dig(&dir).args(["commit", "-m", "x"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let module_path = store_dir(&dir)
        .join("modules")
        .join(format!("{store_id}-{root}.dig"));
    let module = std::fs::read(&module_path).unwrap();
    let pubkey = host_pubkey(&dir);
    let sig = genesis_push_sig(&dir, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pubkey, &module, sig);
    (server, store_id, root)
}

#[test]
fn clone_passes_when_onchain_root_matches() {
    let (server, store_id, root) = published_store();
    let dest = tmp_dig();
    let url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dest)
        .args(["clone", &url])
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT", &root)
        .assert()
        .success();
}

#[test]
fn clone_fails_closed_when_onchain_root_differs() {
    let (server, store_id, _root) = published_store();
    let dest = tmp_dig();
    let url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dest)
        .args(["clone", &url])
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT", "11".repeat(32))
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("on-chain root"));
}

#[test]
fn clone_fails_closed_when_chain_unreachable() {
    let (server, store_id, _root) = published_store();
    let dest = tmp_dig();
    let url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dest)
        .args(["clone", &url])
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_UNREACHABLE", "1")
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("unreachable"));
}
