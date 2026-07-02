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
use std::sync::{Arc, OnceLock};

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

pub mod dht;
pub mod download;
pub mod net;
pub mod peer;
pub mod pex;

/// JSON-RPC error code: the served/requested root is NOT the store's
/// chain-anchored root (gap #127). A content read is gated on this: it serves
/// against the CHIP-0035 singleton's current on-chain root or it FAILS CLOSED
/// with this code — a compromised upstream/host can never pick which generation
/// is served, and a module that carries no on-chain anchor is rejected (not
/// silently downgraded to a no-op). Catalogued in docs.dig.net error tables and
/// uniform with the CLI clone/pull pin (which fails closed with the same
/// "chain is the authority" semantics).
const ROOT_NOT_ANCHORED: i64 = -32005;

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
    /// Resolver for the store's CHIP-0035 chain-anchored root — the trusted-root
    /// source for the MANDATORY read-path pin (#127). Production is
    /// [`CoinsetResolver`] (the live singleton walk); tests inject a deterministic
    /// one so the fail-closed gate is unit-tested without a chain.
    anchored_root_resolver: Arc<dyn AnchoredRootResolver>,
    /// Live, pool-oriented status of the node's L7 peer network (the connected peer pool + the
    /// mTLS peer-RPC server). Shared with the background peer-network task spawned by the standalone
    /// [`run`]; surfaced via `control.peerStatus`. In the in-process FFI path (the browser) no peer
    /// network runs, so this stays "not running" — the browser is a consumer, not a reachable peer.
    /// (Replaces the retired bespoke relay-connection status; relay reachability now lives in
    /// dig-nat/dig-gossip and is reported here as the pool's relay-reservation flag.)
    peer_status: Arc<peer::PeerStatus>,
    /// The P2P content engine (#164/#165): the dig-download multi-source fetch path + the
    /// redirect-on-miss provider lookup. Set ONCE by the standalone peer-network bring-up
    /// ([`peer::spawn_peer_network`]) via [`Node::set_p2p_content`]; NEVER set in the in-process FFI
    /// path (the browser is a pure consumer), so a content miss there behaves exactly as before (no
    /// redirect/fetch-through — the miss handler is a no-op without this). See [`crate::download`].
    p2p_content: OnceLock<Arc<download::NodeContent>>,
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
///
/// The probe name is unique PER CALL (pid + a monotonic counter), NOT per-pid:
/// `resolve_cache_dir` runs on every `cache_dir()`/`config_path()`/`lockfile_path()`
/// call, so two threads of one process probe concurrently. A shared probe name
/// let one thread's `remove_file` race the other's `write` (a transient
/// sharing-violation `Err` on Windows), spuriously reporting the dir UNwritable
/// → that one call returned the private-fallback dir → its `config_path()` pointed
/// at a DIFFERENT file → a lost config update. A unique name makes the probe
/// race-free, so resolution is stable under concurrency.
fn dir_is_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    static PROBE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
    let probe = dir.join(format!(".write-probe-{}-{}", std::process::id(), seq));
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

