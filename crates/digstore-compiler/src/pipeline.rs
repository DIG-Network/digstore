use digstore_core::{
    Bytes32, Bytes48, CompilationResult, CompilationStats, MetadataManifest, TrustedHostKey,
};

use crate::atomic_write::atomic_write_module;
use crate::config::{CompilerConfig, CompilerStats};
use crate::data_section::{encode_data_section, DataSectionInputs};
use crate::error::{CompilerError, Result};
use crate::inject::inject_data_section;
use crate::key_table::{build_chunk_index_and_key_table, GenerationView};
use crate::obfuscate::obfuscate;
use crate::pool::build_pool;
use crate::template::{baked_template_bytes, load_template};

/// Fixed memory offset where the data-section blob is placed. This is page 1
/// (65536), the reserved region declared by the guest template. SINGLE SOURCE OF
/// TRUTH: the digstore-guest crate reads its data section from this same offset.
pub const DATA_SECTION_MEM_OFFSET: u32 = 65536;

/// Outcome of a successful compilation (CONVENTIONS C6).
///
/// `result` is the canonical [`digstore_core::CompilationResult`] (its `stats`
/// field is the canonical [`digstore_core::CompilationStats`]). `detail` is the
/// compiler's richer [`CompilerStats`] — a SEPARATE struct, deliberately NOT a
/// second `CompilationStats`.
#[derive(Debug, Clone)]
pub struct CompileOutcome {
    pub result: CompilationResult,
    pub detail: CompilerStats,
}

/// The dig-compiler entry point.
pub struct Compiler;

impl Compiler {
    /// Run the full deterministic pipeline. `generations` must be in load order;
    /// the last generation's root is the module's roothash / current generation.
    pub fn compile<G: GenerationView>(
        config: &CompilerConfig,
        store_id: Bytes32,
        store_pubkey: Bytes48,
        generations: &[G],
        manifest: MetadataManifest,
        trusted_keys: &[TrustedHostKey],
    ) -> Result<CompileOutcome> {
        // Stage 1: trusted-key precondition (§5.3, §19.2).
        if trusted_keys.is_empty() {
            return Err(CompilerError::NoTrustedKeys);
        }
        if generations.is_empty() {
            return Err(CompilerError::GenerationLoad("no generations".into()));
        }

        // Stages 3+4: dedup + key-table; then integrity check.
        let (chunk_index, key_table) = build_chunk_index_and_key_table(generations);
        key_table.verify_against(chunk_index.len() as u32)?;

        // Current generation = last loaded.
        let roothash = generations.last().unwrap().root();
        let root_history: Vec<Bytes32> = generations.iter().map(|g| g.root()).collect();

        // Stage: interleaved pool with deterministic filler (§8.3, §19.3).
        let bodies: Vec<Vec<u8>> = chunk_index.bodies_in_order().map(|b| b.to_vec()).collect();
        let total_content_bytes: u64 = bodies.iter().map(|b| b.len() as u64).sum();
        let pool = build_pool(&store_id, &roothash, &bodies);

        // Stage 6: data-section encode (manifest via core Encode, NOT JSON).
        let inputs = DataSectionInputs {
            store_id,
            roothash,
            root_history,
            store_pubkey,
            pool_bytes: pool.bytes.clone(),
            pool_descriptors: pool.descriptors.clone(),
            key_table: key_table.entries().to_vec(),
            manifest,
            trusted_keys: trusted_keys.to_vec(),
        };
        let data_blob = encode_data_section(&inputs);

        // Stage 5: load prebuilt template (or override) and validate (§5.1).
        let template_bytes = match &config.template_override {
            Some(b) => b.clone(),
            None => baked_template_bytes().to_vec(),
        };
        load_template(&template_bytes)?;

        // Stage: data inject (bumps memory min pages to fit the blob).
        let mut module = inject_data_section(&template_bytes, &data_blob, DATA_SECTION_MEM_OFFSET)?;

        // Stage 7: optional obfuscation (deterministic).
        let obfuscation_applied = config.obfuscate;
        if obfuscation_applied {
            module = obfuscate(&module)?;
        }

        // Stage 8 (wasm-opt) intentionally skipped for determinism portability.
        // Stage 9: final validate (re-parse exports + memory bounds).
        load_template(&module)?;

        // Stage 10: atomic write.
        let output_path = atomic_write_module(&config.output_dir, &store_id, &roothash, &module)?;
        let output_size = std::fs::metadata(&output_path)?.len();

        // Canonical core stats (C6): chunk_count, total_bytes, generation_count.
        let core_stats = CompilationStats {
            chunk_count: chunk_index.len() as u64,
            total_bytes: total_content_bytes,
            generation_count: generations.len() as u64,
        };

        // Rich compiler-specific detail (separate struct, NOT a second
        // `CompilationStats`).
        let detail = CompilerStats {
            generation_count: generations.len() as u32,
            unique_chunk_count: chunk_index.len() as u32,
            resource_count: key_table.entries().len() as u32,
            pool_byte_len: pool.bytes.len() as u64,
            data_section_byte_len: data_blob.len() as u64,
            obfuscation_applied,
        };

        let result = CompilationResult {
            store_id,
            roothash,
            output_path,
            output_size,
            stats: core_stats,
        };

        Ok(CompileOutcome { result, detail })
    }
}
