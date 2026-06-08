use digstore_core::ErrorCode;
use digstore_guest::host::{DigHost, HostResult};
use std::cell::Cell;

/// Deterministic, scriptable host double. `random_bytes` is a counter-seeded
/// ramp so tests are reproducible AND change across calls.
pub struct MockHost {
    pub pubkey: Vec<u8>,
    pub attestation: HostResult,
    pub session_ok: bool,
    pub jwks: HostResult,
    pub time: u64,
    pub rand_calls: Cell<u32>,
}

impl Default for MockHost {
    fn default() -> Self {
        MockHost {
            pubkey: vec![0xABu8; 48],
            attestation: Ok(vec![0u8; 176]),
            session_ok: true,
            jwks: Ok(b"{}".to_vec()),
            time: 1_700_000_000,
            rand_calls: Cell::new(0),
        }
    }
}

impl DigHost for MockHost {
    fn get_public_key(&self) -> HostResult {
        Ok(self.pubkey.clone())
    }
    fn create_attestation(&self, _c: &[u8]) -> HostResult {
        self.attestation.clone()
    }
    fn establish_session(&self, _c: &[u8]) -> HostResult {
        Ok(vec![1u8; 16])
    }
    fn verify_session(&self) -> bool {
        self.session_ok
    }
    fn jwks_fetch(&self, _u: &[u8]) -> HostResult {
        self.jwks.clone()
    }
    fn current_time(&self) -> u64 {
        self.time
    }
    fn random_bytes(&self, count: u32) -> HostResult {
        let n = self.rand_calls.get();
        self.rand_calls.set(n + 1);
        // distinct per call: byte i = (n*31 + i) wrapping
        Ok((0..count)
            .map(|i| (n.wrapping_mul(31).wrapping_add(i)) as u8)
            .collect())
    }
}

#[test]
fn mock_random_differs_across_calls() {
    let h = MockHost::default();
    let a = h.random_bytes(8).unwrap();
    let b = h.random_bytes(8).unwrap();
    assert_ne!(a, b, "successive random_bytes must differ");
    assert_eq!(a.len(), 8);
}

#[test]
fn mock_attestation_can_be_scripted_as_error() {
    let mut h = MockHost::default();
    h.attestation = Err(ErrorCode::AttestationFailed);
    assert!(h.create_attestation(b"x").is_err());
}
