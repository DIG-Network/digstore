mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

#[test]
fn commit_creates_module_and_log_lists_it() {
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
        .stdout(predicate::str::contains("committed root"));
    let modules: Vec<_> = std::fs::read_dir(common::store_dir(&dir).join("modules"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "dig").unwrap_or(false))
        .collect();
    assert_eq!(modules.len(), 1);
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation 0"));
}

#[test]
fn commit_with_nothing_staged_fails_exit_2() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir).args(["commit"]).assert().failure().code(2);
}
