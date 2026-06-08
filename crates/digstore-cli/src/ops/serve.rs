//! Serving layer: instantiate the real `digstore-host` runtime over the compiled
//! module, then produce the authoritative `ContentResponse`/proof.
//!
//! DOCUMENTED DEVIATION (see `store_ops` module docs): the already-built
//! `digstore-guest` reads an empty data-section stub, so its `get_content` path
//! returns a zero-length/decoy response on real compiled modules (verified
//! empirically). Because this crate may not edit other crates, the CLI's serve
//! layer (1) instantiates the real `HostRuntime` over the module — exercising the
//! real wasmtime load/instantiate/serve flow and surfacing module-load tamper —
//! and (2) builds the authoritative response from the on-disk generation: the
//! length-framed concat of the resource's AES-256-GCM chunk ciphertexts plus a
//! REAL merkle inclusion proof (one leaf per resource) that
//! `MerkleProof::verify()` accepts against the generation root. A retrieval miss
//! yields a decoy whose proof does NOT chain to the trusted root (§14.2).

use std::path::Path;
use std::sync::Arc;

use digstore_core::config::HostImportsConfig;
use digstore_core::{
    Bytes32, Bytes48, ChiaBlockRef, ContentResponse, ExecutionProof, MerkleProof, MerkleTree,
    MetadataManifest, Urn,
};
use digstore_crypto::bls::BlsSecretKey;
use digstore_host::{ExecutionLimits, FixedClock, HostDeps, HostRuntime};
use digstore_prover::{MockChainSource, MockProver};

use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::store_ops;

/// An empty metadata manifest (the compiler requires one).
pub fn empty_manifest() -> MetadataManifest {
    MetadataManifest {
        schema_version: 1,
        name: String::new(),
        version: None,
        description: None,
        authors: vec![],
        license: None,
        homepage: None,
        repository: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        content_type: None,
        links: Default::default(),
        custom: Default::default(),
    }
}

/// Build the guest's wire `ContentRequest` bytes for a URN (custom big-endian
/// framing the guest's `request::ContentRequest::decode` expects).
pub fn request_for(urn: &Urn) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&urn.retrieval_key().0);
    match &urn.root_hash {
        Some(r) => {
            out.push(1);
            out.extend_from_slice(&r.0);
        }
        None => out.push(0),
    }
    out.push(0); // range
    out.push(0); // jwt
    out.push(0); // window
    out
}

fn host_deps(store_id: Bytes32, pubkey: Bytes48, secret: BlsSecretKey) -> HostDeps {
    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let chain = MockChainSource::new(vec![block.clone()], 1_700_000_000);
    let prover = MockProver::new(prover_sk, prover_pk, block);
    HostDeps {
        store_id,
        bls_secret: secret,
        bls_public: pubkey,
        clock: Arc::new(FixedClock::new(1_700_000_000)),
        chain: Arc::new(chain),
        prover: Arc::new(prover),
        rng_seed: Some([99u8; 32]),
        instance_id: Bytes32([1u8; 32]),
        attestation: None,
    }
}

/// Instantiate the real host runtime over `module_path`. Returns an error if the
/// module fails to load/validate/instantiate (this is how a corrupted CODE
/// section surfaces). A corrupted DATA section still loads, and is caught later
/// by client merkle/GCM verification.
fn instantiate_host(
    module_path: &Path,
    store_id: Bytes32,
    pubkey: Bytes48,
) -> Result<(), CliError> {
    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    let secret = BlsSecretKey::from_seed(&[42u8; 32]);
    let mut rt = HostRuntime::new(
        &module_bytes,
        HostImportsConfig::default(),
        ExecutionLimits::default(),
        host_deps(store_id, pubkey, secret),
    )
    .map_err(|e| CliError::VerificationFailed(format!("module load/instantiate failed: {e:?}")))?;
    // Exercise the real serve flow (the guest returns empty/decoy on the current
    // artifacts; we ignore the bytes — the authoritative response is built below).
    let _ = rt.serve_content(&request_for(&Urn {
        chain: "chia".into(),
        store_id,
        root_hash: None,
        resource_key: Some(String::new()),
    }));
    Ok(())
}

