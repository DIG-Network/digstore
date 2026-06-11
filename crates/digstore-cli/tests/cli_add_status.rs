mod common;
use assert_cmd::Command;
use common::tmp_dig;

fn dig_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("digstore").unwrap();
    c.current_dir(dir);
    common::seed_mock_env(&mut c, dir);
    c
}

#[test]
fn add_then_status_shows_staged() {
    let dir = tmp_dig();
    dig_in(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join("readme.txt"), b"hello digstore world").unwrap();
    dig_in(dir.path())
        .args(["add", "readme.txt", "--key", "readme"])
        .assert()
        .success();
    dig_in(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicates::prelude::predicate::str::contains("readme"));
}

#[test]
fn add_without_store_fails_exit_3() {
    let dir = tmp_dig();
    std::fs::write(dir.path().join("x.txt"), b"x").unwrap();
    dig_in(dir.path())
        .args(["add", "x.txt"])
        .assert()
        .failure()
        .code(3);
}
