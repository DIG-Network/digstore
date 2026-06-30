//! digstore-stage — the context-free stage→compile engine.
//!
//! This crate is the SINGLE home of the "turn a file set into a capsule" pipeline:
//! AES-256-GCM-seal each resource's chunks under its per-URN key, build the
//! generation merkle tree over the **ciphertext** resource leaves (so client
//! merkle verification of ciphertext-to-root is genuine), persist the generation
//! manifest + ciphertext chunk bodies, and compile a real self-serving `.dig`
//! WASM module (BINDING contract D6 — compiled with the embedded `digstore-guest`
//! wasm, so the module serves itself through `digstore_host::HostRuntime`).
//!
//! It is the EXACT engine the `digstore` CLI `commit`/`compile`/`deploy` use —
//! the CLI's `ops::store_ops` now delegates here (no fork), so a CLI commit and
//! an in-process [`stage_and_compile`] of the same files + store id + salt
//! produce byte-identical modules and roots.
//!
//! ## Why this crate exists (the in-process publishing job, #95 Pass C)
//!
//! The DIG Browser runs a native dig-node in-process (`dig_runtime.dll` →
//! `dig-node`). For the browser to publish — turn a folder into a capsule for a
//! local deploy — it needs this pipeline WITHOUT shelling out to the `digstore`
//! CLI binary. The pipeline used to live only in the CLI (a binary crate
//! `dig-node` cannot depend on) and the guest wasm was embedded only in the CLI.
//! Lifting both into this library crate lets BOTH the CLI and `dig-node` use one
//! copy, and embeds the guest wasm once (see `build.rs` / [`embedded_guest_wasm`]).
//!
//! The engine is build-only: it stages + compiles + returns the capsule and
//! module path. The on-chain root advance (Pass B `chia_advanceStore`) and the
//! §21 push are the wallet method + remote push respectively — Pass C is the
//! staging/compile half.

use std::path::{Path, PathBuf};

use digstore_chunker::{chunk_slice, Chunk};
use digstore_core::{
    AuthenticationInfo, Bytes32, Bytes48, ChunkerConfig, MerkleTree, MetadataManifest, SecretSalt,
    StoreConfig, TrustedHostKey, Urn, Visibility, CHAIN, MAX_STORE_BYTES,
};
use digstore_store::{ChunkRef, GenerationManifest, KeyTableRecord};

/// Errors the stage→compile engine can return. Stable variants so callers
/// (the CLI, and dig-node's `dig.stage` RPC) can map them to catalogued error
/// codes without string-matching.
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    /// No files were supplied to stage (an empty capsule is not meaningful).
    #[error("nothing to stage; supply at least one file")]
    EmptyStaging,
    /// Staged content exceeds the store's size cap.
    #[error("staged content is {got_mb:.1} MB, over the {cap_mb:.1} MB limit")]
    OverCap { got_mb: f64, cap_mb: f64 },
    /// The compiler failed to produce a module.
    #[error("compile failed: {0}")]
    Compile(String),
    /// A filesystem error while persisting the generation / chunk bodies.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// The REAL `digstore-guest` wasm, embedded at build time (see `build.rs`).
/// The compiler uses this as the `template_override` so the produced module is
/// genuinely self-serving through `digstore_host::HostRuntime::serve_content`
/// (BINDING contract D6). This is the single embedded copy for the whole engine
/// — `digstore-cli` re-exports it rather than embedding its own.
pub fn embedded_guest_wasm() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/digstore_guest.wasm"))
}

/// Canonical chunker config (matches `digstore-store`'s commit defaults). Public
/// so callers that only need a chunk-count PREVIEW (e.g. the CLI `add` summary)
/// use the SAME config the commit pipeline does.
pub fn chunker_config() -> ChunkerConfig {
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
        chain: CHAIN.to_string(),
        store_id,
        root_hash: None,
        resource_key: Some(resource_key.to_string()),
    }
}

fn salt_of(visibility: &Visibility) -> Option<SecretSalt> {
    match visibility {
        Visibility::Private(s) => Some(*s),
        Visibility::Public => None,
    }
}

