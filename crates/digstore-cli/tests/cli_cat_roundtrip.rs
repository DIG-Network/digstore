mod common;
use common::{dig, store_id_and_root, tmp_dig};

#[test]
fn add_commit_cat_round_trips_public_store() {
    let dir = tmp_dig();
    let content = b"the quick brown fox jumps over the lazy dog 1234567890";
    let f = dir.path().join("doc.txt");
    std::fs::write(&f, content).unwrap();

    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{}:{}/doc", store_id, root);
    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(
        out.status.success(),
        "cat failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, content, "cat must return original plaintext");
}

#[test]
fn cat_with_verify_proof_succeeds() {
    let dir = tmp_dig();
    let content = b"verified content here";
    let f = dir.path().join("doc.txt");
    std::fs::write(&f, content).unwrap();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();
    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{}:{}/doc", store_id, root);
    let out = dig(&dir)
        .args(["cat", &urn, "--verify-proof"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cat --verify-proof failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, content);
}

#[test]
fn multi_chunk_resource_round_trips() {
    let dir = tmp_dig();
    let mut content = Vec::with_capacity(700 * 1024);
    for i in 0..(700 * 1024) {
        content.push((i % 251) as u8);
    }
    let f = dir.path().join("big.bin");
    std::fs::write(&f, &content).unwrap();

    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "big"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{}:{}/big", store_id, root);
    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(
        out.status.success(),
        "cat failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout, content,
        "multi-chunk cat must reassemble exactly"
    );
}

#[test]
fn cat_keyless_urn_resolves_to_index_html_default_view() {
    // §8.5 social convention: a URN with NO resource key resolves to the store's
    // landing resource `index.html` (its default view) when that key exists.
    let dir = tmp_dig();
    let content = b"<html><body>landing page default view</body></html>";
    let f = dir.path().join("index.html");
    std::fs::write(&f, content).unwrap();

    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "index.html"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    // Key-less URN: no trailing `/resource` segment.
    let urn = format!("urn:dig:chia:{}:{}", store_id, root);
    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(
        out.status.success(),
        "cat of key-less URN failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout, content,
        "key-less URN must serve index.html's content (§8.5 default view)"
    );
}

#[test]
fn cat_unknown_resource_decoy_fails_verification_exit_5() {
    let dir = tmp_dig();
    let f = dir.path().join("doc.txt");
    std::fs::write(&f, b"real content here").unwrap();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();
    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{}:{}/does-not-exist", store_id, root);
    dig(&dir).args(["cat", &urn]).assert().failure().code(5);
}
