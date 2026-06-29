//! Agent-friendly machine surface (AGENT_FRIENDLY.md): the `--help-json`
//! invocation contract and the structured `--json` error envelope, driven through
//! the REAL installed `digstore` binary.

mod common;
use common::{dig, tmp_dig};

/// `digstore --help-json` emits a COMPLETE invocation contract: the command tree,
/// the global flags, per-arg `choices`/`default`/`value_name`, AND a differentiated
/// exit-code table — so one introspection call yields everything an agent needs.
#[test]
fn help_json_is_a_complete_contract() {
    let out = dig(&tmp_dig()).args(["--help-json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    // Top-level shape.
    assert_eq!(v["name"].as_str(), Some("digstore"));
    assert!(v["version"].is_string());

    // Globals are documented once at the root and include the headline flags.
    let globals: Vec<&str> = v["globals"]
        .as_array()
        .expect("globals array")
        .iter()
        .filter_map(|a| a["long"].as_str())
        .collect();
    for g in ["json", "verbose", "quiet", "color", "store"] {
        assert!(globals.contains(&g), "missing global --{g}: {globals:?}");
    }

    // The command tree carries the headline commands.
    let commands: Vec<&str> = v["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    for c in ["new", "dev", "init", "commit", "deploy", "completion"] {
        assert!(commands.contains(&c), "missing command {c}");
    }

    // The exit-code table is present, differentiated, and includes the success row.
    let exits = v["exit_codes"].as_array().expect("exit_codes array");
    assert!(exits.len() >= 10, "exit table should be differentiated");
    assert!(exits
        .iter()
        .any(|r| r["code"] == "OK" && r["exit_code"] == 0));
    assert!(exits
        .iter()
        .any(|r| r["code"] == "INSUFFICIENT_FUNDS" && r["exit_code"] == 12));

    // A value-enum arg surfaces its choices + default (the `--color` global).
    let color = v["globals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["long"] == "color")
        .expect("--color global");
    let choices: Vec<&str> = color["choices"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s.as_str())
        .collect();
    assert!(choices.contains(&"auto") && choices.contains(&"never"));
    assert!(color["default"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d == "auto"));

    // A per-command arg surfaces its value_name (`commit --writer-key WRITER_SEED`).
    let commit = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "commit")
        .unwrap();
    let wk = commit["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["long"] == "writer-key")
        .expect("commit has --writer-key");
    assert_eq!(wk["value_name"].as_str(), Some("WRITER_SEED"));
}

/// Under `--json`, a failing command emits a STRUCTURED error object to stdout —
/// `{ok:false,error:{code,exit_code,message,hint}}` — not human prose to stderr.
/// An agent can branch on `error.code` / `error.exit_code` without scraping text.
#[test]
fn json_error_envelope_on_failure() {
    let d = tmp_dig();
    // `status` with no store here fails with NO_STORE / exit 3.
    let out = dig(&d).args(["--json", "status"]).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert_eq!(v["error"]["code"].as_str(), Some("NO_STORE"));
    assert_eq!(v["error"]["exit_code"].as_u64(), Some(3));
    assert!(v["error"]["message"].is_string());
    // NO_STORE carries an actionable hint.
    assert!(v["error"]["hint"]
        .as_str()
        .unwrap_or("")
        .contains("digstore init"));
}

/// A RUNTIME bad-argument failure under `--json` carries the INVALID_ARGUMENT code
/// (exit 2) in the structured envelope. (`link` with a non-store target validates at
/// runtime, so it flows through our CliError envelope rather than clap's own parser.)
#[test]
fn json_error_envelope_invalid_argument() {
    let d = tmp_dig();
    let out = dig(&d)
        .args(["--json", "link", "not-a-valid-store-target"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert_eq!(v["error"]["code"].as_str(), Some("INVALID_ARGUMENT"));
    assert_eq!(v["error"]["exit_code"].as_u64(), Some(2));
}
