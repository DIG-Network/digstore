//! Pure operations behind the local store git verbs.
//!
//! ## Architecture note (DOCUMENTED DEVIATION vs the plan)
//!
//! The plan assumed (a) `StagingArea` had a `stage_resource`/`seal_generation`
//! API, (b) `digstore-store`/`digstore-compiler` encrypted chunk bodies, and
//! (c) the compiled guest module, when served by `digstore-host`, returns the
//! resource's encrypted chunk ciphertext with a merkle proof that
//! `MerkleProof::verify()` accepts against the generation root.
//!
//! The REAL crates differ:
//! - `digstore-store` chunks/stores **plaintext** and exposes a different
//!   `StagingArea` (`open`/`append`/`records`/`clear`).
//! - No crate encrypts; the compiler copies chunk bodies verbatim into the pool.
//! - The already-built `digstore-guest` reads its data section from the
//!   `__digstore_data` static stub (9-byte empty section); the compiler injects
//!   the real data at memory page 1 but never rewrites that symbol, AND the two
//!   data-section framings (compiler: u8-kind 9-byte rows; guest: u16-id 10-byte
//!   rows) are incompatible. Empirically (verified before writing this), a real
//!   compiled module served through `HostRuntime::serve_content` returns a
//!   zero-length / decoy response and `get_store_id` returns empty.
//!
//! Since this crate may NOT edit other crates, the CLI owns the client-crypto
//! contract end-to-end (CONVENTIONS C5/C9/C10: store does mechanics, CLI does
//! presentation + ALL client decryption; the module never decrypts):
//! - **commit** chunks plaintext, AES-256-GCM-seals each chunk under the
//!   resource's per-URN key (`digstore_crypto`), builds the generation merkle
//!   tree over the **ciphertext** chunk leaves (so client merkle verification of
//!   ciphertext-to-root is genuine), persists a `GenerationManifest` + ciphertext
//!   chunk bodies, appends the real generation id/root/timestamp to `roots.log`,
//!   and compiles a real `.wasm` module (so the host actually instantiates it and
//!   `module_path`/push/clone have a real artifact).
//! - **serve** (see `ops::serve`) instantiates the real `HostRuntime` over that
//!   module (real load + instantiate), then produces the authoritative
//!   `ContentResponse` from the on-disk generation (ciphertext + a real merkle
//!   proof from the generation tree). A miss yields a decoy whose proof does not
//!   chain to the trusted root. §18.4 NOTE: the host runtime returns the module's
//!   bytes VERBATIM ("neither decrypts nor inspects the payload"); the
//!   `ContentResponse` envelope DECODE that `ops::serve::serve_content` performs
//!   is a CLIENT-SIDE step in this reader (it decodes framing, it does not decrypt
//!   — the result is still ciphertext). `ops::serve::serve_content_raw` exposes
//!   the host's verbatim output to make that boundary explicit and testable.
//! - **cat/checkout** verify each chunk's merkle inclusion to the trusted root
//!   and AES-256-GCM-open it client-side (`ops::client_crypto`).
//!
//! Every cryptographic guarantee the tests assert (real AES-256-GCM tags, real
//! merkle-to-root, decoy detection, tamper detection, private-salt key change)
//! is therefore genuine.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use digstore_chunker::{chunk_slice, Chunk};
use digstore_core::{
    AuthenticationInfo, Bytes32, Bytes48, ChunkerConfig, GenerationState, MerkleTree, SecretSalt,
    StoreConfig, TrustedHostKey, Urn, Visibility, MAX_STORE_BYTES,
};
use digstore_store::{
    ChunkRef, GenerationManifest, KeyTableRecord, RootHistory, StagingArea, Store, SystemClock,
};

use crate::context::CliContext;
use crate::error::CliError;
use crate::output::{DiffEntry, LogEntry, StatusView};

/// Canonical chunker config (matches `digstore-store`'s commit defaults).
fn chunker() -> ChunkerConfig {
    ChunkerConfig {
        min_size: 16 * 1024,
        target_size: 64 * 1024,
        max_size: 256 * 1024,
        mask: (1u64 << 16) - 1,
    }
}

/// The canonical root-INDEPENDENT URN for a resource (used for both the
/// retrieval key and the AES key, matching `digstore-store`'s own convention).
/// The client must reconstruct this same URN (root dropped) when decrypting.
pub fn canonical_resource_urn(store_id: Bytes32, resource_key: &str) -> Urn {
    Urn {
        chain: "chia".to_string(),
        store_id,
        root_hash: None,
        resource_key: Some(resource_key.to_string()),
    }
}

fn salt_of(cfg: &StoreConfig) -> Option<SecretSalt> {
    match &cfg.visibility {
        Visibility::Private(s) => Some(*s),
        Visibility::Public => None,
    }
}

#[derive(Debug)]
pub struct InitResult {
    pub store_id: Bytes32,
    pub host_public_key: Bytes48,
}

pub fn init_store(
    ctx: &CliContext,
    private: bool,
    data_dir: Option<String>,
) -> Result<InitResult, CliError> {
    if ctx.config_path().exists() {
        return Err(CliError::InvalidArgument(format!(
            "store already initialized at {}",
            ctx.dig_dir.display()
        )));
    }

    // Host BLS keypair (chia AugScheme). store_id = SHA-256(public key) (§20.1).
    let seed = random_seed();
    let secret = digstore_crypto::bls::SecretKey::from_seed(&seed);
    let host_public_key = secret.public_key().to_bytes();
    let store_id = digstore_crypto::sha256(&host_public_key.0);

    let visibility = if private {
        // SecretSalt is INDEPENDENT randomness, not derived from the signing key.
        Visibility::Private(SecretSalt(random_seed()))
    } else {
        Visibility::Public
    };

    let dd = data_dir.unwrap_or_else(|| ctx.dig_dir.display().to_string());
    let cfg = StoreConfig {
        store_id,
        data_dir: dd,
        max_size: MAX_STORE_BYTES,
        visibility,
    };

    // Real store init: writes config.toml + the §4.4 directory tree + staging + roots.log.
    Store::init(cfg.clone(), SystemClock)
        .map_err(|e| CliError::Other(anyhow::anyhow!("store init: {e}")))?;

    // Persist the host signing key SEED (never embedded in modules). The BLS
    // SecretKey is not extractable, so we persist the deterministic seed and
    // reconstruct the key via `from_seed`.
    write_secret_file(&ctx.dig_dir.join("signing_key.bin"), &seed)
        .map_err(|e| CliError::Other(e.into()))?;

    // Surface SecretSalt deterministically for scripting `cat --salt`. This is
    // the private master-key material, so it is owner-only on disk.
    if let Visibility::Private(salt) = &cfg.visibility {
        write_secret_file(&ctx.salt_path(), Bytes32(salt.0).to_hex().as_bytes())
            .map_err(|e| CliError::Other(e.into()))?;
    }

    // Persist the single canonical trusted host key (the compiler reads this).
    let trusted = vec![TrustedHostKey {
        public_key: host_public_key.0,
        label: format!("dig-host-key-v1:{}", host_public_key.to_hex()),
    }];
    fs::write(
        ctx.dig_dir.join("trusted_keys.json"),
        serde_json::to_string_pretty(&serialize_keys(&trusted))
            .map_err(|e| CliError::Other(e.into()))?,
    )
    .map_err(|e| CliError::Other(e.into()))?;

    // Git convenience: ignore the workspace `.dig/` once.
    ensure_dig_gitignored(&ctx.workspace_dir);

    Ok(InitResult {
        store_id,
        host_public_key,
    })
}

