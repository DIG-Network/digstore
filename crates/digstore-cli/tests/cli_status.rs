mod common;
use assert_cmd::Command;
use common::tmp_dig;

fn dig_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("digstore").unwrap();
    c.current_dir(dir);
    common::seed_mock_env(&mut c, dir);
    c
}

#[test]
fn status_reports_untracked_then_staged_then_modified() {
    let d = tmp_dig();
    std::fs::write(d.path().join("a.txt"), b"one").unwrap();
    dig_in(d.path()).arg("init").assert().success();

    // a.txt is untracked.
    let v: serde_json::Value = serde_json::from_slice(
        &dig_in(d.path())
            .args(["--json", "status"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let untracked: Vec<&str> = v["untracked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(
        untracked.contains(&"a.txt"),
        "a.txt should be untracked; got {:?}",
        untracked
    );
    assert!(v["staged"].as_array().unwrap().is_empty());

    // stage it -> staged.
    dig_in(d.path()).args(["add", "a.txt"]).assert().success();
    let v: serde_json::Value = serde_json::from_slice(
        &dig_in(d.path())
            .args(["--json", "status"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(v["staged"].as_array().unwrap().iter().any(|x| x == "a.txt"));

    // commit, then edit the source -> modified.
    dig_in(d.path())
        .args(["commit", "-m", "one"])
        .assert()
        .success();
    std::fs::write(d.path().join("a.txt"), b"two-different").unwrap();
    let v: serde_json::Value = serde_json::from_slice(
        &dig_in(d.path())
            .args(["--json", "status"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(
        v["modified"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "a.txt"),
        "edited committed file shows modified; got {v}"
    );
}
