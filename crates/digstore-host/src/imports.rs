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

    // --- temporary stubs, replaced in 11d–11e ---
    for name in ["host_verify_session"] {
        linker
            .func_wrap(m, name, |_c: Caller<'_, RuntimeState>| -> i32 { 0 })
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;
    }
    for name in ["host_create_attestation", "host_establish_session"] {
        linker
            .func_wrap(m, name, |_c: Caller<'_, RuntimeState>, _p: i32| -> i32 {
                ErrorCode::GeneralError as i32
            })
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;
    }
    linker
        .func_wrap(m, "jwks_fetch", |_c: Caller<'_, RuntimeState>, _p: i32, _l: i32| -> i32 {
            ErrorCode::NoSession as i32
        })
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