/// Ensure the project's `.gitignore` ignores the `.dig/` store directory.
///
/// Only applies to the conventional layout where the store lives in a directory
/// literally named `.dig` (the default for `digstore init`); when `--dig-dir`
/// points somewhere else the caller is managing their own layout, so this is a
/// no-op. Creates `.gitignore` in the store's parent directory if it does not
/// exist, appends `.dig/` if not already present (idempotent), and preserves any
/// existing content. Best-effort: any IO error is ignored so `init` still
/// succeeds.
fn ensure_dig_gitignored(dig_dir: &Path) {
    if dig_dir.file_name().and_then(|n| n.to_str()) != Some(".dig") {
        return;
    }
    let project_root = match dig_dir.parent() {
        Some(p) => p,
        None => return,
    };
    let gitignore = project_root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let already = existing
        .lines()
        .any(|l| matches!(l.trim(), ".dig" | ".dig/"));
    if already {
        return;
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(".dig/\n");
    let _ = std::fs::write(&gitignore, updated);
}

#[derive(Debug)]
pub struct AddResult {
    pub resource_key: String,
    pub chunk_count: usize,
    pub total_size: u64,
}

pub fn add_path(ctx: &CliContext, path: &Path, key: Option<String>) -> Result<AddResult, CliError> {
    let cfg = ctx.load_config()?;
    let data = fs::read(path).map_err(|e| CliError::Other(e.into()))?;
    let resource_key = key.unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into())
    });

    // Enforce StoreConfig.max_size (§20.2).
    if cfg.max_size != 0 {
        let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
            .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
        let already: u64 = staging
            .records()
            .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
            .iter()
            .map(|r| r.content.len() as u64)
            .sum();
        let projected = already + data.len() as u64;
        if projected > cfg.max_size {
            return Err(CliError::InvalidArgument(format!(
                "staged size {} exceeds store max_size {}",
                projected, cfg.max_size
            )));
        }
        staging
            .append(&resource_key, &data)
            .map_err(|e| CliError::Other(anyhow::anyhow!("stage: {e}")))?;
    }

    let chunk_count = chunk_slice(&data, &chunker()).len().max(1);
    Ok(AddResult {
        resource_key,
        chunk_count,
        total_size: data.len() as u64,
    })
}

/// Result of an `add` invocation.
pub struct AddOutcome {
    pub staged: Vec<(String, u64)>, // (key, size) newly staged
    pub unchanged: usize,
    pub dry_run: bool,
    pub staged_bytes: u64, // total staged after this add (or projected, for dry-run)
    pub limit_bytes: u64,
}

/// Resolve `paths`/`all` and stage each file under its store-root-relative key.
pub fn add_files(
    ctx: &CliContext,
    paths: &[PathBuf],
    all: bool,
    dry_run: bool,
    key: Option<String>,
) -> Result<AddOutcome, CliError> {
    use crate::ops::walk::{self, Resolved};

    let cfg = ctx.load_config()?;
    // §2.8: add scans the resolved operating directory (op_dir), NOT a path
    // derived from the per-store dig_dir (which now lives at
    // `<workspace>/stores/<name>`). Keys are relative to op_dir.
    let root = ctx.op_dir.clone();

    let skip = ctx.workspace_dir.clone();

    // Resolve the file set.
    let mut resolved: Vec<Resolved> = Vec::new();
    if all {
        resolved = walk::resolve_all(&root, &skip);
    } else {
        for p in paths {
            let arg = p.to_string_lossy();
            walk::resolve_arg(&root, &skip, &arg, &mut resolved)
                .map_err(CliError::InvalidArgument)?;
        }
    }
    resolved.sort_by(|a, b| a.key.cmp(&b.key));
    resolved.dedup_by(|a, b| a.key == b.key);

    // --key only with exactly one file.
    if key.is_some() && resolved.len() != 1 {
        return Err(CliError::InvalidArgument(
            "--key requires exactly one file path".into(),
        ));
    }

    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let already: HashMap<String, Vec<u8>> = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
        .into_iter()
        .map(|r| (r.resource_key, r.content))
        .collect();
    let already_bytes: u64 = already.values().map(|c| c.len() as u64).sum();

    // Read all incoming, decide new vs unchanged, and pre-sum — ATOMIC cap check.
    struct Incoming {
        key: String,
        data: Vec<u8>,
    }
    let mut incoming: Vec<Incoming> = Vec::new();
    let mut unchanged = 0usize;
    for r in resolved {
        let data = fs::read(&r.path).map_err(|e| CliError::Other(e.into()))?;
        let effective_key = key.clone().unwrap_or_else(|| r.key.clone());
        if already
            .get(&effective_key)
            .map(|c| c == &data)
            .unwrap_or(false)
        {
            unchanged += 1;
            continue;
        }
        incoming.push(Incoming {
            key: effective_key,
            data,
        });
    }
    let incoming_bytes: u64 = incoming.iter().map(|i| i.data.len() as u64).sum();
    // Re-staging a key REPLACES its existing staged content (StagingArea is
    // last-write-wins), so its old bytes must not be double-counted in the cap
    // arithmetic. Subtract the old size of every incoming key already staged.
    let replaced_bytes: u64 = incoming
        .iter()
        .filter_map(|i| already.get(&i.key).map(|c| c.len() as u64))
        .sum();
    let base_bytes = already_bytes.saturating_sub(replaced_bytes);
    let cap = cap_of(cfg.max_size);
    let projected = base_bytes + incoming_bytes;
    if projected > cap {
        let store = ctx.store_name.clone().unwrap_or_else(|| "this".into());
        return Err(CliError::InvalidArgument(format!(
            "staging would reach {} MB, over the {} store's {} MB limit ({} MB free); stage fewer files or create another store (digstore init <name2>)",
            mb(projected), store, mb(cap), mb(cap.saturating_sub(base_bytes))
        )));
    }

    let mut outcome = AddOutcome {
        staged: Vec::new(),
        unchanged,
        dry_run,
        staged_bytes: projected,
        limit_bytes: cap,
    };
    if !dry_run {
        for i in incoming {
            let size = i.data.len() as u64;
            staging
                .append(&i.key, &i.data)
                .map_err(|e| CliError::Other(anyhow::anyhow!("stage: {e}")))?;
            outcome.staged.push((i.key, size));
        }
    } else {
        for i in incoming {
            outcome.staged.push((i.key.clone(), i.data.len() as u64));
        }
        // dry-run: staged_bytes is the projected total if these were applied.
    }
    Ok(outcome)
}

/// Decimal MB, one decimal place.
fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}

/// The effective per-store cap: the persisted `max_size`, or the workspace
/// default ([`MAX_STORE_BYTES`]) when it is unset (`0`).
fn cap_of(max_size: u64) -> u64 {
    if max_size == 0 {
        MAX_STORE_BYTES
    } else {
        max_size
    }
}

/// Clear the selected store's staging area; returns how many entries were dropped.
pub fn clear_staging(ctx: &CliContext) -> Result<usize, CliError> {
    let cfg = ctx.load_config()?;
    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let n = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
        .len();
    staging
        .clear()
        .map_err(|e| CliError::Other(anyhow::anyhow!("clear staging: {e}")))?;
    Ok(n)
}

/// List staged entries `(key, size)` sorted by key, plus the staged total and
/// the effective per-store cap.
#[allow(clippy::type_complexity)]
pub fn list_staged(ctx: &CliContext) -> Result<(Vec<(String, u64)>, u64, u64), CliError> {
    let cfg = ctx.load_config()?;
    let staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let mut entries: Vec<(String, u64)> = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
        .into_iter()
        .map(|r| (r.resource_key, r.content.len() as u64))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let total: u64 = entries.iter().map(|(_, s)| *s).sum();
    let cap = cap_of(cfg.max_size);
    Ok((entries, total, cap))
}

