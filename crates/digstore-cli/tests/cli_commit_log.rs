mod common;
use common::{dig, store_id_and_root, tmp_dig, TestServer};
use predicates::prelude::*;
use tempfile::TempDir;

/// Read the store's host public key (48 bytes) from trusted_keys.json — needed to
/// stand up a `TestServer` that hosts the store so `commit --push` can fast-forward it.
fn host_pubkey(dir: &TempDir) -> [u8; 48] {
    let text = std::fs::read_to_string(common::store_dir(dir).join("trusted_keys.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let hex = v[0]["public_key"].as_str().unwrap();
    let bytes = hex::decode(hex).unwrap();
    bytes.try_into().unwrap()
}

/// The root the in-process server is currently serving for `store_id`, or `None`
/// if the server has never received content (still at genesis / all-zero root).
fn server_served_root(server: &TestServer, store_id_hex: &str) -> Option<String> {
    use digstore_remote::RemoteBackend;
    let store_id = digstore_core::Bytes32::from_hex(store_id_hex).unwrap();
    let head = server.backend().head_state(&store_id).ok()?;
    if head.served_root == digstore_core::Bytes32([0u8; 32]) {
        None
    } else {
        Some(head.served_root.to_hex())
    }
}

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
        .stdout(predicate::str::contains("deployment 0"));
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
    dig(&dir)
        .args(["commit", "-m", "g2"])
        .assert()
        .failure()
        .code(2);

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
        .stdout(predicate::str::contains("deployment").not());
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
        .stdout(predicate::str::contains("deployment 0"));

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
        .stdout(predicate::str::contains("deployment").not());

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
        .stdout(predicate::str::contains("deployment 0"));
    let (status, last_root) = anchor_status_and_root(&dir);
    assert_eq!(status, "confirmed");
    assert_eq!(last_root, pending_root);
}

/// `commit --resubmit` forces a fresh on-chain update from a Pending state
/// (escape hatch for a stuck in-flight update), rather than only re-confirming
/// the old coin. From a timed-out Pending commit, `--resubmit` (no timeout)
/// submits anew, confirms, and finalizes the generation.
#[test]
fn commit_resubmit_forces_fresh_update_from_pending() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let f = dir.path().join("r.txt");
    std::fs::write(&f, b"resubmit content").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&f)
        .args(["--key", "r"])
        .assert()
        .success();

    // First attempt times out → Pending, no generation.
    dig(&dir)
        .env("DIGSTORE_ANCHOR_MOCK_TIMEOUT", "1")
        .args(["commit", "-m", "first try"])
        .assert()
        .failure()
        .code(14);
    let (status, _) = anchor_status_and_root(&dir);
    assert_eq!(status, "pending");

    // --resubmit (no timeout env) forces a fresh update → confirms + finalizes.
    dig(&dir)
        .args(["commit", "-m", "resubmit", "--resubmit"])
        .assert()
        .success();
    dig(&dir)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deployment 0"));
    let (status, _) = anchor_status_and_root(&dir);
    assert_eq!(status, "confirmed");
}

// ---------------------------------------------------------------------------
// `commit` offers to publish the confirmed deployment to DIGHub.
// The offline harness runs non-interactively (stdin/stdout are not a TTY), so the
// interactive prompt is gated off — only the explicit `--push` flag pushes. These
// tests exercise: --push pushes to the in-process §21 server; --no-push and the
// non-interactive default do NOT push (and never hang); --json output is unchanged.
// ---------------------------------------------------------------------------

/// `commit --push` pushes the confirmed deployment to the default remote (`origin`),
/// fast-forwarding the in-process §21 test server to the committed root — the same
/// target as `digstore push origin`, but without a separate command.
#[test]
fn commit_push_flag_pushes_to_remote() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pk = host_pubkey(&dir);
    let (store_id, _genesis) = {
        // store_id is stable from init; read it before any commit (root is genesis here).
        let cfg = std::fs::read_to_string(common::store_dir(&dir).join("config.toml")).unwrap();
        let line = cfg.lines().find(|l| l.contains("store_id")).unwrap();
        (line.split('"').nth(1).unwrap().to_string(), String::new())
    };

    // Empty server hosting the store so a first push fast-forwards from genesis.
    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dir)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();

    assert!(
        server_served_root(&server, &store_id).is_none(),
        "server starts at genesis (nothing pushed yet)"
    );

    let f = dir.path().join("a.txt");
    std::fs::write(&f, b"push me").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    dig(&dir)
        .args(["commit", "-m", "first", "--push"])
        .assert()
        .success()
        .stdout(predicate::str::contains("committed root"))
        .stdout(predicate::str::contains("pushed root"));

    // The server now serves the committed root.
    let (_sid, committed) = store_id_and_root(&dir);
    assert_eq!(
        server_served_root(&server, &store_id).as_deref(),
        Some(committed.as_str()),
        "commit --push fast-forwarded the remote to the committed root"
    );
}

