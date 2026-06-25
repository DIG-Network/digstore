//! dig-runtime — the DIG browser's NATIVE in-process runtime.
//!
//! The browser process **is** the DIG node. `dig_runtime.dll` (a cargo `cdylib`
//! shipped next to the browser executable like `chrome.dll`) exposes a direct
//! C-ABI entrypoint, [`dig_rpc`] (`request_json -> response_json`), that the
//! browser's dig:// handler calls IN-PROCESS — there is no loopback server, no
//! socket, and no `dig-node.exe` sidecar. It runs the exact same
//! `dig_node::handle_rpc` dispatch the standalone node uses, on a shared
//! multi-thread tokio runtime owned by this library.
//!
//! Heavy Rust deps (wasmtime, tokio, reqwest, blst via the node) build freely as
//! a cargo cdylib — the route Chromium's restricted Rust/GN build can't take —
//! and run in the browser (broker) process, which is unsandboxed (full token)
//! and not ACG/JIT-locked the way renderer processes are.

use std::ffi::{c_char, CStr, CString};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::OnceLock;

use dig_node::Node;

/// The process-wide DIG runtime: the tokio runtime + the node it drives. Built
/// once, lazily, on first use (or eagerly by `dig_runtime_start`).
struct DigRuntime {
    rt: tokio::runtime::Runtime,
    node: Arc<Node>,
}

static RUNTIME: OnceLock<DigRuntime> = OnceLock::new();

fn runtime() -> &'static DigRuntime {
    RUNTIME.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("dig-runtime: tokio runtime");
        let node = Node::from_env();
        DigRuntime { rt, node }
    })
}

/// Initialize the native DIG runtime (build the node + tokio runtime, load the
/// §21 identity, prepare the cache). Idempotent and cheap to call again. Optional
/// — [`dig_rpc`] initializes lazily — but the browser calls this once at startup
/// so the identity + cache are ready before the first dig:// navigation.
///
/// # Safety
/// C-ABI export for `GetProcAddress`. Takes no arguments and never unwinds across
/// the FFI boundary (any panic during init is caught).
#[no_mangle]
pub extern "C" fn dig_runtime_start() {
    let _ = std::panic::catch_unwind(|| {
        runtime();
    });
}

/// Execute one DIG JSON-RPC request in-process and return the JSON-RPC response.
///
/// `request_json` is a NUL-terminated UTF-8 JSON string owned by the caller. The
/// return value is a NUL-terminated UTF-8 JSON string owned by this library; the
/// caller MUST return it to [`dig_free`]. Returns null only on a null/invalid
/// input pointer or an allocation failure.
///
/// Blocking: drives the request to completion on the shared runtime, so callers
/// must invoke it from a thread allowed to block (e.g. a `base::MayBlock` task),
/// never the browser UI/IO thread. Concurrent calls are safe.
///
/// # Safety
/// `request_json` must be a valid NUL-terminated C string for the duration of the
/// call. The returned pointer must be freed exactly once with [`dig_free`].
#[no_mangle]
pub unsafe extern "C" fn dig_rpc(request_json: *const c_char) -> *mut c_char {
    if request_json.is_null() {
        return std::ptr::null_mut();
    }
    let req = unsafe { CStr::from_ptr(request_json) }
        .to_string_lossy()
        .into_owned();
    let out = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let rt = runtime();
        rt.rt.block_on(dig_node::handle_rpc_json(&rt.node, &req))
    }));
    match out.ok().and_then(|s| CString::new(s).ok()) {
        Some(c) => c.into_raw(),
        None => std::ptr::null_mut(),
    }
}

/// Free a string previously returned by [`dig_rpc`].
///
/// # Safety
/// `ptr` must be a pointer returned by [`dig_rpc`] and not yet freed; passing any
/// other value (or freeing twice) is undefined behavior. Null is ignored.
#[no_mangle]
pub unsafe extern "C" fn dig_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A full FFI round-trip that needs no network: an unknown method dispatches
    // through handle_rpc to the JSON-RPC "method not found" error. Exercises
    // dig_runtime_start (lazy init), dig_rpc (parse -> dispatch -> serialize),
    // and dig_free, proving the browser-side path works with no loopback server.
    #[test]
    fn ffi_roundtrip_unknown_method() {
        // Isolate the identity + cache the node creates so the test is hermetic.
        let tmp = std::env::temp_dir().join("dig-runtime-test");
        std::env::set_var("DIG_IDENTITY_DIR", tmp.join("id"));
        std::env::set_var("DIG_NODE_CACHE", tmp.join("cache"));

        dig_runtime_start();
        let req = CString::new(r#"{"jsonrpc":"2.0","id":7,"method":"nope"}"#).unwrap();
        let resp_ptr = unsafe { dig_rpc(req.as_ptr()) };
        assert!(!resp_ptr.is_null());
        let resp = unsafe { CStr::from_ptr(resp_ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { dig_free(resp_ptr) };
        assert!(resp.contains("method not found"), "got: {resp}");
        assert!(resp.contains("\"id\":7"), "id should round-trip: {resp}");
    }
}