/// A previewed URN for a resource that would be staged from `op_dir`.
#[derive(Debug, Clone)]
pub struct UrnPreview {
    pub path: String,
    pub key: String,
    pub urn: String,
    pub retrieval_key: String,
}

/// Preview the URNs `add` would produce for `paths`/`all`, mirroring `add_files`
/// resolution (op_dir scope, content-root-relative keys). The display URN is
/// rootless by default; when `root_hex` is given it is root-pinned. The
/// `retrieval_key` is ALWAYS derived from the rootless canonical URN.
pub fn preview_urns(
    ctx: &CliContext,
    paths: &[PathBuf],
    all: bool,
    root_hex: Option<&str>,
) -> Result<Vec<UrnPreview>, CliError> {
    use crate::ops::walk::{self, Resolved};
    let cfg = ctx.load_config()?;
    let root = ctx.op_dir.clone();
    let skip = ctx.workspace_dir.clone();
    let mut resolved: Vec<Resolved> = Vec::new();
    if all {
        resolved = walk::resolve_all(&root, &skip);
    } else {
        for p in paths {
            walk::resolve_arg(&root, &skip, &p.to_string_lossy(), &mut resolved)
                .map_err(CliError::InvalidArgument)?;
        }
    }
    resolved.sort_by(|a, b| a.key.cmp(&b.key));
    resolved.dedup_by(|a, b| a.key == b.key);

    let pinned_root = match root_hex {
        Some(h) => Some(
            Bytes32::from_hex(h)
                .map_err(|_| CliError::InvalidArgument(format!("bad root hex: {h}")))?,
        ),
        None => None,
    };
    let mut out = Vec::new();
    for r in resolved {
        // Retrieval key is ALWAYS from the rootless canonical URN.
        let rootless = canonical_resource_urn(cfg.store_id, &r.key);
        let display = Urn {
            chain: "chia".to_string(),
            store_id: cfg.store_id,
            root_hash: pinned_root,
            resource_key: Some(r.key.clone()),
        };
        out.push(UrnPreview {
            path: r.path.display().to_string(),
            key: r.key,
            urn: display.canonical(),
            retrieval_key: rootless.retrieval_key().to_hex(),
        });
    }
    Ok(out)
}

/// §8.5 social conventions: stage the `/.well-known/dig/manifest.json` discovery
/// manifest as a NORMAL resource. The publisher elects to expose the resources
/// currently staged (every staged key except the discovery key itself), each
/// with a human label (the key) and an inferred content type. `commit` then
/// seals/chunks/merkle-roots it exactly like any other resource, so a discoverer
/// who knows the store ID can construct its URN, derive the retrieval key, and
/// `cat` it back (`read_discovery_manifest`).
///
/// Returns the staged entries (for presentation).
pub fn stage_discovery_manifest(
    ctx: &CliContext,
) -> Result<crate::ops::discovery::DiscoveryManifest, CliError> {
    use crate::ops::discovery::{
        infer_content_type, DiscoveryEntry, DiscoveryManifest, DISCOVERY_RESOURCE_KEY,
    };

    let cfg = ctx.load_config()?;
    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;

    // Publisher-elected resources = everything staged so far, except the
    // discovery manifest itself (so re-staging is idempotent in content).
    let mut entries: Vec<DiscoveryEntry> = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
        .into_iter()
        .map(|r| r.resource_key)
        .filter(|k| k != DISCOVERY_RESOURCE_KEY)
        .map(|key| DiscoveryEntry {
            label: key.clone(),
            content_type: infer_content_type(&key),
            key,
        })
        .collect();
    // Deterministic order independent of staging order (§19.3 spirit).
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries.dedup_by(|a, b| a.key == b.key);

    let manifest = DiscoveryManifest::new(entries);
    let body = manifest.to_json_bytes();

    // Enforce StoreConfig.max_size (§20.2), mirroring `add_path`.
    if cfg.max_size != 0 {
        let already: u64 = staging
            .records()
            .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
            .iter()
            .filter(|r| r.resource_key != DISCOVERY_RESOURCE_KEY)
            .map(|r| r.content.len() as u64)
            .sum();
        if already + body.len() as u64 > cfg.max_size {
            return Err(CliError::InvalidArgument(
                "staged size with discovery manifest exceeds store max_size".into(),
            ));
        }
    }

    staging
        .append(DISCOVERY_RESOURCE_KEY, &body)
        .map_err(|e| CliError::Other(anyhow::anyhow!("stage discovery: {e}")))?;
    Ok(manifest)
}

/// §8.5 reader: fetch and parse the discovery manifest by its conventional
/// retrieval key. This is an ordinary `cat` — it drives the real compiled module
/// through the host, GCM-decrypts client-side, and parses the bytes. Returns
/// `NotFound` if the publisher did not publish one.
pub fn read_discovery_manifest(
    ctx: &CliContext,
    store_id: Bytes32,
    root: Bytes32,
    salt: Option<&[u8; 32]>,
) -> Result<crate::ops::discovery::DiscoveryManifest, CliError> {
    use crate::ops::discovery::{DiscoveryManifest, DISCOVERY_RESOURCE_KEY};

    let urn = canonical_resource_urn(store_id, DISCOVERY_RESOURCE_KEY);
    let module_path = module_path_for(ctx, &store_id, Some(root))?;
    let resp = crate::ops::serve::serve_content(ctx, &module_path, &urn, root)?;
    let chunk_lens = resource_chunk_lens(ctx, &root, DISCOVERY_RESOURCE_KEY).unwrap_or_default();
    let bytes =
        crate::ops::client_crypto::decrypt_and_verify(&resp, &urn, salt, &root, &chunk_lens)?;
    DiscoveryManifest::from_json_bytes(&bytes)
        .map_err(|e| CliError::Other(anyhow::anyhow!("parse discovery manifest: {e}")))
}

pub fn status(ctx: &CliContext) -> Result<StatusView, CliError> {
    let cfg = ctx.load_config()?;
    let staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let records = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?;
    let staged_bytes: u64 = records.iter().map(|r| r.content.len() as u64).sum();
    let staged = records.into_iter().map(|r| r.resource_key).collect();
    let root = current_root(ctx)?.map(|r| r.to_hex());
    let limit_bytes = cap_of(cfg.max_size);
    Ok(StatusView {
        root,
        staged,
        modified: Vec::new(),
        untracked: Vec::new(),
        staged_bytes,
        limit_bytes,
    })
}

