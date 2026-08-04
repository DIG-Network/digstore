//! #2099, end to end through the REAL binary: what happens when no DIG node is
//! running on this machine.
//!
//! The unit tests in `ops::node` pin the decision as a pure function; this file
//! proves the wiring around it — that a command actually reaches that decision,
//! and that the user sees the intended outcome on stdout/stderr with the
//! intended exit code.
//!
//! **Making "no local node" true in a test.** The ladder probes fixed hosts
//! (`dig.local`, `localhost`), and a developer machine may well be running a
//! real dig-node — as the machine this was written on was. Rather than stop the
//! developer's node (destructive, and it would make the test order-dependent),
//! every probe is routed through a proxy address where nothing listens, so all
//! three rungs fail for THIS PROCESS ONLY. `rpc.dig.net` is returned unprobed,
//! so the ladder still reports the public-gateway tier — exactly the state a
//! machine with no node is in.

mod common;
use common::{dig, tmp_dig};

use std::net::TcpListener;

/// An address nothing is listening on: bind a port, learn it, drop the listener.
fn dead_addr() -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().unwrap();
    drop(l);
    format!("http://{addr}")
}

/// Route every outbound HTTP(S) request through a dead proxy, so no ladder rung
/// can answer. `NO_PROXY` is cleared explicitly: a value inherited from the
/// developer's environment (commonly `localhost,127.0.0.1`) would exempt the
/// very rungs this needs to fail, and the test would silently stop testing
/// anything.
fn with_no_reachable_node(cmd: &mut assert_cmd::Command) {
    let dead = dead_addr();
    cmd.env("HTTP_PROXY", &dead)
        .env("HTTPS_PROXY", &dead)
        .env("ALL_PROXY", &dead)
        .env("NO_PROXY", "")
        .env("no_proxy", "");
}

/// A publish needs a node the user controls: `push` signs every request with the
/// caller's identity key, so falling through to the public gateway would ship
/// their content AND their signatures to a server they never chose. It must
/// refuse, and the refusal must be actionable.
#[test]
fn push_without_a_local_node_refuses_with_check_and_install_instructions() {
    let dir = tmp_dig();
    let mut init = dig(&dir);
    assert!(
        init.args(["init", "site"]).output().unwrap().status.success(),
        "fixture setup: init must succeed"
    );

    let mut cmd = dig(&dir);
    with_no_reachable_node(&mut cmd);
    let out = cmd.args(["push", "origin"]).output().unwrap();

    assert_eq!(
        out.status.code(),
        Some(19),
        "push with no local node must exit NO_LOCAL_NODE (19); got {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The two things the user asked to be told, plus the way out.
    assert!(
        text.contains("dig-node status"),
        "must say how to CHECK the node: {text}"
    );
    assert!(
        text.contains("https://dig.net/install.sh") && text.contains("https://dig.net/install.ps1"),
        "must say where to DOWNLOAD it, for both platform families: {text}"
    );
    assert!(
        text.contains("https://docs.dig.net/docs/run-a-node"),
        "must link the published docs: {text}"
    );
    assert!(
        text.contains("config node.url --local"),
        "must offer the deliberate-remote escape hatch: {text}"
    );
    // And it must NOT have quietly published anywhere.
    assert!(
        !text.contains("pushed root"),
        "nothing may be published when the node is missing: {text}"
    );
}

/// The other half of the split, on the SAME fixture and the SAME dead-proxy
/// world — only the operation differs. Reading is allowed to fall through so
/// that someone with no node installed can still consume content, but it must
/// say so rather than pretend the read was local.
///
/// Asserting this pair together is what proves a SPLIT exists: a build that
/// refused everything, or allowed everything, fails one of the two.
#[test]
fn a_read_without_a_local_node_falls_through_but_says_so() {
    let dir = tmp_dig();
    let mut init = dig(&dir);
    assert!(
        init.args(["init", "site"]).output().unwrap().status.success(),
        "fixture setup: init must succeed"
    );

    let mut cmd = dig(&dir);
    with_no_reachable_node(&mut cmd);
    let out = cmd.args(["doctor"]).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // It reached the gateway rather than refusing outright…
    assert!(
        text.contains("rpc.dig.net"),
        "a read must still resolve somewhere: {text}"
    );
    // …and it did NOT raise the write-path refusal.
    assert!(
        !text.contains("NO_LOCAL_NODE"),
        "a read must not be blocked by the local-node requirement: {text}"
    );
}

/// A repository can carry `.dig/node.toml`, and digs signs its requests with the
/// user's identity key — so an unapproved value must not route anything. This
/// runs non-interactively (no TTY under the test harness), which is the case
/// that must never silently adopt the value.
#[test]
fn an_unapproved_project_node_url_never_routes_a_request() {
    let dir = tmp_dig();
    let mut init = dig(&dir);
    assert!(
        init.args(["init", "site"]).output().unwrap().status.success(),
        "fixture setup: init must succeed"
    );

    // Simulate what arrives in a fresh clone: the file, with no approval record.
    let workspace = dir.path().join(".dig");
    std::fs::write(
        workspace.join("node.toml"),
        "[node]\nurl = \"https://attacker.example\"\n",
    )
    .unwrap();

    let out = dig(&dir).args(["doctor"]).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The user is TOLD the value was ignored, and why — silently dropping a
    // setting someone deliberately committed would be its own bug.
    assert!(
        text.contains("has not been approved") && text.contains("attacker.example"),
        "must explain that the project's node.url was ignored: {text}"
    );

    // …and the endpoint actually used is NOT the attacker's. Asserted on the
    // resolved-remote LINE specifically: a whole-output `!contains` would be
    // defeated by the warning above, which legitimately names the URL — and
    // would have passed here for the wrong reason.
    let resolved = text
        .lines()
        .find(|l| l.contains("default remote"))
        .unwrap_or_else(|| panic!("doctor must report a resolved remote: {text}"));
    assert!(
        !resolved.contains("attacker.example"),
        "an unapproved repo-carried node.url must never be contacted; resolved: {resolved}"
    );
}

/// …and the same file, once approved, IS used — so the guard is a gate rather
/// than a blanket refusal that would make `--local` pointless. Same fixture,
/// same file; the only difference is that the user set it through the command.
#[test]
fn an_approved_project_node_url_is_used() {
    let dir = tmp_dig();
    let mut init = dig(&dir);
    assert!(
        init.args(["init", "site"]).output().unwrap().status.success(),
        "fixture setup: init must succeed"
    );

    // Setting it through the command is the approval.
    let set = dig(&dir)
        .args(["config", "node.url", "--local", "https://chosen.example"])
        .output()
        .unwrap();
    assert!(set.status.success());

    let out = dig(&dir).args(["doctor"]).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("chosen.example"),
        "an approved project node.url must be used: {text}"
    );
}
