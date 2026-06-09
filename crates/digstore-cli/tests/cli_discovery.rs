//! §8.5 Social Conventions — the `/.well-known/dig/manifest.json` discovery
//! manifest, end to end through the REAL CLI binary.
//!
//! PROVES: `digstore add --discovery` writes the discovery manifest as a NORMAL
//! resource at the conventional key, listing the publisher-elected resources with
//! labels and types; and a reader can fetch it back by its conventional retrieval
//! key (an ordinary `cat`) and parse it. Secret-keyed/unstaged resources are NOT
//! advertised.

mod common;
use common::{dig, store_id_and_root, tmp_dig};

// The displayed URN path form (leading `/` is the URN separator); the resource
// KEY parsed from it is `.well-known/dig/manifest.json` (no leading slash).
const DISCOVERY_URN_PATH: &str = "/.well-known/dig/manifest.json";
const DISCOVERY_RESOURCE_KEY: &str = ".well-known/dig/manifest.json";

#[test]
fn discovery_manifest_round_trips_by_conventional_key() {
    let dir = tmp_dig();

    // Publisher elects to expose two resources.
    let index = dir.path().join("index.html");
    std::fs::write(&index, b"<html><body>home</body></html>").unwrap();
    let data = dir.path().join("data.json");
    std::fs::write(&data, br#"{"hello":"world"}"#).unwrap();

    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&index)
        .args(["--key", "index.html"])
        .assert()
        .success();
    dig(&dir)
        .args(["add"])
        .arg(&data)
        .args(["--key", "data.json"])
        .assert()
        .success();

    // Stage the discovery manifest listing the staged resources (§8.5).
    dig(&dir).args(["add", "--discovery"]).assert().success();

    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);

    // ---- READER: fetch the discovery manifest by its CONVENTIONAL retrieval key.
    // A discoverer who knows only the store ID constructs the URN for the
    // well-known key and `cat`s it (an ordinary read). The key contains slashes,
    // which the URN's resource_key segment carries verbatim.
    let urn = format!("urn:dig:chia:{}:{}{}", store_id, root, DISCOVERY_URN_PATH);
    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(
        out.status.success(),
        "cat of discovery manifest failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // ---- PARSE it back as machine-readable JSON.
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "discovery manifest is not JSON: {e}; raw = {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });

    assert_eq!(parsed["schema_version"], 1);
    let resources = parsed["resources"].as_array().expect("resources array");
    let keys: Vec<&str> = resources
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"index.html"), "lists index.html: {keys:?}");
    assert!(keys.contains(&"data.json"), "lists data.json: {keys:?}");
    // The manifest must NOT advertise itself.
    assert!(
        !keys.contains(&DISCOVERY_RESOURCE_KEY),
        "discovery manifest must not list itself: {keys:?}"
    );

    // Each entry carries a label and a TYPE (§8.5: "with labels and types").
    for r in resources {
        assert!(r["label"].is_string(), "entry has a label: {r}");
        assert!(r["type"].is_string(), "entry has a type: {r}");
    }
    // index.html's inferred type is text/html.
    let idx = resources.iter().find(|r| r["key"] == "index.html").unwrap();
    assert_eq!(idx["type"], "text/html");
    let dat = resources.iter().find(|r| r["key"] == "data.json").unwrap();
    assert_eq!(dat["type"], "application/json");
}

#[test]
fn discovery_does_not_advertise_unstaged_secret_resource() {
    // §8.5 privacy: a resource the publisher does NOT stage before generating the
    // discovery manifest stays opaque (nothing maps a public name to it).
    let dir = tmp_dig();
    let public = dir.path().join("index.html");
    std::fs::write(&public, b"<html>public</html>").unwrap();

    dig(&dir).arg("init").assert().success();
    dig(&dir)
        .args(["add"])
        .arg(&public)
        .args(["--key", "index.html"])
        .assert()
        .success();
    // Generate discovery NOW (only index.html staged).
    dig(&dir).args(["add", "--discovery"]).assert().success();

    // Stage a "secret" resource AFTER the manifest was generated.
    let secret = dir.path().join("secret.bin");
    std::fs::write(&secret, b"top secret payload").unwrap();
    dig(&dir)
        .args(["add"])
        .arg(&secret)
        .args(["--key", "super-secret-resource-key"])
        .assert()
        .success();

    dig(&dir).args(["commit"]).assert().success();

    let (store_id, root) = store_id_and_root(&dir);
    let urn = format!("urn:dig:chia:{}:{}{}", store_id, root, DISCOVERY_URN_PATH);
    let out = dig(&dir).args(["cat", &urn]).output().unwrap();
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let keys: Vec<&str> = parsed["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"index.html"));
    assert!(
        !keys.contains(&"super-secret-resource-key"),
        "manifest must not advertise the unstaged secret resource: {keys:?}"
    );
}
