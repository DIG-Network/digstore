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
use tempfile::TempDir;

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
        init.args(["init", "site"])
            .output()
            .unwrap()
            .status
            .success(),
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
        init.args(["init", "site"])
            .output()
            .unwrap()
            .status
            .success(),
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
        init.args(["init", "site"])
            .output()
            .unwrap()
            .status
            .success(),
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

/// `--show` must not present an unapproved value as the configuration.
///
/// Found by driving the installed binary: a repo carrying `evil.example` was
/// correctly IGNORED by the resolver, but `config node.url --local --show`
/// printed `https://evil.example` bare — so a user investigating why a clone
/// behaves oddly is shown the attacker's URL as though it were their setting.
/// The gate held; the reporting undermined it.
#[test]
fn showing_an_unapproved_project_node_url_marks_it_as_ignored() {
    let dir = tmp_dig();
    let mut init = dig(&dir);
    assert!(
        init.args(["init", "site"])
            .output()
            .unwrap()
            .status
            .success(),
        "fixture setup: init must succeed"
    );
    std::fs::write(
        dir.path().join(".dig").join("node.toml"),
        "[node]\nurl = \"https://attacker.example\"\n",
    )
    .unwrap();

    let out = dig(&dir)
        .args(["config", "node.url", "--local", "--show"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("NOT APPROVED"),
        "an unapproved value must be shown as not in effect: {text}"
    );

    // The machine-readable form must carry the same distinction, or a script
    // reading `node_url` would act on a value digs is ignoring.
    let json = dig(&dir)
        .args(["config", "node.url", "--local", "--show", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("--json must emit valid JSON");
    assert_eq!(
        v["in_effect"], false,
        "an unapproved value must not report in_effect: {v}"
    );

    // Once approved, the same value reports plainly — so this is a marker, not
    // a blanket warning that would fire on every legitimate setting.
    assert!(dig(&dir)
        .args(["config", "node.url", "--local", "https://attacker.example"])
        .output()
        .unwrap()
        .status
        .success());
    let after = dig(&dir)
        .args(["config", "node.url", "--local", "--show", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
    assert_eq!(
        v["in_effect"], true,
        "an approved value must report in_effect: {v}"
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
        init.args(["init", "site"])
            .output()
            .unwrap()
            .status
            .success(),
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

/// A local node that answers the ladder's health probe, on an ephemeral port.
///
/// Returns the port. The thread serves until the test process exits; each probe is one
/// short-lived connection, so a simple accept loop is enough.
fn spawn_health_node() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind health node");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            // Read just enough to let the client finish sending; the path does not matter,
            // the ladder only needs a 2xx.
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut s, &mut buf);
            let body = b"{\"status\":\"ok\"}";
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            use std::io::Write as _;
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
        }
    });
    port
}

/// A refused `node.toml` must not knock the caller off their own node.
///
/// Rejecting the spoofable value was the fix; propagating that rejection turned it into a
/// FORCED-DOWNGRADE primitive — a hostile repo that cannot redirect you could still knock you onto
/// the public gateway.
///
/// # Why this test stands up a real node instead of comparing two runs
///
/// The obvious shape — resolve with no project file, resolve with the refused one, assert equal —
/// is VACUOUS on a machine with no local node. `doctor`'s fallback and the ladder's no-node
/// outcome are both `rpc.dig.net`, so with nothing local the two sides are byte-identical whether
/// or not the downgrade bug is present: it passes green in CI and only fails on a developer laptop
/// that happens to be running a node. "Knocked off your own node" is undetectable on a machine
/// that has no own node.
///
/// So the test provides one. A health listener on an ephemeral port becomes rung 3, and the
/// `dig.local` rungs above it are forced to fail by routing them through a dead proxy while
/// `NO_PROXY` exempts loopback. That makes the winning rung deterministic on any machine, and the
/// assertion — resolution stays on `http://localhost:<port>` — actually discriminates.
#[test]
fn a_refused_project_node_file_still_lets_the_ladder_run() {
    let port = spawn_health_node();
    let dead = dead_addr();

    // rung 1/2 (`dig.local`) go through a proxy that is not listening and fail; rung 3
    // (`localhost`) is exempted by NO_PROXY and reaches the listener above. Deterministic
    // regardless of whether the host running the tests has a real node.
    let configured = |dir: &TempDir| {
        let mut cmd = dig(dir);
        cmd.env("HTTP_PROXY", &dead)
            .env("HTTPS_PROXY", &dead)
            .env("ALL_PROXY", &dead)
            .env("NO_PROXY", "localhost,127.0.0.1")
            .env("no_proxy", "localhost,127.0.0.1")
            .env("DIG_NODE_PORT", port.to_string());
        cmd
    };

    let resolve = |dir: &TempDir| -> (String, String) {
        let out = configured(dir).args(["doctor"]).output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let line = text
            .lines()
            .find(|l| l.contains("default remote"))
            .unwrap_or_else(|| panic!("doctor must report a resolved remote: {text}"))
            .to_string();
        (line, text)
    };

    let init = |dir: &TempDir| {
        assert!(
            dig(dir)
                .args(["init", "site"])
                .output()
                .unwrap()
                .status
                .success(),
            "fixture setup: init must succeed"
        );
    };

    let expected = format!("http://localhost:{port}");

    // Baseline: no project file. This must land on the local node, or the fixture is not
    // exercising what it claims and every assertion below is meaningless.
    let baseline_dir = tmp_dig();
    init(&baseline_dir);
    let (baseline, baseline_text) = resolve(&baseline_dir);
    assert!(
        baseline.contains(&expected),
        "fixture: the ladder must reach the local health node, got: {baseline}\n{baseline_text}"
    );

    // Now the same project, carrying a node.url the guard refuses.
    let refused_dir = tmp_dig();
    init(&refused_dir);
    // TWO-CHARACTER escapes. TOML decodes `\n`/`\t` into real control characters, and it is the
    // DECODED value the guard rejects. Writing raw bytes makes this an invalid TOML basic string,
    // which fails earlier on a different path and never reaches the guard at all.
    let node_toml = refused_dir.path().join(".dig").join("node.toml");
    std::fs::write(
        &node_toml,
        "[node]\nurl = \"https://rpc.dig.net\\n\\t\\t\\t\\t.evil.example/\"\n",
    )
    .unwrap();
    // Precondition, because this fixture has been written the wrong way twice: the file must hold
    // the escape as two characters, not a literal newline.
    let on_disk = std::fs::read_to_string(&node_toml).unwrap();
    assert!(
        on_disk.contains("\\n") && !on_disk.contains("net\n\t"),
        "fixture must hold the two-character escape, not a raw newline: {on_disk:?}"
    );

    let (with_refused, text) = resolve(&refused_dir);

    // The user is told, rather than the value being dropped in silence.
    assert!(
        text.contains("ignoring this project's node.url"),
        "a refused project node.url must be reported: {text}"
    );

    // …and refused BY THE GUARD, naming the control character.
    //
    // This assertion is what keeps the fixture honest. Writing raw `\n`/`\t` bytes instead of the
    // two-character escapes makes the file an invalid TOML basic string, so it is rejected one
    // layer EARLIER — and because both paths emit the same "ignoring this project's node.url"
    // warning, the assertion above passes either way while the guard is never exercised. That
    // regression shipped once. Asserting the REASON is what distinguishes them.
    assert!(
        text.contains("line feed"),
        "the control-character guard must be what refused this, not a TOML syntax error — \
         check the fixture uses two-character escapes: {text}"
    );

    // Never the attacker's host. Asserted on the RESOLVED line specifically: the warning above
    // legitimately names the URL, so a whole-output check could never fail.
    assert!(
        !with_refused.contains("evil.example"),
        "must never resolve to the attacker host; resolved: {with_refused}"
    );

    // THE PROPERTY: a refused file contributes nothing, so the local node still wins.
    assert!(
        with_refused.contains(&expected),
        "a refused project file knocked resolution off the local node — that is the forced \
         downgrade. resolved: {with_refused}"
    );
    assert!(
        !with_refused.contains("rpc.dig.net"),
        "resolution fell through to the public gateway despite a live local node: {with_refused}"
    );
    assert_eq!(
        with_refused, baseline,
        "a refused project file changed which endpoint won"
    );
}
