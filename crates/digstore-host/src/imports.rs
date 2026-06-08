//! Registration of the eight `dig_host` import functions (§6.3, §12, §18.3).

use crate::error::HostError;
use crate::runtime::RuntimeState;
use digstore_core::abi::ErrorCode;
use wasmtime::{Caller, Linker};

pub fn register(linker: &mut Linker<RuntimeState>) -> Result<(), HostError> {
    let m = "dig_host";

    // host_get_current_time() -> i64 (§12). Injectable Clock.
    linker
        .func_wrap(m, "host_get_current_time", |caller: Caller<'_, RuntimeState>| -> i64 {
            caller.data().host.clock.now_unix_secs() as i64
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_random_bytes(count) -> i32 length written, or InvalidParameter (§12).
    linker
        .func_wrap(
            m,
            "host_random_bytes",
            |mut caller: Caller<'_, RuntimeState>, count: i32| -> i32 {
                if count < 0 {
                    return ErrorCode::InvalidParameter as i32;
                }
                let max = caller.data().host.config.max_random_bytes as usize;
                let state = &mut caller.data_mut().host;
                match state.rng.fill(count as usize, max) {
                    Some(bytes) => match state.return_buffer.set(&bytes) {
                        Ok(n) => n as i32,
                        Err(_) => ErrorCode::GeneralError as i32,
                    },
                    None => ErrorCode::InvalidParameter as i32,
                }
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_get_public_key() -> i32 length (48 bytes BLS G1) written (§12).
    linker
        .func_wrap(m, "host_get_public_key", |mut caller: Caller<'_, RuntimeState>| -> i32 {
            let pk = caller.data().host.keys.bls_public.0; // [u8; 48]
            match caller.data_mut().host.return_buffer.set(&pk) {
                Ok(n) => n as i32,
                Err(_) => ErrorCode::GeneralError as i32,
            }
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    const CHALLENGE_LEN: usize = 32 + 32 + 8;
    const SESSION_TTL_SECS: u64 = 300;

    // host_create_attestation(challenge_ptr) -> i32 length of AttestationResponse (§12, §13.6).
    linker
        .func_wrap(
            m,
            "host_create_attestation",
            |mut caller: Caller<'_, RuntimeState>, challenge_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let data = mem.data(&caller);
                let start = challenge_ptr as usize;
                let end = match start.checked_add(CHALLENGE_LEN) {
                    Some(e) if e <= data.len() => e,
                    _ => return ErrorCode::InvalidParameter as i32,
                };
                let challenge = data[start..end].to_vec();
                let state = &mut caller.data_mut().host;
                let sig = match state.attestation.attest(&challenge) {
                    Ok(s) => s,
                    Err(_) => return ErrorCode::AttestationFailed as i32,
                };
                let pk = state.attestation.public_key();
                let mut resp = Vec::with_capacity(48 + 32 + 96);
                resp.extend_from_slice(&pk.0);
                resp.extend_from_slice(&state.instance_id.0);
                resp.extend_from_slice(&sig.0);
                state.last_signature = Some(sig);
                match state.return_buffer.set(&resp) {
                    Ok(n) => n as i32,
                    Err(_) => ErrorCode::GeneralError as i32,
                }
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_establish_session(challenge_ptr) -> i32 (>=0 ok) (§12).
    linker
        .func_wrap(
            m,
            "host_establish_session",
            |mut caller: Caller<'_, RuntimeState>, challenge_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let data = mem.data(&caller);
                let start = challenge_ptr as usize;
                let end = match start.checked_add(CHALLENGE_LEN) {
                    Some(e) if e <= data.len() => e,
                    _ => return ErrorCode::InvalidParameter as i32,
                };
                let challenge = &data[start..end];
                let mut nonce = [0u8; 32];
                let mut store_id = [0u8; 32];
                nonce.copy_from_slice(&challenge[..32]);
                store_id.copy_from_slice(&challenge[32..64]);
                let now = caller.data().host.clock.now_unix_secs();
                caller
                    .data_mut()
                    .host
                    .sessions
                    .establish(nonce, store_id, now, SESSION_TTL_SECS);
                0
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_verify_session() -> i32 (1 valid / 0 invalid) (§12).
    linker
        .func_wrap(m, "host_verify_session", |caller: Caller<'_, RuntimeState>| -> i32 {
            let now = caller.data().host.clock.now_unix_secs();
            if caller.data().host.sessions.is_valid(now) {
                1
            } else {
                0
            }
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // jwks_fetch(url_ptr, url_len) -> i32. SESSION-GATED (§6.3).
    // NOTE (§18.2): epoch interruption does not cover blocking host I/O; this
    // call is bounded by its own reqwest timeout, derived from ExecutionLimits.
    linker
        .func_wrap(
            m,
            "jwks_fetch",
            |mut caller: Caller<'_, RuntimeState>, url_ptr: i32, url_len: i32| -> i32 {
                let now = caller.data().host.clock.now_unix_secs();
                if !caller.data().host.sessions.is_valid(now) {
                    return ErrorCode::NoSession as i32;
                }
                let timeout_secs = caller.data().host.http_timeout_secs;
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let data = mem.data(&caller);
                let start = url_ptr as usize;
                let end = match start.checked_add(url_len as usize) {
                    Some(e) if e <= data.len() => e,
                    _ => return ErrorCode::InvalidParameter as i32,
                };
                let url = match std::str::from_utf8(&data[start..end]) {
                    Ok(u) => u.to_string(),
                    Err(_) => return ErrorCode::InvalidParameter as i32,
                };
                let resp = match reqwest::blocking::Client::new()
                    .get(&url)
                    .timeout(std::time::Duration::from_secs(timeout_secs))
                    .send()
                {
                    Ok(r) => r,
                    Err(e) if e.is_timeout() => return ErrorCode::Timeout as i32,
                    Err(_) => return ErrorCode::NetworkError as i32,
                };
                let body = match resp.bytes() {
                    Ok(b) => b,
                    Err(_) => return ErrorCode::NetworkError as i32,
                };
                match caller.data_mut().host.return_buffer.set(&body) {
                    Ok(n) => n as i32,
                    Err(_) => ErrorCode::GeneralError as i32,
                }
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_read_return_buffer(dest_ptr) -> i32 bytes copied (§6.4).
    linker
        .func_wrap(
            m,
            "host_read_return_buffer",
            |mut caller: Caller<'_, RuntimeState>, dest_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let buf = caller.data().host.return_buffer.as_slice().to_vec();
                let data = mem.data_mut(&mut caller);
                let start = dest_ptr as usize;
                let end = match start.checked_add(buf.len()) {
                    Some(e) => e,
                    None => return ErrorCode::InvalidParameter as i32,
                };
                if end > data.len() {
                    return ErrorCode::BufferTooSmall as i32;
                }
                data[start..end].copy_from_slice(&buf);
                buf.len() as i32
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    Ok(())
}
