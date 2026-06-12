//! Read-crypto primitives, BYTE-IDENTICAL to the canonical host-side crypto in
//! `digstore-crypto` (`src/kdf.rs`, `src/aead.rs`). This crate cannot depend on
//! `digstore-crypto` directly (its `chia-bls`/`blst` dep does not compile to
//! wasm32, and BLS is not on the read path), so the small KDF + AEAD are
//! reproduced here with the EXACT same domain constants and crate versions.
//! Parity is asserted by the native `tests/parity.rs` oracle against the real
//! `digstore-crypto`.

use aes_gcm_siv::aead::{Aead, KeyInit};
use aes_gcm_siv::{Aes256GcmSiv, Key, Nonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

// --- KDF (mirror of digstore-crypto::kdf, paper §11.1/§11.4) -----------------

/// Fixed HKDF salt domain string for stores (paper §11.1, §11.4).
/// MUST equal `digstore-crypto`'s `HKDF_SALT_DOMAIN`.
const HKDF_SALT_DOMAIN: &[u8] = b"digstore-hkdf-salt-v1";
/// Fixed HKDF `info` context for the AES-256-GCM content key (paper §11.1).
/// MUST equal `digstore-crypto`'s `HKDF_INFO`.
const HKDF_INFO: &[u8] = b"digstore-aes-256-gcm-key-v1";

/// Derive the 32-byte AES-256 content key for a resource from its canonical URN.
///
/// `ikm = canonical_urn` bytes. Public stores use `salt = SHA-256(HKDF_SALT_DOMAIN)`.
/// Private stores (paper §11.4) mix the 32-byte secret salt in:
/// `salt = SHA-256(HKDF_SALT_DOMAIN || secret_salt)`. Distinct URNs (and distinct
/// salts) derive distinct keys — the invariant that makes the fixed GCM-SIV nonce
/// safe (§11.2). Byte-identical to `digstore_crypto::derive_decryption_key`.
pub fn derive_decryption_key(canonical_urn: &str, secret_salt: Option<&[u8; 32]>) -> [u8; 32] {
    let mut salt_hasher = Sha256::new();
    salt_hasher.update(HKDF_SALT_DOMAIN);
    if let Some(secret) = secret_salt {
        salt_hasher.update(secret);
    }
    let salt_digest = salt_hasher.finalize();
    let mut salt = [0u8; 32];
    salt.copy_from_slice(&salt_digest);

    let hk = Hkdf::<Sha256>::new(Some(&salt), canonical_urn.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(HKDF_INFO, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}

// --- AEAD (mirror of digstore-crypto::aead, paper §11.2) ---------------------

/// Fixed 12-byte nonce for the misuse-resistant AEAD (RFC 8452, AES-256-GCM-SIV).
/// MUST equal `digstore-crypto`'s `FIXED_NONCE`. The synthetic IV is derived
/// internally over key+plaintext, so a fixed nonce is safe and keeps the
/// ciphertext-committed merkle root reproducible.
const FIXED_NONCE: [u8; 12] = [0u8; 12];

/// Decrypt and authenticate a chunk (`ciphertext || tag`). A failed tag check is
/// a tamper/wrong-key error. Byte-identical to `digstore_crypto::decrypt_chunk`.
pub fn decrypt_chunk(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher.decrypt(nonce, ciphertext).map_err(|_| ())
}

/// Encrypt a chunk under `key` (test-parity helper; mirrors
/// `digstore_crypto::encrypt_chunk`). Used by the parity test and by callers that
/// want to round-trip in the browser; the read path only ever decrypts.
#[allow(dead_code)]
pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher
        .encrypt(nonce, plaintext)
        .expect("AES-256-GCM-SIV encryption is infallible for in-memory plaintext")
}

/// SHA-256 over `data` as a raw 32-byte array (matches `digstore_core::sha256`'s
/// digest, returned unwrapped so callers compare against `MerkleProof` leaves).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}
