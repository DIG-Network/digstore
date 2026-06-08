# digstore-compiler Implementation Plan

> **For agentic workers:** Execute this plan with the **REQUIRED SUB-SKILL `superpowers:subagent-driven-development`**. Each task is a bite-sized TDD cycle: write the failing test (full code shown) → run it and observe the exact FAIL → write the minimal implementation (full code shown) → run it and observe PASS → commit with the exact conventional-commit message. Do not skip the red step. Do not batch multiple unrelated behaviors into one commit. Always run cargo with the absolute `-p digstore-compiler` form from the workspace root `C:/Users/micha/workspace/dig_network/digstore_wasm`.

**Goal:** Implement `dig-compiler`, the deterministic transform that turns a store's on-disk generations into a single self-serving `{hex(store_id)}-{hex(roothash)}.wasm` module by injecting a Chia-streamable-encoded data section (interleaved chunk pool + deterministic ChaCha20 filler, key table, store header, plaintext manifest, trusted host keys) into the prebuilt `digstore-guest` template and optionally obfuscating its code.

**Architecture:** A 10-stage pipeline (paper §5.3): config/generation load → global chunk dedup (`ChunkIndex`) → key-table build → template load → data-section encode (segments via `digstore-core`'s canonical `Encode` codec, local outer framing) → WASM data-section injection via `wasm-encoder`/`wasmparser` with whole-section raw passthrough → optional deterministic obfuscation passes (§17.1) → optional `wasm-opt` (disabled for determinism portability) → re-parse/validate including memory-bound checks (§5.1) → atomic temp-file-then-rename write. Determinism is enforced end-to-end: the filler is a ChaCha20 stream keyed by `SHA-256(store_id ‖ roothash ‖ b"digstore-filler-v1")`, the prebuilt template is a single pinned committed fixture (never built inside `build.rs`), and a double-compile test asserts byte-identical output (§19.3).

**Tech Stack:** Rust (host-side `std`), `digstore-core` (canonical types + custom big-endian `Encode`/`Decode` codec), `digstore-store` (loaded generations, via a `GenerationView` adapter), a committed prebuilt `digstore-guest` template wasm fixture, `wasm-encoder` (0.221), `wasmparser` (0.221), `chacha20`, `sha2`, `wasmtime` (dev-only, behavior-equivalence harness), `wat` (dev-only + build-dep, assembles the committed `.wat` source of the template fixture), `hex`, `thiserror`.

---

## File Structure

All paths under `crates/digstore-compiler/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest: deps `digstore-core`, `digstore-store`, `wasm-encoder`, `wasmparser`, `chacha20`, `sha2`, `hex`, `thiserror`; build-dep `wat`; dev-deps `wasmtime`, `wat`. |
| `build.rs` | Build script that assembles the committed `fixtures/digstore_guest_template.wat` into `OUT_DIR/digstore_guest_template.wasm`. NEVER invokes `cargo build` for the guest (determinism + no recursion). |
| `fixtures/digstore_guest_template.wat` | The single, pinned, committed guest template source (declares full export ABI, a 4-page memory, a reserved data region). The deterministic input to compilation. |
| `src/lib.rs` | Crate root: module wiring, crate-level deviation docs, public re-exports. |
| `src/error.rs` | `CompilerError` enum (incl. `NoTrustedKeys`, `MissingChunk`) + `Result` alias. |
| `src/config.rs` | `CompilerConfig` + `CompilationStats`. |
| `src/chunk_index.rs` | `ChunkIndex`: global hash→u32 dedup across generations, stable insertion ordering. |
| `src/key_table.rs` | `GenerationView`/`ResourceView` traits + `KeyTable` build from generations, ordered lookup, chunk-index integrity check. |
| `src/filler.rs` | Deterministic ChaCha20 filler stream keyed by the documented seed (deviation #2). |
| `src/pool.rs` | Interleaved chunk pool assembly with bucketed length + deterministic filler in gaps (§8.3). |
| `src/data_section.rs` | Encode/parse the full data-section blob: `DIGS` magic, format version, section offset table, then segments built via `digstore-core::Encode`. |
| `src/template.rs` | Load + validate the pinned template wasm; assert required exports AND memory bounds (§5.1). |
| `src/inject.rs` | Whole-section raw passthrough + replacement of the `Data` section via `wasm-encoder`/`wasmparser`; bumps memory min-pages to fit the blob. |
| `src/obfuscate.rs` | Deterministic, behavior-preserving WASM obfuscation pass (§17.1). |
| `src/pipeline.rs` | `Compiler::compile`: orchestrate all 10 stages, produce `CompilationResult`, atomic write. |
| `src/atomic_write.rs` | Temp-file-then-rename atomic write with exact `{hex(store_id)}-{hex(roothash)}.wasm` filename. |
| `tests/common/mod.rs` | Test fixtures: `FakeGeneration`/`ResourceSpec` implementing `GenerationView`, trusted keys, manifest. |
| `tests/chunk_index.rs` | Dedup behavior across generations. |
| `tests/key_table.rs` | Key-table build, ordered lookup, integrity error. |
| `tests/filler.rs` | Filler determinism + seed binding. |
| `tests/data_section_golden.rs` | Structural assertions + cross-crate `Decode` round-trip + pinned golden vector. |
| `tests/inject.rs` | Whole-section byte-identity, validity, exports survive, blob present, memory bumped. |
| `tests/determinism.rs` | §19.3 double-compile byte-identical (plain + obfuscated). |
| `tests/obfuscation.rs` | Obfuscated module valid + behavior-equivalent in a wasmtime harness. |
| `tests/pipeline.rs` | End-to-end: `NoTrustedKeys`, exact filename, `CompilationResult` fields, bucketed pool length. |
| `tests/fixtures/golden_data_section.hex` | Committed golden data-section bytes (independently structurally validated). |

---

## Task 1 — Crate scaffold, error type, config

**Files:**
- Create: `crates/digstore-compiler/Cargo.toml`
- Create: `crates/digstore-compiler/src/lib.rs`
- Create: `crates/digstore-compiler/src/error.rs`
- Create: `crates/digstore-compiler/src/config.rs`
- Modify: workspace root `Cargo.toml`
- Test: `crates/digstore-compiler/src/error.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] **Create the crate manifest FIRST** so the workspace member never dangles. Create `crates/digstore-compiler/Cargo.toml`:
```toml
[package]
name = "digstore-compiler"
version = "0.1.0"
edition = "2021"

[dependencies]
digstore-core = { path = "../digstore-core" }
digstore-store = { path = "../digstore-store" }
wasm-encoder = "=0.221.0"
wasmparser = "=0.221.0"
chacha20 = "0.9"
sha2 = "0.10"
hex = "0.4"
thiserror = "1"

[build-dependencies]
wat = "1"

[dev-dependencies]
wasmtime = "27"
wat = "1"
```

- [ ] **Create `crates/digstore-compiler/src/lib.rs`** with module wiring and stub modules inline so the crate builds before the workspace edit. Create the file with this content (later tasks replace stub bodies):
```rust
//! `dig-compiler`: deterministic transform from on-disk generations to a single
//! self-serving WASM module (paper §5, §8.3, §17.1, §19).

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

pub use config::{CompilationStats, CompilerConfig};
pub use error::{CompilerError, Result};
```

- [ ] **Create empty stub modules** so the crate compiles. Create each of these files containing only the single line `// stub`:
  `src/atomic_write.rs`, `src/chunk_index.rs`, `src/data_section.rs`, `src/filler.rs`, `src/inject.rs`, `src/key_table.rs`, `src/obfuscate.rs`, `src/pipeline.rs`, `src/pool.rs`, `src/template.rs`.

- [ ] **Add the crate to the workspace AND create the manifest in the same step** (manifest already created above, so the member is never dangling). Edit the workspace root `Cargo.toml`: if a `[workspace]` table with `members` exists, append `"crates/digstore-compiler"`; otherwise create:
```toml
[workspace]
resolver = "2"
members = ["crates/digstore-compiler"]
```

- [ ] **Write the failing test** for `CompilerError` in `src/error.rs` (replace the stub):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trusted_keys_renders_documented_message() {
        let e = CompilerError::NoTrustedKeys;
        assert_eq!(
            e.to_string(),
            "compilation requires at least one trusted host key"
        );
    }
}
```

- [ ] **Run it (expect FAIL — type does not exist):**
```
cargo test -p digstore-compiler error::tests::no_trusted_keys_renders_documented_message
```
Expected: `error[E0433]: failed to resolve` / `cannot find type CompilerError in this scope`.

- [ ] **Implement `src/error.rs`** (prepend the enum above the test module):
```rust
use thiserror::Error;

