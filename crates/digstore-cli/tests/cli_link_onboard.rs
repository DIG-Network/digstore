//! Integration tests for the web↔CLI bridge + onboarding + machine-readable
//! surface: `digstore link`, `digstore setup`/`auth`, `digstore completion`, and
//! `digstore --help-json`. These exercise the real installed binary end-to-end.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;
use std::fs;

/// `digstore link <storeId>` writes a committable dig.toml pinning the store +
/// remote, WITHOUT minting, spending, or needing a seed. The first redeploy step
/// (`deploy`) then reads it.
#[test]
fn link_writes_dig_toml_pinning_store() {
    let d = tmp_dig();
    let store_id = "ab".repeat(32);

    let out = dig(&d)
        .args(["link", &store_id, "--output-dir", "dist", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["linked"].as_bool(), Some(true));
    assert_eq!(v["store_id"].as_str().unwrap(), store_id);
    assert_eq!(v["output_dir"].as_str().unwrap(), "dist");

    // The dig.toml is real and parses back with the pinned store id.
    let toml_path = d.path().join("dig.toml");
    assert!(toml_path.exists(), "link must write dig.toml");
    let text = fs::read_to_string(&toml_path).unwrap();
    assert!(text.contains(&format!("store-id = \"{store_id}\"")));
    assert!(text.contains("output-dir = \"dist\""));
    // Nothing on-chain/local was created.
    assert!(
        !d.path().join(".dig").exists(),
        "link must not create a workspace"
    );
}

/// `digstore link` accepts a full `urn:dig:…` URN (the hub share link) and
/// extracts the store id from it.
#[test]
fn link_accepts_urn() {
    let d = tmp_dig();
    let store_id = "cd".repeat(32);
    let urn = format!("urn:dig:chia:{store_id}");
    dig(&d).args(["link", &urn]).assert().success();
    let text = fs::read_to_string(d.path().join("dig.toml")).unwrap();
    assert!(text.contains(&format!("store-id = \"{store_id}\"")));
}

/// `link` refuses to clobber an existing dig.toml without `--force`.
#[test]
fn link_refuses_existing_dig_toml_without_force() {
    let d = tmp_dig();
    fs::write(d.path().join("dig.toml"), "output-dir = \"keep\"\n").unwrap();
    dig(&d)
        .args(["link", &"ab".repeat(32)])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    // The original file is untouched.
    let text = fs::read_to_string(d.path().join("dig.toml")).unwrap();
    assert!(text.contains("keep"));
    // With --force it overwrites.
    dig(&d)
        .args(["link", &"ab".repeat(32), "--force"])
        .assert()
        .success();
}

/// `link` rejects a garbage target with a clear message (not a panic).
#[test]
fn link_rejects_bad_target() {
    let d = tmp_dig();
    dig(&d)
        .args(["link", "not-a-store-id"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("store id"));
}

/// `digstore setup --no-login --json` runs the guided flow non-interactively
/// against the mock wallet: the seed is already present (the harness seeds it), so
/// it keeps the seed, checks funds, skips login, and reports a structured result.
#[test]
fn setup_runs_guided_flow_json() {
    let d = tmp_dig();
    let out = dig(&d)
        .args(["setup", "--no-login", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "setup failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // The harness pre-seeds an unlocked session, so setup keeps it.
    assert_eq!(v["seed"].as_str(), Some("kept"));
    assert_eq!(v["seed_unlocked"].as_bool(), Some(true));
    // login skipped via --no-login (no valid session in the JSON-mode path here).
    assert!(v.get("logged_in").is_some());
}

/// `auth` is a working alias for `setup`.
#[test]
fn auth_alias_runs_setup() {
    let d = tmp_dig();
    dig(&d)
        .args(["auth", "--no-login", "--json"])
        .assert()
        .success();
}

/// `digstore completion bash` prints a usable bash completion script naming the binary.
#[test]
fn completion_bash_prints_script() {
    let d = tmp_dig();
    dig(&d)
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("digstore").and(predicate::str::contains("complete")));
}

/// Every supported shell produces a non-empty script.
#[test]
fn completion_all_shells() {
    let d = tmp_dig();
    for shell in ["zsh", "fish", "powershell", "elvish"] {
        let out = dig(&d).args(["completion", shell]).output().unwrap();
        assert!(out.status.success(), "completion {shell} failed");
        assert!(!out.stdout.is_empty(), "completion {shell} was empty");
    }
}

/// `digstore --help-json` prints the machine-readable command schema (with no
/// subcommand) covering the headline commands + deploy's new flags.
#[test]
fn help_json_prints_command_schema() {
    let d = tmp_dig();
    let out = dig(&d).args(["--help-json"]).output().unwrap();
    assert!(
        out.status.success(),
        "--help-json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["name"].as_str(), Some("digstore"));
    let cmds = v["commands"].as_array().unwrap();
    let names: Vec<&str> = cmds.iter().filter_map(|c| c["name"].as_str()).collect();
    for expected in [
        "deploy",
        "new",
        "dev",
        "doctor",
        "setup",
        "link",
        "completion",
    ] {
        assert!(names.contains(&expected), "schema missing {expected}");
    }
    // deploy advertises --if-changed and --dry-run.
    let deploy = cmds.iter().find(|c| c["name"] == "deploy").unwrap();
    let longs: Vec<&str> = deploy["args"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["long"].as_str())
        .collect();
    assert!(longs.contains(&"if-changed"));
    assert!(longs.contains(&"dry-run"));
}
