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
fn init_when_disk_store_exists_but_registry_lost_fails_before_minting() {
    // Money-relevant guard: if the workspace registry (workspace.toml) and the
    // on-disk store layout disagree — e.g. a prior run minted + scaffolded the
    // store but the registry was lost/edited — a re-`init` must NOT mint a second
    // singleton (spending XCH) only to then hit the disk "already initialized"
    // check and orphan the fresh coin. The disk-level guard runs PRE-mint.
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    assert!(common::store_dir(&dir).join("config.toml").exists());

    // Drop the registry so the `ws.stores.contains_key` check would pass, leaving
    // only the disk-level guard to catch the still-scaffolded store on disk.
    let registry = dir.path().join(".dig").join("workspace.toml");
    assert!(registry.exists(), "registry written by first init");
    std::fs::remove_file(&registry).unwrap();

    // Second init: exit 2 (InvalidArgument) from the PRE-mint disk guard.
    dig(&dir)
        .arg("init")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("already initialized"));
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
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.current_dir(dir);
    common::seed_mock_env(&mut cmd, dir);
    cmd.arg("init").assert()
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

// --- on-chain mint gate (mocked) -----------------------------------------------
// These exercise the new `init` flow: it mints a store singleton via the env-gated
// in-memory mock (never the network) and the launcher id becomes the store_id.

#[test]
fn init_mints_and_confirms_against_mock() {
    let dir = tmp_dig();
    let out = dig(&dir).args(["--json", "init"]).output().unwrap();
    assert!(out.status.success(), "init should succeed (confirmed mock)");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["store_id"].as_str().is_some(), "carries a store_id");
    assert_eq!(v["anchor_status"], "confirmed");
    assert_eq!(v["mocked"], true);
    assert!(v["coin_id"].as_str().is_some());

    // anchor.toml exists in the store dir with status = "confirmed".
    let anchor = common::store_dir(&dir).join("anchor.toml");
    assert!(anchor.exists(), "anchor.toml written");
    let text = std::fs::read_to_string(&anchor).unwrap();
    assert!(
        text.contains("status = \"confirmed\""),
        "anchor.toml should be confirmed; got:\n{text}"
    );
}

#[test]
fn init_insufficient_funds_exits_12_and_creates_no_store() {
    let dir = tmp_dig();
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_BALANCE", "0")
        .arg("init")
        .assert()
        .failure()
        .code(12);
    // Hard gate: nothing on disk before the mint.
    assert!(
        !common::store_dir(&dir).exists(),
        "no store dir on insufficient funds"
    );
}

#[test]
fn init_confirm_timeout_exits_14_and_keeps_pending_store() {
    let dir = tmp_dig();
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["init", "--wait-timeout", "1"])
        .assert()
        .failure()
        .code(14);
    // The store IS created (resumable) and anchor.toml stays pending.
    let anchor = common::store_dir(&dir).join("anchor.toml");
    assert!(anchor.exists(), "store kept on confirm timeout");
    let text = std::fs::read_to_string(&anchor).unwrap();
    assert!(
        text.contains("status = \"pending\""),
        "anchor.toml should be pending; got:\n{text}"
    );
}

#[test]
fn init_without_seed_exits_9() {
    // A bare DIGSTORE_HOME with NO session → unlock yields NoSeed (exit 9).
    let dir = tmp_dig();
    let home = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .arg("--dig-dir")
        .arg(dir.path().join(".dig"))
        .current_dir(dir.path())
        .env("DIGSTORE_HOME", home.path())
        .env("DIGSTORE_ANCHOR_MOCK", "1")
        .arg("init")
        .assert()
        .failure()
        .code(9);
}
