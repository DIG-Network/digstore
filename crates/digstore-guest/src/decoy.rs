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

/// Maps a 64-bit seed word to its log-spaced size in `[MIN_SIZE, MAX_SIZE]`.
///
/// The top 3 bits (`v >> 61`) choose the octave (size doubles per octave), and
/// the low 32 bits choose a uniform offset inside that octave's
/// `[2^k, 2^(k+1))` window. With `OCTAVES == 8` the top 3 bits cover exactly
/// the octave range, so no seed bits are dead: bits 32..=60 are reserved/unused
/// only because they fall between the offset's low 32 bits and the octave's top
/// 3 bits, which is intentional headroom rather than an addressing gap.
fn size_of(v: u64) -> usize {
    // octave in [0, OCTAVES); top 3 bits choose the octave.
    let octave = (v >> 61) % OCTAVES; // top bits choose the octave
    let frac = v & 0xFFFF_FFFF; // low 32 bits choose the offset

    let band_lo = (MIN_SIZE as u64) << octave; // 2^octave * MIN
    let band_hi = (band_lo * 2).min(MAX_SIZE as u64);
    let span = band_hi - band_lo;
    let offset = if span == 0 { 0 } else { (frac * span) >> 32 };
    let size = (band_lo + offset) as usize;
    size.clamp(MIN_SIZE, MAX_SIZE)
}

/// Logarithmic size in [MIN_SIZE, MAX_SIZE], deterministic per retrieval key.
///
/// Maps an 8-byte seed fraction through OCTAVES log-spaced bands via
/// [`size_of`]: the top 3 bits pick the octave (size doubles per octave), the
/// low 32 bits pick a uniform offset inside that octave's `[2^k, 2^(k+1))`
/// window. The result spreads across the whole band rather than collapsing to a
/// few values.
pub fn decoy_size(retrieval_key: &Bytes32) -> usize {
    let s = seed(retrieval_key, b"digstore-decoy-size-v1");
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&s[0..8]);
    let v = u64::from_be_bytes(raw);
    size_of(v)
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

/// Target chunk size of the real content-defined chunker (digstore-chunker config). A decoy's
/// `chunk_lens` is split around this so a miss is indistinguishable from a real multi-chunk
/// resource of the same total size (a single-chunk decoy of a 200 KiB body would otherwise stand
/// out against a real ~64 KiB-chunked resource).
const CHUNK_TARGET: usize = 64 * 1024;

/// Plausible per-chunk CIPHERTEXT lengths for a decoy of `total` bytes, deterministic per key.
/// Bodies at/under the target stay a single chunk (the real chunker also emits one chunk below a
/// boundary); larger bodies split into pseudo-random ~target-sized chunks summing to `total`.
fn decoy_chunk_lens(retrieval_key: &Bytes32, total: usize) -> Vec<u32> {
    if total == 0 {
        return Vec::new();
    }
    if total <= CHUNK_TARGET {
        return vec![total as u32];
    }
    let bytes = stream(
        seed(retrieval_key, b"digstore-decoy-chunklens-v1"),
        4 * (total / CHUNK_TARGET + 2),
    );
    let mut lens: Vec<u32> = Vec::new();
    let mut rem = total;
    let mut i = 0usize;
    while rem > CHUNK_TARGET {
        let r = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        i += 4;
        // Chunk size in [TARGET/2, 3*TARGET/2) so the mean is ~TARGET; never leave a sub-half-
        // target tail (merge it into this chunk) so the final remainder is itself plausible.
        let mut size = CHUNK_TARGET / 2 + (r % CHUNK_TARGET);
        if rem.saturating_sub(size) < CHUNK_TARGET / 2 {
            size = rem;
        }
        lens.push(size as u32);
        rem -= size;
    }
    if rem > 0 {
        lens.push(rem as u32);
    }
    lens
}

/// Build a decoy `ContentResponse` with the SAME field shape as a real hit:
/// deterministic ciphertext + a structurally-real (but unverifiable) merkle
/// proof + the requested root + plausible per-chunk lengths. Indistinguishable
/// on the wire from a real hit.
pub fn decoy_content_response(retrieval_key: &Bytes32, root: &Bytes32) -> ContentResponse {
    let ciphertext = decoy_bytes(retrieval_key);
    let chunk_lens = decoy_chunk_lens(retrieval_key, ciphertext.len());
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
    ContentResponse {
        ciphertext,
        merkle_proof,
        roothash: *root,
        chunk_lens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §14.2 "top bits choose the octave": the octave band of the size is a
    /// function of the TOP 3 bits of the seed word (`v >> 61`), not of any
    /// middle bits. This pins the actual bit layout the doc comment describes
    /// and rules out the dead-bits regime where bits 32..=39 carried signal.
    #[test]
    fn octave_uses_top_three_bits() {
        // For each of the 8 octaves, the band floor must be MIN_SIZE << octave.
        for octave in 0..OCTAVES {
            // Place `octave` in the top 3 bits; zero everywhere else => offset 0
            // => size lands exactly on the band floor.
            let v = octave << 61;
            let expected_floor = (MIN_SIZE as u64) << octave;
            assert_eq!(
                size_of(v) as u64,
                expected_floor,
                "top 3 bits = {octave} must select band floor {expected_floor}"
            );
        }
    }

    /// The bits between the offset (low 32) and the octave (top 3) carry no
    /// signal: flipping any of bits 32..=60 while holding the top 3 bits and the
    /// low 32 bits fixed must NOT change the size. Under the old `v >> 40`
    /// selector, flipping bits 40..=42 changed the octave; this asserts it does
    /// not anymore.
    #[test]
    fn middle_bits_are_dead() {
        let base = 0u64; // octave 0, offset 0
        let size_base = size_of(base);
        for bit in 32..=60u32 {
            let v = base | (1u64 << bit);
            assert_eq!(
                size_of(v),
                size_base,
                "flipping bit {bit} must not change the size (no middle-bit signal)"
            );
        }
    }
}