/// Errors produced by the dig-compiler pipeline.
#[derive(Debug, Error)]
pub enum CompilerError {
    /// At least one trusted host key is required (paper §5.3, §19.2).
    #[error("compilation requires at least one trusted host key")]
    NoTrustedKeys,
    /// The prebuilt guest template was malformed or missing a required export
    /// or violated memory bounds (§5.1).
    #[error("invalid guest template: {0}")]
    InvalidTemplate(String),
    /// A generation directory could not be loaded.
    #[error("generation load failed: {0}")]
    GenerationLoad(String),
    /// The WASM module failed re-validation after data injection / obfuscation.
    #[error("emitted module failed validation: {0}")]
    Validation(String),
    /// A key-table entry referenced a chunk index outside the chunk index.
    #[error("key table references missing chunk index {0}")]
    MissingChunk(u32),
    /// I/O failure during atomic write or template load.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, CompilerError>;
```

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler error::tests::no_trusted_keys_renders_documented_message
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Add the second error behavior — TDD red first.** Append to the `tests` module in `src/error.rs`:
```rust
    #[test]
    fn invalid_template_carries_reason() {
        let e = CompilerError::InvalidTemplate("missing export get_content".into());
        assert!(e.to_string().contains("missing export get_content"));
    }
```

- [ ] **Run it (expect PASS — variant already exists):**
```
cargo test -p digstore-compiler error::tests::invalid_template_carries_reason
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Implement `src/config.rs`** (replace stub):
```rust
use std::path::PathBuf;

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

/// Statistics reported on the `CompilationResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompilationStats {
    pub generation_count: u32,
    pub unique_chunk_count: u32,
    pub resource_count: u32,
    pub pool_byte_len: u64,
    pub data_section_byte_len: u64,
    pub obfuscation_applied: bool,
}
```

- [ ] **Run the whole error+config build (expect PASS):**
```
cargo test -p digstore-compiler error::tests
```
Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/Cargo.toml crates/digstore-compiler/src Cargo.toml
git commit -m "feat(compiler): scaffold crate with CompilerError and CompilerConfig"
```

---

## Task 2 — `ChunkIndex`: global hash→u32 dedup across generations

**Files:**
- Modify: `crates/digstore-compiler/src/chunk_index.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: `crates/digstore-compiler/tests/chunk_index.rs`

> Each behavior gets its own red→green→commit cycle.

Steps:

- [ ] **Write the first failing test** `crates/digstore-compiler/tests/chunk_index.rs`:
```rust
use digstore_compiler::ChunkIndex;
use digstore_core::Bytes32;

fn h(b: u8) -> Bytes32 {
    Bytes32([b; 32])
}

#[test]
fn inserts_assign_sequential_indices() {
    let mut idx = ChunkIndex::new();
    assert_eq!(idx.insert(h(1), vec![0xAA]), 0);
    assert_eq!(idx.insert(h(2), vec![0xBB]), 1);
    assert_eq!(idx.len(), 2);
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler --test chunk_index inserts_assign_sequential_indices
```
Expected: `error[E0432]: unresolved import digstore_compiler::ChunkIndex`.

- [ ] **Implement `src/chunk_index.rs`:**
```rust
use std::collections::HashMap;

use digstore_core::Bytes32;

/// Deduplicated global chunk index: maps a chunk's SHA-256 content address to a
/// stable global `u32` index, deduplicating identical chunks across all
/// generations (paper §5.2, §8.3). Insertion order is preserved so the resulting
/// pool layout is deterministic.
#[derive(Debug, Default)]
pub struct ChunkIndex {
    map: HashMap<Bytes32, u32>,
    bodies: Vec<Vec<u8>>,
}

impl ChunkIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a chunk by hash. Returns the existing index if the hash is already
    /// known (dedup), otherwise assigns and returns the next sequential index.
    pub fn insert(&mut self, hash: Bytes32, body: Vec<u8>) -> u32 {
        if let Some(&i) = self.map.get(&hash) {
            return i;
        }
        let i = self.bodies.len() as u32;
        self.map.insert(hash, i);
        self.bodies.push(body);
        i
    }

    /// Look up the global index for a hash, if present.
    pub fn index_of(&self, hash: &Bytes32) -> Option<u32> {
        self.map.get(hash).copied()
    }

    /// Number of unique chunks.
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// Chunk bodies in stable insertion (global-index) order.
    pub fn bodies_in_order(&self) -> impl Iterator<Item = &[u8]> {
        self.bodies.iter().map(|b| b.as_slice())
    }
}
```

- [ ] **Re-export in `src/lib.rs`:** add after the `error` re-export line: `pub use chunk_index::ChunkIndex;`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler --test chunk_index inserts_assign_sequential_indices
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/chunk_index.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/tests/chunk_index.rs
git commit -m "feat(compiler): ChunkIndex sequential index assignment"
```

- [ ] **Add the dedup behavior — red first.** Append to `tests/chunk_index.rs`:
```rust
#[test]
fn duplicate_hash_returns_existing_index_and_does_not_grow() {
    let mut idx = ChunkIndex::new();
    let first = idx.insert(h(7), vec![0x01, 0x02]);
    let again = idx.insert(h(7), vec![0x01, 0x02]);
    assert_eq!(first, again);
    assert_eq!(idx.len(), 1);
}
```

- [ ] **Run it (expect PASS — dedup already implemented):**
```
cargo test -p digstore-compiler --test chunk_index duplicate_hash_returns_existing_index_and_does_not_grow
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Add ordering + lookup behaviors — red first.** Append to `tests/chunk_index.rs`:
```rust
#[test]
fn bodies_returned_in_insertion_order() {
    let mut idx = ChunkIndex::new();
    idx.insert(h(3), vec![0x30]);
    idx.insert(h(1), vec![0x10]);
    idx.insert(h(2), vec![0x20]);
    let bodies: Vec<Vec<u8>> = idx.bodies_in_order().map(|b| b.to_vec()).collect();
    assert_eq!(bodies, vec![vec![0x30], vec![0x10], vec![0x20]]);
}

#[test]
fn index_of_resolves_known_hash() {
    let mut idx = ChunkIndex::new();
    idx.insert(h(5), vec![0x55]);
    idx.insert(h(6), vec![0x66]);
    assert_eq!(idx.index_of(&h(6)), Some(1));
    assert_eq!(idx.index_of(&h(9)), None);
}
```

- [ ] **Run the full file (expect PASS):**
```
cargo test -p digstore-compiler --test chunk_index
```
Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/chunk_index.rs
git commit -m "test(compiler): ChunkIndex dedup, ordering, and lookup"
```

---

## Task 3 — Key-table traits + test fixtures

**Files:**
- Modify: `crates/digstore-compiler/src/key_table.rs` (replace stub — traits only this task)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Create: `crates/digstore-compiler/tests/common/mod.rs`

> We define the `GenerationView`/`ResourceView` traits FIRST (Task 3) so the fixtures in `tests/common/mod.rs` can implement them immediately, eliminating the Task-3/Task-4 ordering defect. We add a real green signal via a trivial fixtures-build test.

Steps:

- [ ] **Implement the view traits in `src/key_table.rs`** (replace stub; `KeyTable` struct + builder come in Task 4):
```rust
use digstore_core::Bytes32;

/// A read-only view of one resource within a generation, so the compiler can
/// consume both `digstore_store::LoadedGeneration` and test fixtures.
pub trait ResourceView {
    fn resource_key(&self) -> Bytes32;
    /// (chunk_hash, chunk_body) pairs in resource order.
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)>;
}

/// A read-only view of one loaded generation.
pub trait GenerationView {
    fn root(&self) -> Bytes32;
    fn resources(&self) -> Vec<Box<dyn ResourceView + '_>>;
}
```

- [ ] **Re-export in `src/lib.rs`:** add `pub use key_table::{GenerationView, ResourceView};`.

- [ ] **Create `tests/common/mod.rs`** with fixtures that implement the traits (uses only the canonical `Bytes32`/`Bytes48`/`MetadataManifest`/`TrustedHostKey` types):
```rust
#![allow(dead_code)]

use digstore_compiler::{GenerationView, ResourceView};
use digstore_core::{Bytes32, Bytes48, MetadataManifest, TrustedHostKey};
use sha2::{Digest, Sha256};

/// A single resource's contribution to a synthetic generation.
pub struct ResourceSpec {
    pub resource_key: Bytes32,
    /// (chunk_hash, chunk_body) in resource order.
    pub chunks: Vec<(Bytes32, Vec<u8>)>,
}

/// Minimal in-memory stand-in for `digstore_store::LoadedGeneration` consumed by
/// the compiler via the `GenerationView` trait.
pub struct FakeGeneration {
    pub root: Bytes32,
    pub generation_id: u64,
    pub resources: Vec<ResourceSpec>,
}

pub fn chunk(body: &[u8]) -> (Bytes32, Vec<u8>) {
    let mut h = Sha256::new();
    h.update(body);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    (Bytes32(out), body.to_vec())
}

pub fn resource_key(name: &str) -> Bytes32 {
    let mut h = Sha256::new();
    h.update(name.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    Bytes32(out)
}

/// Two generations sharing one chunk, for dedup + key-table tests.
pub fn sample_generations() -> Vec<FakeGeneration> {
    let shared = chunk(b"shared-chunk-body-0000");
    let a = chunk(b"alpha-body-1111");
    let b = chunk(b"beta-body-2222");
    vec![
        FakeGeneration {
            root: Bytes32([0x11; 32]),
            generation_id: 1,
            resources: vec![ResourceSpec {
                resource_key: resource_key("index.html"),
                chunks: vec![shared.clone(), a.clone()],
            }],
        },
        FakeGeneration {
            root: Bytes32([0x22; 32]),
            generation_id: 2,
            resources: vec![ResourceSpec {
                resource_key: resource_key("about.html"),
                chunks: vec![shared, b],
            }],
        },
    ]
}

pub fn trusted_keys() -> Vec<TrustedHostKey> {
    let pk = [0x42u8; 48];
    vec![TrustedHostKey {
        public_key: pk,
        label: format!("dig-host-key-v1:{}", hex::encode(pk)),
    }]
}

pub fn sample_manifest() -> MetadataManifest {
    MetadataManifest {
        schema_version: 1,
        name: "sample-store".to_string(),
        version: Some("1.0.0".to_string()),
        description: Some("fixture".to_string()),
        authors: vec![],
        license: None,
        homepage: None,
        repository: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        content_type: None,
        links: Default::default(),
        custom: Default::default(),
    }
}

pub fn store_id() -> Bytes32 {
    Bytes32([0xAB; 32])
}

pub fn store_pubkey() -> Bytes48 {
    Bytes48([0xCD; 48])
}

// ---- trait impls so fixtures plug into the compiler pipeline ----

impl ResourceView for ResourceSpec {
    fn resource_key(&self) -> Bytes32 {
        self.resource_key
    }
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
        self.chunks.clone()
    }
}

/// Borrowing adapter so `GenerationView::resources` can hand out trait objects.
pub struct ResourceSpecRef<'a>(pub &'a ResourceSpec);

impl<'a> ResourceView for ResourceSpecRef<'a> {
    fn resource_key(&self) -> Bytes32 {
        self.0.resource_key
    }
    fn chunks(&self) -> Vec<(Bytes32, Vec<u8>)> {
        self.0.chunks.clone()
    }
}

impl GenerationView for FakeGeneration {
    fn root(&self) -> Bytes32 {
        self.root
    }
    fn resources(&self) -> Vec<Box<dyn ResourceView + '_>> {
        self.resources
            .iter()
            .map(|r| Box::new(ResourceSpecRef(r)) as Box<dyn ResourceView + '_>)
            .collect()
    }
}
```

- [ ] **Write a trivial fixtures-build test** that actually references `mod common`, giving a real green signal. Create `crates/digstore-compiler/tests/chunk_index.rs`? No — `chunk_index.rs` exists; instead add a small test FILE. Create `crates/digstore-compiler/tests/key_table.rs` with ONLY this for now (the full key-table tests come in Task 4):
```rust
mod common;

#[test]
fn fixtures_build_and_expose_two_generations() {
    let gens = common::sample_generations();
    assert_eq!(gens.len(), 2);
    let rk = common::resource_key("about.html");
    assert!(gens.iter().any(|g| g.resources.iter().any(|r| r.resource_key == rk)));
}
```

- [ ] **Run it (expect FAIL — `common` references traits not yet re-exported? They are; this should compile and PASS).** Run:
```
cargo test -p digstore-compiler --test key_table fixtures_build_and_expose_two_generations
```
Expected: `test result: ok. 1 passed; 0 failed`. (If `unresolved import digstore_compiler::GenerationView` appears, the Task 3 re-export step was skipped — add it.)

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/key_table.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/tests/common/mod.rs crates/digstore-compiler/tests/key_table.rs
git commit -m "feat(compiler): GenerationView/ResourceView traits and test fixtures"
```

---

## Task 4 — `KeyTable` build, ordered lookup, integrity check

**Files:**
- Modify: `crates/digstore-compiler/src/key_table.rs`
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: `crates/digstore-compiler/tests/key_table.rs`

Steps:

- [ ] **Add the build/order test — red first.** Append to `tests/key_table.rs`:
```rust
use digstore_compiler::{build_chunk_index_and_key_table, KeyTable};
use digstore_core::Bytes32;

#[test]
fn entries_map_resource_to_ordered_global_chunk_indices() {
    let gens = common::sample_generations();
    let (index, table) = build_chunk_index_and_key_table(&gens);

    // Shared chunk deduped => 3 unique chunks total (shared, alpha, beta).
    assert_eq!(index.len(), 3);
    assert_eq!(table.entries().len(), 2);

    // index.html: [shared(0), alpha(1)]
    let e0 = &table.entries()[0];
    assert_eq!(e0.chunk_indices, vec![0, 1]);
    assert_eq!(e0.generation, Bytes32([0x11; 32]));

    // about.html: [shared(0), beta(2)] -- reuses shared index 0
    let e1 = &table.entries()[1];
    assert_eq!(e1.chunk_indices, vec![0, 2]);
    assert_eq!(e1.generation, Bytes32([0x22; 32]));
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler --test key_table entries_map_resource_to_ordered_global_chunk_indices
```
Expected: `error[E0432]: unresolved import digstore_compiler::build_chunk_index_and_key_table`.

- [ ] **Implement the `KeyTable` + builder in `src/key_table.rs`** (append below the traits):
```rust
use digstore_core::KeyTableEntry;

use crate::chunk_index::ChunkIndex;
use crate::error::{CompilerError, Result};

/// Resource-key table: ordered `KeyTableEntry`s. Order is deterministic:
/// generations in load order, resources in their per-generation order.
#[derive(Debug, Default)]
pub struct KeyTable {
    entries: Vec<KeyTableEntry>,
}

impl KeyTable {
    pub fn entries(&self) -> &[KeyTableEntry] {
        &self.entries
    }

    /// First entry whose `static_key` matches `rk`.
    pub fn lookup(&self, rk: &Bytes32) -> Option<&KeyTableEntry> {
        self.entries.iter().find(|e| &e.static_key == rk)
    }

    fn push(&mut self, e: KeyTableEntry) {
        self.entries.push(e);
    }

    /// Integrity check: every chunk index referenced by every entry must be
    /// within `[0, chunk_count)`. Returns `CompilerError::MissingChunk` otherwise.
    pub fn verify_against(&self, chunk_count: u32) -> Result<()> {
        for e in &self.entries {
            for &i in &e.chunk_indices {
                if i >= chunk_count {
                    return Err(CompilerError::MissingChunk(i));
                }
            }
        }
        Ok(())
    }
}

/// Stage 3 + 4 of the pipeline (§5.3): deduplicate chunks across generations into
/// the global `ChunkIndex`, then build the `KeyTable` mapping each resource key to
/// its ordered global chunk indices and reassembled size.
pub fn build_chunk_index_and_key_table<G: GenerationView>(
    generations: &[G],
) -> (ChunkIndex, KeyTable) {
    let mut index = ChunkIndex::new();
    let mut table = KeyTable::default();

    for gen in generations {
        let root = gen.root();
        for resource in gen.resources() {
            let mut chunk_indices = Vec::new();
            let mut total_size: u64 = 0;
            for (hash, body) in resource.chunks() {
                total_size += body.len() as u64;
                let gi = index.insert(hash, body);
                chunk_indices.push(gi);
            }
            table.push(KeyTableEntry {
                static_key: resource.resource_key(),
                generation: root,
                chunk_indices,
                total_size,
            });
        }
    }

    (index, table)
}
```

- [ ] **Re-export in `src/lib.rs`:** replace the prior `pub use key_table::{GenerationView, ResourceView};` line with `pub use key_table::{build_chunk_index_and_key_table, GenerationView, KeyTable, ResourceView};`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler --test key_table entries_map_resource_to_ordered_global_chunk_indices
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/key_table.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/tests/key_table.rs
git commit -m "feat(compiler): KeyTable build with global dedup and ordered entries"
```

- [ ] **Add total_size + lookup behaviors — red first.** Append to `tests/key_table.rs`:
```rust
#[test]
fn total_size_is_sum_of_chunk_body_lengths() {
    use common::{chunk, FakeGeneration, ResourceSpec};
    let gens = vec![FakeGeneration {
        root: Bytes32([1; 32]),
        generation_id: 1,
        resources: vec![ResourceSpec {
            resource_key: Bytes32([9; 32]),
            chunks: vec![chunk(b"abc"), chunk(b"de")],
        }],
    }];
    let (_index, table) = build_chunk_index_and_key_table(&gens);
    assert_eq!(table.entries()[0].total_size, 5);
}

#[test]
fn lookup_by_resource_key_returns_entry() {
    let gens = common::sample_generations();
    let (_index, table) = build_chunk_index_and_key_table(&gens);
    let rk = common::resource_key("about.html");
    let entry = table.lookup(&rk).expect("about.html present");
    assert_eq!(entry.chunk_indices, vec![0, 2]);
    assert!(table.lookup(&Bytes32([0xFF; 32])).is_none());
}
```

- [ ] **Run both (expect PASS):**
```
cargo test -p digstore-compiler --test key_table
```
Expected: `test result: ok. 4 passed; 0 failed` (fixtures_build + the three above).

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/key_table.rs
git commit -m "test(compiler): KeyTable total_size and resource-key lookup"
```

- [ ] **Add the integrity-check behavior — red first.** Append to `tests/key_table.rs`:
```rust
#[test]
fn verify_against_flags_out_of_range_index() {
    let gens = common::sample_generations();
    let (index, table) = build_chunk_index_and_key_table(&gens);
    // Real count passes.
    assert!(table.verify_against(index.len() as u32).is_ok());
    // Pretend there are fewer chunks than referenced -> MissingChunk(2).
    let err = table.verify_against(2).unwrap_err();
    assert!(matches!(err, digstore_compiler::CompilerError::MissingChunk(2)));
}
```

- [ ] **Run it (expect PASS — `verify_against` already implemented above):**
```
cargo test -p digstore-compiler --test key_table verify_against_flags_out_of_range_index
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/key_table.rs
git commit -m "test(compiler): KeyTable integrity check emits MissingChunk"
```

---

## Task 5 — Deterministic ChaCha20 filler (deviation #2)

**Files:**
- Modify: `crates/digstore-compiler/src/filler.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: `crates/digstore-compiler/tests/filler.rs`

> **Documented deviation #2 (§8.3 / §19.3):** the paper calls pool-gap padding "random filler"; pure randomness breaks byte-identical compilation. The filler is a **deterministic ChaCha20 keystream** seeded by `SHA-256(store_id ‖ roothash ‖ b"digstore-filler-v1")` — indistinguishable from random without the seed yet fully reproducible.

Steps:

- [ ] **Write the determinism test — red first.** Create `crates/digstore-compiler/tests/filler.rs`:
```rust
use digstore_compiler::deterministic_filler;
use digstore_core::Bytes32;

#[test]
fn same_seed_inputs_yield_same_bytes() {
    let sid = Bytes32([1; 32]);
    let root = Bytes32([2; 32]);
    let a = deterministic_filler(&sid, &root, 100);
    let b = deterministic_filler(&sid, &root, 100);
    assert_eq!(a, b);
    assert_eq!(a.len(), 100);
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler --test filler same_seed_inputs_yield_same_bytes
```
Expected: `error[E0432]: unresolved import digstore_compiler::deterministic_filler`.

- [ ] **Implement `src/filler.rs`:**
```rust
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use sha2::{Digest, Sha256};

use digstore_core::Bytes32;

/// Domain-separation tag for the filler seed (documented deviation #2, §19.3).
const FILLER_DOMAIN: &[u8] = b"digstore-filler-v1";

/// Produce `len` bytes of deterministic pseudo-random filler for the interleaved
/// pool gaps. The keystream is positional, so a shorter request is a prefix of a
/// longer one for the same seed.
///
/// seed = SHA-256(store_id || roothash || b"digstore-filler-v1")
/// key  = seed (32 bytes), nonce = 12 zero bytes.
pub fn deterministic_filler(store_id: &Bytes32, roothash: &Bytes32, len: usize) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(store_id.0);
    hasher.update(roothash.0);
    hasher.update(FILLER_DOMAIN);
    let seed: [u8; 32] = hasher.finalize().into();

    let nonce = [0u8; 12];
    let mut cipher = ChaCha20::new(&seed.into(), &nonce.into());
    let mut buf = vec![0u8; len];
    cipher.apply_keystream(&mut buf); // XOR with zero buffer = raw keystream
    buf
}
```

- [ ] **Re-export in `src/lib.rs`:** add `pub use filler::deterministic_filler;`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler --test filler same_seed_inputs_yield_same_bytes
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/filler.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/tests/filler.rs
git commit -m "feat(compiler): deterministic ChaCha20 pool filler keyed by store_id+roothash"
```

- [ ] **Add seed-binding + prefix behaviors — red first.** Append to `tests/filler.rs`:
```rust
#[test]
fn different_store_id_changes_stream() {
    let root = Bytes32([2; 32]);
    let a = deterministic_filler(&Bytes32([1; 32]), &root, 64);
    let b = deterministic_filler(&Bytes32([9; 32]), &root, 64);
    assert_ne!(a, b);
}

#[test]
fn different_roothash_changes_stream() {
    let sid = Bytes32([1; 32]);
    let a = deterministic_filler(&sid, &Bytes32([2; 32]), 64);
    let b = deterministic_filler(&sid, &Bytes32([3; 32]), 64);
    assert_ne!(a, b);
}

#[test]
fn prefix_property_first_bytes_match_longer_request() {
    let sid = Bytes32([7; 32]);
    let root = Bytes32([8; 32]);
    let short = deterministic_filler(&sid, &root, 16);
    let long = deterministic_filler(&sid, &root, 64);
    assert_eq!(short, &long[..16]);
}

#[test]
fn zero_length_is_empty() {
    let f = deterministic_filler(&Bytes32([0; 32]), &Bytes32([0; 32]), 0);
    assert!(f.is_empty());
}
```

- [ ] **Run the full file (expect PASS):**
```
cargo test -p digstore-compiler --test filler
```
Expected: `test result: ok. 5 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/filler.rs
git commit -m "test(compiler): filler seed binding and positional prefix property"
```

---

## Task 6 — Interleaved chunk pool with bucketed length (§8.3)

**Files:**
- Modify: `crates/digstore-compiler/src/pool.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/pool.rs`

> **§8.3:** all chunks across all resources go into one interleaved pool with filler in the gaps and **no observable resource boundary**. Total pool length is bucketed up to the next power-of-two bucket (64-byte floor) so the size leaks only a coarse bucket; the tail gap is filled with deterministic filler (Task 5).

Steps:

- [ ] **Write the bucket-function test — red first.** Add to the bottom of `src/pool.rs` (replacing the stub):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_function_rounds_up_to_next_power_of_two_from_floor() {
        assert_eq!(next_pool_bucket(0), 64);
        assert_eq!(next_pool_bucket(35), 64);
        assert_eq!(next_pool_bucket(64), 64);
        assert_eq!(next_pool_bucket(65), 128);
        assert_eq!(next_pool_bucket(4096), 4096);
        assert_eq!(next_pool_bucket(4097), 8192);
    }
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler pool::tests::bucket_function_rounds_up_to_next_power_of_two_from_floor
```
Expected: `error[E0425]: cannot find function next_pool_bucket in this scope`.

- [ ] **Implement the bucket function in `src/pool.rs`** (prepend above the test module):
```rust
use digstore_core::Bytes32;

use crate::filler::deterministic_filler;

/// Location of one chunk inside the interleaved pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkLoc {
    pub offset: u32,
    pub len: u32,
}

/// The assembled interleaved pool: a flat byte buffer with chunk bodies packed in
/// global-index order, the trailing gap filled with deterministic filler, and a
/// parallel list of `(offset,len)` descriptors. No resource boundary is encoded.
#[derive(Debug)]
pub struct InterleavedPool {
    pub bytes: Vec<u8>,
    pub descriptors: Vec<ChunkLoc>,
}

/// Smallest bucket >= `n`, stepping in powers of two from a 64-byte floor. Hides
/// the exact content byte count so module size leaks only a coarse bucket (§8.3).
pub fn next_pool_bucket(n: usize) -> usize {
    let mut b = 64usize;
    while b < n {
        b <<= 1;
    }
    b
}
```

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler pool::tests::bucket_function_rounds_up_to_next_power_of_two_from_floor
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Re-export in `src/lib.rs`:** add `pub use pool::{build_pool, next_pool_bucket, ChunkLoc, InterleavedPool};`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/pool.rs crates/digstore-compiler/src/lib.rs
git commit -m "feat(compiler): next_pool_bucket power-of-two length bucketing"
```

- [ ] **Add the pool-assembly test — red first.** Append inside `mod tests`:
```rust
    fn bodies() -> Vec<Vec<u8>> {
        vec![vec![1u8; 10], vec![2u8; 20], vec![3u8; 5]]
    }

    #[test]
    fn pool_contains_each_chunk_body_in_index_order() {
        let sid = Bytes32([1; 32]);
        let root = Bytes32([2; 32]);
        let pool = build_pool(&sid, &root, &bodies());
        assert_eq!(pool.descriptors[0], ChunkLoc { offset: 0, len: 10 });
        assert_eq!(pool.descriptors[1], ChunkLoc { offset: 10, len: 20 });
        assert_eq!(pool.descriptors[2], ChunkLoc { offset: 30, len: 5 });
        assert_eq!(&pool.bytes[0..10], &[1u8; 10]);
        assert_eq!(&pool.bytes[10..30], &[2u8; 20]);
        assert_eq!(&pool.bytes[30..35], &[3u8; 5]);
    }
```
(Add `use digstore_core::Bytes32;` to `mod tests` if not present — it is, via `super::*`.)

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler pool::tests::pool_contains_each_chunk_body_in_index_order
```
Expected: `error[E0425]: cannot find function build_pool in this scope`.

- [ ] **Implement `build_pool` in `src/pool.rs`** (append below `next_pool_bucket`):
```rust
/// Build the interleaved pool from chunk bodies in global-index order.
pub fn build_pool(store_id: &Bytes32, roothash: &Bytes32, bodies: &[Vec<u8>]) -> InterleavedPool {
    let content_len: usize = bodies.iter().map(|b| b.len()).sum();
    let total = next_pool_bucket(content_len);

    // Start from the full-length deterministic filler, then overwrite the content
    // prefix. Because the keystream is positional, the filler tail is identical to
    // what a fresh `deterministic_filler` of the same length produces at that range.
    let mut bytes = deterministic_filler(store_id, roothash, total);

    let mut descriptors = Vec::with_capacity(bodies.len());
    let mut cursor = 0u32;
    for body in bodies {
        let len = body.len() as u32;
        let start = cursor as usize;
        bytes[start..start + body.len()].copy_from_slice(body);
        descriptors.push(ChunkLoc { offset: cursor, len });
        cursor += len;
    }

    InterleavedPool { bytes, descriptors }
}
```

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler pool::tests::pool_contains_each_chunk_body_in_index_order
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/pool.rs
git commit -m "feat(compiler): interleaved pool packs chunk bodies in index order"
```

- [ ] **Add the bucketing + filler-gap + empty-set behaviors — red first.** Append inside `mod tests`:
```rust
    #[test]
    fn pool_length_is_bucketed_above_content_and_filled_with_filler() {
        let sid = Bytes32([1; 32]);
        let root = Bytes32([2; 32]);
        let pool = build_pool(&sid, &root, &bodies()); // 35 content bytes
        assert_eq!(pool.bytes.len(), 64);
        let filler = crate::filler::deterministic_filler(&sid, &root, 64);
        assert_eq!(&pool.bytes[35..64], &filler[35..64]);
    }

    #[test]
    fn empty_chunk_set_still_yields_filled_bucket() {
        let pool = build_pool(&Bytes32([0; 32]), &Bytes32([0; 32]), &[]);
        assert_eq!(pool.bytes.len(), 64);
        assert!(pool.descriptors.is_empty());
    }
```

- [ ] **Run the full module (expect PASS):**
```
cargo test -p digstore-compiler pool::tests
```
Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/pool.rs
git commit -m "test(compiler): pool bucketing, filler gaps, and empty-set bucket"
```

---

## Task 7 — Data-section encoding: header, offset table, canonical-`Encode` segments

**Files:**
- Modify: `crates/digstore-compiler/src/data_section.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/data_section.rs`

> **Layout (locked):** magic `b"DIGS"`, `u8 format_version = 1`, then a **section offset table** (4-byte BE count, then rows of `kind:u8, offset:u32 BE, len:u32 BE`), then the concatenated segment bodies. The outer header/offset-table framing is LOCAL to the compiler (the guest's first read parses it). Every **typed segment body** is produced by `digstore-core`'s canonical custom big-endian `Encode` trait so the bytes are byte-identical to what the guest's `Decode` consumes (fixes the hand-rolled-codec divergence blocker). Multi-byte integers are big-endian (Chia streamable, deviation #1).
>
> **Segment kinds (documented for the guest):**
> - `0 = SEG_POOL`: `Bytes(pool_bytes)` then `Vec<ChunkLoc>` — both via core `Encode`.
> - `1 = SEG_KEY_TABLE`: `Vec<KeyTableEntry>` via core `Encode`.
> - `2 = SEG_STORE_HEADER`: store-identity fields, individually addressable: `store_id: Bytes32`, `roothash: Bytes32`, `root_history: Vec<Bytes32>`, `store_pubkey: Bytes48` — all via core `Encode`. (Renamed from "Metadata" to avoid colliding with the manifest; this is the segment the guest's `get_store_id`/`get_current_roothash`/`get_roothash_history`/`get_public_key` read.)
> - `3 = SEG_MANIFEST`: the `MetadataManifest` via core `Encode` (NOT JSON — fixes the JSON blocker).
> - `4 = SEG_TRUSTED_KEYS`: `Vec<TrustedHostKey>` via core `Encode`.

> **Assumed `digstore-core` API (from the canonical catalog):** `trait Encode { fn encode(&self, out: &mut Vec<u8>); }` and `trait Decode { fn decode(buf: &mut &[u8]) -> core::result::Result<Self, digstore_core::CodecError> where Self: Sized; }`, both implemented for `Bytes32`, `Bytes48`, `String`, `u32`, `u64`, `Vec<T: Encode>`, `KeyTableEntry`, `MetadataManifest`, `TrustedHostKey`. `ChunkLoc` is a compiler type, so the compiler implements `Encode`/`Decode` for it here.

Steps:

- [ ] **Implement `Encode`/`Decode` for `ChunkLoc` — red first.** Add to the bottom of `src/pool.rs`:
```rust
#[cfg(test)]
mod codec_tests {
    use super::*;
    use digstore_core::{Decode, Encode};

    #[test]
    fn chunkloc_round_trips_via_core_codec() {
        let loc = ChunkLoc { offset: 7, len: 41 };
        let mut buf = Vec::new();
        loc.encode(&mut buf);
        assert_eq!(buf, vec![0, 0, 0, 7, 0, 0, 0, 41]); // two BE u32s
        let mut slice = buf.as_slice();
        let back = ChunkLoc::decode(&mut slice).unwrap();
        assert_eq!(back, loc);
        assert!(slice.is_empty());
    }
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler pool::codec_tests::chunkloc_round_trips_via_core_codec
```
Expected: `error[E0277]: the trait bound ChunkLoc: digstore_core::Encode is not satisfied`.

- [ ] **Implement the `ChunkLoc` codec in `src/pool.rs`** (append, above the test modules):
```rust
impl digstore_core::Encode for ChunkLoc {
    fn encode(&self, out: &mut Vec<u8>) {
        self.offset.encode(out);
        self.len.encode(out);
    }
}

impl digstore_core::Decode for ChunkLoc {
    fn decode(buf: &mut &[u8]) -> core::result::Result<Self, digstore_core::CodecError> {
        let offset = u32::decode(buf)?;
        let len = u32::decode(buf)?;
        Ok(ChunkLoc { offset, len })
    }
}
```
(Add `use digstore_core::{Decode, Encode};` at the top of `src/pool.rs` next to the existing imports.)

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler pool::codec_tests::chunkloc_round_trips_via_core_codec
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/pool.rs
git commit -m "feat(compiler): ChunkLoc implements core Encode/Decode (big-endian)"
```

- [ ] **Write the header+offset-table test — red first.** Add to the bottom of `src/data_section.rs` (replacing the stub):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, Bytes48, KeyTableEntry, MetadataManifest, TrustedHostKey};
    use crate::pool::ChunkLoc;

    fn manifest() -> MetadataManifest {
        MetadataManifest {
            schema_version: 1,
            name: "n".into(),
            version: None,
            description: None,
            authors: vec![],
            license: None,
            homepage: None,
            repository: None,
            keywords: vec![],
            categories: vec![],
            icon: None,
            content_type: None,
            links: Default::default(),
            custom: Default::default(),
        }
    }

    fn inputs() -> DataSectionInputs {
        DataSectionInputs {
            store_id: Bytes32([0xAB; 32]),
            roothash: Bytes32([0x11; 32]),
            root_history: vec![Bytes32([0x11; 32])],
            store_pubkey: Bytes48([0xCD; 48]),
            pool_bytes: vec![9u8; 64],
            pool_descriptors: vec![ChunkLoc { offset: 0, len: 64 }],
            key_table: vec![KeyTableEntry {
                static_key: Bytes32([1; 32]),
                generation: Bytes32([0x11; 32]),
                chunk_indices: vec![0],
                total_size: 64,
            }],
            manifest: manifest(),
            trusted_keys: vec![TrustedHostKey {
                public_key: [0x42u8; 48],
                label: "dig-host-key-v1:abc".into(),
            }],
        }
    }

    #[test]
    fn starts_with_magic_and_version() {
        let blob = encode_data_section(&inputs());
        assert_eq!(&blob[0..4], b"DIGS");
        assert_eq!(blob[4], 1u8);
    }

    #[test]
    fn offset_table_has_five_segments_big_endian_count() {
        let blob = encode_data_section(&inputs());
        let count = u32::from_be_bytes([blob[5], blob[6], blob[7], blob[8]]);
        assert_eq!(count, 5);
    }
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler data_section::tests::starts_with_magic_and_version
```
Expected: `error[E0422]: cannot find struct DataSectionInputs` / `cannot find function encode_data_section`.

- [ ] **Implement `src/data_section.rs`:**
```rust
use digstore_core::{Bytes32, Bytes48, Decode, Encode, KeyTableEntry, MetadataManifest, TrustedHostKey};

use crate::error::{CompilerError, Result};
use crate::pool::ChunkLoc;

pub const MAGIC: &[u8; 4] = b"DIGS";
pub const FORMAT_VERSION: u8 = 1;

pub const SEG_POOL: u8 = 0;
pub const SEG_KEY_TABLE: u8 = 1;
pub const SEG_STORE_HEADER: u8 = 2;
pub const SEG_MANIFEST: u8 = 3;
pub const SEG_TRUSTED_KEYS: u8 = 4;

/// One row of the section offset table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionEntry {
    pub kind: u8,
    pub offset: u32,
    pub len: u32,
}

/// All inputs needed to encode the data section (gathered by the pipeline). All
/// 48-byte keys use the canonical `Bytes48` newtype; `TrustedHostKey` carries its
/// own `[u8;48]` per the catalog and is encoded via core `Encode`.
pub struct DataSectionInputs {
    pub store_id: Bytes32,
    pub roothash: Bytes32,
    pub root_history: Vec<Bytes32>,
    pub store_pubkey: Bytes48,
    pub pool_bytes: Vec<u8>,
    pub pool_descriptors: Vec<ChunkLoc>,
    pub key_table: Vec<KeyTableEntry>,
    pub manifest: MetadataManifest,
    pub trusted_keys: Vec<TrustedHostKey>,
}

// ---- segment body encoders (ALL via canonical core Encode) ----

fn encode_pool_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut s = Vec::new();
    i.pool_bytes.encode(&mut s); // Bytes: BE u32 len + raw bytes (core codec)
    i.pool_descriptors.encode(&mut s); // Vec<ChunkLoc>: BE u32 count + items
    s
}

fn encode_key_table_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut s = Vec::new();
    i.key_table.encode(&mut s);
    s
}

fn encode_store_header_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut s = Vec::new();
    i.store_id.encode(&mut s);
    i.roothash.encode(&mut s);
    i.root_history.encode(&mut s);
    i.store_pubkey.encode(&mut s);
    s
}

fn encode_manifest_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut s = Vec::new();
    i.manifest.encode(&mut s);
    s
}

fn encode_trusted_keys_segment(i: &DataSectionInputs) -> Vec<u8> {
    let mut s = Vec::new();
    i.trusted_keys.encode(&mut s);
    s
}

/// Encode the full data-section blob: magic + version + offset table + segments.
pub fn encode_data_section(i: &DataSectionInputs) -> Vec<u8> {
    let segments: Vec<(u8, Vec<u8>)> = vec![
        (SEG_POOL, encode_pool_segment(i)),
        (SEG_KEY_TABLE, encode_key_table_segment(i)),
        (SEG_STORE_HEADER, encode_store_header_segment(i)),
        (SEG_MANIFEST, encode_manifest_segment(i)),
        (SEG_TRUSTED_KEYS, encode_trusted_keys_segment(i)),
    ];

    // Header = magic(4) + version(1) + count(4) + table(count * (1+4+4)).
    let header_len = 4 + 1 + 4 + segments.len() * (1 + 4 + 4);
    let mut offset = header_len as u32;

    let mut table = Vec::with_capacity(segments.len());
    for (kind, body) in &segments {
        table.push(SectionEntry { kind: *kind, offset, len: body.len() as u32 });
        offset += body.len() as u32;
    }

    let mut out = Vec::with_capacity(offset as usize);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&(segments.len() as u32).to_be_bytes());
    for e in &table {
        out.push(e.kind);
        out.extend_from_slice(&e.offset.to_be_bytes());
        out.extend_from_slice(&e.len.to_be_bytes());
    }
    for (_kind, body) in &segments {
        out.extend_from_slice(body);
    }
    debug_assert_eq!(out.len(), offset as usize);
    out
}

/// Parse the offset table. Returns `CompilerError::Validation` on malformed input
/// (bad magic/version, truncated table) rather than panicking.
pub fn parse_offset_table(blob: &[u8]) -> Result<Vec<SectionEntry>> {
    if blob.len() < 9 {
        return Err(CompilerError::Validation("data section too short".into()));
    }
    if &blob[0..4] != MAGIC {
        return Err(CompilerError::Validation("bad data-section magic".into()));
    }
    if blob[4] != FORMAT_VERSION {
        return Err(CompilerError::Validation(format!(
            "unsupported data-section version {}",
            blob[4]
        )));
    }
    let count = u32::from_be_bytes([blob[5], blob[6], blob[7], blob[8]]) as usize;
    let table_end = 9 + count * 9;
    if blob.len() < table_end {
        return Err(CompilerError::Validation("offset table truncated".into()));
    }
    let mut entries = Vec::with_capacity(count);
    let mut p = 9usize;
    for _ in 0..count {
        let kind = blob[p];
        let offset = u32::from_be_bytes([blob[p + 1], blob[p + 2], blob[p + 3], blob[p + 4]]);
        let len = u32::from_be_bytes([blob[p + 5], blob[p + 6], blob[p + 7], blob[p + 8]]);
        if (offset as usize)
            .checked_add(len as usize)
            .map(|end| end > blob.len())
            .unwrap_or(true)
        {
            return Err(CompilerError::Validation("segment out of bounds".into()));
        }
        entries.push(SectionEntry { kind, offset, len });
        p += 9;
    }
    Ok(entries)
}

/// Decode the store-header segment back to its fields (used by the round-trip test
/// and mirrors the guest's decode path).
pub fn decode_store_header(
    body: &[u8],
) -> Result<(Bytes32, Bytes32, Vec<Bytes32>, Bytes48)> {
    let mut buf = body;
    let store_id = Bytes32::decode(&mut buf).map_err(|e| CompilerError::Validation(e.to_string()))?;
    let roothash = Bytes32::decode(&mut buf).map_err(|e| CompilerError::Validation(e.to_string()))?;
    let root_history =
        Vec::<Bytes32>::decode(&mut buf).map_err(|e| CompilerError::Validation(e.to_string()))?;
    let pubkey = Bytes48::decode(&mut buf).map_err(|e| CompilerError::Validation(e.to_string()))?;
    Ok((store_id, roothash, root_history, pubkey))
}
```

- [ ] **Re-export in `src/lib.rs`:** add
```rust
pub use data_section::{
    decode_store_header, encode_data_section, parse_offset_table, DataSectionInputs, SectionEntry,
    SEG_KEY_TABLE, SEG_MANIFEST, SEG_POOL, SEG_STORE_HEADER, SEG_TRUSTED_KEYS,
};
```

- [ ] **Run header tests (expect PASS):**
```
cargo test -p digstore-compiler data_section::tests::starts_with_magic_and_version data_section::tests::offset_table_has_five_segments_big_endian_count
```
Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/data_section.rs crates/digstore-compiler/src/lib.rs
git commit -m "feat(compiler): data-section codec via core Encode with offset table"
```

- [ ] **Add the offset-table integrity test — red first.** Append inside `data_section::tests`:
```rust
    #[test]
    fn segment_offsets_are_ascending_in_bounds_and_canonical_order() {
        let blob = encode_data_section(&inputs());
        let table = parse_offset_table(&blob).expect("table parses");
        assert_eq!(table.len(), 5);
        let kinds: Vec<u8> = table.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![0, 1, 2, 3, 4]);
        let mut prev_end = 0u32;
        for e in &table {
            assert!(e.offset >= prev_end);
            assert!((e.offset + e.len) as usize <= blob.len());
            prev_end = e.offset + e.len;
        }
    }

    #[test]
    fn parse_offset_table_rejects_bad_magic() {
        let mut blob = encode_data_section(&inputs());
        blob[0] = b'X';
        assert!(parse_offset_table(&blob).is_err());
    }
```

- [ ] **Run them (expect PASS):**
```
cargo test -p digstore-compiler data_section::tests::segment_offsets_are_ascending_in_bounds_and_canonical_order data_section::tests::parse_offset_table_rejects_bad_magic
```
Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/data_section.rs
git commit -m "test(compiler): offset table ordering, bounds, and malformed-input rejection"
```

- [ ] **Add the store-header round-trip test (cross-checks the rename + addressability) — red first.** Append inside `data_section::tests`:
```rust
    #[test]
    fn store_header_segment_round_trips_via_decode() {
        let inp = inputs();
        let blob = encode_data_section(&inp);
        let table = parse_offset_table(&blob).unwrap();
        let seg = table.iter().find(|e| e.kind == SEG_STORE_HEADER).unwrap();
        let body = &blob[seg.offset as usize..(seg.offset + seg.len) as usize];
        let (sid, root, hist, pk) = decode_store_header(body).unwrap();
        assert_eq!(sid, inp.store_id);
        assert_eq!(root, inp.roothash);
        assert_eq!(hist, inp.root_history);
        assert_eq!(pk, inp.store_pubkey);
    }
```

- [ ] **Run it (expect PASS):**
```
cargo test -p digstore-compiler data_section::tests::store_header_segment_round_trips_via_decode
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/data_section.rs
git commit -m "test(compiler): store-header segment decode round-trip (addressable fields)"
```

---

## Task 8 — Golden data-section vector, independently validated

**Files:**
- Create: `crates/digstore-compiler/tests/data_section_golden.rs`
- Create: `crates/digstore-compiler/tests/fixtures/golden_data_section.hex`

> This test does NOT just echo the implementation output. It (a) asserts the layout structurally via an INDEPENDENT decode path (`parse_offset_table` + core `Decode`), and (b) pins a golden hex. The golden hex is generated ONCE, then the structural assertions guard it on every change so a wrong-but-stable layout is still caught by the independent path.

Steps:

- [ ] **Write the structural + golden test — red first.** Create `crates/digstore-compiler/tests/data_section_golden.rs`:
```rust
mod common;

use digstore_compiler::{
    decode_store_header, encode_data_section, parse_offset_table, ChunkLoc, DataSectionInputs,
    SEG_KEY_TABLE, SEG_MANIFEST, SEG_POOL, SEG_STORE_HEADER, SEG_TRUSTED_KEYS,
};
use digstore_core::{Bytes32, Bytes48, Decode, KeyTableEntry, MetadataManifest, TrustedHostKey};

fn fixed_inputs() -> DataSectionInputs {
    DataSectionInputs {
        store_id: Bytes32([0xAB; 32]),
        roothash: Bytes32([0x11; 32]),
        root_history: vec![Bytes32([0x11; 32])],
        store_pubkey: Bytes48([0xCD; 48]),
        pool_bytes: vec![0x09; 16],
        pool_descriptors: vec![ChunkLoc { offset: 0, len: 16 }],
        key_table: vec![KeyTableEntry {
            static_key: Bytes32([0x01; 32]),
            generation: Bytes32([0x11; 32]),
            chunk_indices: vec![0],
            total_size: 16,
        }],
        manifest: common::sample_manifest(),
        trusted_keys: vec![TrustedHostKey {
            public_key: [0x42u8; 48],
            label: "L".into(),
        }],
    }
}

#[test]
fn structure_is_independently_valid() {
    let inp = fixed_inputs();
    let blob = encode_data_section(&inp);

    // Header.
    assert_eq!(&blob[0..4], b"DIGS");
    assert_eq!(blob[4], 1u8);

    // Offset table: five canonical segments, ascending, in bounds.
    let table = parse_offset_table(&blob).expect("table parses");
    let kinds: Vec<u8> = table.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![SEG_POOL, SEG_KEY_TABLE, SEG_STORE_HEADER, SEG_MANIFEST, SEG_TRUSTED_KEYS]
    );

    // Pool segment decodes to the original bytes + descriptors.
    let pool_seg = table.iter().find(|e| e.kind == SEG_POOL).unwrap();
    let mut body = &blob[pool_seg.offset as usize..(pool_seg.offset + pool_seg.len) as usize];
    let pool_bytes = Vec::<u8>::decode(&mut body).unwrap();
    let descs = Vec::<ChunkLoc>::decode(&mut body).unwrap();
    assert_eq!(pool_bytes, vec![0x09u8; 16]);
    assert_eq!(descs, vec![ChunkLoc { offset: 0, len: 16 }]);

    // Key table decodes.
    let kt_seg = table.iter().find(|e| e.kind == SEG_KEY_TABLE).unwrap();
    let mut body = &blob[kt_seg.offset as usize..(kt_seg.offset + kt_seg.len) as usize];
    let entries = Vec::<KeyTableEntry>::decode(&mut body).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].chunk_indices, vec![0]);
    assert_eq!(entries[0].total_size, 16);

    // Store header decodes to the original identity fields.
    let sh_seg = table.iter().find(|e| e.kind == SEG_STORE_HEADER).unwrap();
    let sh_body = &blob[sh_seg.offset as usize..(sh_seg.offset + sh_seg.len) as usize];
    let (sid, root, hist, pk) = decode_store_header(sh_body).unwrap();
    assert_eq!(sid, inp.store_id);
    assert_eq!(root, inp.roothash);
    assert_eq!(hist, inp.root_history);
    assert_eq!(pk, inp.store_pubkey);

    // Manifest decodes.
    let mf_seg = table.iter().find(|e| e.kind == SEG_MANIFEST).unwrap();
    let mut body = &blob[mf_seg.offset as usize..(mf_seg.offset + mf_seg.len) as usize];
    let mf = MetadataManifest::decode(&mut body).unwrap();
    assert_eq!(mf.name, "sample-store");

    // Trusted keys decode.
    let tk_seg = table.iter().find(|e| e.kind == SEG_TRUSTED_KEYS).unwrap();
    let mut body = &blob[tk_seg.offset as usize..(tk_seg.offset + tk_seg.len) as usize];
    let keys = Vec::<TrustedHostKey>::decode(&mut body).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].label, "L");
}

#[test]
fn data_section_matches_golden_vector() {
    let blob = encode_data_section(&fixed_inputs());
    let got = hex::encode(&blob);
    let expected = include_str!("fixtures/golden_data_section.hex").trim();
    if got != expected {
        eprintln!("GOLDEN MISMATCH. Review the structural test, then if intentional update the fixture to:\n{got}");
    }
    assert_eq!(got, expected, "data-section layout changed; structural test guards correctness");
}
```

- [ ] **Create the placeholder fixture** so the test compiles. Create `crates/digstore-compiler/tests/fixtures/golden_data_section.hex` with the single line:
```
PENDING
```

- [ ] **Run the structural test FIRST (expect PASS — it validates correctness independently):**
```
cargo test -p digstore-compiler --test data_section_golden structure_is_independently_valid
```
Expected: `test result: ok. 1 passed; 0 failed`. (This is the genuine correctness gate; the golden vector below is a regression pin, not the correctness oracle.)

- [ ] **Run the golden test (expect FAIL — prints the bytes the now-validated encoder produces):**
```
cargo test -p digstore-compiler --test data_section_golden data_section_matches_golden_vector
```
Expected: `GOLDEN MISMATCH. ... <long hex>` then `assertion `left == right` failed`.

- [ ] **Pin the golden.** Copy the printed hex string verbatim and write it as the sole line of `crates/digstore-compiler/tests/fixtures/golden_data_section.hex`, replacing `PENDING`. (The bytes are already proven correct by `structure_is_independently_valid`.)

- [ ] **Run both (expect PASS):**
```
cargo test -p digstore-compiler --test data_section_golden
```
Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/data_section_golden.rs crates/digstore-compiler/tests/fixtures/golden_data_section.hex
git commit -m "test(compiler): independently-validated golden data-section vector"
```

---

## Task 9 — Pinned guest template fixture + build script + template loader (§5.1)

**Files:**
- Create: `crates/digstore-compiler/fixtures/digstore_guest_template.wat`
- Create: `crates/digstore-compiler/build.rs`
- Modify: `crates/digstore-compiler/src/template.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/template.rs`

> **Determinism (fixes the build.rs nondeterminism blocker):** the template is a SINGLE pinned committed fixture. `build.rs` only assembles the committed `.wat` into wasm with `wat` (a pure, version-pinned transform); it NEVER invokes `cargo build -p digstore-guest`. Every developer/CI gets byte-identical template bytes, so §19.3 holds across environments. The real `digstore-guest` wasm, when released, is committed in place of this fixture (same path), keeping the template a pinned input.
>
> **§5.1 memory bounds:** the template declares a 4-page (256 KiB) min / 256-page (16 MiB) max memory and reserves a data region. `load_template` parses the `MemorySection` and asserts `max <= 256` and that a minimum is declared.

Steps:

- [ ] **Create the pinned template source** `crates/digstore-compiler/fixtures/digstore_guest_template.wat`. It declares the full export ABI, a 4-page/256-page memory, and a reserved-region marker data segment that injection replaces:
```wat
(module
  (memory (export "memory") 4 256)
  (func (export "get_store_id") (result i64) (i64.const 0))
  (func (export "get_current_roothash") (result i64) (i64.const 0))
  (func (export "get_roothash_history") (result i64) (i64.const 0))
  (func (export "get_public_key") (result i64) (i64.const 0))
  (func (export "get_metadata") (result i64) (i64.const 0))
  (func (export "get_authentication_info") (result i64) (i64.const 0))
  (func (export "get_content") (param i32 i32) (result i64) (i64.const 0))
  (func (export "get_proof") (param i32 i32) (result i64) (i64.const 0))
  (func (export "alloc") (param i32) (result i32) (i32.const 0))
  (func (export "dealloc") (param i32 i32))
  (func (export "init") (result i32) (i32.const 0))
  (data (i32.const 65536) "\00")
)
```

- [ ] **Create `crates/digstore-compiler/build.rs`** (pure assembly only; no recursive cargo):
```rust
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_dir.join("fixtures/digstore_guest_template.wat");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("digstore_guest_template.wasm");

    let wat = std::fs::read_to_string(&src).expect("read template wat");
    let wasm = wat::parse_str(&wat).expect("assemble template wat");
    std::fs::write(&dest, wasm).expect("write template wasm");

    println!("cargo:rerun-if-changed=fixtures/digstore_guest_template.wat");
}
```

- [ ] **Write the export-validation test — red first.** Add to the bottom of `src/template.rs` (replacing the stub):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_template_has_all_required_exports() {
        let bytes = baked_template_bytes();
        let t = load_template(bytes).expect("template valid");
        for name in REQUIRED_EXPORTS {
            assert!(t.has_export(name), "missing export {name}");
        }
    }
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler template::tests::baked_template_has_all_required_exports
```
Expected: `error[E0425]: cannot find function baked_template_bytes` (and/or `load_template`).

- [ ] **Implement `src/template.rs`:**
```rust
use wasmparser::{Parser, Payload};

use crate::error::{CompilerError, Result};

/// Maximum linear-memory pages the served module may declare (§5.1: 16 MiB ceiling).
pub const MAX_MEMORY_PAGES: u64 = 256;

/// Exports the served module must expose (guest ABI).
pub const REQUIRED_EXPORTS: &[&str] = &[
    "get_store_id",
    "get_current_roothash",
    "get_roothash_history",
    "get_public_key",
    "get_metadata",
    "get_authentication_info",
    "get_content",
    "get_proof",
    "alloc",
    "dealloc",
    "init",
    "memory",
];

/// A validated guest template ready for data injection.
pub struct Template {
    pub bytes: Vec<u8>,
    exports: Vec<String>,
    /// Declared memory limits (min_pages, max_pages_opt) of memory 0.
    pub memory_min_pages: u64,
    pub memory_max_pages: Option<u64>,
}

impl Template {
    pub fn has_export(&self, name: &str) -> bool {
        self.exports.iter().any(|e| e == name)
    }
}

/// The pinned template bytes assembled by `build.rs` from the committed `.wat`.
pub fn baked_template_bytes() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/digstore_guest_template.wasm"))
}

/// Parse + validate the template (§5.1): collect export names, assert the full
/// required ABI surface, and assert memory bounds (a memory exists, max <= 256).
pub fn load_template(bytes: &[u8]) -> Result<Template> {
    let mut exports = Vec::new();
    let mut memory_min_pages: Option<u64> = None;
    let mut memory_max_pages: Option<u64> = None;

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
        match payload {
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export =
                        export.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    exports.push(export.name.to_string());
                }
            }
            Payload::MemorySection(reader) => {
                for mem in reader {
                    let mem = mem.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    if memory_min_pages.is_none() {
                        memory_min_pages = Some(mem.initial);
                        memory_max_pages = mem.maximum;
                    }
                }
            }
            _ => {}
        }
    }

    for name in REQUIRED_EXPORTS {
        if !exports.iter().any(|e| e == name) {
            return Err(CompilerError::InvalidTemplate(format!(
                "missing export {name}"
            )));
        }
    }

    let min = memory_min_pages
        .ok_or_else(|| CompilerError::InvalidTemplate("template declares no memory".into()))?;
    if let Some(max) = memory_max_pages {
        if max > MAX_MEMORY_PAGES {
            return Err(CompilerError::InvalidTemplate(format!(
                "memory max {max} pages exceeds ceiling {MAX_MEMORY_PAGES}"
            )));
        }
    }

    Ok(Template {
        bytes: bytes.to_vec(),
        exports,
        memory_min_pages: min,
        memory_max_pages,
    })
}
```

- [ ] **Re-export in `src/lib.rs`:** add `pub use template::{baked_template_bytes, load_template, Template, MAX_MEMORY_PAGES, REQUIRED_EXPORTS};`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler template::tests::baked_template_has_all_required_exports
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/fixtures/digstore_guest_template.wat crates/digstore-compiler/build.rs crates/digstore-compiler/src/template.rs crates/digstore-compiler/src/lib.rs
git commit -m "feat(compiler): pinned guest template fixture and ABI/memory validation"
```

- [ ] **Add the missing-export + memory-bound rejection tests — red first.** Append inside `template::tests`:
```rust
    #[test]
    fn template_missing_export_is_rejected() {
        let watsrc = r#"(module (memory (export "memory") 1 256))"#;
        let bytes = wat::parse_str(watsrc).unwrap();
        let err = load_template(&bytes).unwrap_err();
        assert!(err.to_string().contains("get_content"));
    }

    #[test]
    fn template_with_memory_max_over_ceiling_is_rejected() {
        // Full ABI but max pages 257 (> 256) -> rejected.
        let watsrc = r#"(module
          (memory (export "memory") 1 257)
          (func (export "get_store_id") (result i64) (i64.const 0))
          (func (export "get_current_roothash") (result i64) (i64.const 0))
          (func (export "get_roothash_history") (result i64) (i64.const 0))
          (func (export "get_public_key") (result i64) (i64.const 0))
          (func (export "get_metadata") (result i64) (i64.const 0))
          (func (export "get_authentication_info") (result i64) (i64.const 0))
          (func (export "get_content") (param i32 i32) (result i64) (i64.const 0))
          (func (export "get_proof") (param i32 i32) (result i64) (i64.const 0))
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "dealloc") (param i32 i32))
          (func (export "init") (result i32) (i32.const 0)))"#;
        let bytes = wat::parse_str(watsrc).unwrap();
        let err = load_template(&bytes).unwrap_err();
        assert!(err.to_string().contains("exceeds ceiling"));
    }
