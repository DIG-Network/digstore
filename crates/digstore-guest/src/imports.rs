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
