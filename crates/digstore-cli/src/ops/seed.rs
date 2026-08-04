//! Publisher-side seed push: hand a freshly-committed `.dig` capsule to the
//! operator's OWN local dig-node so the node becomes a discoverable DHT holder
//! the instant the content is published — the seed leg of the content-replication
//! flywheel (dig_ecosystem#1476, LANE B).
//!
//! This runs automatically after a SUCCESSFUL `digstore commit` (independent of
//! `--push`/DIGHUb). It is BEST-EFFORT and STRICTLY NON-FATAL: the commit already
//! succeeded and the `.dig` is on disk, so a missing/asleep node, a missing control
//! token, or any transport error yields a YELLOW warning and success, never a
//! failed commit.
//!
//! # Wire (locked by dig-node `SPEC.md` §5.5.3 / `cache.pushCapsule`)
//!
//! The capsule is pushed to the node's loopback JSON-RPC surface in ≤3 MiB base64
//! windows the node reassembles. Params: `{ store_id, root, data, offset,
//! total_length }`; the node acks `{ offset, complete, next_offset, size_bytes }`
//! (+ `served_root` / `already_cached` on completion). The client sends `offset=0`
//! first and follows `next_offset` until `complete == true`.
//!
//! # Trust — the LOCAL CONTROL TOKEN (same gate as `cache.fetchAndCache`)
//!
//! `cache.pushCapsule` makes the node a durable holder, so over loopback HTTP it
//! requires the node's master control token (`X-Dig-Control-Token`), exactly like
//! `cache.fetchAndCache` (dig-node `SPEC.md` §5.5.3 / §7). A same-machine caller
//! obtains it from [`local_control_token`]. See that function for the acquisition
//! path + the known headless gap.
//!
//! # Local tiers ONLY
//!
//! The seed push targets `dig.local` / `localhost` ONLY (never `rpc.dig.net`):
//! seeding the public gateway is not the local-cache flywheel, and an explicit
//! `--node` override may point anywhere, so this path deliberately ignores the
//! override ladder and probes only the two local tiers.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use digstore_remote::{
    resolve_node, HealthProbe, HttpHealthProbe, OverrideInputs, ResolvedTier,
    DEFAULT_LOCAL_NODE_PORT, DEFAULT_PROBE_TIMEOUT, DIG_LOCAL_HOST,
};

use crate::runtime::block_on;

/// The maximum bytes of a single push window: ≤3 MiB, mirroring dig-node's
/// `CAPSULE_WINDOW_BYTES` (a capsule exceeds the ~6 MB inline JSON-RPC ceiling, so
/// it MUST be chunked). Chosen FROM the protocol's own window ceiling, not an
/// arbitrary size.
pub const CHUNK_BYTES: usize = 3 * 1024 * 1024;

/// The JSON-RPC method the node exposes for the seed push (dig-node `SPEC.md`
/// §5.5.3). A dig-node-LOCAL method (not a `dig-rpc-protocol` variant) — pushed by
/// string over loopback HTTP.
const PUSH_METHOD: &str = "cache.pushCapsule";

/// The request header the control token is presented in — byte-identical to
/// dig-node's `control::CONTROL_TOKEN_HEADER` (canonical cross-repo contract).
const CONTROL_TOKEN_HEADER: &str = "X-Dig-Control-Token";

/// The env var a headless caller (a CI runner, a sandboxed publisher) supplies the
/// node's control token through directly, as the clean escape hatch when the
/// on-disk token file is not readable by this process. See [`local_control_token`].
const CONTROL_TOKEN_ENV: &str = "DIG_NODE_CONTROL_TOKEN";

/// The env var that overrides the node's machine-wide state dir (where the control
/// token lives) — mirrors dig-node's `DIG_NODE_STATE_DIR`.
const STATE_DIR_ENV: &str = "DIG_NODE_STATE_DIR";

/// The env var that pins the local node port — mirrors dig-node's `DIG_NODE_PORT`
/// and digstore's own §5.3 ladder (`ops::node`).
const NODE_PORT_ENV: &str = "DIG_NODE_PORT";

/// The env toggle for auto-push (`flags > env > dig.toml > default-on`).
const AUTOPUSH_ENV: &str = "DIGSTORE_AUTOPUSH";

