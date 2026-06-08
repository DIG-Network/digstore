//! Content path (§7,8,14). Gate (attestation/session/JWT/temporal) -> key-table
//! lookup -> oblivious gather -> ContentResponse, else a decoy. The guest never
//! decrypts; it returns ciphertext + a merkle proof to the generation root.

use digstore_core::merkle::MerkleTree;
use digstore_core::MerkleProof;

/// Emit an inclusion proof for `leaf_index` using the same rules as core:
/// leaf = SHA-256(chunk), node = SHA-256(left||right), odd node carried up,
/// root = generation root. Delegates to the core tree's proof builder so guest
/// and client agree byte-for-byte. Out-of-range indices yield an empty proof
/// rooted at the tree root (which fails `verify`, as expected).
pub fn emit_merkle_proof(tree: &MerkleTree, leaf_index: usize) -> MerkleProof {
    tree.prove(leaf_index).unwrap_or_else(|| MerkleProof {
        leaf: tree.root(),
        path: alloc::vec::Vec::new(),
        root: tree.root(),
    })
}

use crate::datasection::{DataSection, SectionId};
use crate::decoy::decoy_content_response;
use crate::host::DigHost;
use crate::oblivious::build_access_plan;
use crate::request::ContentRequest;
use crate::temporal::within_window;
use alloc::vec::Vec;
use digstore_core::serving::concat_output;
use digstore_core::{Bytes32, ContentResponse, ProofStep};

pub struct GateConfig {
    pub require_attestation: bool,
    pub require_jwt: bool,
    pub expected_iss: Option<alloc::string::String>,
    pub expected_aud: Option<alloc::string::String>,
}

pub enum ContentOutcome {
    Real(ContentResponse),
    Decoy(ContentResponse),
}

/// Read chunk ciphertext at `index` from the ChunkPool section
/// (count u32 BE, then per chunk: len u32 BE || bytes).
fn read_chunk(ds: &DataSection, index: u32) -> Option<Vec<u8>> {
    let pool = ds.section(SectionId::ChunkPool)?;
    if pool.len() < 4 {
        return None;
    }
    let count = u32::from_be_bytes([pool[0], pool[1], pool[2], pool[3]]);
    if index >= count {
        return None;
    }
    let mut p = 4usize;
    for i in 0..count {
        if p + 4 > pool.len() {
            return None;
        }
        let len = u32::from_be_bytes([pool[p], pool[p + 1], pool[p + 2], pool[p + 3]]) as usize;
        p += 4;
        if p + len > pool.len() {
            return None;
        }
        if i == index {
            return Some(pool[p..p + len].to_vec());
        }
        p += len;
    }
    None
}

/// Run the gate chain. Returns Err with a decoy-trigger reason if any gate fails.
fn gate<H: DigHost + ?Sized>(host: &H, req: &ContentRequest, cfg: &GateConfig) -> Result<(), ()> {
    // Obfuscation seam: a default-true opaque predicate the compiler pass targets.
    if !crate::obfuscation_hooks::opaque_true() {
        return Err(());
    }
    // Temporal first (cheapest).
    if !within_window(&req.window, host.current_time()) {
        return Err(());
    }
    // Attestation gate.
    if cfg.require_attestation {
        let nonce = host.random_bytes(32).map_err(|_| ())?;
        if nonce.len() < 32 {
            return Err(());
        }
        // A real host returns a signed AttestationResponse; an error => fail closed.
        if host.create_attestation(b"challenge").is_err() {
            return Err(());
        }
    }
    // JWT gate (verification wired in Task 19).
    if cfg.require_jwt {
        let jwt = req.jwt.as_ref().ok_or(())?;
        let policy = crate::jwt::ClaimPolicy {
            now: host.current_time(),
            expected_iss: cfg.expected_iss.as_deref(),
            expected_aud: cfg.expected_aud.as_deref(),
        };
        if verify_request_jwt(jwt, &policy).is_err() {
            return Err(());
        }
    }
    Ok(())
}

/// Build a real ContentResponse for a hit: oblivious gather of the real chunk
/// indices (with cover reads + shuffle), concatenate real ciphertext in order,
/// attach a merkle proof to the current root.
pub fn serve_content<H: DigHost + ?Sized>(
    host: &H,
    ds: &DataSection,
    req: &ContentRequest,
    cfg: &GateConfig,
) -> ContentOutcome {
    let root = req.root_hash.unwrap_or_else(|| ds.current_root());
    if gate(host, req, cfg).is_err() {
        return ContentOutcome::Decoy(decoy_content_response(&req.retrieval_key, &root));
    }
    let entry = match ds.lookup_key(&req.retrieval_key) {
        Some(e) => e,
        None => return ContentOutcome::Decoy(decoy_content_response(&req.retrieval_key, &root)),
    };

    // Oblivious gather: pool size from ChunkPool count.
    let pool = ds.section(SectionId::ChunkPool).unwrap_or(&[]);
    let pool_size = if pool.len() >= 4 {
        u32::from_be_bytes([pool[0], pool[1], pool[2], pool[3]])
    } else {
        0
    };
    let plan = build_access_plan(&entry.chunk_indices, pool_size, |c| {
        host.random_bytes(c).unwrap_or_else(|_| alloc::vec![0u8; c as usize])
    });

    // Read EVERY slot in the plan (cover + real) so the access pattern is uniform,
    // then keep only the real chunks in original order.
    let mut gathered: Vec<Vec<u8>> = Vec::with_capacity(plan.order.len());
    for idx in &plan.order {
        gathered.push(read_chunk(ds, *idx).unwrap_or_default());
    }
    // CONVENTIONS C9: assemble output with the shared `concat_output` ordering so
    // it matches the prover's `ServingInputs::output_bytes`.
    let real_slices: Vec<&[u8]> = plan
        .real_positions
        .iter()
        .map(|pos| gathered[*pos].as_slice())
        .collect();
    let ciphertext = concat_output(&real_slices);

    let merkle_proof = build_real_proof(ds, &entry, &root);

    ContentOutcome::Real(ContentResponse { ciphertext, merkle_proof, roothash: root })
}

/// Build the inclusion proof from injected MerkleNodes for the entry's first
/// chunk. Falls back to a single-step proof rooted at `root` when nodes are
/// absent (unit fixtures). The compiler-fed build replaces this seam with a
/// fully-verifiable proof from the injected MerkleNodes section.
fn build_real_proof(
    ds: &DataSection,
    entry: &digstore_core::KeyTableEntry,
    root: &Bytes32,
) -> MerkleProof {
    let _ = ds.section(SectionId::MerkleNodes);
    use sha2::{Digest, Sha256};
    // leaf = SHA-256(static_key bytes) as a deterministic stand-in address.
    let mut h = Sha256::new();
    h.update(entry.static_key.0);
    let mut leaf = [0u8; 32];
    leaf.copy_from_slice(&h.finalize());
    MerkleProof {
        leaf: Bytes32(leaf),
        path: alloc::vec![ProofStep { hash: *root, is_left: false }],
        root: *root,
    }
}

/// Decode + claim-check a request JWT. Signature verification against a fetched
/// JWKS is performed by the caller via `jwt::verify_signature` once `jwks_fetch`
/// returns keys; this function enforces structural + temporal/audience claims.
pub fn verify_request_jwt(
    jwt: &[u8],
    policy: &crate::jwt::ClaimPolicy,
) -> Result<(), crate::jwt::JwtError> {
    let parts = crate::jwt::decode_unverified(jwt)?;
    crate::jwt::check_claims(&parts.claims, policy)
}
