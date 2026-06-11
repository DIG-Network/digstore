mod common;
use common::{dig, store_dir, tmp_dig};
use predicates::prelude::*;

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
