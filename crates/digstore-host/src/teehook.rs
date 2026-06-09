//! §13.6 TEE / hardware-attestation alternative hook.
//!
//! `host_create_attestation` delegates to an `AttestationBackend`. The default
//! `BlsAttestationBackend` signs the challenge with the host BLS secret (Chia
//! AugScheme). A hardware-attestation backend can replace it behind the same
//! import surface without touching the linker wiring (§13.6).

use crate::error::HostError;
use digstore_core::types::{Bytes48, Bytes96};
use digstore_crypto::bls::BlsSecretKey;
use std::sync::Arc;

/// Pluggable attestation backend (§13.6). Default is BLS; a TEE backend can
/// drop in here without changing the import surface.
pub trait AttestationBackend: Send + Sync + 'static {
    /// Produce a 96-byte attestation signature over the challenge bytes.
    fn attest(&self, challenge: &[u8]) -> Result<Bytes96, HostError>;
    /// The attesting public key (48-byte BLS G1 for the BLS backend).
    fn public_key(&self) -> Bytes48;
}

/// Shared backend handle carried on `HostState`.
pub type SharedBackend = Arc<dyn AttestationBackend>;

/// Default backend: BLS sign over the challenge (Chia AugScheme).
///
/// NOTE: `digstore_crypto::bls::BlsSecretKey` is not `Clone`, so this backend
/// holds the key via an `Arc` shared with `HostKeys` (the same key material is
/// used for both attestation and node-proof signing).
pub struct BlsAttestationBackend {
    secret: Arc<BlsSecretKey>,
    public: Bytes48,
}

impl BlsAttestationBackend {
    /// Build a backend that owns the secret key directly (public API form).
    pub fn new(secret: BlsSecretKey, public: Bytes48) -> Self {
        BlsAttestationBackend {
            secret: Arc::new(secret),
            public,
        }
    }

    /// Build a backend that shares the secret key with `HostKeys` (used by the
    /// runtime so the un-`Clone`-able key serves both attestation and signing).
    pub fn from_shared(secret: Arc<BlsSecretKey>, public: Bytes48) -> Self {
        BlsAttestationBackend { secret, public }
    }
}

impl AttestationBackend for BlsAttestationBackend {
    fn attest(&self, challenge: &[u8]) -> Result<Bytes96, HostError> {
        Ok(digstore_crypto::bls::bls_sign(
            self.secret.as_ref(),
            challenge,
        ))
    }
    fn public_key(&self) -> Bytes48 {
        self.public
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::types::{Bytes48, Bytes96};

    struct ConstBackend;
    impl AttestationBackend for ConstBackend {
        fn attest(&self, _challenge: &[u8]) -> Result<Bytes96, crate::error::HostError> {
            Ok(Bytes96([0x5Au8; 96]))
        }
        fn public_key(&self) -> Bytes48 {
            Bytes48([0x11u8; 48])
        }
    }

    #[test]
    fn custom_backend_signs() {
        let b = ConstBackend;
        let sig = b.attest(b"challenge").unwrap();
        assert_eq!(sig.0, [0x5Au8; 96]);
        assert_eq!(b.public_key().0, [0x11u8; 48]);
    }
}
