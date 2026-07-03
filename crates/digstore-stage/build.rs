//! Embed the REAL `digstore-guest` wasm into this crate so the in-process
//! stage→compile engine can compile a genuinely self-serving module (BINDING
//! contract D6): the compiled module's `get_content`/`get_proof` run the real
//! guest logic and the host's `serve_content` returns a real `ContentResponse`.
//!
//! This is the SINGLE embedded copy of the guest wasm for the whole engine —
//! `digstore-cli` re-exports `digstore_stage::embedded_guest_wasm()` rather than
//! embedding its own, and `dig-node` (linked by the DIG Browser via
//! `dig_runtime.dll`) gets the same wasm by depending on this crate, so the
//! browser can produce a capsule in-process WITHOUT the CLI.
//!
//! The guest wasm is produced by:
//!   cargo build -p digstore-guest --target wasm32-unknown-unknown --release
//! It lives at `<workspace>/target/wasm32-unknown-unknown/release/digstore_guest.wasm`.
//! We copy it into OUT_DIR so `src/lib.rs` can `include_bytes!` it.
//!
//! ## Guest-wasm location (in-workspace vs. git dependency)
//!
//! When this crate is built INSIDE the digstore workspace, the wasm is the sibling
//! build artifact at `<workspace>/target/...` (the default below). When it is built
//! as a GIT DEPENDENCY of another repo (e.g. the canonical `dig-node`), that
//! `../../target` path is under cargo's read-only git checkout and never holds the
//! artifact, so the consumer MUST point this build at a real wasm via the
//! `DIGSTORE_GUEST_WASM` environment variable (an absolute path to a
//! `digstore_guest.wasm` produced from a matching digstore rev). The env override
//! wins when set; otherwise the in-workspace default applies — so the digstore CLI
//! build is unchanged and an external consumer has a documented, backwards-compatible
//! way to supply the same wasm.

use std::path::PathBuf;

fn main() {
    // Consumer override: an absolute path to the guest wasm, for out-of-workspace
    // (git-dependency) builds. Wins over the in-workspace default when set.
    let override_path = std::env::var_os("DIGSTORE_GUEST_WASM").map(PathBuf::from);
    println!("cargo:rerun-if-env-changed=DIGSTORE_GUEST_WASM");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/digstore-stage -> workspace root is two levels up (in-workspace default).
    let default_guest = manifest_dir
        .join("..")
        .join("..")
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("digstore_guest.wasm");

    let guest = override_path.unwrap_or(default_guest);

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("digstore_guest.wasm");

    match std::fs::read(&guest) {
        Ok(bytes) => {
            std::fs::write(&dest, &bytes).expect("write embedded guest wasm");
        }
        Err(e) => {
            panic!(
                "digstore-stage requires the real guest wasm at {} (BINDING contract D6: \
                 the compiled module must serve itself). Build it first:\n  \
                 cargo build -p digstore-guest --target wasm32-unknown-unknown --release\n\
                 (or, for an out-of-workspace/git-dependency build, set DIGSTORE_GUEST_WASM \
                 to an absolute path to a matching digstore_guest.wasm)\n\
                 underlying error: {e}",
                guest.display()
            );
        }
    }

    println!("cargo:rerun-if-changed={}", guest.display());
}