/// Classify working-directory files vs. staging and the current generation.
pub fn compute_status(ctx: &CliContext) -> Result<StatusView, CliError> {
    let cfg = ctx.load_config()?;
    // §2.8: status classifies files under the resolved operating directory.
    let root_dir = ctx.op_dir.clone();
    let current = current_root(ctx)?;

    // Working set: key -> file content.
    let working: std::collections::BTreeMap<String, Vec<u8>> =
        crate::ops::walk::resolve_all(&root_dir, &ctx.workspace_dir)
            .into_iter()
            .filter_map(|r| fs::read(&r.path).ok().map(|c| (r.key, c)))
            .collect();

    // Staged set: key -> content.
    let staged_map: std::collections::BTreeMap<String, Vec<u8>> =
        match StagingArea::open(ctx.staging_path(&cfg.store_id)) {
            Ok(s) => s
                .records()
                .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
                .into_iter()
                .map(|r| (r.resource_key, r.content))
                .collect(),
            Err(_) => Default::default(),
        };

    let mut staged_keys: Vec<String> = staged_map.keys().cloned().collect();
    staged_keys.sort();
    let staged_bytes: u64 = staged_map.values().map(|c| c.len() as u64).sum();

    let mut modified = Vec::new();
    let mut untracked = Vec::new();
    for (key, content) in &working {
        if staged_map.contains_key(key) {
            continue; // already shown as staged
        }
        match committed_content(ctx, &cfg, current.as_ref(), key)? {
            Some(committed) => {
                if committed != *content {
                    modified.push(key.clone());
                }
            }
            None => untracked.push(key.clone()),
        }
    }
    modified.sort();
    untracked.sort();

    let limit_bytes = cap_of(cfg.max_size);
    Ok(StatusView {
        root: current.map(|r| r.to_hex()),
        staged: staged_keys,
        modified,
        untracked,
        staged_bytes,
        limit_bytes,
    })
}

/// Return the plaintext of a committed resource for `key` at `current` root,
/// or `None` if the key is not a committed resource. Reuses the local
/// serve+decrypt path used by `cat` via `ops::serve::read_resource_plaintext`.
fn committed_content(
    ctx: &CliContext,
    cfg: &digstore_core::StoreConfig,
    current: Option<&Bytes32>,
    key: &str,
) -> Result<Option<Vec<u8>>, CliError> {
    let root = match current {
        Some(r) => *r,
        None => return Ok(None),
    };
    // Check whether this key is actually a committed resource.
    if !list_generation_resources(ctx, &root)?
        .iter()
        .any(|k| k == key)
    {
        return Ok(None);
    }
    // Serve + decrypt exactly as `cat` does.
    let plaintext = crate::ops::serve::read_resource_plaintext(ctx, cfg, &root, key)
        .map_err(|e| CliError::Other(anyhow::anyhow!("read committed {key}: {e}")))?;
    Ok(Some(plaintext))
}

#[derive(Debug)]
pub struct CommitOutcome {
    pub roothash: Bytes32,
    pub output_path: PathBuf,
    pub output_size: u64,
}

pub fn commit(ctx: &CliContext, _message: Option<String>) -> Result<CommitOutcome, CliError> {
    let cfg = ctx.load_config()?;
    let salt = salt_of(&cfg);

    let staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let records = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?;
    if records.is_empty() {
        return Err(CliError::InvalidArgument("nothing staged to commit".into()));
    }

    // Defensive cap check (§3): the stage cap is enforced atomically at `add`,
    // but a legacy/migrated staging file could already exceed it. Refuse to
    // commit content over the store's limit.
    let cap = cap_of(cfg.max_size);
    let staged_total: u64 = records.iter().map(|r| r.content.len() as u64).sum();
    if staged_total > cap {
        return Err(CliError::InvalidArgument(format!(
            "staged content is {:.1} MB, over the {:.1} MB limit; unstage some files (digstore unstage) before committing",
            staged_total as f64 / 1_000_000.0,
            cap as f64 / 1_000_000.0
        )));
    }

    // Build the encrypted chunk pool + key table. Each resource's chunks are
    // AES-256-GCM-sealed under its per-URN key. The served resource ciphertext is
    // the PLAIN ordered concat of its chunk ciphertexts (BINDING contract D5/C9:
    // exactly what the guest's `get_content` returns via `concat_output`). The
    // generation merkle tree has ONE leaf per resource:
    // `leaf = SHA-256(concat_output(ordered chunk ciphertexts))`, so a single
    // `ContentResponse.merkle_proof` fully verifies the served bytes to the root.
    // Leaves are ordered ascending by `static_key` to match the compiler's
    // `current_generation_leaves` (D5), so the store-reported root equals the
    // module's injected `CurrentRoot` and the client gate `proof.root ==
    // trusted_root` holds.
    let mut pool_bodies: Vec<Vec<u8>> = Vec::new(); // chunk ciphertext bodies, global order
    let mut pool_hashes: Vec<Bytes32> = Vec::new(); // SHA-256(chunk ciphertext) (manifest/diff)
    let mut key_records: Vec<(String, Vec<u32>, u64)> = Vec::new();
    // (static_key, leaf) so we can sort leaves ascending by static_key (D5).
    let mut keyed_leaves: Vec<([u8; 32], Bytes32)> = Vec::new();

    for rec in &records {
        let urn = canonical_resource_urn(cfg.store_id, &rec.resource_key);
        let aes_key = digstore_crypto::derive_decryption_key(&urn.canonical(), salt.as_ref());
        let chunks: Vec<Chunk> = chunk_slice(&rec.content, &chunker());
        let chunks = if chunks.is_empty() {
            vec![Chunk::new(0, Vec::new())]
        } else {
            chunks
        };
        let mut indices = Vec::with_capacity(chunks.len());
        let mut chunk_cts: Vec<Vec<u8>> = Vec::with_capacity(chunks.len());
        for c in &chunks {
            let ct = digstore_crypto::encrypt_chunk(&aes_key, &c.data);
            let h = digstore_crypto::sha256(&ct);
            let idx = pool_bodies.len() as u32;
            chunk_cts.push(ct.clone());
            pool_bodies.push(ct);
            pool_hashes.push(h);
            indices.push(idx);
        }
        // D5: leaf = SHA-256(concat_output(chunks)) — the exact bytes get_content
        // returns for this resource (plain ordered concat, NO length framing).
        let slices: Vec<&[u8]> = chunk_cts.iter().map(|c| c.as_slice()).collect();
        let resource_blob = digstore_core::serving::concat_output(&slices);
        keyed_leaves.push((
            urn.retrieval_key().0,
            digstore_crypto::sha256(&resource_blob),
        ));
        key_records.push((rec.resource_key.clone(), indices, rec.content.len() as u64));
    }

    // Ascending by static_key (raw 32 bytes; Bytes32 has no Ord) — the exact
    // order the compiler injects and the guest ranks against (D5).
    keyed_leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let resource_leaves: Vec<Bytes32> = keyed_leaves.into_iter().map(|(_, l)| l).collect();

    let tree = MerkleTree::from_leaves(resource_leaves);
    let root = tree.root();
    let root_hex = root.to_hex();

    let next_id = RootHistory::open(ctx.history_path())
        .and_then(|h| h.next_id())
        .map_err(|e| CliError::Other(anyhow::anyhow!("history: {e}")))?;
    let timestamp = current_time();

    // Persist the generation manifest + ciphertext chunk bodies.
    let chunks_dir = ctx.generations_dir().join(&root_hex).join("chunks");
    fs::create_dir_all(&chunks_dir).map_err(|e| CliError::Other(e.into()))?;
    let mut chunk_refs = Vec::with_capacity(pool_bodies.len());
    for (i, (hash, body)) in pool_hashes.iter().zip(pool_bodies.iter()).enumerate() {
        fs::write(chunks_dir.join(hash.to_hex()), body).map_err(|e| CliError::Other(e.into()))?;
        chunk_refs.push(ChunkRef {
            index: i as u32,
            hash: *hash,
            size: body.len() as u64,
        });
    }
    let key_table: Vec<KeyTableRecord> = key_records
        .iter()
        .map(|(rk, indices, total)| {
            let urn = canonical_resource_urn(cfg.store_id, rk);
            KeyTableRecord {
                resource_key: rk.clone(),
                static_key: urn.retrieval_key(),
                generation: root,
                chunk_indices: indices.clone(),
                total_size: *total,
            }
        })
        .collect();
    let manifest = GenerationManifest {
        schema_version: 1,
        generation_id: next_id,
        root,
        timestamp,
        chunks: chunk_refs,
        key_table,
    };
    manifest
        .write_to(ctx.generations_dir().join(&root_hex).join("manifest.json"))
        .map_err(|e| CliError::Other(anyhow::anyhow!("write manifest: {e}")))?;

    // Append history.
    let mut history = RootHistory::open(ctx.history_path())
        .map_err(|e| CliError::Other(anyhow::anyhow!("history open: {e}")))?;
    history
        .append(&GenerationState {
            id: next_id,
            root,
            timestamp,
        })
        .map_err(|e| CliError::Other(anyhow::anyhow!("history append: {e}")))?;

    // Local URN manifest (§6.1): the publisher's index of shareable URNs. Local
    // only — not embedded, not pushed. Root-pinned URN, rootless retrieval key.
    {
        #[derive(serde::Serialize)]
        struct UrnEntry {
            key: String,
            urn: String,
            retrieval_key: String,
            size: u64,
        }
        #[derive(serde::Serialize)]
        struct UrnManifest {
            store_id: String,
            store: Option<String>,
            root: String,
            generation: u64,
            resources: Vec<UrnEntry>,
        }
        let mut resources = Vec::with_capacity(manifest.key_table.len());
        let mut txt = String::new();
        for rec in &manifest.key_table {
            let pinned = Urn {
                chain: "chia".to_string(),
                store_id: cfg.store_id,
                root_hash: Some(root),
                resource_key: Some(rec.resource_key.clone()),
            };
            let urn = pinned.canonical();
            txt.push_str(&format!("{}\t{}\n", rec.resource_key, urn));
            resources.push(UrnEntry {
                key: rec.resource_key.clone(),
                urn,
                retrieval_key: rec.static_key.to_hex(),
                size: rec.total_size,
            });
        }
        let out = UrnManifest {
            store_id: cfg.store_id.to_hex(),
            store: ctx.store_name.clone(),
            root: root_hex.clone(),
            generation: next_id,
            resources,
        };
        let json = serde_json::to_string_pretty(&out).map_err(|e| CliError::Other(e.into()))?;
        fs::write(ctx.dig_dir.join("urns.json"), json).map_err(|e| CliError::Other(e.into()))?;
        fs::write(ctx.dig_dir.join("urns.txt"), txt).map_err(|e| CliError::Other(e.into()))?;
    }

    // Compile a real module (so a real .wasm exists for host/push/clone).
    let output_path = compile_module(ctx, &cfg, &pool_bodies, &manifest, root)?;
    let output_size = fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);

    // Clear staging.
    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    staging
        .clear()
        .map_err(|e| CliError::Other(anyhow::anyhow!("clear staging: {e}")))?;

    Ok(CommitOutcome {
        roothash: root,
        output_path,
        output_size,
    })
}

