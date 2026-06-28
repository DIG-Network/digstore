//! `digstore doctor` — pre-publish preflight (roadmap #13).
//!
//! `doctor` prints pass/fail for seed, funds, login, remote, and content dir, and
//! exits non-zero if a hard check fails. These tests drive the INSTALLED binary
//! against the mocked anchoring env (so funds/seed are satisfied) and the
//! reachability override, so they never touch the network.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

/// With seed unlocked + funded (mock), logged in, remote reachable, and a content
/// dir present, doctor passes and exits 0.
#[test]
fn doctor_all_green_passes() {
    let dir = tmp_dig();
    // A content dir for the default output (".") — the temp project dir itself
    // exists, so "content dir" passes with output-dir defaulting to ".".
    std::fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();

    dig(&dir)
        .env("DIGSTORE_DOCTOR_REMOTE_OK", "1")
        .args(["doctor"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Ready to publish")
                .and(predicate::str::contains("funds"))
                .and(predicate::str::contains("dighub login")),
        );
}

/// The funds check reflects the mocked DIG balance: with zero DIG, doctor reports
/// a funds failure and exits non-zero.
#[test]
fn doctor_reports_insufficient_funds() {
    let dir = tmp_dig();
    dig(&dir)
        .env("DIGSTORE_DOCTOR_REMOTE_OK", "1")
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .args(["doctor"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("funds"));
}

/// JSON mode emits a machine-readable check list with an overall `ok` flag.
#[test]
fn doctor_json_emits_checks() {
    let dir = tmp_dig();
    std::fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
    let out = dig(&dir)
        .env("DIGSTORE_DOCTOR_REMOTE_OK", "1")
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["checks"].is_array(), "doctor --json has a checks array");
    assert_eq!(v["ok"], serde_json::json!(true));
    // The funds check is present and passing under the funded mock.
    let funds = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "funds")
        .expect("a funds check");
    assert_eq!(funds["status"], serde_json::json!("pass"));
}

/// An unreachable remote is reported as a failing check (non-zero exit).
#[test]
fn doctor_reports_unreachable_remote() {
    let dir = tmp_dig();
    std::fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
    dig(&dir)
        .env("DIGSTORE_DOCTOR_REMOTE_DOWN", "1")
        .args(["doctor"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("default remote"));
}
