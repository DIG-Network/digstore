mod common;
use common::{dig, tmp_dig};

fn root_hex(dir: &tempfile::TempDir) -> String {
    let out = dig(dir).args(["log", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v[0]["root"].as_str().unwrap().to_string()
}

#[test]
fn checkout_materializes_generation() {
    let dir = tmp_dig();
    let content = b"materialize me please";
    let f = dir.path().join("file.txt");
    std::fs::write(&f, content).unwrap();
    dig(&dir).arg("init").assert().success();
    dig(&dir).args(["add"]).arg(&f).args(["--key", "file.txt"]).assert().success();
    dig(&dir).args(["commit"]).assert().success();

    let root = root_hex(&dir);
    let out_dir = dir.path().join("out");
    dig(&dir)
        .args(["checkout", &root, "--out"])
        .arg(&out_dir)
        .assert()
        .success();
    assert_eq!(std::fs::read(out_dir.join("file.txt")).unwrap(), content);
}
