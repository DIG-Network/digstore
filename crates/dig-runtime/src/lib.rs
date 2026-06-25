//! dig-runtime — the DIG browser's NATIVE in-process runtime.
//!
//! Compiled as a `cdylib` (`dig_runtime.dll`) shipped next to the browser
//! executable (like `chrome.dll`) and loaded IN-PROCESS at browser startup
//! (`chrome_browser_main`'s `PostBrowserStart` → `LoadLibrary` +
//! `dig_runtime_start`). It runs the DIG node service on a dedicated tokio
//! runtime on a background thread INSIDE the browser process — there is NO
//! `dig-node.exe` sidecar. The browser's dig:// loader reaches it over loopback
//! exactly as before; the only change is that the browser now hosts the node
//! itself, so a clean install has native dig:// with nothing extra to launch.
//!
//! Heavy Rust deps (wasmtime, tokio, reqwest, blst via the node) build freely as
//! a cargo cdylib — the route Chromium's restricted Rust/GN build can't take —
//! and run fine in the browser (broker) process, which is not ACG/JIT-locked the
//! way renderer processes are.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set once `dig_runtime_start` has spawned the runtime, so repeated calls (or a
/// double load) don't start a second node fighting for the same loopback port.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Start the DIG native runtime exactly once. Idempotent and non-blocking:
/// spawns a background OS thread that owns a multi-thread tokio runtime running
/// the DIG node to completion, then returns immediately so the browser's startup
/// is never blocked.
///
/// # Safety
/// Exported with the C ABI so the browser can `GetProcAddress` it after
/// `LoadLibrary`. It takes no arguments and does not unwind across the FFI
/// boundary: it returns before any work runs, and all node work (including any
/// panic) is confined to the spawned thread.
#[no_mangle]
pub extern "C" fn dig_runtime_start() {
    // Win the race exactly once; a second caller (or a second module load) is a
    // no-op rather than a second node binding the same port.
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("dig-runtime".into())
        .spawn(|| {
            // A panic here unwinds + ends only this thread (workspace builds with
            // panic = "unwind"); the browser process is unaffected.
            if let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                rt.block_on(dig_node::run());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_idempotent_and_nonblocking() {
        // First call flips the guard and returns promptly (the node runs on its
        // own thread). A second call must be a cheap no-op, not a second start.
        dig_runtime_start();
        assert!(STARTED.load(Ordering::SeqCst));
        dig_runtime_start(); // must return without spawning a second runtime
    }
}
