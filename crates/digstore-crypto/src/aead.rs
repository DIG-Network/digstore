use crate::error::TamperError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};

/// Fixed 12-byte GCM nonce (paper §11.2). Safe ONLY under the unique-key-per-URN
/// invariant: each canonical URN derives a distinct key, so no key is ever
/// reused across two plaintexts. See crate-level docs.
const FIXED_NONCE: [u8; 12] = [0u8; 12];

/// Encrypt a chunk with AES-256-GCM under the per-URN `key`.
///
/// Returns `ciphertext || tag` (the `aes-gcm` crate appends the 16-byte tag).
pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher
        .encrypt(nonce, plaintext)
        .expect("AES-256-GCM encryption is infallible for in-memory plaintext")
}

/// Decrypt and authenticate a chunk. A failed GCM tag check is a tamper error.
pub fn decrypt_chunk(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, TamperError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher.decrypt(nonce, ciphertext).map_err(|_| TamperError)
}