/// Compile the generation into a real serving module via `digstore-compiler`.
/// The explicit no-auth policy compiled into a store that requires neither a
/// session nor a JWT (§4.1/§5.2). A JWT- or session-required store would supply
/// its configured `AuthenticationInfo` to `Compiler::compile` instead.
fn default_auth_info() -> AuthenticationInfo {
    AuthenticationInfo {
        requires_session: false,
        requires_jwt: false,
        jwks_url: None,
        accepted_algorithms: Vec::new(),
    }
}

fn compile_module(
    ctx: &CliContext,
    cfg: &StoreConfig,
    pool_bodies: &[Vec<u8>],
    manifest: &GenerationManifest,
    root: Bytes32,
) -> Result<PathBuf, CliError> {
    use digstore_compiler::{Compiler, CompilerConfig, GenerationView, ResourceView};

    struct Res {
        key: Bytes32,
        chunks: Vec<(Bytes32, Vec<u8>)>,
    }
    impl ResourceView for Res {
        fn resource_key(&self) -> Bytes32 {
            self.key
        }
        fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
            self.chunks.clone()
        }
    }
    struct Gen {
        root: Bytes32,
        res: Vec<Res>,
    }
    impl GenerationView for Gen {
        fn root(&self) -> Bytes32 {
            self.root
        }
        fn resources(&self) -> Vec<Box<dyn ResourceView + '_>> {
            self.res
                .iter()
                .map(|r| {
                    Box::new(Res {
                        key: r.key,
                        chunks: r.chunks.clone(),
                    }) as Box<dyn ResourceView + '_>
                })
                .collect()
        }
    }

    let res: Vec<Res> = manifest
        .key_table
        .iter()
        .map(|kt| Res {
            key: kt.static_key,
            chunks: kt
                .chunk_indices
                .iter()
                .map(|&i| {
                    let body = pool_bodies[i as usize].clone();
                    (digstore_crypto::sha256(&body), body)
                })
                .collect(),
        })
        .collect();
    let gen = Gen { root, res };

    let trusted = load_trusted_keys(ctx)?;
    let store_pubkey = load_host_pubkey(ctx)?;
    let ccfg = CompilerConfig {
        output_dir: ctx.modules_dir(),
        obfuscate: false,
        optimize: false,
        // D6: compile with the REAL guest wasm so the module serves itself via
        // `HostRuntime::serve_content` (NOT the stub template). The CLI embeds the
        // guest wasm at build time (see `build.rs` / `serve::embedded_guest_wasm`).
        template_override: Some(crate::ops::serve::embedded_guest_wasm().to_vec()),
        // §8.3 uniform-size filler budget: production pads to the 128 MiB default
        // (or the DIGSTORE_UNIFORM_BLOB_LEN override) so every store is the same
        // module size.
        ..CompilerConfig::default()
    };
    let outcome = Compiler::compile(
        &ccfg,
        cfg.store_id,
        store_pubkey,
        &[gen],
        crate::ops::serve::empty_manifest(),
        // §4.1/§5.2: per-store auth policy is compiled into the module. The CLI
        // supplies the explicit no-auth default here; a JWT/session-required
        // store would thread its configured policy into this argument instead.
        default_auth_info(),
        &trusted,
    )
    .map_err(|e| CliError::Other(anyhow::anyhow!("compile failed: {e:?}")))?;
    Ok(outcome.result.output_path)
}

pub fn log(ctx: &CliContext, limit: Option<usize>) -> Result<Vec<LogEntry>, CliError> {
    let states = read_history(ctx)?;
    let mut states = states;
    states.sort_by(|a, b| b.id.cmp(&a.id));
    let iter = states.into_iter().map(|s| LogEntry {
        id: s.id,
        root: s.root.to_hex(),
        timestamp: s.timestamp,
    });
    Ok(match limit {
        Some(n) => iter.take(n).collect(),
        None => iter.collect(),
    })
}

pub fn current_root(ctx: &CliContext) -> Result<Option<Bytes32>, CliError> {
    Ok(read_history(ctx)?
        .iter()
        .max_by_key(|s| s.id)
        .map(|s| s.root))
}

fn read_history(ctx: &CliContext) -> Result<Vec<GenerationState>, CliError> {
    let path = ctx.history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    RootHistory::open(&path)
        .and_then(|h| h.entries())
        .map_err(|e| CliError::Other(anyhow::anyhow!("history: {e}")))
}

