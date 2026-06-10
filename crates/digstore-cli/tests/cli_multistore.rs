//! Multi-store workspace integration coverage (assert_cmd): store listing &
//! switching, `--store` isolation, the per-store stage cap, `unstage`/`staged`,
//! the per-store content root + rootless URN stability, and legacy migration.
//!
//! These drive the real `digstore` binary against a temp project whose workspace
//! lives at `<project>/.dig` (so the on-disk layout — `.dig/stores/<name>/...`,
//! `.dig/workspace.toml` — matches the multi-store spec exactly). Commands run
//! with `current_dir(project)` so workspace discovery and content-root resolution
//! behave as a real user's would.

mod common;

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

/// A `digstore` invocation whose CWD is the temp project root. Workspace
/// discovery then anchors at `<project>/.dig`, exactly like a real checkout.
fn dig_at(project: &Path) -> Command {
    let mut c = Command::cargo_bin("digstore").unwrap();
    c.current_dir(project);
    c
}

/// Path to a store's per-store config (`<project>/.dig/stores/<name>/config.toml`).
fn store_config_path(project: &Path, store: &str) -> std::path::PathBuf {
    project
        .join(".dig")
        .join("stores")
        .join(store)
        .join("config.toml")
}

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "non-JSON stdout: {e}\nstdout={:?}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

