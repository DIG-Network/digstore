//! §5.1: the emitted module MUST carry the Import section declaring the eight
//! `dig_host` host functions (§6.3). The baked guest template is the pinned
//! input that supplies them, and `inject_data_section` must preserve them into
//! the served module (inject.rs already passes imports through; this proves the
//! template actually carries them so it cannot silently regress to an
//! export-only stub).

use digstore_compiler::{baked_template_bytes, inject_data_section, REQUIRED_HOST_IMPORTS};
use wasmparser::{Parser, Payload, TypeRef};

/// Collect every `(module, name)` import pair declared by `bytes`.
fn imports(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        if let Payload::ImportSection(reader) = payload.unwrap() {
            for imp in reader {
                let imp = imp.unwrap();
                if let TypeRef::Func(_) = imp.ty {
                    out.push((imp.module.to_string(), imp.name.to_string()));
                }
            }
        }
    }
    out
}

#[test]
fn baked_template_declares_all_dig_host_imports() {
    let template = baked_template_bytes();
    let imps = imports(template);
    for name in REQUIRED_HOST_IMPORTS {
        assert!(
            imps.iter().any(|(m, n)| m == "dig_host" && n == name),
            "baked template missing §5.1 dig_host import {name}; declared imports: {imps:?}"
        );
    }
}

#[test]
fn emitted_module_preserves_all_dig_host_imports_after_injection() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0xCDu8; 128];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let imps = imports(&out);
    for name in REQUIRED_HOST_IMPORTS {
        assert!(
            imps.iter().any(|(m, n)| m == "dig_host" && n == name),
            "emitted module dropped §5.1 dig_host import {name}; declared imports: {imps:?}"
        );
    }
}