```

- [ ] **Run them (expect PASS):**
```
cargo test -p digstore-compiler template::tests
```
Expected: `test result: ok. 3 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/template.rs
git commit -m "test(compiler): reject missing exports and over-ceiling memory (5.1)"
```

---

## Task 10 — Inject the data section: whole-section passthrough + memory bump

**Files:**
- Modify: `crates/digstore-compiler/src/inject.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: `crates/digstore-compiler/tests/inject.rs`

> **Fixes the code-section passthrough blocker:** we do NOT iterate `Parser::parse_all` for passthrough (which would split the code section into per-body `CodeSectionEntry` payloads). Instead we scan TOP-LEVEL sections directly with `Parser::parse` over the whole buffer, copying each section (id + payload bytes) verbatim, replacing only the `Data`/`DataCount` sections. This guarantees the code section and every body are copied byte-for-byte.
>
> **Memory bump (fixes the offset/overflow blocker):** the blob is placed at `DATA_SECTION_MEM_OFFSET` (65536 = page 1, the reserved region declared in the template). Injection recomputes the required min pages = `ceil((offset + blob.len()) / 65536)`, and if the template's declared min is smaller, rewrites the `MemorySection` min so the active segment is always in bounds at instantiation. The constant is the single source of truth shared with the guest (documented in `src/lib.rs`).

