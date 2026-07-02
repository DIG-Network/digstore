//! `digstore config node.url` — the persisted, lowest-precedence override for
//! the `CLAUDE.md` §5.3 client->node resolution ladder — driven through the
//! REAL installed `digstore` binary (not just the underlying `config` module
//! unit tests, which cover the storage layer in isolation).

mod common;
use common::{dig, tmp_dig};

/// With nothing configured, `node.url` reports unset (both human and `--json`).
#[test]
fn node_url_defaults_to_unset() {
    let dir = tmp_dig();
    let out = dig(&dir)
        .args(["config", "node.url", "--show"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("unset"),
        "expected unset hint, got: {stdout}"
    );

    let json_out = dig(&dir)
        .args(["--json", "config", "node.url", "--show"])
        .output()
        .unwrap();
    assert!(json_out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert!(v["node_url"].is_null());
}

/// Setting then showing round-trips the value, and it PERSISTS across separate
/// invocations (a fresh `dig()` command each time, only sharing the identity
/// dir via env — proving it is durable config, not in-memory state).
#[test]
fn node_url_set_persists_across_invocations() {
    let dir = tmp_dig();
    let set_out = dig(&dir)
        .args(["config", "node.url", "https://my-node.example:9778"])
        .output()
        .unwrap();
    assert!(set_out.status.success());

    let show_out = dig(&dir)
        .args(["--json", "config", "node.url", "--show"])
        .output()
        .unwrap();
    assert!(show_out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&show_out.stdout).unwrap();
    assert_eq!(v["node_url"].as_str(), Some("https://my-node.example:9778"));
}

/// `--unset` clears a previously-set value back to "(unset)".
#[test]
fn node_url_unset_clears_it() {
    let dir = tmp_dig();
    dig(&dir)
        .args(["config", "node.url", "https://my-node.example"])
        .output()
        .unwrap();
    let unset_out = dig(&dir)
        .args(["config", "node.url", "--unset"])
        .output()
        .unwrap();
    assert!(unset_out.status.success());

    let show_out = dig(&dir)
        .args(["--json", "config", "node.url", "--show"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&show_out.stdout).unwrap();
    assert!(v["node_url"].is_null());
}

/// The command surface is discoverable: `--help`/`--help-json` mention `config`
/// and the global `--node` flag (`CLAUDE.md` §6.2 agent-friendliness — a
/// machine client must be able to introspect the override without prose-
/// scraping docs).
#[test]
fn help_json_documents_node_override_surface() {
    let dir = tmp_dig();
    let out = dig(&dir).args(["--help-json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    let globals: Vec<&str> = v["globals"]
        .as_array()
        .expect("globals array")
        .iter()
        .filter_map(|a| a["long"].as_str())
        .collect();
    assert!(
        globals.contains(&"node"),
        "missing global --node: {globals:?}"
    );

    let commands: Vec<&str> = v["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(
        commands.contains(&"config"),
        "missing `config` command: {commands:?}"
    );
}
