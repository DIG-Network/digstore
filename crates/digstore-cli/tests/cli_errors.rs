mod common;
use assert_cmd::Command;
use common::tmp_dig;
use predicates::prelude::*;

#[test]
fn missing_store_shows_help_hint() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["status"]) // no store here
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:").and(predicate::str::contains("digstore init")));
}