Steps:

- [ ] **Write the whole-section byte-identity test — red first.** Create `crates/digstore-compiler/tests/inject.rs`:
```rust
use digstore_compiler::{baked_template_bytes, inject_data_section, load_template, REQUIRED_EXPORTS};
use wasmparser::{Parser, Payload, Validator, WasmFeatures};

/// Collect (section_id, payload_bytes) for every TOP-LEVEL section, in order,
/// EXCLUDING data + data-count sections (ids 11, 12).
fn non_data_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.unwrap() {
            Payload::DataSection(_) | Payload::DataCountSection { .. } => {}
            Payload::CodeSectionStart { range, .. } => {
                // Whole code section payload range.
                out.push((10u8, bytes[range].to_vec()));
            }
            Payload::TypeSection(r) => out.push((1, bytes[r.range()].to_vec())),
            Payload::ImportSection(r) => out.push((2, bytes[r.range()].to_vec())),
            Payload::FunctionSection(r) => out.push((3, bytes[r.range()].to_vec())),
            Payload::TableSection(r) => out.push((4, bytes[r.range()].to_vec())),
            Payload::MemorySection(r) => out.push((5, bytes[r.range()].to_vec())),
            Payload::GlobalSection(r) => out.push((6, bytes[r.range()].to_vec())),
            Payload::ExportSection(r) => out.push((7, bytes[r.range()].to_vec())),
            Payload::ElementSection(r) => out.push((9, bytes[r.range()].to_vec())),
            _ => {}
        }
    }
    out
}

#[test]
fn non_data_sections_are_byte_identical_after_injection() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0xEEu8; 256];
    // Inject at the reserved offset; template min memory (4 pages) already fits.
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    // Memory section MAY change (min bump), so exclude id 5 from byte-identity.
    let before: Vec<_> = non_data_sections(&template).into_iter().filter(|(id, _)| *id != 5).collect();
    let after: Vec<_> = non_data_sections(&out).into_iter().filter(|(id, _)| *id != 5).collect();
    assert_eq!(before, after, "non-Data, non-Memory sections must be byte-identical");
}

#[test]
fn injected_module_is_valid_wasm() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0x01u8; 64];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let mut validator = Validator::new_with_features(WasmFeatures::default());
    validator.validate_all(&out).expect("module validates");
}

#[test]
fn injected_module_still_exports_full_abi() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0x01u8; 64];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let t = load_template(&out).expect("re-parse ok");
    for name in REQUIRED_EXPORTS {
        assert!(t.has_export(name), "lost export {name}");
    }
}

#[test]
fn injected_data_blob_is_present_in_data_section() {
    let template = baked_template_bytes().to_vec();
    let blob = vec![0xABu8; 32];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");
    let mut found = false;
    for payload in Parser::new(0).parse_all(&out) {
        if let Payload::DataSection(reader) = payload.unwrap() {
            for seg in reader {
                if seg.unwrap().data == blob.as_slice() {
                    found = true;
                }
            }
        }
    }
    assert!(found, "injected blob not found in data section");
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler --test inject non_data_sections_are_byte_identical_after_injection
```
Expected: `error[E0432]: unresolved import digstore_compiler::inject_data_section`.

