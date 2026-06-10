mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

#[test]
fn help_lists_all_verbs() {
    let dir = tmp_dig();
    dig(&dir).arg("--help").assert().success().stdout(
        predicate::str::contains("init")
            .and(predicate::str::contains("commit"))
            .and(predicate::str::contains("cat"))
            .and(predicate::str::contains("clone"))
            .and(predicate::str::contains("push"))
            .and(predicate::str::contains("pull")),
    );
}

#[test]
fn init_creates_store_and_trusted_key() {
    let dir = tmp_dig();
    dig(&dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized store"));
    // Multi-store layout: with the workspace at <dir>/.dig, the default store's
    // files live under <dir>/.dig/stores/default/.
    let store = common::store_dir(&dir);
    assert!(store.join("config.toml").exists());
    assert!(store.join("trusted_keys.json").exists());
    assert!(store.join("modules").exists());
}

#[test]
fn init_twice_fails_with_exit_2() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir).arg("init").assert().failure().code(2);
}

#[test]
fn init_json_emits_store_id() {
    let dir = tmp_dig();
    let out = dig(&dir).args(["--json", "init"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["store_id"].as_str().is_some());
}

// --- .gitignore convenience (default `.dig/` layout) ---------------------------
// These run `init` WITHOUT `--dig-dir` (via `current_dir`) so the store is created
// as `<dir>/.dig` and the helper writes `<dir>/.gitignore`. (The tests above pass
// `--dig-dir <tempdir>`, whose basename is not `.dig`, so they are intentionally
// unaffected.)

use assert_cmd::Command;

fn init_in(dir: &std::path::Path) -> assert_cmd::assert::Assert {
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(dir)
        .arg("init")
        .assert()
}

fn gitignore_lines(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(".gitignore"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn init_creates_gitignore_ignoring_the_dig_store() {
    let dir = tmp_dig();
    init_in(dir.path()).success();

    assert!(dir.path().join(".dig").is_dir(), "store dir created");
    let lines = gitignore_lines(dir.path());
    assert!(
        lines.iter().any(|l| l == ".dig/" || l == ".dig"),
        ".gitignore must ignore the store dir; got {lines:?}"
    );
}

#[test]
fn init_appends_to_existing_gitignore_and_preserves_content() {
    let dir = tmp_dig();
    std::fs::write(dir.path().join(".gitignore"), "target/\n*.log\n").unwrap();
    init_in(dir.path()).success();

    let lines = gitignore_lines(dir.path());
    assert!(lines.iter().any(|l| l == "target/"), "existing entry kept");
    assert!(lines.iter().any(|l| l == "*.log"), "existing entry kept");
    assert!(lines.iter().any(|l| l == ".dig/"), "store dir appended");
}

#[test]
fn init_does_not_duplicate_existing_dig_entry() {
    let dir = tmp_dig();
    std::fs::write(dir.path().join(".gitignore"), ".dig/\n").unwrap();
    init_in(dir.path()).success();

    let count = gitignore_lines(dir.path())
        .iter()
        .filter(|l| l.as_str() == ".dig/" || l.as_str() == ".dig")
        .count();
    assert_eq!(count, 1, "must not duplicate the .dig entry");
}
