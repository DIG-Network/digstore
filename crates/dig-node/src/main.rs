//! dig-node — the DIG Browser's local node sidecar.
//!
//! A loopback JSON-RPC server implementing the SAME `dig.getContent` contract as
//! rpc.dig.net, but LOCAL-FIRST: a `dig://` request is served from a locally
//! cached `.dig` store module (via `digstore_host::serve_blind`, which
//! instantiates the compiled module and returns a `ContentResponse` =
//! ciphertext + merkle proof + chunk_lens), and only on a cache miss is it
//! proxied to rpc.dig.net. The browser points its dig handler at this node, so
//! once a store is cached locally every resource in it is served without leaving
//! the machine. Cached store modules are evicted with an LRU size cap (default
//! 1 GiB).
//!
//! Native Rust so the compiled-module serve path (BLS, wasmtime) works.
//!
//! Cache layout: `<cache_dir>/<store_id_hex>/<root_hex>.module` — the compiled
//! module bytes for that store at that root. The browser sends a concrete root
//! (rootless URNs are resolved to the singleton tip by dig-resolver first), so a
//! module is keyed by (store_id, root).
//!
//! Authenticated whole-store sync (§21.9): on a local cache miss for a concrete
//! (store, root), the node fetches the WHOLE `.dig` module from rpc.dig.net's §21
//! `GET /stores/{id}/module` endpoint and caches it, then serves every subsequent
//! resource in that store locally. That endpoint is dighub-auth gated (it 401s for
//! anonymous clients), so the node carries a native Chia identity signer (paper
//! §21.9): it stamps `X-Dig-Identity/-Timestamp/-Nonce/-Auth` on the request using
//! the SAME persistent identity key the digstore CLI uses
//! ([`digstore_remote::identity`]). The signer is best-effort — if no identity key
//! is available the node simply skips the authenticated sync and falls back to the
//! per-resource proxy below, so it still serves whatever modules are already
//! present (e.g. the user's own digstore stores) and proxies the rest.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use base64::Engine;
use digstore_chain::coinset::Coinset;
use digstore_chain::singleton::sync_datastore;
use digstore_core::codec::{Decode, Encode};
use digstore_core::wire::ContentResponse;
use digstore_core::Bytes32;
use digstore_host::{serve_blind, BlindServeConfig};
use digstore_remote::{identity, DigClient};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const RPC_FALLBACK: &str = "https://rpc.dig.net/";
/// Per-window ciphertext cap (bytes) when paging the JSON-RPC response.
const WINDOW: usize = 3 * 1024 * 1024;
/// Default LRU cap for the on-disk module cache.
const DEFAULT_CACHE_CAP: u64 = 1024 * 1024 * 1024; // 1 GiB

struct Node {
    cache_dir: PathBuf,
    cache_cap: u64,
    http: reqwest::Client,
    /// Upstream rpc.dig.net base URL for the JSON-RPC proxy and the §21 module
    /// sync. Defaults to [`RPC_FALLBACK`]; overridden by `DIG_NODE_UPSTREAM` (a
    /// node-specific name, distinct from the browser's own `DIG_RPC_ENDPOINT`
    /// which points the browser AT this node — reusing that name would make the
    /// node proxy to itself).
    upstream: String,
    /// Serialize cache mutation (eviction) so concurrent requests don't race.
    cache_lock: Mutex<()>,
    /// The persistent §21.9 identity SEED, loaded once at startup. `Some` enables
    /// authenticated whole-store sync (the node mints a fresh `RequestIdentity`
    /// per request via `identity::identity_from_seed`); `None` disables it (the
    /// node falls back to the per-resource proxy). The 32-byte seed — not the
    /// reconstructed BLS key — is held so the signer closure stays `Send + Sync`.
    identity_seed: Option<[u8; 32]>,
}

fn cache_dir() -> PathBuf {
    std::env::var("DIG_NODE_CACHE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let root = std::env::var("LOCALAPPDATA")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| ".".to_string());
            PathBuf::from(root).join("DigNode").join("cache")
        })
}

