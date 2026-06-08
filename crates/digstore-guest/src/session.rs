//! Sessions (§12.4). A session is established after a successful attestation and
//! gates `jwks_fetch`: until `host_verify_session()` is true, the guest must not
//! reach out to fetch JWKS (NoSession). Mirrors the host-side gate so the guest
//! fails closed even if a buggy host forgets to enforce it.

use crate::host::DigHost;
use alloc::vec::Vec;
use digstore_core::ErrorCode;

/// Ensure a session exists; establish one if absent. Returns the session token bytes.
pub fn ensure_session<H: DigHost + ?Sized>(
    host: &H,
    challenge: &[u8],
) -> Result<Vec<u8>, ErrorCode> {
    if host.verify_session() {
        return Ok(Vec::new());
    }
    host.establish_session(challenge)
}

/// jwks_fetch, gated on an active session. NoSession (-100) until established.
pub fn gated_jwks_fetch<H: DigHost + ?Sized>(host: &H, url: &[u8]) -> Result<Vec<u8>, ErrorCode> {
    if !host.verify_session() {
        return Err(ErrorCode::NoSession);
    }
    host.jwks_fetch(url)
}