/// Append a generation state to history (used by clone/pull).
pub(crate) fn append_history(ctx: &CliContext, state: GenerationState) -> Result<(), CliError> {
    let mut history = RootHistory::open(ctx.history_path())
        .map_err(|e| CliError::Other(anyhow::anyhow!("history open: {e}")))?;
    // Tolerate an out-of-order id by realigning to the expected next id.
    let expected = history
        .next_id()
        .map_err(|e| CliError::Other(anyhow::anyhow!("history next_id: {e}")))?;
    history
        .append(&GenerationState {
            id: expected,
            root: state.root,
            timestamp: state.timestamp,
        })
        .map_err(|e| CliError::Other(anyhow::anyhow!("history append: {e}")))
}

pub fn module_path_for(
    ctx: &CliContext,
    store_id: &Bytes32,
    root: Option<Bytes32>,
) -> Result<PathBuf, CliError> {
    let root = match root {
        Some(r) => r,
        None => current_root(ctx)?
            .ok_or_else(|| CliError::NotFound("no committed root; run `digstore commit`".into()))?,
    };
    let path = ctx
        .modules_dir()
        .join(format!("{}-{}.dig", store_id.to_hex(), root.to_hex()));
    if !path.exists() {
        return Err(CliError::NotFound(format!(
            "module for root {}",
            root.to_hex()
        )));
    }
    Ok(path)
}

/// The conventional default-view resource key (§8.5 social conventions): a URN
/// with no resource key resolves to the store's landing resource, `index.html`.
pub const DEFAULT_RESOURCE_KEY: &str = "index.html";

/// Resolve the effective resource key for a URN (§8.5 social conventions).
///
/// When the URN carries an explicit resource key, it is used verbatim. When it
/// has none, the conventional default view `index.html` is used IF the
/// generation at `root` actually exposes that key; otherwise the empty string is
/// returned (store-level fallback — the prior behavior, which yields a
/// non-verifying decoy on a miss).
pub fn resolve_resource_key(ctx: &CliContext, root: &Bytes32, urn: &Urn) -> String {
    if let Some(rk) = &urn.resource_key {
        return rk.clone();
    }
    match load_generation_manifest(ctx, root) {
        Ok(manifest)
            if manifest
                .key_table
                .iter()
                .any(|k| k.resource_key == DEFAULT_RESOURCE_KEY) =>
        {
            DEFAULT_RESOURCE_KEY.to_string()
        }
        _ => String::new(),
    }
}

pub fn list_generation_resources(
    ctx: &CliContext,
    root: &Bytes32,
) -> Result<Vec<String>, CliError> {
    let manifest = load_generation_manifest(ctx, root)?;
    Ok(manifest
        .key_table
        .iter()
        .map(|k| k.resource_key.clone())
        .collect())
}

/// One committed resource with its canonical (store-rooted) URN and retrieval
/// key. The retrieval key is root-independent (= SHA-256 of the rootless URN).
pub struct ResourceKeyEntry {
    pub resource_key: String,
    pub urn: String,
    pub retrieval_key: String,
}

/// List every resource committed in `root` with its canonical store-rooted URN
/// and retrieval key, sorted by resource key.
pub fn list_resource_keys(
    ctx: &CliContext,
    store_id: Bytes32,
    root: &Bytes32,
) -> Result<Vec<ResourceKeyEntry>, CliError> {
    let manifest = load_generation_manifest(ctx, root)?;
    let mut out: Vec<ResourceKeyEntry> = manifest
        .key_table
        .iter()
        .map(|k| ResourceKeyEntry {
            resource_key: k.resource_key.clone(),
            urn: canonical_resource_urn(store_id, &k.resource_key).canonical(),
            retrieval_key: k.static_key.to_hex(),
        })
        .collect();
    out.sort_by(|a, b| a.resource_key.cmp(&b.resource_key));
    Ok(out)
}

/// Resolve the resource key whose retrieval (static) key equals `retrieval_key`
/// within `root`. Used to stream raw encrypted bytes by retrieval key alone.
pub fn resource_key_for_retrieval_key(
    ctx: &CliContext,
    root: &Bytes32,
    retrieval_key: &Bytes32,
) -> Result<String, CliError> {
    let manifest = load_generation_manifest(ctx, root)?;
    manifest
        .key_table
        .iter()
        .find(|k| k.static_key == *retrieval_key)
        .map(|k| k.resource_key.clone())
        .ok_or_else(|| {
            CliError::NotFound(format!(
                "no resource with retrieval key {} in this generation",
                retrieval_key.to_hex()
            ))
        })
}

/// Per-chunk CIPHERTEXT byte lengths for `resource_key` in `root`, in chunk
/// order. The client uses these to split the module's PLAIN-concatenated served
/// ciphertext (D5/C9) back into individual GCM chunks. Returns an empty vec if
/// the resource is absent (the client then treats the blob as one chunk).
pub fn resource_chunk_lens(
    ctx: &CliContext,
    root: &Bytes32,
    resource_key: &str,
) -> Result<Vec<usize>, CliError> {
    let manifest = load_generation_manifest(ctx, root)?;
    let by_index: BTreeMap<u32, u64> = manifest.chunks.iter().map(|c| (c.index, c.size)).collect();
    let entry = manifest
        .key_table
        .iter()
        .find(|k| k.resource_key == resource_key);
    let entry = match entry {
        Some(e) => e,
        None => return Ok(Vec::new()),
    };
    let mut lens = Vec::with_capacity(entry.chunk_indices.len());
    for idx in &entry.chunk_indices {
        let size = by_index
            .get(idx)
            .ok_or_else(|| CliError::VerificationFailed("manifest chunk index missing".into()))?;
        lens.push(*size as usize);
    }
    Ok(lens)
}

pub(crate) fn load_generation_manifest(
    ctx: &CliContext,
    root: &Bytes32,
) -> Result<GenerationManifest, CliError> {
    let path = ctx
        .generations_dir()
        .join(root.to_hex())
        .join("manifest.json");
    if !path.exists() {
        return Err(CliError::NotFound(format!("generation {}", root.to_hex())));
    }
    GenerationManifest::read_from(&path)
        .map_err(|e| CliError::Other(anyhow::anyhow!("read manifest: {e}")))
}

pub fn diff(ctx: &CliContext, from: &Bytes32, to: &Bytes32) -> Result<Vec<DiffEntry>, CliError> {
    let from_map = generation_resource_digests(ctx, from)?;
    let to_map = generation_resource_digests(ctx, to)?;
    let mut out = Vec::new();
    for (key, to_digest) in &to_map {
        match from_map.get(key) {
            None => out.push(DiffEntry {
                resource_key: key.clone(),
                change: "added".into(),
            }),
            Some(from_digest) if from_digest != to_digest => out.push(DiffEntry {
                resource_key: key.clone(),
                change: "modified".into(),
            }),
            Some(_) => {}
        }
    }
    for key in from_map.keys() {
        if !to_map.contains_key(key) {
            out.push(DiffEntry {
                resource_key: key.clone(),
                change: "removed".into(),
            });
        }
    }
    out.sort_by(|a, b| a.resource_key.cmp(&b.resource_key));
    Ok(out)
}

/// Per-resource digest = SHA-256(concat(ordered ciphertext chunk hashes)).
fn generation_resource_digests(
    ctx: &CliContext,
    root: &Bytes32,
) -> Result<BTreeMap<String, Bytes32>, CliError> {
    let manifest = load_generation_manifest(ctx, root)?;
    let by_index: BTreeMap<u32, Bytes32> =
        manifest.chunks.iter().map(|c| (c.index, c.hash)).collect();
    let mut out = BTreeMap::new();
    for kt in &manifest.key_table {
        let mut buf = Vec::new();
        for idx in &kt.chunk_indices {
            if let Some(h) = by_index.get(idx) {
                buf.extend_from_slice(&h.0);
            }
        }
        out.insert(kt.resource_key.clone(), digstore_crypto::sha256(&buf));
    }
    Ok(out)
}

