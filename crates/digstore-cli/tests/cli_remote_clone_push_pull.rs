mod common;
use common::{dig, genesis_push_sig, store_id_and_root, tmp_dig, TestServer};
use predicates::prelude::*;

/// Read the source store's host public key (48 bytes) from trusted_keys.json.
fn host_pubkey(dir: &tempfile::TempDir) -> [u8; 48] {
    let text = std::fs::read_to_string(common::store_dir(dir).join("trusted_keys.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let hex = v[0]["public_key"].as_str().unwrap();
    let bytes = hex::decode(hex).unwrap();
    bytes.try_into().unwrap()
}

#[test]
fn remote_add_and_list_persists() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["remote", "add", "origin", "https://example/stores/abc"])
        .assert()
        .success();
    dig(&dir)
        .args(["remote", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("origin").and(predicate::str::contains("example")));
}

#[test]
fn clone_then_cat_round_trips_from_remote() {
    let src = tmp_dig();
    let content = b"served from a remote digstore";
    let f = src.path().join("doc.txt");
    std::fs::write(&f, content).unwrap();
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
    let sig = genesis_push_sig(&src, &store_id, &root);

    let server = TestServer::start_with_module(&store_id, &root, pk, &module, sig);
    let base = server.base_url();

    let dst = tmp_dig();
    // Clone into an empty project dir: the dir exists (so `current_dir` is valid)
    // but carries no workspace yet — `clone` creates `<dst>/.dig`.
    let url = format!("{base}/stores/{store_id}");
    dig(&dst).args(["clone", &url]).assert().success();

    let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
    let cat = dig(&dst).args(["cat", &urn]).output().unwrap();
    assert!(
        cat.status.success(),
        "cat after clone failed: {}",
        String::from_utf8_lossy(&cat.stderr)
    );
    assert_eq!(cat.stdout, content);
}

#[test]
fn clone_rejects_unauthenticated_or_forged_head() {
    // A real, self-consistent module is served, but with a BOGUS publisher
    // signature over the served root. verify_module_root would pass (the module
    // is genuine and self-certifying), so this isolates the §21.6 authenticated-
    // head check: clone MUST fail closed because the served root is not validly
    // signed by the store key. This is the attack a malicious origin would mount.
    let src = tmp_dig();
    let f = src.path().join("doc.txt");
    std::fs::write(&f, b"content").unwrap();
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

    // Bogus 96-byte signature (does not verify against the store key).
    let server = TestServer::start_with_module(&store_id, &root, pk, &module, [0u8; 96]);
    let base = server.base_url();

    let dst = tmp_dig();
    let url = format!("{base}/stores/{store_id}");
    dig(&dst)
        .args(["clone", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("signature").or(predicate::str::contains("verif")));
}

#[test]
fn push_fast_forward_then_pull_advances() {
    let src = tmp_dig();
    let f = src.path().join("a.txt");
    std::fs::write(&f, b"v1").unwrap();
    dig(&src).arg("init").assert().success();
    dig(&src)
        .args(["add"])
        .arg(&f)
        .args(["--key", "a"])
        .assert()
        .success();
    dig(&src).args(["commit"]).assert().success();

    let (store_id, root1) = store_id_and_root(&src);
    let pk = host_pubkey(&src);

    // Empty server (genesis = empty module). Push advances it to root1, then root2.
    let server = TestServer::start_empty(&store_id, pk);
    let base = server.base_url();
    let store_url = format!("{base}/stores/{store_id}");

    dig(&src)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();
    dig(&src).args(["push", "origin"]).assert().success();

    // Clone into a fresh project dir (empty, but present for `current_dir`).
    let dst = tmp_dig();
    dig(&dst).args(["clone", &store_url]).assert().success();

    // Second commit on the source, then push.
    std::fs::write(&f, b"v2-longer-content-here").unwrap();
    dig(&src)
        .args(["add"])
        .arg(&f)
        .args(["--key", "a"])
        .assert()
        .success();
    dig(&src).args(["commit"]).assert().success();
    dig(&src).args(["push", "origin"]).assert().success();

    let (_sid2, root2) = store_id_and_root(&src);
    assert_ne!(root1, root2);

    // Pull on the clone advances local root to root2.
    dig(&dst)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();
    dig(&dst).args(["pull", "origin"]).assert().success();
    let outd: serde_json::Value =
        serde_json::from_slice(&dig(&dst).args(["log", "--json"]).output().unwrap().stdout)
            .unwrap();
    assert_eq!(outd[0]["root"].as_str().unwrap(), root2);
}