/// Path of a cached store module for (store_id, root), if present. Modules live
/// under `<cache>/modules/` — populated out-of-band (a local digstore store, or
/// authed whole-store sync) and served via `serve_blind`.
fn module_path(dir: &Path, store_hex: &str, root_hex: &str) -> PathBuf {
    dir.join("modules")
        .join(store_hex)
        .join(format!("{root_hex}.module"))
}

/// Filesystem-safe filename for one cached proxy-response window, keyed by
/// (store, root, retrieval_key, offset). All inputs are hex (or empty), so the
/// only sanitizing needed is to reject anything non-hex defensively and bound
/// the length — a key collision would only mean a cache miss, never corruption,
/// because the browser merkle-verifies every response.
fn response_key(store: &str, root: &str, rk: &str, offset: usize) -> String {
    fn hexish(s: &str) -> &str {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            s
        } else {
            "x"
        }
    }
    format!(
        "{}_{}_{}_{}.json",
        hexish(store),
        hexish(root),
        hexish(rk),
        offset
    )
}

/// Is this request a candidate for authenticated whole-store sync? Only when we
/// have BOTH a concrete store id and a concrete generation root, each a canonical
/// 32-byte (64-hex) value. A rootless request (`root` empty, or the `"latest"`
/// sentinel, or anything non-hex) is NOT eligible: the browser resolves rootless
/// URNs to a concrete root via dig-resolver *before* calling, so a non-concrete
/// root here means the synced module could not be keyed deterministically.
fn sync_eligible(store_hex: &str, root_hex: &str) -> bool {
    fn is_hex64(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
    }
    is_hex64(store_hex) && is_hex64(root_hex)
}

/// Decide which cached files to evict so total bytes fit under `cap`. LRU:
/// evict oldest (smallest mtime) first, stopping as soon as the remaining total
/// is at/under `cap`. `entries` is (path, mtime, size); returns paths to delete.
fn plan_eviction(entries: &[(PathBuf, std::time::SystemTime, u64)], cap: u64) -> Vec<PathBuf> {
    let total: u64 = entries.iter().map(|(_, _, sz)| *sz).sum();
    if total <= cap {
        return Vec::new();
    }
    let mut sorted: Vec<&(PathBuf, std::time::SystemTime, u64)> = entries.iter().collect();
    sorted.sort_by_key(|(_, t, _)| *t); // oldest first
    let mut running = total;
    let mut victims = Vec::new();
    for (path, _, sz) in sorted {
        if running <= cap {
            break;
        }
        victims.push(path.clone());
        running = running.saturating_sub(*sz);
    }
    victims
}

/// Build the JSON-RPC `result` object for one window of a decoded ContentResponse.
fn build_result(resp: &ContentResponse, offset: usize) -> Value {
    let total = resp.ciphertext.len();
    let start = offset.min(total);
    let end = (start + WINDOW).min(total);
    let window = &resp.ciphertext[start..end];
    let complete = end >= total;

    let mut result = json!({
        "ciphertext": base64::engine::general_purpose::STANDARD.encode(window),
        "root": resp.roothash.to_hex(),
        "complete": complete,
    });
    if !complete {
        result["next_offset"] = json!(end);
    }
    // The proof + chunk_lens are sent on the FIRST window only (the client keeps
    // the first non-empty proof). Match rpc.dig.net / the digstore client.
    if start == 0 {
        result["inclusion_proof"] =
            json!(base64::engine::general_purpose::STANDARD.encode(resp.merkle_proof.to_bytes()));
        result["chunk_lens"] = json!(resp.chunk_lens);
    }
    result
}

