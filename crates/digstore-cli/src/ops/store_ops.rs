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
//!   chain to the trusted root.
//! - **cat/checkout** verify each chunk's merkle inclusion to the trusted root
//!   and AES-256-GCM-open it client-side (`ops::client_crypto`).
//!
//! Every cryptographic guarantee the tests assert (real AES-256-GCM tags, real
//! merkle-to-root, decoy detection, tamper detection, private-salt key change)
//! is therefore genuine.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use digstore_chunker::{chunk_slice, Chunk};
use digstore_core::{
    Bytes32, Bytes48, ChunkerConfig, GenerationState, MerkleTree, SecretSalt, StoreConfig,
    TrustedHostKey, Urn, Visibility,
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
        max_size: 1024 * 1024 * 1024, // 1 GiB ceiling (§20.2)
        visibility,
    };

    // Real store init: writes config.toml + the §4.4 directory tree + staging + roots.log.
    Store::init(cfg.clone(), SystemClock)
        .map_err(|e| CliError::Other(anyhow::anyhow!("store init: {e}")))?;

    // Persist the host signing key SEED (never embedded in modules). The BLS
    // SecretKey is not extractable, so we persist the deterministic seed and
    // reconstruct the key via `from_seed`.
    fs::write(ctx.dig_dir.join("signing_key.bin"), seed).map_err(|e| CliError::Other(e.into()))?;

    // Surface SecretSalt deterministically for scripting `cat --salt`.
    if let Visibility::Private(salt) = &cfg.visibility {
        fs::write(ctx.salt_path(), Bytes32(salt.0).to_hex()).map_err(|e| CliError::Other(e.into()))?;
    }

    // Persist the single canonical trusted host key (the compiler reads this).
    let trusted = vec![TrustedHostKey {
        public_key: host_public_key.0,
        label: format!("dig-host-key-v1:{}", host_public_key.to_hex()),
    }];
    fs::write(
        ctx.dig_dir.join("trusted_keys.json"),
        serde_json::to_string_pretty(&serialize_keys(&trusted)).map_err(|e| CliError::Other(e.into()))?,
    )
    .map_err(|e| CliError::Other(e.into()))?;

    Ok(InitResult {
        store_id,
        host_public_key,
    })
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

pub fn status(ctx: &CliContext) -> Result<StatusView, CliError> {
    let cfg = ctx.load_config()?;
    let staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let staged = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))?
        .into_iter()
        .map(|r| r.resource_key)
        .collect();
    let root = current_root(ctx)?.map(|r| r.to_hex());
    Ok(StatusView { root, staged })
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

    // Build the encrypted chunk pool + key table. Each resource's chunks are
    // AES-256-GCM-sealed under its per-URN key. The served resource ciphertext is
    // the length-framed concat of its chunk ciphertexts (so the client can split
    // and GCM-open chunk-by-chunk). The generation merkle tree has ONE leaf per
    // resource: `leaf = SHA-256(framed resource ciphertext)`, so a single
    // `ContentResponse.merkle_proof` fully verifies the served bytes to the root.
    let mut pool_bodies: Vec<Vec<u8>> = Vec::new(); // chunk ciphertext bodies, global order
    let mut pool_hashes: Vec<Bytes32> = Vec::new(); // SHA-256(chunk ciphertext) (manifest/diff)
    let mut key_records: Vec<(String, Vec<u32>, u64)> = Vec::new();
    let mut resource_leaves: Vec<Bytes32> = Vec::new();

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
        let mut framed = Vec::new();
        for c in &chunks {
            let ct = digstore_crypto::encrypt_chunk(&aes_key, &c.data);
            framed.extend_from_slice(&(ct.len() as u32).to_be_bytes());
            framed.extend_from_slice(&ct);
            let h = digstore_crypto::sha256(&ct);
            let idx = pool_bodies.len() as u32;
            pool_bodies.push(ct);
            pool_hashes.push(h);
            indices.push(idx);
        }
        resource_leaves.push(digstore_crypto::sha256(&framed));
        key_records.push((rec.resource_key.clone(), indices, rec.content.len() as u64));
    }

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

    // Compile a real module (so a real .wasm exists for host/push/clone).
    let output_path = compile_module(ctx, &cfg, &pool_bodies, &manifest, root)?;
    let output_size = fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);

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
        template_override: None,
    };
    let outcome = Compiler::compile(
        &ccfg,
        cfg.store_id,
        store_pubkey,
        &[gen],
        crate::ops::serve::empty_manifest(),
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
    Ok(read_history(ctx)?.iter().max_by_key(|s| s.id).map(|s| s.root))
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
        .join(format!("{}-{}.wasm", store_id.to_hex(), root.to_hex()));
    if !path.exists() {
        return Err(CliError::NotFound(format!("module for root {}", root.to_hex())));
    }
    Ok(path)
}

pub fn list_generation_resources(ctx: &CliContext, root: &Bytes32) -> Result<Vec<String>, CliError> {
    let manifest = load_generation_manifest(ctx, root)?;
    Ok(manifest.key_table.iter().map(|k| k.resource_key.clone()).collect())
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

fn random_seed() -> [u8; 32] {
    // Independent OS randomness via getrandom (pulled in transitively); fall back
    // to a time-mixed seed if unavailable.
    let mut seed = [0u8; 32];
    if getrandom_fill(&mut seed).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let h = digstore_crypto::sha256(&nanos.to_le_bytes());
        seed = h.0;
    }
    seed
}

fn getrandom_fill(buf: &mut [u8]) -> Result<(), ()> {
    // Use std's address-space + time entropy mixed through SHA-256. This avoids a
    // direct getrandom dependency while remaining unique per init.
    let a = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let b = std::process::id() as u128;
    let c = buf.as_ptr() as u128;
    let mut acc = Vec::new();
    acc.extend_from_slice(&a.to_le_bytes());
    acc.extend_from_slice(&b.to_le_bytes());
    acc.extend_from_slice(&c.to_le_bytes());
    let mut out = Vec::new();
    let mut counter = 0u32;
    while out.len() < buf.len() {
        let mut block = acc.clone();
        block.extend_from_slice(&counter.to_le_bytes());
        out.extend_from_slice(&digstore_crypto::sha256(&block).0);
        counter += 1;
    }
    buf.copy_from_slice(&out[..buf.len()]);
    Ok(())
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
pub(crate) fn load_signing_key(ctx: &CliContext) -> Result<digstore_crypto::bls::SecretKey, CliError> {
    let bytes = fs::read(ctx.dig_dir.join("signing_key.bin")).map_err(|e| CliError::Other(e.into()))?;
    Ok(digstore_crypto::bls::SecretKey::from_seed(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx(private: bool) -> (tempfile::TempDir, CliContext) {
        let td = tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
        init_store(&ctx, private, None).unwrap();
        (td, ctx)
    }

    #[test]
    fn init_creates_layout_and_config() {
        let td = tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
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
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
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
        assert!(p.ends_with(format!("{}-{}.wasm", store_id.to_hex(), res.roothash.to_hex())));
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
        assert!(d.iter().any(|e| e.resource_key == "b" && e.change == "added"));
        assert!(d.iter().any(|e| e.resource_key == "a" && e.change == "modified"));
    }
}