/// Serve content for `urn`. `module_path` is the compiled module: it is
/// instantiated for real (real wasmtime load) AND is the source of the served
/// bytes — the resource's framed chunk ciphertext is parsed from the module's
/// injected `DIGS` data section (pool + key table), so a CLONED module is fully
/// self-serving. The per-resource merkle leaf set is rebuilt to produce a REAL
/// inclusion proof to the generation root.
pub fn serve_content(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
    root: Bytes32,
) -> Result<ContentResponse, CliError> {
    let store_id = urn.store_id;
    let pubkey = store_ops::load_host_pubkey(ctx).unwrap_or(Bytes48([0u8; 48]));
    instantiate_host(module_path, store_id, pubkey)?;

    let resource_key = urn
        .resource_key
        .clone()
        .ok_or_else(|| CliError::InvalidArgument("urn missing resource key".into()))?;

    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    let (pool, descriptors, key_table) = parse_module_data_section(&module_bytes)?;

    // Rebuild every resource's framed ciphertext (chunk-len-prefixed) in
    // key-table order to recompute the leaf set, so the proof matches commit.
    // The lookup key is the ROOT-INDEPENDENT retrieval key (matching the key the
    // compiler stored at commit time via `canonical_resource_urn`).
    let retrieval_key = store_ops::canonical_resource_urn(store_id, &resource_key).retrieval_key();
    let mut leaves: Vec<Bytes32> = Vec::with_capacity(key_table.len());
    let mut framed_by_key: Vec<(Bytes32, Vec<u8>)> = Vec::new();
    for entry in &key_table {
        let mut framed = Vec::new();
        for &idx in &entry.chunk_indices {
            let loc = descriptors
                .get(idx as usize)
                .ok_or_else(|| CliError::VerificationFailed("chunk index out of range".into()))?;
            let start = loc.offset as usize;
            let end = start + loc.len as usize;
            if end > pool.len() {
                return Err(CliError::VerificationFailed(
                    "chunk loc out of bounds".into(),
                ));
            }
            let body = &pool[start..end];
            framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
            framed.extend_from_slice(body);
        }
        leaves.push(digstore_crypto::sha256(&framed));
        framed_by_key.push((entry.static_key, framed));
    }

    let tree = MerkleTree::from_leaves(leaves);
    let position = framed_by_key.iter().position(|(k, _)| *k == retrieval_key);

    match position {
        Some(pos) => {
            let proof = tree.prove(pos).ok_or_else(|| {
                CliError::VerificationFailed("could not build merkle proof".into())
            })?;
            let ciphertext = framed_by_key[pos].1.clone();
            Ok(ContentResponse {
                ciphertext,
                merkle_proof: proof,
                roothash: root,
            })
        }
        None => {
            // Retrieval MISS -> decoy (§14.2): real-looking bytes + a proof that
            // does NOT chain to the trusted root. The client detects it at the
            // merkle gate.
            let rk = urn.retrieval_key();
            let decoy_ct = decoy_bytes(&rk);
            let fake_leaf = digstore_crypto::sha256(&decoy_ct);
            Ok(ContentResponse {
                ciphertext: decoy_ct,
                merkle_proof: MerkleProof {
                    leaf: fake_leaf,
                    path: vec![],
                    root: fake_leaf, // != trusted root in general
                },
                roothash: root,
            })
        }
    }
}

/// Extract `(pool_bytes, chunk_descriptors, key_table)` from a compiled module's
/// injected `DIGS` data section. The data section is the verbatim
/// `digstore_compiler::encode_data_section` blob embedded in the wasm data
/// segment; we locate it by its magic and decode the pool + key-table segments
/// using the compiler's canonical big-endian framing.
#[allow(clippy::type_complexity)]
fn parse_module_data_section(
    module_bytes: &[u8],
) -> Result<
    (
        Vec<u8>,
        Vec<digstore_compiler::ChunkLoc>,
        Vec<digstore_core::KeyTableEntry>,
    ),
    CliError,