#[test]
fn two_stores_list_with_active_on_first() {
    let tmp = TempDir::new().unwrap();
    dig_at(tmp.path()).args(["init", "a"]).assert().success();
    dig_at(tmp.path()).args(["init", "b"]).assert().success();

    let out = dig_at(tmp.path())
        .args(["stores", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stores --json failed: {out:?}");
    let v = json_stdout(&out);
    let rows = v.as_array().unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(
        names.contains(&"a") && names.contains(&"b"),
        "names = {names:?}"
    );

    // `a` was created first, so it is the active store.
    let active = rows
        .iter()
        .find(|r| r["active"] == true)
        .expect("one active store");
    assert_eq!(active["name"], "a", "first-created store should be active");
}

#[test]
fn use_switches_the_active_store() {
    let tmp = TempDir::new().unwrap();
    dig_at(tmp.path()).args(["init", "a"]).assert().success();
    dig_at(tmp.path()).args(["init", "b"]).assert().success();

    dig_at(tmp.path()).args(["use", "b"]).assert().success();

    let out = dig_at(tmp.path())
        .args(["stores", "--json"])
        .output()
        .unwrap();
    let v = json_stdout(&out);
    let active = v
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["active"] == true)
        .expect("one active store");
    assert_eq!(active["name"], "b");
}

#[test]
fn store_flag_targets_a_specific_store_with_isolation() {
    let tmp = TempDir::new().unwrap();
    dig_at(tmp.path()).args(["init", "a"]).assert().success();
    dig_at(tmp.path()).args(["init", "b"]).assert().success();
    std::fs::write(tmp.path().join("file.txt"), b"content").unwrap();

    // Stage into `a` explicitly (even though `a` is active, prove --store wins).
    dig_at(tmp.path())
        .args(["--store", "a", "add", "file.txt"])
        .assert()
        .success();

    // `a` shows the staged file...
    let out_a = dig_at(tmp.path())
        .args(["staged", "--store", "a", "--json"])
        .output()
        .unwrap();
    let va = json_stdout(&out_a);
    let keys_a: Vec<&str> = va["staged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["key"].as_str().unwrap())
        .collect();
    assert!(keys_a.contains(&"file.txt"), "a.staged = {keys_a:?}");

    // ...and `b` is empty (per-store isolation).
    let out_b = dig_at(tmp.path())
        .args(["staged", "--store", "b", "--json"])
        .output()
        .unwrap();
    let vb = json_stdout(&out_b);
    assert!(
        vb["staged"].as_array().unwrap().is_empty(),
        "b.staged should be empty, got {:?}",
        vb["staged"]
    );
}

#[test]
fn add_over_the_per_store_cap_is_rejected_and_stages_nothing() {
    let tmp = TempDir::new().unwrap();
    dig_at(tmp.path()).args(["init"]).assert().success();

    // Shrink the persisted cap to 10 bytes directly in the store's config.toml
    // (read-modify-write the TOML) so we exercise the cap arithmetic without a
    // 128 MB fixture.
    let cfg_path = store_config_path(tmp.path(), "default");
    let text = std::fs::read_to_string(&cfg_path).unwrap();
    let new_text: String = text
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("max_size") {
                "max_size = 10".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        new_text.contains("max_size = 10"),
        "config had no max_size line: {text}"
    );
    std::fs::write(&cfg_path, new_text).unwrap();

    // A >10-byte file must be rejected.
    std::fs::write(tmp.path().join("big.txt"), b"this is more than ten bytes").unwrap();
    let add = dig_at(tmp.path())
        .args(["add", "big.txt"])
        .output()
        .unwrap();
    assert!(
        !add.status.success(),
        "add over cap should fail, got success: {add:?}"
    );
    let msg = String::from_utf8_lossy(&add.stderr);
    assert!(
        msg.contains("over the") && msg.contains("limit"),
        "expected an 'over the … limit' message, got stderr: {msg}"
    );

    // Nothing was staged: the total stays 0.
    let staged = dig_at(tmp.path())
        .args(["staged", "--json"])
        .output()
        .unwrap();
    let v = json_stdout(&staged);
    assert_eq!(
        v["total_bytes"].as_u64().unwrap(),
        0,
        "staged total must stay 0"
    );
    assert!(v["staged"].as_array().unwrap().is_empty());
}

#[test]
fn unstage_empties_the_staging_area() {
    let tmp = TempDir::new().unwrap();
    dig_at(tmp.path()).args(["init"]).assert().success();
    std::fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
    dig_at(tmp.path()).args(["add", "a.txt"]).assert().success();

    // Sanity: staged before unstage.
    let before = dig_at(tmp.path())
        .args(["staged", "--json"])
        .output()
        .unwrap();
    assert!(
        !json_stdout(&before)["staged"]
            .as_array()
            .unwrap()
            .is_empty(),
        "should have something staged before unstage"
    );

    dig_at(tmp.path()).args(["unstage"]).assert().success();

    let after = dig_at(tmp.path())
        .args(["staged", "--json"])
        .output()
        .unwrap();
    let v = json_stdout(&after);
    assert!(
        v["staged"].as_array().unwrap().is_empty(),
        "staging should be empty after unstage, got {:?}",
        v["staged"]
    );
    assert_eq!(v["total_bytes"].as_u64().unwrap(), 0);
}

#[test]
fn staged_json_reports_total_and_limit() {
    let tmp = TempDir::new().unwrap();
    dig_at(tmp.path()).args(["init"]).assert().success();
    std::fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
    dig_at(tmp.path()).args(["add", "a.txt"]).assert().success();

    let out = dig_at(tmp.path())
        .args(["staged", "--json"])
        .output()
        .unwrap();
    let v = json_stdout(&out);
    assert!(
        v.get("total_bytes").is_some(),
        "staged --json must include total_bytes"
    );
    assert!(
        v.get("limit_bytes").is_some(),
        "staged --json must include limit_bytes"
    );
    assert_eq!(v["total_bytes"].as_u64().unwrap(), 5, "5-byte file staged");
    // The default per-store cap is 128 MB (decimal) = MAX_STORE_BYTES.
    assert_eq!(v["limit_bytes"].as_u64().unwrap(), 128_000_000);
}

#[test]
fn content_root_urn_is_stable_regardless_of_operating_dir() {
    let tmp = TempDir::new().unwrap();
    // A store whose content root is `dist`.
    dig_at(tmp.path())
        .args(["init", "site", "--dir", "dist"])
        .assert()
        .success();

    let css_dir = tmp.path().join("dist").join("css");
    std::fs::create_dir_all(&css_dir).unwrap();
    std::fs::write(css_dir.join("app.css"), b"body{color:red}").unwrap();

    // `--store site add -A` from the project root: the content root makes the
    // key content-root-relative, i.e. `css/app.css` (NOT `dist/css/app.css`).
    dig_at(tmp.path())
        .args(["--store", "site", "add", "-A"])
        .assert()
        .success();

    // URN preview from the project root (op_dir resolves via content_root=dist).
    let out_root = dig_at(tmp.path())
        .args(["--store", "site", "urn", "css/app.css", "--json"])
        .output()
        .unwrap();
    assert!(
        out_root.status.success(),
        "urn from root failed: {out_root:?}"
    );
    let vr = json_stdout(&out_root);
    let entry_root = &vr.as_array().unwrap()[0];
    assert_eq!(entry_root["key"], "css/app.css");
    let urn_root = entry_root["urn"].as_str().unwrap().to_string();
    let rkey_root = entry_root["retrieval_key"].as_str().unwrap().to_string();

    // The same URN computed with an explicit `-C dist` (op_dir override) must be
    // byte-identical: the key/URN/retrieval_key depend only on the content-root-
    // relative key, never on where the command runs from.
    let out_c = dig_at(tmp.path())
        .args([
            "--store",
            "site",
            "-C",
            "dist",
            "urn",
            "css/app.css",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out_c.status.success(), "urn -C dist failed: {out_c:?}");
    let vc = json_stdout(&out_c);
    let entry_c = &vc.as_array().unwrap()[0];
    assert_eq!(entry_c["key"], "css/app.css");
    assert_eq!(
        entry_c["urn"].as_str().unwrap(),
        urn_root,
        "URN must be identical"
    );
    assert_eq!(
        entry_c["retrieval_key"].as_str().unwrap(),
        rkey_root,
        "retrieval_key must be identical"
    );
}

#[test]
fn legacy_flat_layout_is_migrated_into_default_store() {
    let tmp = TempDir::new().unwrap();
    // Create a normal store, then rewrite the on-disk layout to the pre-multistore
    // FLAT form: store files directly under `.dig/`, no `stores/`, no workspace.toml.
    dig_at(tmp.path()).args(["init"]).assert().success();

    let dig = tmp.path().join(".dig");
    let default_store = dig.join("stores").join("default");
    // Move every entry from .dig/stores/default/ up to .dig/.
    for entry in std::fs::read_dir(&default_store).unwrap() {
        let entry = entry.unwrap();
        let to = dig.join(entry.file_name());
        std::fs::rename(entry.path(), to).unwrap();
    }
    std::fs::remove_dir_all(dig.join("stores")).unwrap();
    let ws_toml = dig.join("workspace.toml");
    if ws_toml.exists() {
        std::fs::remove_file(&ws_toml).unwrap();
    }
    // Sanity: we now have a legacy flat layout.
    assert!(
        dig.join("config.toml").exists(),
        "legacy config.toml at .dig/ root"
    );
    assert!(!dig.join("stores").exists());
    assert!(!ws_toml.exists());

    // Any command triggers load_or_migrate.
    let out = dig_at(tmp.path())
        .args(["stores", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stores after legacy layout failed: {out:?}"
    );
    let v = json_stdout(&out);
    let names: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["default"],
        "migration should yield a single 'default' store"
    );

    // Migration moved the files back into stores/default/ and wrote workspace.toml.
    assert!(
        store_config_path(tmp.path(), "default").exists(),
        "migration must create .dig/stores/default/config.toml"
    );
    assert!(
        ws_toml.exists(),
        "migration must create .dig/workspace.toml"
    );
}
