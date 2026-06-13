//! HKDF content-key derivation (paper §11.1/§11.4).
//!
//! The implementation now lives in [`digstore_core::crypto`] — the single source
//! of truth shared with the producer and the browser verifier. This module
//! re-exports it so host call-sites (`digstore-cli`, `digstore-store`) are
//! unchanged.

pub use digstore_core::crypto::derive_decryption_key;