impl Node {
    /// Try to serve a request from a locally cached module. Returns the decoded
    /// ContentResponse on a hit, or None on a cache miss.
    fn serve_local(
        &self,
        store_hex: &str,
        root_hex: &str,
        retrieval_key: &[u8; 32],
    ) -> Option<ContentResponse> {
        let path = module_path(&self.cache_dir, store_hex, root_hex);
        let module = std::fs::read(&path).ok()?;
        let store_id = Bytes32::from_hex(store_hex).ok()?;
        // Ephemeral host key: the browser verifies the merkle proof against the
        // chain-anchored root, not a host signature, so the serve key is local-only.
        let cfg = BlindServeConfig::from_seed(store_id, &[0u8; 32]);
        let bytes = serve_blind(&module, retrieval_key, cfg).ok()?;
        let resp = ContentResponse::from_bytes(&bytes).ok()?;
        touch(&path); // LRU recency
        Some(resp)
    }

    fn responses_dir(&self) -> PathBuf {
        self.cache_dir.join("responses")
    }

    /// Return a previously-proxied JSON-RPC `result` for this exact request
    /// window, if cached. Touches the file for LRU recency on a hit.
    fn serve_cached_response(&self, key: &str) -> Option<Value> {
        let path = self.responses_dir().join(key);
        let bytes = std::fs::read(&path).ok()?;
        let v: Value = serde_json::from_slice(&bytes).ok()?;
        touch(&path);
        Some(v)
    }

    /// Persist a proxied `result` window to the response cache, then evict
    /// oldest entries (LRU) until the cache is under its size cap.
    async fn store_response(&self, key: &str, result: &Value) {
        let dir = self.responses_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(result) {
            let _ = std::fs::write(dir.join(key), bytes);
        }
        // Serialize eviction so concurrent writers don't race the size scan.
        let _guard = self.cache_lock.lock().await;
        self.evict_if_needed(&dir);
    }

