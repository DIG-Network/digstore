//! Deterministic decoys (§14.2). On a retrieval miss the guest returns
//! real-looking bytes whose size follows a logarithmic distribution seeded by
//! the retrieval key, and a real-looking (but unverifiable) proof blob, with a
//! success status. Same miss -> same bytes (DOC DEVIATION 2 rationale: filler
//! determinism). Stream = ChaCha20 keyed by SHA-256(retrieval_key || tag).
//!
//! The size mapping uses integer arithmetic (no float `ln`/`exp`) so it is
//! `no_std`-clean on wasm32: the seed selects one of N log-spaced buckets
//! (each ~2x the previous) and a deterministic offset within that bucket.

use alloc::vec;
use alloc::vec::Vec;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use digstore_core::Bytes32;
use sha2::{Digest, Sha256};

const MIN_SIZE: usize = 1024;
const MAX_SIZE: usize = 256 * 1024;
/// Number of doublings from MIN_SIZE (1KiB) to MAX_SIZE (256KiB) = 8 octaves.
const OCTAVES: u64 = 8;

fn seed(retrieval_key: &Bytes32, tag: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(retrieval_key.0);
    h.update(tag);
    let out = h.finalize();
    let mut s = [0u8; 32];
    s.copy_from_slice(&out);
    s
}

fn stream(seed: [u8; 32], len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    let nonce = [0u8; 12]; // unique key per (retrieval_key,tag) => fixed nonce safe here
    let mut c = ChaCha20::new(&seed.into(), &nonce.into());
    c.apply_keystream(&mut buf);
    buf
}

/// Logarithmic size in [MIN_SIZE, MAX_SIZE], deterministic per retrieval key.
///
/// Maps an 8-byte seed fraction through OCTAVES log-spaced bands: the high bits
/// pick the octave (size doubles per octave), the low bits pick a uniform offset
/// inside that octave's [2^k, 2^(k+1)) window. The result spreads across the
/// whole band rather than collapsing to a few values.
pub fn decoy_size(retrieval_key: &Bytes32) -> usize {
    let s = seed(retrieval_key, b"digstore-decoy-size-v1");
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&s[0..8]);
    let v = u64::from_be_bytes(raw);

    // octave in [0, OCTAVES); within-octave fraction taken from the low 32 bits.
    let octave = (v >> 40) % OCTAVES; // top bits choose the octave
    let frac = v & 0xFFFF_FFFF; // low 32 bits choose the offset

    let band_lo = (MIN_SIZE as u64) << octave; // 2^octave * MIN
    let band_hi = (band_lo * 2).min(MAX_SIZE as u64);
    let span = band_hi - band_lo;
    let offset = if span == 0 { 0 } else { (frac * span) >> 32 };
    let size = (band_lo + offset) as usize;
    size.clamp(MIN_SIZE, MAX_SIZE)
}

/// Deterministic decoy ciphertext of `decoy_size` bytes.
pub fn decoy_bytes(retrieval_key: &Bytes32) -> Vec<u8> {
    let n = decoy_size(retrieval_key);
    stream(seed(retrieval_key, b"digstore-decoy-bytes-v1"), n)
}

/// A real-looking proof blob (opaque bytes shaped like a serialized proof).
pub fn decoy_proof_blob(retrieval_key: &Bytes32) -> Vec<u8> {
    stream(seed(retrieval_key, b"digstore-decoy-proof-v1"), 256)
}

use digstore_core::{ContentResponse, MerkleProof, ProofStep};

/// Build a decoy `ContentResponse` with the SAME field shape as a real hit:
/// deterministic ciphertext + a structurally-real (but unverifiable) merkle
/// proof + the requested root. Indistinguishable on the wire from a real hit.
pub fn decoy_content_response(retrieval_key: &Bytes32, root: &Bytes32) -> ContentResponse {
    let ciphertext = decoy_bytes(retrieval_key);
    let leaf_seed = seed(retrieval_key, b"digstore-decoy-leaf-v1");
    let step_seed = seed(retrieval_key, b"digstore-decoy-step-v1");
    let path = alloc::vec![ProofStep {
        hash: Bytes32(step_seed),
        is_left: (step_seed[0] & 1) == 1,
    }];
    let merkle_proof = MerkleProof {
        leaf: Bytes32(leaf_seed),
        path,
        root: *root,
    };
    ContentResponse { ciphertext, merkle_proof, roothash: *root }
}
