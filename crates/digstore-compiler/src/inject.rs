use wasm_encoder::{ConstExpr, DataSection, MemorySection, MemoryType, RawSection};
use wasmparser::{Parser, Payload};

use crate::error::{CompilerError, Result};

const WASM_PAGE: u64 = 65536;

/// Inject `blob` as a single active data segment at `mem_offset` in memory 0,
/// copying every other section of `template` through verbatim. The original
/// `Data`/`DataCount` sections are dropped and replaced. The `Memory` section is
/// re-emitted with its min pages bumped (if necessary) so the segment is in
/// bounds at instantiation.
pub fn inject_data_section(template: &[u8], blob: &[u8], mem_offset: u32) -> Result<Vec<u8>> {
    // Required min pages so that mem_offset + blob.len() fits.
    let needed_bytes = mem_offset as u64 + blob.len() as u64;
    let needed_pages = needed_bytes.div_ceil(WASM_PAGE);

    let mut module = wasm_encoder::Module::new();

    for payload in Parser::new(0).parse_all(template) {
        let payload = payload.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
        match payload {
            // Drop and re-emit later.
            Payload::DataSection(_) | Payload::DataCountSection { .. } => {}

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
                module.section(&RawSection { id: 10, data: &template[range] });
            }
            // Per-function bodies are part of the code-section range above; skip
            // them explicitly so they are NOT dropped into the catch-all.
            Payload::CodeSectionEntry(_) => {}

            // Every other known section: copy its payload bytes verbatim.
            Payload::TypeSection(r) => { module.section(&RawSection { id: 1, data: &template[r.range()] }); }
            Payload::ImportSection(r) => { module.section(&RawSection { id: 2, data: &template[r.range()] }); }
            Payload::FunctionSection(r) => { module.section(&RawSection { id: 3, data: &template[r.range()] }); }
            Payload::TableSection(r) => { module.section(&RawSection { id: 4, data: &template[r.range()] }); }
            Payload::GlobalSection(r) => { module.section(&RawSection { id: 6, data: &template[r.range()] }); }
            Payload::ExportSection(r) => { module.section(&RawSection { id: 7, data: &template[r.range()] }); }
            Payload::StartSection { range, .. } => { module.section(&RawSection { id: 8, data: &template[range] }); }
            Payload::ElementSection(r) => { module.section(&RawSection { id: 9, data: &template[r.range()] }); }
            Payload::CustomSection(r) => { module.section(&RawSection { id: 0, data: &template[r.range()] }); }
            _ => {}
        }
    }

    // Append the new data section last.
    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(mem_offset as i32), blob.iter().copied());
    module.section(&data);

    let bytes = module.finish();
    // Sanity: ensure parseable; full validation happens in the pipeline stage.
    Parser::new(0)
        .parse_all(&bytes)
        .try_for_each(|p| p.map(|_| ()))
        .map_err(|e| CompilerError::Validation(e.to_string()))?;
    Ok(bytes)
}