    /// LRU-evict cached response windows until total bytes fit under the cap.
    fn evict_if_needed(&self, dir: &Path) {
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if let Ok(md) = e.metadata() {
                    let mtime = md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    entries.push((e.path(), mtime, md.len()));
                }
            }
        }
        for victim in plan_eviction(&entries, self.cache_cap) {
            let _ = std::fs::remove_file(victim);
        }
    }

    /// Authenticated whole-store sync (§21.9) against the configured upstream §21
    /// host. Returns `true` when the synced module's served root matches the
    /// requested root, so the caller can now serve the request locally.
    async fn sync_module(&self, store_hex: &str, root_hex: &str) -> bool {
        self.sync_module_from(&self.upstream, store_hex, root_hex)
            .await
    }

    /// Core of [`Node::sync_module`], parameterized by the §21 host base URL (tests
    /// point it at a local mock). It is a no-op (returns `false`) unless an
    /// identity is configured AND the request is sync-eligible. On success it
    /// fetches the WHOLE `.dig` module from `GET /stores/{id}/module` — stamping
    /// the §21.9 `X-Dig-Identity/-Timestamp/-Nonce/-Auth` headers via the loaded
    /// identity seed — and writes it to `module_path(store, served_root)`, so
    /// `serve_local` then serves it (and every other resource in the store)
    /// without further network.
    ///
    /// The synced module is NOT cryptographically trusted here: every response the
    /// node later serves from it carries its merkle proof, which the browser
    /// verifies against the chain-anchored root — a tampered module fails THAT
    /// gate, not this sync. Sync-time verification is therefore a minimal
    /// non-empty check.
    async fn sync_module_from(&self, base_url: &str, store_hex: &str, root_hex: &str) -> bool {
        let Some(seed) = self.identity_seed else {
            return false;
        };
        if !sync_eligible(store_hex, root_hex) {
            return false;
        }
        let (Ok(store_id), Ok(want_root)) =
            (Bytes32::from_hex(store_hex), Bytes32::from_hex(root_hex))
        else {
            return false;
        };

        // Reuse the node's reqwest client; attach a fresh §21.9 identity (the
        // client takes it by value) minted from the in-memory seed.
        let client = DigClient::with_client(base_url, self.http.clone())
            .with_identity(identity::identity_from_seed(seed));
        let verify = |bytes: &[u8], _served: &Bytes32| -> Result<(), String> {
            if bytes.is_empty() {
                Err("empty module".into())
            } else {
                Ok(())
            }
        };
        let (served_root, bytes) = match client.clone_store(&store_id, verify, None).await {
            Ok(v) => v,
            Err(e) => {
                // Best-effort: log WHY (e.g. a §21 401/403 = the identity is not
                // authorized to clone this store) so the silent fallback to the
                // per-resource proxy is diagnosable, then give up on the sync.
                eprintln!("dig-node: §21 whole-store sync for {store_hex} skipped: {e}");
                return false;
            }
        };
        eprintln!(
            "dig-node: §21 whole-store sync for {store_hex} ok — served root {} ({} bytes)",
            served_root.to_hex(),
            bytes.len()
        );

        // Cache under the SERVED root (which may differ from want_root if the
        // remote head advanced between resolve and sync). Best-effort write.
        let path = module_path(&self.cache_dir, store_hex, &served_root.to_hex());
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        if std::fs::write(&path, &bytes).is_err() {
            return false;
        }
        served_root == want_root
    }

    /// Proxy the raw JSON-RPC body to the upstream rpc.dig.net and return its response.
    async fn proxy(&self, body: &Value) -> Result<Value, String> {
        let resp = self
            .http
            .post(&self.upstream)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }

    /// `dig.getAnchoredRoot`: resolve a store's CHIP-0035 chain-anchored TIP root by
    /// walking its DataStore singleton lineage on coinset.org — NEVER from the
    /// serving node (`digstore_chain::singleton::sync_datastore`). This is the
    /// trusted-root source for the browser's mandatory dig:// root pinning: a
    /// rootless `dig://` URN must verify `proof.root == anchored_root` instead of
    /// trusting the rpc-served "latest" root (which a compromised rpc could forge —
    /// the dig:// verifier must never fail open). Returns a JSON-RPC envelope with
    /// `result.root` (64-hex) on success, or a `-32602`/`-32000` error.
    async fn anchored_root(&self, params: &Value, id: Value) -> Value {
        let Ok(store_id) = parse_store_id_arg(params) else {
            return json!({"jsonrpc":"2.0","id":id,"error":{
                "code":-32602,
                "message":"params.store_id must be a 32-byte (64-hex) launcher id"}});
        };
        match sync_datastore(&resolution_coinset(), store_id).await {
            Ok(store) => json!({"jsonrpc":"2.0","id":id,"result":{
                "store_id": hex::encode(store_id),
                "root": hex::encode(store.info.metadata.root_hash)}}),
            Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{
                "code":-32000,
                "message":format!("resolve anchored root: {e}")}}),
        }
    }
}

/// Bump a file's mtime to "now" so the LRU treats it as freshly used.
fn touch(path: &Path) {
    let _ = filetime::set_file_mtime(path, filetime::FileTime::now());
}

/// Coinset client used to resolve chain-anchored roots. `DIG_NODE_COINSET`
/// overrides the API base (tests / alternate endpoints); defaults to mainnet
/// (api.coinset.org).
fn resolution_coinset() -> Coinset {
    match std::env::var("DIG_NODE_COINSET") {
        Ok(url) if !url.is_empty() => Coinset::with_url(url),
        _ => Coinset::mainnet(),
    }
}

/// Parse a `params.store_id` field into a canonical 32-byte (64-hex) launcher id
/// (`chia_protocol::Bytes32`, as `sync_datastore` expects). Returns `Err(())` for a
/// missing, mis-sized, or non-hex value.
fn parse_store_id_arg(params: &Value) -> Result<chia_protocol::Bytes32, ()> {
    let s = params.get("store_id").and_then(|v| v.as_str()).ok_or(())?;
    if s.len() != 64 {
        return Err(());
    }
    let bytes = hex::decode(s).map_err(|_| ())?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| ())?;
    Ok(chia_protocol::Bytes32::new(arr))
}

