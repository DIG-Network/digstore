//! `digs` is a FIRST-CLASS alias binary for `digstore` (issue #434): the two bins
//! share ONE codepath (`digstore_cli::run()`), expose the SAME command surface, and
//! each reflects its OWN invoked name (arg0) in `--help`/`--version`/completions/
//! `--help-json` — so `digs <args>` behaves identically to `digstore <args>`.
//!
//! These run against the REAL built binaries via `assert_cmd::cargo_bin`, so they
//! also prove the second `[[bin]]` target actually builds + installs.

mod common;
use assert_cmd::Command;
use common::tmp_dig;
use predicates::prelude::*;
use tempfile::TempDir;

/// A `digs` invocation mirroring `common::dig` (the seeded mock-anchoring env) but
/// against the `digs` alias binary — proves dispatch works end-to-end under `digs`.
fn digs(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("digs").unwrap();
    cmd.arg("--dig-dir")
        .arg(dir.path().join(".dig"))
        .current_dir(dir.path());
    common::seed_mock_env(&mut cmd, dir.path());
    cmd
}

/// Both binaries build/run, and each `--version` reports the SAME semver — with its
/// OWN program name (clap prints "<bin> <semver>"): `digs 0.x.y` vs `digstore 0.x.y`.
#[test]
fn digs_and_digstore_report_the_same_version() {
    let ds_out = Command::cargo_bin("digstore")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    let dg_out = Command::cargo_bin("digs")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert!(ds_out.status.success() && dg_out.status.success());
    let ds = String::from_utf8_lossy(&ds_out.stdout);
    let dg = String::from_utf8_lossy(&dg_out.stdout);
    // The trailing semver token must match; the leading program name differs.
    let ds_ver = ds.split_whitespace().last().unwrap();
    let dg_ver = dg.split_whitespace().last().unwrap();
    assert_eq!(ds_ver, dg_ver, "same version: `{ds}` vs `{dg}`");
    assert!(
        ds.starts_with("digstore "),
        "digstore leads with its name: {ds}"
    );
    assert!(
        dg.starts_with("digs "),
        "digs leads with its own name: {dg}"
    );
}

/// `digs --help` renders its OWN name in the usage line, not a hardcoded "digstore".
#[test]
fn digs_help_usage_shows_digs() {
    Command::cargo_bin("digs")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: digs"));
}

/// The two bins expose the IDENTICAL command surface: the `--help-json` `commands`,
/// `globals`, `exit_codes`, and `version` are byte-equal — only the `name` differs,
/// each reporting its own invoked binary name.
#[test]
fn digs_and_digstore_share_the_same_command_surface() {
    let ds = Command::cargo_bin("digstore")
        .unwrap()
        .arg("--help-json")
        .output()
        .unwrap();
    let dg = Command::cargo_bin("digs")
        .unwrap()
        .arg("--help-json")
        .output()
        .unwrap();
    assert!(ds.status.success() && dg.status.success());
    let dsv: serde_json::Value = serde_json::from_slice(&ds.stdout).unwrap();
    let dgv: serde_json::Value = serde_json::from_slice(&dg.stdout).unwrap();
    // Each reports its own invoked name.
    assert_eq!(dsv["name"].as_str(), Some("digstore"));
    assert_eq!(dgv["name"].as_str(), Some("digs"));
    // Everything else is identical — the SAME CLI, byte-for-byte.
    assert_eq!(dsv["commands"], dgv["commands"], "identical command tree");
    assert_eq!(dsv["globals"], dgv["globals"], "identical global flags");
    assert_eq!(dsv["exit_codes"], dgv["exit_codes"], "identical exit codes");
    assert_eq!(dsv["version"], dgv["version"], "identical version");
}

/// `digs completion bash` generates a completion script for `digs` (the invoked
/// name) so tab-completion works for the alias — it must NOT emit a script bound to
/// "digstore". (Note "digstore" contains the substring "digs", so we assert the
/// ABSENCE of "digstore" to make the check discriminating.)
#[test]
fn digs_completion_targets_digs() {
    let out = Command::cargo_bin("digs")
        .unwrap()
        .args(["completion", "bash"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let script = String::from_utf8_lossy(&out.stdout);
    assert!(
        script.contains("_digs"),
        "registers the `_digs` completion fn"
    );
    assert!(
        !script.contains("digstore"),
        "the `digs` completion script must not reference `digstore`:\n{script}"
    );
}

/// A representative help flow is identical under `digs`: `digs new --help` shows the
/// same free-scaffolder help (templates + "NO spend") as `digstore new --help`.
#[test]
fn digs_new_help_matches_digstore() {
    let d = tmp_dig();
    Command::cargo_bin("digs")
        .unwrap()
        .current_dir(d.path())
        .args(["new", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("static-site").and(predicate::str::contains("NO spend")));
}

/// `digs` runs the SAME dispatch path: `digs --json status` with no store fails with
/// the identical structured NO_STORE / exit-3 envelope that `digstore` produces.
#[test]
fn digs_dispatches_commands_like_digstore() {
    let d = tmp_dig();
    let out = digs(&d).args(["--json", "status"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "NO_STORE exits 3 under digs too"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert_eq!(v["error"]["code"].as_str(), Some("NO_STORE"));
}
