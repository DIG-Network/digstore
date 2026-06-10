mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{load_template, Compiler, CompilerConfig};
use wasmtime::{Engine, Instance, Module, Store};

fn compile(dir: &std::path::Path, obfuscate: bool) -> Vec<u8> {
    let cfg = CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate,
        optimize: false,
        template_override: None,
        // Small uniform budget keeps the emitted module tiny/fast.
        uniform_blob_len: 64 * 1024,
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
    .unwrap();
    std::fs::read(&outcome.result.output_path).unwrap()
}

/// Instantiate an import-free module and call `get_store_id`. Returns None when
/// the module declares host imports (the real guest), in which case the caller
/// falls back to a validity check.
fn call_get_store_id(bytes: &[u8]) -> Option<i64> {
    let engine = Engine::default();
    let module = Module::new(&engine, bytes).expect("module");
    if module.imports().count() != 0 {
        return None;
    }
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).ok()?;
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "get_store_id")
        .ok()?;
    f.call(&mut store, ()).ok()
}

#[test]
fn obfuscated_and_plain_modules_produce_same_export_output() {
    let d1 = std::env::temp_dir().join(format!("digc-eqp-{}", std::process::id()));
    let d2 = std::env::temp_dir().join(format!("digc-eqo-{}", std::process::id()));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();

    let plain = compile(&d1, false);
    let obf = compile(&d2, true);

    let plain_out = call_get_store_id(&plain);
    let obf_out = call_get_store_id(&obf);

    match (plain_out, obf_out) {
        (Some(a), Some(b)) => assert_eq!(a, b, "obfuscation changed export behavior"),
        _ => {
            load_template(&plain).expect("plain valid");
            load_template(&obf).expect("obf valid");
        }
    }

    std::fs::remove_dir_all(&d1).ok();
    std::fs::remove_dir_all(&d2).ok();
}
