mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{Compiler, CompilerConfig, CompilerError};

fn cfg(dir: &std::path::Path) -> CompilerConfig {
    CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate: false,
        optimize: false,
        template_override: None,
        // Small uniform budget keeps the emitted module tiny/fast.
        uniform_blob_len: 64 * 1024,
    }
}

#[test]
fn empty_trusted_set_is_refused() {
    let dir = std::env::temp_dir();
    let gens = sample_generations();
    let err = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        common::no_auth(),
        &[],
    )
    .unwrap_err();
    assert!(matches!(err, CompilerError::NoTrustedKeys));
}

#[test]
fn produces_result_with_exact_filename_and_stats() {
    use digstore_compiler::GenerationView;
    let dir = std::env::temp_dir().join(format!("digc-pipe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gens = sample_generations();
    let last_root = GenerationView::root(gens.last().unwrap());

    let outcome = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        common::no_auth(),
        &trusted_keys(),
    )
    .expect("compiles");

    let result = &outcome.result;
    assert_eq!(result.store_id, store_id());
    assert_eq!(result.roothash, last_root);
    let expected_name = format!(
        "{}-{}.dig",
        hex::encode(store_id().0),
        hex::encode(last_root.0)
    );
    assert_eq!(
        result.output_path.file_name().unwrap().to_str().unwrap(),
        expected_name
    );
    assert!(result.output_path.exists());
    assert_eq!(
        result.output_size,
        std::fs::metadata(&result.output_path).unwrap().len()
    );

    // Canonical core stats.
    assert_eq!(result.stats.generation_count, 2);
    assert_eq!(result.stats.chunk_count, 3);

    // Rich compiler detail.
    assert_eq!(outcome.detail.generation_count, 2);
    assert_eq!(outcome.detail.unique_chunk_count, 3);
    assert_eq!(outcome.detail.resource_count, 2);
    assert!(!outcome.detail.obfuscation_applied);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn obfuscation_flag_sets_stat_and_still_writes_valid_module() {
    use wasmparser::{Validator, WasmFeatures};
    let dir = std::env::temp_dir().join(format!("digc-obf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = cfg(&dir);
    c.obfuscate = true;
    let gens = sample_generations();
    let outcome = Compiler::compile(
        &c,
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        common::no_auth(),
        &trusted_keys(),
    )
    .expect("compiles");
    assert!(outcome.detail.obfuscation_applied);
    assert!(outcome.result.output_path.exists());
    let bytes = std::fs::read(&outcome.result.output_path).unwrap();
    let mut v = Validator::new_with_features(WasmFeatures::default());
    v.validate_all(&bytes).expect("obfuscated module validates");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pool_byte_len_is_total_unique_content_bytes() {
    // D4: the ChunkPool holds the unique chunk ciphertexts in global-index order;
    // deterministic filler is now a SEPARATE section (id 11), not interleaved. The
    // detail `pool_byte_len` therefore reports the exact total content byte count.
    let dir = std::env::temp_dir().join(format!("digc-buck-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gens = sample_generations();
    let outcome = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        common::no_auth(),
        &trusted_keys(),
    )
    .unwrap();
    // shared-chunk-body-0000(22) + alpha-body-1111(15) + beta-body-2222(14) = 51.
    assert_eq!(outcome.detail.pool_byte_len, 51);
    assert_eq!(outcome.detail.unique_chunk_count, 3);
    std::fs::remove_dir_all(&dir).ok();
}
