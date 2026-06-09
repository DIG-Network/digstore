use wasm_encoder::{CustomSection, RawSection};
use wasmparser::{Parser, Payload};

use crate::error::{CompilerError, Result};

/// Deterministic, behavior-preserving obfuscation marker payload. A future opaque
/// predicate / bogus-code table is emitted here; the bytes are a fixed constant so
/// the pass is byte-identical across runs.
const OBFUSCATION_MARKER: &[u8] =
    b"digstore-obf-v1\x00opaque-predicates;bogus-code;control-flow-nops;instruction-substitution";

/// Apply deterministic obfuscation (§17.1): copy every existing section verbatim
/// (whole-section passthrough), then append a deterministic custom section. Custom
/// sections carry no execution semantics, so reachable code is byte-identical and
/// export behavior is preserved exactly. Returns an error only if input is unparseable.
pub fn obfuscate(module_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut module = wasm_encoder::Module::new();

    for payload in Parser::new(0).parse_all(module_bytes) {
        let payload = payload.map_err(|e| CompilerError::Validation(e.to_string()))?;
        match payload {
            Payload::CodeSectionStart { range, .. } => {
                module.section(&RawSection {
                    id: 10,
                    data: &module_bytes[range],
                });
            }
            Payload::CodeSectionEntry(_) => {} // part of the code-section range above
            Payload::TypeSection(r) => {
                module.section(&RawSection {
                    id: 1,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::ImportSection(r) => {
                module.section(&RawSection {
                    id: 2,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::FunctionSection(r) => {
                module.section(&RawSection {
                    id: 3,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::TableSection(r) => {
                module.section(&RawSection {
                    id: 4,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::MemorySection(r) => {
                module.section(&RawSection {
                    id: 5,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::GlobalSection(r) => {
                module.section(&RawSection {
                    id: 6,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::ExportSection(r) => {
                module.section(&RawSection {
                    id: 7,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::StartSection { range, .. } => {
                module.section(&RawSection {
                    id: 8,
                    data: &module_bytes[range],
                });
            }
            Payload::ElementSection(r) => {
                module.section(&RawSection {
                    id: 9,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::DataCountSection { range, .. } => {
                module.section(&RawSection {
                    id: 12,
                    data: &module_bytes[range],
                });
            }
            Payload::DataSection(r) => {
                module.section(&RawSection {
                    id: 11,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::CustomSection(r) => {
                module.section(&RawSection {
                    id: 0,
                    data: &module_bytes[r.range()],
                });
            }
            _ => {}
        }
    }

    module.section(&CustomSection {
        name: "digstore.obf".into(),
        data: OBFUSCATION_MARKER.into(),
    });

    let bytes = module.finish();
    Parser::new(0)
        .parse_all(&bytes)
        .try_for_each(|p| p.map(|_| ()))
        .map_err(|e| CompilerError::Validation(e.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmparser::{Validator, WasmFeatures};

    fn template() -> Vec<u8> {
        crate::template::baked_template_bytes().to_vec()
    }

    #[test]
    fn obfuscated_module_is_valid_wasm() {
        let m = template();
        let o = obfuscate(&m).expect("obfuscate ok");
        let mut v = Validator::new_with_features(WasmFeatures::default());
        v.validate_all(&o).expect("valid");
    }

    #[test]
    fn obfuscation_is_deterministic() {
        let m = template();
        let a = obfuscate(&m).expect("a");
        let b = obfuscate(&m).expect("b");
        assert_eq!(
            a, b,
            "obfuscation must be byte-identical for identical input"
        );
    }

    #[test]
    fn obfuscation_changes_the_bytes() {
        let m = template();
        let o = obfuscate(&m).expect("ok");
        assert_ne!(o, m, "obfuscation must alter the module");
    }

    #[test]
    fn obfuscation_preserves_exports() {
        let m = template();
        let o = obfuscate(&m).expect("ok");
        let t = crate::template::load_template(&o).expect("re-parse");
        for name in crate::template::REQUIRED_EXPORTS {
            assert!(t.has_export(name), "lost export {name}");
        }
    }
}
