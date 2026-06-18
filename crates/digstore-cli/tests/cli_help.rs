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

/// Terminology alignment with hub.dig.net: a generation's history reads as
/// "deployment history" in help, not the protocol word "generation".
#[test]
fn log_help_uses_deployment_history() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["log", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("deployment history")
                .and(predicate::str::contains("generation (commit) history").not()),
        );
}

/// `checkout --help` describes materializing a "deployment root", not a
/// "generation root".
#[test]
fn checkout_help_uses_deployment_root() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["checkout", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deployment root"));
}

/// The `projects` alias is discoverable: `digstore projects --help` resolves
/// (proves the alias exists) and the old `stores --help` still works.
#[test]
fn projects_alias_help_resolves() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["projects", "--help"])
        .assert()
        .success();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["stores", "--help"])
        .assert()
        .success();
}
