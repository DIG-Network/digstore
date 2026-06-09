use wasm_encoder::reencode::{Reencode, RoundtripReencoder};
use wasm_encoder::{
    BlockType, CodeSection, CustomSection, Function, FunctionSection, Instruction, RawSection,
    TypeSection, ValType,
};
use wasmparser::{Parser, Payload};

use crate::error::{CompilerError, Result};

/// Deterministic obfuscation metadata marker (§17.1). This is METADATA only — it
/// is NOT a substitute for transformation; the real passes below rewrite the code
/// section. The bytes are a fixed constant so the pass is byte-identical per input.
const OBFUSCATION_MARKER: &[u8] =
    b"digstore-obf-v1\x00opaque-predicates;bogus-code;control-flow-nops;instruction-substitution";

/// How many deterministic dead "bogus" functions to append (§17.1 bogus code).
/// Pure unreferenced bloat: never called, never exported, never in an element
/// segment — so by wasm reachability they cannot affect observable behavior.
const BOGUS_FUNCTION_COUNT: u32 = 8;

/// Apply deterministic, behavior-preserving obfuscation (§17.1).
///
/// REAL transformations performed over the module's CODE section (decode via
/// `wasmparser`, re-encode via `wasm-encoder`/`reencode`), all deterministic (no
/// RNG; every choice is derived from a fixed function/operator index):
///
/// 1. control-flow nops — `nop` (0x01) opcodes inserted at deterministic operator
///    boundaries in every function body. `nop` is a pure no-op and valid anywhere
///    an instruction is valid, so this never changes behavior.
/// 2. opaque predicates — every function body is prefixed with an always-true
///    guard `i32.const 1; if (empty blocktype); nop; end`. It is self-contained
///    and STACK-NEUTRAL (consumes the i32 it pushes, empty block type, empty
///    then-branch) and is inserted as a complete unit at the body start, so it
///    shifts NO existing branch depths — provably behavior-preserving.
/// 3. bogus code — [`BOGUS_FUNCTION_COUNT`] unreferenced dead functions are
///    appended (new fn type + function entries + code bodies). They are never
///    called, exported, or referenced by any element segment, so by wasm
///    reachability they cannot affect behavior.
/// 4. instruction substitution — DEFERRED. A general semantics-preserving operator
///    substitution (e.g. `i32.const k` -> an equivalent computation) requires
///    proving stack/type/value equivalence for every reachable operator context,
///    which is risky to do soundly here. Per the task's "omit rather than risk
///    behavior" guidance it is intentionally NOT applied; (1)-(3) are genuine.
///
/// The custom-section marker is appended as METADATA only and is NOT relied upon
/// as the transformation. Returns an error only if the input is unparseable or the
/// re-encoded module fails to re-parse.
pub fn obfuscate(module_bytes: &[u8]) -> Result<Vec<u8>> {
    // First pass: count the existing types. We must re-emit Type + Function + Code
    // coherently (bogus code needs a new fn type plus new function/body entries),
    // so those three sections are rebuilt rather than passed through verbatim.
    let mut existing_type_count: u32 = 0;
    for payload in Parser::new(0).parse_all(module_bytes) {
        let payload = payload.map_err(|e| CompilerError::Validation(e.to_string()))?;
        if let Payload::TypeSection(reader) = payload {
            existing_type_count = reader.count();
        }
    }
    // The bogus functions share one freshly-appended `() -> ()` type, whose index
    // is the first slot after the existing types.
    let bogus_type_index = existing_type_count;

    // Build the transformed CODE section, then re-emit the module in the original
    // section order substituting the rebuilt Type / Function / Code sections.
    let mut reencoder = RoundtripReencoder;
    let code = build_code_section(module_bytes, &mut reencoder)?;
    let final_bytes = assemble(module_bytes, code, bogus_type_index)?;

    Parser::new(0)
        .parse_all(&final_bytes)
        .try_for_each(|p| p.map(|_| ()))
        .map_err(|e| CompilerError::Validation(e.to_string()))?;
    Ok(final_bytes)
}

