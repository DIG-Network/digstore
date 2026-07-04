//! `digstore manifest` — the normalized public manifest: the store's complete
//! public file surface, latest version per path, with provenance + version depth.
//! Driven end-to-end through the INSTALLED binary against local (mock-anchored)
//! commits.

mod common;
use common::{dig, tmp_dig};

fn add_commit(dir: &tempfile::TempDir, name: &str, key: &str, content: &[u8]) {
    let f = dir.path().join(name);
    std::fs::write(&f, content).unwrap();
    dig(dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", key])
        .assert()
        .success();
    dig(dir).args(["commit"]).assert().success();
}

#[test]
fn manifest_json_normalizes_latest_per_path_across_capsules() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();

    // Capsule 0 (gen 0): index.html v1 + style.css.
    let f_index = dir.path().join("index.html");
    std::fs::write(&f_index, b"<h1>v1</h1>").unwrap();
    let f_css = dir.path().join("style.css");
    std::fs::write(&f_css, b"body{color:red}").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f_index)
        .args(["--key", "index.html"])
        .assert()
        .success();
    dig(&dir)
        .args(["add"])
        .arg(&f_css)
        .args(["--key", "style.css"])
        .assert()
        .success();
    dig(&dir).args(["commit"]).assert().success();

    // Capsule 1 (gen 1): index.html v2 (changed) only — style.css unchanged.
    add_commit(
        &dir,
        "index2.html",
        "index.html",
        b"<h1>v2 with more content</h1>",
    );

    let out = dig(&dir).args(["manifest", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "manifest --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema_version"].as_u64(), Some(1));
    let entries = v["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 2, "two distinct public paths");

    let find = |path: &str| {
        entries
            .iter()
            .find(|e| e["path"].as_str() == Some(path))
            .unwrap_or_else(|| panic!("missing entry for {path}"))
    };

    // index.html: latest version is in the SECOND capsule (gen 1), 2 versions.
    let index = find("index.html");
    assert_eq!(index["generation_index"].as_u64(), Some(1));
    assert_eq!(index["version_count"].as_u64(), Some(2));
    assert_eq!(index["latest_root"].as_str().unwrap().len(), 64);
    assert_eq!(index["sha256_latest"].as_str().unwrap().len(), 64);

    // style.css: latest version still lives in the FIRST capsule (gen 0), 1 version.
    let style = find("style.css");
    assert_eq!(style["generation_index"].as_u64(), Some(0));
    assert_eq!(style["version_count"].as_u64(), Some(1));

    // The two files' latest roots differ (index.html advanced, style.css did not).
    assert_ne!(
        index["latest_root"].as_str(),
        style["latest_root"].as_str(),
        "index.html's latest capsule differs from style.css's"
    );
}

#[test]
fn manifest_human_lists_every_public_file() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    add_commit(&dir, "index.html", "index.html", b"<h1>hi</h1>");

    let out = dig(&dir).args(["manifest"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("index.html"), "human output lists the path");
    assert!(
        text.contains("SHA-256"),
        "human output has the content-hash column"
    );
}

#[test]
fn manifest_empty_before_any_commit() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let out = dig(&dir).args(["manifest", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["entries"].as_array().unwrap().is_empty());
}
