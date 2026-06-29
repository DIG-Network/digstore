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
use std::sync::atomic::{AtomicBool, Ordering};
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
use fs4::FileExt;
use serde_json::{json, Value};
use tokio::sync::Mutex;

const RPC_FALLBACK: &str = "https://rpc.dig.net/";
/// Per-window ciphertext cap (bytes) when paging the JSON-RPC response.
const WINDOW: usize = 3 * 1024 * 1024;
/// Default LRU cap for the on-disk module cache.
const DEFAULT_CACHE_CAP: u64 = 1024 * 1024 * 1024; // 1 GiB

/// The DIG node state. Public so `dig-runtime` can construct one ([`Node::from_env`])
/// and drive it via [`handle_rpc`] in-process inside the browser. Fields stay
/// private — callers only need the constructor + the dispatch.
pub struct Node {
    cache_dir: PathBuf,
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

/// The CANONICAL (shared) cache dir — the one the DIG Browser's in-process
/// dig-node AND the standalone dig-node/dig-companion both resolve to, so they
/// share a `.dig` cache by construction (#96). Precedence:
///
/// 1. `DIG_NODE_CACHE` env override (the installer points both the browser launch
///    env and the standalone service at one dir) — UNCHANGED.
/// 2. Otherwise the per-OS base dir resolved via the `directories` crate (correct
///    on Windows/macOS/Linux even when the raw env vars are unset), suffixed
///    `DigNode/cache`.
/// 3. As a last resort (no home dir resolvable) `./DigNode/cache`.
///
/// To stay byte-identical to dig-companion's `cache_dir()` (so the two keep
/// sharing), Windows uses `data_local_dir()` (= `%LOCALAPPDATA%`) and Unix/macOS
/// use `home_dir()` + `DigNode/cache` — NOT XDG / `Application Support`.
///
/// This is the *intended* shared location; whether it is actually writable (and
/// thus used) is decided by [`resolve_cache_dir`].
fn canonical_cache_dir() -> PathBuf {
    if let Some(env) = std::env::var("DIG_NODE_CACHE")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return PathBuf::from(env);
    }
    let base = directories::BaseDirs::new().map(|b| {
        if cfg!(windows) {
            b.data_local_dir().to_path_buf()
        } else {
            // Preserve the historic `$HOME/DigNode/cache` default on Unix/macOS
            // so the path is byte-identical to dig-companion (shared cache).
            b.home_dir().to_path_buf()
        }
    });
    let root = base
        .or_else(|| std::env::var("LOCALAPPDATA").ok().map(PathBuf::from))
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    root.join("DigNode").join("cache")
}

/// A deterministic process-private fallback cache dir, used only when the
/// canonical shared dir is unwritable. Keyed by PID so it is stable for the
/// process lifetime (every call returns the same path) but isolated from other
/// processes — a degraded, un-shared mode that never fails the node.
fn private_fallback_dir() -> PathBuf {
    std::env::temp_dir()
        .join(format!("DigNode-{}", std::process::id()))
        .join("cache")
}

/// Has the unwritable-canonical-dir warning already been logged this process?
/// (So the structured fallback warning is emitted once, not on every resolve.)
static FALLBACK_WARNED: AtomicBool = AtomicBool::new(false);

/// Is the canonical cache dir writable? Probes by ensuring the dir exists and
/// writing+removing a tiny temp file in it. A miss (read-only volume, perms)
/// means we must fall back to a private dir.
fn dir_is_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".write-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Resolve the EFFECTIVE cache dir and whether it is the canonical shared one.
/// Returns `(dir, shared)`: the canonical dir with `shared = true` when it is
/// writable, else the process-private fallback with `shared = false` (logging a
/// structured one-shot warning). Re-resolved on each call (so a `DIG_NODE_CACHE`
/// change or a settings-driven path takes effect without a restart) — the
/// fallback path is deterministic, so all callers within a process agree.
fn resolve_cache_dir() -> (PathBuf, bool) {
    let canonical = canonical_cache_dir();
    if dir_is_writable(&canonical) {
        return (canonical, true);
    }
    let fallback = private_fallback_dir();
    if !FALLBACK_WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "dig-node: WARN canonical cache dir {} is not writable; \
             falling back to a process-private dir {} (cache NOT shared with \
             other DIG processes this session)",
            canonical.display(),
            fallback.display()
        );
    }
    let _ = std::fs::create_dir_all(&fallback);
    (fallback, false)
}

/// The effective cache dir (canonical shared dir if writable, else a private
/// fallback). See [`resolve_cache_dir`].
fn cache_dir() -> PathBuf {
    resolve_cache_dir().0
}

/// Whether the effective [`cache_dir`] is the canonical dir shared with the
/// standalone dig-node / dig-companion (`true`), or a process-private fallback
/// because the canonical dir was unwritable (`false`). Surfaced additively in
/// `cache.getConfig`.
pub fn cache_dir_is_shared() -> bool {
    resolve_cache_dir().1
}

/// Path to the shared DIG node config (cache cap, etc.) — next to the cache dir.
pub fn config_path() -> PathBuf {
    let dir = cache_dir();
    dir.parent()
        .map(|p| p.join("config.json"))
        .unwrap_or_else(|| dir.join("config.json"))
}

/// Name of the cross-process advisory lockfile, kept at the ROOT of the cache
/// dir (next to `modules/`, `responses/`, and `config.json`). One lockfile
/// coordinates BOTH the config read-modify-write and cache eviction across every
/// DIG process sharing this cache (the in-process browser node, the standalone
/// dig-node, dig-companion).
const LOCKFILE_NAME: &str = ".dignode.lock";

/// Path to the cross-process lockfile for the effective cache dir.
fn lockfile_path() -> PathBuf {
    cache_dir().join(LOCKFILE_NAME)
}

/// A held cross-process advisory lock. Dropping it (or the process exiting)
/// releases the OS-level `flock`. The inner `File` is kept alive solely to hold
/// the lock — it is never read or written.
struct CacheLockGuard {
    _file: std::fs::File,
}

/// Acquire the cross-process advisory lock on `<cache>/.dignode.lock`, blocking
/// briefly until it is free. Best-effort: if the lockfile can't be created or
/// locked (e.g. a filesystem without `flock`), returns `None` and the caller
/// proceeds WITHOUT the cross-process guarantee rather than failing — the
/// in-process mutex + atomic writes still hold, so this only degrades the
/// two-process lost-update protection, it never breaks single-process use.
fn acquire_cache_lock() -> Option<CacheLockGuard> {
    let path = lockfile_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    // Blocking exclusive lock — config RMW and eviction are short, and two DIG
    // processes contending here is rare, so blocking (vs. spin) is fine. Use
    // fs4's portable advisory lock explicitly (fully-qualified so it's the fs4
    // implementation, not std's inherent `File::lock`) so the behaviour is the
    // same flock/LockFileEx across the toolchains CI runs.
    FileExt::lock(&file).ok()?;
    Some(CacheLockGuard { _file: file })
}

/// Read-modify-write the config JSON under the cross-process lock so two
/// processes can't lose each other's update (the lost-update race). Reads the
/// current config, applies `mutate`, and writes it back atomically (temp +
/// rename) — all while holding `<cache>/.dignode.lock`. Pretty-prints to keep
/// the on-disk `config.json` schema byte-compatible with the prior writer.
fn update_config_locked(mutate: impl FnOnce(&mut Value)) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Hold the cross-process lock across the read AND the write so a concurrent
    // process can't read-then-clobber between our read and our write.
    let _lock = acquire_cache_lock();
    let mut v: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    mutate(&mut v);
    let bytes = serde_json::to_vec_pretty(&v).unwrap_or_default();
    write_atomic(&path, &bytes)
}

