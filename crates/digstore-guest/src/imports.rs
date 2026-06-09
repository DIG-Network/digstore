//! Raw `dig_host` imports + safe wrappers + return-buffer reader. Wasm-only.

use alloc::vec;
use alloc::vec::Vec;
use digstore_core::ErrorCode;

#[link(wasm_import_module = "dig_host")]
extern "C" {
    pub fn host_get_public_key() -> i32;
    pub fn host_create_attestation(challenge_ptr: i32) -> i32;
    pub fn host_establish_session(challenge_ptr: i32) -> i32;
    pub fn host_verify_session() -> i32;
    pub fn jwks_fetch(url_ptr: i32, url_len: i32) -> i32;
    pub fn host_get_current_time() -> i64;
    pub fn host_random_bytes(count: i32) -> i32;
    pub fn host_read_return_buffer(dest_ptr: i32) -> i32;
}

/// §5.1 Import section retention. The served module MUST declare all eight
/// `dig_host` host functions (§6.3). LLVM only emits a wasm import that is
/// actually *called* on a reachable path, so any host function the guest does
/// not currently call would be silently dropped from the Import section. This
/// `#[no_mangle]` anchor references every raw import behind an
/// `init`-only-reachable, never-taken branch so the linker keeps all eight
/// declarations without altering runtime behavior (it returns before any call).
///
/// It is wired into `init` (see `abi.rs`) so it is reachable from an export and
/// cannot itself be stripped.
#[cfg(target_arch = "wasm32")]
#[inline(never)]
pub fn retain_dig_host_imports() -> i32 {
    // `core::hint::black_box(false)` is opaque to the optimizer, so the calls
    // below stay in the reachable call graph (forcing the imports to be
    // declared) even though the branch is never taken at runtime.
    if core::hint::black_box(false) {
        let mut acc: i64 = 0;
        unsafe {
            acc ^= host_get_public_key() as i64;
            acc ^= host_create_attestation(0) as i64;
            acc ^= host_establish_session(0) as i64;
            acc ^= host_verify_session() as i64;
            acc ^= jwks_fetch(0, 0) as i64;
            acc ^= host_get_current_time();
            acc ^= host_random_bytes(0) as i64;
            acc ^= host_read_return_buffer(0) as i64;
        }
        return acc as i32;
    }
    0
}

/// Convert a host return code (>=0 length / <0 error) plus a return-buffer copy
/// into a Rust result.
pub fn read_result(code: i32) -> Result<Vec<u8>, ErrorCode> {
    if code < 0 {
        return Err(map_error(code));
    }
    let len = code as usize;
    let mut buf = vec![0u8; len];
    unsafe {
        let written = host_read_return_buffer(buf.as_mut_ptr() as i32);
        if written < 0 {
            return Err(map_error(written));
        }
        buf.truncate(written as usize);
    }
    Ok(buf)
}

pub fn map_error(code: i32) -> ErrorCode {
    match code {
        -100 => ErrorCode::NoSession,
        -101 => ErrorCode::SessionExpired,
        -102 => ErrorCode::AttestationFailed,
        -200 => ErrorCode::NetworkError,
        -203 => ErrorCode::Timeout,
        -300 => ErrorCode::NotFound,
        -301 => ErrorCode::ValidationFailed,
        -2 => ErrorCode::InvalidParameter,
        -3 => ErrorCode::BufferTooSmall,
        _ => ErrorCode::GeneralError,
    }
}