// ---- helpers ----

fn current_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate 32 bytes of cryptographically secure randomness for key/salt seeds.
///
/// Uses the operating system CSPRNG via `getrandom` (`/dev/urandom`, `getrandom(2)`,
/// `BCryptGenRandom`, etc.). There is deliberately NO weak fallback: predictable
/// seeds (time/pid/pointer-derived) would make BLS signing keys and the private
/// `SecretSalt` guessable, so if the OS RNG is unavailable we panic rather than
/// emit attacker-predictable key material.
fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed)
        .expect("operating system CSPRNG must be available to generate key material");
    seed
}

/// Write a secret file (BLS signing seed, private salt) with owner-only
/// permissions. On Unix the file is created mode `0600`; on Windows it inherits
/// the user-profile ACL (the `.dig` dir lives under the user's home), which is
/// already restricted to the owner.
fn write_secret_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        f.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredKey {
    public_key: String,
    label: String,
}

fn serialize_keys(keys: &[TrustedHostKey]) -> Vec<StoredKey> {
    keys.iter()
        .map(|k| StoredKey {
            public_key: hex::encode(k.public_key),
            label: k.label.clone(),
        })
        .collect()
}

pub(crate) fn load_trusted_keys(ctx: &CliContext) -> Result<Vec<TrustedHostKey>, CliError> {
    let path = ctx.dig_dir.join("trusted_keys.json");
    let text = fs::read_to_string(&path).map_err(|e| CliError::Other(e.into()))?;
    let stored: Vec<StoredKey> =
        serde_json::from_str(&text).map_err(|e| CliError::Other(e.into()))?;
    let mut out = Vec::with_capacity(stored.len());
    for s in stored {
        let bytes = hex::decode(&s.public_key)
            .map_err(|_| CliError::InvalidArgument("bad trusted key hex".into()))?;
        let arr: [u8; 48] = bytes
            .try_into()
            .map_err(|_| CliError::InvalidArgument("trusted key must be 48 bytes".into()))?;
        out.push(TrustedHostKey {
            public_key: arr,
            label: s.label,
        });
    }
    Ok(out)
}

pub(crate) fn load_host_pubkey(ctx: &CliContext) -> Result<Bytes48, CliError> {
    let keys = load_trusted_keys(ctx)?;
    let k = keys
        .first()
        .ok_or_else(|| CliError::InvalidArgument("no trusted host key".into()))?;
    Ok(Bytes48(k.public_key))
}

/// Load the host BLS signing key (seed) persisted at init.
pub(crate) fn load_signing_key(
    ctx: &CliContext,
) -> Result<digstore_crypto::bls::SecretKey, CliError> {
    let bytes =
        fs::read(ctx.dig_dir.join("signing_key.bin")).map_err(|e| CliError::Other(e.into()))?;
    Ok(digstore_crypto::bls::SecretKey::from_seed(&bytes))
}

/// Generate a fresh host BLS signing identity: returns the 32-byte seed and the
/// 48-byte G1 public key. The seed is the only persisted secret; the BLS key is
/// reconstructed via `from_seed`.
pub(crate) fn generate_host_key() -> ([u8; 32], digstore_core::Bytes48) {
    let seed = random_seed();
    let secret = digstore_crypto::bls::SecretKey::from_seed(&seed);
    (seed, secret.public_key().to_bytes())
}

