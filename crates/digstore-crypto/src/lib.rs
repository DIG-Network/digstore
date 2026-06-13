//! Host-side cryptography for Digstore.
//!
//! # Documented deviations
//! - **Chunk AEAD (paper §11.2):** chunks are sealed with **AES-256-GCM-SIV**
//!   (RFC 8452), a nonce-misuse-resistant AEAD, under a fixed 12-byte nonce.
//!   Each canonical URN still derives a unique AES-256 key, but GCM-SIV no longer
//!   *depends* on key uniqueness for safety: reusing a (key, nonce) across two
//!   distinct plaintexts neither leaks a keystream XOR nor permits authentication
//!   -key recovery (the GCM "forbidden attack"). The fixed nonce keeps encryption
//!   deterministic so the ciphertext-committed merkle root is reproducible. See
//!   [`aead`].
//! - **BLS (paper §11.3, §12, §13.7, §21.6):** Chia AugScheme via `chia-bls`
//!   (blst). G1 public keys are 48 bytes, G2 signatures are 96 bytes. Messages
//!   are augmented with the public key and hashed with the Chia DST, exactly as
//!   the guest's pure-Rust `bls12_381` verifier expects.
//!
//! # Cross-impl parity contract
//! `tests/fixtures/bls_vectors.json` holds host-signed (blst) AugScheme vectors
//! that `digstore-guest`'s pure-Rust `bls12_381` verifier MUST accept. The
//! scheme tag in that file is [`CHIA_BLS_SCHEME`]; both crates compare against
//! the same constant. Regenerate with
//! `cargo run -p digstore-crypto --example gen_fixtures`.
//!
//! # Type boundary (CONVENTIONS C1)
//! BLS key material lives in the public [`bls`] module as `bls::SecretKey`,
//! `bls::PublicKey`, and `bls::Signature` (with aliases `BlsSecretKey` /
//! `BlsPublicKey`). All public byte material uses canonical `digstore-core`
//! types (`Bytes32`/`Bytes48`/`Bytes96`/`SecretSalt`).

pub mod aead;
pub mod bls;
pub mod error;
pub mod fixtures;
pub mod hash;
pub mod kdf;

pub use aead::{decrypt_chunk, encrypt_chunk};
pub use bls::{
    attestation_signing_message, bls_keygen, bls_sign, bls_verify, node_signing_message,
    push_signing_message, request_signing_message, sign_attestation, sign_node, sign_push,
    sign_request, sign_tombstone, tombstone_signing_message, validate_public_key, verify_push,
    verify_request, verify_tombstone,
};
pub use error::{BlsError, CryptoError, TamperError};
pub use fixtures::{
    write_bls_fixtures, write_kdf_fixtures, BlsFixture, BlsFixtureSet, KdfFixture, KdfFixtureSet,
};
pub use hash::sha256;
pub use kdf::derive_decryption_key;

use digstore_core::Bytes48;

/// Decrypt a chunk AND validate the accompanying store/host public key in one
/// call, returning the unified [`CryptoError`]. The public key is validated
/// (canonical G1) before the plaintext is returned, so a caller that holds both
/// a ciphertext and an unverified key gets a single error type spanning AEAD
/// (`TamperError`) and BLS (`BlsError`) failures.
pub fn decrypt_and_unwrap(
    key: &[u8; 32],
    ciphertext: &[u8],
    public_key: &Bytes48,
) -> Result<Vec<u8>, CryptoError> {
    validate_public_key(public_key)?;
    let plaintext = decrypt_chunk(key, ciphertext)?;
    Ok(plaintext)
}

/// Versioning tag for the crypto domain constants (HKDF salt/info, scheme tag).
/// Bumping this signals a deliberate, breaking change to derived material.
pub const CRYPTO_VERSION: u32 = 1;

/// Chia AugScheme tag shared with `digstore-guest` for cross-impl BLS parity
/// (CONVENTIONS C8). Canonical value per the conventions / plan Task 0.
///
/// NOTE (deviation): the conventions name `digstore_core::CHIA_BLS_SCHEME` as the
/// single source of truth, but `digstore-core` does not currently export that
/// constant and this crate must not modify other crates. The constant is defined
/// here with the canonical value; `digstore-guest` compares its fixtures against
/// this same literal.
pub const CHIA_BLS_SCHEME: &str = "chia-aug-scheme-bls12381-g2-xmd-sha256-sswu-ro";