async fn rpc(State(node): State<Arc<Node>>, Json(req): Json<Value>) -> impl IntoResponse {
    let id = req.get("id").cloned().unwrap_or(json!(1));
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // dig.getAnchoredRoot: resolve a store's chain-anchored tip root (the TRUSTED
    // root for the browser's mandatory dig:// root-pinning — see anchored_root).
    if method == "dig.getAnchoredRoot" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        return Json(node.anchored_root(&params, id).await);
    }
    if method != "dig.getContent" {
        return Json(json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32601,"message":"method not found"}}));
    }
    let params = req.get("params").cloned().unwrap_or(json!({}));
    let store_hex = params
        .get("store_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let root_hex = params.get("root").and_then(|v| v.as_str()).unwrap_or("");
    let rk_hex = params
        .get("retrieval_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    // 1. LOCAL-FIRST: serve from a cached compiled module (no network at all).
    if let (Ok(rk), false) = (decode_rk(rk_hex), root_hex.is_empty()) {
        if let Some(resp) = node.serve_local(store_hex, root_hex, &rk) {
            return Json(json!({"jsonrpc":"2.0","id":id,"result":build_result(&resp, offset)}));
        }
        // 1b. AUTHENTICATED WHOLE-STORE SYNC (§21.9): on a module-cache miss, pull
        //     the whole `.dig` from rpc.dig.net's auth-gated §21 endpoint, cache
        //     it, then serve locally. Best-effort — a failed/disabled sync just
        //     falls through to the per-resource proxy below.
        if node.sync_module(store_hex, root_hex).await {
            if let Some(resp) = node.serve_local(store_hex, root_hex, &rk) {
                return Json(json!({"jsonrpc":"2.0","id":id,"result":build_result(&resp, offset)}));
            }
        }
    }

    // 2. RESPONSE CACHE: a window we previously proxied for this exact request.
    let key = response_key(store_hex, root_hex, rk_hex, offset);
    if let Some(result) = node.serve_cached_response(&key) {
        return Json(json!({"jsonrpc":"2.0","id":id,"result":result}));
    }

    // 3. MISS: proxy to rpc.dig.net, then cache the result window (LRU-capped)
    //    so the next load of this resource is served locally.
    match node.proxy(&req).await {
        Ok(v) => {
            if let Some(result) = v.get("result") {
                node.store_response(&key, result).await;
            }
            Json(v)
        }
        Err(e) => Json(json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32000,"message":format!("upstream: {e}")}})),
    }
}

