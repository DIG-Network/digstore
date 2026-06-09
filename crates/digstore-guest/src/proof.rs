//! Proof path (§13). CONVENTIONS C3: the guest CANNOT build an `ExecutionProof`
//! (no prover, no ChainSource, no node signing key inside wasm32). `get_proof`
//! therefore returns a serialized `digstore_core::wire::ProofPrelude` whose
//! fields are: `roothash` (the generation root being served against),
//! `output_commitment` (SHA-256 of the served output bytes — the same bytes
//! `get_content` returns, ordered via `concat_output`), and `serving_digest`
//! (a commitment over the retrieval_key, ordered chunk indices, and client_nonce).
//!
//! The serving host later wraps this prelude into a full `ExecutionProof` via
//! `digstore_prover` (and signs `node_signature`). On a miss the guest returns a
//! deterministic decoy prelude (indistinguishable success).

use crate::content::GateConfig;
use crate::datasection::{DataSection, SectionId};
use crate::host::DigHost;
use crate::oblivious::build_access_plan;
use crate::request::ProofRequest;
use alloc::vec::Vec;
use digstore_core::serving::concat_output;
use digstore_core::{Bytes32, ProofPrelude};
use sha2::{Digest, Sha256};

pub enum ProofOutcome {
    Real(ProofPrelude),
    Decoy(ProofPrelude),
}

fn sha256(bytes: &[u8]) -> Bytes32 {
    let mut h = Sha256::new();
    h.update(bytes);
    let mut o = [0u8; 32];
    o.copy_from_slice(&h.finalize());
    Bytes32(o)
}

/// Read chunk ciphertext at `index` from the ChunkPool section.
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

/// Deterministic decoy prelude derived from the retrieval key + root, success-shaped.
fn decoy_prelude(rk: &Bytes32, root: &Bytes32) -> ProofPrelude {
    let blob = crate::decoy::decoy_proof_blob(rk);
    let mut oc = Sha256::new();
    oc.update(b"digstore-decoy-output-v1");
    oc.update(rk.0);
    let mut output_commitment = [0u8; 32];
    output_commitment.copy_from_slice(&oc.finalize());
    let mut sd = Sha256::new();
    sd.update(b"digstore-decoy-serving-v1");
    sd.update(&blob);
    let mut serving_digest = [0u8; 32];
    serving_digest.copy_from_slice(&sd.finalize());
    ProofPrelude {
        roothash: *root,
        output_commitment: Bytes32(output_commitment),
        serving_digest: Bytes32(serving_digest),
    }
}

/// Build the `ProofPrelude` for a request (CONVENTIONS C3). The gate is applied
/// like the content path so a gate failure / miss returns a decoy prelude.
pub fn serve_proof<H: DigHost + ?Sized>(
    host: &H,
    ds: &DataSection,
    req: &ProofRequest,
    _cfg: &GateConfig,
) -> ProofOutcome {
    let root = req.root_hash.unwrap_or_else(|| ds.current_root());
    let entry = match ds.lookup_key(&req.retrieval_key) {
        Some(e) => e,
        None => return ProofOutcome::Decoy(decoy_prelude(&req.retrieval_key, &root)),
    };

    // Oblivious gather of the served bytes, ordered via C9 `concat_output`, so the
    // output_commitment matches what `get_content` returns and the prover re-execs.
    let pool = ds.section(SectionId::ChunkPool).unwrap_or(&[]);
    let pool_size = if pool.len() >= 4 {
        u32::from_be_bytes([pool[0], pool[1], pool[2], pool[3]])
    } else {
        0
    };
    let plan = build_access_plan(&entry.chunk_indices, pool_size, |c| {
        host.random_bytes(c)
            .unwrap_or_else(|_| alloc::vec![0u8; c as usize])
    });
    let mut gathered: Vec<Vec<u8>> = Vec::with_capacity(plan.order.len());
    for idx in &plan.order {
        gathered.push(read_chunk(ds, *idx).unwrap_or_default());
    }
    let real_slices: Vec<&[u8]> = plan
        .real_positions
        .iter()
        .map(|pos| gathered[*pos].as_slice())
        .collect();
    let output = concat_output(&real_slices);
    let output_commitment = sha256(&output);

    // serving_digest = SHA-256(retrieval_key || ordered chunk indices (BE) ||
    // client_nonce). Binds the request nonce (§13.5 analog) and the served slice
    // ordering so the host's wrapper proof is tied to exactly this serving.
    let mut sd = Sha256::new();
    sd.update(req.retrieval_key.0);
    for idx in &entry.chunk_indices {
        sd.update(idx.to_be_bytes());
    }
    sd.update(req.client_nonce);
    let mut serving_digest = [0u8; 32];
    serving_digest.copy_from_slice(&sd.finalize());

    ProofOutcome::Real(ProofPrelude {
        roothash: root,
        output_commitment,
        serving_digest: Bytes32(serving_digest),
    })
}