/// The name of the control-token file inside the node's state dir (dig-node
/// `control::CONTROL_TOKEN_FILE`).
const CONTROL_TOKEN_FILE: &str = "control-token";

/// The outcome of an auto-seed attempt. NON-FATAL in every variant — the commit has
/// already succeeded before this runs, so the caller reports the outcome and always
/// returns success (SPEC: node-down is a YELLOW warning, not a failed commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedOutcome {
    /// Auto-push is disabled (`--no-cache` / `DIGSTORE_AUTOPUSH=false` / `dig.toml`
    /// `auto-push = false`). Nothing was attempted.
    Disabled,
    /// The capsule was accepted by the local node (freshly landed, or already held).
    Seeded { already_cached: bool },
    /// The seed push did not complete — no local node answered, the control token
    /// was unavailable, or the push errored. Carries the human reason for the YELLOW
    /// warning. NON-FATAL.
    NotSeeded(String),
}

/// Resolve whether auto-push is ON for this commit, with the uniform precedence
/// `flag > env (DIGSTORE_AUTOPUSH) > dig.toml (auto-push) > default-ON`. PURE.
///
/// * `no_cache_flag` — the `--no-cache` commit flag (an explicit opt-OUT wins).
/// * `env` — the parsed `DIGSTORE_AUTOPUSH` bit (`None` when unset/blank).
/// * `toml_bit` — the `dig.toml` `auto-push` bit (`None` when the field is absent).
pub fn autopush_enabled(no_cache_flag: bool, env: Option<bool>, toml_bit: Option<bool>) -> bool {
    if no_cache_flag {
        return false;
    }
    if let Some(e) = env {
        return e;
    }
    if let Some(t) = toml_bit {
        return t;
    }
    true
}

/// Parse the `DIGSTORE_AUTOPUSH` env var into an explicit bit. Recognizes
/// `1`/`true`/`yes`/`on` (→ `true`) and `0`/`false`/`no`/`off` (→ `false`);
/// unset/blank/unknown → `None` (fall through to the next precedence layer). PURE
/// over the process env.
pub fn autopush_env_bit() -> Option<bool> {
    let raw = std::env::var(AUTOPUSH_ENV).ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// A transport that POSTs one `cache.pushCapsule` window and returns the node's
/// `result` object (or `Err(message)` on any transport / JSON-RPC error). Abstracted
/// so the chunk-and-follow loop is unit-tested against a deterministic mock node
/// without a network.
#[async_trait::async_trait]
pub trait PushTransport: Send + Sync {
    async fn push_window(
        &self,
        base_url: &str,
        token: Option<&str>,
        params: Value,
    ) -> Result<Value, String>;
}

/// Push a whole capsule to the local node in ≤[`CHUNK_BYTES`] windows, following the
/// node's `next_offset` acks until `complete == true` (dig-node `SPEC.md` §5.5.3).
///
/// Returns `Ok(already_cached)` on completion (`already_cached == true` for an
/// idempotent re-push of a capsule the node already holds), or `Err(reason)` on any
/// transport/RPC error or a node that fails to advance the push.
pub async fn push_capsule_chunked(
    transport: &dyn PushTransport,
    base_url: &str,
    token: Option<&str>,
    store_id_hex: &str,
    root_hex: &str,
    bytes: &[u8],
) -> Result<bool, String> {
    let total = bytes.len() as u64;
    let mut offset: u64 = 0;
    loop {
        let start = offset as usize;
        let end = (start + CHUNK_BYTES).min(bytes.len());
        let window = &bytes[start..end];
        let params = json!({
            "store_id": store_id_hex,
            "root": root_hex,
            "data": base64::engine::general_purpose::STANDARD.encode(window),
            "offset": offset,
            "total_length": total,
        });
        let result = transport.push_window(base_url, token, params).await?;

        if result
            .get("complete")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(result
                .get("already_cached")
                .and_then(Value::as_bool)
                .unwrap_or(false));
        }
        // Not complete: the node MUST tell us where to continue. Strict forward
        // progress — a node that does not advance past our current offset would spin
        // the loop forever, so treat a non-advancing `next_offset` as an error.
        let next = result
            .get("next_offset")
            .and_then(Value::as_u64)
            .ok_or_else(|| "node did not report next_offset for an incomplete push".to_string())?;
        if next <= offset {
            return Err(format!(
                "node did not advance the push (offset {offset} → {next})"
            ));
        }
        offset = next;
    }
}