/// The effective per-store cap: the configured `max_size`, or the workspace
/// default ([`MAX_STORE_BYTES`]) when it is unset (`0`).
fn cap_of(max_size: u64) -> u64 {
    if max_size == 0 {
        MAX_STORE_BYTES
    } else {
        max_size
    }
}

/// A staged generation computed from a file set, WITHOUT persistence. Holding
/// this between [`build_prepared`] and [`finalize`] lets the CLI anchor `root`
/// on-chain (and BLOCK until confirmed) BEFORE any local persistence — so local
/// history never advances past the chain. Persists nothing on its own.
pub struct PreparedCommit {
    /// Generation merkle root over the ciphertext resource leaves (D5).
    pub root: Bytes32,
    /// Chunk ciphertext bodies, global pool order.
    pool_bodies: Vec<Vec<u8>>,
    /// SHA-256(chunk ciphertext) per body, same order (manifest/diff).
    pool_hashes: Vec<Bytes32>,
    /// (resource_key, chunk indices into the pool, plaintext total size).
    key_records: Vec<(String, Vec<u32>, u64)>,
    /// The generation id this commit will become.
    next_id: u64,
    /// Commit timestamp.
    timestamp: u64,
    /// The store id these resources belong to (for the key table URNs).
    store_id: Bytes32,
}

