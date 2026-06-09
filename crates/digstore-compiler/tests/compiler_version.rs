//! §5: "Compiler version 1.0.0; module format version 1."
//!
//! The module-format-version half is already carried by the DIGS blob header
//! byte (== 1). This test pins the COMPILER-VERSION half the spec mandates: the
//! compiler must carry the exact version string "1.0.0", the crate version must
//! equal it (so the artifact's `CARGO_PKG_VERSION` matches the spec), and a
//! successful compilation must record that version in its outcome.

mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{Compiler, CompilerConfig, COMPILER_VERSION};

#[test]
fn compiler_version_constant_is_exactly_the_spec_value() {
    // §5: "Compiler version 1.0.0".
    assert_eq!(COMPILER_VERSION, "1.0.0");
}

#[test]
fn crate_version_matches_the_spec_compiler_version() {
    // The artifact must carry the spec-stated version: the crate version (what
    // `CARGO_PKG_VERSION` bakes into the binary) equals the §5 compiler version.
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
    assert_eq!(COMPILER_VERSION, env!("CARGO_PKG_VERSION"));
}

#[test]
fn compile_outcome_records_compiler_version() {
    let dir = std::env::temp_dir().join(format!("digc-ver-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gens = sample_generations();
    let cfg = CompilerConfig {
        output_dir: dir.clone(),
        obfuscate: false,
        optimize: false,
        template_override: None,
    };
    let outcome = Compiler::compile(
        &cfg,
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        common::no_auth(),
        &trusted_keys(),
    )
    .expect("compiles");

    // §5: the emitted artifact carries the compiler version it was built by.
    assert_eq!(outcome.detail.compiler_version, "1.0.0");
    assert_eq!(outcome.detail.compiler_version, COMPILER_VERSION);

    std::fs::remove_dir_all(&dir).ok();
}