- [ ] **Implement `src/inject.rs`** (whole-section scan + memory bump; single concrete impl, no fallback prose):
```rust
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
            Payload::TypeSection(r) => module.section(&RawSection { id: 1, data: &template[r.range()] }),
            Payload::ImportSection(r) => module.section(&RawSection { id: 2, data: &template[r.range()] }),
            Payload::FunctionSection(r) => module.section(&RawSection { id: 3, data: &template[r.range()] }),
            Payload::TableSection(r) => module.section(&RawSection { id: 4, data: &template[r.range()] }),
            Payload::GlobalSection(r) => module.section(&RawSection { id: 6, data: &template[r.range()] }),
            Payload::ExportSection(r) => module.section(&RawSection { id: 7, data: &template[r.range()] }),
            Payload::StartSection { range, .. } => module.section(&RawSection { id: 8, data: &template[range] }),
            Payload::ElementSection(r) => module.section(&RawSection { id: 9, data: &template[r.range()] }),
            Payload::CustomSection(r) => module.section(&RawSection { id: 0, data: &template[r.range()] }),
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
```

> Worker note: `wasm-encoder`/`wasmparser` are pinned to `=0.221.0` in `Cargo.toml`. In 0.221, `wasmparser` section readers expose `.range()` returning the section PAYLOAD byte range (after id + size LEB), and `CodeSectionStart{ range }` spans the entire code-section payload (count + all bodies). `wasm_encoder::RawSection { id, data }` writes `id || LEB(data.len()) || data`, so copying the payload range round-trips the section exactly. `MemoryType` in 0.221 has fields `minimum`, `maximum`, `memory64`, `shared`, `page_size_log2`. These are the concrete, compilable APIs for the pinned versions; there is no conditional fallback.

