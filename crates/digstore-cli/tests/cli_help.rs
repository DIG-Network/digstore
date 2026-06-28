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

/// Roadmap #14: top-level help leads with the TASK, not protocol jargon. `commit`
/// reads as "Publish your staged files as a new version", and the new free-loop
/// commands (`new`, `dev`, `doctor`) are present and task-described.
#[test]
fn top_level_help_is_task_first() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Publish your staged files as a new version")
                .and(
                    predicate::str::contains("free, no wallet, no spend")
                        .or(predicate::str::contains("template — free")),
                )
                .and(predicate::str::contains("Preview your project locally")),
        );
}

/// `digstore new --help` documents the free, no-mint scaffolder and lists templates.
#[test]
fn new_help_lists_templates_and_says_free() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["new", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("NO spend")
                .and(predicate::str::contains("static-site"))
                .and(predicate::str::contains("dapp-window-chia")),
        );
}

/// `digstore dev --help` frames the free local preview loop (real read path, no spend).
#[test]
fn dev_help_describes_free_local_loop() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["dev", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no spend").and(predicate::str::contains("window.chia")));
}

/// `digstore commit --help` documents `--dry-run` as a cost preview that spends nothing.
#[test]
fn commit_help_documents_dry_run() {
    let d = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(d.path())
        .args(["commit", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--dry-run").and(predicate::str::contains("WITHOUT spending")),
        );
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
