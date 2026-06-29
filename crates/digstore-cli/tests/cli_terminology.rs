//! Canonical terminology (SYSTEM.md "Canonical terminology & branding").
//!
//! The ecosystem-wide object vocabulary is `store` (the on-chain singleton identity)
//! and `capsule` (one generation = `storeId:rootHash`). "project" is NOT a
//! user-facing synonym — `--project`/`projects` survive ONLY as hidden back-compat
//! aliases. These tests pin the digstore CLI's USER-FACING wording to store/capsule,
//! while guarding that the machine contracts that must NOT change — `--json` field
//! names and on-disk file names — are left exactly as they were.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

/// Init → add → commit a single file, returning the temp store dir.
fn store_with_one_capsule() -> tempfile::TempDir {
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
    dig(&dir).args(["commit", "-m", "first"]).assert().success();
    dir
}

// --- Canonical wording in human output ---

#[test]
fn init_reports_initialized_store() {
    let dir = tmp_dig();
    // The canonical vocabulary is `store`; "project" must NOT appear in init output.
    dig(&dir).arg("init").assert().success().stdout(
        predicate::str::contains("Initialized store")
            .and(predicate::str::contains("project").not()),
    );
}

#[test]
fn log_does_not_say_project_or_generation() {
    let dir = store_with_one_capsule();
    // `log` lists published capsules; it must not leak the protocol word "generation"
    // nor the deprecated user-facing "project".
    dig(&dir).args(["log"]).assert().success().stdout(
        predicate::str::contains("generation")
            .not()
            .and(predicate::str::contains("project").not()),
    );
}

#[test]
fn status_does_not_say_project() {
    let dir = store_with_one_capsule();
    dig(&dir)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("project").not());
}

// --- Hidden back-compat aliases (must keep working, but are not advertised) ---

#[test]
fn projects_alias_lists_like_stores() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    // The hidden `projects` alias runs the same command as `stores`.
    dig(&dir).args(["projects"]).assert().success();
    // The canonical `stores` command works.
    dig(&dir).args(["stores"]).assert().success();
}

#[test]
fn project_flag_alias_selects_active_store() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    // `--project` is a hidden alias of `--store`; both select a store for the command.
    dig(&dir)
        .args(["--project", "default", "status"])
        .assert()
        .success();
    dig(&dir)
        .args(["--store", "default", "status"])
        .assert()
        .success();
}

/// The deprecated `project` vocabulary is HIDDEN from `--help`: top-level help must
/// not advertise a `projects` alias, and the global flag help must not say "project".
#[test]
fn help_does_not_advertise_project() {
    let out = dig(&tmp_dig()).args(["--help"]).output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        !help.contains("project"),
        "top-level --help must not mention `project`:\n{help}"
    );
}

// --- Regression guards: machine contracts must NOT change ---

#[test]
fn json_log_keys_are_unchanged() {
    let dir = store_with_one_capsule();
    let out = dig(&dir).args(["log", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // The wire contract keeps `id` and `root` — the friendly wording is human-only.
    assert!(v[0]["id"].is_u64(), "log --json must keep the `id` key");
    assert!(
        v[0]["root"].is_string(),
        "log --json must keep the `root` key"
    );
    // It must NOT have sprouted a renamed `deployment` key.
    assert!(v[0].get("deployment").is_none());
}

#[test]
fn on_disk_files_keep_their_names() {
    let dir = store_with_one_capsule();
    let store = common::store_dir(&dir);
    // On-disk layout is a contract for existing stores: do not rename these.
    assert!(
        store.join("roots.log").exists(),
        "roots.log must keep its name"
    );
    assert!(
        store.join("config.toml").exists(),
        "config.toml must keep its name"
    );
    assert!(
        dir.path().join(".dig").join("workspace.toml").exists(),
        "workspace.toml must keep its name"
    );
}
