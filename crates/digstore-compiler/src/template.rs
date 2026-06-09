use wasmparser::{Parser, Payload};

use crate::error::{CompilerError, Result};

/// Maximum linear-memory pages the served module may declare (§5.1: 16 MiB ceiling).
pub const MAX_MEMORY_PAGES: u64 = 256;

/// Nominal linear-memory minimum the guest template declares (§5.1: `minimum: 1`,
/// i.e. one 64 KiB page). Injection raises the EMITTED module's `min` to cover
/// `DIGS_DATA_OFFSET + total_len` (D2), but the committed template literal MUST
/// match the §5.1 nominal value of 1 page.
pub const NOMINAL_MEMORY_MIN_PAGES: u64 = 1;

/// Host functions the served module must import from the `dig_host` module
/// (§5.1 Import section / §6.3 Host Imports). The compiler bakes a guest
/// template that declares all eight; `inject_data_section` preserves the Import
/// section verbatim into the emitted module.
pub const REQUIRED_HOST_IMPORTS: &[&str] = &[
    "host_get_public_key",
    "host_create_attestation",
    "host_establish_session",
    "host_verify_session",
    "jwks_fetch",
    "host_get_current_time",
    "host_random_bytes",
    "host_read_return_buffer",
];

/// Exports the served module must expose (guest ABI).
pub const REQUIRED_EXPORTS: &[&str] = &[
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
];

/// A validated guest template ready for data injection.
#[derive(Debug)]
pub struct Template {
    pub bytes: Vec<u8>,
    exports: Vec<String>,
    /// `(module, name)` pairs of every function import the template declares.
    imports: Vec<(String, String)>,
    /// Declared memory limits (min_pages, max_pages_opt) of memory 0.
    pub memory_min_pages: u64,
    pub memory_max_pages: Option<u64>,
    /// Whether memory 0 is a 64-bit memory (§5.1 requires this to be false).
    pub memory64: bool,
    /// Whether memory 0 is shared (§5.1 requires this to be false).
    pub memory_shared: bool,
}

impl Template {
    pub fn has_export(&self, name: &str) -> bool {
        self.exports.iter().any(|e| e == name)
    }

    /// Whether the template imports `dig_host::{name}` (a function import).
    pub fn has_host_import(&self, name: &str) -> bool {
        self.imports
            .iter()
            .any(|(m, n)| m == "dig_host" && n == name)
    }
}

/// The pinned template bytes assembled by `build.rs` from the committed `.wat`.
pub fn baked_template_bytes() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/digstore_guest_template.wasm"))
}

/// Parse + validate the template (§5.1): collect export names, assert the full
/// required ABI surface, and assert memory bounds (a memory exists, max <= 256).
pub fn load_template(bytes: &[u8]) -> Result<Template> {
    let mut exports = Vec::new();
    let mut imports: Vec<(String, String)> = Vec::new();
    let mut memory_min_pages: Option<u64> = None;
    let mut memory_max_pages: Option<u64> = None;
    let mut memory64 = false;
    let mut memory_shared = false;

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
        match payload {
            Payload::ImportSection(reader) => {
                for import in reader {
                    let import =
                        import.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    imports.push((import.module.to_string(), import.name.to_string()));
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export =
                        export.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    exports.push(export.name.to_string());
                }
            }
            Payload::MemorySection(reader) => {
                for mem in reader {
                    let mem = mem.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    if memory_min_pages.is_none() {
                        memory_min_pages = Some(mem.initial);
                        memory_max_pages = mem.maximum;
                        memory64 = mem.memory64;
                        memory_shared = mem.shared;
                    }
                }
            }
            _ => {}
        }
    }

    for name in REQUIRED_EXPORTS {
        if !exports.iter().any(|e| e == name) {
            return Err(CompilerError::InvalidTemplate(format!(
                "missing export {name}"
            )));
        }
    }

    let min = memory_min_pages
        .ok_or_else(|| CompilerError::InvalidTemplate("template declares no memory".into()))?;
    // §5.1: the single linear memory MUST be 32-bit (`memory64: false`). A
    // template declaring a 64-bit memory is rejected outright.
    if memory64 {
        return Err(CompilerError::InvalidTemplate(
            "memory declares memory64 but §5.1 requires a 32-bit memory (memory64: false)".into(),
        ));
    }
    // §5.1: the single linear memory MUST be unshared (`shared: false`). A
    // template declaring a shared memory is rejected outright so the compiler
    // never copies the shared flag verbatim into the served module.
    if memory_shared {
        return Err(CompilerError::InvalidTemplate(
            "memory declares shared but §5.1 requires an unshared memory (shared: false)".into(),
        ));
    }
    // §5.1: a DECLARED maximum must not exceed the 16 MiB ceiling. A raw guest
    // template (rustc/LLVM output) may legitimately declare NO maximum; the
    // compiler normalizes the EMITTED module to `Some(256)` during injection
    // (see `inject_data_section`) and the strict ceiling is enforced on that
    // emitted module via `assert_memory_ceiling`.
    if let Some(max) = memory_max_pages {
        if max > MAX_MEMORY_PAGES {
            return Err(CompilerError::InvalidTemplate(format!(
                "memory max {max} pages exceeds ceiling {MAX_MEMORY_PAGES}"
            )));
        }
    }

    Ok(Template {
        bytes: bytes.to_vec(),
        exports,
        imports,
        memory_min_pages: min,
        memory_max_pages,
        memory64,
        memory_shared,
    })
}