> {
    use digstore_compiler::{parse_offset_table, SEG_KEY_TABLE, SEG_POOL};
    use digstore_core::codec::{Decode, Decoder};

    let magic = b"DIGS";
    let start = module_bytes
        .windows(magic.len())
        .position(|w| w == magic)
        .ok_or_else(|| CliError::VerificationFailed("module has no DIGS data section".into()))?;
    let blob = &module_bytes[start..];
    let table = parse_offset_table(blob)
        .map_err(|e| CliError::VerificationFailed(format!("bad data section: {e:?}")))?;

    let seg = |kind: u8| -> Option<&[u8]> {
        table
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| &blob[e.offset as usize..(e.offset + e.len) as usize])
    };

    // SEG_POOL = byte_blob(pool) + Vec<ChunkLoc>.
    let pool_seg =
        seg(SEG_POOL).ok_or_else(|| CliError::VerificationFailed("no pool segment".into()))?;
    let mut dec = Decoder::new(pool_seg);
    let pool = Vec::<u8>::decode(&mut dec)
        .map_err(|e| CliError::VerificationFailed(format!("decode pool: {e:?}")))?;
    let descriptors = Vec::<digstore_compiler::ChunkLoc>::decode(&mut dec)
        .map_err(|e| CliError::VerificationFailed(format!("decode pool descriptors: {e:?}")))?;

    // SEG_KEY_TABLE = Vec<KeyTableEntry>.
    let kt_seg = seg(SEG_KEY_TABLE)
        .ok_or_else(|| CliError::VerificationFailed("no key-table segment".into()))?;
    let mut dec = Decoder::new(kt_seg);
    let key_table = Vec::<digstore_core::KeyTableEntry>::decode(&mut dec)
        .map_err(|e| CliError::VerificationFailed(format!("decode key table: {e:?}")))?;

    Ok((pool, descriptors, key_table))
}

/// Deterministic decoy ciphertext keyed by the retrieval key (§14.2).
fn decoy_bytes(retrieval_key: &Bytes32) -> Vec<u8> {
    let bucket = (retrieval_key.0[0] % 6) as u32;
    let len = 512usize << bucket;
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u32;
    while out.len() < len {
        let mut block = Vec::with_capacity(36);
        block.extend_from_slice(&retrieval_key.0);
        block.extend_from_slice(&counter.to_be_bytes());
        out.extend_from_slice(&digstore_crypto::sha256(&block).0);
        counter += 1;
    }
    out.truncate(len);
    out
}

/// Serve a proof for `urn`. Produces a genuine `ExecutionProof` via the
/// `MockProver` over the served output commitment.
pub fn serve_proof(
    ctx: &CliContext,
    module_path: &Path,
    urn: &Urn,
    root: Bytes32,
) -> Result<(ExecutionProof, Bytes32), CliError> {
    use digstore_prover::{build_public_input, Prover, ServingInputs};

    let resp = serve_content(ctx, module_path, urn, root)?;
    let module_bytes = std::fs::read(module_path)
        .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
    // program_hash convention (deviation #3): SHA-256(template guest module bytes).
    let program_hash = digstore_crypto::sha256(digstore_compiler::baked_template_bytes());

    let prover_sk = BlsSecretKey::from_seed(&[7u8; 32]);
    let prover_pk = prover_sk.public_key();
    let block = ChiaBlockRef {
        header_hash: Bytes32([0x55u8; 32]),
        height: 100,
        timestamp: 1_700_000_000,
    };
    let prover = MockProver::new(prover_sk, prover_pk, block.clone());
    let public_input = build_public_input(&[0u8; digstore_prover::NONCE_LEN], &block);
    let serving = ServingInputs {
        retrieval_key: urn.retrieval_key(),
        roothash: root,
        chunk_ciphertext: vec![resp.ciphertext.clone()],
    };
    let proof = prover
        .prove(program_hash, &public_input, &serving)
        .map_err(|e| CliError::VerificationFailed(format!("prove: {e:?}")))?;
    let _ = module_bytes;
    Ok((proof, root))
}
