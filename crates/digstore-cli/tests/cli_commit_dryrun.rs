//! `digstore commit --dry-run` — cost preview without spending (roadmap #13).
//!
//! Dry-run must compute + print the resulting version (root) and the exact
//! DIG/XCH cost WITHOUT spending, anchoring, or finalizing. These tests drive the
//! INSTALLED binary and assert nothing was published.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

/// `commit --dry-run` prints the root + cost and exits 0 WITHOUT publishing: no
/// new version lands and staging is preserved.
#[test]
fn dry_run_previews_cost_without_publishing() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"alpha beta gamma").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "a"])
        .assert()
        .success();

    // Dry-run: discloses the 100 DIG cost and says nothing was spent.
    dig(&dir)
        .args(["commit", "--dry-run"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("100")
                .and(predicate::str::contains("DIG"))
                .and(predicate::str::contains("NOTHING spent")),
        );

    // No deployment landed (log is empty)...
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deployment").not());

    // ...and staging is intact (the file is still staged).
    dig(&dir)
        .args(["staged"])
        .assert()
        .success()
        .stdout(predicate::str::contains("a"));
}

/// `commit --dry-run --json` is machine-readable: a `dry_run` flag, the root, the
/// capsule, and `spent: false`.
#[test]
fn dry_run_json_is_machine_readable() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"alpha beta gamma").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "a"])
        .assert()
        .success();

    let out = dig(&dir)
        .args(["commit", "--dry-run", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["dry_run"], serde_json::json!(true));
    assert_eq!(v["spent"], serde_json::json!(false));
    assert_eq!(v["cost_dig"], serde_json::json!(100_000));
    assert!(v["root"].as_str().unwrap().len() == 64, "64-hex root");
    assert!(v["capsule"].as_str().unwrap().contains(':'), "storeId:root");
}

/// Dry-run on empty staging fails fast (nothing to preview), before any cost work.
#[test]
fn dry_run_empty_staging_errors() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["commit", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing staged"));
}
