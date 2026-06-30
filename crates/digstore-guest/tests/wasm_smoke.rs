//! Builds the guest to wasm32 and asserts the module validates and exports the
//! full ABI. Uses wasmparser to validate and to enumerate exports.

use std::path::PathBuf;
use std::process::Command;

/// Workspace root = parent of `crates/`. CARGO_MANIFEST_DIR points at the crate
/// dir (`<root>/crates/digstore-guest`); go up two levels to reach `<root>`.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn build_wasm() -> Vec<u8> {
    let root = workspace_root();
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&root).args([
        "build",
        "-p",
        "digstore-guest",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ]);
    // The wasm32 guest build must NOT inherit the parent process's RUSTFLAGS.
    // Under `cargo llvm-cov` the harness exports `-C instrument-coverage` (via
    // `RUSTFLAGS` / `CARGO_ENCODED_RUSTFLAGS`); that flag is unsupported on
    // `wasm32-unknown-unknown` and makes this child build fail to compile. The
    // guest is a freestanding wasm artifact that is never coverage-instrumented,
    // so strip those vars for the child — leaving the host coverage run intact.
    cmd.env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTDOCFLAGS");
    // Pin the child build's target dir to `<root>/target` so the artifact lands
    // exactly where we read it below, regardless of any inherited
    // `CARGO_TARGET_DIR` (e.g. the `cargo llvm-cov` target dir, which would
    // otherwise redirect the output away from this read path).
    cmd.env("CARGO_TARGET_DIR", root.join("target"));
    let status = cmd.status().expect("cargo build wasm32");
    assert!(status.success(), "wasm build must succeed");
    let path = root.join("target/wasm32-unknown-unknown/release/digstore_guest.wasm");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read built wasm module {path:?}: {e}"))
}

#[test]
fn module_validates_and_exports_full_abi() {
    let bytes = build_wasm();
    // Validate the module.
    wasmparser::validate(&bytes).expect("module must be valid wasm");

    // Collect exported function/memory names.
    let mut exports = std::collections::BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::ExportSection(reader) = payload.unwrap() {
            for e in reader {
                exports.insert(e.unwrap().name.to_string());
            }
        }
    }
    for required in [
        "get_store_id",
        "get_current_roothash",
        "get_roothash_history",
        "get_public_key",
        "get_metadata",
        "get_authentication_info",
        "get_content",
        "get_proof",
        "alloc",
        "dealloc",
        "init",
        "memory",
    ] {
        assert!(
            exports.contains(required),
            "missing ABI export: {required} (have: {exports:?})"
        );
    }

    // §5.1 Import section / §6.3 Host Imports: the guest module MUST declare all
    // eight dig_host host functions. LLVM only emits an import that is reachable
    // from an export, so `init` anchors them (see `imports::retain_dig_host_imports`);
    // this guards that retention against silent regression.
    let mut imports = std::collections::BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::ImportSection(reader) = payload.unwrap() {
            for i in reader {
                let i = i.unwrap();
                if i.module == "dig_host" {
                    imports.insert(i.name.to_string());
                }
            }
        }
    }
    for required in [
        "host_get_public_key",
        "host_create_attestation",
        "host_establish_session",
        "host_verify_session",
        "jwks_fetch",
        "host_get_current_time",
        "host_random_bytes",
        "host_read_return_buffer",
    ] {
        assert!(
            imports.contains(required),
            "missing §5.1 dig_host import: {required} (have: {imports:?})"
        );
    }
}