- [ ] **Re-export in `src/lib.rs`:** add `pub use inject::inject_data_section;`.

- [ ] **Run the byte-identity test (expect PASS):**
```
cargo test -p digstore-compiler --test inject non_data_sections_are_byte_identical_after_injection
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Run the remaining inject tests (expect PASS):**
```
cargo test -p digstore-compiler --test inject
```
Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/inject.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/tests/inject.rs
git commit -m "feat(compiler): inject data section via whole-section passthrough with memory bump"
```

- [ ] **Add the memory-bump test for a large blob — red first.** Append to `tests/inject.rs`:
```rust
#[test]
fn large_blob_bumps_memory_min_pages_and_stays_valid() {
    let template = baked_template_bytes().to_vec();
    // Offset 65536 + 1 MiB blob => needs ceil((65536+1048576)/65536) = 18 pages.
    let blob = vec![0x5Au8; 1024 * 1024];
    let out = inject_data_section(&template, &blob, 65536).expect("inject ok");

    let mut validator = Validator::new_with_features(WasmFeatures::default());
    validator.validate_all(&out).expect("validates");

    // Re-parse and assert the declared min grew to >= 18 pages.
    let mut min_pages = 0u64;
    for payload in Parser::new(0).parse_all(&out) {
        if let Payload::MemorySection(reader) = payload.unwrap() {
            for m in reader {
                min_pages = m.unwrap().initial;
            }
        }
    }
    assert!(min_pages >= 18, "memory min pages not bumped, got {min_pages}");
}
```

- [ ] **Run it (expect PASS):**
```
cargo test -p digstore-compiler --test inject large_blob_bumps_memory_min_pages_and_stays_valid
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/inject.rs
git commit -m "test(compiler): injection bumps memory min pages to fit large blob (5.1)"
```

---

## Task 11 — Atomic write with exact output filename

**Files:**
- Modify: `crates/digstore-compiler/src/atomic_write.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/atomic_write.rs`

Steps:

- [ ] **Write the filename test — red first.** Add to the bottom of `src/atomic_write.rs` (replacing the stub):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    #[test]
    fn output_filename_is_hex_store_dash_hex_root_dot_wasm() {
        let sid = Bytes32([0xAB; 32]);
        let root = Bytes32([0x01; 32]);
        let name = output_filename(&sid, &root);
        assert_eq!(
            name,
            "abababababababababababababababababababababababababababababababab-\
0101010101010101010101010101010101010101010101010101010101010101.wasm"
        );
    }
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler atomic_write::tests::output_filename_is_hex_store_dash_hex_root_dot_wasm
```
Expected: `error[E0425]: cannot find function output_filename in this scope`.

- [ ] **Implement `src/atomic_write.rs`:**
```rust
use std::io::Write;
use std::path::{Path, PathBuf};

use digstore_core::Bytes32;

use crate::error::Result;

/// The exact output filename: `{hex(store_id)}-{hex(roothash)}.wasm` (§19.4).
pub fn output_filename(store_id: &Bytes32, roothash: &Bytes32) -> String {
    format!("{}-{}.wasm", hex::encode(store_id.0), hex::encode(roothash.0))
}

/// Write `bytes` atomically: write to `<final>.tmp` in the same directory, flush +
/// sync, then rename over the final path (§19.4).
pub fn atomic_write_module(
    dir: &Path,
    store_id: &Bytes32,
    roothash: &Bytes32,
    bytes: &[u8],
) -> Result<PathBuf> {
    let final_path = dir.join(output_filename(store_id, roothash));
    let tmp_path = dir.join(format!("{}.tmp", output_filename(store_id, roothash)));
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}
```

- [ ] **Re-export in `src/lib.rs`:** add `pub use atomic_write::{atomic_write_module, output_filename};`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler atomic_write::tests::output_filename_is_hex_store_dash_hex_root_dot_wasm
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/atomic_write.rs crates/digstore-compiler/src/lib.rs
git commit -m "feat(compiler): exact output filename helper"
```

- [ ] **Add the atomic-write behavior test — red first.** Append inside `atomic_write::tests`:
```rust
    #[test]
    fn atomic_write_creates_final_file_with_contents_and_no_temp_leftover() {
        let dir = std::env::temp_dir().join(format!("digc-aw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sid = Bytes32([1; 32]);
        let root = Bytes32([2; 32]);
        let bytes = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
        let path = atomic_write_module(&dir, &sid, &root, &bytes).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            output_filename(&sid, &root)
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file not renamed away");
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Run the module (expect PASS):**
```
cargo test -p digstore-compiler atomic_write::tests
```
Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/atomic_write.rs
git commit -m "test(compiler): atomic temp-then-rename write leaves no temp file"
```

---

## Task 12 — Deterministic obfuscation pass (§17.1)

**Files:**
- Modify: `crates/digstore-compiler/src/obfuscate.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/obfuscate.rs`

> **§17.1 scope & guarantees:** obfuscation is WASM-level, OPTIONAL, MUST be deterministic, MUST keep the module valid, and MUST preserve export behavior. We achieve all three by copying every existing section VERBATIM (whole-section passthrough, like injection) and APPENDING a deterministic custom section marker (the placeholder for a future opaque-predicate / bogus-code table). Custom sections have no execution semantics, so reachable code is byte-identical and behavior is provably preserved. The wasmtime equivalence harness (Task 15) double-checks export output. Security never rests on obfuscation.

Steps:

- [ ] **Write the validity test — red first.** Add to the bottom of `src/obfuscate.rs` (replacing the stub):
```rust
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
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler obfuscate::tests::obfuscated_module_is_valid_wasm
```
Expected: `error[E0425]: cannot find function obfuscate in this scope`.

