mod common;
use common::{dig, store_dir, tmp_dig};
use predicates::prelude::*;

/// `anchor inspect` on a file that is not a digstore module fails with a clean
/// "not a digstore module" message (exit 2), not a raw wasm-decoder dump.
#[test]
fn anchor_inspect_non_module_errors_cleanly() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let junk = dir.path().join("notamodule.dig");
    std::fs::write(&junk, b"definitely not a wasm module").unwrap();
    dig(&dir)
        .args(["anchor", "inspect"])
        .arg(&junk)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("not a digstore module"));
}

/// `digstore anchor status` on a freshly-init'd (confirmed) store: prints the
/// store_id + "confirmed", and `--json` reports the persisted + live state.
#[test]
fn anchor_status_on_confirmed_store() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();

    // Human status: shows the store_id and the on-chain confirmed line.
    let cfg = std::fs::read_to_string(store_dir(&dir).join("config.toml")).unwrap();
    let store_id = cfg
        .lines()
        .find(|l| l.contains("store_id"))
        .and_then(|l| l.split('"').nth(1))
        .unwrap()
        .to_string();
    dig(&dir)
        .args(["anchor", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&store_id).and(predicate::str::contains("confirmed")));

    // JSON status: persisted confirmed + live confirmed + mocked flag.
    let out = dig(&dir)
        .args(["--json", "anchor", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "confirmed");
    assert_eq!(v["onchain_confirmed"], true);
    assert_eq!(v["mocked"], true);
    assert_eq!(v["store_id"].as_str().unwrap(), store_id);
}

/// Resume: init with a confirm timeout leaves a pending anchor (exit 14); a
/// later `digstore anchor` (no timeout env) flips it to confirmed (exit 0).
#[test]
fn anchor_resume_flips_pending_to_confirmed() {
    let dir = tmp_dig();
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["init", "--wait-timeout", "1"])
        .assert()
        .failure()
        .code(14);

    let anchor = store_dir(&dir).join("anchor.toml");
    let text = std::fs::read_to_string(&anchor).unwrap();
    assert!(
        text.contains("status = \"pending\""),
        "anchor.toml pending after timeout; got:\n{text}"
    );

    // Resume WITHOUT the timeout env → the mock now confirms.
    dig(&dir)
        .arg("anchor")
        .assert()
        .success()
        .stdout(predicate::str::contains("confirmed"));

    let text = std::fs::read_to_string(&anchor).unwrap();
    assert!(
        text.contains("status = \"confirmed\""),
        "anchor.toml flipped to confirmed; got:\n{text}"
    );
}

/// `digstore anchor` on an already-confirmed store: exit 0, "already confirmed".
#[test]
fn anchor_resume_already_confirmed_is_noop() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();

    dig(&dir)
        .arg("anchor")
        .assert()
        .success()
        .stdout(predicate::str::contains("already confirmed"));
}

/// Resume a still-pending anchor (TIMEOUT env on both init AND resume): exit 14,
/// anchor.toml stays `status = "pending"`.
#[test]
fn anchor_resume_still_pending_exits_14() {
    let dir = tmp_dig();
    // Init with a mock that times out → leaves anchor.toml pending (exit 14).
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["init", "--wait-timeout", "1"])
        .assert()
        .failure()
        .code(14);

    // Resume also with the timeout env and --wait-timeout 0 → still pending (exit 14).
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["anchor", "--wait-timeout", "0"])
        .assert()
        .failure()
        .code(14);

    let anchor = store_dir(&dir).join("anchor.toml");
    let text = std::fs::read_to_string(&anchor).unwrap();
    assert!(
        text.contains("status = \"pending\""),
        "anchor.toml must still be pending after failed resume; got:\n{text}"
    );
}

/// `digstore anchor status --json` on a pending store: exit 0, JSON reports
/// `status == "pending"` and `onchain_confirmed == false`.
#[test]
fn anchor_status_json_on_pending_store() {
    let dir = tmp_dig();
    // Init with a mock that times out → leaves anchor.toml pending.
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["init", "--wait-timeout", "1"])
        .assert()
        .failure()
        .code(14);

    // `anchor status --json` with the timeout env active so the live poll also
    // returns pending (timeout 0 inside status).
    let out = dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["--json", "anchor", "status"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "anchor status must exit 0; got: {:?}",
        out.status
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "pending", "persisted status must be pending");
    assert_eq!(
        v["onchain_confirmed"], false,
        "live poll must report not confirmed"
    );
}

/// `digstore anchor status --json` includes a `module_chain_state` field
/// reflecting the embedded on-chain pointer in the current module.
#[test]
fn anchor_status_shows_module_chain_pointer() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();
    dig(&dir).args(["commit", "-m", "x"]).assert().success();
    let out = dig(&dir)
        .args(["--json", "anchor", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["module_chain_state"]["network"].as_str(), Some("mainnet"));
    assert!(v["module_chain_state"]["coin_id"].as_str().is_some());
}

/// `digstore anchor inspect <module>` decodes and prints the embedded chain
/// pointer from the compiled module file.
#[test]
fn anchor_inspect_dumps_a_module_pointer() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();
    dig(&dir).args(["commit", "-m", "x"]).assert().success();
    let modules = store_dir(&dir).join("modules");
    let module = std::fs::read_dir(&modules)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "dig").unwrap_or(false))
        .unwrap();
    let out = dig(&dir)
        .args(["--json", "anchor", "inspect", module.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["network"].as_str(), Some("mainnet"));
    assert!(v["launcher_id"].as_str().is_some());
}

/// `digstore anchor` on a store that is not anchored (no anchor.toml): chain
/// error (exit 13) pointing at `digstore init`.
#[test]
fn anchor_when_not_anchored_errors() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();

    // Remove the anchor record to model an un-anchored store.
    let anchor = store_dir(&dir).join("anchor.toml");
    std::fs::remove_file(&anchor).unwrap();

    dig(&dir)
        .arg("anchor")
        .assert()
        .failure()
        .code(13)
        .stderr(predicate::str::contains("not anchored"));
}
