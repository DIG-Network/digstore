mod common;
use assert_cmd::Command;
use common::tmp_dig;

fn dig_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("digstore").unwrap();
    c.current_dir(dir);
    c
}

fn init(dir: &std::path::Path) {
    dig_in(dir).arg("init").assert().success();
}

#[test]
fn add_all_stages_every_file() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"a").unwrap();
    std::fs::create_dir_all(d.path().join("sub")).unwrap();
    std::fs::write(d.path().join("sub/b.md"), b"b").unwrap();
    init(d.path());
    let out = dig_in(d.path())
        .args(["--json", "add", "-A"])
        .output()
        .unwrap();
    assert!(out.status.success(), "add -A failed: {:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let staged: Vec<String> = v["staged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        staged.contains(&"a.txt".to_string()),
        "staged = {:?}",
        staged
    );
    assert!(
        staged.contains(&"sub/b.md".to_string()),
        "staged = {:?}",
        staged
    );
}

#[test]
fn add_dot_and_glob_and_multiple() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.rs"), b"x").unwrap();
    std::fs::write(d.path().join("b.rs"), b"y").unwrap();
    std::fs::write(d.path().join("c.txt"), b"z").unwrap();
    init(d.path());
    let out = dig_in(d.path())
        .args(["--json", "add", "*.rs"])
        .output()
        .unwrap();
    assert!(out.status.success(), "add *.rs failed: {:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let staged: Vec<String> = v["staged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        staged.contains(&"a.rs".to_string()) && staged.contains(&"b.rs".to_string()),
        "staged = {:?}",
        staged
    );
    assert!(
        !staged.contains(&"c.txt".to_string()),
        "c.txt should not be staged, staged = {:?}",
        staged
    );
}

#[test]
fn add_dry_run_reports_would_stage_but_stages_nothing() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"a").unwrap();
    init(d.path());
    // --dry-run previews what WOULD be staged (C2/C4): the `staged` array lists the
    // would-be entries and `dry_run` is true, but nothing is actually committed to
    // the staging area.
    let out = dig_in(d.path())
        .args(["--json", "add", "-A", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success(), "add --dry-run failed: {:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let staged: Vec<String> = v["staged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        staged.contains(&"a.txt".to_string()),
        "dry-run should preview a.txt; got {:?}",
        v
    );
    assert_eq!(v["dry_run"], true, "dry_run field should be true");

    // The dry run must not have actually staged anything: a real status shows
    // a.txt as untracked, not staged.
    let st = dig_in(d.path())
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert!(st.status.success(), "status failed: {:?}", st);
    let sv: serde_json::Value = serde_json::from_slice(&st.stdout).unwrap();
    assert!(
        sv["staged"].as_array().unwrap().is_empty(),
        "dry-run must not stage; status.staged = {:?}",
        sv["staged"]
    );
}

#[test]
fn add_key_with_multiple_paths_errors() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"a").unwrap();
    std::fs::write(d.path().join("b.txt"), b"b").unwrap();
    init(d.path());
    dig_in(d.path())
        .args(["add", "a.txt", "b.txt", "--key", "x"])
        .assert()
        .failure();
}
