use crate::error::TamperError;
use aes_gcm_siv::aead::{Aead, KeyInit};
use aes_gcm_siv::{Aes256GcmSiv, Key, Nonce};

/// Fixed 12-byte nonce for the misuse-resistant AEAD.
///
/// The chunk AEAD is **AES-256-GCM-SIV** (RFC 8452), a nonce-misuse-resistant
/// scheme. Unlike plain GCM, reusing a (key, nonce) pair across two *distinct*
/// plaintexts does NOT leak a keystream XOR and does NOT permit recovery of the
/// authentication key (the "forbidden attack"). The synthetic IV is derived
/// internally with POLYVAL over key + plaintext, so each distinct plaintext gets
/// an independent IV even under a fixed nonce.
///
/// Holding the nonce fixed keeps encryption **deterministic**: the committed
/// merkle root is taken over the ciphertext bytes (see `digstore-store`), so the
/// same plaintext under the same per-URN key must always seal to the same
/// ciphertext. The only information a fixed nonce can leak is whether two chunks
/// have *identical* (plaintext, key) — which content-addressed dedup already
/// reveals — so no confidentiality or integrity guarantee is weakened.
const FIXED_NONCE: [u8; 12] = [0u8; 12];

/// Encrypt a chunk with AES-256-GCM-SIV under the per-URN `key`.
///
/// Returns `ciphertext || tag` (the underlying crate appends the 16-byte tag).
pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher
        .encrypt(nonce, plaintext)
        .expect("AES-256-GCM-SIV encryption is infallible for in-memory plaintext")
}

/// Decrypt and authenticate a chunk. A failed tag check is a tamper error.
pub fn decrypt_chunk(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, TamperError> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher.decrypt(nonce, ciphertext).map_err(|_| TamperError)
}