- [ ] **Implement `src/obfuscate.rs`:**
```rust
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
                module.section(&RawSection { id: 10, data: &module_bytes[range] });
            }
            Payload::CodeSectionEntry(_) => {} // part of the code-section range above
            Payload::TypeSection(r) => module.section(&RawSection { id: 1, data: &module_bytes[r.range()] }),
            Payload::ImportSection(r) => module.section(&RawSection { id: 2, data: &module_bytes[r.range()] }),
            Payload::FunctionSection(r) => module.section(&RawSection { id: 3, data: &module_bytes[r.range()] }),
            Payload::TableSection(r) => module.section(&RawSection { id: 4, data: &module_bytes[r.range()] }),
            Payload::MemorySection(r) => module.section(&RawSection { id: 5, data: &module_bytes[r.range()] }),
            Payload::GlobalSection(r) => module.section(&RawSection { id: 6, data: &module_bytes[r.range()] }),
            Payload::ExportSection(r) => module.section(&RawSection { id: 7, data: &module_bytes[r.range()] }),
            Payload::StartSection { range, .. } => module.section(&RawSection { id: 8, data: &module_bytes[range] }),
            Payload::ElementSection(r) => module.section(&RawSection { id: 9, data: &module_bytes[r.range()] }),
            Payload::DataCountSection { range, .. } => module.section(&RawSection { id: 12, data: &module_bytes[range] }),
            Payload::DataSection(r) => module.section(&RawSection { id: 11, data: &module_bytes[r.range()] }),
            Payload::CustomSection(r) => module.section(&RawSection { id: 0, data: &module_bytes[r.range()] }),
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
```

