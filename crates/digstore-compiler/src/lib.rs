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
//! ## Shared constant
//! [`DATA_SECTION_MEM_OFFSET`] (65536 = page 1) is the agreed offset where the
//! data section is placed; the `digstore-guest` crate reads from the same offset.

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
mod pool;
mod template;

pub use config::{CompilerConfig, CompilerStats};
pub use error::{CompilerError, Result};
pub use chunk_index::ChunkIndex;

// Re-export the canonical core result/stats so consumers reference one home (C6).
pub use digstore_core::{CompilationResult, CompilationStats};