/// Compute the staged generation's merkle `root` + the in-memory state
/// [`finalize`] needs, WITHOUT persisting anything.
///
/// Each resource's chunks are AES-256-GCM-sealed under its per-URN key. The
/// served resource ciphertext is the PLAIN ordered concat of its chunk
/// ciphertexts (BINDING contract D5/C9: exactly what the guest's `get_content`
/// returns via `concat_output`). The generation merkle tree has ONE leaf per
/// resource: `leaf = SHA-256(concat_output(ordered chunk ciphertexts))`, so a
/// single `ContentResponse.merkle_proof` fully verifies the served bytes to the
/// root. Leaves are ordered ascending by `static_key` to match the compiler's
/// `current_generation_leaves` (D5), so the store-reported root equals the
/// module's injected `CurrentRoot` and the client gate `proof.root ==
/// trusted_root` holds.
///
/// When `pre_encrypted` is true each file's bytes are treated as ALREADY-SEALED
/// ciphertext (the client sealed it under the per-URN key before upload — the
/// server never sees plaintext or the key); the resource is stored as a SINGLE
/// chunk, skipping the chunk + encrypt step. The produced module/merkle/wire
/// format is otherwise identical.
///
/// This is byte-for-byte the logic the CLI used in `store_ops::build_prepared`.
pub fn build_prepared(
    files: &[(String, Vec<u8>)],
    store_id: Bytes32,
    visibility: &Visibility,
    max_size: u64,
    pre_encrypted: bool,
    next_id: u64,
    timestamp: u64,
) -> Result<PreparedCommit, StageError> {
    let salt = salt_of(visibility);

    if files.is_empty() {
        return Err(StageError::EmptyStaging);
    }

    // Defensive cap check (§3): refuse to compile content over the store's limit.
    let cap = cap_of(max_size);
    let staged_total: u64 = files.iter().map(|(_, c)| c.len() as u64).sum();
    if staged_total > cap {
        return Err(StageError::OverCap {
            got_mb: staged_total as f64 / 1_000_000.0,
            cap_mb: cap as f64 / 1_000_000.0,
        });
    }

    let mut pool_bodies: Vec<Vec<u8>> = Vec::new(); // chunk ciphertext bodies, global order
    let mut pool_hashes: Vec<Bytes32> = Vec::new(); // SHA-256(chunk ciphertext) (manifest/diff)
    let mut key_records: Vec<(String, Vec<u32>, u64)> = Vec::new();
    // (static_key, leaf) so we can sort leaves ascending by static_key (D5).
    let mut keyed_leaves: Vec<([u8; 32], Bytes32)> = Vec::new();

    for (resource_key, content) in files {
        let urn = canonical_resource_urn(store_id, resource_key);
        // Ordered CHUNK CIPHERTEXTS for this resource.
        let chunk_cts: Vec<Vec<u8>> = if pre_encrypted {
            // PRE-ENCRYPTED: the bytes ARE the resource's already-sealed ciphertext (the client
            // sealed it under the per-URN key; the server never sees plaintext or the key). Stored
            // as ONE chunk — D5 leaf = SHA-256(these bytes). No chunking, no encryption here.
            vec![content.clone()]
        } else {
            let aes_key = digstore_crypto::derive_decryption_key(&urn.canonical(), salt.as_ref());
            let chunks: Vec<Chunk> = chunk_slice(content, &chunker_config());
            let chunks = if chunks.is_empty() {
                vec![Chunk::new(0, Vec::new())]
            } else {
                chunks
            };
            chunks
                .iter()
                .map(|c| digstore_crypto::encrypt_chunk(&aes_key, &c.data))
                .collect()
        };
        let mut indices = Vec::with_capacity(chunk_cts.len());
        for ct in &chunk_cts {
            let h = digstore_crypto::sha256(ct);
            let idx = pool_bodies.len() as u32;
            pool_bodies.push(ct.clone());
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
        // Declared size: plaintext bytes. Pre-encrypted ciphertext carries a 16-byte GCM-SIV tag.
        let size = if pre_encrypted {
            content.len().saturating_sub(16) as u64
        } else {
            content.len() as u64
        };
        key_records.push((resource_key.clone(), indices, size));
    }

    // Ascending by static_key (raw 32 bytes; Bytes32 has no Ord) — the exact
    // order the compiler injects and the guest ranks against (D5).
    keyed_leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let resource_leaves: Vec<Bytes32> = keyed_leaves.into_iter().map(|(_, l)| l).collect();

    let tree = MerkleTree::from_leaves(resource_leaves);
    let root = tree.root();

    Ok(PreparedCommit {
        root,
        pool_bodies,
        pool_hashes,
        key_records,
        next_id,
        timestamp,
        store_id,
    })
}

/// The result of [`finalize`] / [`stage_and_compile`]: the produced capsule's
/// identity + the on-disk module artifact, plus the [`GenerationManifest`] (the
/// CLI uses it to write its local URN index — not part of the module bytes).
pub struct CompiledCapsule {
    /// The store id the capsule belongs to.
    pub store_id: Bytes32,
    /// The generation merkle root (the capsule's content version).
    pub root: Bytes32,
    /// The compiled `.dig` module on disk.
    pub module_path: PathBuf,
    /// The module's byte size.
    pub size: u64,
    /// The generation manifest (key table + chunk refs) for this root.
    pub manifest: GenerationManifest,
}

impl CompiledCapsule {
    /// The canonical capsule string identity `storeId:rootHash`
    /// (= `digstore_core::Capsule::canonical()`).
    pub fn capsule(&self) -> String {
        format!("{}:{}", self.store_id.to_hex(), self.root.to_hex())
    }

    /// The number of resources committed in this capsule.
    pub fn files(&self) -> usize {
        self.manifest.key_table.len()
    }
}

/// Where [`finalize`] writes the generation manifest + ciphertext chunk bodies
/// and the compiled module, and the serving identity baked into the module.
///
/// This mirrors the CLI's `.dig` layout: `<data_dir>/generations/<root>/…` and
/// `<data_dir>/modules/`. dig-node points these at a scratch dir under its cache.
pub struct FinalizeOptions {
    /// The store's data directory (the `.dig` dir). `generations/` and `modules/`
    /// live directly under it.
    pub data_dir: PathBuf,
    /// The TRUSTED serving host key set compiled into the module (§12.2).
    pub trusted_keys: Vec<TrustedHostKey>,
    /// The store's content-signing public key (compiled into the module).
    pub store_pubkey: Bytes48,
    /// The store-level metadata manifest embedded in the module's data section
    /// (Digstore §8.4, served ungated via the guest `get_metadata` export).
    pub metadata: MetadataManifest,
    /// Optional on-chain pointer to embed (the chainless path passes `None`).
    pub chain_state: Option<digstore_core::datasection::ChainState>,
    /// The per-store auth policy compiled into the module (§4.1/§5.2). Most
    /// stores want [`no_auth`]; a JWT/session-required store supplies its own.
    pub auth: AuthenticationInfo,
}

/// The explicit no-auth policy: a store requiring neither a session nor a JWT.
pub fn no_auth() -> AuthenticationInfo {
    AuthenticationInfo {
        requires_session: false,
        requires_jwt: false,
        jwks_url: None,
        accepted_algorithms: Vec::new(),
    }
}

/// Persist a [`PreparedCommit`] and compile its serving module.
///
/// Writes `<data_dir>/generations/<root>/{manifest.json,chunks/*}` and compiles
/// `<data_dir>/modules/<store>-<root>.dig`. Persists NOTHING else (no history,
/// no URN index, no staging clear — those are caller-owned presentation state).
/// The crypto/merkle/manifest bytes + compiled module are byte-for-byte what the
/// CLI produced before this extraction.
pub fn finalize(
    prepared: PreparedCommit,
    opts: &FinalizeOptions,
) -> Result<CompiledCapsule, StageError> {
    let PreparedCommit {
        root,
        pool_bodies,
        pool_hashes,
        key_records,
        next_id,
        timestamp,
        store_id,
    } = prepared;
    let root_hex = root.to_hex();
    let generations_dir = opts.data_dir.join("generations");

    // Persist the generation manifest + ciphertext chunk bodies.
    let chunks_dir = generations_dir.join(&root_hex).join("chunks");
    std::fs::create_dir_all(&chunks_dir)?;
    let mut chunk_refs = Vec::with_capacity(pool_bodies.len());
    for (i, (hash, body)) in pool_hashes.iter().zip(pool_bodies.iter()).enumerate() {
        std::fs::write(chunks_dir.join(hash.to_hex()), body)?;
        chunk_refs.push(ChunkRef {
            index: i as u32,
            hash: *hash,
            size: body.len() as u64,
        });
    }
    let key_table: Vec<KeyTableRecord> = key_records
        .iter()
        .map(|(rk, indices, total)| {
            let urn = canonical_resource_urn(store_id, rk);
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
        .write_to(generations_dir.join(&root_hex).join("manifest.json"))
        .map_err(|e| StageError::Compile(format!("write manifest: {e}")))?;

    // Compile a real module (so a real .wasm exists for host/push/clone).
    let output_path = compile_module(store_id, &pool_bodies, &manifest, root, opts)?;
    let output_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(CompiledCapsule {
        store_id,
        root,
        module_path: output_path,
        size: output_size,
        manifest,
    })
}

/// One-shot: [`build_prepared`] then [`finalize`]. The straight-through path the
/// in-process node uses to turn a file set into a capsule.
#[allow(clippy::too_many_arguments)]
pub fn stage_and_compile(
    files: &[(String, Vec<u8>)],
    store_id: Bytes32,
    visibility: &Visibility,
    max_size: u64,
    pre_encrypted: bool,
    next_id: u64,
    timestamp: u64,
    opts: &FinalizeOptions,
) -> Result<CompiledCapsule, StageError> {
    let prepared = build_prepared(
        files,
        store_id,
        visibility,
        max_size,
        pre_encrypted,
        next_id,
        timestamp,
    )?;
    finalize(prepared, opts)
}

/// Compile the generation into a real serving module via `digstore-compiler`,
/// using the embedded guest wasm as the compiler template (D6).
fn compile_module(
    store_id: Bytes32,
    pool_bodies: &[Vec<u8>],
    manifest: &GenerationManifest,
    root: Bytes32,
    opts: &FinalizeOptions,
) -> Result<PathBuf, StageError> {
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

    // The compiler writes the module into `output_dir`; in the CLI this dir is
    // created by `Store::init`, but the context-free engine must ensure it exists.
    let output_dir = opts.data_dir.join("modules");
    std::fs::create_dir_all(&output_dir)?;
    let ccfg = CompilerConfig {
        output_dir,
        obfuscate: false,
        optimize: false,
        // D6: compile with the REAL guest wasm so the module serves itself via
        // `HostRuntime::serve_content` (NOT the stub template).
        template_override: Some(embedded_guest_wasm().to_vec()),
        // §8.3 uniform-size filler budget: production pads to the 128 MiB default
        // (or the DIGSTORE_UNIFORM_BLOB_LEN override) so every store is one size.
        ..CompilerConfig::default()
    };
    let outcome = Compiler::compile(
        &ccfg,
        store_id,
        opts.store_pubkey,
        &[gen],
        opts.metadata.clone(),
        opts.auth.clone(),
        &opts.trusted_keys,
        opts.chain_state.clone(),
    )
    .map_err(|e| StageError::Compile(format!("{e:?}")))?;
    Ok(outcome.result.output_path)
}

/// Build a [`StoreConfig`] for an ephemeral/in-process stage (no on-disk store
/// scaffolding required by the engine — [`finalize`] writes only generations +
/// modules). Provided for callers that want to keep the config alongside.
pub fn ephemeral_config(store_id: Bytes32, visibility: Visibility, data_dir: &Path) -> StoreConfig {
    StoreConfig {
        store_id,
        data_dir: data_dir.display().to_string(),
        max_size: MAX_STORE_BYTES,
        visibility,
        label: None,
        description: None,
    }
}

/// An empty metadata manifest (the compiler requires one). The default for a
/// stage/compile with no `--metadata` / `metadata` param.
pub fn empty_manifest() -> MetadataManifest {
    MetadataManifest {
        schema_version: 1,
        name: String::new(),
        version: None,
        description: None,
        authors: Vec::new(),
        license: None,
        homepage: None,
        repository: None,
        keywords: Vec::new(),
        categories: Vec::new(),
        icon: None,
        content_type: None,
        links: Default::default(),
        custom: Default::default(),
    }
}

/// Build a [`MetadataManifest`] from the dighub `Manifest` JSON shape (the 14
/// publisher fields). Tolerant: missing/empty fields collapse to `None`/empty;
/// unknown keys are ignored except `custom`, which is preserved verbatim. This is
/// the inverse of the retrieval Lambda's `manifest_to_json`, so a round-trip
/// (.dig → RPC JSON → recompile) is stable. Shared by the CLI `compile` command
/// and the in-process `dig.stage` RPC (one parser, no fork).
pub fn manifest_from_json(v: &serde_json::Value) -> MetadataManifest {
    use digstore_core::Author;
    use std::collections::BTreeMap;

    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
    let opt = |k: &str| s(k).filter(|t| !t.is_empty());
    let arr_str = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(|t| t.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let authors = v
        .get("authors")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let name = e.get("name").and_then(|n| n.as_str())?.to_string();
                    Some(Author {
                        name,
                        handle: e
                            .get("handle")
                            .and_then(|h| h.as_str())
                            .map(|t| t.to_string()),
                        contact: e
                            .get("contact")
                            .and_then(|h| h.as_str())
                            .map(|t| t.to_string()),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let links = v
        .get("links")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, val)| val.as_str().map(|t| (k.clone(), t.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let custom = v
        .get("custom")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, val)| (k.clone(), val.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    MetadataManifest {
        schema_version: v
            .get("schema_version")
            .and_then(|x| x.as_u64())
            .unwrap_or(1) as u32,
        name: s("name").unwrap_or_default(),
        version: opt("version"),
        description: opt("description"),
        authors,
        license: opt("license"),
        homepage: opt("homepage"),
        repository: opt("repository"),
        keywords: arr_str("keywords"),
        categories: arr_str("categories"),
        icon: opt("icon"),
        content_type: opt("content_type"),
        links,
        custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn trusted_pubkey() -> (Vec<TrustedHostKey>, Bytes48) {
        // A deterministic BLS identity so tests are hermetic.
        let secret = digstore_crypto::bls::SecretKey::from_seed(&[7u8; 32]);
        let pk = secret.public_key().to_bytes();
        (
            vec![TrustedHostKey {
                public_key: pk.0,
                label: format!("test:{}", pk.to_hex()),
            }],
            pk,
        )
    }

    fn finalize_opts(data_dir: &Path) -> FinalizeOptions {
        let (trusted, pk) = trusted_pubkey();
        FinalizeOptions {
            data_dir: data_dir.to_path_buf(),
            trusted_keys: trusted,
            store_pubkey: pk,
            metadata: empty_manifest(),
            chain_state: None,
            auth: no_auth(),
        }
    }

    #[test]
    fn stage_and_compile_produces_a_real_module() {
        let td = tempdir().unwrap();
        let store_id = Bytes32([1u8; 32]);
        let files = vec![("index.html".to_string(), b"<h1>hi</h1>".to_vec())];
        let cap = stage_and_compile(
            &files,
            store_id,
            &Visibility::Public,
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td.path()),
        )
        .unwrap();
        assert!(cap.module_path.exists(), "module must be written to disk");
        assert!(cap.size > 0, "module must be non-empty");
        assert_ne!(
            cap.root,
            Bytes32([0u8; 32]),
            "root must be a real merkle root"
        );
        assert_eq!(cap.files(), 1);
        // The capsule string is the canonical storeId:rootHash.
        assert_eq!(
            cap.capsule(),
            format!("{}:{}", store_id.to_hex(), cap.root.to_hex())
        );
    }

    #[test]
    fn empty_file_set_is_rejected() {
        let td = tempdir().unwrap();
        let err = stage_and_compile(
            &[],
            Bytes32([1u8; 32]),
            &Visibility::Public,
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td.path()),
        );
        assert!(matches!(err, Err(StageError::EmptyStaging)));
    }

    #[test]
    fn over_cap_content_is_rejected() {
        let td = tempdir().unwrap();
        let files = vec![("big".to_string(), vec![0u8; 100])];
        let err = stage_and_compile(
            &files,
            Bytes32([1u8; 32]),
            &Visibility::Public,
            4, // 4-byte cap
            false,
            0,
            0,
            &finalize_opts(td.path()),
        );
        assert!(matches!(err, Err(StageError::OverCap { .. })));
    }

    #[test]
    fn same_inputs_produce_the_same_root() {
        // Determinism guard: the root is a content merkle root, so identical
        // files + store id + (public) visibility must reproduce the same root.
        let td1 = tempdir().unwrap();
        let td2 = tempdir().unwrap();
        let store_id = Bytes32([9u8; 32]);
        let files = vec![
            ("a.txt".to_string(), b"alpha".to_vec()),
            ("b.txt".to_string(), b"beta".to_vec()),
        ];
        let r1 = stage_and_compile(
            &files,
            store_id,
            &Visibility::Public,
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td1.path()),
        )
        .unwrap()
        .root;
        let r2 = stage_and_compile(
            &files,
            store_id,
            &Visibility::Public,
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td2.path()),
        )
        .unwrap()
        .root;
        assert_eq!(r1, r2);
    }

    #[test]
    fn pre_encrypted_resource_is_stored_as_one_chunk_and_size_excludes_gcm_tag() {
        // PRE-ENCRYPTED path: the bytes are treated as already-sealed ciphertext,
        // stored as a single chunk, and the declared plaintext size subtracts the
        // 16-byte GCM-SIV tag. This exercises the `pre_encrypted == true` branch of
        // `build_prepared` (the other tests only cover the plaintext path).
        let store_id = Bytes32([3u8; 32]);
        let ciphertext = vec![0xABu8; 80]; // 64 plaintext + 16-byte tag, conceptually
        let files = vec![("app.js".to_string(), ciphertext.clone())];
        let prepared = build_prepared(
            &files,
            store_id,
            &Visibility::Public,
            MAX_STORE_BYTES,
            true, // pre_encrypted
            0,
            0,
        )
        .unwrap();
        // One resource → one chunk in the pool (no chunking when pre-encrypted).
        assert_eq!(prepared.key_records.len(), 1);
        let (rk, indices, size) = &prepared.key_records[0];
        assert_eq!(rk, "app.js");
        assert_eq!(indices.len(), 1, "pre-encrypted resource is a single chunk");
        assert_eq!(*size, (80 - 16) as u64, "declared size strips the GCM tag");
    }

    #[test]
    fn empty_plaintext_resource_still_produces_a_chunk_and_a_leaf() {
        // A zero-byte file: `chunk_slice` yields no chunks, so the engine inserts a
        // single empty chunk (the `chunks.is_empty()` guard). The capsule must still
        // compile with one resource leaf.
        let td = tempdir().unwrap();
        let store_id = Bytes32([5u8; 32]);
        let files = vec![("empty.txt".to_string(), Vec::new())];
        let cap = stage_and_compile(
            &files,
            store_id,
            &Visibility::Public,
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td.path()),
        )
        .unwrap();
        assert_eq!(cap.files(), 1);
        assert!(cap.module_path.exists());
    }

    #[test]
    fn private_visibility_changes_the_root_vs_public() {
        // The per-URN AES key folds in the secret salt for a private store, so the
        // ciphertext leaves (and thus the root) differ from a public store of the
        // same files. Exercises the `Visibility::Private` salt branch.
        let td_pub = tempdir().unwrap();
        let td_priv = tempdir().unwrap();
        let store_id = Bytes32([6u8; 32]);
        let files = vec![("secret.txt".to_string(), b"classified".to_vec())];
        let public = stage_and_compile(
            &files,
            store_id,
            &Visibility::Public,
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td_pub.path()),
        )
        .unwrap()
        .root;
        let private = stage_and_compile(
            &files,
            store_id,
            &Visibility::Private(SecretSalt([42u8; 32])),
            MAX_STORE_BYTES,
            false,
            0,
            0,
            &finalize_opts(td_priv.path()),
        )
        .unwrap()
        .root;
        assert_ne!(
            public, private,
            "private salt must change the ciphertext leaves (and root)"
        );
    }

    #[test]
    fn ephemeral_config_defaults_to_the_workspace_cap() {
        // `ephemeral_config` is a small constructor helper; assert it wires the
        // store id, data dir, visibility, and the default cap.
        let td = tempdir().unwrap();
        let store_id = Bytes32([7u8; 32]);
        let cfg = ephemeral_config(store_id, Visibility::Public, td.path());
        assert_eq!(cfg.store_id, store_id);
        assert_eq!(cfg.data_dir, td.path().display().to_string());
        assert_eq!(cfg.max_size, MAX_STORE_BYTES);
        assert!(matches!(cfg.visibility, Visibility::Public));
        assert!(cfg.label.is_none());
    }

    #[test]
    fn canonical_resource_urn_drops_the_root_and_keys_on_store_plus_resource() {
        // The retrieval/AES URN is root-INDEPENDENT (the client reconstructs it
        // with the root dropped). Guard that invariant + the chain/store/key wiring.
        let store_id = Bytes32([8u8; 32]);
        let urn = canonical_resource_urn(store_id, "index.html");
        assert_eq!(urn.chain, CHAIN);
        assert_eq!(urn.store_id, store_id);
        assert!(urn.root_hash.is_none(), "URN must be root-independent");
        assert_eq!(urn.resource_key.as_deref(), Some("index.html"));
    }

    #[test]
    fn empty_manifest_and_no_auth_are_the_documented_neutral_defaults() {
        let m = empty_manifest();
        assert_eq!(m.schema_version, 1);
        assert!(m.name.is_empty());
        assert!(m.version.is_none());
        assert!(m.authors.is_empty());
        let a = no_auth();
        assert!(!a.requires_session);
        assert!(!a.requires_jwt);
        assert!(a.jwks_url.is_none());
        assert!(a.accepted_algorithms.is_empty());
    }

    // ---- manifest_from_json: the publisher-metadata parser (previously untested) --

    #[test]
    fn manifest_from_json_parses_all_fields() {
        let v = serde_json::json!({
            "schema_version": 2,
            "name": "My App",
            "version": "1.2.3",
            "description": "a test app",
            "authors": [
                { "name": "Alice", "handle": "@alice", "contact": "alice@example.com" },
                { "name": "Bob" }
            ],
            "license": "GPL-2.0",
            "homepage": "https://example.com",
            "repository": "https://github.com/x/y",
            "keywords": ["dig", "web"],
            "categories": ["tools"],
            "icon": "icon.png",
            "content_type": "text/html",
            "links": { "docs": "https://docs.example.com" },
            "custom": { "x": 1, "nested": { "k": "v" } }
        });
        let m = manifest_from_json(&v);
        assert_eq!(m.schema_version, 2);
        assert_eq!(m.name, "My App");
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
        assert_eq!(m.description.as_deref(), Some("a test app"));
        assert_eq!(m.authors.len(), 2);
        assert_eq!(m.authors[0].name, "Alice");
        assert_eq!(m.authors[0].handle.as_deref(), Some("@alice"));
        assert_eq!(m.authors[0].contact.as_deref(), Some("alice@example.com"));
        // Bob has no handle/contact → both None.
        assert_eq!(m.authors[1].name, "Bob");
        assert!(m.authors[1].handle.is_none());
        assert!(m.authors[1].contact.is_none());
        assert_eq!(m.license.as_deref(), Some("GPL-2.0"));
        assert_eq!(m.homepage.as_deref(), Some("https://example.com"));
        assert_eq!(m.repository.as_deref(), Some("https://github.com/x/y"));
        assert_eq!(m.keywords, vec!["dig", "web"]);
        assert_eq!(m.categories, vec!["tools"]);
        assert_eq!(m.icon.as_deref(), Some("icon.png"));
        assert_eq!(m.content_type.as_deref(), Some("text/html"));
        assert_eq!(m.links.get("docs").map(String::as_str), Some("https://docs.example.com"));
        // custom is preserved verbatim (including nested values).
        assert_eq!(m.custom.get("x"), Some(&serde_json::json!(1)));
        assert_eq!(m.custom.get("nested"), Some(&serde_json::json!({ "k": "v" })));
    }

    #[test]
    fn manifest_from_json_collapses_empty_and_missing_to_neutral() {
        // schema_version defaults to 1; empty strings collapse to None; absent
        // arrays/objects collapse to empty. This is the tolerant-parse contract.
        let v = serde_json::json!({
            "name": "",
            "version": "",        // empty → None (opt())
            "description": "",
            "keywords": "not-an-array", // wrong type → empty
        });
        let m = manifest_from_json(&v);
        assert_eq!(m.schema_version, 1, "default when absent");
        assert_eq!(m.name, "", "name uses unwrap_or_default (NOT opt)");
        assert!(m.version.is_none(), "empty string collapses to None");
        assert!(m.description.is_none());
        assert!(m.authors.is_empty());
        assert!(m.keywords.is_empty(), "non-array keywords → empty");
        assert!(m.categories.is_empty());
        assert!(m.links.is_empty());
        assert!(m.custom.is_empty());
        assert!(m.license.is_none());
    }

    #[test]
    fn manifest_from_json_skips_authors_without_a_name() {
        // An author entry MUST have a name; entries lacking one are dropped (the
        // `?` early-return inside the filter_map).
        let v = serde_json::json!({
            "name": "x",
            "authors": [
                { "handle": "@nameless" },        // no name → dropped
                { "name": "Carol", "handle": 5 }, // non-string handle → handle None
            ]
        });
        let m = manifest_from_json(&v);
        assert_eq!(m.authors.len(), 1);
        assert_eq!(m.authors[0].name, "Carol");
        assert!(m.authors[0].handle.is_none());
    }

    #[test]
    fn manifest_from_json_links_ignores_non_string_values() {
        let v = serde_json::json!({
            "name": "x",
            "links": { "a": "https://a", "b": 123, "c": null }
        });
        let m = manifest_from_json(&v);
        assert_eq!(m.links.len(), 1, "only string-valued links are kept");
        assert_eq!(m.links.get("a").map(String::as_str), Some("https://a"));
    }
}
