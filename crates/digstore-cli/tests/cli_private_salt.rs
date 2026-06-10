mod common;
use common::{dig, store_id_and_root, tmp_dig};

#[test]
fn private_cat_without_salt_fails_with_salt_succeeds() {
    let dir = tmp_dig();
    let content = b"secret private payload";
    let f = dir.path().join("s.txt");
    std::fs::write(&f, content).unwrap();

    dig(&dir).args(["init", "--private"]).assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "s"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{}:{}/s", store_id, root);

    // WITHOUT salt -> wrong key -> AES-GCM tag fails -> exit 5.
    dig(&dir).args(["cat", &urn]).assert().failure().code(5);

    // WITH salt (read from the deterministic secret_salt.hex file) -> plaintext.
    let salt = std::fs::read_to_string(common::store_dir(&dir).join("secret_salt.hex")).unwrap();
    let salt = salt.trim();
    let out = dig(&dir)
        .args(["cat", &urn, "--salt", salt])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cat --salt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, content);
}
