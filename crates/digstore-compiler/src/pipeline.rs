use digstore_core::datasection::DIGS_DATA_OFFSET;
use digstore_core::merkle::MerkleTree;
use digstore_core::serving::concat_output;
use digstore_core::{
    AuthenticationInfo, Bytes32, Bytes48, CompilationResult, CompilationStats, MetadataManifest,
    TrustedHostKey,
};
use sha2::{Digest, Sha256};

use crate::atomic_write::atomic_write_module;
use crate::config::{CompilerConfig, CompilerStats, COMPILER_VERSION};
use crate::data_section::{encode_data_section, DataSectionInputs};
use crate::error::{CompilerError, Result};
use crate::filler::deterministic_filler;
use crate::inject::inject_data_section;
use crate::key_table::{build_chunk_index_and_key_table, GenerationView};
use crate::obfuscate::obfuscate;
use crate::template::{
    assert_host_imports, assert_memory_ceiling, baked_template_bytes, load_template,
};

/// Fixed linear-memory offset where the data-section blob is injected and the
/// guest reads it (BINDING contract D2). SINGLE SOURCE OF TRUTH:
/// [`digstore_core::datasection::DIGS_DATA_OFFSET`] (2 MiB). The compiler injects
/// an ACTIVE data segment at `i32.const DATA_SECTION_MEM_OFFSET` and the guest's
/// `embedded()` reads from this same pointer.
pub const DATA_SECTION_MEM_OFFSET: u32 = DIGS_DATA_OFFSET;

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
    /// the last generation is the current generation whose per-resource merkle
    /// root becomes the module's `CurrentRoot` (D5).
    // The pipeline genuinely needs each of these distinct inputs (identity, keys,
    // generations, metadata, auth policy, trusted set, optional chain anchor);
    // bundling them into a struct would only move the argument list elsewhere.
    #[allow(clippy::too_many_arguments)]
    pub fn compile<G: GenerationView>(
        config: &CompilerConfig,
        store_id: Bytes32,
        store_pubkey: Bytes48,
        generations: &[G],
        manifest: MetadataManifest,
        auth_info: AuthenticationInfo,
        trusted_keys: &[TrustedHostKey],
        chain_state: Option<digstore_core::datasection::ChainState>,
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

        // Generation roots (store-reported) drive the filename + root history.
        let store_roothash = generations.last().unwrap().root();
        let root_history: Vec<Bytes32> = generations.iter().map(|g| g.root()).collect();

        // D5: per-resource merkle leaves of the CURRENT generation. leaf =
        // SHA-256(concat_output(resource's ordered chunk ciphertexts)); leaves are
        // ordered ascending by static_key (the order the guest ranks against). The
        // current generation's root = MerkleTree::from_leaves(leaves).root().
        let merkle_leaves = current_generation_leaves(generations.last().unwrap());
        let current_root = MerkleTree::from_leaves(merkle_leaves.clone()).root();

        // D4: ChunkPool body holds the unique chunk ciphertexts in GLOBAL INDEX
        // order (the order `chunk_indices` address into). Filler is a SEPARATE
        // section (id 11), not interleaved, so global indexing stays exact.
        let chunk_pool_bodies: Vec<Vec<u8>> =
            chunk_index.bodies_in_order().map(|b| b.to_vec()).collect();
        let total_content_bytes: u64 = chunk_pool_bodies.iter().map(|b| b.len() as u64).sum();

        // §8.3 filler (deviation #2): deterministic ChaCha20 keyed by
        // SHA-256(store_id || roothash || domain). UNIFORM-SIZE model: every
        // module's blob is padded to EXACTLY `config.uniform_blob_len` so all
        // stores compile to the same module size, revealing nothing about
        // content size. The filler (id 11, unreferenced) is the ONLY variable
        // section; it does not touch resource leaves or `current_root`, so the
        // filler length never changes what the module serves or proves.
        //
        // `blob_len_without_filler` = the encoded blob length with an EMPTY
        // filler (header + offset table + every other section). Because the
        // Filler section is always present (an empty body here) and adding N
        // filler bytes grows the blob by exactly N, the final blob length is
        // `blob_len_without_filler + filler_len`. Hitting the budget exactly is
        // therefore `filler_len = budget - blob_len_without_filler`.
        let mut inputs = DataSectionInputs {
            store_id,
            current_root,
            root_history,
            store_pubkey,
            trusted_keys: trusted_keys.to_vec(),
            manifest,
            // §4.1/§5.2: per-store auth policy (JWT/session) is supplied by the
            // caller and compiled into the module verbatim — NOT hardcoded here.
            auth_info,
            key_table: key_table.entries().to_vec(),
            chunk_pool_bodies,
            merkle_leaves,
            filler: Vec::new(),
            // Optional on-chain anchor pointer (id 12), embedded verbatim. It is
            // a fixed-size addition independent of the filler budget math below.
            chain_state,
        };
        let blob_len_without_filler = encode_data_section(&inputs).len();
        if blob_len_without_filler > config.uniform_blob_len {
            // Cannot happen under `digstore_core::MAX_STORE_BYTES` with the
            // production `FIXED_BLOB_LEN` budget; never truncate to fit.
            return Err(CompilerError::Validation(
                "content exceeds the uniform-size budget".into(),
            ));
        }
        let filler_len = config
            .uniform_blob_len
            .saturating_sub(blob_len_without_filler);
        inputs.filler = deterministic_filler(&store_id, &store_roothash, filler_len);

        // Stage 6: data-section encode in the canonical contract format (D1).
        let data_blob = encode_data_section(&inputs);

        // Stage 5: load prebuilt template (or override) and validate (§5.1).
        let template_bytes = match &config.template_override {
            Some(b) => b.clone(),
            None => baked_template_bytes().to_vec(),
        };
        load_template(&template_bytes)?;

        // Stage: inject blob as an ACTIVE data segment at DIGS_DATA_OFFSET and bump
        // memory min pages so DIGS_DATA_OFFSET + total_len fits (D2).
        let mut module = inject_data_section(&template_bytes, &data_blob, DATA_SECTION_MEM_OFFSET)?;

        // Stage 7: optional obfuscation (deterministic).
        let obfuscation_applied = config.obfuscate;
        if obfuscation_applied {
            module = obfuscate(&module)?;
        }

        // Stage 8 (wasm-opt) intentionally skipped for determinism portability.
        // Stage 9: final validate (re-parse exports + memory bounds), then assert
        // the §5.1 module-declared memory ceiling on the EMITTED module: it MUST
        // declare `maximum: Some(6144)` (384 MiB). Injection normalizes the raw
        // guest template (which may declare no max) to this exact cap.
        load_template(&module)?;
        assert_memory_ceiling(&module)?;
        // §5.1 Import section: the emitted module MUST import all eight dig_host
        // host functions (§6.3) — guards against a template regressing to an
        // export-only stub.
        assert_host_imports(&module)?;

        // Stage 10: atomic write (filename uses the store-reported roothash).
        let output_path =
            atomic_write_module(&config.output_dir, &store_id, &store_roothash, &module)?;
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
            pool_byte_len: total_content_bytes,
            data_section_byte_len: data_blob.len() as u64,
            obfuscation_applied,
            // §5: the artifact carries the compiler version that produced it.
            compiler_version: COMPILER_VERSION.to_string(),
        };

        let result = CompilationResult {
            store_id,
            roothash: store_roothash,
            output_path,
            output_size,
            stats: core_stats,
        };

        Ok(CompileOutcome { result, detail })
    }
}

/// Compute the current generation's per-resource merkle leaves (D5), ascending by
/// `static_key`. leaf = SHA-256(concat_output(resource's ordered chunk ciphertexts)),
/// i.e. SHA-256 of exactly the bytes `get_content` returns for that resource.
fn current_generation_leaves<G: GenerationView>(current: &G) -> Vec<Bytes32> {
    let mut keyed: Vec<([u8; 32], Bytes32)> = current
        .resources()
        .iter()
        .map(|r| {
            let bodies: Vec<Vec<u8>> = r.chunks().into_iter().map(|(_, body)| body).collect();
            let slices: Vec<&[u8]> = bodies.iter().map(|b| b.as_slice()).collect();
            let blob = concat_output(&slices);
            let mut h = Sha256::new();
            h.update(&blob);
            let mut leaf = [0u8; 32];
            leaf.copy_from_slice(&h.finalize());
            (r.resource_key().0, Bytes32(leaf))
        })
        .collect();

    // Ascending by static_key (raw 32 bytes; Bytes32 has no Ord). This is the
    // exact order the guest's `resource_leaf_index` ranks against (D3/D5).
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.into_iter().map(|(_, leaf)| leaf).collect()
}
