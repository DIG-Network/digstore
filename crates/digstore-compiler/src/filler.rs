use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use sha2::{Digest, Sha256};

use digstore_core::Bytes32;

/// Domain-separation tag for the filler seed (documented deviation #2, §19.3).
const FILLER_DOMAIN: &[u8] = b"digstore-filler-v1";

/// Produce `len` bytes of deterministic pseudo-random filler for the interleaved
/// pool gaps. The keystream is positional, so a shorter request is a prefix of a
/// longer one for the same seed.
///
/// seed = SHA-256(store_id || roothash || b"digstore-filler-v1")
/// key  = seed (32 bytes), nonce = 12 zero bytes.
pub fn deterministic_filler(store_id: &Bytes32, roothash: &Bytes32, len: usize) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(store_id.0);
    hasher.update(roothash.0);
    hasher.update(FILLER_DOMAIN);
    let seed: [u8; 32] = hasher.finalize().into();

    let nonce = [0u8; 12];
    let mut cipher = ChaCha20::new(&seed.into(), &nonce.into());
    let mut buf = vec![0u8; len];
    cipher.apply_keystream(&mut buf); // XOR with zero buffer = raw keystream
    buf
}
