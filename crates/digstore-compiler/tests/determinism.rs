mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{Compiler, CompilerConfig};

fn compile_to_bytes(dir: &std::path::Path, obfuscate: bool) -> Vec<u8> {
    let cfg = CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate,
        optimize: false,
        template_override: None,
    };
    let gens = sample_generations();
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
    std::fs::read(&outcome.result.output_path).unwrap()
}

#[test]
fn two_compiles_are_byte_identical() {
    let d1 = std::env::temp_dir().join(format!("digc-det1-{}", std::process::id()));
    let d2 = std::env::temp_dir().join(format!("digc-det2-{}", std::process::id()));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();

    let a = compile_to_bytes(&d1, false);
    let b = compile_to_bytes(&d2, false);
    assert_eq!(a, b, "compilation must be byte-identical (paper 19.3)");

    std::fs::remove_dir_all(&d1).ok();
    std::fs::remove_dir_all(&d2).ok();
}

#[test]
fn two_obfuscated_compiles_are_byte_identical() {
    let d1 = std::env::temp_dir().join(format!("digc-detob1-{}", std::process::id()));
    let d2 = std::env::temp_dir().join(format!("digc-detob2-{}", std::process::id()));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();

    let a = compile_to_bytes(&d1, true);
    let b = compile_to_bytes(&d2, true);
    assert_eq!(a, b, "obfuscated compilation must also be byte-identical");

    std::fs::remove_dir_all(&d1).ok();
    std::fs::remove_dir_all(&d2).ok();
}
