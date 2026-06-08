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
pub mod hash;

pub use error::{BlsError, CryptoError, TamperError};
pub use hash::sha256;