/// Persist a host signing identity into the dig dir: the secret seed
/// (`signing_key.bin`, never embedded in modules) and the public trusted-key
/// record (`trusted_keys.json`). Used by both `init` and `clone` so a node that
/// serves a module always holds a key the module trusts (§12.2).
pub(crate) fn persist_host_identity(
    ctx: &CliContext,
    seed: &[u8; 32],
    public_key: digstore_core::Bytes48,
) -> Result<(), CliError> {
    fs::create_dir_all(&ctx.dig_dir).map_err(|e| CliError::Other(e.into()))?;
    write_secret_file(&ctx.dig_dir.join("signing_key.bin"), seed)
        .map_err(|e| CliError::Other(e.into()))?;
    let trusted = vec![TrustedHostKey {
        public_key: public_key.0,
        label: format!("dig-host-key-v1:{}", public_key.to_hex()),
    }];
    fs::write(
        ctx.dig_dir.join("trusted_keys.json"),
        serde_json::to_string_pretty(&serialize_keys(&trusted))
            .map_err(|e| CliError::Other(e.into()))?,
    )
    .map_err(|e| CliError::Other(e.into()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx(private: bool) -> (tempfile::TempDir, CliContext) {
        let td = tempdir().unwrap();
        let ctx = CliContext::workspace_only(td.path().to_path_buf(), false, false);
        init_store(&ctx, private, None).unwrap();
        (td, ctx)
    }

    /// Build a single-store context whose `op_dir` is a `work/` subdir of the
    /// temp dir (the operating directory scanned by `add_files`/`compute_status`),
    /// kept distinct from the `.dig` workspace dir so the skip logic is exercised.
    fn test_store_ctx() -> (CliContext, tempfile::TempDir) {
        let td = tempdir().unwrap();
        let dig = td.path().join(".dig");
        let work = td.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut ctx = CliContext::workspace_only(dig.clone(), false, false);
        ctx.op_dir = work;
        init_store(&ctx, false, None).unwrap();
        (ctx, td)
    }

    #[test]
    fn init_creates_layout_and_config() {
        let td = tempdir().unwrap();
        let ctx = CliContext::workspace_only(td.path().to_path_buf(), false, false);
        let res = init_store(&ctx, false, None).unwrap();
        assert!(ctx.config_path().exists());
        assert!(ctx.modules_dir().exists());
        assert!(ctx.generations_dir().exists());
        assert!(td.path().join("trusted_keys.json").exists());
        assert!(td.path().join("signing_key.bin").exists());
        assert_ne!(res.store_id, Bytes32([0u8; 32]));
    }

    #[test]
    fn init_store_id_is_sha256_of_pubkey() {
        let td = tempdir().unwrap();
        let ctx = CliContext::workspace_only(td.path().to_path_buf(), false, false);
        let res = init_store(&ctx, false, None).unwrap();
        let expected = digstore_crypto::sha256(&res.host_public_key.0);
        assert_eq!(res.store_id, expected);
    }

    #[test]
    fn init_private_records_secret_salt_file() {
        let (_td, ctx) = ctx(true);
        let cfg = ctx.load_config().unwrap();
        assert!(matches!(cfg.visibility, Visibility::Private(_)));
        let salt_hex = std::fs::read_to_string(ctx.salt_path()).unwrap();
        assert_eq!(salt_hex.trim().len(), 64);
    }

    #[test]
    fn add_path_stages_and_status_shows_it() {
        let (td, ctx) = ctx(false);
        let f = td.path().join("readme.txt");
        std::fs::write(&f, b"hello digstore").unwrap();
        let added = add_path(&ctx, &f, Some("readme".into())).unwrap();
        assert_eq!(added.resource_key, "readme");
        assert!(added.chunk_count >= 1);
        let s = status(&ctx).unwrap();
        assert!(s.staged.iter().any(|x| x == "readme"));
    }

    #[test]
    fn add_path_defaults_key_to_file_name() {
        let (td, ctx) = ctx(false);
        let f = td.path().join("notes.md");
        std::fs::write(&f, b"x").unwrap();
        let added = add_path(&ctx, &f, None).unwrap();
        assert_eq!(added.resource_key, "notes.md");
    }

    #[test]
    fn log_is_empty_before_any_commit() {
        let (_td, ctx) = ctx(false);
        assert!(log(&ctx, None).unwrap().is_empty());
    }

    #[test]
    fn commit_builds_module_and_appends_root() {
        let (td, ctx) = ctx(false);
        let f = td.path().join("a.txt");
        std::fs::write(&f, b"alpha beta gamma delta").unwrap();
        add_path(&ctx, &f, Some("a".into())).unwrap();
        let res = commit(&ctx, Some("first".into())).unwrap();
        assert!(res.output_path.exists());
        let entries = log(&ctx, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].root, res.roothash.to_hex());
    }

    #[test]
    fn commit_with_nothing_staged_errors() {
        let (_td, ctx) = ctx(false);
        assert!(commit(&ctx, None).is_err());
    }

    #[test]
    fn module_path_for_resolves_latest_when_root_omitted() {
        let (td, ctx) = ctx(false);
        let f = td.path().join("x.txt");
        std::fs::write(&f, b"data").unwrap();
        add_path(&ctx, &f, Some("x".into())).unwrap();
        let res = commit(&ctx, None).unwrap();
        let store_id = ctx.find_store_id().unwrap();
        let p = module_path_for(&ctx, &store_id, None).unwrap();
        assert!(p.ends_with(format!(
            "{}-{}.dig",
            store_id.to_hex(),
            res.roothash.to_hex()
        )));
    }

    #[test]
    fn list_generation_resources_returns_committed_keys() {
        let (td, ctx) = ctx(false);
        let f = td.path().join("a.txt");
        std::fs::write(&f, b"alpha").unwrap();
        add_path(&ctx, &f, Some("a".into())).unwrap();
        let res = commit(&ctx, None).unwrap();
        let keys = list_generation_resources(&ctx, &res.roothash).unwrap();
        assert!(keys.iter().any(|k| k == "a"));
    }

    #[test]
    fn add_over_cap_stages_nothing_and_errors() {
        let (ctx, _td) = test_store_ctx();
        // Lower the persisted cap to a tiny value so the atomic cap arithmetic is
        // exercised without writing a 128 MB fixture. `add_files` reads the cap
        // from the store's StoreConfig.max_size.
        let mut cfg = ctx.load_config().unwrap();
        cfg.max_size = 4;
        digstore_store::save_config(ctx.config_path(), &cfg).unwrap();

        std::fs::write(ctx.op_dir.join("big.txt"), b"way over four bytes").unwrap();
        let err = add_files(&ctx, &[], true, false, None);
        assert!(err.is_err(), "over-cap add must error");

        // Nothing was staged.
        let staging = StagingArea::open(ctx.staging_path(&cfg.store_id)).unwrap();
        assert!(
            staging.records().unwrap().is_empty(),
            "stage nothing on cap"
        );
    }

    #[test]
    fn add_outcome_reports_staged_total_and_limit() {
        let (ctx, _td) = test_store_ctx();
        std::fs::write(ctx.op_dir.join("a.txt"), b"hello").unwrap();
        let out = add_files(&ctx, &[], true, false, None).unwrap();
        assert_eq!(out.limit_bytes, MAX_STORE_BYTES);
        assert_eq!(out.staged_bytes, 5);
    }

    #[test]
    fn re_adding_a_modified_staged_file_replaces_not_accumulates() {
        // Regression for the cap double-count: StagingArea is last-write-wins, so
        // re-adding a modified file REPLACES its staged content. With a 10-byte
        // cap and a 6-byte file, the first add stages 6; re-adding 6 *different*
        // bytes must stay at 6 (not 12, which would spuriously trip the cap).
        let (ctx, _td) = test_store_ctx();
        let mut cfg = ctx.load_config().unwrap();
        cfg.max_size = 10;
        digstore_store::save_config(ctx.config_path(), &cfg).unwrap();

        let f = ctx.op_dir.join("a.txt");
        std::fs::write(&f, b"aaaaaa").unwrap(); // 6 bytes
        let first = add_files(&ctx, &[], true, false, None).unwrap();
        assert_eq!(first.staged_bytes, 6);

        std::fs::write(&f, b"bbbbbb").unwrap(); // 6 different bytes
        let second = add_files(&ctx, &[], true, false, None)
            .expect("re-staging a modified file must not double-count toward the cap");
        assert_eq!(
            second.staged_bytes, 6,
            "re-stage replaces (6); it must not accumulate (12)"
        );

        // Staging holds exactly one record carrying the new content.
        let staging = StagingArea::open(ctx.staging_path(&cfg.store_id)).unwrap();
        let recs = staging.records().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].content, b"bbbbbb");
    }

    #[test]
    fn commit_writes_urn_manifest() {
        let (ctx, _td) = test_store_ctx();
        std::fs::write(ctx.op_dir.join("readme.md"), b"hi").unwrap();
        add_files(&ctx, &[], true, false, None).unwrap();
        let res = commit(&ctx, None).unwrap();
        let store_id = ctx.load_config().unwrap().store_id;
        let json = std::fs::read_to_string(ctx.dig_dir.join("urns.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["resources"][0]["key"], "readme.md");
        let urn = v["resources"][0]["urn"].as_str().unwrap();
        assert!(urn.contains(&res.roothash.to_hex()));
        assert!(urn.starts_with("urn:dig:chia:"));

        // Headline invariant: the manifest's retrieval_key is the ROOTLESS
        // canonical key (derived from the root-INDEPENDENT URN), NOT a hash of
        // the root-pinned URN string. This is what a discoverer reconstructs.
        let manifest_key = v["resources"][0]["retrieval_key"].as_str().unwrap();
        let rootless_key = canonical_resource_urn(store_id, "readme.md")
            .retrieval_key()
            .to_hex();
        assert_eq!(
            manifest_key, rootless_key,
            "manifest retrieval_key must be the rootless canonical key"
        );
        // And it must differ from sha256(root-pinned URN string): the root must
        // NOT participate in the retrieval key.
        let root_pinned_urn = Urn {
            chain: "chia".to_string(),
            store_id,
            root_hash: Some(res.roothash),
            resource_key: Some("readme.md".to_string()),
        }
        .canonical();
        let root_pinned_hash = digstore_crypto::sha256(root_pinned_urn.as_bytes()).to_hex();
        assert_ne!(
            manifest_key, root_pinned_hash,
            "retrieval_key must be rootless, not a hash of the root-pinned URN"
        );

        let txt = std::fs::read_to_string(ctx.dig_dir.join("urns.txt")).unwrap();
        assert!(txt.contains("readme.md\turn:dig:chia:"));
    }

    #[test]
    fn diff_reports_added_and_modified_resources() {
        let (td, ctx) = ctx(false);
        let f = td.path().join("a.txt");
        std::fs::write(&f, b"v1").unwrap();
        add_path(&ctx, &f, Some("a".into())).unwrap();
        let r1 = commit(&ctx, None).unwrap().roothash;

        std::fs::write(&f, b"v2-different-content").unwrap();
        add_path(&ctx, &f, Some("a".into())).unwrap();
        let g = td.path().join("b.txt");
        std::fs::write(&g, b"brand new").unwrap();
        add_path(&ctx, &g, Some("b".into())).unwrap();
        let r2 = commit(&ctx, None).unwrap().roothash;

        let d = diff(&ctx, &r1, &r2).unwrap();
        assert!(d
            .iter()
            .any(|e| e.resource_key == "b" && e.change == "added"));
        assert!(d
            .iter()
            .any(|e| e.resource_key == "a" && e.change == "modified"));
    }
}