fn reencode_err(
    e: wasm_encoder::reencode::Error<core::convert::Infallible>,
) -> CompilerError {
    CompilerError::Validation(format!("reencode failed: {e:?}"))
}

/// Build the transformed CODE section: every original body is rewritten with an
/// opaque-predicate prefix + deterministic nops, then [`BOGUS_FUNCTION_COUNT`]
/// dead bogus bodies are appended.
fn build_code_section(
    module_bytes: &[u8],
    reencoder: &mut RoundtripReencoder,
) -> Result<CodeSection> {
    let mut code = CodeSection::new();
    let mut func_ordinal: u32 = 0;
    for payload in Parser::new(0).parse_all(module_bytes) {
        let payload = payload.map_err(|e| CompilerError::Validation(e.to_string()))?;
        if let Payload::CodeSectionEntry(body) = payload {
            let f = transform_body(reencoder, &body, func_ordinal)?;
            code.function(&f);
            func_ordinal += 1;
        }
    }
    // Bogus dead functions: deterministic, side-effect-free, returning nothing.
    for i in 0..BOGUS_FUNCTION_COUNT {
        let mut f = Function::new(Vec::<(u32, ValType)>::new());
        // A short deterministic dead sequence varying by index; never reachable.
        for _ in 0..(i + 1) {
            f.instruction(&Instruction::Nop);
        }
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    Ok(code)
}

/// Rewrite one function body: parse locals + operators, prepend an always-true
/// opaque-predicate guard, and splice deterministic `nop`s between operators.
fn transform_body(
    reencoder: &mut RoundtripReencoder,
    body: &wasmparser::FunctionBody<'_>,
    func_ordinal: u32,
) -> Result<Function> {
    // Preserve locals exactly.
    let mut locals = Vec::new();
    for pair in body.get_locals_reader().map_err(parser_err)? {
        let (cnt, ty) = pair.map_err(parser_err)?;
        locals.push((cnt, reencoder.val_type(ty).map_err(reencode_err)?));
    }
    let mut f = Function::new(locals);

    // (2) Opaque predicate: always-true, stack-neutral, self-contained guard
    // inserted at body start. `i32.const 1; if (empty); nop; end`.
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::Nop);
    f.instruction(&Instruction::End);

    // Read the original operators and splice deterministic nops between them.
    let mut reader = body.get_operators_reader().map_err(parser_err)?;
    let mut op_index: u32 = 0;
    // The final operator of a body is the function-closing `End`; we must NOT
    // insert a nop AFTER it (nothing may follow the closing End). Track it by
    // peeking: collect operators first.
    let mut ops = Vec::new();
    while !reader.eof() {
        ops.push(reader.read().map_err(parser_err)?);
    }
    let last = ops.len().saturating_sub(1);
    for (i, op) in ops.into_iter().enumerate() {
        let instr = reencoder.instruction(op).map_err(reencode_err)?;
        f.instruction(&instr);
        // Deterministic nop insertion: after every operator except the closing
        // End, insert a nop when (op_index + func_ordinal) is even. Derived purely
        // from indices => no RNG, fully deterministic.
        if i != last && (op_index.wrapping_add(func_ordinal)) % 2 == 0 {
            f.instruction(&Instruction::Nop);
        }
        op_index = op_index.wrapping_add(1);
    }
    Ok(f)
}

fn parser_err(e: wasmparser::BinaryReaderError) -> CompilerError {
    CompilerError::Validation(e.to_string())
}

