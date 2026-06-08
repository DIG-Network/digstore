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

    // --- temporary stubs, replaced in 11b–11e ---
    for name in ["host_get_public_key", "host_verify_session"] {
        linker
            .func_wrap(m, name, |_c: Caller<'_, RuntimeState>| -> i32 { 0 })
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;
    }
    for name in ["host_create_attestation", "host_establish_session", "host_read_return_buffer"] {
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

    Ok(())
}
