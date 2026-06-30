//! Pure symmetric read-crypto for the Digstore content contract (paper §11):
//! the per-URN AES-256-GCM-SIV chunk seal + the HKDF content-key derivation.
//!
//! This lives in `digstore-core` so EVERY layer derives keys and seals/opens
//! chunks from ONE implementation: the producer (`digstore-store`), the host
//! serve path, and the browser verifier (`dig-client-wasm`). `digstore-crypto`
//! re-exports these (and layers BLS + a typed `TamperError` on top); the wasm
//! read crate depends only on this crate. Previously the wasm crate reproduced
//! this byte-for-byte (policed by a parity test) because `digstore-crypto` pulls
//! `chia-bls → blst` (not wasm-buildable) — a duplication that allowed crypto to
//! skew between layers. Centralizing it here removes that class of bug.
//!
//! `no_std` + `alloc`, wasm-clean: AES-256-GCM-SIV under a FIXED nonce (no RNG —
//! see below) and HKDF-SHA256, neither of which drags in `getrandom` or `blst`.

use crate::config::SecretSalt;
use aes_gcm_siv::aead::{Aead, KeyInit};
use aes_gcm_siv::{Aes256GcmSiv, Key, Nonce};
use alloc::vec::Vec;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

/// The Chia BLS AugScheme ciphersuite tag — the SINGLE SOURCE OF TRUTH for the
/// scheme string shared across every BLS layer (CONVENTIONS C8): `digstore-crypto`
/// (native BLS sign/verify) and `digstore-guest` (the wasm verifier) re-export /
/// compare against THIS constant so the ciphersuite can never skew between layers.
/// (#131: previously `digstore-crypto` defined it locally with a NOTE that core
/// should own it; core now exports it and crypto re-exports, closing the
/// one-source-of-truth gap.)
pub const CHIA_BLS_SCHEME: &str = "chia-aug-scheme-bls12381-g2-xmd-sha256-sswu-ro";

/// Fixed HKDF salt domain string for stores (paper §11.1, §11.4).
const HKDF_SALT_DOMAIN: &[u8] = b"digstore-hkdf-salt-v1";
/// Fixed HKDF `info` context for the AES-256-GCM content key (paper §11.1).
const HKDF_INFO: &[u8] = b"digstore-aes-256-gcm-key-v1";

/// Fixed 12-byte nonce for the misuse-resistant AEAD (RFC 8452, AES-256-GCM-SIV).
///
/// GCM-SIV derives its synthetic IV internally with POLYVAL over key + plaintext,
/// so each distinct plaintext gets an independent IV even under a fixed nonce —
/// reusing a (key, nonce) pair leaks neither a keystream XOR nor the auth key
/// (unlike plain GCM's "forbidden attack"). Holding the nonce fixed keeps
/// encryption deterministic so the ciphertext-committed merkle root is
/// reproducible (the committed root is taken over the ciphertext bytes).
const FIXED_NONCE: [u8; 12] = [0u8; 12];

/// Derive the 32-byte AES-256 content key for a resource from its canonical URN.
///
/// `ikm = canonical_urn` bytes. Public stores use `salt = SHA-256(HKDF_SALT_DOMAIN)`.
/// Private stores (paper §11.4) mix the 32-byte secret salt in:
/// `salt = SHA-256(HKDF_SALT_DOMAIN || secret_salt)`. Distinct URNs (and distinct
/// salts) derive distinct keys — the invariant that makes the fixed nonce safe.
/// The salt is borrowed (`Option<&SecretSalt>`, CONVENTIONS C10).
pub fn derive_decryption_key(canonical_urn: &str, secret_salt: Option<&SecretSalt>) -> [u8; 32] {
    let mut salt_hasher = Sha256::new();
    salt_hasher.update(HKDF_SALT_DOMAIN);
    if let Some(SecretSalt(secret)) = secret_salt {
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

/// Encrypt a chunk with AES-256-GCM-SIV under the per-URN `key`, returning
/// `ciphertext || tag`. Infallible for in-memory plaintext.
pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher
        .encrypt(nonce, plaintext)
        .expect("AES-256-GCM-SIV encryption is infallible for in-memory plaintext")
}

/// Decrypt + authenticate a chunk. `Err(())` is a tamper / wrong-key failure;
/// the unit error is deliberate — every caller layers its own typed error on top
/// (`digstore-crypto` → `TamperError`, `dig-client-wasm` → `JsError`), so a
/// dedicated error type here would only force a redundant conversion.
#[allow(clippy::result_unit_err)]
pub fn decrypt_chunk(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
    let cipher = Aes256GcmSiv::new(Key::<Aes256GcmSiv>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher.decrypt(nonce, ciphertext).map_err(|_| ())
}
