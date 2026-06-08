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
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "build",
            "-p",
            "digstore-guest",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
        ])
        .status()
        .expect("cargo build wasm32");
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
        assert!(exports.contains(required), "missing ABI export: {required} (have: {exports:?})");
    }
}