/// Atomically write `bytes` to `path` via a temp file in the SAME directory +
/// `fs::rename` (atomic on NTFS and POSIX). A reader (this or another process)
/// therefore never observes a torn/partial file — it sees either the old
/// contents or the fully-written new ones. Used for content-addressed module
/// bytes (immutable per capsule, so concurrent writers converge) and for the
/// config read-modify-write.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    // Unique temp name in the same dir so `rename` stays within one filesystem
    // (cross-device rename would fail). PID + nanos + a per-process monotonic
    // counter keeps concurrent writers (even on a coarse clock) from colliding
    // on the temp path.
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".tmp-{}-{}-{}", std::process::id(), nanos, seq));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Clean up the temp file on a failed rename so we don't leak it.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// The local-cache size cap in bytes. Read from config.json (set via the DIG
/// settings page), falling back to `DIG_NODE_CACHE_CAP`, then the 1 GiB default.
/// Read dynamically so a settings change takes effect without a restart.
pub fn cache_cap_bytes() -> u64 {
    if let Ok(txt) = std::fs::read_to_string(config_path()) {
        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
            if let Some(cap) = v.get("cache_cap_bytes").and_then(|c| c.as_u64()) {
                if cap > 0 {
                    return cap;
                }
            }
        }
    }
    std::env::var("DIG_NODE_CACHE_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CACHE_CAP)
}

/// Persist the cache size cap (bytes) to config.json (the DIG settings page).
/// Read-modify-write under the cross-process lock so a concurrent writer (e.g.
/// dig-companion setting `wc_project_id`) can't lose this update or vice-versa.
pub fn set_cache_cap_bytes(cap: u64) -> std::io::Result<()> {
    update_config_locked(|v| {
        v["cache_cap_bytes"] = json!(cap);
    })
}

/// Total bytes currently held in the local cache (modules + response windows).
pub fn cache_used_bytes() -> u64 {
    fn walk(p: &Path, total: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, total);
                } else if let Ok(md) = e.metadata() {
                    *total += md.len();
                }
            }
        }
    }
    let mut total = 0u64;
    walk(&cache_dir(), &mut total);
    total
}

/// Delete all locally cached DIG content (the settings "clear cache" action).
pub fn clear_cache() {
    let _ = std::fs::remove_dir_all(cache_dir());
}

/// The config key for the WalletConnect projectId (the native wallet acts as a
/// WC responder; the relay needs a Reown/WalletConnect Cloud projectId).
const WC_PROJECT_ID_KEY: &str = "wc_project_id";

/// Resolve the effective WalletConnect projectId from the two sources, in
/// precedence order: a persisted config value wins; otherwise the
/// `DIG_WALLET_WC_PROJECT_ID` env var is the initial/default; otherwise none.
///
/// Pure (no disk/env) so the precedence policy is unit-tested directly. A blank
/// persisted value is treated as "unset" so it falls through to the env default
/// rather than pinning an empty id.
fn resolve_wc_project_id(persisted: Option<&str>, env: Option<&str>) -> Option<String> {
    let clean = |s: &str| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    };
    persisted.and_then(clean).or_else(|| env.and_then(clean))
}

/// The projectId persisted in config.json, if any (blank → `None`).
fn persisted_wc_project_id() -> Option<String> {
    let txt = std::fs::read_to_string(config_path()).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    v.get(WC_PROJECT_ID_KEY)
        .and_then(|p| p.as_str())
        .map(str::to_string)
}

/// The effective WalletConnect projectId: persisted config value if set, else the
/// `DIG_WALLET_WC_PROJECT_ID` env var, else `None`. Read dynamically so a settings
/// change applies without restarting the browser.
pub fn wc_project_id() -> Option<String> {
    let persisted = persisted_wc_project_id();
    let env = std::env::var("DIG_WALLET_WC_PROJECT_ID").ok();
    resolve_wc_project_id(persisted.as_deref(), env.as_deref())
}