/// The local node port: `DIG_NODE_PORT` env, else the §5.3 default (9778) —
/// mirroring `ops::node::local_candidate_urls`.
fn local_port() -> u16 {
    std::env::var(NODE_PORT_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_LOCAL_NODE_PORT)
}

/// Resolve the LOCAL node base URL for the seed push — `dig.local` then `localhost`,
/// over plain HTTP (dig-node's loopback JSON-RPC surface is `http://`, not TLS;
/// mTLS is a separate node-class listener). Returns `None` when neither local tier
/// answers, so the caller NEVER seeds the public gateway.
///
/// The override ladder is deliberately ignored: an explicit `--node` may point at
/// `rpc.dig.net` or a remote peer, and seeding those is not the local-cache flywheel.
async fn resolve_local_node(probe: &dyn HealthProbe, timeout: Duration) -> Option<String> {
    let port = local_port();
    let dig_local = format!("http://{DIG_LOCAL_HOST}:{port}");
    let localhost = format!("http://localhost:{port}");
    let resolved = resolve_node(
        &OverrideInputs::default(),
        &dig_local,
        &localhost,
        probe,
        timeout,
    )
    .await;
    match resolved.tier {
        ResolvedTier::DigLocal | ResolvedTier::Localhost => Some(resolved.base_url),
        // PublicGateway (nothing local answered) / Override — never seeded.
        _ => None,
    }
}

/// The ordered directories a same-machine caller looks for the node's
/// `control-token` file in — MIRRORS dig-node's `state::state_dir` resolution
/// (`crate::state` in dig-node-service): the `DIG_NODE_STATE_DIR` override, the
/// per-OS machine-wide state dir, then the legacy per-user dir.
///
/// This duplication is a canonical cross-repo contract (see the GAP note in
/// [`local_control_token`]); it is kept minimal and documented so a dig-node change
/// to the token location is a coordinated update here.
fn control_token_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut dirs = Vec::new();

    if let Ok(over) = std::env::var(STATE_DIR_ENV) {
        let over = over.trim();
        if !over.is_empty() {
            dirs.push(PathBuf::from(over));
        }
    }

    #[cfg(windows)]
    {
        if let Ok(pd) = std::env::var("PROGRAMDATA") {
            if !pd.trim().is_empty() {
                dirs.push(PathBuf::from(pd).join("DigNode"));
            }
        }
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            if !la.trim().is_empty() {
                dirs.push(PathBuf::from(la).join("DigNode")); // legacy per-user
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Application Support/DigNode"));
        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                dirs.push(PathBuf::from(home).join("DigNode")); // legacy per-user
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs.push(PathBuf::from("/var/lib/dig-node"));
        dirs.push(PathBuf::from("/etc/dig-node"));
        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                dirs.push(PathBuf::from(home).join("DigNode")); // legacy per-user dev run
            }
        }
    }

    dirs
}

