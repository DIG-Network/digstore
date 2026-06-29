//! Canonical terminology (SYSTEM.md "Canonical terminology & branding").
//!
//! The ecosystem-wide object vocabulary is `store` (the on-chain singleton identity)
//! and `capsule` (one generation = `storeId:rootHash`). "project" is NOT a
//! user-facing synonym — `--project`/`projects` survive ONLY as hidden back-compat
//! aliases. These tests pin the digstore CLI's USER-FACING wording to store/capsule,
//! while guarding that the machine contracts that must NOT change — `--json` field
//! names and on-disk file names — are left exactly as they were.

mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

/// Init → add → commit a single file, returning the temp store dir.
fn store_with_one_capsule() -> tempfile::TempDir {
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
    dig(&dir).args(["commit", "-m", "first"]).assert().success();
    dir
}

// --- Canonical wording in human output ---

#[test]
fn init_reports_initialized_store() {
    let dir = tmp_dig();
    // The canonical vocabulary is `store`; "project" must NOT appear in init output.
    dig(&dir).arg("init").assert().success().stdout(
        predicate::str::contains("Initialized store")
            .and(predicate::str::contains("project").not()),
    );
}

#[test]
fn log_does_not_say_project_or_generation() {
    let dir = store_with_one_capsule();
    // `log` lists published capsules; it must not leak the protocol word "generation"
    // nor the deprecated user-facing "project".
    dig(&dir).args(["log"]).assert().success().stdout(
        predicate::str::contains("generation")
            .not()
            .and(predicate::str::contains("project").not()),
    );
}

#[test]
fn status_does_not_say_project() {
    let dir = store_with_one_capsule();
    dig(&dir)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("project").not());
}

// --- Canonical branding (SYSTEM.md "Canonical terminology & branding") ---

/// Help text never renders the mis-cased hub wordmark `DIGHub`. The canonical
/// form is `DIGHUb` (capital U). Checked across the whole `--help` tree via the
/// commands most likely to name the hub.
#[test]
fn help_uses_canonical_dighub_casing() {
    for args in [
        vec!["--help"],
        vec!["init", "--help"],
        vec!["commit", "--help"],
        vec!["deploy", "--help"],
        vec!["deploy-key", "--help"],
        vec!["setup", "--help"],
        vec!["login", "--help"],
    ] {
        let out = dig(&tmp_dig()).args(&args).output().unwrap();
        let help = String::from_utf8_lossy(&out.stdout);
        // The off-canon `DIGHub` (capital H, lowercase u) must never appear; the
        // canonical `DIGHUb` may. (The lowercase code id `dighub` is allowed.)
        assert!(
            !help.contains("DIGHub"),
            "`{args:?}` help must use DIGHUb, not DIGHub:\n{help}"
        );
    }
}

/// `deploy --preview` content-open address uses the canonical `chia://` scheme
/// (what the DIG Browser/extension register), NOT `dig://`. The §21 remote and
/// `urn:dig:` namespace are unaffected.
#[test]
fn deploy_preview_content_address_is_chia_scheme() {
    let ci = tmp_dig();
    let dist = ci.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), b"<h1>hi</h1>").unwrap();
    std::fs::write(ci.path().join("dig.toml"), "output-dir = \"dist\"\n").unwrap();

    let out = dig(&ci)
        .args(["deploy", "--preview", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let addr = v["content_address"].as_str().expect("content_address");
    assert!(
        addr.starts_with("chia://"),
        "preview content address must be chia://, got {addr}"
    );
    assert!(
        !addr.starts_with("dig://"),
        "preview content address must not be dig://, got {addr}"
    );
}

/// Static help/after_help prose never pins a flat "100 DIG" price — pricing copy
/// is NEUTRAL (the live computed amount is shown only at spend time, not in prose).
/// `init`/`commit`/`deploy` help are the surfaces that historically said it.
#[test]
fn help_pricing_prose_is_neutral_not_flat_100() {
    for args in [
        vec!["init", "--help"],
        vec!["commit", "--help"],
        vec!["deploy", "--help"],
    ] {
        let out = dig(&tmp_dig()).args(&args).output().unwrap();
        let help = String::from_utf8_lossy(&out.stdout);
        assert!(
            !help.contains("100 DIG"),
            "`{args:?}` help must not pin a flat `100 DIG`:\n{help}"
        );
        assert!(
            !help.contains("Costs 100"),
            "`{args:?}` help must not say `Costs 100`:\n{help}"
        );
    }
}

/// The `$DIG` sigil appears on the token's first reference in user-facing help.
#[test]
fn help_uses_dig_sigil() {
    let out = dig(&tmp_dig()).args(["init", "--help"]).output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        help.contains("$DIG"),
        "init --help should reference the $DIG token with its sigil:\n{help}"
    );
}

