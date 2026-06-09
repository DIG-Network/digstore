use std::path::PathBuf;

/// The dig-compiler version (paper §5: "Compiler version 1.0.0; module format
/// version 1."). Tied to the crate version via `CARGO_PKG_VERSION` so the
/// artifact carries exactly the spec-stated version string; the crate is pinned
/// to `1.0.0` in `Cargo.toml`. The module-format-version half of the §5 claim is
/// the DIGS blob header byte (== 1, owned by `digstore_core::datasection`).
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Compiler options (paper §19.1: obfuscation + optimization toggles).
#[derive(Debug, Clone)]
pub struct CompilerConfig {
    /// Directory the final `{store_id}-{roothash}.wasm` is written to.
    pub output_dir: PathBuf,
    /// Apply deterministic obfuscation passes (§17.1).
    pub obfuscate: bool,
    /// Run wasm-opt after injection (§5.3 stage 8). Off by default: wasm-opt
    /// output is not guaranteed byte-stable across versions, which would break
    /// the §19.3 determinism guarantee.
    pub optimize: bool,
    /// Optional override of the prebuilt guest template bytes; when `None`, the
    /// pinned baked-in template fixture is used.
    pub template_override: Option<Vec<u8>>,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("."),
            obfuscate: false,
            optimize: false,
            template_override: None,
        }
    }
}

/// Rich per-run compiler statistics (CONVENTIONS C6: this is the compiler's own
/// detail struct, deliberately NOT named `CompilationStats` to avoid colliding
/// with the canonical [`digstore_core::CompilationStats`] carried inside
/// [`digstore_core::CompilationResult`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompilerStats {
    pub generation_count: u32,
    pub unique_chunk_count: u32,
    pub resource_count: u32,
    pub pool_byte_len: u64,
    pub data_section_byte_len: u64,
    pub obfuscation_applied: bool,
    /// The dig-compiler version that produced the artifact (§5: "Compiler
    /// version 1.0.0"). Always [`COMPILER_VERSION`].
    pub compiler_version: String,
}
