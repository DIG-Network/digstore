use digstore_compiler::{
    baked_template_bytes, inject_data_section, load_template, REQUIRED_EXPORTS,
};
use wasmparser::{Parser, Payload, Validator, WasmFeatures};

/// Collect (section_id, payload_bytes) for every TOP-LEVEL section, in order,
/// EXCLUDING data + data-count sections (ids 11, 12).
fn non_data_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.unwrap() {
            Payload::DataSection(_) | Payload::DataCountSection { .. } => {}
            Payload::CodeSectionStart { range, .. } => {
                out.push((10u8, bytes[range].to_vec()));
            }
            Payload::TypeSection(r) => out.push((1, bytes[r.range()].to_vec())),
            Payload::ImportSection(r) => out.push((2, bytes[r.range()].to_vec())),
            Payload::FunctionSection(r) => out.push((3, bytes[r.range()].to_vec())),
            Payload::TableSection(r) => out.push((4, bytes[r.range()].to_vec())),
            Payload::MemorySection(r) => out.push((5, bytes[r.range()].to_vec())),
            Payload::GlobalSection(r) => out.push((6, bytes[r.range()].to_vec())),
            Payload::ExportSection(r) => out.push((7, bytes[r.range()].to_vec())),
            Payload::ElementSection(r) => out.push((9, bytes[r.range()].to_vec())),
            _ => {}
        }
    }
    out
}

#[test]
fn non_data_sections_are_byte_identical_after_injection() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0xEEu8; 256];
    // Inject at the reserved offset; template min memory (4 pages) already fits.
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    // Memory section MAY change (min bump), so exclude id 5 from byte-identity.
    let before: Vec<_> = non_data_sections(&template)
        .into_iter()
        .filter(|(id, _)| *id != 5)
        .collect();
    let after: Vec<_> = non_data_sections(&out)
        .into_iter()
        .filter(|(id, _)| *id != 5)
        .collect();
    assert_eq!(
        before, after,
        "non-Data, non-Memory sections must be byte-identical"
    );
}

#[test]
fn injected_module_is_valid_wasm() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0x01u8; 64];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let mut validator = Validator::new_with_features(WasmFeatures::default());
    validator.validate_all(&out).expect("module validates");
}

#[test]
fn injected_module_still_exports_full_abi() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0x01u8; 64];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let t = load_template(&out).expect("re-parse ok");
    for name in REQUIRED_EXPORTS {
        assert!(t.has_export(name), "lost export {name}");
    }
}

#[test]
fn injected_data_blob_is_present_in_data_section() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0xABu8; 32];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let mut found = false;
    for payload in Parser::new(0).parse_all(&out) {
        if let Payload::DataSection(reader) = payload.unwrap() {
            for seg in reader {
                if seg.unwrap().data == blob.as_slice() {
                    found = true;
                }
            }
        }
    }
    assert!(found, "injected blob not found in data section");
}

#[test]
fn existing_data_segments_are_preserved_alongside_the_injected_blob() {
    use wasmparser::{Parser, Payload};
    // A template with its OWN data segment (e.g. the guest's .rodata) at offset
    // 1024. Injecting the DIGS blob at a DIFFERENT offset must NOT drop the
    // template's segment — a real module needs its own static data to run.
    let watsrc = r#"(module
      (memory (export "memory") 4 256)
      (func (export "get_store_id") (result i64) (i64.const 0))
      (func (export "get_current_roothash") (result i64) (i64.const 0))
      (func (export "get_roothash_history") (result i64) (i64.const 0))
      (func (export "get_public_key") (result i64) (i64.const 0))
      (func (export "get_metadata") (result i64) (i64.const 0))
      (func (export "get_authentication_info") (result i64) (i64.const 0))
      (func (export "get_content") (param i32 i32) (result i64) (i64.const 0))
      (func (export "get_proof") (param i32 i32) (result i64) (i64.const 0))
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "dealloc") (param i32 i32))
      (func (export "init") (result i32) (i32.const 0))
      (data (i32.const 1024) "GUEST-RODATA-MARKER"))"#;
    let template = wat::parse_str(watsrc).unwrap();
    let blob = vec![0xABu8; 32];
    // Inject at a high offset so it does not overlap the template's 1024 segment.
    let out = inject_data_section(&template, &blob, 1_048_576).expect("inject ok");

    let mut saw_rodata = false;
    let mut saw_blob = false;
    for payload in Parser::new(0).parse_all(&out) {
        if let Payload::DataSection(reader) = payload.unwrap() {
            for seg in reader {
                let data = seg.unwrap().data;
                if data == b"GUEST-RODATA-MARKER" {
                    saw_rodata = true;
                }
                if data == blob.as_slice() {
                    saw_blob = true;
                }
            }
        }
    }
    assert!(saw_rodata, "template's own data segment must be preserved");
    assert!(saw_blob, "injected DIGS blob must be present");
}

#[test]
fn large_blob_bumps_memory_min_pages_and_stays_valid() {
    let template = baked_template_bytes().to_vec();
    // Offset 65536 + 1 MiB blob => needs ceil((65536+1048576)/65536) = 17 pages
    // (65536*17 = 1114112 = 65536 + 1048576, exact). The template declares only
    // 4 pages, so the min MUST be bumped to fit.
    let blob = vec![0x5Au8; 1024 * 1024];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");

    let mut validator = Validator::new_with_features(WasmFeatures::default());
    validator.validate_all(&out).expect("validates");

    // Re-parse and assert the declared min grew to the required 17 pages.
    let mut min_pages = 0u64;
    for payload in Parser::new(0).parse_all(&out) {
        if let Payload::MemorySection(reader) = payload.unwrap() {
            for m in reader {
                min_pages = m.unwrap().initial;
            }
        }
    }
    assert!(
        min_pages >= 17,
        "memory min pages not bumped, got {min_pages}"
    );
}