/// When the CLI dead-ends a user on insufficient DIG, it names where to get $DIG
/// (the three canonical venues), so the user is never stuck without a path. Minting
/// (`init`) is free of $DIG (#111), so the dead-end is at `commit` (a capsule pays).
#[test]
fn insufficient_dig_hint_names_get_dig_venues() {
    let dir = tmp_dig();
    // Mint is free of $DIG, so init succeeds even with zero DIG.
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .arg("init")
        .assert()
        .success();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"alpha beta gamma").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "a"])
        .assert()
        .success();
    // Committing a capsule with zero DIG dead-ends with the get-$DIG hint.
    let out = dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_DIG", "0")
        .args(["commit", "-m", "first"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(12));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("TibetSwap")
            && stderr.contains("dexie.space")
            && stderr.contains("xch.9mm.pro"),
        "insufficient-DIG hint must name the 3 Get-$DIG venues:\n{stderr}"
    );
}

// --- Hidden back-compat aliases (must keep working, but are not advertised) ---

#[test]
fn projects_alias_lists_like_stores() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    // The hidden `projects` alias runs the same command as `stores`.
    dig(&dir).args(["projects"]).assert().success();
    // The canonical `stores` command works.
    dig(&dir).args(["stores"]).assert().success();
}

#[test]
fn project_flag_alias_selects_active_store() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    // `--project` is a hidden alias of `--store`; both select a store for the command.
    dig(&dir)
        .args(["--project", "default", "status"])
        .assert()
        .success();
    dig(&dir)
        .args(["--store", "default", "status"])
        .assert()
        .success();
}

/// The deprecated `project` vocabulary is HIDDEN from `--help`: top-level help must
/// not advertise a `projects` alias, and the global flag help must not say "project".
#[test]
fn help_does_not_advertise_project() {
    let out = dig(&tmp_dig()).args(["--help"]).output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        !help.contains("project"),
        "top-level --help must not mention `project`:\n{help}"
    );
}

// --- Regression guards: machine contracts must NOT change ---

#[test]
fn json_log_keys_are_unchanged() {
    let dir = store_with_one_capsule();
    let out = dig(&dir).args(["log", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // The wire contract keeps `id` and `root` — the friendly wording is human-only.
    assert!(v[0]["id"].is_u64(), "log --json must keep the `id` key");
    assert!(
        v[0]["root"].is_string(),
        "log --json must keep the `root` key"
    );
    // It must NOT have sprouted a renamed `deployment` key.
    assert!(v[0].get("deployment").is_none());
}

#[test]
fn on_disk_files_keep_their_names() {
    let dir = store_with_one_capsule();
    let store = common::store_dir(&dir);
    // On-disk layout is a contract for existing stores: do not rename these.
    assert!(
        store.join("roots.log").exists(),
        "roots.log must keep its name"
    );
    assert!(
        store.join("config.toml").exists(),
        "config.toml must keep its name"
    );
    assert!(
        dir.path().join(".dig").join("workspace.toml").exists(),
        "workspace.toml must keep its name"
    );
}
