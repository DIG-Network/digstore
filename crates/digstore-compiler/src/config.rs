use std::path::PathBuf;

/// The dig-compiler version (paper §5: "Compiler version 1.0.0; module format
/// version 1."). Tied to the crate version via `CARGO_PKG_VERSION` so the
/// artifact carries exactly the spec-stated version string; the crate is pinned
/// to `1.0.0` in `Cargo.toml`. The module-format-version half of the §5 claim is
/// the DIGS blob header byte (== 1, owned by `digstore_core::datasection`).
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Uniform data-blob budget (§8.3 size obfuscation): every module's injected
/// data blob is padded to EXACTLY this length so all production stores compile
/// to the same module size, revealing nothing about content size. 128 MiB —
/// covers a max-cap store (`digstore_core::MAX_STORE_BYTES` = 128 MB = 122.07
/// MiB) ciphertext + key table + merkle + header with ~6 MiB headroom.
pub const FIXED_BLOB_LEN: usize = 128 * 1024 * 1024;

/// Environment variable that overrides [`FIXED_BLOB_LEN`] for the
/// [`CompilerConfig::default`] uniform-blob budget. Production leaves it unset
/// (→ 128 MiB uniform module); tests/CI set it to a small value (e.g.
/// `1048576`) so they don't each emit a ~128 MiB module.
pub const UNIFORM_BLOB_LEN_ENV: &str = "DIGSTORE_UNIFORM_BLOB_LEN";

/// Compiler options (paper §19.1: obfuscation + optimization toggles).
#[derive(Debug, Clone)]
pub struct CompilerConfig {
    /// Directory the final `{store_id}-{roothash}.dig` is written to.
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
    /// Uniform data-blob budget (§8.3): the pipeline pads every module's
    /// injected data blob to EXACTLY this many bytes so all stores are the same
    /// module size. Defaults to [`FIXED_BLOB_LEN`] (128 MiB), or the parsed
    /// value of the [`UNIFORM_BLOB_LEN_ENV`] environment variable when set (so
    /// tests/CI can stay small/fast). A store whose blob already exceeds this
    /// budget is rejected — production must keep this ≥ the worst-case blob at
    /// `digstore_core::MAX_STORE_BYTES`.
    pub uniform_blob_len: usize,
}

/// Resolve the default uniform-blob budget: the [`UNIFORM_BLOB_LEN_ENV`] env
/// override if it parses, else [`FIXED_BLOB_LEN`].
pub fn default_uniform_blob_len() -> usize {
    match std::env::var(UNIFORM_BLOB_LEN_ENV) {
        Ok(v) => v.trim().parse::<usize>().unwrap_or(FIXED_BLOB_LEN),
        Err(_) => FIXED_BLOB_LEN,
    }
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("."),
            obfuscate: false,
            optimize: false,
            template_override: None,
            uniform_blob_len: default_uniform_blob_len(),
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