/// `commit --no-push` finalizes the deployment but does NOT push, does NOT prompt,
/// and does NOT hang. The remote stays at genesis and the usual hint is shown.
#[test]
fn commit_no_push_flag_does_not_push() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pk = host_pubkey(&dir);
    let cfg = std::fs::read_to_string(common::store_dir(&dir).join("config.toml")).unwrap();
    let store_id = cfg
        .lines()
        .find(|l| l.contains("store_id"))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap()
        .to_string();

    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dir)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();

    std::fs::write(dir.path().join("a.txt"), b"keep local").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    dig(&dir)
        .args(["commit", "-m", "first", "--no-push"])
        .assert()
        .success()
        .stdout(predicate::str::contains("committed root"))
        .stdout(predicate::str::contains("pushed root").not())
        .stdout(predicate::str::contains("digstore push origin"));

    assert!(
        server_served_root(&server, &store_id).is_none(),
        "commit --no-push must not push: the remote stays at genesis"
    );
}

/// Non-interactive default (no flag, no TTY): commit must NOT prompt and must NOT
/// push — it behaves exactly as before, printing the `digstore push origin` hint.
/// This is the critical non-interactive-safety guard.
#[test]
fn commit_non_interactive_default_does_not_push_or_hang() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pk = host_pubkey(&dir);
    let cfg = std::fs::read_to_string(common::store_dir(&dir).join("config.toml")).unwrap();
    let store_id = cfg
        .lines()
        .find(|l| l.contains("store_id"))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap()
        .to_string();

    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dir)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();

    std::fs::write(dir.path().join("a.txt"), b"default behaviour").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    // No --push / --no-push. assert_cmd closes stdin, so a stray prompt would hang
    // (the test would time out / read EOF) — proving the gate holds.
    dig(&dir)
        .args(["commit", "-m", "first"])
        .assert()
        .success()
        .stdout(predicate::str::contains("committed root"))
        .stdout(predicate::str::contains("pushed root").not())
        .stdout(predicate::str::contains("digstore push origin"));

    assert!(
        server_served_root(&server, &store_id).is_none(),
        "non-interactive default must not push"
    );
}

/// `--json` commit output is unchanged by the push feature: it carries the same
/// fields, emits no prompt/hint, and never pushes (no `--push` given).
#[test]
fn commit_json_output_unchanged_and_never_pushes() {
    let dir = tmp_dig();
    dig(&dir).arg("init").assert().success();
    let pk = host_pubkey(&dir);
    let cfg = std::fs::read_to_string(common::store_dir(&dir).join("config.toml")).unwrap();
    let store_id = cfg
        .lines()
        .find(|l| l.contains("store_id"))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap()
        .to_string();

    let server = TestServer::start_empty(&store_id, pk);
    let store_url = format!("{}/stores/{}", server.base_url(), store_id);
    dig(&dir)
        .args(["remote", "add", "origin", &store_url])
        .assert()
        .success();

    std::fs::write(dir.path().join("a.txt"), b"json content").unwrap();
    dig(&dir).args(["add", "a.txt"]).assert().success();

    let out = dig(&dir)
        .args(["commit", "-m", "first", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "json commit should succeed");
    // Output is a single JSON object with the established fields — no prompt/hint text.
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["anchor_status"].as_str().unwrap(), "confirmed");
    assert!(v["root"].as_str().unwrap().len() == 64);
    assert!(v.get("module").is_some());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("Push this deployment"),
        "json mode must never prompt"
    );
    assert!(
        !stdout.contains("pushed root"),
        "json commit without --push must not push"
    );

    assert!(
        server_served_root(&server, &store_id).is_none(),
        "json commit without --push must not push"
    );
}
