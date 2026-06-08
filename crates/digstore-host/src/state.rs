//! Per-instance host state shared with the `dig_host` imports (§6.4, §12, §18.3).

use crate::clock::Clock;
use crate::error::HostError;
use crate::random::HostRng;
use crate::session::SessionTable;
use crate::teehook::SharedBackend;
use digstore_core::config::HostImportsConfig;
use digstore_core::types::{Bytes32, Bytes48, Bytes96};
use digstore_crypto::bls::BlsSecretKey;
use digstore_prover::{ChainSource, Prover};
use std::sync::Arc;

/// Growable shared return buffer (§6.4): the single channel imports use to
/// hand variable-length results back to the guest.
pub struct ReturnBuffer {
    bytes: Vec<u8>,
    max: usize,
}

impl ReturnBuffer {
    pub fn new(cfg: &HostImportsConfig) -> Self {
        ReturnBuffer {
            bytes: Vec::with_capacity(cfg.return_buffer_capacity),
            max: cfg.max_return_buffer_size,
        }
    }

    /// Replace buffer contents; returns the number of bytes written, or
    /// `ReturnBufferOverflow` if it exceeds `max_return_buffer_size`.
    pub fn set(&mut self, data: &[u8]) -> Result<usize, HostError> {
        if data.len() > self.max {
            return Err(HostError::ReturnBufferOverflow {
                needed: data.len(),
                max: self.max,
            });
        }
        self.bytes.clear();
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

/// Host BLS key material used for attestation and node-proof signing (§12).
///
/// DEVIATION: `digstore_crypto::bls::BlsSecretKey` is not `Clone`, and the
/// default `BlsAttestationBackend` must sign with the same key. The secret is
/// therefore held behind an `Arc` so it can be shared between this keystore and
/// the default attestation backend without duplicating the (un-clonable) key.
pub struct HostKeys {
    pub bls_secret: Arc<BlsSecretKey>,
    pub bls_public: Bytes48,
}

/// State threaded through every `dig_host` import call (§18.3).
pub struct HostState {
    pub store_id: Bytes32,
    pub config: HostImportsConfig,
    pub return_buffer: ReturnBuffer,
    pub keys: Arc<HostKeys>,
    pub attestation: SharedBackend,
    pub clock: Arc<dyn Clock>,
    pub sessions: SessionTable,
    pub chain: Arc<dyn ChainSource>,
    pub prover: Arc<dyn Prover>,
    pub rng: HostRng,
    pub instance_id: Bytes32,
    /// Reqwest request budget for blocking host I/O (jwks_fetch), seeded from
    /// ExecutionLimits.timeout. Epoch interruption does NOT cover blocking host
    /// calls, so jwks is bounded by this independent request timeout (§18.2 note).
    pub http_timeout_secs: u64,
    /// Set by attestation so the serve flow can record the last signature.
    pub last_signature: Option<Bytes96>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::config::HostImportsConfig;

    fn cfg() -> HostImportsConfig {
        HostImportsConfig {
            return_buffer_capacity: 64 * 1024,
            max_return_buffer_size: 16 * 1024 * 1024,
            max_random_bytes: 1024,
            host_version: "dig-host-test/0.1".to_string(),
        }
    }

    #[test]
    fn set_return_then_read_round_trips() {
        let mut rb = ReturnBuffer::new(&cfg());
        let written = rb.set(&[1, 2, 3, 4]).unwrap();
        assert_eq!(written, 4);
        assert_eq!(rb.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn buffer_grows_past_initial_capacity() {
        let mut rb = ReturnBuffer::new(&cfg());
        let big = vec![7u8; 128 * 1024];
        let written = rb.set(&big).unwrap();
        assert_eq!(written, 128 * 1024);
        assert_eq!(rb.as_slice().len(), 128 * 1024);
    }

    #[test]
    fn buffer_rejects_over_max() {
        let mut rb = ReturnBuffer::new(&cfg());
        let too_big = vec![0u8; 16 * 1024 * 1024 + 1];
        let err = rb.set(&too_big).unwrap_err();
        assert!(matches!(err, crate::error::HostError::ReturnBufferOverflow { .. }));
    }
}