/// In-process serializer for the config read-modify-write. The cross-process
/// `flock` (`.dignode.lock`) is NOT sufficient on its own: on Windows
/// `LockFileEx` is per-handle and does NOT block a SECOND lock taken by the SAME
/// process (two threads each open their own handle and both acquire), so two
/// threads of one process can still interleave read/read/write/write and lose an
/// increment. This process-global mutex makes the RMW atomic *within* this
/// process; the flock makes it atomic *across* processes. Together they give the
/// lost-update-free guarantee the doc above promises, on every OS.
static CONFIG_RMW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Read-modify-write the config JSON under both an in-process mutex and the
/// cross-process lock so neither two threads nor two processes can lose each
/// other's update (the lost-update race). Reads the current config, applies
/// `mutate`, and writes it back atomically (temp + rename) — all while holding
/// both locks. Pretty-prints to keep the on-disk `config.json` schema
/// byte-compatible with the prior writer.
fn update_config_locked(mutate: impl FnOnce(&mut Value)) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Serialize this PROCESS's RMWs (recover from a poisoned lock — a prior
    // panicker left the guarded config in a consistent on-disk state, so the
    // poison carries no broken invariant we must honor).
    let _in_proc = CONFIG_RMW_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Hold the cross-process lock across the read AND the write so a concurrent
    // PROCESS can't read-then-clobber between our read and our write.
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
            // The chia:// content-open address for this capsule (the user-facing
            // scheme the DIG Browser/extension register; matches deploy --preview).
            "content_address": format!("chia://{store_hex}:{root_hex}/"),
            "files": compiled.files(),
            // true ⇒ a preview capsule with a content-derived id (NOT a real store).
            "ephemeral": ephemeral,
        }})
    }

    // -- Public collection reads (#39) -----------------------------------------
    //
    // Owner-independent, third-party-indexer-free reads of an NFT collection from
    // DIG's own coinset data. Read-only: NO spend bundles are built or pushed. The
    // item set is the NFT launcher ids the collection mint produced — the stable,
    // owner-independent anchor (a DID-attributed NFT is hinted to its OWNER at mint,
    // not to the creator DID, so launcher ids — not the DID — are the discovery key;
    // see digstore_chain::collection_index). Each launcher is resolved to its CURRENT
    // on-chain owner + royalty + CHIP-0007 metadata by walking the singleton lineage
    // forward to the unspent tip, so the reported owner is always live, not mint-time.

    /// Parse `params.launcher_ids` (an array of 64-hex strings) into canonical
    /// [`chia_protocol::Bytes32`] launcher ids, preserving order (the result is
    /// deterministic in input order). `Err(bad_value)` names the first malformed id.
    fn parse_launcher_ids(params: &Value) -> Result<Vec<chia_protocol::Bytes32>, String> {
        let arr = params
            .get("launcher_ids")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                "params.launcher_ids must be an array of 64-hex launcher ids".to_string()
            })?;
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            let s = v
                .as_str()
                .ok_or_else(|| "each launcher id must be a 64-hex string".to_string())?;
            let h = s.trim_start_matches("0x");
            let bytes = hex::decode(h).map_err(|_| format!("launcher id is not hex: {s}"))?;
            let a: [u8; 32] = bytes
                .try_into()
                .map_err(|_| format!("launcher id must be 32 bytes (64 hex): {s}"))?;
            out.push(chia_protocol::Bytes32::new(a));
        }
        Ok(out)
    }

    /// Render one resolved [`IndexedNft`](digstore_chain::collection_index::IndexedNft)
    /// as the stable JSON-RPC item shape. Field names mirror the asset CLI
    /// (`launcher_id`/`coin_id`/`owner_did`/`royalty_*`/`owner_puzzle_hash`), with the
    /// decoded on-chain CHIP-0007 metadata under `metadata` (null when it does not
    /// decode). The on-chain `NftMetadata` (CLVM struct) carries no serde derive, so
    /// the metadata object is rendered field-by-field with stable names + lowercase-hex
    /// 32-byte hashes — a self-describing, agent-consumable shape.
    fn item_json(item: &digstore_chain::collection_index::IndexedNft) -> Value {
        let metadata = item
            .metadata
            .as_ref()
            .map(|m| {
                json!({
                    "edition_number": m.edition_number,
                    "edition_total": m.edition_total,
                    "data_uris": m.data_uris,
                    "data_hash": m.data_hash.map(hex::encode),
                    "metadata_uris": m.metadata_uris,
                    "metadata_hash": m.metadata_hash.map(hex::encode),
                    "license_uris": m.license_uris,
                    "license_hash": m.license_hash.map(hex::encode),
                })
            })
            .unwrap_or(Value::Null);
        json!({
            "launcher_id": hex::encode(item.launcher_id),
            "coin_id": hex::encode(item.coin_id),
            "owner_did": item.owner_did.map(hex::encode),
            "royalty_puzzle_hash": hex::encode(item.royalty_puzzle_hash),
            "royalty_basis_points": item.royalty_basis_points,
            "owner_puzzle_hash": hex::encode(item.owner_puzzle_hash),
            "metadata": metadata,
        })
    }

    /// `dig.getCollection` — collection-level facts for a given item set.
    ///
    /// Params: `launcher_ids` (required array of 64-hex), optional `did` (64-hex; the
    /// collection's creator DID, echoed + used as the expected attribution). Resolves
    /// every launcher to its current state, then derives the shared creator DID (if
    /// uniform), the resolved item count, and the uniform royalty.
    ///
    /// Result: `{ did, declared_did, item_count, resolved_count, royalty_basis_points }`.
    /// Errors: `-32602` invalid params.
    async fn get_collection(params: &Value, id: Value) -> Value {
        let launcher_ids = match Self::parse_launcher_ids(params) {
            Ok(v) => v,
            Err(msg) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":msg}})
            }
        };
        // Optional declared creator DID (echoed back; the source of truth is the
        // items' on-chain attribution).
        let declared_did = params
            .get("did")
            .and_then(|v| v.as_str())
            .map(|s| s.trim_start_matches("0x").to_string());

        let chain = resolution_coinset();
        let items =
            match digstore_chain::collection_index::index_collection_items(&chain, &launcher_ids)
                .await
            {
                Ok(items) => items,
                Err(e) => {
                    return json!({"jsonrpc":"2.0","id":id,"error":{
                    "code":-32000,"message":format!("read collection: {e}")}})
                }
            };
        let summary = digstore_chain::collection_index::summarize_collection(&items);
        json!({"jsonrpc":"2.0","id":id,"result":{
            // The creator DID the items AGREE on (None if mixed/none), lowercase hex.
            "did": summary.did.map(hex::encode),
            // The DID the caller declared (echoed; may be null).
            "declared_did": declared_did,
            // How many launcher ids were requested vs how many resolved to a live NFT.
            "item_count": launcher_ids.len(),
            "resolved_count": summary.item_count,
            // The royalty every item agrees on (basis points), or null when mixed.
            "royalty_basis_points": summary.royalty_basis_points,
        }})
    }

    /// `dig.listCollectionItems` — a deterministic, paginated page of a collection's
    /// items resolved to their CURRENT on-chain state.
    ///
    /// Params: `launcher_ids` (required array of 64-hex; the authoritative item set),
    /// optional `offset` (default 0) + `limit` (default 50, capped 200). Pagination is
    /// applied over the launcher-id list BEFORE resolution, so only the requested page
    /// is read from chain. Order is the input order (stable).
    ///
    /// Result: `{ items: [ {launcher_id, coin_id, owner_did, royalty_puzzle_hash,
    /// royalty_basis_points, owner_puzzle_hash, metadata} ], offset, limit, total,
    /// next_offset }`. `next_offset` is null on the last page. Errors: `-32602`.
    async fn list_collection_items(params: &Value, id: Value) -> Value {
        let launcher_ids = match Self::parse_launcher_ids(params) {
            Ok(v) => v,
            Err(msg) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":msg}})
            }
        };
        let total = launcher_ids.len();
        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        // Default page 50, capped at 200 so one call can't fan out unbounded chain reads.
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(200))
            .unwrap_or(50) as usize;

        let page: Vec<chia_protocol::Bytes32> = launcher_ids
            .iter()
            .skip(offset)
            .take(limit)
            .copied()
            .collect();

        let chain = resolution_coinset();
        let resolved =
            match digstore_chain::collection_index::index_collection_items(&chain, &page).await {
                Ok(items) => items,
                Err(e) => {
                    return json!({"jsonrpc":"2.0","id":id,"error":{
                    "code":-32000,"message":format!("list collection items: {e}")}})
                }
            };
        let items: Vec<Value> = resolved.iter().map(Self::item_json).collect();
        // next_offset points past this page unless we have reached the end of the input.
        let consumed = offset.saturating_add(page.len());
        let next_offset = if consumed < total {
            json!(consumed)
        } else {
            Value::Null
        };
        json!({"jsonrpc":"2.0","id":id,"result":{
            "items": items,
            "offset": offset,
            "limit": limit,
            "total": total,
            "next_offset": next_offset,
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

    // -- L7 peer RPC (PHASE-2b, #162) — serving the node's LOCAL inventory ------
    //
    // The node serves the SAME content over the peer network that it serves over §21 / the HTTP read
    // path: the capsules cached on disk. These build the L7 answers (`dig.getAvailability`,
    // `dig.listInventory`, `dig.fetchRange`, `dig.getNetworkInfo`) from `cache_list_cached()` +
    // `serve_local`. They are pure reads of local state (no chain, no upstream), so a peer only ever
    // learns what this node already holds. Every byte a peer fetches carries its own merkle proof
    // (verified by the caller against the chain-anchored root), so the node is never the trust anchor.

    /// The node's own `peer_id` (64-hex) derived from its persistent §21 identity seed, or `None` if
    /// no identity is configured. This is the mTLS SPKI-hash identity the node presents on the peer
    /// network (see [`peer::identity_from_seed`]).
    pub fn peer_id_hex(&self) -> Option<String> {
        let seed = self.identity_seed?;
        peer::identity_from_seed(&seed)
            .ok()
            .map(|id| id.peer_id.to_hex())
    }

    /// `dig.getAvailability` — answer one queried item against the local inventory, enriching the
    /// pure presence answer (`peer::availability_presence`) with the per-resource `total_length` +
    /// `chunk_count` when the item is at resource granularity (`store_id` + `root` + `retrieval_key`)
    /// and the resource is actually served locally. Returns one `AvailabilityAnswer` value.
    async fn availability_answer(&self, item: &Value) -> Value {
        let store = item.get("store_id").and_then(Value::as_str).unwrap_or("");
        let root = item.get("root").and_then(Value::as_str);
        let rk = item.get("retrieval_key").and_then(Value::as_str);
        let cached = self.cache_list_cached().await;
        let mut answer = peer::availability_presence(&cached, store, root, rk);

        // Resource granularity: if we hold this capsule AND can serve the resource, report its
        // ciphertext length + chunk count so the caller can plan ranges without a probe fetch.
        if let (Some(root_hex), Some(rk_hex)) = (root, rk) {
            if answer["available"].as_bool() == Some(true) {
                if let Ok(rk_bytes) = decode_rk(rk_hex) {
                    if let Some(resp) = self.serve_local(store, root_hex, &rk_bytes) {
                        if let Some(obj) = answer.as_object_mut() {
                            obj.insert("total_length".into(), json!(resp.ciphertext.len()));
                            obj.insert("chunk_count".into(), json!(chunk_count_for(&resp)));
                            obj.insert("complete".into(), json!(true));
                        }
                    }
                }
            }
        }

        // NOT-HELD → REDIRECT-ON-MISS hint (#165, read tier): if this node lacks the item but its P2P
        // engine locates holders in the DHT, name them in a `providers` array so the caller re-requests
        // against a holder instead of dead-ending — the availability-shaped counterpart to the
        // getContent/fetchRange redirect. No engine / no provider → the plain not-available answer
        // stands (the field is simply absent). Self is excluded by `find_providers`.
        if answer["available"].as_bool() != Some(true) {
            if let Some(pc) = self.p2p_content() {
                if let Some(content) = download::availability_content_id(store, root, rk) {
                    let providers = pc.find_providers(&content).await;
                    if !providers.is_empty() {
                        if let Some(obj) = answer.as_object_mut() {
                            obj.insert("providers".into(), download::providers_json(&providers));
                        }
                    }
                }
            }
        }
        answer
    }

    /// `dig.getAvailability` — batch answer for `items` (positionally aligned). Wraps
    /// [`Node::availability_answer`] per item into the `{ "items": [...] }` result shape.
    pub async fn availability_batch(&self, items: &[Value]) -> Value {
        let mut answers = Vec::with_capacity(items.len());
        for item in items {
            answers.push(self.availability_answer(item).await);
        }
        json!({ "items": answers })
    }

    /// `dig.fetchRange` — build ONE range frame (the node window is a single frame; the caller streams
    /// further windows by advancing `offset`). Serves the resource's ciphertext from a locally cached
    /// module and slices `[offset, offset+length)` (clamped to the node window). The FIRST frame
    /// (`offset == 0`) carries the verification metadata (`total_length`, `chunk_lens`, `chunk_index`,
    /// `inclusion_proof`, `root`) so the range is independently verifiable against the chain-anchored
    /// root. Returns `Err((code, message))` with the catalogued `-32004`/`-32007` on a miss / bad
    /// range. (Capsule fetches — `capsule: true` — are not yet served here; that lands with the whole
    /// `.dig` streaming path and returns `-32004` for now, a clean seam.)
    pub async fn fetch_range_frame(
        &self,
        store_hex: &str,
        root_hex: &str,
        rk_hex: &str,
        offset: usize,
        length: usize,
    ) -> Result<Value, (i64, String)> {
        let rk = decode_rk(rk_hex).map_err(|_| {
            (
                -32602,
                "retrieval_key must be 32 bytes (64-hex)".to_string(),
            )
        })?;
        let resp = self.serve_local(store_hex, root_hex, &rk).ok_or((
            -32004,
            "resource not held at the requested root".to_string(),
        ))?;

        let total = resp.ciphertext.len();
        // offset past the end is unsatisfiable (spec -32007). offset == total is the empty terminal.
        if offset > total {
            return Err((
                -32007,
                format!("offset {offset} beyond resource length {total}"),
            ));
        }
        let start = offset.min(total);
        let end = (start + length.min(peer::RANGE_WINDOW)).min(total);
        let window = resp.ciphertext[start..end].to_vec();
        let complete = end >= total;

        let mut frame = json!({
            "offset": start,
            "length": window.len(),
            "bytes": base64::engine::general_purpose::STANDARD.encode(&window),
            "complete": complete,
        });
        // First frame carries the per-range verification metadata (spec §9).
        if start == 0 {
            if let Some(obj) = frame.as_object_mut() {
                obj.insert("total_length".into(), json!(total));
                obj.insert("chunk_lens".into(), json!(resp.chunk_lens));
                obj.insert("chunk_index".into(), json!(0));
                obj.insert(
                    "inclusion_proof".into(),
                    json!(base64::engine::general_purpose::STANDARD
                        .encode(resp.merkle_proof.to_bytes())),
                );
                obj.insert("root".into(), json!(resp.roothash.to_hex()));
            }
        }
        Ok(frame)
    }

    /// `dig.getNetworkInfo` — this node's own network posture: its `peer_id`, network id, listen
    /// address, candidate addresses, reachability, and relay-reservation state. Reads the shared
    /// [`peer::PeerStatus`] so it reflects the live pool/relay state (or "not running" in the FFI
    /// path). Never touches the chain or an upstream.
    pub fn network_info(&self) -> Value {
        let peer_id = self.peer_id_hex();
        let network_id = peer::network_id_from_env();
        let endpoint = peer::relay_url_from_env();
        let port = peer::peer_port_from_env();
        // The node's REAL advertised candidate addresses, ordered IPv6-first (ecosystem HARD RULE):
        // a routable IPv6 address (when discoverable) precedes the IPv4 fallback. `listen_addr` reports
        // the primary (IPv6-preferred) advertised endpoint — a dialable address, NOT the wildcard bind
        // address (`[::]` / `0.0.0.0`) the listener binds. (The listener itself binds `[::]` dual-stack;
        // that wildcard is a bind target, never a dialable candidate to report to peers.)
        let candidates = net::advertised_socket_addrs(port, net::advertise_loopback_from_env());
        let candidate_addresses: Vec<String> = candidates.iter().map(|a| a.to_string()).collect();
        let listen = candidate_addresses
            .first()
            .cloned()
            .unwrap_or_else(|| format!("[::]:{port}"));
        let snap = self.peer_status.snapshot_json(&endpoint, &network_id);
        let reserved = snap["relay"]["reserved"].as_bool().unwrap_or(false);
        // Conservative, honest reachability: while a relay reservation is held we report "relayed"
        // (a NAT'd node reached via the relay). A confirmed direct inbound mapping (UPnP/NAT-PMP/PCP)
        // is not yet surfaced by the pool, so "direct" is reported only when no relay is in use rather
        // than claimed without evidence. (A future mapping-probe upgrades this to "direct".)
        let reachability = if reserved { "relayed" } else { "direct" };
        json!({
            "peer_id": peer_id,
            "network_id": network_id,
            "listen_addr": listen,
            "reflexive_addr": Value::Null,
            "candidate_addresses": candidate_addresses,
            "reachability": reachability,
            "relay": snap["relay"],
        })
    }
}

/// The number of chunks a served [`ContentResponse`] carries: the length of `chunk_lens`, or `1` for
/// a single-chunk resource (which omits `chunk_lens`). Pure over the response.
fn chunk_count_for(resp: &ContentResponse) -> usize {
    if resp.chunk_lens.is_empty() {
        1
    } else {
        resp.chunk_lens.len()
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

/// Resolve a store's CHIP-0035 chain-anchored TIP root. This is the trusted-root
/// source for the MANDATORY read-path pin (#127): a content read serves against
/// the on-chain current root or fails closed — it never trusts an upstream-/
/// host-reported root.
///
/// Implemented as a trait so the read-path pin is unit-testable without a live
/// chain: production uses [`CoinsetResolver`] (walks the singleton lineage on
/// coinset.org); tests inject a deterministic resolver. `Ok(Some(root))` = the
/// resolved tip; `Ok(None)` = the store is not minted / has no confirmed
/// generation (treated as fail-closed by the caller); `Err` = the chain was
/// unreachable (also fail-closed).
#[async_trait::async_trait]
pub trait AnchoredRootResolver: Send + Sync {
    /// Resolve `store_id`'s current on-chain root, or `None` if the store has no
    /// confirmed generation yet, or `Err` if the chain is unreachable.
    async fn anchored_root(&self, store_id: &[u8; 32]) -> Result<Option<Bytes32>, String>;
}

/// Production resolver: walks the store's DataStore singleton lineage on
/// coinset.org (`digstore_chain::singleton::sync_datastore`) to the unspent tip
/// and returns its metadata root — exactly the source `dig.getAnchoredRoot` and
/// `dig-resolver` already use, and the same authority the CLI clone/pull pin
/// resolves against (`current_root`). NEVER consults the serving node.
struct CoinsetResolver;

#[async_trait::async_trait]
impl AnchoredRootResolver for CoinsetResolver {
    async fn anchored_root(&self, store_id: &[u8; 32]) -> Result<Option<Bytes32>, String> {
        let launcher = chia_protocol::Bytes32::new(*store_id);
        match sync_datastore(&resolution_coinset(), launcher).await {
            Ok(store) => {
                // Convert chia_protocol::Bytes32 → digstore_core::Bytes32 (the
                // node's content-root type), mirroring the CLI clone/pull pin.
                let mut a = [0u8; 32];
                a.copy_from_slice(store.info.metadata.root_hash.as_ref());
                Ok(Some(Bytes32(a)))
            }
            Err(e) => {
                // A "not minted yet" / "launcher unspent" lineage error is a
                // legitimate absence (no confirmed generation), distinct from an
                // unreachable chain. Either way the read FAILS CLOSED at the
                // caller; we only distinguish them for a clearer error message.
                let msg = e.to_string();
                if msg.contains("not minted") || msg.contains("unspent") {
                    Ok(None)
                } else {
                    Err(msg)
                }
            }
        }
    }
}

/// The default anchored-root resolver (production coinset walk).
fn default_anchored_resolver() -> Arc<dyn AnchoredRootResolver> {
    Arc::new(CoinsetResolver)
}

/// Whether the mandatory read-path root pin is enforced. Default: ENFORCED
/// (fail-closed). The ONLY opt-out is the explicit `DIG_NODE_PIN=off`
/// environment variable for offline/local development — a deliberate, named
/// escape hatch, never the default. Any other value (or unset) enforces the pin.
///
/// This mirrors the CLI's stance (the pin is on; offline tests opt out via the
/// `DIGSTORE_ANCHOR_MOCK*` envs): a read either resolves against the
/// chain-anchored root or refuses to serve.
fn pin_enforced() -> bool {
    !matches!(
        std::env::var("DIG_NODE_PIN").ok().as_deref(),
        Some("off") | Some("0") | Some("false")
    )
}

/// Outcome of the read-path anchored-root pin for one `dig.getContent` call.
enum PinDecision {
    /// Serve against this concrete root (the chain-anchored tip). For an
    /// explicit-root request this equals the requested root; for a rootless
    /// request it is the resolved tip.
    ServeAt(Bytes32),
    /// Pinning is disabled (`DIG_NODE_PIN=off`); serve against the requested root
    /// as-is. The browser/SDK client still verifies the proof against its own
    /// trust root, so this only relaxes the NODE-side gate for local dev.
    Unpinned,
    /// Fail closed with this JSON-RPC error code + message (mismatch / chain
    /// unreachable / no confirmed generation / rootless under enforcement).
    Reject(i64, String),
}

/// Decide what root a `dig.getContent` call may serve against, enforcing the
/// mandatory chain-anchored pin (#127). Pure over its inputs (the resolved
/// `anchored` value), so the policy is unit-tested directly:
///
/// - pin disabled → [`PinDecision::Unpinned`].
/// - chain unreachable (`Err`) → reject (fail closed; never serve a root the
///   chain could not confirm).
/// - no confirmed generation (`Ok(None)`) → reject.
/// - explicit `requested` root present → it MUST equal the anchored root, else
///   reject; on match, serve at the anchored root.
/// - rootless request (`requested` is `None`) → serve at the resolved anchored
///   root (the chain tip is the authority — NEVER an upstream "latest").
fn decide_pin(
    enforced: bool,
    requested: Option<Bytes32>,
    anchored: Result<Option<Bytes32>, String>,
) -> PinDecision {
    if !enforced {
        return PinDecision::Unpinned;
    }
    let anchored = match anchored {
        Ok(Some(root)) => root,
        Ok(None) => {
            return PinDecision::Reject(
                ROOT_NOT_ANCHORED,
                "store has no confirmed on-chain generation (chain is the authority)".into(),
            )
        }
        Err(e) => {
            return PinDecision::Reject(
                ROOT_NOT_ANCHORED,
                format!("could not read the store's on-chain root: {e} (chain is the authority)"),
            )
        }
    };
    match requested {
        Some(req) if req != anchored => PinDecision::Reject(
            ROOT_NOT_ANCHORED,
            format!(
                "served root {} does not match the store's on-chain root {} (chain is the authority)",
                req.to_hex(),
                anchored.to_hex()
            ),
        ),
        // Explicit root matches the chain tip, or rootless → serve at the tip.
        _ => PinDecision::ServeAt(anchored),
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

/// Build a JSON-RPC 2.0 error response envelope. A free function (not the local `err` closure inside
/// [`handle_rpc`]'s getContent section) so the early peer-RPC handlers can report catalogued errors
/// before that closure is in scope.
fn rpc_err(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
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
    // dig.getCollection / dig.listCollectionItems (#39): PUBLIC, owner-independent
    // collection reads computed from DIG's own coinset data — no third-party indexer.
    // Read-only (no spend bundles). The item set is the NFT launcher ids the mint
    // produced (the authoritative, owner-independent anchor; see
    // digstore_chain::collection_index for why launcher ids, not the creator DID
    // hint, are the discovery key). Each item is resolved to its CURRENT on-chain
    // owner + royalty + CHIP-0007 metadata by walking the singleton lineage forward.
    if method == "dig.getCollection" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        return Node::get_collection(&params, id).await;
    }
    if method == "dig.listCollectionItems" {
        let params = req.get("params").cloned().unwrap_or(json!({}));
        return Node::list_collection_items(&params, id).await;
    }
    // -- L7 peer RPC (PHASE-2b, #162) — the node-profile peer-network methods -----------------------
    //
    // Additive JSON-RPC methods that expose the peer network over the node's RPC surface, so an agent
    // (or the peer transport's JSON-RPC stream path) drives discovery + availability + range fetch
    // without speaking the binary peer protocol. They are served here (over §21/FFI AND over an
    // inbound mTLS peer stream, which routes JSON-RPC frames through this same dispatch). See
    // docs.dig.net → L7 · DIG Node peer network + openrpc-node.json.
    if method == "dig.getNetworkInfo" {
        // This node's own posture (identity, reachability, candidate addrs, relay reservation).
        return json!({"jsonrpc":"2.0","id":id,"result": node.network_info()});
    }
    if method == "dig.getPeers" {
        // The peers this node currently knows (peer-exchange over RPC). The connected-pool source is
        // owned by the live GossipService in the standalone run(); the node struct here does not hold
        // the gossip handle (it stays FFI-safe), so this base dispatch returns the node's own view:
        // an empty peer list when no pool is wired. The standalone peer-network task answers inbound
        // `dig.getPeers` from the live pool via its own responder override (see peer::PoolResponder).
        return json!({"jsonrpc":"2.0","id":id,"result": {"peers": []}});
    }
    if method == "dig.announce" {
        // Accept an announcement (peer_id + candidate addresses). The base node has no pool to fold it
        // into, so it acknowledges without growing a peer view; the live peer-network task overrides
        // this to register the announced peer with the pool/introducer. Validates the required params.
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let peer_id_ok = params
            .get("peer_id")
            .and_then(Value::as_str)
            .map(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .unwrap_or(false);
        let has_addrs = params
            .get("addresses")
            .map(Value::is_array)
            .unwrap_or(false);
        if !peer_id_ok || !has_addrs {
            return rpc_err(
                &id,
                -32602,
                "dig.announce requires peer_id (64-hex) + addresses (array)",
            );
        }
        return json!({"jsonrpc":"2.0","id":id,"result": {"accepted": true, "known_peers": 0}});
    }
    if method == "dig.getAvailability" {
        // Batch-answer whether this node holds the queried stores/roots/capsules (from local
        // inventory), so a downloader confirms holders + plans ranges before any fetch.
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let items = match params.get("items").and_then(Value::as_array) {
            Some(items) => items.clone(),
            None => {
                return rpc_err(
                    &id,
                    -32602,
                    "dig.getAvailability requires params.items (array)",
                )
            }
        };
        return json!({"jsonrpc":"2.0","id":id,"result": node.availability_batch(&items).await});
    }
    if method == "dig.listInventory" {
        // Enumerate what this node serves: its stores, or the roots it holds for a given store.
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let store_id = params.get("store_id").and_then(Value::as_str);
        if let Some(s) = store_id {
            if !(s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
                return rpc_err(&id, -32602, "store_id must be 64-hex");
            }
        }
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let cached = node.cache_list_cached().await;
        return json!({"jsonrpc":"2.0","id":id,
            "result": peer::list_inventory(&cached, store_id, limit)});
    }
    if method == "dig.fetchRange" {
        // A single range frame of a resource this node holds (the JSON-RPC face of the streamed
        // peer-transport range fetch; the caller advances `offset` for further windows). The frame
        // carries the per-range verification metadata on the first window.
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let store_hex = params.get("store_id").and_then(Value::as_str).unwrap_or("");
        let root_hex = params.get("root").and_then(Value::as_str).unwrap_or("");
        let rk_hex = params
            .get("retrieval_key")
            .and_then(Value::as_str)
            .unwrap_or("");
        let capsule = params
            .get("capsule")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let length = params.get("length").and_then(Value::as_u64).unwrap_or(0) as usize;
        if store_hex.len() != 64 || length == 0 {
            return rpc_err(
                &id,
                -32602,
                "dig.fetchRange requires store_id (64-hex) + length (>0)",
            );
        }
        if capsule {
            // Whole-capsule streaming is a clean follow-up seam (the .dig streaming path); resource
            // range fetch is served now. Report the catalogued unavailable code for capsule mode.
            return rpc_err(
                &id,
                -32004,
                "capsule range fetch not served by this node yet (use resource retrieval_key)",
            );
        }
        if rk_hex.len() != 64 || root_hex.len() != 64 {
            return rpc_err(
                &id,
                -32602,
                "resource fetchRange requires retrieval_key + root (64-hex each)",
            );
        }
        return match node
            .fetch_range_frame(store_hex, root_hex, rk_hex, offset, length)
            .await
        {
            Ok(frame) => json!({"jsonrpc":"2.0","id":id,"result": frame}),
            // A LOCAL MISS (-32004): try the #165 P2P miss path — redirect to a holder (default) or
            // fetch-through via dig-download — before returning the bare not-found. An empty engine
            // (FFI path) or no provider yields `None` and the original error stands (no silent 404
            // when a provider exists). Other errors (e.g. -32007 bad range) pass through unchanged.
            Err((code, message)) => {
                if code == download::RESOURCE_UNAVAILABLE {
                    if let Some(content) = download::miss_content_for(store_hex, root_hex, rk_hex) {
                        let depth = download::redirect_depth(&params);
                        if let Some(envelope) = node
                            .range_miss_envelope(&id, &content, depth, offset, length)
                            .await
                        {
                            return envelope;
                        }
                    }
                }
                rpc_err(&id, code, &message)
            }
        };
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
    // control.peerStatus — live, pool-oriented status of the node's L7 peer network (the connected
    // peer pool + the relay reservation for NAT reachability). Read-only; safe before/without a peer
    // network running (then `running:false`). Replaces the retired `control.relayStatus`: relay
    // reachability now lives in dig-nat/dig-gossip and is reported here as the pool's relay flag.
    if method == "control.peerStatus" {
        let endpoint = peer::relay_url_from_env();
        let network_id = peer::network_id_from_env();
        return json!({"jsonrpc":"2.0","id":id,
            "result": node.peer_status.snapshot_json(&endpoint, &network_id)});
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
    let requested_root_hex = params.get("root").and_then(|v| v.as_str()).unwrap_or("");
    let rk_hex = params
        .get("retrieval_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let err = |id: &Value, code: i64, msg: String| -> Value {
        json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})
    };

    // -- MANDATORY anchored-root pin (#127) ------------------------------------
    //
    // Before serving ANY content (local module, §21 sync, cached window, or an
    // upstream proxy), resolve the store's CHIP-0035 chain-anchored TIP root and
    // require the served generation to BE that root, or FAIL CLOSED. The chain —
    // not the request, the cached module, or the upstream — is the authority over
    // which generation is served. A rootless request resolves to the chain tip; an
    // explicit root must equal it. This is the same pin the CLI clone/pull enforce,
    // now uniform across the node read path (a compromised upstream can no longer
    // choose the served generation).
    let store_id_arr = match parse_store_id_arg(&params) {
        Ok(b) => b.into(),
        Err(()) => {
            return err(
                &id,
                -32602,
                "params.store_id must be a 32-byte (64-hex) launcher id".into(),
            )
        }
    };
    // A concrete, valid requested root (non-empty, 64-hex). The `"latest"`
    // sentinel and any malformed value are treated as ROOTLESS (resolve the tip).
    let requested_root = Bytes32::from_hex(requested_root_hex).ok();
    let pinned_root: Option<Bytes32> = if pin_enforced() {
        let anchored = node
            .anchored_root_resolver
            .anchored_root(&store_id_arr)
            .await;
        match decide_pin(true, requested_root, anchored) {
            PinDecision::ServeAt(root) => Some(root),
            PinDecision::Reject(code, msg) => return err(&id, code, msg),
            // `decide_pin(true, ..)` never returns Unpinned.
            PinDecision::Unpinned => requested_root,
        }
    } else {
        // Pin disabled (DIG_NODE_PIN=off, offline/local dev): serve against the
        // requested root as-is; the client still verifies against its trust root.
        requested_root
    };

    // The concrete root hash everything below serves against. With the pin on this
    // is the chain-anchored tip; with it off it is the requested root (or empty).
    let root_hex = pinned_root
        .map(|r| r.to_hex())
        .unwrap_or_else(|| requested_root_hex.to_string());

    // Tag the result with where it was served from so the browser can show a
    // "local" chip: "local" = from this device's cache (a compiled module or a
    // previously-cached window), "remote" = freshly fetched from rpc.dig.net.
    let local = |id: &Value, mut result: Value| -> Value {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("source".into(), json!("local"));
        }
        json!({"jsonrpc":"2.0","id":id,"result":result})
    };

    // 1. LOCAL-FIRST: serve from a cached compiled module (no network at all). The
    //    served module's own root MUST equal the pinned chain-anchored root — a
    //    cached module whose generation is not the anchored tip is rejected (it is
    //    not served as if current).
    if let (Ok(rk), false) = (decode_rk(rk_hex), root_hex.is_empty()) {
        if let Some(resp) = node.serve_local(store_hex, &root_hex, &rk) {
            if let Some(pin) = pinned_root {
                if resp.roothash != pin {
                    return err(
                        &id,
                        ROOT_NOT_ANCHORED,
                        format!(
                            "served module root {} does not match the store's on-chain root {} (chain is the authority)",
                            resp.roothash.to_hex(),
                            pin.to_hex()
                        ),
                    );
                }
            }
            return local(&id, build_result(&resp, offset));
        }
        // 1b. AUTHENTICATED WHOLE-STORE SYNC (§21.9): on a module-cache miss, pull
        //     the whole `.dig` from rpc.dig.net's auth-gated §21 endpoint, cache
        //     it, then serve locally. Best-effort — a failed/disabled sync just
        //     falls through to the per-resource proxy below. `sync_module` returns
        //     true only when the SERVED root == the requested (= pinned) root, so a
        //     synced module is keyed by the anchored root before we serve it.
        if node.sync_module(store_hex, &root_hex).await {
            if let Some(resp) = node.serve_local(store_hex, &root_hex, &rk) {
                if pinned_root.map(|p| resp.roothash == p).unwrap_or(true) {
                    return local(&id, build_result(&resp, offset));
                }
            }
        }
    }

    // 2. RESPONSE CACHE: a window we previously proxied for this exact request.
    //    Keyed by the PINNED root, so a window cached for a stale/mismatched root
    //    is never replayed for the anchored read.
    let key = response_key(store_hex, &root_hex, rk_hex, offset);
    if let Some(result) = node.serve_cached_response(&key) {
        return local(&id, result);
    }

    // 2b. P2P REDIRECT-ON-MISS (#165): this node does NOT hold the content locally. If it runs a P2P
    //     content engine (the standalone peer network — never the in-process FFI/browser path) and the
    //     DHT locates a holder, answer with a REDIRECT to that holder (default) or FETCH-THROUGH via
    //     dig-download (`DIG_NODE_ON_MISS=fetch`) instead of dead-ending — never a silent miss while a
    //     provider exists. A bounded `redirect_depth` (echoed by the caller) prevents redirect loops.
    //     Applies only to a concrete resource (store+root+retrieval_key); an empty engine or no
    //     provider falls through to the upstream proxy below (byte-identical to before).
    if let Some(content) = download::miss_content_for(store_hex, &root_hex, rk_hex) {
        let depth = download::redirect_depth(&params);
        let pin_hex = pinned_root.map(|r| r.to_hex());
        if let Some(envelope) = node
            .content_miss_envelope(&id, &content, depth, offset, pin_hex.as_deref())
            .await
        {
            return envelope;
        }
    }

    // 3. MISS: proxy to rpc.dig.net, then cache the result window (LRU-capped)
    //    so the next load of this resource is served locally. (rpc.dig.net is the
    //    remote DIG network, not a local server — the in-process node IS local.)
    //
    //    The upstream request is pinned to the anchored root (rewriting/forcing
    //    `params.root`), and the upstream-returned root is re-checked against the
    //    pin — so even on the proxy path the node never serves a generation the
    //    chain did not confirm.
    let upstream_req = pinned_root
        .map(|pin| pin_request_root(&req, &pin.to_hex()))
        .unwrap_or_else(|| req.clone());
    match node.proxy(&upstream_req).await {
        Ok(mut v) => {
            // Verify the upstream served the pinned root before trusting/caching it.
            if let Some(pin) = pinned_root {
                let served = v
                    .get("result")
                    .and_then(|r| r.get("root"))
                    .and_then(|r| r.as_str())
                    .and_then(|s| Bytes32::from_hex(s).ok());
                if let Some(served) = served {
                    if served != pin {
                        return err(
                            &id,
                            ROOT_NOT_ANCHORED,
                            format!(
                                "upstream served root {} does not match the store's on-chain root {} (chain is the authority)",
                                served.to_hex(),
                                pin.to_hex()
                            ),
                        );
                    }
                }
            }
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

/// Return a clone of the JSON-RPC `req` with `params.root` forced to `root_hex`
/// (the pinned chain-anchored root). Used so a proxied `dig.getContent` asks the
/// upstream for the chain-anchored generation, never the caller's (possibly
/// rootless or stale) root.
fn pin_request_root(req: &Value, root_hex: &str) -> Value {
    let mut out = req.clone();
    if let Some(obj) = out.as_object_mut() {
        let params = obj.entry("params").or_insert_with(|| json!({}));
        if let Some(p) = params.as_object_mut() {
            p.insert("root".into(), json!(root_hex));
        }
    }
    out
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
            anchored_root_resolver: default_anchored_resolver(),
            peer_status: peer::PeerStatus::new(),
            p2p_content: OnceLock::new(),
        })
    }

    /// The shared peer-network status (for the standalone `run` to hand to the peer-network task and
    /// for `control.peerStatus`).
    pub fn peer_status(&self) -> Arc<peer::PeerStatus> {
        self.peer_status.clone()
    }

    /// The node's persistent identity seed, if configured — the source of the STABLE mTLS `peer_id`
    /// for the L7 peer network (see [`peer::identity_from_seed`]). `None` disables the peer network
    /// (the node still serves the HTTP read path).
    pub fn identity_seed_for_peer(&self) -> Option<[u8; 32]> {
        self.identity_seed
    }

    /// The directory the L7 peer network keeps its TLS cert/key + peer address book under (a
    /// `peer-net/` subdir of the cache dir, so it shares the node's data root + writability handling).
    pub fn peer_cert_dir(&self) -> PathBuf {
        self.cache_dir.join("peer-net")
    }

    /// The node's cache dir root — the data root the P2P content engine's download staging
    /// (`<cache>/downloads`) + `.download.tmp` GC live under (shares the node's writability handling).
    pub fn cache_dir_path(&self) -> &Path {
        &self.cache_dir
    }
}

/// Run the DIG node as a standalone loopback server (the `dig-node` binary only —
/// the browser does NOT use this; it calls [`handle_rpc`] in-process via the
/// `dig-runtime` FFI). Binds 127.0.0.1:`DIG_NODE_PORT` (default 9778).
///
/// On startup it also brings up the **L7 peer network** (PHASE-2b, #162): dig-gossip's connected peer
/// pool (introducer-backed auto-discovery via `relay.dig.net`) + the mTLS peer-RPC server, so nodes
/// across machines discover + connect over the relay, maintain a peer pool, and serve/issue the L7
/// peer RPC. Disable with `DIG_PEER_NETWORK=off` (or the relay alone with `DIG_RELAY_URL=off`). The
/// peer network runs in the standalone binary ONLY; the in-process FFI path (a pure consumer) never
/// opens one, so the byte-exact §21/FFI contract is unchanged.
pub async fn run() {
    let node = Node::from_env();

    // Bring up the L7 peer network (pool + discovery + mTLS peer-RPC server) unless opted out. This
    // replaces the retired bespoke relay client: the relay reservation now lives inside
    // dig-nat/dig-gossip and is surfaced through the pool status. Best-effort — a failed bring-up
    // logs + leaves `control.peerStatus` reporting not-running; the HTTP read path below still serves.
    if peer::peer_network_enabled() {
        peer::spawn_peer_network(node.clone());
    } else {
        println!("dig-node: L7 peer network disabled (DIG_PEER_NETWORK=off)");
    }

    let app = Router::new()
        .route("/", post(rpc))
        .route("/health", axum::routing::get(health))
        .with_state(node);

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

/// `GET /health` — a tiny liveness + peer-network probe for the standalone node (so an operator or
/// the installer can confirm the node is up and whether its L7 peer network is running).
async fn health(State(node): State<Arc<Node>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "peer_network_running": node.peer_status.is_running(),
    }))
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

    /// A deterministic [`AnchoredRootResolver`] for tests: maps each store id hex
    /// to its anchored-root resolution outcome so the read-path pin can be
    /// exercised without a live chain. `Ok(Some(root))` = a confirmed tip;
    /// `Ok(None)` = no confirmed generation; `Err(msg)` = chain unreachable.
    struct MockResolver {
        outcomes: std::collections::HashMap<String, Result<Option<Bytes32>, String>>,
    }

    impl MockResolver {
        /// One store that resolves to `root`.
        fn one(store_hex: &str, root: Bytes32) -> Arc<dyn AnchoredRootResolver> {
            let mut outcomes = std::collections::HashMap::new();
            outcomes.insert(store_hex.to_string(), Ok(Some(root)));
            Arc::new(MockResolver { outcomes })
        }
        /// A resolver whose every lookup is `outcome` (e.g. chain-unreachable).
        fn always(outcome: Result<Option<Bytes32>, String>) -> Arc<dyn AnchoredRootResolver> {
            Arc::new(MockResolver {
                outcomes: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("*".to_string(), outcome);
                    m
                },
            })
        }
    }

    #[async_trait::async_trait]
    impl AnchoredRootResolver for MockResolver {
        async fn anchored_root(&self, store_id: &[u8; 32]) -> Result<Option<Bytes32>, String> {
            let hex = hex::encode(store_id);
            self.outcomes
                .get(&hex)
                .or_else(|| self.outcomes.get("*"))
                .cloned()
                .unwrap_or(Ok(None))
        }
    }

    /// Build a `Node` with a throwaway cache dir and an optional identity seed. The
    /// returned `TempDir` must be kept alive for the duration of the test.
    ///
    /// The anchored-root resolver defaults to "no confirmed generation" for every
    /// store, so any `dig.getContent` test that does not explicitly inject a tip
    /// fails closed under the pin — make the pin policy explicit per test via
    /// [`test_node_with_resolver`] or by disabling the pin (`DIG_NODE_PIN=off`).
    fn test_node(identity_seed: Option<[u8; 32]>) -> (Node, tempfile::TempDir) {
        test_node_with_resolver(identity_seed, MockResolver::always(Ok(None)))
    }

    /// Like [`test_node`] but with an explicit anchored-root resolver (the pin's
    /// trusted-root source) so the fail-closed read-path gate can be unit-tested.
    fn test_node_with_resolver(
        identity_seed: Option<[u8; 32]>,
        anchored_root_resolver: Arc<dyn AnchoredRootResolver>,
    ) -> (Node, tempfile::TempDir) {
        let td = tempfile::tempdir().unwrap();
        let node = Node {
            cache_dir: td.path().to_path_buf(),
            http: reqwest::Client::new(),
            // Default to an UNROUTABLE upstream so a proxy fallback fails fast and
            // hermetically (no live rpc.dig.net). Tests needing a real upstream set
            // `node.upstream` explicitly (e.g. fetch_and_cache_*).
            upstream: "http://127.0.0.1:1/".to_string(),
            cache_lock: Mutex::new(()),
            identity_seed,
            anchored_root_resolver,
            peer_status: peer::PeerStatus::new(),
            p2p_content: OnceLock::new(),
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

    // -- #39 public collection reads (param validation + pagination, no chain) --
    //
    // These exercise dig.getCollection / dig.listCollectionItems through the real
    // handle_rpc router WITHOUT touching the network: a bad/empty launcher_ids list
    // is handled before any coinset read (an empty set resolves to zero items
    // immediately), so the dispatch, param parsing, and pagination math are verified
    // offline. (The lineage resolution itself is proven on the in-process Chia
    // simulator in digstore_chain::collection_index.)

    #[tokio::test]
    async fn list_collection_items_rejects_missing_launcher_ids() {
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":3,"method":"dig.listCollectionItems","params":{}}),
        )
        .await;
        assert_eq!(resp["id"], json!(3));
        assert_eq!(resp["error"]["code"], json!(-32602));
        assert!(resp.get("result").is_none());
    }

    #[tokio::test]
    async fn list_collection_items_rejects_non_hex_launcher_id() {
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":4,"method":"dig.listCollectionItems",
                   "params":{"launcher_ids":["nope"]}}),
        )
        .await;
        assert_eq!(resp["error"]["code"], json!(-32602));
    }

    #[tokio::test]
    async fn list_collection_items_empty_set_is_a_deterministic_empty_page() {
        // An empty item set resolves to an empty page with no chain reads, and the
        // pagination envelope (offset/limit/total/next_offset) is well-formed.
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":5,"method":"dig.listCollectionItems",
                   "params":{"launcher_ids":[], "offset":0, "limit":10}}),
        )
        .await;
        let result = &resp["result"];
        assert_eq!(result["items"], json!([]));
        assert_eq!(result["total"], json!(0));
        assert_eq!(result["offset"], json!(0));
        assert_eq!(result["limit"], json!(10));
        assert_eq!(
            result["next_offset"],
            Value::Null,
            "no next page past an empty set"
        );
    }

    #[tokio::test]
    async fn list_collection_items_caps_limit_at_200() {
        // A caller-supplied limit above the 200 cap is clamped (so one call can't
        // fan out unbounded chain reads); with an empty set the page is still empty.
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":6,"method":"dig.listCollectionItems",
                   "params":{"launcher_ids":[], "limit":100000}}),
        )
        .await;
        assert_eq!(resp["result"]["limit"], json!(200), "limit clamped to 200");
    }

    #[tokio::test]
    async fn get_collection_empty_set_resolves_to_zero_items() {
        // dig.getCollection over an empty set: zero resolved items, no uniform DID or
        // royalty, the declared DID echoed back, item_count == requested length.
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":8,"method":"dig.getCollection",
                   "params":{"launcher_ids":[], "did":"ab".repeat(32)}}),
        )
        .await;
        let result = &resp["result"];
        assert_eq!(result["item_count"], json!(0));
        assert_eq!(result["resolved_count"], json!(0));
        assert_eq!(result["did"], Value::Null);
        assert_eq!(result["declared_did"], json!("ab".repeat(32)));
        assert_eq!(result["royalty_basis_points"], Value::Null);
    }

    #[tokio::test]
    async fn get_collection_rejects_bad_launcher_ids() {
        let (node, _td) = test_node(None);
        let resp = handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":9,"method":"dig.getCollection",
                   "params":{"launcher_ids":"not-an-array"}}),
        )
        .await;
        assert_eq!(resp["error"]["code"], json!(-32602));
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        // content_address is the chia:// open address for the capsule.
        assert_eq!(
            r["content_address"].as_str(),
            Some(format!("chia://{store_hex}:{root_hex}/").as_str())
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
    // threads of one process. `ENV_GUARD` serializes them. Acquire it with
    // `.unwrap_or_else(|p| p.into_inner())` so that ONE test's failure (which
    // poisons the mutex) does not cascade into spurious failures of every other
    // env-touching test — each failure should stand on its own.
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
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

    #[test]
    fn control_peer_status_reports_not_running_by_default() {
        // The peer-status RPC is read-only and safe with NO peer network running (the in-process FFI
        // path): it reports `running:false` + the resolved relay endpoint + network id.
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_RELAY_URL");
        std::env::remove_var("DIG_NETWORK_ID");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (node, _td) = test_node(None);
        let got = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":7,"method":"control.peerStatus"}),
        ));
        let result = got["result"].as_object().expect("result object");
        assert_eq!(result["running"], json!(false));
        assert_eq!(
            result["relay"]["url"],
            json!(peer::DEFAULT_RELAY_URL),
            "defaults to relay.dig.net when DIG_RELAY_URL unset"
        );
        assert_eq!(result["network_id"], json!(peer::DEFAULT_NETWORK_ID));
        assert_eq!(result["connected_peers"], json!(0));
        assert_eq!(got["id"], json!(7));
        assert_eq!(got["jsonrpc"], json!("2.0"));
    }

    // -- #127 MANDATORY anchored-root pin on the read path ----------------------
    //
    // Every `dig.getContent` resolves the store's CHIP-0035 chain-anchored TIP
    // root and serves against IT, or fails closed with `ROOT_NOT_ANCHORED`
    // (-32005). A compromised upstream/host can never choose which generation is
    // served; a rootless URN resolves to the chain tip; an explicit root must
    // equal the tip. These tests pin the policy (pure `decide_pin`) and the
    // fail-closed read-path behavior (end-to-end through `handle_rpc`).

    #[test]
    fn decide_pin_serves_the_tip_for_a_rootless_request() {
        // Rootless (no requested root) → serve at the resolved chain tip.
        let tip = Bytes32([0xAA; 32]);
        match decide_pin(true, None, Ok(Some(tip))) {
            PinDecision::ServeAt(root) => assert_eq!(root, tip),
            _ => panic!("rootless under a confirmed tip must ServeAt the tip"),
        }
    }

    #[test]
    fn decide_pin_serves_when_explicit_root_matches_the_tip() {
        let tip = Bytes32([0xAA; 32]);
        match decide_pin(true, Some(tip), Ok(Some(tip))) {
            PinDecision::ServeAt(root) => assert_eq!(root, tip),
            _ => panic!("explicit root == tip must ServeAt"),
        }
    }

    #[test]
    fn decide_pin_rejects_when_explicit_root_differs_from_the_tip() {
        let tip = Bytes32([0xAA; 32]);
        let other = Bytes32([0xBB; 32]);
        match decide_pin(true, Some(other), Ok(Some(tip))) {
            PinDecision::Reject(code, msg) => {
                assert_eq!(code, ROOT_NOT_ANCHORED);
                assert!(msg.contains("chain is the authority"), "{msg}");
            }
            _ => panic!("explicit root != tip must fail closed"),
        }
    }

    #[test]
    fn decide_pin_fails_closed_when_chain_unreachable() {
        match decide_pin(true, None, Err("coinset down".into())) {
            PinDecision::Reject(code, _) => assert_eq!(code, ROOT_NOT_ANCHORED),
            _ => panic!("unreachable chain must fail closed, never serve"),
        }
    }

    #[test]
    fn decide_pin_fails_closed_when_no_confirmed_generation() {
        match decide_pin(true, None, Ok(None)) {
            PinDecision::Reject(code, _) => assert_eq!(code, ROOT_NOT_ANCHORED),
            _ => panic!("no confirmed generation must fail closed"),
        }
    }

    #[test]
    fn decide_pin_is_unpinned_only_when_enforcement_is_off() {
        let other = Bytes32([0xBB; 32]);
        // Even a mismatch is allowed through when the pin is explicitly disabled.
        match decide_pin(false, Some(other), Ok(Some(Bytes32([0xAA; 32])))) {
            PinDecision::Unpinned => {}
            _ => panic!("pin off → Unpinned regardless of mismatch"),
        }
    }

    #[test]
    fn pin_enforced_is_default_on_and_off_only_for_explicit_opt_out() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        assert!(pin_enforced(), "default (unset) → ENFORCED");
        for off in ["off", "0", "false"] {
            std::env::set_var("DIG_NODE_PIN", off);
            assert!(!pin_enforced(), "DIG_NODE_PIN={off} → disabled");
        }
        std::env::set_var("DIG_NODE_PIN", "on");
        assert!(pin_enforced(), "any non-opt-out value → ENFORCED");
        std::env::remove_var("DIG_NODE_PIN");
    }

    /// A valid 32-byte retrieval key hex (so the request reaches the serve path,
    /// not a -32602 param rejection) — content is never actually served in the
    /// fail-closed tests because the pin rejects first.
    fn any_rk_hex() -> String {
        "cd".repeat(32)
    }

    /// A current-thread runtime for the env-mutating pin tests. These hold the
    /// std `ENV_GUARD` (so the process-global `DIG_NODE_PIN` is stable for the
    /// test) and must NOT hold it across an `.await` (clippy `await_holding_lock`),
    /// so they are plain `#[test]` fns driving the async dispatch via `block_on` —
    /// the same pattern the cache.* env tests use.
    fn pin_test_rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn get_content_rejects_explicit_root_that_is_not_the_anchored_root() {
        // The classic #127 attack: a caller (or a compromised resolver upstream)
        // asks for a specific generation that is NOT the chain tip. The node MUST
        // refuse rather than serve the attacker-chosen generation.
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        let rt = pin_test_rt();
        let store = Bytes32([1u8; 32]);
        let tip = Bytes32([0xAA; 32]);
        let attacker_root = Bytes32([0xBB; 32]);
        let (node, _td) = test_node_with_resolver(None, MockResolver::one(&store.to_hex(), tip));

        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.getContent","params":{
                "store_id": store.to_hex(),
                "root": attacker_root.to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));

        assert_eq!(
            resp["error"]["code"], ROOT_NOT_ANCHORED,
            "a non-anchored explicit root must fail closed: {resp}"
        );
        assert!(resp.get("result").is_none(), "no content served: {resp}");
    }

    #[test]
    fn get_content_fails_closed_when_chain_is_unreachable() {
        // The chain (the authority) cannot be reached → the node must NOT fall back
        // to serving an unverified root; it fails closed.
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        let rt = pin_test_rt();
        let store = Bytes32([2u8; 32]);
        let (node, _td) =
            test_node_with_resolver(None, MockResolver::always(Err("coinset 503".into())));

        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":2,"method":"dig.getContent","params":{
                "store_id": store.to_hex(),
                "root": Bytes32([0xAA; 32]).to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));

        assert_eq!(resp["error"]["code"], ROOT_NOT_ANCHORED, "{resp}");
        assert!(resp.get("result").is_none());
    }

    #[test]
    fn get_content_fails_closed_when_store_has_no_confirmed_generation() {
        // A store with no confirmed on-chain generation has no anchored root to pin
        // to → fail closed (never serve a forgeable/unanchored generation).
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        let rt = pin_test_rt();
        let store = Bytes32([3u8; 32]);
        let (node, _td) = test_node_with_resolver(None, MockResolver::always(Ok(None)));

        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":3,"method":"dig.getContent","params":{
                "store_id": store.to_hex(),
                "root": Bytes32([0xAA; 32]).to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));

        assert_eq!(resp["error"]["code"], ROOT_NOT_ANCHORED, "{resp}");
    }

    #[test]
    fn get_content_rejects_a_bad_store_id_before_touching_the_chain() {
        // Param validation precedes the chain read (a -32602, not a pin error).
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        let rt = pin_test_rt();
        let (node, _td) = test_node(None);
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":4,"method":"dig.getContent","params":{
                "store_id": "nope",
                "root": Bytes32([0xAA; 32]).to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));
        assert_eq!(resp["error"]["code"], json!(-32602), "{resp}");
    }

    /// Stage a real `.dig` module from `files` for `store`, returning its root and
    /// the on-disk module bytes — used to seed the local cache for a serve test.
    fn stage_real_module(
        node: &Node,
        store: &Bytes32,
        files: &[(&str, &[u8])],
    ) -> (Bytes32, Vec<u8>) {
        let src = tempfile::tempdir().unwrap();
        for (name, bytes) in files {
            let p = src.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
        }
        let resp = node.stage(
            &json!({"dir": src.path().display().to_string(), "store_id": store.to_hex()}),
            json!(1),
        );
        let r = &resp["result"];
        let root = Bytes32::from_hex(r["root"].as_str().expect("root")).unwrap();
        let module = std::fs::read(r["module_path"].as_str().expect("module_path")).unwrap();
        (root, module)
    }

    #[test]
    fn get_content_does_not_serve_a_cached_stale_generation_as_current() {
        // Defense in depth: a module for an OLD generation (root R) is in the local
        // cache, but the chain tip has advanced to R'. A read pinned to R' must NOT
        // serve the cached R module — the cache key is the anchored root, so the
        // stale module is simply not found at R', and the read does not return it.
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        let rt = pin_test_rt();
        // Upstream is unroutable (test_node default) → after the local miss the read
        // falls through to a proxy attempt that errors out (no fabricated content).
        let store = Bytes32([7u8; 32]);
        let advanced_tip = Bytes32([0x99; 32]); // R' — what the chain says is current
        let (node, _td) =
            test_node_with_resolver(None, MockResolver::one(&store.to_hex(), advanced_tip));

        // Seed a real cached module at its REAL (old) root R != R'.
        let (old_root, module) =
            stage_real_module(&node, &store, &[("index.html", b"<h1>old</h1>")]);
        assert_ne!(old_root, advanced_tip, "the cached generation is stale");
        let seeded = module_path(&node.cache_dir, &store.to_hex(), &old_root.to_hex());
        std::fs::create_dir_all(seeded.parent().unwrap()).unwrap();
        std::fs::write(&seeded, &module).unwrap();

        // Request the (advanced) tip generation. The pin serves at R'; the stale R
        // module is at a different cache key, so serve_local misses and the node
        // never returns the old generation's content. With no upstream it errors —
        // crucially NOT a success carrying the stale module.
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":5,"method":"dig.getContent","params":{
                "store_id": store.to_hex(),
                "root": advanced_tip.to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));
        // It must not have served the stale cached module as the current generation.
        let served_local = resp["result"]["source"].as_str() == Some("local");
        assert!(
            !served_local,
            "a stale cached generation must never be served as the anchored tip: {resp}"
        );
    }

    #[test]
    fn get_content_unpinned_mode_serves_the_requested_root_as_before() {
        // With the pin explicitly disabled (offline/local dev), the node serves the
        // requested root as-is (legacy behavior) — the resolver is never consulted.
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("DIG_NODE_PIN", "off");
        let rt = pin_test_rt();
        let store = Bytes32([8u8; 32]);
        // A resolver that would FAIL if consulted — proving the unpinned path skips it.
        let (node, _td) =
            test_node_with_resolver(None, MockResolver::always(Err("must not be called".into())));

        // No module cached, unroutable upstream → the call reaches the proxy and
        // errors, but crucially it is an UPSTREAM error (-32000), NOT a pin rejection.
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":6,"method":"dig.getContent","params":{
                "store_id": store.to_hex(),
                "root": Bytes32([0xAA; 32]).to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));
        std::env::remove_var("DIG_NODE_PIN");
        assert_ne!(
            resp["error"]["code"], ROOT_NOT_ANCHORED,
            "pin off → no pin rejection: {resp}"
        );
    }

    #[test]
    fn pin_request_root_forces_params_root() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"dig.getContent",
            "params":{"store_id":"aa","root":"old","retrieval_key":"rk"}});
        let pinned = pin_request_root(&req, "newroot");
        assert_eq!(pinned["params"]["root"], json!("newroot"));
        // Other params are preserved.
        assert_eq!(pinned["params"]["store_id"], json!("aa"));
        assert_eq!(pinned["params"]["retrieval_key"], json!("rk"));
    }

    // -- #126 honest read-path: real inclusion proof + chain root, NO mock proof --
    //
    // The dig-node read path must never present a forgeable/mock proof AS verified.
    // On `dig.getContent` the trust-bearing fields are REAL — the guest-computed
    // merkle inclusion proof + the chain-anchored root (#127) — and there is no
    // execution attestation on the wire to fake: `ContentResponse`/`build_result`
    // carry no execution-proof field, and the node does not implement
    // `dig.getProof` (it returns -32601 rather than a fabricated mock receipt). A
    // real, verified execution attestation is gated on the RISC0 toolchain
    // (SECURITY.md residual #3) and is honestly absent here, never faked.

    #[test]
    fn get_content_result_carries_real_inclusion_proof_and_no_execution_proof() {
        use digstore_core::wire::ContentResponse;
        // A minimal real ContentResponse: a single-leaf merkle proof rooted at a
        // concrete root (the shape the guest serves). build_result renders it.
        let root = Bytes32([0x42; 32]);
        let resp = ContentResponse {
            ciphertext: vec![1, 2, 3, 4],
            merkle_proof: digstore_core::merkle::MerkleProof {
                leaf: root,
                path: Vec::new(),
                root,
            },
            roothash: root,
            chunk_lens: vec![4],
        };
        let result = build_result(&resp, 0);

        // The REAL inclusion proof + chain-verifiable root are present.
        assert!(
            result.get("inclusion_proof").is_some(),
            "real merkle inclusion proof is on the wire: {result}"
        );
        assert_eq!(
            result["root"].as_str(),
            Some(root.to_hex().as_str()),
            "the served root is reported (chain-pinned by #127): {result}"
        );
        // NO execution-attestation field is fabricated — the node never reports a
        // mock/absent execution proof AS a verified attestation (#126/#134).
        for forbidden in [
            "execution_proof",
            "execution_proof_status",
            "attestation",
            "proof_status",
            "receipt",
            "trusted",
        ] {
            assert!(
                result.get(forbidden).is_none(),
                "dig.getContent must not carry a (mock) `{forbidden}` field: {result}"
            );
        }
    }

    #[test]
    fn get_proof_is_not_served_as_a_verified_proof_by_the_node() {
        // dig-node does not implement dig.getProof — it returns the catalogued
        // -32601 (method not found) rather than fabricating a mock execution
        // proof. (The standalone node has no upstream here, so the dispatch's own
        // method-not-found is observed directly.)
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        let rt = pin_test_rt();
        let (node, _td) = test_node(None);
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":9,"method":"dig.getProof","params":{
                "store_id": Bytes32([1u8; 32]).to_hex(),
                "retrieval_key": any_rk_hex(),
            }}),
        ));
        assert_eq!(
            resp["error"]["code"],
            json!(-32601),
            "dig.getProof must be method-not-found on the node, never a fabricated proof: {resp}"
        );
        assert!(
            resp.get("result").is_none(),
            "no proof result is fabricated: {resp}"
        );
    }

    // -- REDIRECT-ON-MISS (#165) — the content-orchestration miss handler wired into the RPC ----------
    //
    // These drive the REAL `dig.getContent` / `dig.fetchRange` dispatch on a node that does NOT hold the
    // requested resource but has a P2P content engine attached (the standalone peer path). With a mock
    // DHT locator + mock range transport (dig-download's testkit — no real network) they assert: a
    // holder exists → REDIRECT (not not-found); no holder → proper not-found; the hop cap is honored;
    // and `DIG_NODE_ON_MISS=fetch` fetches-through and serves the bytes. The pin resolver returns the
    // tip so the read gets past the anchored-root gate into the miss path.

    use crate::download::{MissMode, NodeContent, CONTENT_REDIRECT, REDIRECT_HOP_CAP};
    use dig_download::ContentId;

    /// A `MockContent` whose `root`/`inclusion_proof` are a REAL digstore merkle proof over its bytes,
    /// so the chain-binding `DigstoreProofVerifier` (and the download's whole-resource verify) pass for
    /// honest bytes — the same construction `download::tests::anchored_mock_content` uses.
    fn anchored_mock_content(n: usize, chunks: usize) -> dig_download::testkit::MockContent {
        use digstore_core::codec::Encode;
        let mut content = dig_download::testkit::MockContent::even(n, chunks);
        let leaf = digstore_core::resource_leaf(&content.bytes);
        let tree = digstore_core::MerkleTree::from_leaves(vec![leaf]);
        let proof = tree.prove(0).expect("single-leaf proof");
        content.root = tree.root().to_hex();
        content.inclusion_proof =
            Some(base64::engine::general_purpose::STANDARD.encode(Encode::to_bytes(&proof)));
        content
    }

    /// Attach a P2P content engine to `node` with a mock locator (the given providers) + a mock
    /// transport serving `content`, in `mode`. Returns nothing — the engine lives on the node.
    fn attach_p2p(
        node: &Node,
        providers: Vec<dig_download::ProviderRecord>,
        content: dig_download::testkit::MockContent,
        mode: MissMode,
        td: &tempfile::TempDir,
    ) {
        let locator = Arc::new(dig_download::testkit::MockProviderLocator::fixed(providers));
        let transport = Arc::new(dig_download::testkit::MockRangeTransport::new(content));
        let pc = NodeContent::new(locator, transport, mode, None, td.path());
        node.set_p2p_content(pc);
    }

    /// A store + its chain tip, with a request that resolves past the pin into the miss path.
    fn miss_setup() -> (Bytes32, Bytes32, String) {
        (Bytes32([0x21; 32]), Bytes32([0x22; 32]), any_rk_hex())
    }

    #[test]
    fn get_content_miss_with_a_provider_redirects_not_notfound() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        std::env::remove_var("DIG_NODE_ON_MISS");
        let rt = pin_test_rt();
        let (store, tip, rk) = miss_setup();
        let (node, td) = test_node_with_resolver(None, MockResolver::one(&store.to_hex(), tip));
        // A holder exists in the DHT for this content.
        let cid = ContentId::resource(store.0, tip.0, [0xcd; 32]);
        attach_p2p(
            &node,
            vec![dig_download::testkit::mock_provider(3, &cid)],
            dig_download::testkit::MockContent::even(10, 1),
            MissMode::Redirect,
            &td,
        );
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.getContent","params":{
                "store_id": store.to_hex(), "root": tip.to_hex(), "retrieval_key": rk,
            }}),
        ));
        // Not held locally, but a provider exists → a REDIRECT (never a silent miss/upstream error).
        assert_eq!(
            resp["error"]["code"],
            json!(CONTENT_REDIRECT),
            "expected redirect: {resp}"
        );
        let redirect = &resp["error"]["data"]["redirect"];
        assert_eq!(
            redirect["providers"][0]["peer_id"],
            json!(dig_download::testkit::mock_peer_hex(3))
        );
        assert_eq!(redirect["redirect_depth"], json!(1), "depth advanced 0 → 1");
        assert_eq!(redirect["max_redirects"], json!(REDIRECT_HOP_CAP));
        assert_eq!(redirect["content"]["store_id"], json!(store.to_hex()));
    }

    #[test]
    fn get_content_miss_with_no_provider_is_notfound_not_redirect() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        std::env::remove_var("DIG_NODE_ON_MISS");
        let rt = pin_test_rt();
        let (store, tip, rk) = miss_setup();
        let (node, td) = test_node_with_resolver(None, MockResolver::one(&store.to_hex(), tip));
        // NO provider in the DHT for this content.
        attach_p2p(
            &node,
            vec![],
            dig_download::testkit::MockContent::even(10, 1),
            MissMode::Redirect,
            &td,
        );
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.getContent","params":{
                "store_id": store.to_hex(), "root": tip.to_hex(), "retrieval_key": rk,
            }}),
        ));
        // No provider anywhere → NOT a redirect. The engine yields None and the request falls through
        // to the upstream proxy, which (unroutable in tests) returns a -32000 upstream error, never a
        // -32008 redirect.
        assert_ne!(
            resp["error"]["code"],
            json!(CONTENT_REDIRECT),
            "no provider must NOT redirect: {resp}"
        );
    }

    #[test]
    fn get_content_miss_honors_the_redirect_hop_cap() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        std::env::remove_var("DIG_NODE_ON_MISS");
        let rt = pin_test_rt();
        let (store, tip, rk) = miss_setup();
        let (node, td) = test_node_with_resolver(None, MockResolver::one(&store.to_hex(), tip));
        let cid = ContentId::resource(store.0, tip.0, [0xcd; 32]);
        attach_p2p(
            &node,
            vec![dig_download::testkit::mock_provider(3, &cid)],
            dig_download::testkit::MockContent::even(10, 1),
            MissMode::Redirect,
            &td,
        );
        // A request already redirected up to the cap → NO further redirect (loop guard), even though a
        // provider exists.
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":1,"method":"dig.getContent","params":{
                "store_id": store.to_hex(), "root": tip.to_hex(), "retrieval_key": rk,
                "redirect_depth": REDIRECT_HOP_CAP,
            }}),
        ));
        assert_ne!(
            resp["error"]["code"],
            json!(CONTENT_REDIRECT),
            "at the hop cap the node must not redirect again: {resp}"
        );
    }

    #[test]
    fn fetch_range_miss_with_a_provider_redirects() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        std::env::remove_var("DIG_NODE_ON_MISS");
        let rt = pin_test_rt();
        let (store, tip, rk) = miss_setup();
        let (node, td) = test_node(None);
        let cid = ContentId::resource(store.0, tip.0, [0xcd; 32]);
        attach_p2p(
            &node,
            vec![dig_download::testkit::mock_provider(5, &cid)],
            dig_download::testkit::MockContent::even(10, 1),
            MissMode::Redirect,
            &td,
        );
        // dig.fetchRange for a resource the node does not hold → redirect (fetchRange has no pin gate).
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":7,"method":"dig.fetchRange","params":{
                "store_id": store.to_hex(), "root": tip.to_hex(), "retrieval_key": rk,
                "length": 4096, "offset": 0,
            }}),
        ));
        assert_eq!(
            resp["error"]["code"],
            json!(CONTENT_REDIRECT),
            "fetchRange miss → redirect: {resp}"
        );
        assert_eq!(
            resp["error"]["data"]["redirect"]["providers"][0]["peer_id"],
            json!(dig_download::testkit::mock_peer_hex(5))
        );
    }

    #[test]
    fn fetch_through_pulls_from_the_holder_and_serves_the_bytes() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("DIG_NODE_PIN");
        std::env::remove_var("DIG_NODE_ON_MISS");
        let rt = pin_test_rt();
        let (store, tip, rk) = miss_setup();
        let (node, td) = test_node(None);
        // A holder serves an ANCHORED resource (real digstore proof over its bytes) so the download's
        // whole-resource verify against the chain-anchored root passes.
        let content = anchored_mock_content(30, 3);
        let cid = dig_download::testkit::mock_content_id();
        attach_p2p(
            &node,
            vec![
                dig_download::testkit::mock_provider(1, &cid),
                dig_download::testkit::mock_provider(2, &cid),
            ],
            content.clone(),
            MissMode::FetchThrough,
            &td,
        );
        // fetch-through: the node pulls the resource from the holders and serves it directly. The
        // request's content id must be the mock content id the holders serve.
        let (store_hex, tip_hex, rk_hex) = match &cid {
            ContentId::Resource {
                store_id,
                root,
                retrieval_key,
            } => (
                hex::encode(store_id),
                hex::encode(root),
                hex::encode(retrieval_key),
            ),
            _ => unreachable!("mock_content_id is a resource"),
        };
        let _ = (store, tip, rk);
        let resp = rt.block_on(handle_rpc(
            &node,
            json!({"jsonrpc":"2.0","id":9,"method":"dig.fetchRange","params":{
                "store_id": store_hex, "root": tip_hex, "retrieval_key": rk_hex,
                "length": 4096, "offset": 0,
            }}),
        ));
        // A fetched-through frame is served (NOT a redirect, NOT a miss): the first frame carries the
        // reassembled bytes + verification metadata.
        assert!(
            resp.get("result").is_some(),
            "fetch-through serves a frame: {resp}"
        );
        let frame = &resp["result"];
        assert_eq!(frame["complete"], json!(true));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(frame["bytes"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            bytes, content.bytes,
            "fetch-through serves the holder's bytes"
        );
        assert_eq!(frame["root"], json!(content.root));
    }

    /// `dig.getNetworkInfo` must never report the wildcard bind address as a dialable endpoint, and
    /// its candidate list must be IPv6-first (ecosystem HARD RULE). The exact addresses are
    /// host-dependent (real local-address discovery), so this asserts the host-independent invariants:
    /// no `0.0.0.0` / `[::]` leaks, and any IPv4 candidate follows every IPv6 candidate.
    #[test]
    fn network_info_reports_ipv6_first_dialable_addrs_never_the_wildcard() {
        let (node, _td) = test_node(Some([5u8; 32]));
        let info = node.network_info();

        let listen = info["listen_addr"].as_str().expect("listen_addr string");
        assert!(
            !listen.starts_with("0.0.0.0:") && !listen.starts_with("[::]:"),
            "listen_addr must be a dialable address, never the wildcard bind address: {listen}"
        );

        let candidates: Vec<std::net::SocketAddr> = info["candidate_addresses"]
            .as_array()
            .expect("candidate_addresses array")
            .iter()
            .map(|v| v.as_str().unwrap().parse().expect("a socket addr"))
            .collect();
        // No wildcard address ever appears as an advertised candidate.
        for c in &candidates {
            assert!(!c.ip().is_unspecified(), "no wildcard candidate: {c}");
        }
        // IPv6-first: once an IPv4 candidate appears, no later candidate may be IPv6.
        let mut seen_ipv4 = false;
        for c in &candidates {
            if c.is_ipv4() {
                seen_ipv4 = true;
            } else {
                assert!(
                    !seen_ipv4,
                    "IPv6 candidate must not follow an IPv4 one: {candidates:?}"
                );
            }
        }
    }
}
