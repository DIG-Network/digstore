//! Integration tests for the CI auto-deploy feature: `digstore deploy` +
//! `digstore deploy-key export`, plus the `commit --push --json` regression.
//!
//! These exercise the real installed CLI end-to-end against an in-process §21
//! `TestServer`, under the offline mock-anchor harness (`seed_mock_env` +
//! `DIGSTORE_ANCHOR_MOCK`). The payoff test `deploy_advances_existing_store_*`
//! proves a FRESH checkout (no `.dig`) can advance an existing store and have the
//! DIGHub remote ACCEPT the push — which only works because `deploy` reconstructs
//! the store with the ORIGINAL publisher key (the remote pinned it at first push).

mod common;
use common::{dig, store_id_and_root, tmp_dig, TestServer};
use predicates::prelude::*;
use tempfile::TempDir;

/// Read the store's host/publisher public key (48 bytes) from trusted_keys.json.
fn host_pubkey(dir: &TempDir) -> [u8; 48] {
    let text = std::fs::read_to_string(common::store_dir(dir).join("trusted_keys.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let hex = v[0]["public_key"].as_str().unwrap();
    let bytes = hex::decode(hex).unwrap();
    bytes.try_into().unwrap()
}

/// The store id (hex) read from config.toml (stable from init).
fn store_id_of(dir: &TempDir) -> String {
    let cfg = std::fs::read_to_string(common::store_dir(dir).join("config.toml")).unwrap();
    let line = cfg.lines().find(|l| l.contains("store_id")).unwrap();
    line.split('"').nth(1).unwrap().to_string()
}

/// The root the in-process server currently serves for `store_id`, or `None` if
/// it has never received content (still genesis / all-zero root).
fn server_served_root(server: &TestServer, store_id_hex: &str) -> Option<String> {
    use digstore_remote::RemoteBackend;
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
    let head = server.backend().head_state(&store_id).ok()?;
    if head.served_root == digstore_core::Bytes32([0u8; 32]) {
        None
    } else {
        Some(head.served_root.to_hex())
    }
}

/// `deploy-key export` prints the store's publisher seed; it must equal the bytes
/// in `signing_key.bin` (the key that authorizes §21 head pushes for CI).
#[test]
fn deploy_key_export_matches_signing_key() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();

    let out = dig(&dir).args(["deploy-key", "export"]).output().unwrap();
    assert!(
        out.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8(out.stdout).unwrap();
    let printed = printed.trim();

    let on_disk = std::fs::read(common::store_dir(&dir).join("signing_key.bin")).unwrap();
    assert_eq!(
        printed,
        hex::encode(&on_disk),
        "exported key must equal signing_key.bin"
    );
    assert_eq!(printed.len(), 64, "deploy key is a 32-byte (64-hex) seed");
}

/// `deploy-key export --json` emits the seed under `deploy_key`.
#[test]
fn deploy_key_export_json() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let out = dig(&dir)
        .args(["deploy-key", "export", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let on_disk = std::fs::read(common::store_dir(&dir).join("signing_key.bin")).unwrap();
    assert_eq!(v["deploy_key"].as_str().unwrap(), hex::encode(&on_disk));
}

/// REGRESSION: `commit --push --json` must actually PUSH. `--push` exists for CI,
/// which runs with `--json`; before the fix the push only happened in the human
/// branch, so `--push --json` silently published nothing. This guards that bug.
#[test]
fn commit_push_json_actually_pushes() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pk = host_pubkey(&dir);
    let store_id = store_id_of(&dir);

    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dir)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();

    std::fs::write(dir.path().join("a.txt"), b"push me via json").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    let out = dig(&dir)
        .args(["commit", "-m", "first", "--push", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "json push commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["pushed"].as_bool(),
        Some(true),
        "json must report the push happened"
    );

    let (_sid, committed) = store_id_and_root(&dir);
    assert_eq!(
        server_served_root(&server, &store_id).as_deref(),
        Some(committed.as_str()),
        "commit --push --json must advance the remote"
    );
}

/// REGRESSION guard kept honest: `commit --json` WITHOUT `--push` still pushes
/// nothing and emits no `pushed` field (the existing offline-safety contract).
#[test]
fn commit_json_without_push_flag_does_not_push() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pk = host_pubkey(&dir);
    let store_id = store_id_of(&dir);
    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dir)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();

    std::fs::write(dir.path().join("a.txt"), b"no push").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    let out = dig(&dir)
        .args(["commit", "-m", "first", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("pushed").is_none(), "no --push => no push attempted");
    assert!(
        server_served_root(&server, &store_id).is_none(),
        "remote must stay at genesis"
    );
}

/// THE CORE TEST: a FRESH checkout (no `.dig`) advances an EXISTING store via
/// `digstore deploy`, and the DIGHub remote ACCEPTS the push because deploy
/// reconstructs the store with the ORIGINAL publisher key. This is the whole
/// feature; it fails with a clone-based approach (wrong publisher key → 403).
#[test]
fn deploy_advances_existing_store_with_original_publisher_key() {
    // --- Phase 1: the developer creates + first-publishes the store locally. ---
    let dev = tmp_dig();
    dig(&dev).arg("init").assert().success();
    let pk = host_pubkey(&dev);
    let store_id = store_id_of(&dev);

    // The hub pins THIS store's publisher key at first push. Start empty, push v1.
    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dev)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();
    std::fs::write(dev.path().join("index.html"), b"<h1>v1</h1>").unwrap();
    dig(&dev).args(["add", "index.html"]).assert().success();
    dig(&dev)
        .args(["commit", "-m", "v1", "--push"])
        .assert()
        .success();
    let (_sid, root_v1) = store_id_and_root(&dev);
    assert_eq!(
        server_served_root(&server, &store_id).as_deref(),
        Some(root_v1.as_str()),
        "v1 must be live on the remote"
    );

    // Export the deploy key (the seed CI will carry as a secret).
    let key_out = dig(&dev).args(["deploy-key", "export"]).output().unwrap();
    let deploy_key = String::from_utf8(key_out.stdout)
        .unwrap()
        .trim()
        .to_string();

    // --- Phase 2: a FRESH CI checkout deploys v2 (no `.dig` at all). ---
    let ci = tmp_dig();
    // The built site lives in `dist/` (the default output dir).
    let dist = ci.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), b"<h1>v2 from CI</h1>").unwrap();
    // A committable dig.toml pinning the store + remote.
    std::fs::write(
        ci.path().join("dig.toml"),
        format!("store-id = \"{store_id}\"\noutput-dir = \"dist\"\nremote = \"{store_url}\"\n"),
    )
    .unwrap();

    // Deploy: reconstruct + add + commit + push, with the deploy key + the chain
    // tip (the mock anchor's "on-chain root" is provided via env).
    let out = dig(&ci)
        .args(["deploy", "-m", "deploy v2", "--wait-timeout", "0", "--json"])
        .env("DIGSTORE_DEPLOY_KEY", &deploy_key)
        .env("DIGSTORE_ANCHOR_MOCK_CHAIN_ROOT", &root_v1)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "deploy failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    // The deploy published a NEW capsule and the remote ACCEPTED it.
    assert_eq!(
        v["pushed"].as_bool(),
        Some(true),
        "deploy must push to DIGHub"
    );
    let new_root = v["root"].as_str().unwrap().to_string();
    assert_ne!(new_root, root_v1, "deploy must advance to a NEW root");
    let capsule = v["capsule"].as_str().unwrap();
    assert_eq!(
        capsule,
        format!("{store_id}:{new_root}"),
        "capsule = storeId:rootHash"
    );

    assert_eq!(
        server_served_root(&server, &store_id).as_deref(),
        Some(new_root.as_str()),
        "the remote must now serve the CI-deployed v2 root (push accepted with the original key)"
    );
}

/// `deploy` refuses to run without a deploy key (the one irreducible CI secret
/// beyond the wallet), with a clear message — never a panic.
#[test]
fn deploy_without_deploy_key_errors_clearly() {
    let ci = tmp_dig();
    let store_id = "ab".repeat(32);
    let dist = ci.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), b"x").unwrap();
    std::fs::write(
        ci.path().join("dig.toml"),
        format!("store-id = \"{store_id}\"\noutput-dir = \"dist\"\n"),
    )
    .unwrap();

    dig(&ci)
        .args(["deploy", "--wait-timeout", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("deploy key"));
}

/// `deploy` requires a store id (from dig.toml or --store-id); a missing one is a
/// clean error, not a panic.
#[test]
fn deploy_without_store_id_errors_clearly() {
    let ci = tmp_dig();
    let dist = ci.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), b"x").unwrap();

    dig(&ci)
        .args(["deploy", "--wait-timeout", "0"])
        .env("DIGSTORE_DEPLOY_KEY", "ab".repeat(32))
        .assert()
        .failure()
        .stderr(predicate::str::contains("store id"));
}
