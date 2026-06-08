mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

#[test]
fn add_then_status_shows_staged() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("readme.txt");
    std::fs::write(&f, b"hello digstore world").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "readme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("staged readme"));
    dig(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("staged: readme"));
}

#[test]
fn add_without_store_fails_exit_3() {
    let dir = tmp_dig();
    let f = dir.path().join("x.txt");
    std::fs::write(&f, b"x").unwrap();
    dig(&dir).args(["add"]).arg(&f).assert().failure().code(3);
}
