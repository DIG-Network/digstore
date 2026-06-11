//! End-to-end tests for the seed/lock commands. Each test points
//! `DIGSTORE_HOME` at a fresh tempdir so the real `~/.dig` is never touched,
//! and supplies the passphrase via `DIGSTORE_PASSPHRASE` (non-interactive).

use assert_cmd::Command;
use predicates::str::contains;

fn digstore(home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.env("DIGSTORE_HOME", home);
    cmd.env("DIGSTORE_PASSPHRASE", "test-pass");
    cmd
}

#[test]
fn status_reports_no_seed_initially() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("no seed"));
}

#[test]
fn generate_then_status_unlocked() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path())
        .args(["seed", "generate"])
        .assert()
        .success();
    assert!(home.path().join("seed.enc").exists());
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("present, unlocked"));
}

#[test]
fn lock_then_status_locked() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path())
        .args(["seed", "generate"])
        .assert()
        .success();
    digstore(home.path()).args(["lock"]).assert().success();
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("present, locked"));
}

#[test]
fn import_known_mnemonic_round_trips() {
    let home = tempfile::tempdir().unwrap();
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    digstore(home.path())
        .args(["seed", "import", "--mnemonic", PHRASE])
        .assert()
        .success();
    digstore(home.path())
        .args(["seed", "status"])
        .assert()
        .success()
        .stdout(contains("present, unlocked"));
}

#[test]
fn import_rejects_bad_mnemonic() {
    let home = tempfile::tempdir().unwrap();
    digstore(home.path())
        .args(["seed", "import", "--mnemonic", "not a real mnemonic at all"])
        .assert()
        .failure()
        .stderr(contains("invalid mnemonic"));
}
