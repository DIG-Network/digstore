mod common;
use common::{corrupt_data_section, dig, store_id_and_root, tmp_dig};

/// A flipped byte inside the module's injected data section (the served content
/// pool) must make CLIENT-side merkle/GCM verification fail with exit 5. The
/// module still instantiates (no code corruption); the failure is purely
/// client-side (§9.3), exactly as a tampered served payload should behave.
#[test]
fn tampered_module_data_section_fails_client_verification_exit_5() {
    let dir = tmp_dig();
    std::fs::write(
        dir.path().join("doc.txt"),
        b"important verified content that spans one chunk",
    )
    .unwrap();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(dir.path().join("doc.txt"))
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let module = dir
        .path()
        .join("modules")
        .join(format!("{store_id}-{root}.dig"));
    corrupt_data_section(&module);

    let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
    dig(&dir).args(["cat", &urn]).assert().failure().code(5);
}

/// Sanity: without corruption, the same cat round-trips successfully (so the
/// failure above is genuinely caused by the tamper, not an unrelated error).
#[test]
fn untampered_module_cats_successfully() {
    let dir = tmp_dig();
    let content = b"important verified content that spans one chunk";
    std::fs::write(dir.path().join("doc.txt"), content).unwrap();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(dir.path().join("doc.txt"))
        .args(["--key", "doc"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, content);
}
