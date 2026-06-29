//! DIG-cost preflight + up-front cost disclosure for `init` and `commit`.
//!
//! Per #111, **minting a store (`init`) is FREE of $DIG** — it pays only the XCH
//! network fee; the per-capsule $DIG price is charged on `commit` (a capsule). The
//! per-capsule DIG amount is dynamic + USD-pegged (the hub computes it; the CLI accepts
//! it via `--dig-amount`/`DIGSTORE_DIG_AMOUNT`/dig.toml `dig-amount`) with a 100 DIG
//! protocol default. These tests prove (a) `init` does NOT charge $DIG (succeeds with
//! zero DIG and discloses it is free of $DIG), (b) `commit` BLOCKS before any spend when
//! short on DIG (exit 12, staging intact), (c) `commit` discloses the DIG cost UP FRONT,
//! and (d) `commit` honors the configurable amount (flag/env). The mock anchor honors
//! `DIGSTORE_ANCHOR_MOCK_DIG` for the short-DIG cases.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

// --- init (minting is FREE of $DIG, #111) ---------------------------------

/// init with ZERO DIG SUCCEEDS — minting a store is free of $DIG (#111). The mint
/// charges only the XCH fee, so an empty DIG balance never blocks `init`.
#[test]
fn init_with_zero_dig_succeeds_minting_is_free() {
    let dir = tmp_dig();
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .arg("init")
        .assert()
        .success();
    // The store WAS created — minting needed no DIG.
    assert!(
        common::store_dir(&dir).exists(),
        "store dir created on a DIG-free mint"
    );
}

/// init discloses up front that minting is FREE of $DIG (no "DIG" cost figure on
/// the mint), pointing the $DIG charge at publish/commit instead.
#[test]
fn init_human_discloses_mint_is_free_of_dig() {
    let dir = tmp_dig();
    dig(&dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("free of $DIG"));
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

    // No deployment finalized.
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deployment").not());

    // Staging intact: the file is still staged.
    dig(&dir)
        .args(["staged"])
        .assert()
        .success()
        .stdout(predicate::str::contains("a"));
}

/// commit with default DIG → succeeds AND the human cost line is shown up front:
/// "100" + "DIG" (the 100 DIG protocol default).
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
        .stdout(predicate::str::contains("100.000").and(predicate::str::contains("DIG")));
}

// --- configurable DIG amount (dynamic, USD-pegged; passed in, not flat 100) ----

/// `commit --dry-run --dig-amount 87.5` previews the EXACT configured amount in the
/// machine-readable cost (not the flat 100 DIG default). Nothing is spent.
#[test]
fn commit_dig_amount_flag_sets_dry_run_cost() {
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
        .args(["--json", "commit", "--dry-run", "--dig-amount", "87.5"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // 87.5 DIG == 87_500 base units; not the 100_000 flat default.
    assert_eq!(v["cost_dig"].as_u64(), Some(87_500));
    assert_eq!(v["cost_dig_display"].as_str(), Some("87.500"));
}

/// `DIGSTORE_DIG_AMOUNT` env sets the amount when no flag is given (flag > env).
#[test]
fn commit_dig_amount_env_sets_dry_run_cost() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"x").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "a"])
        .assert()
        .success();

    let out = dig(&dir)
        .env("DIGSTORE_DIG_AMOUNT", "55")
        .args(["--json", "commit", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["cost_dig"].as_u64(), Some(55_000));
    // The flag still wins over the env.
    let out = dig(&dir)
        .env("DIGSTORE_DIG_AMOUNT", "55")
        .args(["--json", "commit", "--dry-run", "--dig-amount", "12"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["cost_dig"].as_u64(), Some(12_000));
}

/// A malformed `--dig-amount` is rejected at parse time (exit 2), never a panic.
#[test]
fn commit_rejects_bad_dig_amount() {
    let dir = tmp_dig();
    dig(&dir)
        .args(["commit", "--dry-run", "--dig-amount", "1.2345"])
        .assert()
        .failure()
        .code(2);
}