- [ ] **Re-export in `src/lib.rs`:** add `pub use obfuscate::obfuscate;`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler obfuscate::tests::obfuscated_module_is_valid_wasm
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/obfuscate.rs crates/digstore-compiler/src/lib.rs
git commit -m "feat(compiler): deterministic behavior-preserving obfuscation pass"
```

- [ ] **Add determinism + changes-bytes + exports-preserved behaviors — red first.** Append inside `obfuscate::tests`:
```rust
    #[test]
    fn obfuscation_is_deterministic() {
        let m = template();
        let a = obfuscate(&m).expect("a");
        let b = obfuscate(&m).expect("b");
        assert_eq!(a, b, "obfuscation must be byte-identical for identical input");
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
```

- [ ] **Run the module (expect PASS):**
```
cargo test -p digstore-compiler obfuscate::tests
```
Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/obfuscate.rs
git commit -m "test(compiler): obfuscation deterministic, mutating, export-preserving"
```

---

## Task 13 — Pipeline orchestration + `CompilationResult` + `NoTrustedKeys`

**Files:**
- Modify: `crates/digstore-compiler/src/pipeline.rs` (replace stub)
- Modify: `crates/digstore-compiler/src/lib.rs`
- Test: `crates/digstore-compiler/tests/pipeline.rs`

> Orchestrates the 10 stages (§5.3). Consumes `GenerationView`s + manifest + trusted keys + store identity, producing `CompilationResult { store_id, roothash, output_path, output_size, stats }`. Refuses an empty trusted set with `CompilerError::NoTrustedKeys`. The `roothash` is the **current** (last) generation's root. The manifest is encoded via core `Encode` (NOT JSON). The data blob is placed at the reserved offset and the key-table integrity is verified against the chunk index.

Steps:

- [ ] **Write the NoTrustedKeys test — red first.** Create `crates/digstore-compiler/tests/pipeline.rs`:
```rust
mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{Compiler, CompilerConfig, CompilerError};

fn cfg(dir: &std::path::Path) -> CompilerConfig {
    CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate: false,
        optimize: false,
        template_override: None,
    }
}

#[test]
fn empty_trusted_set_is_refused() {
    let dir = std::env::temp_dir();
    let gens = sample_generations();
    let err = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        &[],
    )
    .unwrap_err();
    assert!(matches!(err, CompilerError::NoTrustedKeys));
}
```

- [ ] **Run it (expect FAIL):**
```
cargo test -p digstore-compiler --test pipeline empty_trusted_set_is_refused
```
Expected: `error[E0432]: unresolved import digstore_compiler::Compiler`.

- [ ] **Implement `src/pipeline.rs`:**
```rust
use std::path::PathBuf;

use digstore_core::{Bytes32, Bytes48, MetadataManifest, TrustedHostKey};

use crate::atomic_write::atomic_write_module;
use crate::config::{CompilationStats, CompilerConfig};
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

/// Result of a successful compilation (§5.3, §19.4).
#[derive(Debug, Clone)]
pub struct CompilationResult {
    pub store_id: Bytes32,
    pub roothash: Bytes32,
    pub output_path: PathBuf,
    pub output_size: u64,
    pub stats: CompilationStats,
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
    ) -> Result<CompilationResult> {
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
        let mut module =
            inject_data_section(&template_bytes, &data_blob, DATA_SECTION_MEM_OFFSET)?;

        // Stage 7: optional obfuscation (deterministic).
        let obfuscation_applied = config.obfuscate;
        if obfuscation_applied {
            module = obfuscate(&module)?;
        }

        // Stage 8 (wasm-opt) intentionally skipped for determinism portability.
        // Stage 9: final validate (re-parse exports + memory bounds).
        load_template(&module)?;

        // Stage 10: atomic write.
        let output_path =
            atomic_write_module(&config.output_dir, &store_id, &roothash, &module)?;
        let output_size = std::fs::metadata(&output_path)?.len();

        let stats = CompilationStats {
            generation_count: generations.len() as u32,
            unique_chunk_count: chunk_index.len() as u32,
            resource_count: key_table.entries().len() as u32,
            pool_byte_len: pool.bytes.len() as u64,
            data_section_byte_len: data_blob.len() as u64,
            obfuscation_applied,
        };

        Ok(CompilationResult {
            store_id,
            roothash,
            output_path,
            output_size,
            stats,
        })
    }
}
```

- [ ] **Re-export in `src/lib.rs`:** add `pub use pipeline::{CompilationResult, Compiler, DATA_SECTION_MEM_OFFSET};`.

- [ ] **Run test (expect PASS):**
```
cargo test -p digstore-compiler --test pipeline empty_trusted_set_is_refused
```
Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/pipeline.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/tests/pipeline.rs
git commit -m "feat(compiler): pipeline orchestration with NoTrustedKeys precondition"
```

- [ ] **Add the result-fields test — red first.** Append to `tests/pipeline.rs` (note: `GenerationView` is imported to call `.root()` unambiguously; NO local `RootAccess` trait):
```rust
use digstore_compiler::GenerationView;

#[test]
fn produces_result_with_exact_filename_and_stats() {
    let dir = std::env::temp_dir().join(format!("digc-pipe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gens = sample_generations();
    let last_root = GenerationView::root(gens.last().unwrap());

    let result = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        &trusted_keys(),
    )
    .expect("compiles");

    assert_eq!(result.store_id, store_id());
    assert_eq!(result.roothash, last_root);
    let expected_name = format!(
        "{}-{}.wasm",
        hex::encode(store_id().0),
        hex::encode(last_root.0)
    );
    assert_eq!(
        result.output_path.file_name().unwrap().to_str().unwrap(),
        expected_name
    );
    assert!(result.output_path.exists());
    assert_eq!(
        result.output_size,
        std::fs::metadata(&result.output_path).unwrap().len()
    );
    assert_eq!(result.stats.generation_count, 2);
    assert_eq!(result.stats.unique_chunk_count, 3);
    assert_eq!(result.stats.resource_count, 2);
    assert!(!result.stats.obfuscation_applied);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn obfuscation_flag_sets_stat_and_still_writes_valid_module() {
    use wasmparser::{Validator, WasmFeatures};
    let dir = std::env::temp_dir().join(format!("digc-obf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = cfg(&dir);
    c.obfuscate = true;
    let gens = sample_generations();
    let result = Compiler::compile(
        &c,
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        &trusted_keys(),
    )
    .expect("compiles");
    assert!(result.stats.obfuscation_applied);
    assert!(result.output_path.exists());
    let bytes = std::fs::read(&result.output_path).unwrap();
    let mut v = Validator::new_with_features(WasmFeatures::default());
    v.validate_all(&bytes).expect("obfuscated module validates");
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Add `wasmparser` to `[dev-dependencies]`** in `Cargo.toml` (needed by the pipeline test's validator):
```toml
wasmparser = "=0.221.0"
```

- [ ] **Run both (expect PASS):**
```
cargo test -p digstore-compiler --test pipeline produces_result_with_exact_filename_and_stats obfuscation_flag_sets_stat_and_still_writes_valid_module
```
Expected: `test result: ok. 2 passed; 0 failed` (run alongside the existing `empty_trusted_set_is_refused`, the file has 3 tests so far).

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/pipeline.rs crates/digstore-compiler/Cargo.toml
git commit -m "feat(compiler): CompilationResult fields and obfuscation flag wiring"
```

---

## Task 14 — §19.3 double-compile determinism (plain + obfuscated)

**Files:**
- Create: `crates/digstore-compiler/tests/determinism.rs`

Steps:

- [ ] **Write the determinism tests.** Create `crates/digstore-compiler/tests/determinism.rs`:
```rust
mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{Compiler, CompilerConfig};

fn compile_to_bytes(dir: &std::path::Path, obfuscate: bool) -> Vec<u8> {
    let cfg = CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate,
        optimize: false,
        template_override: None,
    };
    let gens = sample_generations();
    let r = Compiler::compile(
        &cfg,
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        &trusted_keys(),
    )
    .expect("compiles");
    std::fs::read(&r.output_path).unwrap()
}

#[test]
fn two_compiles_are_byte_identical() {
    let d1 = std::env::temp_dir().join(format!("digc-det1-{}", std::process::id()));
    let d2 = std::env::temp_dir().join(format!("digc-det2-{}", std::process::id()));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();

    let a = compile_to_bytes(&d1, false);
    let b = compile_to_bytes(&d2, false);
    assert_eq!(a, b, "compilation must be byte-identical (paper 19.3)");

    std::fs::remove_dir_all(&d1).ok();
    std::fs::remove_dir_all(&d2).ok();
}

#[test]
fn two_obfuscated_compiles_are_byte_identical() {
    let d1 = std::env::temp_dir().join(format!("digc-detob1-{}", std::process::id()));
    let d2 = std::env::temp_dir().join(format!("digc-detob2-{}", std::process::id()));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();

    let a = compile_to_bytes(&d1, true);
    let b = compile_to_bytes(&d2, true);
    assert_eq!(a, b, "obfuscated compilation must also be byte-identical");

    std::fs::remove_dir_all(&d1).ok();
    std::fs::remove_dir_all(&d2).ok();
}
```

- [ ] **Run it (expect PASS — determinism already engineered via fixed template + deterministic filler + insertion-ordered structures):**
```
cargo test -p digstore-compiler --test determinism
```
Expected: `test result: ok. 2 passed; 0 failed`. If it FAILS, the failure localizes a nondeterminism source (e.g., a `HashMap` iteration leaking into output order); fix by making that ordering insertion-based, then re-run. Do not weaken the assertion.

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/determinism.rs
git commit -m "test(compiler): double-compile byte-identical determinism (paper 19.3)"
```

---

## Task 15 — Obfuscation behavior-equivalence harness (wasmtime)

**Files:**
- Create: `crates/digstore-compiler/tests/obfuscation.rs`

> Compiles the SAME inputs twice — once `obfuscate=false`, once `obfuscate=true` — instantiates BOTH modules in wasmtime, and asserts `get_store_id` produces identical packed results. The pinned template's exports are import-free, so both arms instantiate and return identical values, proving obfuscation preserved behavior (§17.1). If a future real guest declares host imports, the harness instead asserts both modules validate (covered end-to-end by the host crate).

Steps:

- [ ] **Write the equivalence test.** Create `crates/digstore-compiler/tests/obfuscation.rs`:
```rust
mod common;

use common::{sample_generations, sample_manifest, store_id, store_pubkey, trusted_keys};
use digstore_compiler::{load_template, Compiler, CompilerConfig};
use wasmtime::{Engine, Instance, Module, Store};

fn compile(dir: &std::path::Path, obfuscate: bool) -> Vec<u8> {
    let cfg = CompilerConfig {
        output_dir: dir.to_path_buf(),
        obfuscate,
        optimize: false,
        template_override: None,
    };
    let gens = sample_generations();
    let r = Compiler::compile(
        &cfg,
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        &trusted_keys(),
    )
    .unwrap();
    std::fs::read(&r.output_path).unwrap()
}

/// Instantiate an import-free module and call `get_store_id`. Returns None when
/// the module declares host imports (the real guest), in which case the caller
/// falls back to a validity check.
fn call_get_store_id(bytes: &[u8]) -> Option<i64> {
    let engine = Engine::default();
    let module = Module::new(&engine, bytes).expect("module");
    if module.imports().count() != 0 {
        return None;
    }
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).ok()?;
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "get_store_id")
        .ok()?;
    f.call(&mut store, ()).ok()
}

#[test]
fn obfuscated_and_plain_modules_produce_same_export_output() {
    let d1 = std::env::temp_dir().join(format!("digc-eqp-{}", std::process::id()));
    let d2 = std::env::temp_dir().join(format!("digc-eqo-{}", std::process::id()));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();

    let plain = compile(&d1, false);
    let obf = compile(&d2, true);

    let plain_out = call_get_store_id(&plain);
    let obf_out = call_get_store_id(&obf);

    match (plain_out, obf_out) {
        (Some(a), Some(b)) => assert_eq!(a, b, "obfuscation changed export behavior"),
        _ => {
            load_template(&plain).expect("plain valid");
            load_template(&obf).expect("obf valid");
        }
    }

    std::fs::remove_dir_all(&d1).ok();
    std::fs::remove_dir_all(&d2).ok();
}
```

- [ ] **Run it (expect PASS):**
```
cargo test -p digstore-compiler --test obfuscation
```
Expected: `test result: ok. 1 passed; 0 failed`. (With the pinned import-free template both arms return `Some(0)` and assert equal.)

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/obfuscation.rs
git commit -m "test(compiler): wasmtime harness proves obfuscation preserves export behavior"
```

---

## Task 16 — Pool indistinguishability (bucketed length) end-to-end

**Files:**
- Modify: `crates/digstore-compiler/tests/pipeline.rs` (add tests)

> §8.3/§15.2: module size must leak only a coarse bucket. Verify (a) two content sizes in the same bucket round to the same pool length, and (b) the pipeline records the bucketed pool length. The fixture's content is `shared(22) + alpha(15) + beta(15) = 52` bytes → bucket 64.

Steps:

- [ ] **Add the bucket-independence test — red first.** Append to `crates/digstore-compiler/tests/pipeline.rs`:
```rust
#[test]
fn pool_length_is_bucketed_and_independent_of_exact_content_size() {
    use digstore_compiler::next_pool_bucket;
    // Two content sizes in the same bucket round to the same pool length.
    assert_eq!(next_pool_bucket(30), next_pool_bucket(50)); // both 64
    assert_eq!(next_pool_bucket(70), next_pool_bucket(120)); // both 128

    // The pipeline records the bucketed pool length in stats.
    let dir = std::env::temp_dir().join(format!("digc-buck-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gens = sample_generations();
    let r = Compiler::compile(
        &cfg(&dir),
        store_id(),
        store_pubkey(),
        &gens,
        sample_manifest(),
        &trusted_keys(),
    )
    .unwrap();
    // shared(22) + alpha(15) + beta(15) = 52 content bytes -> bucket 64.
    assert_eq!(r.stats.pool_byte_len, 64);
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Run it (expect PASS):**
```
cargo test -p digstore-compiler --test pipeline pool_length_is_bucketed_and_independent_of_exact_content_size
```
Expected: `test result: ok. 1 passed; 0 failed`. If the printed `pool_byte_len` differs (fixture byte counts changed), set the literal to the printed value (do NOT change `next_pool_bucket`).

- [ ] **Run the FULL pipeline binary to reconcile its total count (now 4 tests):**
```
cargo test -p digstore-compiler --test pipeline
```
Expected: `test result: ok. 4 passed; 0 failed` (`empty_trusted_set_is_refused`, `produces_result_with_exact_filename_and_stats`, `obfuscation_flag_sets_stat_and_still_writes_valid_module`, `pool_length_is_bucketed_and_independent_of_exact_content_size`).

- [ ] **Commit:**
```
git add crates/digstore-compiler/tests/pipeline.rs
git commit -m "test(compiler): pool length bucketed, hides exact content size (8.3)"
```

---

## Task 17 — Full suite green + crate-level deviation docs

**Files:**
- Modify: `crates/digstore-compiler/src/lib.rs` (crate doc summarizing deviations + shared offset)

Steps:

- [ ] **Add the deviations + shared-offset note** to the top of `src/lib.rs` (immediately below the existing first `//!` line):
```rust
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
//!
//! ## Shared constant
//! [`DATA_SECTION_MEM_OFFSET`] (65536 = page 1) is the agreed offset where the
//! data section is placed; the `digstore-guest` crate reads from the same offset.
```

- [ ] **Run the whole crate suite (expect ALL PASS):**
```
cargo test -p digstore-compiler
```
Expected: every binary reports `ok`. Per-binary totals: `chunk_index` 4, `key_table` 5 (fixtures_build + entries + total_size + lookup + verify), `filler` 5, `data_section_golden` 2, `inject` 5, `pipeline` 4, `determinism` 2, `obfuscation` 1; inline modules: `pool::tests` 4, `pool::codec_tests` 1, `data_section::tests` 6, `template::tests` 3, `atomic_write::tests` 2, `obfuscate::tests` 4, `error::tests` 2.

- [ ] **Confirm a clean build with no warnings (warnings treated as failures per plan policy):**
```
cargo build -p digstore-compiler --all-targets
```
Expected: `Finished` with no warning lines. If any `unused import`/`dead_code` warning appears in a test or module, remove the offending import or add `#[allow(...)]` only where the item is intentionally part of the public surface.

- [ ] **Commit:**
```
git add crates/digstore-compiler/src/lib.rs
git commit -m "docs(compiler): record codec/filler/obfuscation/template deviations and shared offset"
```

---

## Definition of Done

- [ ] **§5.1 (Module sections / memory bounds):** `load_template` parses the `MemorySection`, asserts a memory exists and `max <= 256` pages; injection bumps memory min pages to fit the blob and re-validates; whole-section passthrough preserves the section structure byte-for-byte (Tasks 9, 10).
- [ ] **§5.2 (What gets embedded):** `ChunkIndex` dedup, `KeyTable` build, store-header + manifest + trusted-keys segments embedded via canonical `Encode` (Tasks 2, 4, 7, 13).
- [ ] **§5.3 (Compilation pipeline, 10 stages):** `Compiler::compile` runs config→generation→dedup→key-table(+integrity)→template load/validate→data embed→data inject→obfuscation→(wasm-opt skip)→final validate→atomic write; `CompilationResult` populated; `CompilerError::NoTrustedKeys` enforced (Tasks 1, 11, 13).
- [ ] **§8.3 (Interleaved pool):** single pool, no resource boundaries, deterministic filler in gaps, bucketed length proven independent of exact content size (Tasks 5, 6, 16).
- [ ] **§17.1 (Obfuscation):** optional, deterministic, valid, behavior-preserving; proven via wasmtime equivalence harness (Tasks 12, 15).
- [ ] **§19.1 (Inputs):** pipeline consumes store id/config, generations (via `GenerationView`), trusted keys, manifest, compiler options (Tasks 1, 13).
- [ ] **§19.2 (Trusted host keys):** `TrustedHostKey` list embedded as a segment via core `Encode`; empty set refused (Tasks 7, 13).
- [ ] **§19.3 (Determinism):** double-compile byte-identical for plain and obfuscated builds (Task 14); filler determinism (Task 5); pinned template fixture (Task 9); independently-validated golden data-section vector (Task 8).
- [ ] **§19.4 (Output):** atomic temp-then-rename write to exact `{hex(store_id)}-{hex(roothash)}.wasm`; reports path + size + stats (Tasks 11, 13).
- [ ] **Deviations documented** in crate docs (#1 big-endian core codec, #2 deterministic filler, obfuscation scope, pinned template) (Task 17).
- [ ] `cargo test -p digstore-compiler` and `cargo build -p digstore-compiler --all-targets` fully green with no warnings (Task 17).


---

## Plan metadata

- **Crate:** digstore-compiler
- **Assigned paper sections:** 5.1,5.2,5.3,8.3,17.1,19.1,19.2,19.3,19.4
- **Depends on:** digstore-core, digstore-store, digstore-guest
- **Spec sections covered (claimed):** 5.1, 5.2, 5.3, 8.3, 17.1, 19.1, 19.2, 19.3, 19.4

### Public items exported (consumed by other crates)

```
pub struct CompilerConfig { pub output_dir: PathBuf, pub obfuscate: bool, pub optimize: bool, pub template_override: Option<Vec<u8>> }
impl Default for CompilerConfig
pub struct CompilationStats { pub generation_count: u32, pub unique_chunk_count: u32, pub resource_count: u32, pub pool_byte_len: u64, pub data_section_byte_len: u64, pub obfuscation_applied: bool }
pub enum CompilerError { NoTrustedKeys, InvalidTemplate(String), GenerationLoad(String), Validation(String), MissingChunk(u32), Io(std::io::Error) }
pub type Result<T> = core::result::Result<T, CompilerError>
pub struct ChunkIndex
impl ChunkIndex { pub fn new() -> Self; pub fn insert(&mut self, hash: digstore_core::Bytes32, body: Vec<u8>) -> u32; pub fn index_of(&self, hash: &digstore_core::Bytes32) -> Option<u32>; pub fn len(&self) -> usize; pub fn is_empty(&self) -> bool; pub fn bodies_in_order(&self) -> impl Iterator<Item = &[u8]> }
pub trait ResourceView { fn resource_key(&self) -> digstore_core::Bytes32; fn chunks(&self) -> Vec<(digstore_core::Bytes32, Vec<u8>)>; }
pub trait GenerationView { fn root(&self) -> digstore_core::Bytes32; fn resources(&self) -> Vec<Box<dyn ResourceView + '_>>; }
pub struct KeyTable
impl KeyTable { pub fn entries(&self) -> &[digstore_core::KeyTableEntry]; pub fn lookup(&self, rk: &digstore_core::Bytes32) -> Option<&digstore_core::KeyTableEntry>; pub fn verify_against(&self, chunk_count: u32) -> Result<()> }
pub fn build_chunk_index_and_key_table<G: GenerationView>(generations: &[G]) -> (ChunkIndex, KeyTable)
pub fn deterministic_filler(store_id: &digstore_core::Bytes32, roothash: &digstore_core::Bytes32, len: usize) -> Vec<u8>
pub struct ChunkLoc { pub offset: u32, pub len: u32 }
impl digstore_core::Encode for ChunkLoc
impl digstore_core::Decode for ChunkLoc
pub struct InterleavedPool { pub bytes: Vec<u8>, pub descriptors: Vec<ChunkLoc> }
pub fn next_pool_bucket(n: usize) -> usize
pub fn build_pool(store_id: &digstore_core::Bytes32, roothash: &digstore_core::Bytes32, bodies: &[Vec<u8>]) -> InterleavedPool
pub const MAGIC: &[u8; 4] = b"DIGS"
pub const FORMAT_VERSION: u8 = 1
pub const SEG_POOL: u8 = 0
pub const SEG_KEY_TABLE: u8 = 1
pub const SEG_STORE_HEADER: u8 = 2
pub const SEG_MANIFEST: u8 = 3
pub const SEG_TRUSTED_KEYS: u8 = 4
pub struct SectionEntry { pub kind: u8, pub offset: u32, pub len: u32 }
pub struct DataSectionInputs { pub store_id: digstore_core::Bytes32, pub roothash: digstore_core::Bytes32, pub root_history: Vec<digstore_core::Bytes32>, pub store_pubkey: digstore_core::Bytes48, pub pool_bytes: Vec<u8>, pub pool_descriptors: Vec<ChunkLoc>, pub key_table: Vec<digstore_core::KeyTableEntry>, pub manifest: digstore_core::MetadataManifest, pub trusted_keys: Vec<digstore_core::TrustedHostKey> }
pub fn encode_data_section(i: &DataSectionInputs) -> Vec<u8>
pub fn parse_offset_table(blob: &[u8]) -> Result<Vec<SectionEntry>>
pub fn decode_store_header(body: &[u8]) -> Result<(digstore_core::Bytes32, digstore_core::Bytes32, Vec<digstore_core::Bytes32>, digstore_core::Bytes48)>
pub const MAX_MEMORY_PAGES: u64 = 256
pub const REQUIRED_EXPORTS: &[&str]
pub struct Template { pub bytes: Vec<u8>, pub memory_min_pages: u64, pub memory_max_pages: Option<u64> }
impl Template { pub fn has_export(&self, name: &str) -> bool }
pub fn baked_template_bytes() -> &'static [u8]
pub fn load_template(bytes: &[u8]) -> Result<Template>
pub fn inject_data_section(template: &[u8], blob: &[u8], mem_offset: u32) -> Result<Vec<u8>>
pub fn obfuscate(module_bytes: &[u8]) -> Result<Vec<u8>>
pub fn output_filename(store_id: &digstore_core::Bytes32, roothash: &digstore_core::Bytes32) -> String
pub fn atomic_write_module(dir: &std::path::Path, store_id: &digstore_core::Bytes32, roothash: &digstore_core::Bytes32, bytes: &[u8]) -> Result<std::path::PathBuf>
pub const DATA_SECTION_MEM_OFFSET: u32 = 65536
pub struct CompilationResult { pub store_id: digstore_core::Bytes32, pub roothash: digstore_core::Bytes32, pub output_path: std::path::PathBuf, pub output_size: u64, pub stats: CompilationStats }
pub struct Compiler
impl Compiler { pub fn compile<G: GenerationView>(config: &CompilerConfig, store_id: digstore_core::Bytes32, store_pubkey: digstore_core::Bytes48, generations: &[G], manifest: digstore_core::MetadataManifest, trusted_keys: &[digstore_core::TrustedHostKey]) -> Result<CompilationResult> }
```