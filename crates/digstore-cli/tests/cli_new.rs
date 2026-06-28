//! `digstore new <template>` — the FREE, NO-MINT scaffolder (roadmap #5).
//!
//! The whole point is "see it work before you pay": `new` must produce a runnable
//! local project with NO wallet, NO chain, and NO spend. These tests drive the
//! INSTALLED binary end-to-end and assert exactly that.

mod common;
use assert_cmd::Command;
use common::tmp_dig;
use predicates::prelude::*;

/// `digstore new static-site <dir>` writes a runnable project (dig.toml + index)
/// and creates NOTHING on-chain — no `.dig` workspace, no mint.
#[test]
fn new_static_site_scaffolds_without_minting() {
    let td = tmp_dig();
    let target = td.path().join("site");
    Command::cargo_bin("digstore")
        .unwrap()
        .args(["new", "static-site"])
        .arg(&target)
        .assert()
        .success()
        .stdout(predicate::str::contains("no wallet, no chain, no spend"));

    assert!(target.join("dig.toml").exists(), "dig.toml scaffolded");
    assert!(target.join("index.html").exists(), "index.html scaffolded");
    // No mint / no workspace was created.
    assert!(
        !target.join(".dig").exists(),
        "new must not create a workspace"
    );
}

/// The dapp template ships a working `window.chia` usage example.
#[test]
fn new_dapp_includes_window_chia_example() {
    let td = tmp_dig();
    let target = td.path().join("dapp");
    Command::cargo_bin("digstore")
        .unwrap()
        .args(["new", "dapp-window-chia"])
        .arg(&target)
        .assert()
        .success();
    let app = std::fs::read_to_string(target.join("app.js")).unwrap();
    assert!(app.contains("window.chia"), "dapp demonstrates window.chia");
}

/// `--list` prints the catalog and exits 0 (works as JSON too).
#[test]
fn new_list_shows_all_templates() {
    Command::cargo_bin("digstore")
        .unwrap()
        .args(["new", "x", "--list"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("static-site")
                .and(predicate::str::contains("vite-react"))
                .and(predicate::str::contains("next-static"))
                .and(predicate::str::contains("nft-drop"))
                .and(predicate::str::contains("dapp-window-chia")),
        );
}

/// An unknown template fails with a helpful message listing the real ones.
#[test]
fn new_unknown_template_errors() {
    let td = tmp_dig();
    Command::cargo_bin("digstore")
        .unwrap()
        .args(["new", "not-a-template"])
        .arg(td.path().join("out"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown template"));
}

/// Refuses to scaffold into a non-empty directory without `--force`.
#[test]
fn new_refuses_nonempty_dir_without_force() {
    let td = tmp_dig();
    std::fs::write(td.path().join("keep.txt"), b"hi").unwrap();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(td.path())
        .args(["new", "static-site"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
}

/// The vite-react template scaffolds nested files (src/App.jsx) and a build-ready
/// dig.toml (output-dir = dist, a build-command).
#[test]
fn new_vite_react_is_build_ready() {
    let td = tmp_dig();
    let target = td.path().join("app");
    Command::cargo_bin("digstore")
        .unwrap()
        .args(["new", "vite-react"])
        .arg(&target)
        .assert()
        .success();
    assert!(target.join("src").join("App.jsx").exists());
    let toml = std::fs::read_to_string(target.join("dig.toml")).unwrap();
    assert!(toml.contains("output-dir"));
    assert!(toml.contains("build-command"));
}
