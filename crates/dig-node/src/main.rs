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
//! module is keyed by (store_id, root). Whole-store sync that POPULATES this
//! cache from rpc.dig.net needs the §21 module endpoint, which is dighub-auth
//! gated — that sync is a follow-up; this node serves whatever modules are
//! present (e.g. the user's own digstore stores) and proxies the rest.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use base64::Engine;
use digstore_core::codec::{Decode, Encode};
use digstore_core::wire::ContentResponse;
use digstore_core::Bytes32;
use digstore_host::{serve_blind, BlindServeConfig};
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
    /// Serialize cache mutation (eviction) so concurrent requests don't race.
    cache_lock: Mutex<()>,
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

    /// Proxy the raw JSON-RPC body to rpc.dig.net and return its response.
    async fn proxy(&self, body: &Value) -> Result<Value, String> {
        let resp = self
            .http
            .post(RPC_FALLBACK)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }
}

/// Bump a file's mtime to "now" so the LRU treats it as freshly used.
fn touch(path: &Path) {
    let _ = filetime::set_file_mtime(path, filetime::FileTime::now());
}

async fn rpc(State(node): State<Arc<Node>>, Json(req): Json<Value>) -> impl IntoResponse {
    let id = req.get("id").cloned().unwrap_or(json!(1));
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
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
    let node = Arc::new(Node {
        cache_dir: dir.clone(),
        cache_cap: cap,
        http: reqwest::Client::builder()
            .user_agent("dig-node/0.1")
            .build()
            .expect("http client"),
        cache_lock: Mutex::new(()),
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
}