/// Obtain the local node's master control token for the `X-Dig-Control-Token` header.
///
/// Precedence:
/// 1. `DIG_NODE_CONTROL_TOKEN` env — the clean headless/CI escape hatch.
/// 2. the node's on-disk `control-token` file, read READ-ONLY from the same state
///    dir dig-node resolves ([`control_token_dirs`]) — the standard local-capability
///    pattern the DIG Browser / extension use.
///
/// GAP (reported as a dig-node follow-up): a headless publisher CLI run as a normal
/// user CANNOT read the token when the node runs as a service under another OS
/// account (LocalSystem / a root daemon minted it with a restrictive ACL). There is
/// today no cross-repo published path for a same-machine CLI to obtain it without
/// either the `DIG_NODE_CONTROL_TOKEN` env or matching account/elevation. When the
/// token is absent the seed push proceeds WITHOUT the header and the node answers
/// `Unauthorized`, which surfaces as a NON-FATAL YELLOW warning (never a bypass).
pub fn local_control_token() -> Option<String> {
    if let Ok(t) = std::env::var(CONTROL_TOKEN_ENV) {
        let t = t.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    for dir in control_token_dirs() {
        if let Ok(s) = std::fs::read_to_string(dir.join(CONTROL_TOKEN_FILE)) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The production [`PushTransport`]: a `cache.pushCapsule` JSON-RPC POST to the
/// node's loopback surface. Pinned to a DIRECT connection (`.no_proxy()`) so the
/// token-bearing POST is never routed through a hostile env proxy — mirroring
/// dig-node's own `control_client` rationale.
struct HttpPushTransport {
    http: reqwest::Client,
    timeout: Duration,
}

impl HttpPushTransport {
    fn new(timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http, timeout }
    }
}

#[async_trait::async_trait]
impl PushTransport for HttpPushTransport {
    async fn push_window(
        &self,
        base_url: &str,
        token: Option<&str>,
        params: Value,
    ) -> Result<Value, String> {
        let url = format!("{}/", base_url.trim_end_matches('/'));
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": PUSH_METHOD, "params": params });
        let mut req = self.http.post(&url).json(&body);
        if let Some(t) = token {
            req = req.header(CONTROL_TOKEN_HEADER, t);
        }
        let resp = tokio::time::timeout(self.timeout, req.send())
            .await
            .map_err(|_| "the local dig-node did not respond in time".to_string())?
            .map_err(|e| format!("could not reach the local dig-node: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("the local dig-node returned an unreadable response: {e}"))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(format!("the local dig-node rejected the seed push: {msg}"));
        }
        Ok(v.get("result").cloned().unwrap_or_else(|| json!({})))
    }
}

/// Auto-seed a freshly-committed capsule to the operator's local dig-node.
///
/// BEST-EFFORT + NON-FATAL: reads the `.dig` from disk, resolves a LOCAL node,
/// obtains the control token, and pushes the capsule chunked. Any failure (disabled,
/// no node, no token, transport error) is captured in the returned [`SeedOutcome`] —
/// this NEVER returns an error, because the commit has already succeeded.
///
/// `autopush` is the resolved config bit ([`autopush_enabled`]).
pub fn seed_after_commit(
    store_id_hex: &str,
    root_hex: &str,
    dig_path: &Path,
    autopush: bool,
) -> SeedOutcome {
    if !autopush {
        return SeedOutcome::Disabled;
    }
    let bytes = match std::fs::read(dig_path) {
        Ok(b) => b,
        Err(e) => {
            return SeedOutcome::NotSeeded(format!(
                "could not read the committed capsule for seeding: {e}"
            ))
        }
    };
    let token = local_control_token();

    // Drive the async resolve+push on a short-lived runtime. `block_on` only fails to
    // BUILD the runtime — map that (and any inner reason) to a NON-FATAL outcome.
    let outcome = block_on(async move {
        let probe = HttpHealthProbe::default();
        let Some(base_url) = resolve_local_node(&probe, DEFAULT_PROBE_TIMEOUT).await else {
            return SeedOutcome::NotSeeded(
                "local dig-node not running: committed locally, not yet cached/reshared"
                    .to_string(),
            );
        };
        let transport = HttpPushTransport::new(Duration::from_secs(30));
        match push_capsule_chunked(
            &transport,
            &base_url,
            token.as_deref(),
            store_id_hex,
            root_hex,
            &bytes,
        )
        .await
        {
            Ok(already_cached) => SeedOutcome::Seeded { already_cached },
            Err(reason) => SeedOutcome::NotSeeded(reason),
        }
    });

    outcome.unwrap_or_else(|_| {
        SeedOutcome::NotSeeded("could not start the seed push runtime".to_string())
    })
}

/// Report a [`SeedOutcome`] to the human console. A [`SeedOutcome::NotSeeded`] is a
/// YELLOW warning (never an error — the commit succeeded); a success is a quiet
/// confirmation that the capsule is now discoverable on the local node.
pub fn report_seed(ui: &crate::ui::Ui, outcome: &SeedOutcome) {
    match outcome {
        SeedOutcome::Disabled => {}
        SeedOutcome::Seeded { already_cached } => {
            if *already_cached {
                ui.line("  seeded on your local dig-node (already cached) — discoverable now");
            } else {
                ui.line("  seeded on your local dig-node — discoverable + reshared now");
            }
        }
        SeedOutcome::NotSeeded(reason) => ui.warn(reason),
    }
}

/// The JSON fields describing a seed outcome, folded into `commit --json` output so a
/// scripted publisher can see whether the capsule was seeded.
pub fn seed_json_fields(outcome: &SeedOutcome) -> Value {
    match outcome {
        SeedOutcome::Disabled => json!({ "seeded": false, "seed_skipped": true }),
        SeedOutcome::Seeded { already_cached } => {
            json!({ "seeded": true, "already_cached": already_cached })
        }
        SeedOutcome::NotSeeded(reason) => json!({ "seeded": false, "seed_warning": reason }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ---- config precedence (c) ------------------------------------------------

    #[test]
    fn autopush_defaults_on() {
        assert!(autopush_enabled(false, None, None));
    }

    #[test]
    fn no_cache_flag_disables_regardless_of_env_and_toml() {
        // The flag is the highest-precedence opt-out: even env=on / toml=on cannot
        // re-enable it.
        assert!(!autopush_enabled(true, Some(true), Some(true)));
    }

    #[test]
    fn env_beats_toml_and_default() {
        assert!(!autopush_enabled(false, Some(false), Some(true)));
        assert!(autopush_enabled(false, Some(true), Some(false)));
    }

    #[test]
    fn toml_beats_default_when_no_flag_or_env() {
        assert!(!autopush_enabled(false, None, Some(false)));
    }

    // ---- a deterministic mock node -------------------------------------------

    /// A mock dig-node implementing the `cache.pushCapsule` ack protocol: it
    /// reassembles windows STRICTLY forward (panicking on an out-of-order offset, so
    /// a broken chunk-follow loop fails loudly), records every presented token, and
    /// acks `complete` once the declared `total_length` is met.
    struct MockNode {
        received: Mutex<Vec<u8>>,
        windows: Mutex<u32>,
        tokens: Mutex<Vec<Option<String>>>,
        already_cached: bool,
    }

    impl MockNode {
        fn new(already_cached: bool) -> Self {
            Self {
                received: Mutex::new(Vec::new()),
                windows: Mutex::new(0),
                tokens: Mutex::new(Vec::new()),
                already_cached,
            }
        }
    }

    #[async_trait::async_trait]
    impl PushTransport for MockNode {
        async fn push_window(
            &self,
            _base_url: &str,
            token: Option<&str>,
            params: Value,
        ) -> Result<Value, String> {
            *self.windows.lock().unwrap() += 1;
            self.tokens.lock().unwrap().push(token.map(str::to_string));

            let offset = params["offset"].as_u64().unwrap();
            let total = params["total_length"].as_u64().unwrap();
            let data = base64::engine::general_purpose::STANDARD
                .decode(params["data"].as_str().unwrap())
                .unwrap();

            if self.already_cached {
                // Idempotent re-push: complete immediately, never accumulate.
                return Ok(json!({
                    "offset": total, "complete": true, "next_offset": Value::Null,
                    "size_bytes": total, "served_root": params["root"], "already_cached": true,
                }));
            }

            let mut buf = self.received.lock().unwrap();
            assert_eq!(offset as usize, buf.len(), "push must be strictly forward");
            buf.extend_from_slice(&data);
            let assembled = buf.len() as u64;
            if assembled >= total {
                Ok(json!({
                    "offset": offset, "complete": true, "next_offset": Value::Null,
                    "size_bytes": assembled, "served_root": params["root"],
                }))
            } else {
                Ok(json!({
                    "offset": offset, "complete": false, "next_offset": assembled,
                    "size_bytes": assembled,
                }))
            }
        }
    }

    fn hex64(b: u8) -> String {
        hex::encode([b; 32])
    }

    // ---- (a) chunked delivery across ≥2 windows for a >3 MiB capsule ----------

    #[tokio::test]
    async fn pushes_large_capsule_chunked_and_reassembles_exactly() {
        // A capsule just over 2× the window ceiling ⇒ MUST take 3 windows. Sized
        // FROM the protocol's own 3 MiB window limit, not an arbitrary value.
        let size = 2 * CHUNK_BYTES + 1024;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let node = MockNode::new(false);

        let already = push_capsule_chunked(
            &node,
            "http://localhost:9778",
            Some("tok"),
            &hex64(0xAB),
            &hex64(0xCD),
            &bytes,
        )
        .await
        .expect("push completes");

        assert!(!already);
        assert_eq!(
            *node.received.lock().unwrap(),
            bytes,
            "the node must reassemble the capsule byte-for-byte"
        );
        assert!(
            *node.windows.lock().unwrap() >= 2,
            "a >3 MiB capsule must be chunked across ≥2 windows, got {}",
            *node.windows.lock().unwrap()
        );
    }

    // ---- (e) the control-token header is carried on every window --------------

    #[tokio::test]
    async fn every_window_carries_the_control_token() {
        let bytes: Vec<u8> = (0..CHUNK_BYTES + 512).map(|i| (i % 97) as u8).collect();
        let node = MockNode::new(false);

        push_capsule_chunked(
            &node,
            "http://localhost:9778",
            Some("secret-token"),
            &hex64(0x01),
            &hex64(0x02),
            &bytes,
        )
        .await
        .unwrap();

        let tokens = node.tokens.lock().unwrap();
        assert!(tokens.len() >= 2, "expected ≥2 windows");
        assert!(
            tokens.iter().all(|t| t.as_deref() == Some("secret-token")),
            "every window must present the control token: {tokens:?}"
        );
    }

    /// Idempotent re-push: the node reports `already_cached` and the loop returns
    /// `Ok(true)` without asserting reassembly.
    #[tokio::test]
    async fn already_cached_repush_is_reported() {
        let bytes = vec![7u8; 10];
        let node = MockNode::new(true);
        let already = push_capsule_chunked(
            &node,
            "http://localhost:9778",
            Some("t"),
            &hex64(0x03),
            &hex64(0x04),
            &bytes,
        )
        .await
        .unwrap();
        assert!(
            already,
            "a re-push of a held capsule reports already_cached"
        );
    }

    // ---- (d) local-tier only: never rpc.dig.net -------------------------------

    struct FixedProbe {
        answers: bool,
    }

    #[async_trait::async_trait]
    impl HealthProbe for FixedProbe {
        async fn probe(&self, _base_url: &str, _timeout: Duration) -> bool {
            self.answers
        }
    }

    #[tokio::test]
    async fn resolves_local_node_when_a_local_tier_answers() {
        let probe = FixedProbe { answers: true };
        let url = resolve_local_node(&probe, DEFAULT_PROBE_TIMEOUT)
            .await
            .expect("a local tier answered");
        // dig.local is the first local tier probed; it must be an http loopback URL,
        // NEVER the public gateway.
        assert!(url.starts_with("http://dig.local"), "url={url}");
        assert!(!url.contains("rpc.dig.net"));
    }

    #[tokio::test]
    async fn returns_none_when_no_local_tier_answers_never_public_gateway() {
        let probe = FixedProbe { answers: false };
        // Nothing local answers ⇒ the §5.3 ladder would fall through to the public
        // gateway, but the seed path must REFUSE it and return None.
        assert_eq!(
            resolve_local_node(&probe, DEFAULT_PROBE_TIMEOUT).await,
            None
        );
    }

    // ---- (b) node-down is a non-fatal NotSeeded outcome -----------------------

    #[test]
    fn seed_after_commit_disabled_short_circuits() {
        // Disabled ⇒ Disabled, and the `.dig` path is never touched (a non-existent
        // path proves no read was attempted).
        let outcome = seed_after_commit(
            &hex64(0x05),
            &hex64(0x06),
            Path::new("/nonexistent/should-not-be-read.dig"),
            false,
        );
        assert_eq!(outcome, SeedOutcome::Disabled);
    }

    #[test]
    fn seed_after_commit_missing_file_is_non_fatal() {
        // A missing capsule (enabled) is a NON-FATAL NotSeeded, never a panic/error.
        let outcome = seed_after_commit(
            &hex64(0x07),
            &hex64(0x08),
            Path::new("/nonexistent/missing.dig"),
            true,
        );
        assert!(matches!(outcome, SeedOutcome::NotSeeded(_)));
    }

    #[test]
    fn seed_json_fields_shapes() {
        assert_eq!(
            seed_json_fields(&SeedOutcome::Disabled)["seeded"],
            json!(false)
        );
        assert_eq!(
            seed_json_fields(&SeedOutcome::Seeded {
                already_cached: false
            })["seeded"],
            json!(true)
        );
        assert_eq!(
            seed_json_fields(&SeedOutcome::NotSeeded("x".into()))["seed_warning"],
            json!("x")
        );
    }
}
