//! Host abstraction. All guest logic depends on `&dyn DigHost`, never on the
//! raw `dig_host` imports directly, so logic is unit-testable natively.

use alloc::vec::Vec;
use digstore_core::ErrorCode;

/// Result of a host import: either bytes written to the return buffer, or an error code.
pub type HostResult = Result<Vec<u8>, ErrorCode>;

pub trait DigHost {
    /// host_get_public_key -> 48-byte BLS G1 of the serving host instance.
    fn get_public_key(&self) -> HostResult;
    /// host_create_attestation(challenge) -> serialized AttestationResponse bytes.
    fn create_attestation(&self, challenge: &[u8]) -> HostResult;
    /// host_establish_session(challenge) -> opaque session token bytes.
    fn establish_session(&self, challenge: &[u8]) -> HostResult;
    /// host_verify_session -> true if a valid, unexpired session exists.
    fn verify_session(&self) -> bool;
    /// jwks_fetch(url) -> JWKS JSON bytes. SESSION-GATED at the host boundary.
    fn jwks_fetch(&self, url: &[u8]) -> HostResult;
    /// host_get_current_time -> unix seconds.
    fn current_time(&self) -> u64;
    /// host_random_bytes(count) -> `count` fresh random bytes (re-randomized per call).
    fn random_bytes(&self, count: u32) -> HostResult;
}

#[cfg(target_arch = "wasm32")]
pub struct WasmHost;

#[cfg(target_arch = "wasm32")]
impl DigHost for WasmHost {
    fn get_public_key(&self) -> HostResult {
        crate::imports::read_result(unsafe { crate::imports::host_get_public_key() })
    }
    fn create_attestation(&self, challenge: &[u8]) -> HostResult {
        crate::imports::read_result(unsafe {
            crate::imports::host_create_attestation(challenge.as_ptr() as i32)
        })
    }
    fn establish_session(&self, challenge: &[u8]) -> HostResult {
        crate::imports::read_result(unsafe {
            crate::imports::host_establish_session(challenge.as_ptr() as i32)
        })
    }
    fn verify_session(&self) -> bool {
        unsafe { crate::imports::host_verify_session() == 1 }
    }
    fn jwks_fetch(&self, url: &[u8]) -> HostResult {
        crate::imports::read_result(unsafe {
            crate::imports::jwks_fetch(url.as_ptr() as i32, url.len() as i32)
        })
    }
    fn current_time(&self) -> u64 {
        unsafe { crate::imports::host_get_current_time() as u64 }
    }
    fn random_bytes(&self, count: u32) -> HostResult {
        crate::imports::read_result(unsafe { crate::imports::host_random_bytes(count as i32) })
    }
}