/// Assemble the final module in canonical wasm section order, substituting the
/// rebuilt Type / Function / Code sections and appending the metadata marker.
fn assemble(
    module_bytes: &[u8],
    code: CodeSection,
    bogus_type_index: u32,
) -> Result<Vec<u8>> {
    let mut module = wasm_encoder::Module::new();
    let mut reencoder = RoundtripReencoder;
    let mut code = Some(code);

    for payload in Parser::new(0).parse_all(module_bytes) {
        let payload = payload.map_err(|e| CompilerError::Validation(e.to_string()))?;
        match payload {
            Payload::TypeSection(reader) => {
                let mut types = TypeSection::new();
                reencoder
                    .parse_type_section(&mut types, reader)
                    .map_err(reencode_err)?;
                types
                    .ty()
                    .function(Vec::<ValType>::new(), Vec::<ValType>::new());
                module.section(&types);
            }
            Payload::ImportSection(r) => {
                module.section(&RawSection {
                    id: 2,
                    data: &module_bytes[r.range()],
                });
            }
            Payload::FunctionSection(reader) => {
                let mut funcs = FunctionSection::new();
                for ff in reader {
                    funcs.function(ff.map_err(|e| CompilerError::Validation(e.to_string()))?);
                }
                for _ in 0..BOGUS_FUNCTION_COUNT {
                    funcs.function(bogus_type_index);
                }
                module.section(&funcs);
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
            // Emit the rebuilt Code section in the original Code position.
            Payload::CodeSectionStart { .. } => {
                if let Some(c) = code.take() {
                    module.section(&c);
                }
            }
            Payload::CodeSectionEntry(_) => {}
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

    Ok(module.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmparser::{Validator, WasmFeatures};

    fn template() -> Vec<u8> {
        crate::template::baked_template_bytes().to_vec()
    }

    /// Count total `nop` (0x01) opcodes across all function bodies in the module.
    fn count_nops(module_bytes: &[u8]) -> usize {
        let mut count = 0usize;
        for payload in Parser::new(0).parse_all(module_bytes) {
            if let Payload::CodeSectionEntry(body) = payload.unwrap() {
                let mut reader = body.get_operators_reader().unwrap();
                while !reader.eof() {
                    if matches!(reader.read().unwrap(), wasmparser::Operator::Nop) {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Count the number of function bodies (code-section entries) in the module.
    fn count_function_bodies(module_bytes: &[u8]) -> usize {
        let mut count = 0usize;
        for payload in Parser::new(0).parse_all(module_bytes) {
            if let Payload::CodeSectionEntry(_) = payload.unwrap() {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn obfuscated_module_is_valid_wasm() {
        let m = template();
        let o = obfuscate(&m).expect("obfuscate ok");
        let mut v = Validator::new_with_features(WasmFeatures::default());
        v.validate_all(&o).expect("valid");
    }

    #[test]
    fn obfuscation_inserts_real_nops_into_code_section() {
        // §17.1 control-flow nops: the obfuscated module MUST contain strictly
        // more `nop` opcodes in its function bodies than the input. This guards
        // against a future no-op regression where obfuscate() only appends a
        // custom-section marker.
        let m = template();
        let before = count_nops(&m);
        let o = obfuscate(&m).expect("ok");
        let after = count_nops(&o);
        assert!(
            after > before,
            "obfuscation must insert real nops (before={before}, after={after})"
        );
    }

    #[test]
    fn obfuscation_appends_bogus_dead_functions() {
        // §17.1 bogus code: the obfuscated module MUST carry strictly more
        // function bodies than the input (unreferenced, never-called dead code).
        let m = template();
        let before = count_function_bodies(&m);
        let o = obfuscate(&m).expect("ok");
        let after = count_function_bodies(&o);
        assert!(
            after > before,
            "obfuscation must append bogus dead functions (before={before}, after={after})"
        );
    }

    #[test]
    fn obfuscation_changes_code_section_structurally_not_just_marker() {
        // The transformation must alter the CODE section bytes specifically —
        // not merely append a trailing custom-section marker. We compare the
        // raw code-section payload bytes of input vs output.
        fn code_section_bytes(module_bytes: &[u8]) -> Vec<u8> {
            for payload in Parser::new(0).parse_all(module_bytes) {
                if let Payload::CodeSectionStart { range, .. } = payload.unwrap() {
                    return module_bytes[range].to_vec();
                }
            }
            Vec::new()
        }
        let m = template();
        let o = obfuscate(&m).expect("ok");
        assert_ne!(
            code_section_bytes(&m),
            code_section_bytes(&o),
            "obfuscation must transform the code section, not just append a marker"
        );
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
