mod common;
use common::{dig, tmp_dig};
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn commit_creates_module_and_log_lists_it() {
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
    dig(&dir)
        .args(["commit", "-m", "first"])
        .assert()
        .success()
        .stdout(predicate::str::contains("committed root"));
    let modules: Vec<_> = std::fs::read_dir(common::store_dir(&dir).join("modules"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "dig").unwrap_or(false))
        .collect();
    assert_eq!(modules.len(), 1);
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation 0"));
}

#[test]
fn commit_with_nothing_staged_fails_exit_2() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    dig(&dir).args(["commit"]).assert().failure().code(2);
}

/// Re-committing UNCHANGED content must be refused (exit 2), NOT anchored again.
/// Committing clears staging, so re-`add`ing an identical file re-stages it and the
/// recomputed root equals the current head root — without a guard, `commit` would
/// anchor a duplicate root on-chain (spending real XCH) and append a no-op
/// generation. Regression: caught by permutation testing.
#[test]
fn commit_unchanged_content_is_rejected_not_reanchored() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"identical bytes").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();
    dig(&dir).args(["commit", "-m", "g1"]).assert().success();

    // Re-stage the SAME content and try to commit again → refused, no new generation.
    dig(&dir).args(["add", "a.txt"]).assert().success();
    dig(&dir).args(["commit", "-m", "g2"]).assert().failure().code(2);

    let out = dig(&dir).args(["log", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v.as_array().unwrap().len(),
        1,
        "an unchanged commit must not add a generation"
    );
}

/// Committing on a store whose INITIAL mint never confirmed (pending init) must be
/// refused with a clear pointer to `digstore anchor`, not a confusing chain error —
/// and must not finalize any generation. (On a real chain the update would fail at
/// lineage sync; the guard makes the precondition explicit.)
#[test]
fn commit_on_pending_init_is_refused_clearly() {
    let dir = tmp_dig();
    // init times out → store kept, mint anchor stays Pending with no root anchored.
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .arg("init")
        .assert()
        .failure()
        .code(14);
    std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    dig(&dir)
        .args(["commit", "-m", "g1"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("digstore anchor"));

    // No generation finalized.
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation").not());
}

/// The anchor.toml status + last_root for the default store.
fn anchor_status_and_root(dir: &TempDir) -> (String, String) {
    let text = std::fs::read_to_string(common::store_dir(dir).join("anchor.toml")).unwrap();
    let field = |name: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(name))
            .and_then(|l| l.split('"').nth(1))
            .unwrap_or("")
            .to_string()
    };
    (field("status"), field("last_root"))
}

/// Happy path: init (mock, confirmed) → add → commit anchors + finalizes; the
/// generation lands, anchor.toml is confirmed with last_root == committed root,
/// and the JSON commit output carries anchor_status/coin_id/mocked.
#[test]
fn commit_anchors_and_finalizes_on_confirm() {
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

    let out = dig(&dir)
        .args(["commit", "-m", "first", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "commit should succeed");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let committed_root = v["root"].as_str().unwrap().to_string();
    assert_eq!(v["anchor_status"].as_str().unwrap(), "confirmed");
    assert!(v["mocked"].as_bool().unwrap(), "mock anchor reports mocked");
    assert_eq!(
        v["coin_id"].as_str().unwrap().len(),
        64,
        "coin_id is 32-byte hex"
    );

    // The generation is finalized: log shows it.
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation 0"));

    // anchor.toml reflects the confirmed update at the committed root.
    let (status, last_root) = anchor_status_and_root(&dir);
    assert_eq!(status, "confirmed");
    assert_eq!(last_root, committed_root);
}

/// The compiled module carries the on-chain `ChainState` pointer embedded at
/// commit finalize: after a confirmed (mock) anchor, the `.dig` module's data
/// section decodes to a `ChainState` with the store's launcher id and network.
#[test]
fn commit_embeds_chain_state_in_module() {
    let dir = common::tmp_dig();
    common::dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    common::dig(&dir).args(["add", "a.txt"]).assert().success();
    let out = common::dig(&dir)
        .args(["--json", "commit", "-m", "x"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let modules = common::store_dir(&dir).join("modules");
    let module = std::fs::read_dir(&modules)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "dig").unwrap_or(false))
        .expect("a .dig module");
    let bytes = std::fs::read(&module).unwrap();
    let cs = digstore_cli::ops::store_ops::read_module_chain_state(&bytes)
        .expect("read")
        .expect("module carries ChainState");
    assert_eq!(cs.network, "mainnet");
    assert_ne!(cs.launcher_id, digstore_core::Bytes32([0u8; 32]));
}

/// Commit blocks and times out: with DIGSTORE_ANCHOR_MOCK_TIMEOUT=1 the confirm
/// stays Pending → exit 14, NO new generation (roots.log not advanced), staging
/// intact, anchor.toml Pending with last_root = the would-be root.
#[test]
fn commit_blocks_until_confirmed_and_does_not_finalize_on_timeout() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("b.txt");
    std::fs::write(&f, b"pending content").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "b"])
        .assert()
        .success();

    // Timeout env set on THIS command only.
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["commit", "-m", "pending"])
        .assert()
        .failure()
        .code(14);

    // roots.log NOT advanced: log shows no generation.
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation").not());

    // Staging intact: the file is still staged.
    dig(&dir)
        .args(["staged"])
        .assert()
        .success()
        .stdout(predicate::str::contains("b"));

    // anchor.toml left Pending, pointing at the would-be root.
    let (status, last_root) = anchor_status_and_root(&dir);
    assert_eq!(status, "pending");
    assert_eq!(last_root.len(), 64, "last_root recorded the in-flight root");
}

/// Idempotent resume: after a timeout left a Pending update, re-running commit
/// WITHOUT the timeout env reuses the in-flight update (no error), confirms, and
/// finalizes — the generation lands and anchor.toml flips to confirmed.
#[test]
fn commit_resumes_pending_update_idempotently() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("c.txt");
    std::fs::write(&f, b"resume content").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "c"])
        .assert()
        .success();

    // First attempt times out (Pending).
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["commit", "-m", "first try"])
        .assert()
        .failure()
        .code(14);
    let (status, pending_root) = anchor_status_and_root(&dir);
    assert_eq!(status, "pending");

    // Re-run without the timeout env: confirms + finalizes.
    let out = dig(&dir)
        .args(["commit", "-m", "retry", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "resume should succeed");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["anchor_status"].as_str().unwrap(), "confirmed");
    assert_eq!(
        v["root"].as_str().unwrap(),
        pending_root,
        "same root as the pending update"
    );

    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("generation 0"));
    let (status, last_root) = anchor_status_and_root(&dir);
    assert_eq!(status, "confirmed");
    assert_eq!(last_root, pending_root);
}
