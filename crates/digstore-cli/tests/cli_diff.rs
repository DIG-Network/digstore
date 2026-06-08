mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;

fn roots(dir: &tempfile::TempDir) -> Vec<String> {
    let out = dig(dir).args(["log", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["root"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn diff_two_generations_lists_changes() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"one").unwrap();
    dig(&dir).args(["add"]).arg(&f).args(["--key", "a"]).assert().success();
    dig(&dir).args(["commit"]).assert().success();

    let g = dir.path().join("b.txt");
    std::fs::write(&g, b"two new").unwrap();
    dig(&dir).args(["add"]).arg(&g).args(["--key", "b"]).assert().success();
    dig(&dir).args(["commit"]).assert().success();

    let r = roots(&dir); // newest first: r[0]=to, r[1]=from
    dig(&dir)
        .args(["diff", &r[1], &r[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains("+ b"));
}