/// Persist the WalletConnect projectId to config.json (the DIG settings page).
/// A blank value clears the persisted override (falling back to the env default).
/// Read-modify-write under the cross-process lock so a concurrent writer (e.g.
/// the cache-cap setter) can't lose this update or vice-versa.
pub fn set_wc_project_id(id: &str) -> std::io::Result<()> {
    let trimmed = id.trim().to_string();
    update_config_locked(|v| {
        if trimmed.is_empty() {
            if let Some(obj) = v.as_object_mut() {
                obj.remove(WC_PROJECT_ID_KEY);
            }
        } else {
            v[WC_PROJECT_ID_KEY] = json!(trimmed);
        }
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

/// Recursively read every file under `root` into `(resource_key, bytes)`, where
/// the key is the file path relative to `root`, FORWARD-SLASHED — the exact key
/// convention the CLI `add` walk uses (`ops::walk::key_for`), so the same folder
/// produces the same capsule root through the CLI and the in-process node.
/// Sorted by key for deterministic staging order. Used by the `dig.stage` RPC
/// (#95 Pass C); a symlink loop or unreadable entry is skipped best-effort.
fn walk_dir_files(root: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    fn rec(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                rec(base, &path, out)?;
            } else if ft.is_file() {
                // Key = path relative to base, forward-slashed (URN-safe).
                let rel = path.strip_prefix(base).unwrap_or(&path);
                let key = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let bytes = std::fs::read(&path)?;
                out.push((key, bytes));
            }
            // Symlinks / other types are skipped (not staged).
        }
        Ok(())
    }
    let mut out = Vec::new();
    rec(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
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
    ///
    /// Held under the cross-process lock for the whole scan→plan→delete so two
    /// DIG processes sharing the cache can't both scan the same set and
    /// double-evict (or race a concurrent write into a torn size accounting).
    /// The in-process `cache_lock` (held by the caller) serializes this process's
    /// own writers; the file lock serializes across processes.
    fn evict_if_needed(&self, dir: &Path) {
        let _xproc = acquire_cache_lock();
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if let Ok(md) = e.metadata() {
                    let mtime = md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    entries.push((e.path(), mtime, md.len()));
                }
            }
        }
        // Read the cap dynamically so changes from the DIG settings page apply
        // without restarting the browser. `self.cache_cap` is the startup default.
        let cap = cache_cap_bytes();
        for victim in plan_eviction(&entries, cap) {
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
        // remote head advanced between resolve and sync). Best-effort.
        //
        // ATOMIC + CONTENT-ADDRESSED: a module is keyed by capsule
        // (storeId:rootHash) and its bytes are immutable, so two writers (the
        // browser's in-process node + the standalone node sharing this cache)
        // produce identical bytes. `write_atomic` (temp + rename) guarantees a
        // reader never observes a torn/partial file and that the two writers
        // converge on the same final file.
        let path = module_path(&self.cache_dir, store_hex, &served_root.to_hex());
        if write_atomic(&path, &bytes).is_err() {
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

    /// dig.stage (#95 Pass C): turn a local folder into a CAPSULE (`.dig` module)
    /// in process — the staging/compile half of a local deploy.
    ///
    /// This drives the SHARED stage→compile engine ([`digstore_stage`]) the CLI
    /// `commit`/`compile` use, so the produced module + root are byte-identical to
    /// a CLI build of the same files. It is build-only: NO wallet, NO chain, NO
    /// §21 push. The browser then signs the on-chain root advance with the Pass B
    /// `chia_advanceStore` wallet method and §21-pushes `module_path`.
    ///
    /// Params:
    /// - `dir` (required): absolute path to the folder to publish.
    /// - `store_id` (optional 64-hex): the EXISTING store's launcher id this
    ///   capsule advances. Absent ⇒ an EPHEMERAL, content-derived store id
    ///   (`sha256(fresh host pubkey)`, like `digstore init`) — a preview capsule
    ///   that NEVER advances or impersonates a real store (`ephemeral:true`).
    /// - `salt` (optional 64-hex): present ⇒ a PRIVATE store (retrieval keys are
    ///   derived from `urn + salt`); absent ⇒ public.
    /// - `metadata` (optional): the dighub `Manifest` JSON embedded in the module.
    ///
    /// Result `{capsule, store_id, root, module_path, size, content_address,
    /// files, ephemeral}`. Catalogued errors: `-32602` invalid params,
    /// `-32011` dir not a readable directory, `-32012` no files staged,
    /// `-32013` over the store size cap, `-32014` compile/IO failure.
    fn stage(&self, params: &Value, id: Value) -> Value {
        let err = |code: i64, msg: String| -> Value {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})
        };

        // 1. The folder to publish (required, must be a readable directory).
        let Some(dir) = params
            .get("dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            return err(
                -32602,
                "params.dir is required (absolute folder path)".into(),
            );
        };
        let dir = std::path::PathBuf::from(dir);
        if !dir.is_dir() {
            return err(-32011, format!("not a directory: {}", dir.display()));
        }

        // 2. Optional store id (advance an EXISTING store) or ephemeral preview id.
        let store_id_arg = match params.get("store_id").and_then(|v| v.as_str()) {
            Some(h) if !h.is_empty() => match Bytes32::from_hex(h.trim_start_matches("0x")) {
                Ok(b) => Some(b),
                Err(_) => return err(-32602, "params.store_id must be 64-hex".into()),
            },
            _ => None,
        };

        // 3. Optional secret salt ⇒ a private store.
        let visibility = match params.get("salt").and_then(|v| v.as_str()) {
            Some(h) if !h.is_empty() => match Bytes32::from_hex(h.trim_start_matches("0x")) {
                Ok(b) => digstore_core::Visibility::Private(digstore_core::SecretSalt(b.0)),
                Err(_) => return err(-32602, "params.salt must be 64-hex".into()),
            },
            _ => digstore_core::Visibility::Public,
        };

        // 4. Fresh host BLS identity for the compiled module's trusted/serving key
        //    (mirrors `digstore init`: a content-authoring key, persisted nowhere
        //    here — the browser's wallet signs the on-chain advance, and the §21
        //    push is authenticated by the node's own §21 identity, not this key).
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("OS CSPRNG must be available for the stage key");
        let host_pubkey = digstore_crypto::bls::SecretKey::from_seed(&seed)
            .public_key()
            .to_bytes();

        // Ephemeral store id is content-derived (= `sha256(host pubkey)`, exactly
        // like `init_store`); a supplied store_id is used verbatim.
        let ephemeral = store_id_arg.is_none();
        let store_id = store_id_arg.unwrap_or_else(|| digstore_crypto::sha256(&host_pubkey.0));

        // 5. Walk the folder into (resource_key, bytes), keys relative to `dir`.
        let files = match walk_dir_files(&dir) {
            Ok(f) => f,
            Err(e) => return err(-32011, format!("read folder {}: {e}", dir.display())),
        };
        if files.is_empty() {
            return err(-32012, format!("no files to stage under {}", dir.display()));
        }

        // 6. Optional metadata manifest (the dighub `Manifest` JSON); else empty.
        //    Reuses the SHARED parser the CLI `compile` uses (no fork).
        let metadata = match params.get("metadata") {
            Some(v) if !v.is_null() => digstore_stage::manifest_from_json(v),
            _ => digstore_stage::empty_manifest(),
        };

        // 7. Scratch data dir under the cache: `<cache>/staging/<store>-<pid>-<ns>`.
        //    The compiled module lands in `<scratch>/modules/`; the browser §21-pushes it.
        let scratch = self.cache_dir.join("staging").join(format!(
            "{}-{}-{}",
            store_id.to_hex(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        let opts = digstore_stage::FinalizeOptions {
            data_dir: scratch,
            trusted_keys: vec![digstore_core::TrustedHostKey {
                public_key: host_pubkey.0,
                label: format!("dig-host-key-v1:{}", host_pubkey.to_hex()),
            }],
            store_pubkey: host_pubkey,
            metadata,
            chain_state: None,
            auth: digstore_stage::no_auth(),
        };

        // 8. Stage → compile (generation 0; the browser advances the on-chain root).
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let compiled = match digstore_stage::stage_and_compile(
            &files,
            store_id,
            &visibility,
            digstore_core::MAX_STORE_BYTES,
            false,
            0,
            timestamp,
            &opts,
        ) {
            Ok(c) => c,
            Err(digstore_stage::StageError::EmptyStaging) => {
                return err(-32012, format!("no files to stage under {}", dir.display()))
            }
            Err(e @ digstore_stage::StageError::OverCap { .. }) => {
                return err(-32013, e.to_string())
            }
            Err(e) => return err(-32014, format!("stage/compile failed: {e}")),
        };

        let root_hex = compiled.root.to_hex();
        let store_hex = store_id.to_hex();
        json!({"jsonrpc":"2.0","id":id,"result":{
            // The canonical capsule identity (storeId:rootHash) — the unit the
            // browser advances on-chain + §21-pushes.
            "capsule": format!("{store_hex}:{root_hex}"),
            "store_id": store_hex,
            "root": root_hex,
            "module_path": compiled.module_path.display().to_string(),
            "size": compiled.size,
            // The dig:// content address for this capsule (matches deploy --preview).
            "content_address": format!("dig://{store_hex}:{root_hex}/"),
            "files": compiled.files(),
            // true ⇒ a preview capsule with a content-derived id (NOT a real store).
            "ephemeral": ephemeral,
        }})
    }

    // -- Cached-store management (the DIG-settings cache manager, task #32) -----
    //
    // Every cached module is one CAPSULE — the canonical `(store_id, root_hash)`
    // identity (`digstore_core::Capsule`, rendered `storeId:rootHash`). The
    // on-disk cache key IS that capsule: each module lives at
    // `module_path(store_hex, root_hex)` = `<cache>/modules/<storeId>/<root>.module`,
    // so listing/removing/fetching are all keyed by capsule identity.

    /// List every cached capsule (`storeId:rootHash`) with its on-disk size and
    /// last-used time. Walks `<cache>/modules/<storeId_hex>/<root_hex>.module`
    /// (the same layout `module_path`/`serve_local`/`sync_module_from` use),
    /// reusing the directory-enumerate pattern from [`cache_used_bytes`] and
    /// [`Node::evict_if_needed`]. `last_used_unix_ms` is the file mtime (the LRU
    /// recency stamp bumped by [`touch`] on every local serve), in Unix epoch ms.
    pub async fn cache_list_cached(&self) -> Vec<CachedCapsule> {
        let modules_root = self.cache_dir.join("modules");
        let mut out = Vec::new();
        // Outer level: one directory per store id (hex). Inner: `<root>.module`.
        let Ok(stores) = std::fs::read_dir(&modules_root) else {
            return out; // no modules cached yet
        };
        for store_entry in stores.flatten() {
            if !store_entry.path().is_dir() {
                continue;
            }
            let Some(store_hex) = store_entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Ok(modules) = std::fs::read_dir(store_entry.path()) else {
                continue;
            };
            for m in modules.flatten() {
                let path = m.path();
                // A capsule module is `<root_hex>.module`; skip anything else.
                let Some(root_hex) = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .and_then(|f| f.strip_suffix(".module"))
                    .map(str::to_string)
                else {
                    continue;
                };
                let Ok(md) = m.metadata() else { continue };
                let last_used_unix_ms = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                out.push(CachedCapsule {
                    store_id: store_hex.clone(),
                    root: root_hex,
                    size_bytes: md.len(),
                    last_used_unix_ms,
                });
            }
        }
        out
    }

    /// Remove one cached capsule's module by `(store_id_hex, root_hex)`. Returns
    /// `Ok(true)` if a module was unlinked, `Ok(false)` if it was already absent
    /// (idempotent), or `Err` for invalid input.
    ///
    /// PATH-TRAVERSAL DEFENSE: the hex inputs are validated 64-hex (mirroring the
    /// `response_key`/`sync_eligible` sanitization), then the resolved path is
    /// canonicalized and asserted to live UNDER the cache dir before any unlink —
    /// so a crafted `store_id`/`root` can never delete a file outside the cache.
    /// Holds the existing `cache_lock` for the unlink so it can't race eviction.
    /// (Async because that lock is a `tokio::sync::Mutex`, acquired with `.await`.)
    pub async fn cache_remove_cached(
        &self,
        store_id_hex: &str,
        root_hex: &str,
    ) -> Result<bool, String> {
        fn is_hex64(s: &str) -> bool {
            s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
        }
        if !is_hex64(store_id_hex) {
            return Err(format!("invalid store_id (want 64-hex): {store_id_hex}"));
        }
        if !is_hex64(root_hex) {
            return Err(format!("invalid root (want 64-hex): {root_hex}"));
        }
        let path = module_path(&self.cache_dir, store_id_hex, root_hex);

        let _guard = self.cache_lock.lock().await;
        if !path.exists() {
            return Ok(false); // nothing to remove — idempotent no-op
        }
        // Canonicalize and confirm the target is contained by the cache dir. With
        // 64-hex inputs this always holds; the check is defense-in-depth so the
        // unlink can never reach outside the cache even if the layout changes.
        let canon = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        let cache_canon = std::fs::canonicalize(&self.cache_dir).map_err(|e| e.to_string())?;
        if !canon.starts_with(&cache_canon) {
            return Err("refusing to remove a path outside the cache dir".to_string());
        }
        std::fs::remove_file(&canon).map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Fetch and cache one capsule on demand over the §21 authenticated
    /// whole-store sync path (the same `sync_module_from` / `DigClient::clone_store`
    /// the local-first miss path uses, signed with the startup `identity_seed`).
    /// Returns `(size_bytes, served_root_hex)` on success.
    ///
    /// If the capsule is already cached it returns its size without re-downloading
    /// (the RPC reports `already_cached`). The cache write itself happens inside
    /// `sync_module_from`, which already serializes via the module path; this also
    /// holds the `cache_lock` around the call so concurrent on-demand fetches of
    /// the same capsule don't race each other.
    pub async fn cache_fetch_and_cache(
        &self,
        store_id_hex: &str,
        root_hex: &str,
    ) -> Result<(u64, String), String> {
        // Already cached → report its size, no network.
        let existing = module_path(&self.cache_dir, store_id_hex, root_hex);
        if let Ok(md) = std::fs::metadata(&existing) {
            return Ok((md.len(), root_hex.to_string()));
        }
        // Serialize on-demand writes so two fetches of the same capsule don't race.
        let _guard = self.cache_lock.lock().await;
        // sync_module_from returns true only when the served root == requested
        // root; either way the module lands under its SERVED root, so we read the
        // file back to report size + confirm the capsule is now present.
        let matched = self
            .sync_module_from(&self.upstream, store_id_hex, root_hex)
            .await;
        let path = module_path(&self.cache_dir, store_id_hex, root_hex);
        match std::fs::metadata(&path) {
            Ok(md) => Ok((md.len(), root_hex.to_string())),
            Err(_) if matched => {
                // matched but no file: should not happen, surface it.
                Err("sync reported a match but the module is not cached".to_string())
            }
            Err(_) => Err(format!(
                "could not fetch capsule {store_id_hex}:{root_hex} (no §21 identity, \
                 not authorized, or served root differs)"
            )),
        }
    }
}

/// One cached capsule, as returned by [`Node::cache_list_cached`]. Identity is the
/// `(store_id, root)` capsule (`digstore_core::Capsule`, `storeId:rootHash`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedCapsule {
    /// Store id (lowercase 64-hex) — the directory name under `<cache>/modules/`.
    pub store_id: String,
    /// Generation root hash (lowercase 64-hex) — the `<root>.module` file stem.
    pub root: String,
    /// On-disk size of the cached module, in bytes.
    pub size_bytes: u64,
    /// Last-used time (file mtime, the LRU recency stamp) in Unix epoch ms.
    pub last_used_unix_ms: u64,
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

/// Axum route: a thin wrapper over [`handle_rpc`] for the standalone `dig-node`
/// binary. The DIG browser does NOT use this — it calls `handle_rpc` directly
/// in-process via the `dig-runtime` FFI, with no loopback server.
async fn rpc(State(node): State<Arc<Node>>, Json(req): Json<Value>) -> impl IntoResponse {
    Json(handle_rpc(&node, req).await)
}

/// String-in / string-out convenience over [`handle_rpc`] for FFI callers
/// (`dig-runtime`): parse the JSON-RPC request text, dispatch, return the
/// response as JSON text. Keeps serde out of the FFI crate so the browser side
/// is a plain `*const c_char -> *mut c_char` call.
pub async fn handle_rpc_json(node: &Node, req_json: &str) -> String {
    let req: Value = match serde_json::from_str(req_json) {
        Ok(v) => v,
        Err(e) => {
            return json!({"jsonrpc":"2.0","id":null,
                "error":{"code":-32700,"message":format!("parse error: {e}")}})
            .to_string()
        }
    };
    handle_rpc(node, req).await.to_string()
}

/// Core JSON-RPC dispatch — the actual DIG node. Takes the request Value and
/// returns the response Value. This is the single source of truth shared by the
/// axum route (standalone bin) AND the in-process FFI (`dig-runtime`), so the
/// browser process can *be* the node: its dig:// handler calls this directly,
/// no HTTP, no socket, no sidecar.
pub async fn handle_rpc(node: &Node, req: Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(json!(1));
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // dig.getAnchoredRoot: resolve a store's chain-anchored tip root (the TRUSTED
    // root for the browser's mandatory dig:// root-pinning — see anchored_root).
    if method == "dig.getAnchoredRoot" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        return node.anchored_root(&params, id).await;
    }
    // dig.stage (#95 Pass C): turn a local folder into a capsule (.dig module) IN
    // PROCESS — the staging/compile half of a local deploy. The DIG Browser's
    // in-process node calls this (no CLI binary) to produce the artifact, then
    // signs the on-chain root advance via the Pass B `chia_advanceStore` wallet
    // method and §21-pushes the module. ADDITIVE — no existing method is touched.
    if method == "dig.stage" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        return node.stage(&params, id);
    }
    // cache.* — the local-cache config for the chrome://settings DIG section.
    // The browser's Mojo handler reaches these via the in-process CallDigRpc FFI;
    // dig-node owns the cache, so it is the single source of truth (same fns the
    // dig-wallet /api/dig-config endpoint uses).
    if method == "cache.getConfig" {
        // ADDITIVE fields (#96): `cache_dir` = the effective resolved cache path,
        // `shared` = whether that path is the canonical dir shared with the
        // standalone dig-node / dig-companion (`false` = a process-private
        // fallback because the canonical dir was unwritable). Existing
        // `cap_bytes`/`used_bytes` are UNCHANGED — the FFI contract is
        // additive-only (see SYSTEM.md change-impact + the regression test).
        let (dir, shared) = resolve_cache_dir();
        return json!({"jsonrpc":"2.0","id":id,"result":{
            "cap_bytes": cache_cap_bytes(),
            "used_bytes": cache_used_bytes(),
            "cache_dir": dir.display().to_string(),
            "shared": shared}});
    }
    if method == "cache.setCapBytes" {
        let requested = req
            .get("params")
            .and_then(|p| p.get("cap_bytes"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // Floor at 64 MiB so a stray 0 can't disable caching (mirrors dig-wallet).
        let cap = requested.max(64 * 1024 * 1024);
        return match set_cache_cap_bytes(cap) {
            Ok(()) => json!({"jsonrpc":"2.0","id":id,"result":{"cap_bytes": cap}}),
            Err(e) => json!({"jsonrpc":"2.0","id":id,
                "error":{"code":-32000,"message": e.to_string()}}),
        };
    }
    if method == "cache.clear" {
        clear_cache();
        return json!({"jsonrpc":"2.0","id":id,"result":{}});
    }
    // cache.listCached / removeCached / fetchAndCache — the cached-store manager
    // (task #32). Each cached module is a CAPSULE (storeId:rootHash), so these are
    // keyed by capsule identity (`digstore_core::Capsule`).
    if method == "cache.listCached" {
        let cached: Vec<Value> = node
            .cache_list_cached()
            .await
            .into_iter()
            .map(|c| {
                json!({
                    // The canonical capsule string identity (storeId:rootHash),
                    // identical to digstore_core::Capsule::canonical().
                    "capsule": format!("{}:{}", c.store_id, c.root),
                    "store_id": c.store_id,
                    "root": c.root,
                    "size_bytes": c.size_bytes,
                    "last_used_unix_ms": c.last_used_unix_ms,
                })
            })
            .collect();
        return json!({"jsonrpc":"2.0","id":id,"result":{"cached": cached}});
    }
    if method == "cache.removeCached" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let store_hex = params
            .get("store_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let root_hex = params.get("root").and_then(|v| v.as_str()).unwrap_or("");
        return match node.cache_remove_cached(store_hex, root_hex).await {
            Ok(removed) => json!({"jsonrpc":"2.0","id":id,"result":{"removed": removed}}),
            Err(e) => json!({"jsonrpc":"2.0","id":id,
                "error":{"code":-32602,"message": e}}),
        };
    }
    if method == "cache.fetchAndCache" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let store_hex = params
            .get("store_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let root_hex = params.get("root").and_then(|v| v.as_str()).unwrap_or("");
        // Was it already present before this call? (so we can report
        // already_cached vs a fresh cached, per the spec's status field.)
        let already = module_path(&node.cache_dir, store_hex, root_hex).exists();
        return match node.cache_fetch_and_cache(store_hex, root_hex).await {
            Ok((size_bytes, served_root)) => json!({"jsonrpc":"2.0","id":id,"result":{
                "status": if already { "already_cached" } else { "cached" },
                "size_bytes": size_bytes,
                "served_root": served_root}}),
            // A failed fetch is reported in-band (status:"failed") so the settings
            // manager can show it without treating it as a transport error.
            Err(e) => json!({"jsonrpc":"2.0","id":id,"result":{
                "status": "failed",
                "message": e}}),
        };
    }
    if method != "dig.getContent" {
        return json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32601,"message":"method not found"}});
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

    // Tag the result with where it was served from so the browser can show a
    // "local" chip: "local" = from this device's cache (a compiled module or a
    // previously-cached window), "remote" = freshly fetched from rpc.dig.net.
    let local = |id: &Value, mut result: Value| -> Value {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("source".into(), json!("local"));
        }
        json!({"jsonrpc":"2.0","id":id,"result":result})
    };

    // 1. LOCAL-FIRST: serve from a cached compiled module (no network at all).
    if let (Ok(rk), false) = (decode_rk(rk_hex), root_hex.is_empty()) {
        if let Some(resp) = node.serve_local(store_hex, root_hex, &rk) {
            return local(&id, build_result(&resp, offset));
        }
        // 1b. AUTHENTICATED WHOLE-STORE SYNC (§21.9): on a module-cache miss, pull
        //     the whole `.dig` from rpc.dig.net's auth-gated §21 endpoint, cache
        //     it, then serve locally. Best-effort — a failed/disabled sync just
        //     falls through to the per-resource proxy below.
        if node.sync_module(store_hex, root_hex).await {
            if let Some(resp) = node.serve_local(store_hex, root_hex, &rk) {
                return local(&id, build_result(&resp, offset));
            }
        }
    }

    // 2. RESPONSE CACHE: a window we previously proxied for this exact request.
    let key = response_key(store_hex, root_hex, rk_hex, offset);
    if let Some(result) = node.serve_cached_response(&key) {
        return local(&id, result);
    }

    // 3. MISS: proxy to rpc.dig.net, then cache the result window (LRU-capped)
    //    so the next load of this resource is served locally. (rpc.dig.net is the
    //    remote DIG network, not a local server — the in-process node IS local.)
    match node.proxy(&req).await {
        Ok(mut v) => {
            if let Some(result) = v.get("result") {
                node.store_response(&key, result).await;
            }
            // Mark this window as freshly fetched from the network.
            if let Some(result) = v.get_mut("result").and_then(|r| r.as_object_mut()) {
                result.insert("source".into(), json!("remote"));
            }
            v
        }
        Err(e) => json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32000,"message":format!("upstream: {e}")}}),
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

