//! Layer 1 (signed root-revocation tombstones) end-to-end tests
//! (SECURITY.md residual #1, design §5).
//!
//! A publisher can retract a published root with a signed tombstone the remote
//! persists; clients verify each tombstone against the store-id-bound module key
//! and fail closed on a revoked root/store. An unsigned / wrong-key tombstone is
//! ignored (does not revoke).

mod common;
use common::{
    dig, genesis_push_sig, seed_tombstone, sign_root_tombstone, sign_store_tombstone,
    store_id_and_root, tmp_dig, TestServer,
};
use predicates::prelude::*;

fn host_pubkey(dir: &tempfile::TempDir) -> [u8; 48] {
    let text = std::fs::read_to_string(common::store_dir(dir).join("trusted_keys.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let hex = v[0]["public_key"].as_str().unwrap();
    let bytes = hex::decode(hex).unwrap();
    bytes.try_into().unwrap()
}

/// Build a committed source store and return (TempDir, store_id, root, module, pk).
fn make_source() -> (tempfile::TempDir, String, String, Vec<u8>, [u8; 48]) {
    let src = tmp_dig();
    let f = src.path().join("doc.txt");
    std::fs::write(&f, b"revocable content").unwrap();
    dig(&src).arg("init").assert().success();
    dig(&src)
        .args(["add"])
        .arg(&f)
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(&src).args(["commit"]).assert().success();
    let (store_id, root) = store_id_and_root(&src);
    let module = std::fs::read(
        common::store_dir(&src)
            .join("modules")
            .join(format!("{store_id}-{root}.dig")),
    )
    .unwrap();
    let pk = host_pubkey(&src);
    (src, store_id, root, module, pk)
}

#[test]
fn clone_fails_closed_on_signed_root_tombstone() {
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);

    // Seed a VALID signed Root tombstone for the served root.
    let (t, tsig) = sign_root_tombstone(&src, &store_id, &root);
    seed_tombstone(&server, &store_id, t, tsig);

    let dst = tmp_dig();
    let url = format!("{}/stores/{store_id}", server.base_url());
    dig(&dst)
        .args(["clone", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("revoked"));
}

#[test]
fn clone_fails_closed_on_signed_store_tombstone() {
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);

    // A Store-scoped tombstone refuses the WHOLE store, regardless of the root.
    let (t, tsig) = sign_store_tombstone(&src, &store_id);
    seed_tombstone(&server, &store_id, t, tsig);

    let dst = tmp_dig();
    let url = format!("{}/stores/{store_id}", server.base_url());
    dig(&dst)
        .args(["clone", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("revoked"));
}

#[test]
fn clone_ignores_unsigned_or_wrong_key_tombstone() {
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);

    // A tombstone for the served root, but signed with a DIFFERENT key (all-zero
    // / bogus signature): it must be IGNORED, so the root still serves.
    let store_id_b = digstore_core::Bytes32::from_hex(&store_id).unwrap();
    let root_b = digstore_core::Bytes32::from_hex(&root).unwrap();
    let bogus = digstore_core::Tombstone::root(
        store_id_b,
        root_b,
        1_700_000_000,
        digstore_core::RevocationReason::Compromise,
    );
    seed_tombstone(&server, &store_id, bogus, [0u8; 96]);

    let dst = tmp_dig();
    let url = format!("{}/stores/{store_id}", server.base_url());
    // Clone still succeeds and content reads back: the bogus tombstone did not revoke.
    dig(&dst).args(["clone", &url]).assert().success();
    let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
    let cat = dig(&dst).args(["cat", &urn]).output().unwrap();
    assert!(cat.status.success(), "cat must succeed: bogus tombstone ignored");
    assert_eq!(cat.stdout, b"revocable content");
}

#[test]
fn clone_serves_normally_when_not_revoked() {
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    // No tombstone at all → a normal, non-revoked clone.
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);

    let dst = tmp_dig();
    let url = format!("{}/stores/{store_id}", server.base_url());
    dig(&dst).args(["clone", &url]).assert().success();
    let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
    let cat = dig(&dst).args(["cat", &urn]).output().unwrap();
    assert!(cat.status.success());
    assert_eq!(cat.stdout, b"revocable content");
}

#[test]
fn revoke_command_publishes_then_clone_fails_closed() {
    // Full publisher loop: `digstore revoke --root <root>` signs a tombstone with
    // the store key and POSTs it (the remote verifies the signature before
    // storing); a SUBSEQUENT clone of that root then fails closed.
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);
    let url = format!("{}/stores/{store_id}", server.base_url());

    // The source store publishes the revocation through the configured remote.
    dig(&src)
        .args(["remote", "add", "origin", &url])
        .assert()
        .success();
    dig(&src)
        .args(["revoke", "--root", &root, "--reason", "compromise", "origin"])
        .assert()
        .success();

    // A fresh clone now refuses the revoked root.
    let dst = tmp_dig();
    dig(&dst)
        .args(["clone", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("revoked"));
}

#[test]
fn revoke_all_command_refuses_the_whole_store() {
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);
    let url = format!("{}/stores/{store_id}", server.base_url());

    dig(&src)
        .args(["remote", "add", "origin", &url])
        .assert()
        .success();
    dig(&src)
        .args(["revoke", "--all", "--reason", "takedown", "origin"])
        .assert()
        .success();

    let dst = tmp_dig();
    dig(&dst)
        .args(["clone", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("revoked"));
}

#[test]
fn remote_rejects_tombstone_with_bad_signature() {
    // The POST handler fails closed: a tombstone whose signature does not verify
    // against the store's published key is rejected (403), never persisted.
    let (src, store_id, root, module, pk) = make_source();
    let sig = genesis_push_sig(&src, &store_id, &root);
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);
    let url = format!("{}/stores/{store_id}/tombstone", server.base_url());

    let store_id_b = digstore_core::Bytes32::from_hex(&store_id).unwrap();
    let root_b = digstore_core::Bytes32::from_hex(&root).unwrap();
    let t = digstore_core::Tombstone::root(
        store_id_b,
        root_b,
        1,
        digstore_core::RevocationReason::Compromise,
    );
    use digstore_core::Encode;
    let body = serde_json::json!({
        "record": hex::encode(t.to_bytes()),
        "signature": hex::encode([0u8; 96]),
    });

    let rt = tokio::runtime::Runtime::new().unwrap();
    let status = rt.block_on(async {
        reqwest::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    assert_eq!(status, 403, "bad-signature tombstone must be rejected (403)");

    // And because it was never stored, a clone of the root still succeeds.
    let dst = tmp_dig();
    let clone_url = format!("{}/stores/{store_id}", server.base_url());
    dig(&dst).args(["clone", &clone_url]).assert().success();
}
