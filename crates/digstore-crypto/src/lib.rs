//! Host-side cryptography for Digstore.
//!
//! # Documented deviations
//! - **Fixed GCM nonce (paper §11.2):** AES-256-GCM uses a fixed all-zero
//!   12-byte nonce. Safe *only* because every canonical URN derives a unique
//!   AES-256 key (unique-key-per-URN invariant). A key is never reused across
//!   two plaintexts.
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

pub mod error;
pub mod fixtures;
pub mod hash;
pub mod kdf;

pub use error::{BlsError, CryptoError, TamperError};
pub use hash::sha256;
pub use fixtures::{write_kdf_fixtures, KdfFixture, KdfFixtureSet};
pub use kdf::derive_decryption_key;

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