impl Node {
    /// Build a node from the environment (cache dir/cap, §21 identity, upstream).
    /// Used by both the standalone bin's [`run`] and the in-process `dig-runtime`.
    pub fn from_env() -> Arc<Node> {
        let dir = cache_dir();
        let _ = std::fs::create_dir_all(&dir);
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
        Arc::new(Node {
            cache_dir: dir,
            http: reqwest::Client::builder()
                .user_agent("dig-node/0.1")
                .build()
                .expect("http client"),
            upstream: std::env::var("DIG_NODE_UPSTREAM")
                .unwrap_or_else(|_| RPC_FALLBACK.to_string()),
            cache_lock: Mutex::new(()),
            identity_seed,
        })
    }
}

/// Run the DIG node as a standalone loopback server (the `dig-node` binary only —
/// the browser does NOT use this; it calls [`handle_rpc`] in-process via the
/// `dig-runtime` FFI). Binds 127.0.0.1:`DIG_NODE_PORT` (default 9778).
pub async fn run() {
    let node = Node::from_env();
    let app = Router::new().route("/", post(rpc)).with_state(node);

    let port: u16 = std::env::var("DIG_NODE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9778);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("dig-node: cannot bind {addr}: {e}"));
    println!("dig-node listening on http://{addr}");
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
    fn wc_project_id_precedence_persisted_over_env_over_none() {
        // Persisted value wins over the env default.
        assert_eq!(
            resolve_wc_project_id(Some("persisted"), Some("from_env")),
            Some("persisted".to_string())
        );
        // No persisted value → fall back to the env default.
        assert_eq!(
            resolve_wc_project_id(None, Some("from_env")),
            Some("from_env".to_string())
        );
        // A blank persisted value is treated as unset (falls through to env),
        // never pinning an empty id.
        assert_eq!(
            resolve_wc_project_id(Some("   "), Some("from_env")),
            Some("from_env".to_string())
        );
        // Nothing configured anywhere → None (the "not configured" UI state).
        assert_eq!(resolve_wc_project_id(None, None), None);
        assert_eq!(resolve_wc_project_id(Some(""), Some("")), None);
        // Values are trimmed.
        assert_eq!(
            resolve_wc_project_id(Some("  abc  "), None),
            Some("abc".to_string())
        );
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

    // -- cache.* RPC (the chrome://settings DIG section) -----------------------

    /// Regression guard for the cache config RPC the browser's Mojo handler calls
    /// (cache.getConfig / cache.setCapBytes / cache.clear). Points the global
    /// cache dir at a throwaway tempdir via DIG_NODE_CACHE — no other test reads
    /// that env or `cache_dir()`, so the process-global set is safe here.
    // NB: this and `get_config_shape_*` mutate the PROCESS-GLOBAL `DIG_NODE_CACHE`
    // env and so hold `ENV_GUARD` for the whole body. They are plain `#[test]`
    // fns driving a current-thread runtime via `block_on` (not `#[tokio::test]`)
    // so the std mutex guard is never held across an `.await` (clippy
    // `await_holding_lock`), while still serializing against the other env tests.
    #[test]
    fn cache_rpc_config_roundtrip_and_clear() {
        let _g = ENV_GUARD.lock().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_NODE_CACHE", td.path().join("cache"));
        std::env::remove_var("DIG_NODE_CACHE_CAP");
        let (node, _td) = test_node(None);

        // setCapBytes persists the cap and echoes the effective value.
        let five_gib = 5u64 * 1024 * 1024 * 1024;
        let set = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"cache.setCapBytes",
                   "params":{"cap_bytes": five_gib}}),
        ));
        assert_eq!(set["result"]["cap_bytes"].as_u64(), Some(five_gib));

        // getConfig reflects the persisted cap and reports a used figure.
        let got = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":2,"method":"cache.getConfig"}),
        ));
        assert_eq!(got["result"]["cap_bytes"].as_u64(), Some(five_gib));
        assert!(got["result"]["used_bytes"].as_u64().is_some());

        // A below-floor request is clamped up to the 64 MiB minimum (a stray 0
        // must never disable caching).
        let low = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":3,"method":"cache.setCapBytes",
                   "params":{"cap_bytes": 1}}),
        ));
        assert_eq!(low["result"]["cap_bytes"].as_u64(), Some(64 * 1024 * 1024));

        // clear succeeds with an empty result object.
        let cleared = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":4,"method":"cache.clear"}),
        ));
        assert!(cleared["result"].is_object());

        std::env::remove_var("DIG_NODE_CACHE");
    }

    // -- Cached-store management RPCs (the DIG-settings cache manager, task #32) -

    /// Write a fake cached module for capsule (store, root) at the real
    /// `module_path` location so the management primitives see it. Returns the
    /// path written.
    fn seed_module(node: &Node, store_hex: &str, root_hex: &str, bytes: &[u8]) -> PathBuf {
        let path = module_path(&node.cache_dir, store_hex, root_hex);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[tokio::test]
    async fn list_cached_reports_capsules_with_size_and_mtime() {
        // cache.listCached enumerates every cached `.module` as a capsule
        // (storeId:rootHash) with its on-disk size and last-used time.
        let (node, _td) = test_node(None);
        let store_a = "aa".repeat(32);
        let root_a = "11".repeat(32);
        let store_b = "bb".repeat(32);
        let root_b = "22".repeat(32);
        seed_module(&node, &store_a, &root_a, b"module-a-bytes"); // 14 bytes
        seed_module(&node, &store_b, &root_b, b"bb"); // 2 bytes

        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"cache.listCached"}),
        )
        .await;
        let items = resp["result"]["cached"].as_array().unwrap();
        assert_eq!(items.len(), 2, "both cached capsules are listed");

        // Find capsule A and assert its identity + stats.
        let a = items
            .iter()
            .find(|c| c["store_id"].as_str() == Some(store_a.as_str()))
            .expect("capsule A present");
        assert_eq!(a["root"].as_str(), Some(root_a.as_str()));
        assert_eq!(a["size_bytes"].as_u64(), Some(14));
        assert!(a["last_used_unix_ms"].as_u64().is_some());
        // The canonical capsule string identity is carried verbatim.
        assert_eq!(
            a["capsule"].as_str(),
            Some(format!("{store_a}:{root_a}").as_str())
        );
    }

    #[tokio::test]
    async fn list_cached_is_empty_when_no_modules() {
        let (node, _td) = test_node(None);
        let cached = node.cache_list_cached().await;
        assert!(cached.is_empty(), "no modules → empty capsule list");
    }

    // -- dig.stage (#95 Pass C): in-process capsule staging/compile -------------
    //
    // The browser links `dig_runtime.dll` and reaches dig-node only through this
    // FFI JSON-RPC; a method/field rename silently breaks it at runtime (no
    // compile error across the FFI boundary). These tests LOCK the additive
    // `dig.stage` request params, the success result shape, and the catalogued
    // error codes (SYSTEM.md change-impact rule for the in-process dig-node FFI).

    #[tokio::test]
    async fn dig_stage_returns_the_capsule_result_shape() {
        let (node, _td) = test_node(None);
        // A folder to publish (nested, to exercise forward-slashed relative keys).
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("index.html"), b"<h1>hi</h1>").unwrap();
        std::fs::create_dir_all(src.path().join("assets")).unwrap();
        std::fs::write(src.path().join("assets").join("app.js"), b"console.log(1)").unwrap();

        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":7,"method":"dig.stage",
                "params":{"dir": src.path().display().to_string()}}),
        )
        .await;

        assert_eq!(resp["id"], 7, "id round-trips: {resp}");
        let r = &resp["result"];
        // capsule == storeId:rootHash (canonical capsule identity).
        let capsule = r["capsule"].as_str().expect("capsule string");
        let (store_hex, root_hex) = capsule.split_once(':').expect("storeId:rootHash");
        assert_eq!(store_hex.len(), 64, "store id is 64-hex: {resp}");
        assert_eq!(root_hex.len(), 64, "root is 64-hex: {resp}");
        assert_eq!(r["store_id"].as_str(), Some(store_hex));
        assert_eq!(r["root"].as_str(), Some(root_hex));
        // content_address is the dig:// URN for the capsule.
        assert_eq!(
            r["content_address"].as_str(),
            Some(format!("dig://{store_hex}:{root_hex}/").as_str())
        );
        // module_path points at a real on-disk .dig module.
        let module_path = r["module_path"].as_str().expect("module_path");
        assert!(
            std::path::Path::new(module_path).exists(),
            "module written to disk: {module_path}"
        );
        assert!(
            r["size"].as_u64().unwrap_or(0) > 0,
            "module non-empty: {resp}"
        );
        assert_eq!(r["files"].as_u64(), Some(2), "two staged files: {resp}");
        // No store_id supplied ⇒ an ephemeral (preview) capsule.
        assert_eq!(r["ephemeral"], true, "no store_id ⇒ ephemeral: {resp}");
    }

    #[tokio::test]
    async fn dig_stage_honors_a_supplied_store_id_and_is_not_ephemeral() {
        let (node, _td) = test_node(None);
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("index.html"), b"x").unwrap();
        let store = "ab".repeat(32);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.stage",
                "params":{"dir": src.path().display().to_string(), "store_id": store}}),
        )
        .await;
        let r = &resp["result"];
        assert_eq!(
            r["store_id"].as_str(),
            Some(store.as_str()),
            "store id verbatim: {resp}"
        );
        assert_eq!(
            r["ephemeral"], false,
            "supplied store_id ⇒ not ephemeral: {resp}"
        );
    }

    #[tokio::test]
    async fn dig_stage_missing_dir_is_invalid_params() {
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.stage","params":{}}),
        )
        .await;
        assert_eq!(
            resp["error"]["code"], -32602,
            "missing dir ⇒ -32602: {resp}"
        );
    }

    #[tokio::test]
    async fn dig_stage_nonexistent_dir_is_catalogued_error() {
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.stage",
                "params":{"dir":"/no/such/folder/xyzzy"}}),
        )
        .await;
        assert_eq!(resp["error"]["code"], -32011, "bad dir ⇒ -32011: {resp}");
    }

    #[tokio::test]
    async fn dig_stage_empty_folder_is_catalogued_error() {
        let (node, _td) = test_node(None);
        let src = tempfile::tempdir().unwrap();
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.stage",
                "params":{"dir": src.path().display().to_string()}}),
        )
        .await;
        assert_eq!(
            resp["error"]["code"], -32012,
            "empty folder ⇒ -32012: {resp}"
        );
    }

    #[tokio::test]
    async fn dig_stage_bad_store_id_hex_is_invalid_params() {
        let (node, _td) = test_node(None);
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("index.html"), b"x").unwrap();
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.stage",
                "params":{"dir": src.path().display().to_string(), "store_id":"nothex"}}),
        )
        .await;
        assert_eq!(
            resp["error"]["code"], -32602,
            "bad store_id ⇒ -32602: {resp}"
        );
    }

    #[tokio::test]
    async fn remove_cached_deletes_the_capsule_module() {
        let (node, _td) = test_node(None);
        let store = "cc".repeat(32);
        let root = "33".repeat(32);
        let path = seed_module(&node, &store, &root, b"to-be-removed");
        assert!(path.exists());

        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"cache.removeCached",
                   "params":{"store_id": store, "root": root}}),
        )
        .await;
        assert!(resp["result"]["removed"].as_bool() == Some(true));
        assert!(!path.exists(), "the module file is unlinked");
    }

    #[tokio::test]
    async fn remove_cached_rejects_path_traversal() {
        // A non-hex store id that tries to escape the cache dir is refused and
        // never deletes anything outside it.
        let (node, _td) = test_node(None);
        let err = node
            .cache_remove_cached("../../etc", &"33".repeat(32))
            .await
            .unwrap_err();
        assert!(
            err.contains("invalid") || err.contains("hex"),
            "traversal attempt rejected as invalid input, got: {err}"
        );
    }

    #[tokio::test]
    async fn remove_cached_missing_module_is_not_an_error() {
        // Removing a capsule that isn't cached is a no-op success (removed:false),
        // so the settings manager can call it idempotently.
        let (node, _td) = test_node(None);
        let removed = node
            .cache_remove_cached(&"dd".repeat(32), &"44".repeat(32))
            .await
            .unwrap();
        assert!(!removed, "absent capsule → removed:false");
    }

    #[tokio::test]
    async fn fetch_and_cache_syncs_a_capsule_on_demand() {
        // cache.fetchAndCache pulls a whole store over the §21 authed sync path and
        // lands it in the cache, reporting the served root + size.
        let module = b"freshly-fetched-module".to_vec();
        let (base, store_hex) = spawn_authed_remote(module.clone()).await;
        let (mut node, _td) = test_node(Some([5u8; 32]));
        node.upstream = base; // point the on-demand fetch at the authed remote
        let root_hex = "10".repeat(32); // the served genesis root

        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"cache.fetchAndCache",
                   "params":{"store_id": store_hex, "root": root_hex}}),
        )
        .await;
        assert_eq!(resp["result"]["status"].as_str(), Some("cached"));
        assert_eq!(
            resp["result"]["served_root"].as_str(),
            Some(root_hex.as_str())
        );
        assert_eq!(
            resp["result"]["size_bytes"].as_u64(),
            Some(module.len() as u64)
        );

        let cached = std::fs::read(module_path(&node.cache_dir, &store_hex, &root_hex)).unwrap();
        assert_eq!(cached, module, "fetched module is cached for local-first");

        // A second fetch of the now-present capsule reports already_cached without
        // re-downloading.
        let again = node
            .cache_fetch_and_cache(&store_hex, &root_hex)
            .await
            .unwrap();
        assert_eq!(again.0, module.len() as u64);
        let again_resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":2,"method":"cache.fetchAndCache",
                   "params":{"store_id": store_hex, "root": root_hex}}),
        )
        .await;
        assert_eq!(
            again_resp["result"]["status"].as_str(),
            Some("already_cached")
        );
    }

    #[tokio::test]
    async fn fetch_and_cache_without_identity_fails() {
        // No §21 identity → the authed sync can't run, so the fetch reports failed
        // rather than silently succeeding.
        let (node, _td) = test_node(None);
        let store = "ee".repeat(32);
        let root = "55".repeat(32);
        let err = node.cache_fetch_and_cache(&store, &root).await.unwrap_err();
        assert!(!err.is_empty(), "fetch without identity surfaces an error");

        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"cache.fetchAndCache",
                   "params":{"store_id": store, "root": root}}),
        )
        .await;
        assert_eq!(resp["result"]["status"].as_str(), Some("failed"));
    }

    // -- Shared .dig cache (#96) -----------------------------------------------
    //
    // Tests that drive the PROCESS-GLOBAL `cache_dir()` (via the `DIG_NODE_CACHE`
    // env) must not run concurrently with each other or with
    // `cache_rpc_config_roundtrip_and_clear`, since cargo runs tests in parallel
    // threads of one process. `ENV_GUARD` serializes them.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // Item 1 — Atomic content-addressed module writes.

    #[test]
    fn write_atomic_leaves_no_partial_and_overwrites_cleanly() {
        // A module written via write_atomic appears in full or not at all, never
        // as a torn temp file, and a second write of (immutable) bytes converges.
        let td = tempfile::tempdir().unwrap();
        let path = td.path().join("modules").join("aa").join("bb.module");
        write_atomic(&path, b"capsule-bytes").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"capsule-bytes");
        // No leftover temp files in the target dir (rename consumed it).
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp-* partial files left behind");
        // Re-writing identical immutable bytes converges to the same content.
        write_atomic(&path, b"capsule-bytes").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"capsule-bytes");
    }

    #[tokio::test]
    async fn concurrent_module_writers_converge_with_no_partial_observed() {
        // Two "writers" race to write the SAME capsule module concurrently; a
        // reader polling in parallel must only ever see the full bytes (never a
        // partial), and the final file is exactly the module bytes.
        use std::sync::atomic::{AtomicBool, Ordering};
        let td = tempfile::tempdir().unwrap();
        let dir = td.path().to_path_buf();
        let store = "ab".repeat(32);
        let root = "cd".repeat(32);
        let module: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let path = module_path(&dir, &store, &root);

        let stop = Arc::new(AtomicBool::new(false));
        let saw_partial = Arc::new(AtomicBool::new(false));
        // Reader: while writers run, every readable version must equal `module`.
        let reader = {
            let path = path.clone();
            let module = module.clone();
            let stop = stop.clone();
            let saw_partial = saw_partial.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(bytes) = std::fs::read(&path) {
                        if bytes != module {
                            saw_partial.store(true, Ordering::Relaxed);
                        }
                    }
                }
            })
        };

        // Two writers of the identical (immutable) module bytes.
        let mut handles = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let module = module.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..20 {
                    write_atomic(&path, &module).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        reader.join().unwrap();

        assert!(
            !saw_partial.load(Ordering::Relaxed),
            "a reader observed a torn/partial module — atomic write violated"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            module,
            "writers converge on the full module bytes"
        );
    }

    // Item 2 — Cross-process advisory lock (config lost-update + eviction).

    #[test]
    fn concurrent_config_rmw_loses_no_update() {
        // The canonical lost-update test: two "processes" each increment a shared
        // counter key via the config read-modify-write N times. Each increment is
        // read-current → +1 → write. WITHOUT the cross-process lock, interleaved
        // read/read/write/write loses increments and the final count is < 2N;
        // WITH the lock every increment is serialized and the count is EXACTLY 2N.
        let _g = ENV_GUARD.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_NODE_CACHE", td.path().join("cache"));
        let _ = std::fs::remove_file(config_path());

        const N: u64 = 100;
        fn bump() {
            for _ in 0..N {
                update_config_locked(|v| {
                    let cur = v.get("counter").and_then(|c| c.as_u64()).unwrap_or(0);
                    v["counter"] = json!(cur + 1);
                })
                .unwrap();
            }
        }
        let a = std::thread::spawn(bump);
        let b = std::thread::spawn(bump);
        a.join().unwrap();
        b.join().unwrap();

        let txt = std::fs::read_to_string(config_path()).unwrap();
        let v: Value = serde_json::from_str(&txt).expect("config.json is valid JSON");
        assert_eq!(
            v["counter"].as_u64(),
            Some(2 * N),
            "no increments lost — every read-modify-write was serialized"
        );

        std::env::remove_var("DIG_NODE_CACHE");
    }

    #[test]
    fn concurrent_setters_keep_both_keys() {
        // The two real config setters (cache cap vs wc projectId) run concurrently;
        // both keys survive in a single valid config.json (no clobber, no torn file).
        let _g = ENV_GUARD.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_NODE_CACHE", td.path().join("cache"));
        let _ = std::fs::remove_file(config_path());

        let cap = std::thread::spawn(|| {
            for i in 0..100 {
                set_cache_cap_bytes(64 * 1024 * 1024 + i).unwrap();
            }
        });
        let wc = std::thread::spawn(|| {
            for i in 0..100 {
                set_wc_project_id(&format!("proj-{i}")).unwrap();
            }
        });
        cap.join().unwrap();
        wc.join().unwrap();

        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(config_path()).unwrap()).unwrap();
        assert!(v.get("cache_cap_bytes").and_then(|x| x.as_u64()).is_some());
        assert!(v.get("wc_project_id").and_then(|x| x.as_str()).is_some());

        std::env::remove_var("DIG_NODE_CACHE");
    }

    #[test]
    fn cache_lock_is_exclusive_then_released() {
        // The advisory lock is genuinely exclusive: while one guard is held a
        // direct try_lock on the same file would block (WouldBlock); once dropped
        // it can be re-acquired. Proves eviction/config RMW are actually serialized.
        let _g = ENV_GUARD.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_NODE_CACHE", td.path().join("cache"));

        let guard = acquire_cache_lock().expect("first lock acquires");
        // A second, independent handle on the same lockfile must NOT acquire.
        let path = lockfile_path();
        let other = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(
            FileExt::try_lock(&other).is_err(),
            "a held lock must block a concurrent try_lock"
        );
        drop(guard);
        assert!(
            FileExt::try_lock(&other).is_ok(),
            "after release the lock is re-acquirable"
        );
        let _ = FileExt::unlock(&other);

        std::env::remove_var("DIG_NODE_CACHE");
    }

    // Item 3 — Robust dir resolver + writability fallback.

    #[test]
    fn canonical_cache_dir_honors_env_override() {
        let _g = ENV_GUARD.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        let want = td.path().join("custom-cache");
        std::env::set_var("DIG_NODE_CACHE", &want);
        assert_eq!(canonical_cache_dir(), want);
        std::env::remove_var("DIG_NODE_CACHE");
    }

    #[test]
    fn canonical_cache_dir_default_ends_in_dignode_cache() {
        // With no override the default path keeps the historic, byte-exact
        // `.../DigNode/cache` suffix (the shared-cache contract with dig-companion).
        let _g = ENV_GUARD.lock().unwrap();
        std::env::remove_var("DIG_NODE_CACHE");
        let dir = canonical_cache_dir();
        assert!(
            dir.ends_with("DigNode/cache") || dir.ends_with("DigNode\\cache"),
            "default cache dir must end in DigNode/cache, got {}",
            dir.display()
        );
        // On Windows the base is %LOCALAPPDATA%; on Unix/macOS it is $HOME — both
        // matching dig-companion so the cache is shared by construction.
    }

    #[test]
    fn resolve_cache_dir_reports_shared_for_writable_canonical() {
        let _g = ENV_GUARD.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_NODE_CACHE", td.path().join("cache"));
        let (dir, shared) = resolve_cache_dir();
        assert!(shared, "a writable canonical dir is reported as shared");
        assert!(dir.starts_with(td.path()), "uses the canonical (env) dir");
        std::env::remove_var("DIG_NODE_CACHE");
    }

    #[test]
    fn resolve_cache_dir_falls_back_to_private_when_unwritable() {
        // Point the canonical dir at a path that cannot be created (a child of a
        // regular FILE), forcing the writability probe to fail → private fallback.
        let _g = ENV_GUARD.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        let file = td.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let unwritable = file.join("cache"); // can't mkdir under a file
        std::env::set_var("DIG_NODE_CACHE", &unwritable);

        let (dir, shared) = resolve_cache_dir();
        assert!(
            !shared,
            "an unwritable canonical dir falls back, shared=false"
        );
        assert_eq!(dir, private_fallback_dir(), "uses the process-private dir");
        assert_ne!(dir, unwritable, "does not use the unwritable canonical dir");

        std::env::remove_var("DIG_NODE_CACHE");
    }

    // Item 4 — Additive cache.getConfig FFI shape (regression guard).

    #[test]
    fn get_config_shape_is_additive_existing_fields_intact_plus_new() {
        // FFI change-impact rule (SYSTEM.md): cache.getConfig must keep its
        // existing fields and ONLY add `cache_dir` + `shared`. This pins the shape
        // so a rename/removal of cap_bytes/used_bytes breaks the build, not the
        // browser silently at runtime.
        let _g = ENV_GUARD.lock().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_NODE_CACHE", td.path().join("cache"));
        std::env::remove_var("DIG_NODE_CACHE_CAP");
        let (node, _node_td) = test_node(None);

        let got = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":42,"method":"cache.getConfig"}),
        ));
        let result = got["result"].as_object().expect("result is an object");

        // EXISTING fields (must remain, same types).
        assert!(
            result.get("cap_bytes").and_then(|v| v.as_u64()).is_some(),
            "cap_bytes still present (u64)"
        );
        assert!(
            result.get("used_bytes").and_then(|v| v.as_u64()).is_some(),
            "used_bytes still present (u64)"
        );
        // NEW additive fields.
        let dir = result
            .get("cache_dir")
            .and_then(|v| v.as_str())
            .expect("cache_dir present (string)");
        assert!(!dir.is_empty(), "cache_dir is the effective resolved path");
        let shared = result
            .get("shared")
            .and_then(|v| v.as_bool())
            .expect("shared present (bool)");
        assert!(shared, "a writable env-set cache dir is shared");
        // Envelope intact.
        assert_eq!(got["id"], json!(42));
        assert_eq!(got["jsonrpc"], json!("2.0"));

        std::env::remove_var("DIG_NODE_CACHE");
    }
}
