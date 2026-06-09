//! `dig-compiler`: deterministic transform from on-disk generations to a single
//! self-serving WASM module (paper §5, §8.3, §17.1, §19).
//!
//! ## Documented deviations
//! - **Endianness (deviation #1):** the data-section codec is BIG-ENDIAN (Chia
//!   streamable framing via `digstore_core::Encode`), not the paper's
//!   "little-endian" note (§5.3). Chia compatibility wins.
//! - **Filler (deviation #2):** §8.3 "random filler" is a DETERMINISTIC ChaCha20
//!   keystream seeded by `SHA-256(store_id || roothash || b"digstore-filler-v1")`,
//!   so compilation is byte-identical (§19.3).
//! - **Obfuscation (§17.1):** optional, WASM-level, deterministic, and
//!   behavior-preserving; security never rests on it.
//! - **Template (§19.3):** the guest template is a single PINNED committed input,
//!   assembled by `build.rs` from `fixtures/digstore_guest_template.wat`; the
//!   build script never invokes `cargo build` for the guest, so the template
//!   bytes (and thus the final module) are byte-identical across environments.
//! - **Stats (CONVENTIONS C6):** the canonical [`digstore_core::CompilationStats`]
//!   is reused for [`digstore_core::CompilationResult::stats`]; the compiler's
//!   richer detail is a SEPARATE struct [`CompilerStats`] (NOT a second
//!   `CompilationStats`). [`Compiler::compile`] returns a [`CompileOutcome`]
//!   carrying both.
//!
//! ## Data-section contract (BINDING D1–D5)
//! The byte-exact data-section format is owned by `digstore_core::datasection`.
//! The compiler emits the blob via that module and injects it as an ACTIVE data
//! segment at [`DATA_SECTION_MEM_OFFSET`] (= `digstore_core::datasection::DIGS_DATA_OFFSET`,
//! 1 MiB), raising the module's memory min pages to fit. The `digstore-guest`
//! crate reads from the same offset. The compiler's old private `SEG_*` format is
//! deleted: core is the single source of truth.

mod atomic_write;
mod chunk_index;
mod config;
mod data_section;
mod error;
mod filler;
mod inject;
mod key_table;
mod obfuscate;
mod pipeline;
mod template;

pub use atomic_write::{atomic_write_module, output_filename};
pub use chunk_index::ChunkIndex;
pub use config::{CompilerConfig, CompilerStats, COMPILER_VERSION};
pub use data_section::{encode_data_section, rekey_module_trusted, DataSectionInputs};
pub use error::{CompilerError, Result};
pub use filler::deterministic_filler;
pub use inject::{extract_data_section, inject_data_section};
pub use key_table::{build_chunk_index_and_key_table, GenerationView, KeyTable, ResourceView};
pub use obfuscate::obfuscate;
pub use pipeline::{CompileOutcome, Compiler, DATA_SECTION_MEM_OFFSET};
pub use template::{
    assert_memory_ceiling, baked_template_bytes, load_template, Template, MAX_MEMORY_PAGES,
    REQUIRED_EXPORTS,
};

// Re-export the canonical core result/stats so consumers reference one home (C6).
pub use digstore_core::{CompilationResult, CompilationStats};