fn decode_rk(hex_str: &str) -> Result<[u8; 32], ()> {
    let v = hex::decode(hex_str).map_err(|_| ())?;
    if v.len() != 32 {
        return Err(());
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    Ok(a)
}

#[tokio::main]
async fn main() {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let cap = std::env::var("DIG_NODE_CACHE_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CACHE_CAP);
    // Load the persistent §21.9 identity (best-effort). Present → authenticated
    // whole-store sync is enabled; absent → the node still serves local modules
    // and proxies per-resource.
    let identity_seed = match identity::load_or_create_seed() {
        Ok((seed, pk)) => {
            println!(
                "dig-node identity {} (authenticated §21 whole-store sync enabled)",
                pk.to_hex()
            );
            Some(seed)
        }
        Err(e) => {
            eprintln!("dig-node: no identity key ({e}); authenticated §21 sync disabled");
            None
        }
    };
    let node = Arc::new(Node {
        cache_dir: dir.clone(),
        cache_cap: cap,
        http: reqwest::Client::builder()
            .user_agent("dig-node/0.1")
            .build()
            .expect("http client"),
        upstream: std::env::var("DIG_NODE_UPSTREAM").unwrap_or_else(|_| RPC_FALLBACK.to_string()),
        cache_lock: Mutex::new(()),
        identity_seed,
    });

    let app = Router::new().route("/", post(rpc)).with_state(node);

    let port: u16 = std::env::var("DIG_NODE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9778);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("dig-node: cannot bind {addr}: {e}"));
    println!(
        "dig-node listening on http://{addr} (cache {} MiB cap at {})",
        cap / (1024 * 1024),
        dir.display()
    );
    axum::serve(listener, app).await.expect("dig-node server");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn response_key_is_stable_and_safe() {
        let k = response_key("aa", "bb", "cc", 0);
        assert_eq!(k, "aa_bb_cc_0.json");
        // Different offset → different file (so windows don't collide).
        assert_ne!(k, response_key("aa", "bb", "cc", 100));
        // Non-hex input is neutralized (no path traversal in the filename).
        let bad = response_key("../../etc", "bb", "cc", 0);
        assert!(!bad.contains('/'));
        assert!(!bad.contains(".."));
    }

    #[test]
    fn evicts_nothing_when_under_cap() {
        let t = UNIX_EPOCH + Duration::from_secs(10);
        let entries = vec![(PathBuf::from("a"), t, 100), (PathBuf::from("b"), t, 100)];
        assert!(plan_eviction(&entries, 1000).is_empty());
    }

    #[test]
    fn evicts_oldest_first_until_under_cap() {
        let old = UNIX_EPOCH + Duration::from_secs(1);
        let mid = UNIX_EPOCH + Duration::from_secs(2);
        let new = UNIX_EPOCH + Duration::from_secs(3);
        // total 300, cap 150 → must drop 'old' (100) and 'mid' (100) → 100 left.
        let entries = vec![
            (PathBuf::from("new"), new, 100),
            (PathBuf::from("old"), old, 100),
            (PathBuf::from("mid"), mid, 100),
        ];
        let victims = plan_eviction(&entries, 150);
        assert_eq!(victims, vec![PathBuf::from("old"), PathBuf::from("mid")]);
    }

    #[test]
    fn stops_as_soon_as_under_cap() {
        let old = UNIX_EPOCH + Duration::from_secs(1);
        let new = UNIX_EPOCH + Duration::from_secs(2);
        // total 300, cap 250 → dropping just 'old' (100) leaves 200 ≤ 250.
        let entries = vec![
            (PathBuf::from("old"), old, 100),
            (PathBuf::from("new"), new, 200),
        ];
        assert_eq!(plan_eviction(&entries, 250), vec![PathBuf::from("old")]);
    }

    // -- Authenticated whole-store sync (§21.9) --------------------------------

    #[test]
    fn sync_eligible_requires_concrete_store_and_root() {
        let h = "ab".repeat(32); // 64 hex
        assert!(sync_eligible(&h, &h));
        assert!(!sync_eligible(&h, "")); // rootless
        assert!(!sync_eligible(&h, "latest")); // sentinel, not a concrete root
        assert!(!sync_eligible("", &h)); // no store id
        assert!(!sync_eligible(&h, &"zz".repeat(32))); // right length, non-hex
        assert!(!sync_eligible(&h, &"ab".repeat(31))); // too short
    }

    /// Build a `Node` with a throwaway cache dir and an optional identity seed. The
    /// returned `TempDir` must be kept alive for the duration of the test.
    fn test_node(identity_seed: Option<[u8; 32]>) -> (Node, tempfile::TempDir) {
        let td = tempfile::tempdir().unwrap();
        let node = Node {
            cache_dir: td.path().to_path_buf(),
            cache_cap: DEFAULT_CACHE_CAP,
            http: reqwest::Client::new(),
            upstream: RPC_FALLBACK.to_string(),
            cache_lock: Mutex::new(()),
            identity_seed,
        };
        (node, td)
    }

    /// Spawn the REAL §21 `RemoteServer` (auth REQUIRED by default) over an
    /// in-memory backend seeded with one store whose module is `module` at root
    /// 0x10. Returns `(base_url, store_id_hex)`. Unlike the header-recording mock
    /// below, this exercises the actual §21.9 auth middleware end-to-end.
    async fn spawn_authed_remote(module: Vec<u8>) -> (String, String) {
        use digstore_core::Bytes48;
        use digstore_remote::{InMemoryBackend, RemoteServer};
        let be = Arc::new(InMemoryBackend::new());
        let store_id = Bytes32([1u8; 32]);
        be.add_store(
            store_id,
            Bytes48([2u8; 48]),
            Bytes32([0x10; 32]),
            module,
            None,
        );
        let app = RemoteServer::new(be).router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), store_id.to_hex())
    }

    #[tokio::test]
    async fn authed_identity_syncs_module_from_authed_remote() {
        // The native §21.9 identity is admitted by an auth-REQUIRED §21 server, the
        // whole module is synced, and it lands in the on-disk cache for local-first.
        let module = b"compiled-module-bytes".to_vec();
        let (base, store_hex) = spawn_authed_remote(module.clone()).await;
        let (node, _td) = test_node(Some([5u8; 32]));
        let root_hex = "10".repeat(32); // served genesis root
        let matched = node.sync_module_from(&base, &store_hex, &root_hex).await;
        assert!(matched, "authed sync to served root 0x10 should match");
        let cached = std::fs::read(module_path(&node.cache_dir, &store_hex, &root_hex)).unwrap();
        assert_eq!(cached, module, "served module must be cached locally");
    }

    #[tokio::test]
    async fn anonymous_request_rejected_by_authed_remote() {
        // Prove the auth gate is real (not an open server) — so the test above is
        // meaningful: a client carrying NO §21.9 identity is rejected.
        let (base, store_hex) = spawn_authed_remote(b"m".to_vec()).await;
        let store_id = Bytes32::from_hex(&store_hex).unwrap();
        let anon = DigClient::new(base);
        let r = anon.clone_store(&store_id, |_b, _r| Ok(()), None).await;
        assert!(
            r.is_err(),
            "anonymous clone must be rejected by the auth-required remote"
        );
    }

    /// Spawn a mock §21 host serving `GET /stores/:id/module`: it records the
    /// request headers into `captured` and replies 200 with `body` + an ETag of
    /// `root` (the wire form `clone_store` expects). Returns the base URL.
    async fn spawn_mock_module_server(
        captured: Arc<std::sync::Mutex<Option<axum::http::HeaderMap>>>,
        root: Bytes32,
        body: Vec<u8>,
    ) -> String {
        use axum::body::Body;
        use axum::http::{header, HeaderMap};
        use axum::response::Response;
        use axum::routing::get;

        let handler = move |headers: HeaderMap| {
            let captured = captured.clone();
            let body = body.clone();
            async move {
                *captured.lock().unwrap() = Some(headers);
                Response::builder()
                    .header(header::ETAG, digstore_remote::etag::etag_for_root(&root))
                    .body(Body::from(body))
                    .unwrap()
            }
        };
        let app = Router::new().route("/stores/:id/module", get(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn authed_module_sync_carries_verifiable_identity() {
        let seed = [7u8; 32];
        let store = Bytes32([3u8; 32]);
        let root = Bytes32([9u8; 32]);
        let captured = Arc::new(std::sync::Mutex::new(None));
        let url = spawn_mock_module_server(captured.clone(), root, b"MODULE".to_vec()).await;

        let (node, _td) = test_node(Some(seed));
        let matched = node
            .sync_module_from(&url, &store.to_hex(), &root.to_hex())
            .await;
        assert!(matched, "served root == requested root");

        let headers = captured
            .lock()
            .unwrap()
            .take()
            .expect("server saw a request");
        let id_hex = headers.get("x-dig-identity").unwrap().to_str().unwrap();
        let ts: u64 = headers
            .get("x-dig-timestamp")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let nonce_hex = headers.get("x-dig-nonce").unwrap().to_str().unwrap();
        let auth_hex = headers.get("x-dig-auth").unwrap().to_str().unwrap();

        // The identity must be exactly the one derived from our seed.
        assert_eq!(id_hex, identity::identity_from_seed(seed).pubkey_hex);

        // And the signature must verify for method "module" over (store, ts, nonce),
        // so a §21 remote will accept it (and it can't be replayed as another op).
        let pk = digstore_crypto::bls::PublicKey::from_bytes(
            &digstore_core::Bytes48::from_hex(id_hex).unwrap(),
        )
        .unwrap();
        let mut nonce = [0u8; 32];
        hex::decode_to_slice(nonce_hex, &mut nonce).unwrap();
        let sig = digstore_core::Bytes96(
            <[u8; 96]>::try_from(hex::decode(auth_hex).unwrap().as_slice()).unwrap(),
        );
        assert!(digstore_crypto::verify_request(
            &pk, "module", &store, ts, &nonce, &sig
        ));
    }

    #[tokio::test]
    async fn sync_caches_module_under_served_root_and_reports_mismatch() {
        let seed = [1u8; 32];
        let store = Bytes32([2u8; 32]);
        let served = Bytes32([0xAA; 32]);
        let requested = Bytes32([0xBB; 32]); // differs from served
        let captured = Arc::new(std::sync::Mutex::new(None));
        let url = spawn_mock_module_server(captured, served, b"DIGMODULE".to_vec()).await;

        let (node, _td) = test_node(Some(seed));
        let matched = node
            .sync_module_from(&url, &store.to_hex(), &requested.to_hex())
            .await;
        assert!(!matched, "served (AA..) != requested (BB..)");

        // The module is cached under the SERVED root with the served bytes …
        let served_path = module_path(&node.cache_dir, &store.to_hex(), &served.to_hex());
        assert_eq!(std::fs::read(&served_path).unwrap(), b"DIGMODULE");
        // … and nothing is cached under the (unmatched) requested root.
        assert!(!module_path(&node.cache_dir, &store.to_hex(), &requested.to_hex()).exists());
    }

    // -- Anchored-root resolution (dig.getAnchoredRoot) ------------------------

    #[test]
    fn parse_store_id_arg_accepts_only_canonical_launcher_ids() {
        let ok = json!({ "store_id": "ab".repeat(32) });
        assert!(parse_store_id_arg(&ok).is_ok());
        assert!(parse_store_id_arg(&json!({})).is_err()); // missing
        assert!(parse_store_id_arg(&json!({ "store_id": "ab".repeat(31) })).is_err()); // short
        assert!(parse_store_id_arg(&json!({ "store_id": "zz".repeat(32) })).is_err()); // non-hex
        assert!(parse_store_id_arg(&json!({ "store_id": 123 })).is_err()); // wrong type
    }

    #[tokio::test]
    async fn anchored_root_rejects_bad_store_id_without_touching_chain() {
        // A malformed store_id is rejected with a JSON-RPC -32602 BEFORE any chain
        // read, so the trusted-root endpoint validates input up front.
        let (node, _td) = test_node(None);
        let resp = node
            .anchored_root(&json!({ "store_id": "nope" }), json!(7))
            .await;
        assert_eq!(resp["id"], json!(7));
        assert_eq!(resp["error"]["code"], json!(-32602));
        assert!(resp.get("result").is_none());
    }

    #[tokio::test]
    async fn sync_skipped_without_identity_makes_no_request() {
        let (node, _td) = test_node(None);
        let store = Bytes32([2u8; 32]);
        let root = Bytes32([3u8; 32]);
        // No identity → must short-circuit to false WITHOUT touching the network
        // (the URL is intentionally unroutable; the call returns immediately).
        let matched = node
            .sync_module_from("http://127.0.0.1:1", &store.to_hex(), &root.to_hex())
            .await;
        assert!(!matched);
        assert!(!module_path(&node.cache_dir, &store.to_hex(), &root.to_hex()).exists());
    }
}
