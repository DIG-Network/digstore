use wasmparser::{Parser, Payload};

use crate::error::{CompilerError, Result};

/// Maximum linear-memory pages the served module may declare (§5.1: 16 MiB ceiling).
pub const MAX_MEMORY_PAGES: u64 = 256;

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
    /// Declared memory limits (min_pages, max_pages_opt) of memory 0.
    pub memory_min_pages: u64,
    pub memory_max_pages: Option<u64>,
}

impl Template {
    pub fn has_export(&self, name: &str) -> bool {
        self.exports.iter().any(|e| e == name)
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
    let mut memory_min_pages: Option<u64> = None;
    let mut memory_max_pages: Option<u64> = None;

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
        match payload {
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
        memory_min_pages: min,
        memory_max_pages,
    })
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
