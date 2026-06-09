//! Embed the REAL `digstore-guest` wasm into the CLI binary so a `commit` can
//! compile a genuinely self-serving module (BINDING contract D6): the compiled
//! module's `get_content`/`get_proof` run the real guest logic and the host's
//! `serve_content` returns a real `ContentResponse`.
//!
//! The guest wasm is produced by:
//!   cargo build -p digstore-guest --target wasm32-unknown-unknown --release
//! It lives at `<workspace>/target/wasm32-unknown-unknown/release/digstore_guest.wasm`.
//! We copy it into OUT_DIR so `src/ops/serve.rs` can `include_bytes!` it.

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/digstore-cli -> workspace root is two levels up.
    let guest = manifest_dir
        .join("..")
        .join("..")
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("digstore_guest.wasm");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("digstore_guest.wasm");

    match std::fs::read(&guest) {
        Ok(bytes) => {
            std::fs::write(&dest, &bytes).expect("write embedded guest wasm");
        }
        Err(e) => {
            panic!(
                "digstore-cli requires the real guest wasm at {} (BINDING contract D6: \
                 the compiled module must serve itself). Build it first:\n  \
                 cargo build -p digstore-guest --target wasm32-unknown-unknown --release\n\
                 underlying error: {e}",
                guest.display()
            );
        }
    }

    println!("cargo:rerun-if-changed={}", guest.display());
}
