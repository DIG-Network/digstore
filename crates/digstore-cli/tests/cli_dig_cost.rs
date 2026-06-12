//! DIG-cost preflight + up-front cost disclosure for `init` and `commit`.
//!
//! Every `init` pays 100 DIG and every `commit` pays 10 DIG, embedded in the
//! on-chain bundle. These tests prove the CLI (a) BLOCKS before any spend when
//! the wallet is short on DIG (exit 12, leaving nothing on disk / no new
//! generation), and (b) discloses the DIG cost UP FRONT in human output. The
//! mock anchor honors `DIGSTORE_ANCHOR_MOCK_DIG` for the short-DIG cases.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

// --- init -----------------------------------------------------------------

/// init with zero DIG → exit 12, stderr mentions DIG, and NO store dir is
/// created (the DIG preflight runs BEFORE any mint/scaffold, like the XCH one).
#[test]
fn init_insufficient_dig_exits_12_and_creates_no_store() {
    let dir = tmp_dig();
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .arg("init")
        .assert()
        .failure()
        .code(12)
        .stderr(predicate::str::contains("DIG"));
    // Hard gate: nothing on disk before the (would-be) mint.
    assert!(
        !common::store_dir(&dir).exists(),
        "no store dir on insufficient DIG"
    );
}

/// init with default (seeded) mock DIG → succeeds AND the human cost line is
/// shown up front: "100" + "DIG".
#[test]
fn init_human_discloses_dig_cost() {
    let dir = tmp_dig();
    dig(&dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("100").and(predicate::str::contains("DIG")));
}

// --- commit ---------------------------------------------------------------

/// commit with zero DIG on an already-committed store → exit 12, stderr mentions
/// DIG, and NO new generation lands (staging stays intact). The DIG preflight
/// runs BEFORE the on-chain update.
#[test]
fn commit_insufficient_dig_exits_12_and_keeps_staging() {
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

    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .args(["commit", "-m", "first"])
        .assert()
        .failure()
        .code(12)
        .stderr(predicate::str::contains("DIG"));

    // No generation finalized.
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation").not());

    // Staging intact: the file is still staged.
    dig(&dir)
        .args(["staged"])
        .assert()
        .success()
        .stdout(predicate::str::contains("a"));
}

/// commit with default DIG → succeeds AND the human cost line is shown up front:
/// "10" + "DIG".
#[test]
fn commit_human_discloses_dig_cost() {
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
    dig(&dir)
        .args(["commit", "-m", "first"])
        .assert()
        .success()
        .stdout(predicate::str::contains("10").and(predicate::str::contains("DIG")));
}