/// Enforce the §5.1 module-declared memory ceiling on an EMITTED module: memory 0
/// MUST declare `maximum: Some(256)` exactly (16 MiB). Unlike [`load_template`]
/// (which tolerates a raw guest template that declares no maximum), this is the
/// strict post-injection invariant — the served `.wasm` always carries the cap.
pub fn assert_memory_ceiling(module: &[u8]) -> Result<()> {
    let t = load_template(module)?;
    match t.memory_max_pages {
        Some(max) if max == MAX_MEMORY_PAGES => Ok(()),
        Some(max) => Err(CompilerError::Validation(format!(
            "emitted module memory max {max} pages must equal §5.1 ceiling {MAX_MEMORY_PAGES} (16 MiB)"
        ))),
        None => Err(CompilerError::Validation(format!(
            "emitted module must declare memory maximum {MAX_MEMORY_PAGES} pages (§5.1 16 MiB ceiling)"
        ))),
    }
}

/// Enforce the §5.1 Import section on a module: it MUST import every
/// `dig_host` host function in [`REQUIRED_HOST_IMPORTS`] (§6.3). Used as a
/// post-injection invariant so the emitted module can never silently regress to
/// an export-only stub that declares no host imports.
pub fn assert_host_imports(module: &[u8]) -> Result<()> {
    let t = load_template(module)?;
    for name in REQUIRED_HOST_IMPORTS {
        if !t.has_host_import(name) {
            return Err(CompilerError::Validation(format!(
                "emitted module missing §5.1 dig_host import {name}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_template_has_all_required_exports() {
        let bytes = baked_template_bytes();
        let t = load_template(bytes).expect("template valid");
        for name in REQUIRED_EXPORTS {
            assert!(t.has_export(name), "missing export {name}");
        }
    }

    #[test]
    fn template_missing_export_is_rejected() {
        // Full ABI EXCEPT `get_content` -> rejection must name get_content.
        let watsrc = r#"(module
          (memory (export "memory") 1 256)
          (func (export "get_store_id") (result i64) (i64.const 0))
          (func (export "get_current_roothash") (result i64) (i64.const 0))
          (func (export "get_roothash_history") (result i64) (i64.const 0))
          (func (export "get_public_key") (result i64) (i64.const 0))
          (func (export "get_metadata") (result i64) (i64.const 0))
          (func (export "get_authentication_info") (result i64) (i64.const 0))
          (func (export "get_proof") (param i32 i32) (result i64) (i64.const 0))
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "dealloc") (param i32 i32))
          (func (export "init") (result i32) (i32.const 0)))"#;
        let bytes = wat::parse_str(watsrc).unwrap();
        let err = load_template(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("get_content"),
            "unexpected error: {err}"
        );
    }

    const FULL_ABI_FUNCS: &str = r#"
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
          (func (export "init") (result i32) (i32.const 0)))"#;

    fn full_abi_module(memory_decl: &str) -> Vec<u8> {
        let src = format!("(module\n          {memory_decl}\n{FULL_ABI_FUNCS}");
        wat::parse_str(&src).unwrap()
    }

    #[test]
    fn raw_template_without_declared_memory_max_is_accepted_by_load_template() {
        // A raw guest template (rustc/LLVM) legitimately declares NO maximum;
        // `load_template` tolerates it. The §5.1 ceiling is imposed on the
        // EMITTED module by injection + `assert_memory_ceiling`.
        let bytes = full_abi_module(r#"(memory (export "memory") 1)"#);
        let t = load_template(&bytes).expect("raw template valid");
        assert_eq!(t.memory_max_pages, None);
    }

    #[test]
    fn emitted_module_without_declared_memory_max_fails_ceiling_check() {
        // §5.1: the served module MUST declare `maximum: Some(256)`.
        let bytes = full_abi_module(r#"(memory (export "memory") 1)"#);
        let err = assert_memory_ceiling(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("must declare memory maximum"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn emitted_module_with_memory_max_under_ceiling_fails_ceiling_check() {
        // §5.1: the module-declared cap is EXACTLY 256 pages (16 MiB).
        let bytes = full_abi_module(r#"(memory (export "memory") 1 128)"#);
        let err = assert_memory_ceiling(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("must equal §5.1 ceiling"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn emitted_module_with_memory_max_exactly_ceiling_passes() {
        let bytes = full_abi_module(r#"(memory (export "memory") 1 256)"#);
        assert_memory_ceiling(&bytes).expect("256 is the ceiling");
        // The baked template is committed with the exact ceiling too.
        let t = load_template(baked_template_bytes()).expect("baked template valid");
        assert_eq!(t.memory_max_pages, Some(MAX_MEMORY_PAGES));
    }

    #[test]
    fn template_declaring_memory64_is_rejected() {
        // §5.1: the single linear memory MUST be `memory64: false`. A template
        // that declares a 64-bit memory must be rejected by `load_template`.
        let bytes = full_abi_module(r#"(memory (export "memory") i64 1 256)"#);
        let err = load_template(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("memory64"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn template_declaring_shared_memory_is_rejected() {
        // §5.1: the single linear memory MUST be `shared: false`. A template that
        // declares a shared memory must be rejected by `load_template` (the
        // compiler must not copy the shared flag verbatim into the served module).
        // A shared memory must declare a maximum, so use 1 256 shared.
        let bytes = full_abi_module(r#"(memory (export "memory") 1 256 shared)"#);
        let err = load_template(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("shared"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn baked_template_declares_nominal_memory_min_of_one_page() {
        // §5.1 (Module Sections) fixes the nominal `MemoryType { minimum: 1, .. }`
        // (one 64 KiB page). The committed guest template literal MUST equal that
        // nominal minimum; injection (D2) independently raises the EMITTED module's
        // `min` to cover DIGS_DATA_OFFSET + total_len.
        let t = load_template(baked_template_bytes()).expect("baked template valid");
        assert_eq!(
            t.memory_min_pages, NOMINAL_MEMORY_MIN_PAGES,
            "§5.1 nominal MemoryType.minimum is {NOMINAL_MEMORY_MIN_PAGES} page; \
             baked template declares {}",
            t.memory_min_pages
        );
    }

    #[test]
    fn template_with_memory_max_over_ceiling_is_rejected() {
        // Full ABI but max pages 257 (> 256) -> rejected.
        let watsrc = r#"(module
          (memory (export "memory") 1 257)
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
          (func (export "init") (result i32) (i32.const 0)))"#;
        let bytes = wat::parse_str(watsrc).unwrap();
        let err = load_template(&bytes).unwrap_err();
        assert!(err.to_string().contains("exceeds ceiling"));
    }
}
