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
        .stdout(predicate::str::contains("Initialized digstore"));
    assert!(dir.path().join("config.toml").exists());
    assert!(dir.path().join("trusted_keys.json").exists());
    assert!(dir.path().join("modules").exists());
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
