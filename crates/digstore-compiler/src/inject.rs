use wasm_encoder::{ConstExpr, DataSection, MemorySection, MemoryType, RawSection};
use wasmparser::{DataKind, Operator, Parser, Payload};

use crate::error::{CompilerError, Result};

const WASM_PAGE: u64 = 65536;

/// Inject `blob` as an active data segment at `mem_offset` in memory 0, copying
/// every other section of `template` through verbatim. The template's OWN data
/// segments (its `.data`/`.rodata`) are PRESERVED — the blob segment is appended
/// LAST so it wins on any byte overlap, but the rest of the guest's static data
/// survives (a real module needs it to run). The `Memory` section is re-emitted
/// with its min pages bumped (if necessary) so the blob is in bounds at
/// instantiation. The original `DataCount` section is dropped and recomputed by
/// the re-emitted `DataSection`.
pub fn inject_data_section(template: &[u8], blob: &[u8], mem_offset: u32) -> Result<Vec<u8>> {
    // Required min pages so that mem_offset + blob.len() fits.
    let needed_bytes = mem_offset as u64 + blob.len() as u64;
    let needed_pages = needed_bytes.div_ceil(WASM_PAGE);

    // Collected re-encoder for the data section: existing segments first, then the
    // injected blob last.
    let mut data = DataSection::new();

    let mut module = wasm_encoder::Module::new();

    for payload in Parser::new(0).parse_all(template) {
        let payload = payload.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
        match payload {
            // Preserve the template's own data segments (re-emitted below, before
            // the injected blob). The DataCount section is recomputed by the new
            // DataSection, so drop the original.
            Payload::DataSection(reader) => {
                for seg in reader {
                    let seg = seg.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    match seg.kind {
                        DataKind::Passive => {
                            data.passive(seg.data.iter().copied());
                        }
                        DataKind::Active {
                            memory_index,
                            offset_expr,
                        } => {
                            let off = const_i32_offset(&offset_expr)?;
                            data.active(
                                memory_index,
                                &ConstExpr::i32_const(off),
                                seg.data.iter().copied(),
                            );
                        }
                    }
                }
            }
            Payload::DataCountSection { .. } => {}

            // Re-emit the memory section with a possibly-bumped min.
            Payload::MemorySection(reader) => {
                let mut mem = MemorySection::new();
                for m in reader {
                    let m = m.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    let min = m.initial.max(needed_pages);
                    let max = m.maximum;
                    if let Some(max_pages) = max {
                        if needed_pages > max_pages {
                            return Err(CompilerError::Validation(format!(
                                "data section needs {needed_pages} pages but memory max is {max_pages}"
                            )));
                        }
                    }
                    mem.memory(MemoryType {
                        minimum: min,
                        maximum: max,
                        memory64: m.memory64,
                        shared: m.shared,
                        page_size_log2: None,
                    });
                }
                module.section(&mem);
            }

            // Whole code section payload range (count + all bodies) copied verbatim.
            Payload::CodeSectionStart { range, .. } => {
                module.section(&RawSection {
                    id: 10,
                    data: &template[range],
                });
            }
            // Per-function bodies are part of the code-section range above; skip
            // them explicitly so they are NOT dropped into the catch-all.
            Payload::CodeSectionEntry(_) => {}

            // Every other known section: copy its payload bytes verbatim.
            Payload::TypeSection(r) => {
                module.section(&RawSection {
                    id: 1,
                    data: &template[r.range()],
                });
            }
            Payload::ImportSection(r) => {
                module.section(&RawSection {
                    id: 2,
                    data: &template[r.range()],
                });
            }
            Payload::FunctionSection(r) => {
                module.section(&RawSection {
                    id: 3,
                    data: &template[r.range()],
                });
            }
            Payload::TableSection(r) => {
                module.section(&RawSection {
                    id: 4,
                    data: &template[r.range()],
                });
            }
            Payload::GlobalSection(r) => {
                module.section(&RawSection {
                    id: 6,
                    data: &template[r.range()],
                });
            }
            Payload::ExportSection(r) => {
                module.section(&RawSection {
                    id: 7,
                    data: &template[r.range()],
                });
            }
            Payload::StartSection { range, .. } => {
                module.section(&RawSection {
                    id: 8,
                    data: &template[range],
                });
            }
            Payload::ElementSection(r) => {
                module.section(&RawSection {
                    id: 9,
                    data: &template[r.range()],
                });
            }
            Payload::CustomSection(r) => {
                module.section(&RawSection {
                    id: 0,
                    data: &template[r.range()],
                });
            }
            _ => {}
        }
    }

    // Append the injected blob LAST so it overwrites any overlapping bytes of an
    // earlier segment at instantiation (active segments apply in order).
    data.active(
        0,
        &ConstExpr::i32_const(mem_offset as i32),
        blob.iter().copied(),
    );
    module.section(&data);

    let bytes = module.finish();
    // Sanity: ensure parseable; full validation happens in the pipeline stage.
    Parser::new(0)
        .parse_all(&bytes)
        .try_for_each(|p| p.map(|_| ()))
        .map_err(|e| CompilerError::Validation(e.to_string()))?;
    Ok(bytes)
}

/// Read a `i32.const N` (the only offset form Rust/LLVM emits for active wasm32
/// data segments) from an offset const-expression.
fn const_i32_offset(offset_expr: &wasmparser::ConstExpr) -> Result<i32> {
    let mut ops = offset_expr.get_operators_reader();
    let op = ops
        .read()
        .map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
    match op {
        Operator::I32Const { value } => Ok(value),
        other => Err(CompilerError::InvalidTemplate(format!(
            "unsupported active data-segment offset expression: {other:?}"
        ))),
    }
}
