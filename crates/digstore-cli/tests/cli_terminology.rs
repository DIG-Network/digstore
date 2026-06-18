//! Terminology alignment with hub.dig.net.
//!
//! hub.dig.net abstracts protocol jargon into a friendly mental model: a `store`
//! is a **project** and a `generation` is a **deployment**. These tests pin the
//! digstore CLI's USER-FACING wording to that scheme, while guarding that the
//! machine contracts that must NOT change — `--json` field names and on-disk file
//! names — are left exactly as they were.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

/// Init → add → commit a single file, returning the temp project dir.
fn project_with_one_deployment() -> tempfile::TempDir {
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

// --- Friendly wording in human output ---

#[test]
fn init_reports_initialized_project() {
    let dir = tmp_dig();
    dig(&dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized project"));
}

#[test]
fn log_uses_deployment_not_generation() {
    let dir = project_with_one_deployment();
    dig(&dir).args(["log"]).assert().success().stdout(
        predicate::str::contains("deployment 0").and(predicate::str::contains("generation").not()),
    );
}

#[test]
fn status_uses_deployment_root() {
    let dir = project_with_one_deployment();
    dig(&dir)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deployment root"));
}

// --- Aliases (additive, backward-compatible behavior) ---

#[test]
fn projects_alias_lists_like_stores() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    // The friendly `projects` alias runs the same command as `stores`.
    dig(&dir).args(["projects"]).assert().success();
    // Backward-compat: the original `stores` command still works.
    dig(&dir).args(["stores"]).assert().success();
}

#[test]
fn project_flag_alias_selects_active_project() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    // `--project` is an alias of `--store`; both select a project for the command.
    dig(&dir)
        .args(["--project", "default", "status"])
        .assert()
        .success();
    dig(&dir)
        .args(["--store", "default", "status"])
        .assert()
        .success();
}

// --- Regression guards: machine contracts must NOT change ---

#[test]
fn json_log_keys_are_unchanged() {
    let dir = project_with_one_deployment();
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
    let dir = project_with_one_deployment();
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
