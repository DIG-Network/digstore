mod common;
use assert_cmd::Command;
use common::tmp_dig;
use predicates::prelude::*;

#[test]
fn add_help_shows_examples() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["add", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("EXAMPLES").and(predicate::str::contains("digstore add -A")),
        );
}
