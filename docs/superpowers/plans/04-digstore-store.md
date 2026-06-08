# digstore-store Implementation Plan

> **For agentic workers:** This plan is executed with the **REQUIRED SUB-SKILL `superpowers:subagent-driven-development`**. Each numbered Task is dispatched to a fresh subagent. Every step is one 2–5 minute action in strict TDD order: write a failing test (full code shown) → run it and observe the exact FAIL → write the minimal implementation (full code shown) → run the test and observe PASS → commit with the exact conventional-commit message. Do not batch steps. Do not skip the red phase. Commit after every green test. **No step ships code that depends on a prose fallback — every shown snippet compiles as written against the locked contracts below.**

**Goal:** Implement the host-side `Store` entity that owns the on-disk store layout, the binary staging area, content-defined chunking-on-commit, generation commits (global dedup + merkle + monotonic root history), config persistence, content-addressed chunk resolution, and generation diff/log.

**Architecture:** `digstore-store` is a pure host (std) crate. It wraps a `StoreConfig` and a `data_dir` and materializes the exact §4.4 directory tree under `~/.dig` (or a configurable root). `add`/`stage_file` push resource bytes into a binary staging file; `commit` chunks staged content, deduplicates chunks **globally** against all prior generations' chunk files, builds a per-generation merkle tree (via `digstore-core::MerkleTree`), produces a monotonic `GenerationState`, appends it to an append-only root history, and writes a `manifest.json` generation directory. Because dedup is global and content-addressed, a chunk's bytes physically live under the first generation that introduced them; `Store::resolve_chunk` resolves any chunk by hash across all generation dirs. All wall-clock time enters through an injectable `Clock` trait so commits are deterministic in tests. Compilation to WASM is invoked separately by `digstore-compiler` over the generation directories this crate produces.

**Tech Stack:** Rust 2021 (std); `serde` + `serde_json` (manifest.json), `toml` (config.toml), `thiserror` (error enum), `hex` (locally, for the salt), `tempfile` (dev-dependency, per-test temp dirs).

**LOCKED INTER-CRATE CONTRACTS (hard dependencies — these exact signatures are guaranteed by the depended-on crates' `public_items`; the code below calls them verbatim):**

- From `digstore-core`:
  - `Bytes32([u8; 32])` derives **`Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug`** plus `serde::{Serialize, Deserialize}` and custom-codec `Encode`/`Decode`. (Ord+Copy are required by the `BTreeSet<Bytes32>` and `.copied()`/`.difference()` uses in this crate; this is a locked requirement on the core plan, not a prose fallback.)
  - `Bytes32::from(arr: [u8; 32]) -> Bytes32` (the `From<[u8;32]>` impl).
  - `Bytes32::to_hex(&self) -> String` (lower-case, 64 chars).
  - `Bytes32::from_hex(s: &str) -> Result<Bytes32, digstore_core::HexError>`.
  - `StoreConfig { store_id: Bytes32, data_dir: String, max_size: u64, visibility: Visibility }`.
  - `enum Visibility { Public, Private(SecretSalt) }`; `SecretSalt(pub [u8; 32])`.
  - `GenerationState { id: u64, root: Bytes32, timestamp: u64 }` (Clone, PartialEq, Eq, Debug).
  - `KeyTableEntry { static_key: Bytes32, generation: Bytes32, chunk_indices: Vec<u32>, total_size: u64 }` (Clone, PartialEq, Eq, Debug, serde).
  - `Urn { chain: String, store_id: Bytes32, root_hash: Option<Bytes32>, resource_key: Option<String> }`; `Urn::canonical(&self) -> String`; `Urn::retrieval_key(&self) -> Bytes32` (= SHA-256 of the canonical string).
  - `MerkleTree`; `MerkleTree::from_leaves(leaves: &[Bytes32]) -> MerkleTree`; `MerkleTree::root(&self) -> Bytes32` (leaf = SHA-256(chunk); node = SHA-256(left‖right); odd node carried up; root = generation root).
- From `digstore-chunker`:
  - `ChunkerConfig { min_size: usize, target_size: usize, max_size: usize, mask: u64 }` (public fields; no `Default` is relied upon — this crate constructs it explicitly).
  - `fn chunk_bytes(data: &[u8], cfg: &ChunkerConfig) -> Vec<Vec<u8>>` (content-defined boundaries; concatenation of the returned chunks equals `data`).
- From `digstore-crypto`:
  - `fn sha256(data: &[u8]) -> Bytes32`.

> Section-ownership note: this crate **owns** §4.1–4.4, §8.2, §20.1–20.3. Sections it *references but does not own* (the values it produces are **consumed by `digstore-compiler`/`digstore-guest`**) are: §8.3 interleaved-pool ordering, §9.1 merkle construction (owned by `digstore-core`), §19.3 byte-identical compilation (owned by `digstore-compiler`). §20.4 `log`/`diff` are implemented here as store mechanics adjacent to §20.3.

---

## File Structure

All paths under `crates/digstore-store/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest; deps on core/chunker/crypto, serde, serde_json, toml, thiserror, hex; dev-dep tempfile. |
| `src/lib.rs` | Crate root; module declarations and public re-exports (grown one line per task — no placeholder block). |
| `src/error.rs` | `StoreError` enum (all fallible store operations) + `Result` alias. |
| `src/clock.rs` | `Clock` trait + `SystemClock` (real) + `FixedClock` (deterministic, test/inject). |
| `src/paths.rs` | `StorePaths`: pure path builder for the §4.4 layout (no I/O). |
| `src/config.rs` | `config.toml` (de)serialization for `StoreConfig`/`Visibility`; round-trip. |
| `src/staging.rs` | Binary staging file: framed (big-endian) append of `(resource_key, bytes)` records; last-write-wins read-back. |
| `src/chunkstore.rs` | Per-directory content-addressed write-once chunk store. |
| `src/generation.rs` | `GenerationManifest` (manifest.json schema) + `ChunkRef` + `KeyTableRecord` (manifest superset of canonical `KeyTableEntry`, with `to_key_table_entry`). |
| `src/history.rs` | Append-only, monotonic root-history file (`roots.log`) read/append. |
| `src/diff.rs` | `GenerationDiff` (chunk-set + resource-key diff between two generations). |
| `src/store.rs` | `Store` entity: `init`, `open`, `add`, `stage_file`, `commit`, `resolve_chunk`, `log`, `diff`, `root_history`, accessors. |
| `tests/layout.rs` | Integration test: `init`/`open` produce the exact §4.4 directory tree. |
| `tests/commit_flow.rs` | Integration test: stage → commit → dedup → resolve → log/diff → deterministic root. |

---

## Task 1 — Crate skeleton and dependencies (no placeholders)

**Files:**
- Create: `crates/digstore-store/Cargo.toml`
- Create: `crates/digstore-store/src/lib.rs`
- Create: `crates/digstore-store/src/error.rs`

This task does **not** create empty placeholder files. It creates `Cargo.toml`, a `lib.rs` that declares only the `error` module (the one real module Task 1 ships), and a real, test-backed `error.rs`. Every subsequent task adds its own `mod` + `pub use` line in the same task that creates the module's real, tested content.

Steps:

- [ ] Create `crates/digstore-store/Cargo.toml` with exactly:
```toml
[package]
name = "digstore-store"
version = "0.1.0"
edition = "2021"

[dependencies]
digstore-core = { path = "../digstore-core" }
digstore-chunker = { path = "../digstore-chunker" }
digstore-crypto = { path = "../digstore-crypto" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
thiserror = "1"
hex = "0.4"

[dev-dependencies]
tempfile = "3"
```
- [ ] Add `crates/digstore-store` to the workspace `members` list in the root `C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml` (append `"crates/digstore-store"` to the `members` array).
- [ ] Create `crates/digstore-store/src/lib.rs` with exactly the module declaration and re-export for the one module shipped in this task:
```rust
//! digstore-store: the host-side Store entity, on-disk layout, staging, and generations.
//!
//! Implements paper sections 4.1–4.4 (store structure), 8.2 (generations), and
//! 20.1–20.3 store mechanics (init / add / commit). Generation directories
//! produced here are consumed by `digstore-compiler` (which owns §8.3 pool
//! ordering and §19.3 byte-identical compilation) and `digstore-guest`.

mod error;

pub use error::{Result, StoreError};
```
- [ ] Create `crates/digstore-store/src/error.rs` with the failing test FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_already_exists_displays_path() {
        let e = StoreError::AlreadyExists("/tmp/x".into());
        assert_eq!(e.to_string(), "store already exists at /tmp/x");
    }

    #[test]
    fn io_error_wraps_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let e: StoreError = io.into();
        assert!(matches!(e, StoreError::Io(_)));
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib error::`
  Expected: FAIL — `error[E0433]: failed to resolve: use of undeclared type StoreError`.
- [ ] In `src/error.rs`, add the implementation above the test module:
```rust
use std::path::PathBuf;

/// Result alias used throughout digstore-store.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Errors produced by store operations (init/open/add/commit/diff).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store already exists at {0}")]
    AlreadyExists(String),

    #[error("store not found at {0}")]
    NotFound(String),

    #[error("invalid store configuration: {0}")]
    InvalidConfig(String),

    #[error("staging area corrupt: {0}")]
    CorruptStaging(String),

    #[error("generation {0} not found")]
    GenerationNotFound(String),

    #[error("chunk {0} not found in any generation")]
    ChunkNotFound(String),

    #[error("root history is not monotonic: generation id {got} follows {last}")]
    NonMonotonicHistory { last: u64, got: u64 },

    #[error("nothing staged to commit")]
    EmptyStaging,

    #[error("manifest parse error: {0}")]
    Manifest(String),

    #[error("config (de)serialization error: {0}")]
    Config(String),

    #[error("path is not under the staging base: {0}")]
    PathEscape(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```
- [ ] Run: `cargo test -p digstore-store --lib error::`
  Expected: PASS — `test error::tests::store_already_exists_displays_path ... ok`, `test error::tests::io_error_wraps_source ... ok`. Summary `test result: ok. 2 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`
  Expected: PASS — `Compiling digstore-store v0.1.0` then `Finished`.
- [ ] Commit:
```
git add crates/digstore-store/Cargo.toml crates/digstore-store/src/lib.rs crates/digstore-store/src/error.rs Cargo.toml
git commit -m "feat(store): scaffold crate with StoreError enum and Result alias"
```

---

## Task 2 — `Clock` trait, `SystemClock`, `FixedClock`

**Files:**
- Create: `crates/digstore-store/src/clock.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/clock.rs`

Steps:

- [ ] Create `crates/digstore-store/src/clock.rs` with the failing test FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_fixed_value() {
        let c = FixedClock::new(1_717_000_000);
        assert_eq!(c.unix_seconds(), 1_717_000_000);
        assert_eq!(c.unix_seconds(), 1_717_000_000);
    }

    #[test]
    fn fixed_clock_can_advance() {
        let c = FixedClock::new(100);
        c.advance(50);
        assert_eq!(c.unix_seconds(), 150);
    }

    #[test]
    fn system_clock_is_nonzero() {
        let c = SystemClock;
        assert!(c.unix_seconds() > 1_600_000_000);
    }
}
```
- [ ] In `src/lib.rs`, add `mod clock;` directly under `mod error;`, and add `pub use clock::{Clock, FixedClock, SystemClock};` under the existing `pub use`.
- [ ] Run: `cargo test -p digstore-store --lib clock::`
  Expected: FAIL — `cannot find type FixedClock in this scope` / `cannot find type SystemClock in this scope`.
- [ ] In `src/clock.rs`, add the implementation above the test module:
```rust
use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of wall-clock time. Injected into `Store` so commits are deterministic
/// in tests. Generation timestamps (unix seconds) come from this.
pub trait Clock {
    /// Current time in unix seconds.
    fn unix_seconds(&self) -> u64;
}

/// Real system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_secs()
    }
}

/// Deterministic clock for tests; can be advanced explicitly.
pub struct FixedClock {
    now: Cell<u64>,
}

impl FixedClock {
    pub fn new(now: u64) -> Self {
        Self { now: Cell::new(now) }
    }
    /// Move the clock forward by `delta` seconds.
    pub fn advance(&self, delta: u64) {
        self.now.set(self.now.get() + delta);
    }
}

impl Clock for FixedClock {
    fn unix_seconds(&self) -> u64 {
        self.now.get()
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib clock::`
  Expected: PASS — `test result: ok. 3 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/clock.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): add Clock trait with SystemClock and FixedClock"
```

---

## Task 3 — `StorePaths` layout builder (pure, no I/O)

**Files:**
- Create: `crates/digstore-store/src/paths.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/paths.rs`

These paths are the §4.4 layout. `StorePaths` does NOT touch the filesystem; it only computes paths so they can be asserted exactly. Uses `Bytes32::from([u8;32])` and `Bytes32::to_hex(&self) -> String` (locked contracts).

Steps:

- [ ] Create `crates/digstore-store/src/paths.rs` with the failing test FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;
    use std::path::PathBuf;

    fn sid() -> Bytes32 {
        Bytes32::from([0x11u8; 32])
    }

    #[test]
    fn root_and_top_level_files() {
        let p = StorePaths::new("/data", sid());
        assert_eq!(p.root(), PathBuf::from("/data"));
        assert_eq!(p.config_file(), PathBuf::from("/data/config.toml"));
        let hex = "11".repeat(32);
        assert_eq!(p.staging_file(), PathBuf::from(format!("/data/{hex}.staging.bin")));
    }

    #[test]
    fn generation_subtree() {
        let p = StorePaths::new("/data", sid());
        let root_hex = "ab".repeat(32);
        let gen = p.generation_dir(&root_hex);
        assert_eq!(gen, PathBuf::from(format!("/data/generations/{root_hex}")));
        assert_eq!(
            p.generation_manifest(&root_hex),
            PathBuf::from(format!("/data/generations/{root_hex}/manifest.json"))
        );
        assert_eq!(
            p.generation_chunks_dir(&root_hex),
            PathBuf::from(format!("/data/generations/{root_hex}/chunks"))
        );
        assert_eq!(
            p.chunk_file(&root_hex, "cc"),
            PathBuf::from(format!("/data/generations/{root_hex}/chunks/cc"))
        );
    }

    #[test]
    fn module_and_history_paths() {
        let p = StorePaths::new("/data", sid());
        let sid_hex = "11".repeat(32);
        let root_hex = "ab".repeat(32);
        assert_eq!(
            p.module_file(&root_hex),
            PathBuf::from(format!("/data/modules/{sid_hex}-{root_hex}.wasm"))
        );
        assert_eq!(p.generations_dir(), PathBuf::from("/data/generations"));
        assert_eq!(p.modules_dir(), PathBuf::from("/data/modules"));
        assert_eq!(p.history_file(), PathBuf::from("/data/roots.log"));
    }
}
```
- [ ] In `src/lib.rs`, add `mod paths;` and `pub use paths::StorePaths;`.
- [ ] Run: `cargo test -p digstore-store --lib paths::`
  Expected: FAIL — `cannot find type StorePaths in this scope`.
- [ ] In `src/paths.rs`, add the implementation above the test module:
```rust
use digstore_core::Bytes32;
use std::path::{Path, PathBuf};

/// Pure builder for the §4.4 on-disk layout. Performs no filesystem I/O.
///
/// ```text
/// {data_dir}/
///   {store_id_hex}.staging.bin
///   config.toml
///   roots.log                         // append-only root history
///   generations/{roothash_hex}/manifest.json
///   generations/{roothash_hex}/chunks/{chunk_hash_hex}   // sparse after dedup
///   modules/{store_id_hex}-{roothash_hex}.wasm
/// ```
#[derive(Debug, Clone)]
pub struct StorePaths {
    root: PathBuf,
    store_id_hex: String,
}

impl StorePaths {
    pub fn new(data_dir: impl AsRef<Path>, store_id: Bytes32) -> Self {
        Self {
            root: data_dir.as_ref().to_path_buf(),
            store_id_hex: store_id.to_hex(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn history_file(&self) -> PathBuf {
        self.root.join("roots.log")
    }

    pub fn staging_file(&self) -> PathBuf {
        self.root.join(format!("{}.staging.bin", self.store_id_hex))
    }

    pub fn generations_dir(&self) -> PathBuf {
        self.root.join("generations")
    }

    pub fn modules_dir(&self) -> PathBuf {
        self.root.join("modules")
    }

    pub fn generation_dir(&self, root_hex: &str) -> PathBuf {
        self.generations_dir().join(root_hex)
    }

    pub fn generation_manifest(&self, root_hex: &str) -> PathBuf {
        self.generation_dir(root_hex).join("manifest.json")
    }

    pub fn generation_chunks_dir(&self, root_hex: &str) -> PathBuf {
        self.generation_dir(root_hex).join("chunks")
    }

    pub fn chunk_file(&self, root_hex: &str, chunk_hash_hex: &str) -> PathBuf {
        self.generation_chunks_dir(root_hex).join(chunk_hash_hex)
    }

    pub fn module_file(&self, root_hex: &str) -> PathBuf {
        self.modules_dir()
            .join(format!("{}-{}.wasm", self.store_id_hex, root_hex))
    }

    pub fn store_id_hex(&self) -> &str {
        &self.store_id_hex
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib paths::`
  Expected: PASS — `test result: ok. 3 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/paths.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): add StorePaths builder for the on-disk layout (§4.4)"
```

---

## Task 4 — `config.toml` round-trip (Public and Private)

**Files:**
- Create: `crates/digstore-store/src/config.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/config.rs`

`StoreConfig`, `Visibility`, and `SecretSalt` are canonical types from `digstore-core`. This module owns a TOML-friendly mirror (`ConfigToml`) so the file stays human-readable (a 32-byte salt + tagged enum do not map cleanly to TOML). Uses `SecretSalt(pub [u8;32])` (`.0` access is part of the locked contract).

Steps:

- [ ] Create `crates/digstore-store/src/config.rs` with the failing tests FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, SecretSalt, StoreConfig, Visibility};
    use tempfile::tempdir;

    fn public_cfg() -> StoreConfig {
        StoreConfig {
            store_id: Bytes32::from([0x22u8; 32]),
            data_dir: "/data".to_string(),
            max_size: 1_000_000,
            visibility: Visibility::Public,
        }
    }

    #[test]
    fn public_config_roundtrips_through_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = public_cfg();
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.store_id, cfg.store_id);
        assert_eq!(loaded.data_dir, "/data");
        assert_eq!(loaded.max_size, 1_000_000);
        assert!(matches!(loaded.visibility, Visibility::Public));
    }

    #[test]
    fn private_config_preserves_salt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = public_cfg();
        cfg.visibility = Visibility::Private(SecretSalt([0x07u8; 32]));
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        match loaded.visibility {
            Visibility::Private(salt) => assert_eq!(salt.0, [0x07u8; 32]),
            Visibility::Public => panic!("expected private visibility"),
        }
    }

    #[test]
    fn config_toml_is_human_readable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save_config(&path, &public_cfg()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("store_id = \""));
        assert!(text.contains("visibility = \"public\""));
        assert!(text.contains(&"22".repeat(32)));
    }

    #[test]
    fn private_without_salt_is_invalid_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "store_id = \"22\"\ndata_dir = \"/data\"\nmax_size = 1\nvisibility = \"private\"\n",
        )
        .unwrap();
        let err = load_config(&path).unwrap_err();
        assert!(matches!(err, StoreError::InvalidConfig(_)));
    }
}
```
- [ ] In `src/lib.rs`, add `mod config;` and `pub use config::{load_config, save_config};`.
- [ ] Run: `cargo test -p digstore-store --lib config::`
  Expected: FAIL — `cannot find function save_config in this scope`.
- [ ] In `src/config.rs`, add the implementation above the test module:
```rust
use crate::error::{Result, StoreError};
use digstore_core::{Bytes32, SecretSalt, StoreConfig, Visibility};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// TOML-friendly mirror of `StoreConfig`. Visibility is flattened to a string
/// tag plus an optional hex salt so the file stays human-readable.
#[derive(Debug, Serialize, Deserialize)]
struct ConfigToml {
    store_id: String,
    data_dir: String,
    max_size: u64,
    visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_salt: Option<String>,
}

impl ConfigToml {
    fn from_config(cfg: &StoreConfig) -> Self {
        let (visibility, secret_salt) = match &cfg.visibility {
            Visibility::Public => ("public".to_string(), None),
            Visibility::Private(salt) => ("private".to_string(), Some(hex::encode(salt.0))),
        };
        Self {
            store_id: cfg.store_id.to_hex(),
            data_dir: cfg.data_dir.clone(),
            max_size: cfg.max_size,
            visibility,
            secret_salt,
        }
    }

    fn into_config(self) -> Result<StoreConfig> {
        let store_id = Bytes32::from_hex(&self.store_id)
            .map_err(|_| StoreError::InvalidConfig(format!("bad store_id hex: {}", self.store_id)))?;
        let visibility = match self.visibility.as_str() {
            "public" => Visibility::Public,
            "private" => {
                let salt_hex = self
                    .secret_salt
                    .ok_or_else(|| StoreError::InvalidConfig("private store missing secret_salt".into()))?;
                let bytes = hex::decode(&salt_hex)
                    .map_err(|_| StoreError::InvalidConfig("bad secret_salt hex".into()))?;
                let arr: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| StoreError::InvalidConfig("secret_salt must be 32 bytes".into()))?;
                Visibility::Private(SecretSalt(arr))
            }
            other => return Err(StoreError::InvalidConfig(format!("unknown visibility: {other}"))),
        };
        Ok(StoreConfig {
            store_id,
            data_dir: self.data_dir,
            max_size: self.max_size,
            visibility,
        })
    }
}

/// Serialize a `StoreConfig` to `config.toml` at `path`.
pub fn save_config(path: impl AsRef<Path>, cfg: &StoreConfig) -> Result<()> {
    let toml_repr = ConfigToml::from_config(cfg);
    let text = toml::to_string_pretty(&toml_repr).map_err(|e| StoreError::Config(e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Load a `StoreConfig` from a `config.toml` at `path`.
pub fn load_config(path: impl AsRef<Path>) -> Result<StoreConfig> {
    let text = std::fs::read_to_string(path)?;
    let toml_repr: ConfigToml = toml::from_str(&text).map_err(|e| StoreError::Config(e.to_string()))?;
    toml_repr.into_config()
}
```
- [ ] Run: `cargo test -p digstore-store --lib config::`
  Expected: PASS — `test result: ok. 4 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/config.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): config.toml round-trip for StoreConfig and Visibility (§4.1)"
```

---

## Task 5 — Binary staging area: framed append + read-back

**Files:**
- Create: `crates/digstore-store/src/staging.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/staging.rs`

The staging file (`{store_id}.staging.bin`) holds the latest bytes for each `resource_key` staged since the last commit. Each appended record is framed with the project's **big-endian / Chia streamable** conventions: a 4-byte BE resource-key byte-length, the UTF-8 resource key, an 8-byte BE content length, then the content bytes. Re-staging the same key appends a new record; read-back returns the last record per key (last-write-wins), preserving first-seen order.

Steps:

- [ ] Create `crates/digstore-store/src/staging.rs` with the failing tests FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_one_record_and_read_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let mut area = StagingArea::open(&path).unwrap();
        area.append("index.html", b"<html>").unwrap();

        let records = StagingArea::open(&path).unwrap().records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resource_key, "index.html");
        assert_eq!(records[0].content, b"<html>");
    }

    #[test]
    fn last_write_wins_per_key_in_first_seen_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let mut area = StagingArea::open(&path).unwrap();
        area.append("a.txt", b"old").unwrap();
        area.append("b.txt", b"bee").unwrap();
        area.append("a.txt", b"new").unwrap();

        let records = area.records().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].resource_key, "a.txt");
        assert_eq!(records[0].content, b"new");
        assert_eq!(records[1].resource_key, "b.txt");
        assert_eq!(records[1].content, b"bee");
    }

    #[test]
    fn empty_staging_reads_no_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let area = StagingArea::open(&path).unwrap();
        assert_eq!(area.records().unwrap().len(), 0);
        assert!(area.is_empty().unwrap());
    }

    #[test]
    fn clear_truncates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        let mut area = StagingArea::open(&path).unwrap();
        area.append("a.txt", b"x").unwrap();
        area.clear().unwrap();
        assert!(area.is_empty().unwrap());
    }

    #[test]
    fn truncated_frame_is_reported_corrupt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.staging.bin");
        // 4-byte length claims 10 bytes of key but none follow.
        std::fs::write(&path, 10u32.to_be_bytes()).unwrap();
        let area = StagingArea::open(&path).unwrap();
        let err = area.records().unwrap_err();
        assert!(matches!(err, StoreError::CorruptStaging(_)));
    }
}
```
- [ ] In `src/lib.rs`, add `mod staging;` and `pub use staging::{StagedRecord, StagingArea};`.
- [ ] Run: `cargo test -p digstore-store --lib staging::`
  Expected: FAIL — `cannot find type StagingArea in this scope`.
- [ ] In `src/staging.rs`, add the implementation above the test module:
```rust
use crate::error::{Result, StoreError};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One staged resource: its key and the latest bytes staged for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRecord {
    pub resource_key: String,
    pub content: Vec<u8>,
}

/// Append-only binary staging file. Frame (Chia big-endian conventions):
/// `u32 BE key_len | key utf8 | u64 BE content_len | content`.
/// Re-staging a key appends a new frame; read-back is last-write-wins,
/// preserving first-seen order.
pub struct StagingArea {
    path: PathBuf,
}

impl StagingArea {
    /// Open (creating if absent) the staging file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            std::fs::File::create(&path)?;
        }
        Ok(Self { path })
    }

    /// Append a staged resource frame.
    pub fn append(&mut self, resource_key: &str, content: &[u8]) -> Result<()> {
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        let key_bytes = resource_key.as_bytes();
        f.write_all(&(key_bytes.len() as u32).to_be_bytes())?;
        f.write_all(key_bytes)?;
        f.write_all(&(content.len() as u64).to_be_bytes())?;
        f.write_all(content)?;
        Ok(())
    }

    /// Read all frames, collapsing to last-write-wins per key in first-seen order.
    pub fn records(&self) -> Result<Vec<StagedRecord>> {
        let raw = std::fs::read(&self.path)?;
        let mut cursor = 0usize;
        let mut order: Vec<String> = Vec::new();
        let mut latest: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        while cursor < raw.len() {
            let key_len = read_u32(&raw, &mut cursor)? as usize;
            let key = read_bytes(&raw, &mut cursor, key_len)?;
            let key = String::from_utf8(key)
                .map_err(|_| StoreError::CorruptStaging("non-utf8 resource key".into()))?;
            let content_len = read_u64(&raw, &mut cursor)? as usize;
            let content = read_bytes(&raw, &mut cursor, content_len)?;
            if !latest.contains_key(&key) {
                order.push(key.clone());
            }
            latest.insert(key, content);
        }
        Ok(order
            .into_iter()
            .map(|k| StagedRecord {
                content: latest.remove(&k).unwrap(),
                resource_key: k,
            })
            .collect())
    }

    /// True when no records are staged.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.records()?.is_empty())
    }

    /// Truncate the staging file to zero length.
    pub fn clear(&mut self) -> Result<()> {
        std::fs::File::create(&self.path)?;
        Ok(())
    }
}

fn read_u32(buf: &[u8], cursor: &mut usize) -> Result<u32> {
    let end = *cursor + 4;
    if end > buf.len() {
        return Err(StoreError::CorruptStaging("truncated u32".into()));
    }
    let v = u32::from_be_bytes(buf[*cursor..end].try_into().unwrap());
    *cursor = end;
    Ok(v)
}

fn read_u64(buf: &[u8], cursor: &mut usize) -> Result<u64> {
    let end = *cursor + 8;
    if end > buf.len() {
        return Err(StoreError::CorruptStaging("truncated u64".into()));
    }
    let v = u64::from_be_bytes(buf[*cursor..end].try_into().unwrap());
    *cursor = end;
    Ok(v)
}

fn read_bytes(buf: &[u8], cursor: &mut usize, len: usize) -> Result<Vec<u8>> {
    let end = *cursor + len;
    if end > buf.len() {
        return Err(StoreError::CorruptStaging("truncated payload".into()));
    }
    let v = buf[*cursor..end].to_vec();
    *cursor = end;
    Ok(v)
}
```
- [ ] Run: `cargo test -p digstore-store --lib staging::`
  Expected: PASS — `test result: ok. 5 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/staging.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): big-endian framed staging area with last-write-wins read-back"
```

> **Documented deviation #1 surfaced here:** the staging-record framing uses **big-endian** length prefixes (Chia streamable framing), per the locked decision, NOT the paper's "little-endian" note. Chia-compat wins.

---

## Task 6 — Per-directory content-addressed write-once chunk store

**Files:**
- Create: `crates/digstore-store/src/chunkstore.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/chunkstore.rs`

The chunk store writes one file per unique chunk under a given chunks directory, named by lower-case hex of its SHA-256 hash (computed by `digstore-crypto`). Writing an already-present chunk is a no-op (dedup). This is the per-directory primitive; **global** cross-generation dedup is layered on top in `Store::commit` (Task 13) and resolution across generations is `Store::resolve_chunk` (Task 14).

Steps:

- [ ] Create `crates/digstore-store/src/chunkstore.rs` with the failing tests FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;
    use tempfile::tempdir;

    fn h(b: u8) -> Bytes32 {
        Bytes32::from([b; 32])
    }

    #[test]
    fn put_writes_chunk_file_named_by_hash() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        let wrote = cs.put(h(0xaa), b"chunk-bytes").unwrap();
        assert!(wrote, "first put writes");
        let file = dir.path().join("aa".repeat(32));
        assert!(file.exists());
        assert_eq!(std::fs::read(&file).unwrap(), b"chunk-bytes");
    }

    #[test]
    fn duplicate_put_is_noop_and_returns_false() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        assert!(cs.put(h(0xbb), b"data").unwrap());
        let wrote_again = cs.put(h(0xbb), b"data").unwrap();
        assert!(!wrote_again, "second put deduplicates");
        assert_eq!(cs.count().unwrap(), 1);
    }

    #[test]
    fn count_reflects_unique_chunks() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        cs.put(h(1), b"a").unwrap();
        cs.put(h(2), b"b").unwrap();
        cs.put(h(1), b"a").unwrap(); // dup
        cs.put(h(3), b"c").unwrap();
        assert_eq!(cs.count().unwrap(), 3);
    }

    #[test]
    fn contains_and_get_roundtrip() {
        let dir = tempdir().unwrap();
        let cs = ChunkStore::new(dir.path());
        assert!(!cs.contains(h(9)).unwrap());
        cs.put(h(9), b"nine").unwrap();
        assert!(cs.contains(h(9)).unwrap());
        assert_eq!(cs.get(h(9)).unwrap(), b"nine");
    }
}
```
- [ ] In `src/lib.rs`, add `mod chunkstore;` and `pub use chunkstore::ChunkStore;`.
- [ ] Run: `cargo test -p digstore-store --lib chunkstore::`
  Expected: FAIL — `cannot find type ChunkStore in this scope`.
- [ ] In `src/chunkstore.rs`, add the implementation above the test module:
```rust
use crate::error::{Result, StoreError};
use digstore_core::Bytes32;
use std::path::{Path, PathBuf};

/// Per-directory content-addressed, write-once chunk store. One file per unique
/// chunk, named by lower-case hex of its SHA-256 hash. A repeated `put` is a
/// no-op (deduplication within this directory, §8.2).
pub struct ChunkStore {
    chunks_dir: PathBuf,
}

impl ChunkStore {
    /// Create over an existing or to-be-created chunks directory.
    pub fn new(chunks_dir: impl AsRef<Path>) -> Self {
        Self {
            chunks_dir: chunks_dir.as_ref().to_path_buf(),
        }
    }

    fn chunk_path(&self, hash: Bytes32) -> PathBuf {
        self.chunks_dir.join(hash.to_hex())
    }

    /// Store `data` under `hash`. Returns `true` if newly written, `false` if it
    /// already existed (deduplicated).
    pub fn put(&self, hash: Bytes32, data: &[u8]) -> Result<bool> {
        std::fs::create_dir_all(&self.chunks_dir)?;
        let path = self.chunk_path(hash);
        if path.exists() {
            return Ok(false);
        }
        // Atomic-ish: write to temp then rename within the same dir.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &path)?;
        Ok(true)
    }

    /// True if a chunk with this hash is present in this directory.
    pub fn contains(&self, hash: Bytes32) -> Result<bool> {
        Ok(self.chunk_path(hash).exists())
    }

    /// Read a chunk's bytes.
    pub fn get(&self, hash: Bytes32) -> Result<Vec<u8>> {
        let path = self.chunk_path(hash);
        if !path.exists() {
            return Err(StoreError::ChunkNotFound(hash.to_hex()));
        }
        Ok(std::fs::read(&path)?)
    }

    /// Number of unique chunk files present.
    pub fn count(&self) -> Result<usize> {
        if !self.chunks_dir.exists() {
            return Ok(0);
        }
        let mut n = 0;
        for entry in std::fs::read_dir(&self.chunks_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            // Ignore stray .tmp files left by an interrupted write.
            if name.to_string_lossy().ends_with(".tmp") {
                continue;
            }
            if entry.file_type()?.is_file() {
                n += 1;
            }
        }
        Ok(n)
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib chunkstore::`
  Expected: PASS — `test result: ok. 4 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/chunkstore.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): per-directory content-addressed write-once chunk store"
```

---

## Task 7 — Generation `manifest.json` schema + canonical `KeyTableEntry` mapping

**Files:**
- Create: `crates/digstore-store/src/generation.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/generation.rs`

> **Locked dependency for this task:** `Bytes32` derives `Copy + Ord + Hash` (see LOCKED INTER-CRATE CONTRACTS). The `BTreeSet<Bytes32>` helpers below rely on `Ord + Copy`; this is guaranteed, not a fallback.

`manifest.json` records the generation metadata the compiler later reads: schema version, generation id, root hash, timestamp, the ordered chunk list (the §8.3 interleaved-pool order source, consumed by `digstore-compiler`), and the key-table records mapping resource keys to ordered chunk indices.

The manifest's `KeyTableRecord` is a **deliberate superset** of the canonical `KeyTableEntry { static_key, generation, chunk_indices, total_size }`: it stores the same `chunk_indices`/`total_size`, carries the canonical `static_key` (the per-resource key the guest looks up = the URN retrieval key) and `generation` (= this generation's root), AND adds a human-readable `resource_key` for diffing/logging. A `to_key_table_entry()` method projects it down to the exact canonical `KeyTableEntry` so `digstore-compiler` can build the on-wire table without divergence.

Steps:

- [ ] Create `crates/digstore-store/src/generation.rs` with the failing tests FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, KeyTableEntry};
    use tempfile::tempdir;

    fn b(x: u8) -> Bytes32 {
        Bytes32::from([x; 32])
    }

    fn sample() -> GenerationManifest {
        GenerationManifest {
            schema_version: 1,
            generation_id: 3,
            root: b(0xab),
            timestamp: 1_717_000_000,
            chunks: vec![
                ChunkRef { index: 0, hash: b(0x01), size: 16 },
                ChunkRef { index: 1, hash: b(0x02), size: 32 },
            ],
            key_table: vec![KeyTableRecord {
                resource_key: "index.html".into(),
                static_key: b(0xff),
                generation: b(0xab),
                chunk_indices: vec![0, 1],
                total_size: 48,
            }],
        }
    }

    #[test]
    fn manifest_roundtrips_through_json() {
        let m = sample();
        let json = m.to_json().unwrap();
        let back = GenerationManifest::from_json(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn manifest_json_uses_hex_for_hashes() {
        let json = sample().to_json().unwrap();
        assert!(json.contains(&"ab".repeat(32))); // root + generation
        assert!(json.contains("\"index.html\""));
        assert!(json.contains("\"generation_id\": 3"));
    }

    #[test]
    fn manifest_writes_and_reads_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let m = sample();
        m.write_to(&path).unwrap();
        let back = GenerationManifest::read_from(&path).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn malformed_json_is_manifest_error() {
        let err = GenerationManifest::from_json("{ not json").unwrap_err();
        assert!(matches!(err, crate::StoreError::Manifest(_)));
    }

    #[test]
    fn invalid_root_hex_is_manifest_error() {
        // Structurally valid JSON, but `root` is not valid 32-byte hex.
        let json = r#"{
            "schema_version": 1,
            "generation_id": 0,
            "root": "zz",
            "timestamp": 1,
            "chunks": [],
            "key_table": []
        }"#;
        let err = GenerationManifest::from_json(json).unwrap_err();
        assert!(matches!(err, crate::StoreError::Manifest(_)));
    }

    #[test]
    fn key_table_record_projects_to_canonical_entry() {
        let rec = KeyTableRecord {
            resource_key: "index.html".into(),
            static_key: b(0xff),
            generation: b(0xab),
            chunk_indices: vec![0, 1],
            total_size: 48,
        };
        let entry: KeyTableEntry = rec.to_key_table_entry();
        assert_eq!(entry.static_key, b(0xff));
        assert_eq!(entry.generation, b(0xab));
        assert_eq!(entry.chunk_indices, vec![0, 1]);
        assert_eq!(entry.total_size, 48);
    }

    #[test]
    fn chunk_and_resource_set_helpers() {
        let m = sample();
        let chunks = m.chunk_hashes();
        assert!(chunks.contains(&b(0x01)));
        assert!(chunks.contains(&b(0x02)));
        assert_eq!(chunks.len(), 2);
        let keys = m.resource_keys();
        assert!(keys.contains("index.html"));
        assert_eq!(keys.len(), 1);
    }
}
```
- [ ] In `src/lib.rs`, add `mod generation;` and `pub use generation::{ChunkRef, GenerationManifest, KeyTableRecord};`.
- [ ] Run: `cargo test -p digstore-store --lib generation::`
  Expected: FAIL — `cannot find type GenerationManifest in this scope`.
- [ ] In `src/generation.rs`, add the implementation above the test module:
```rust
use crate::error::{Result, StoreError};
use digstore_core::{Bytes32, KeyTableEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

fn ser_hash<S: serde::Serializer>(h: &Bytes32, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&h.to_hex())
}

fn de_hash<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Bytes32, D::Error> {
    let s = String::deserialize(d)?;
    Bytes32::from_hex(&s).map_err(|_| serde::de::Error::custom("invalid 32-byte hex"))
}

/// One chunk's placement in the generation: its pool index, content hash, size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub index: u32,
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub hash: Bytes32,
    pub size: u64,
}

/// Manifest key-table record. A deliberate **superset** of the canonical
/// `KeyTableEntry { static_key, generation, chunk_indices, total_size }`:
/// it carries those exact canonical fields plus a human-readable `resource_key`
/// for diff/log. Use `to_key_table_entry` to project to the canonical type the
/// compiler embeds on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTableRecord {
    /// Human-readable resource key (diff/log only; not part of `KeyTableEntry`).
    pub resource_key: String,
    /// Canonical `KeyTableEntry::static_key` — the per-resource lookup key
    /// (= the URN retrieval key for this resource).
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub static_key: Bytes32,
    /// Canonical `KeyTableEntry::generation` — this generation's root.
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub generation: Bytes32,
    pub chunk_indices: Vec<u32>,
    pub total_size: u64,
}

impl KeyTableRecord {
    /// Project to the exact canonical `KeyTableEntry` (drops `resource_key`).
    pub fn to_key_table_entry(&self) -> KeyTableEntry {
        KeyTableEntry {
            static_key: self.static_key,
            generation: self.generation,
            chunk_indices: self.chunk_indices.clone(),
            total_size: self.total_size,
        }
    }
}

/// Generation metadata written to `generations/{root}/manifest.json` (§4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationManifest {
    pub schema_version: u32,
    pub generation_id: u64,
    #[serde(serialize_with = "ser_hash", deserialize_with = "de_hash")]
    pub root: Bytes32,
    pub timestamp: u64,
    pub chunks: Vec<ChunkRef>,
    pub key_table: Vec<KeyTableRecord>,
}

impl GenerationManifest {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| StoreError::Manifest(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| StoreError::Manifest(e.to_string()))
    }

    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }

    pub fn read_from(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json(&text)
    }

    /// Set of unique chunk hashes in this generation (for diff, §20.4).
    pub fn chunk_hashes(&self) -> BTreeSet<Bytes32> {
        self.chunks.iter().map(|c| c.hash).collect()
    }

    /// Set of resource keys in this generation (for diff, §20.4).
    pub fn resource_keys(&self) -> BTreeSet<String> {
        self.key_table.iter().map(|k| k.resource_key.clone()).collect()
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib generation::`
  Expected: PASS — `test result: ok. 7 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/generation.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): generation manifest with KeyTableEntry projection (§8.2)"
```

---

## Task 8 — Append-only monotonic root history

**Files:**
- Create: `crates/digstore-store/src/history.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/history.rs`

`roots.log` is one line per generation: `{id}\t{root_hex}\t{timestamp}`. `append` enforces monotonicity (each id is exactly `last_id + 1`, or `0` for the first). The history is the §4.3 append-only list exported later via the guest's `get_roothash_history`.

Steps:

- [ ] Create `crates/digstore-store/src/history.rs` with the failing tests FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, GenerationState};
    use tempfile::tempdir;

    fn gs(id: u64, b: u8, ts: u64) -> GenerationState {
        GenerationState { id, root: Bytes32::from([b; 32]), timestamp: ts }
    }

    #[test]
    fn append_and_read_back_in_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        h.append(&gs(0, 0xa0, 100)).unwrap();
        h.append(&gs(1, 0xa1, 200)).unwrap();

        let all = RootHistory::open(&path).unwrap().entries().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, 0);
        assert_eq!(all[0].root, Bytes32::from([0xa0; 32]));
        assert_eq!(all[1].id, 1);
        assert_eq!(all[1].timestamp, 200);
    }

    #[test]
    fn first_append_must_be_id_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        let err = h.append(&gs(5, 0xff, 1)).unwrap_err();
        assert!(matches!(err, crate::StoreError::NonMonotonicHistory { last: _, got: 5 }));
    }

    #[test]
    fn non_consecutive_append_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        h.append(&gs(0, 0x00, 1)).unwrap();
        let err = h.append(&gs(2, 0x02, 2)).unwrap_err();
        assert!(matches!(err, crate::StoreError::NonMonotonicHistory { last: 0, got: 2 }));
    }

    #[test]
    fn head_returns_latest_generation() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        assert!(h.head().unwrap().is_none());
        h.append(&gs(0, 0x00, 1)).unwrap();
        h.append(&gs(1, 0x11, 2)).unwrap();
        let head = h.head().unwrap().unwrap();
        assert_eq!(head.id, 1);
        assert_eq!(head.root, Bytes32::from([0x11; 32]));
    }

    #[test]
    fn next_id_is_zero_when_empty_then_increments() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roots.log");
        let mut h = RootHistory::open(&path).unwrap();
        assert_eq!(h.next_id().unwrap(), 0);
        h.append(&gs(0, 0x00, 1)).unwrap();
        assert_eq!(h.next_id().unwrap(), 1);
    }
}
```
- [ ] In `src/lib.rs`, add `mod history;` and `pub use history::RootHistory;`.
- [ ] Run: `cargo test -p digstore-store --lib history::`
  Expected: FAIL — `cannot find type RootHistory in this scope`.
- [ ] In `src/history.rs`, add the implementation above the test module:
```rust
use crate::error::{Result, StoreError};
use digstore_core::{Bytes32, GenerationState};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Append-only, monotonic root history backed by `roots.log` (§4.3).
/// Line format: `{id}\t{root_hex}\t{timestamp}`.
pub struct RootHistory {
    path: PathBuf,
}

impl RootHistory {
    /// Open (creating if absent) the history file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            std::fs::File::create(&path)?;
        }
        Ok(Self { path })
    }

    /// All generation states, oldest first.
    pub fn entries(&self) -> Result<Vec<GenerationState>> {
        let text = std::fs::read_to_string(&self.path)?;
        let mut out = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split('\t');
            let id = parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| StoreError::CorruptStaging("history: bad id".into()))?;
            let root_hex = parts
                .next()
                .ok_or_else(|| StoreError::CorruptStaging("history: missing root".into()))?;
            let root = Bytes32::from_hex(root_hex)
                .map_err(|_| StoreError::CorruptStaging("history: bad root hex".into()))?;
            let timestamp = parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| StoreError::CorruptStaging("history: bad timestamp".into()))?;
            out.push(GenerationState { id, root, timestamp });
        }
        Ok(out)
    }

    /// The latest generation, or `None` if the history is empty.
    pub fn head(&self) -> Result<Option<GenerationState>> {
        Ok(self.entries()?.into_iter().last())
    }

    /// The id the next appended generation must use.
    pub fn next_id(&self) -> Result<u64> {
        Ok(match self.head()? {
            Some(h) => h.id + 1,
            None => 0,
        })
    }

    /// Append a generation, enforcing strict monotonic id (`last + 1`, or `0`).
    pub fn append(&mut self, gen: &GenerationState) -> Result<()> {
        let expected = self.next_id()?;
        if gen.id != expected {
            let last = expected.saturating_sub(1);
            return Err(StoreError::NonMonotonicHistory { last, got: gen.id });
        }
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(f, "{}\t{}\t{}", gen.id, gen.root.to_hex(), gen.timestamp)?;
        Ok(())
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib history::`
  Expected: PASS — `test result: ok. 5 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/history.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): append-only monotonic root history (roots.log, §4.3)"
```

---

## Task 9 — `GenerationDiff` (chunk-set + resource-key diff)

**Files:**
- Create: `crates/digstore-store/src/diff.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/diff.rs`

> **Locked dependency for this task:** `Bytes32: Ord + Copy` (LOCKED CONTRACTS). The `.difference(...).copied()` calls below compile against the guaranteed derives.

`digstore diff <a> <b>` (§20.4) compares two generations by their chunk sets and resource keys. This is a pure function over two `GenerationManifest`s.

Steps:

- [ ] Create `crates/digstore-store/src/diff.rs` with the failing tests FIRST:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{ChunkRef, GenerationManifest, KeyTableRecord};
    use digstore_core::Bytes32;

    fn b(x: u8) -> Bytes32 {
        Bytes32::from([x; 32])
    }

    fn gen(id: u64, chunk_bytes: &[u8], keys: &[&str]) -> GenerationManifest {
        GenerationManifest {
            schema_version: 1,
            generation_id: id,
            root: b(id as u8),
            timestamp: id,
            chunks: chunk_bytes
                .iter()
                .enumerate()
                .map(|(i, &c)| ChunkRef { index: i as u32, hash: b(c), size: 1 })
                .collect(),
            key_table: keys
                .iter()
                .map(|k| KeyTableRecord {
                    resource_key: (*k).into(),
                    static_key: b(0xee),
                    generation: b(id as u8),
                    chunk_indices: vec![0],
                    total_size: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn diff_reports_added_and_removed_chunks() {
        let a = gen(0, &[1, 2, 3], &["a.txt"]);
        let b = gen(1, &[2, 3, 4], &["a.txt"]);
        let d = GenerationDiff::between(&a, &b);
        assert_eq!(d.chunks_added, vec![Bytes32::from([4u8; 32])]);
        assert_eq!(d.chunks_removed, vec![Bytes32::from([1u8; 32])]);
    }

    #[test]
    fn diff_reports_added_and_removed_resource_keys() {
        let a = gen(0, &[1], &["a.txt", "b.txt"]);
        let b = gen(1, &[1], &["b.txt", "c.txt"]);
        let d = GenerationDiff::between(&a, &b);
        assert_eq!(d.keys_added, vec!["c.txt".to_string()]);
        assert_eq!(d.keys_removed, vec!["a.txt".to_string()]);
    }

    #[test]
    fn identical_generations_produce_empty_diff() {
        let a = gen(0, &[1, 2], &["a.txt"]);
        let b = gen(1, &[1, 2], &["a.txt"]);
        let d = GenerationDiff::between(&a, &b);
        assert!(d.is_empty());
    }
}
```
- [ ] In `src/lib.rs`, add `mod diff;` and `pub use diff::GenerationDiff;`.
- [ ] Run: `cargo test -p digstore-store --lib diff::`
  Expected: FAIL — `cannot find type GenerationDiff in this scope`.
- [ ] In `src/diff.rs`, add the implementation above the test module:
```rust
use crate::generation::GenerationManifest;
use digstore_core::Bytes32;

/// Difference between two generations (§20.4): chunk-set delta + resource-key
/// delta. Results are sorted by hex / lexicographic order for determinism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationDiff {
    pub chunks_added: Vec<Bytes32>,
    pub chunks_removed: Vec<Bytes32>,
    pub keys_added: Vec<String>,
    pub keys_removed: Vec<String>,
}

impl GenerationDiff {
    /// Compute the diff transforming generation `a` into generation `b`.
    pub fn between(a: &GenerationManifest, b: &GenerationManifest) -> Self {
        let a_chunks = a.chunk_hashes();
        let b_chunks = b.chunk_hashes();
        let mut chunks_added: Vec<Bytes32> = b_chunks.difference(&a_chunks).copied().collect();
        let mut chunks_removed: Vec<Bytes32> = a_chunks.difference(&b_chunks).copied().collect();
        chunks_added.sort_by_key(|h| h.to_hex());
        chunks_removed.sort_by_key(|h| h.to_hex());

        let a_keys = a.resource_keys();
        let b_keys = b.resource_keys();
        let mut keys_added: Vec<String> = b_keys.difference(&a_keys).cloned().collect();
        let mut keys_removed: Vec<String> = a_keys.difference(&b_keys).cloned().collect();
        keys_added.sort();
        keys_removed.sort();

        Self { chunks_added, chunks_removed, keys_added, keys_removed }
    }

    /// True when the generations have identical chunk sets and resource keys.
    pub fn is_empty(&self) -> bool {
        self.chunks_added.is_empty()
            && self.chunks_removed.is_empty()
            && self.keys_added.is_empty()
            && self.keys_removed.is_empty()
    }
}
```
- [ ] Run: `cargo test -p digstore-store --lib diff::`
  Expected: PASS — `test result: ok. 3 passed; 0 failed`.
- [ ] Run: `cargo build -p digstore-store`  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/diff.rs crates/digstore-store/src/lib.rs
git commit -m "feat(store): GenerationDiff over chunk sets and resource keys (§20.4)"
```

---

## Task 10 — `Store::init` / `open` create the exact §4.4 directory tree

**Files:**
- Create: `crates/digstore-store/src/store.rs`
- Modify: `crates/digstore-store/src/lib.rs`
- Create: `crates/digstore-store/tests/layout.rs`

The `Store` entity is generic over a `Clock`. `init` writes `config.toml`, an empty staging file, an empty `roots.log`, and the `generations/` and `modules/` directories. It refuses to clobber an existing store.

Steps:

- [ ] Create `crates/digstore-store/tests/layout.rs` with the failing integration tests:
```rust
use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_store::{FixedClock, Store};
use tempfile::tempdir;

fn config(dir: &std::path::Path) -> StoreConfig {
    StoreConfig {
        store_id: Bytes32::from([0x33u8; 32]),
        data_dir: dir.to_string_lossy().to_string(),
        max_size: 10_000_000,
        visibility: Visibility::Public,
    }
}

#[test]
fn init_creates_exact_layout() {
    let dir = tempdir().unwrap();
    let clock = FixedClock::new(1_717_000_000);
    let store = Store::init(config(dir.path()), clock).unwrap();

    let sid_hex = "33".repeat(32);
    assert!(dir.path().join("config.toml").exists(), "config.toml");
    assert!(dir.path().join(format!("{sid_hex}.staging.bin")).exists(), "staging");
    assert!(dir.path().join("roots.log").exists(), "roots.log");
    assert!(dir.path().join("generations").is_dir(), "generations dir");
    assert!(dir.path().join("modules").is_dir(), "modules dir");

    assert_eq!(store.store_id(), Bytes32::from([0x33u8; 32]));
    assert!(store.root_history().unwrap().is_empty());
}

#[test]
fn init_refuses_to_clobber_existing_store() {
    let dir = tempdir().unwrap();
    Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let err = Store::init(config(dir.path()), FixedClock::new(1)).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::AlreadyExists(_)));
}

#[test]
fn open_reloads_an_existing_store() {
    let dir = tempdir().unwrap();
    Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let reopened = Store::open(dir.path(), FixedClock::new(99)).unwrap();
    assert_eq!(reopened.store_id(), Bytes32::from([0x33u8; 32]));
    assert!(matches!(reopened.config().visibility, Visibility::Public));
}

#[test]
fn open_missing_store_errors() {
    let dir = tempdir().unwrap();
    let err = Store::open(dir.path(), FixedClock::new(1)).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::NotFound(_)));
}
```
- [ ] In `src/lib.rs`, add `mod store;` and `pub use store::Store;`.
- [ ] Run: `cargo test -p digstore-store --test layout`
  Expected: FAIL — `cannot find type Store in crate digstore_store` / unresolved import.
- [ ] Create `crates/digstore-store/src/store.rs` with the initial `Store` implementation (init/open/accessors):
```rust
use crate::clock::Clock;
use crate::config::{load_config, save_config};
use crate::error::{Result, StoreError};
use crate::history::RootHistory;
use crate::paths::StorePaths;
use crate::staging::StagingArea;
use digstore_core::{Bytes32, GenerationState, StoreConfig};
use std::path::Path;

/// The host-side Store entity (§4). Owns the on-disk layout, staging, and
/// generations. Generic over a `Clock` so commit timestamps are injectable.
pub struct Store<C: Clock> {
    config: StoreConfig,
    paths: StorePaths,
    clock: C,
}

impl<C: Clock> Store<C> {
    /// Create a new store: write config + the §4.4 directory tree. Refuses to
    /// overwrite an existing store (presence of `config.toml`).
    pub fn init(config: StoreConfig, clock: C) -> Result<Self> {
        let paths = StorePaths::new(&config.data_dir, config.store_id);
        if paths.config_file().exists() {
            return Err(StoreError::AlreadyExists(paths.root().display().to_string()));
        }
        std::fs::create_dir_all(paths.root())?;
        std::fs::create_dir_all(paths.generations_dir())?;
        std::fs::create_dir_all(paths.modules_dir())?;
        save_config(paths.config_file(), &config)?;
        StagingArea::open(paths.staging_file())?;
        RootHistory::open(paths.history_file())?;
        Ok(Self { config, paths, clock })
    }

    /// Open an existing store rooted at `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>, clock: C) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let config_file = data_dir.join("config.toml");
        if !config_file.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        let config = load_config(&config_file)?;
        let paths = StorePaths::new(data_dir, config.store_id);
        Ok(Self { config, paths, clock })
    }

    pub fn store_id(&self) -> Bytes32 {
        self.config.store_id
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// All generation states, oldest first (§4.3 root history).
    pub fn root_history(&self) -> Result<Vec<GenerationState>> {
        RootHistory::open(self.paths.history_file())?.entries()
    }
}
```
- [ ] Run: `cargo test -p digstore-store --test layout`
  Expected: PASS — `test result: ok. 4 passed; 0 failed`.
- [ ] Run: `cargo test -p digstore-store --lib`  Expected: PASS (all module unit tests still green).
- [ ] Commit:
```
git add crates/digstore-store/src/store.rs crates/digstore-store/src/lib.rs crates/digstore-store/tests/layout.rs
git commit -m "feat(store): Store::init/open creating the §4.4 on-disk layout (§20.1)"
```

---

## Task 11 — `Store::add` / `stage_file` (path becomes resource key)

**Files:**
- Modify: `crates/digstore-store/src/store.rs`
- Create: `crates/digstore-store/tests/commit_flow.rs`

`add(<file>, <base>)` (§20.2) stages a file: the file's path relative to `base` becomes its resource_key (forward-slash normalized), and bytes are appended to the staging area. `stage_file(resource_key, bytes)` is the lower-level entry used by `add`. Chunking happens at commit time (Task 13); `add` records bytes verbatim into staging.

Steps:

- [ ] Create `crates/digstore-store/tests/commit_flow.rs` with the failing staging tests:
```rust
use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_store::{FixedClock, StagingArea, Store};
use std::io::Write;
use tempfile::tempdir;

fn config(dir: &std::path::Path) -> StoreConfig {
    StoreConfig {
        store_id: Bytes32::from([0x44u8; 32]),
        data_dir: dir.to_string_lossy().to_string(),
        max_size: 10_000_000,
        visibility: Visibility::Public,
    }
}

#[test]
fn stage_file_appends_to_staging() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    store.stage_file("index.html", b"<html>hello</html>").unwrap();

    let staged = StagingArea::open(store.paths().staging_file())
        .unwrap()
        .records()
        .unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].resource_key, "index.html");
    assert_eq!(staged[0].content, b"<html>hello</html>");
}

#[test]
fn add_uses_relative_path_as_resource_key() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();

    let src_dir = tempdir().unwrap();
    let nested = src_dir.path().join("assets");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("logo.svg");
    let mut f = std::fs::File::create(&file).unwrap();
    f.write_all(b"<svg/>").unwrap();

    store.add(&file, src_dir.path()).unwrap();

    let staged = StagingArea::open(store.paths().staging_file())
        .unwrap()
        .records()
        .unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].resource_key, "assets/logo.svg");
    assert_eq!(staged[0].content, b"<svg/>");
}

#[test]
fn add_rejects_file_outside_base() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let base = tempdir().unwrap();
    let other = tempdir().unwrap();
    let file = other.path().join("x.txt");
    std::fs::write(&file, b"x").unwrap();
    let err = store.add(&file, base.path()).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::PathEscape(_)));
}
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow stage_file_appends_to_staging`
  Expected: FAIL — `no method named stage_file found for struct Store`.
- [ ] In `src/store.rs`, add the staging methods to the `impl<C: Clock> Store<C>` block:
```rust
    /// Stage raw bytes under an explicit resource key (§20.2).
    pub fn stage_file(&mut self, resource_key: &str, bytes: &[u8]) -> Result<()> {
        let mut staging = StagingArea::open(self.paths.staging_file())?;
        staging.append(resource_key, bytes)?;
        Ok(())
    }

    /// Stage a file from disk. The path relative to `base` becomes the resource
    /// key (forward-slash normalized); the file bytes are staged verbatim.
    pub fn add(&mut self, file: impl AsRef<Path>, base: impl AsRef<Path>) -> Result<()> {
        let file = file.as_ref();
        let base = base.as_ref();
        let rel = file
            .strip_prefix(base)
            .map_err(|_| StoreError::PathEscape(file.to_path_buf()))?;
        let resource_key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(file)?;
        self.stage_file(&resource_key, &bytes)
    }
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow stage_file_appends_to_staging`
  Expected: PASS.
- [ ] Run: `cargo test -p digstore-store --test commit_flow add_uses_relative_path_as_resource_key add_rejects_file_outside_base`
  Expected: PASS — 2 passed.
- [ ] Commit:
```
git add crates/digstore-store/src/store.rs crates/digstore-store/tests/commit_flow.rs
git commit -m "feat(store): Store::add and stage_file (path becomes resource key, §20.2)"
```

---

## Task 12 — `Store::commit` (chunk, merkle, generation, history) — single-generation dedup

**Files:**
- Modify: `crates/digstore-store/src/store.rs`
- Test: extend `crates/digstore-store/tests/commit_flow.rs`

`commit` (§20.3, §8.2): (1) reads staged records; (2) chunks each via `digstore_chunker::chunk_bytes` and hashes each chunk with `digstore_crypto::sha256`; (3) builds the ordered chunk pool (the §8.3 source, consumed by the compiler) in staged-record order, recording each resource's pool indices into a `KeyTableRecord` whose `static_key = SHA-256(canonical_urn)` and `generation = root`; (4) builds the per-generation merkle tree over chunk leaves via `digstore_core::MerkleTree::from_leaves`; (5) creates `GenerationState { id = next_id, root, timestamp = clock }`; (6) writes the generation dir (`manifest.json` + per-directory dedup `chunks/`); (7) appends to root history; (8) clears staging; (9) returns the new root. Global cross-generation dedup is layered in Task 13. Compilation is NOT done here.

> **URN root-independence decision (documented minor):** `commit` builds the URN with `root_hash: None`, so `static_key` (= `Urn::retrieval_key`) is **root-independent**: the same resource keeps the same lookup key across generations. This is deliberate and is asserted for client parity in Task 13. Clients/guests that key by resource (not by `(root, resource)`) match this value.

Steps:

- [ ] Append the failing commit tests to `crates/digstore-store/tests/commit_flow.rs`:
```rust
#[test]
fn commit_creates_generation_and_advances_history() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1_717_000_000)).unwrap();
    store.stage_file("index.html", &vec![0xABu8; 200_000]).unwrap();

    let root = store.commit().unwrap();

    let hist = store.root_history().unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].id, 0);
    assert_eq!(hist[0].root, root);
    assert_eq!(hist[0].timestamp, 1_717_000_000);

    let root_hex = root.to_hex();
    assert!(store.paths().generation_manifest(&root_hex).exists());
    assert!(store.paths().generation_chunks_dir(&root_hex).is_dir());

    assert!(
        digstore_store::StagingArea::open(store.paths().staging_file())
            .unwrap()
            .is_empty()
            .unwrap()
    );
}

#[test]
fn commit_refuses_empty_staging() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    let err = store.commit().unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::EmptyStaging));
}

#[test]
fn commit_is_deterministic_for_fixed_input() {
    // Two independent stores with identical store_id, content, and clock must
    // produce the identical root hash (store-side determinism feeding §19.3).
    fn build() -> Bytes32 {
        let dir = tempdir().unwrap();
        let mut store = Store::init(config(dir.path()), FixedClock::new(42)).unwrap();
        store.stage_file("a.txt", b"deterministic content here").unwrap();
        store.stage_file("b.txt", &vec![7u8; 100_000]).unwrap();
        store.commit().unwrap()
    }
    assert_eq!(build(), build());
}

#[test]
fn commit_static_key_matches_client_url_retrieval_key() {
    // The manifest's static_key for a resource equals the retrieval key a client
    // computes from the canonical root-less URN (documented root-independence).
    use digstore_core::Urn;
    use digstore_store::GenerationManifest;

    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    store.stage_file("index.html", b"hello").unwrap();
    let root = store.commit().unwrap();

    let manifest =
        GenerationManifest::read_from(store.paths().generation_manifest(&root.to_hex())).unwrap();
    let rec = manifest
        .key_table
        .iter()
        .find(|r| r.resource_key == "index.html")
        .unwrap();

    let client_urn = Urn {
        chain: "chia".to_string(),
        store_id: Bytes32::from([0x44u8; 32]),
        root_hash: None,
        resource_key: Some("index.html".to_string()),
    };
    assert_eq!(rec.static_key, client_urn.retrieval_key());
    assert_eq!(rec.generation, root);
}
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow commit_creates_generation_and_advances_history`
  Expected: FAIL — `no method named commit found for struct Store`.
- [ ] In `src/store.rs`, extend the top-of-file imports (add these `use` lines):
```rust
use crate::chunkstore::ChunkStore;
use crate::generation::{ChunkRef, GenerationManifest, KeyTableRecord};
use digstore_chunker::{chunk_bytes, ChunkerConfig};
use digstore_core::{MerkleTree, Urn};
```
- [ ] In `src/store.rs`, add the `commit` method to the `impl<C: Clock> Store<C>` block. Note `ChunkerConfig` is constructed explicitly with the catalog defaults including `mask` (no reliance on `Default`):
```rust
    /// Finalize a generation (§20.3, §8.2): chunk staged content, build the
    /// per-generation merkle tree, append the root to history, write the
    /// generation directory. Returns the new root hash. Does NOT compile the
    /// module (that is `digstore-compiler`'s job over this generation dir).
    pub fn commit(&mut self) -> Result<Bytes32> {
        let mut staging = StagingArea::open(self.paths.staging_file())?;
        let records = staging.records()?;
        if records.is_empty() {
            return Err(StoreError::EmptyStaging);
        }

        // Catalog chunker defaults (min 16 KiB, target 64 KiB, max 256 KiB).
        // `mask` selects the average chunk size; (1<<16)-1 targets ~64 KiB.
        let chunker = ChunkerConfig {
            min_size: 16 * 1024,
            target_size: 64 * 1024,
            max_size: 256 * 1024,
            mask: (1u64 << 16) - 1,
        };

        // Build the chunk pool in staged-record order (the §8.3 source consumed
        // by the compiler) and the key table mapping each resource to its
        // ordered pool indices.
        let mut pool: Vec<(Bytes32, Vec<u8>)> = Vec::new();
        let mut key_table: Vec<KeyTableRecord> = Vec::new();

        for rec in &records {
            let chunks = chunk_bytes(&rec.content, &chunker);
            let mut indices = Vec::with_capacity(chunks.len());
            let mut total: u64 = 0;
            for chunk in &chunks {
                let hash = digstore_crypto::sha256(chunk);
                let index = pool.len() as u32;
                pool.push((hash, chunk.clone()));
                indices.push(index);
                total += chunk.len() as u64;
            }
            // root_hash: None -> retrieval key is root-independent (documented).
            let urn = Urn {
                chain: "chia".to_string(),
                store_id: self.config.store_id,
                root_hash: None,
                resource_key: Some(rec.resource_key.clone()),
            };
            key_table.push(KeyTableRecord {
                resource_key: rec.resource_key.clone(),
                static_key: urn.retrieval_key(),
                generation: Bytes32::from([0u8; 32]), // placeholder set after root
                chunk_indices: indices,
                total_size: total,
            });
        }

        // Merkle tree over chunk leaves in pool order (§9.1, owned by core).
        let leaves: Vec<Bytes32> = pool.iter().map(|(h, _)| *h).collect();
        let tree = MerkleTree::from_leaves(&leaves);
        let root = tree.root();
        let root_hex = root.to_hex();

        // Now that we know the root, stamp each key-table record's generation.
        for rec in &mut key_table {
            rec.generation = root;
        }

        // Write generation dir (per-directory dedup; global dedup added in
        // Task 13 over `chunk_exists_anywhere`).
        let chunks_dir = self.paths.generation_chunks_dir(&root_hex);
        std::fs::create_dir_all(&chunks_dir)?;
        let chunkstore = ChunkStore::new(&chunks_dir);
        let mut chunk_refs = Vec::with_capacity(pool.len());
        for (i, (hash, data)) in pool.iter().enumerate() {
            chunkstore.put(*hash, data)?;
            chunk_refs.push(ChunkRef { index: i as u32, hash: *hash, size: data.len() as u64 });
        }

        let next_id = RootHistory::open(self.paths.history_file())?.next_id()?;
        let timestamp = self.clock.unix_seconds();

        let manifest = GenerationManifest {
            schema_version: 1,
            generation_id: next_id,
            root,
            timestamp,
            chunks: chunk_refs,
            key_table,
        };
        manifest.write_to(self.paths.generation_manifest(&root_hex))?;

        let mut history = RootHistory::open(self.paths.history_file())?;
        history.append(&GenerationState { id: next_id, root, timestamp })?;
        staging.clear()?;

        Ok(root)
    }
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow commit_creates_generation_and_advances_history`
  Expected: PASS.
- [ ] Run: `cargo test -p digstore-store --test commit_flow commit_refuses_empty_staging commit_is_deterministic_for_fixed_input commit_static_key_matches_client_url_retrieval_key`
  Expected: PASS — 3 passed (deterministic root + client-parity static_key confirmed).
- [ ] Commit:
```
git add crates/digstore-store/src/store.rs crates/digstore-store/tests/commit_flow.rs
git commit -m "feat(store): Store::commit — chunk, merkle root, generation, history (§20.3)"
```

---

## Task 13 — Global cross-generation chunk dedup (shared chunk stored once)

**Files:**
- Test: extend `crates/digstore-store/tests/commit_flow.rs`
- Modify: `crates/digstore-store/src/store.rs`

Per §8.2, chunks shared with prior generations are not re-stored. The Task 12 `commit` writes chunks under the new generation's `chunks/` dir, so two generations sharing a chunk each get a copy. To honor global dedup, `commit` must check all generation dirs for the chunk before writing, and only write when the chunk is absent everywhere. We assert the **total** on-disk chunk-file count equals the number of **unique** chunks across both commits.

Steps:

- [ ] Append the failing dedup test to `crates/digstore-store/tests/commit_flow.rs`:
```rust
fn count_all_chunk_files(generations_dir: &std::path::Path) -> usize {
    let mut n = 0;
    for gen in std::fs::read_dir(generations_dir).unwrap() {
        let chunks = gen.unwrap().path().join("chunks");
        if chunks.is_dir() {
            for e in std::fs::read_dir(&chunks).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_file()
                    && !e.file_name().to_string_lossy().ends_with(".tmp")
                {
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
fn shared_chunk_is_stored_once_across_generations() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();

    // Generation 0: one big resource (forces several chunks).
    let payload = vec![0x5Au8; 300_000];
    store.stage_file("data.bin", &payload).unwrap();
    let root0 = store.commit().unwrap();

    // Generation 1: identical resource bytes -> identical chunks -> all dedup,
    // plus one brand-new chunk from a second resource.
    store.stage_file("data.bin", &payload).unwrap();
    store.stage_file("note.txt", b"a unique small note").unwrap();
    let root1 = store.commit().unwrap();

    assert_ne!(root0, root1, "different generations have different roots");

    use digstore_store::GenerationManifest;
    let m0 = GenerationManifest::read_from(store.paths().generation_manifest(&root0.to_hex())).unwrap();
    let m1 = GenerationManifest::read_from(store.paths().generation_manifest(&root1.to_hex())).unwrap();
    let mut union = m0.chunk_hashes();
    union.extend(m1.chunk_hashes());

    let on_disk = count_all_chunk_files(&store.paths().generations_dir());
    assert_eq!(
        on_disk,
        union.len(),
        "each unique chunk stored exactly once across all generations"
    );
}
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow shared_chunk_is_stored_once_across_generations`
  Expected: FAIL — `on_disk` (chunks duplicated per generation) is greater than `union.len()`; assertion `left == right` fails with `left: <bigger>, right: <smaller>`.
- [ ] In `src/store.rs`, add a private helper (inside `impl<C: Clock> Store<C>`) that checks whether a chunk exists in ANY generation:
```rust
    /// True if a chunk with this hash is already stored under some generation
    /// directory (global dedup index, §8.2).
    fn chunk_exists_anywhere(&self, hash: Bytes32) -> Result<bool> {
        let gens = self.paths.generations_dir();
        if !gens.exists() {
            return Ok(false);
        }
        let name = hash.to_hex();
        for entry in std::fs::read_dir(&gens)? {
            let chunks = entry?.path().join("chunks");
            if chunks.join(&name).exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }
```
- [ ] In `src/store.rs`, change the chunk-writing loop inside `commit` to skip chunks already present in any generation. Replace:
```rust
        for (i, (hash, data)) in pool.iter().enumerate() {
            chunkstore.put(*hash, data)?;
            chunk_refs.push(ChunkRef { index: i as u32, hash: *hash, size: data.len() as u64 });
        }
```
with:
```rust
        for (i, (hash, data)) in pool.iter().enumerate() {
            // §8.2: only store the chunk if it is not already present in this or
            // any prior generation. `chunk_refs` still records every chunk's
            // index so reassembly is complete regardless of where the bytes live
            // (resolved globally by `Store::resolve_chunk`, Task 14).
            if !self.chunk_exists_anywhere(*hash)? {
                chunkstore.put(*hash, data)?;
            }
            chunk_refs.push(ChunkRef { index: i as u32, hash: *hash, size: data.len() as u64 });
        }
```
> Because the new generation's chunks dir is itself under `generations/`, intra-generation duplicate chunks are also caught: the first lookup for an in-generation repeat returns false (not yet written), `put` writes it once; a later identical chunk in the same pool finds it present and skips. `ChunkStore::put` stays idempotent as a backstop.
- [ ] Run: `cargo test -p digstore-store --test commit_flow shared_chunk_is_stored_once_across_generations`
  Expected: PASS — `on_disk == union.len()`.
- [ ] Run: `cargo test -p digstore-store --test commit_flow`  Expected: PASS (all commit-flow tests still green).
- [ ] Commit:
```
git add crates/digstore-store/src/store.rs crates/digstore-store/tests/commit_flow.rs
git commit -m "feat(store): global cross-generation chunk dedup (§8.2)"
```

---

## Task 14 — `Store::resolve_chunk` (global content-addressed resolution)

**Files:**
- Modify: `crates/digstore-store/src/store.rs`
- Test: extend `crates/digstore-store/tests/commit_flow.rs`

Because dedup is global (Task 13), a chunk introduced in generation 0 is NOT re-stored under generation 1's `chunks/` dir — that dir is **sparse** after dedup. So a consumer must resolve chunk bytes by hash across ALL generation dirs, not just the chunk's own generation. `Store::resolve_chunk(hash)` scans every generation's `chunks/` and returns the bytes (or `ChunkNotFound`). This reconciles the §4.4 per-generation layout with the §8.2 dedup optimization. We assert that a deduplicated chunk of generation 1 (whose bytes physically live under generation 0) resolves successfully.

Steps:

- [ ] Append the failing resolve test to `crates/digstore-store/tests/commit_flow.rs`:
```rust
#[test]
fn resolve_chunk_reads_deduplicated_chunk_across_generations() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();

    let payload = vec![0x5Au8; 300_000];
    store.stage_file("data.bin", &payload).unwrap();
    let _root0 = store.commit().unwrap();

    // Generation 1 re-stages identical bytes; its chunks are deduplicated to
    // generation 0's chunk files (sparse chunks/ dir under generation 1).
    store.stage_file("data.bin", &payload).unwrap();
    let root1 = store.commit().unwrap();

    use digstore_store::GenerationManifest;
    let m1 = GenerationManifest::read_from(store.paths().generation_manifest(&root1.to_hex())).unwrap();
    let shared = m1.chunks[0].hash;

    // The chunk file does NOT exist under generation 1 (deduplicated)...
    let gen1_local = store
        .paths()
        .chunk_file(&root1.to_hex(), &shared.to_hex());
    assert!(!gen1_local.exists(), "dedup leaves generation 1 chunks/ sparse");

    // ...but resolve_chunk finds it globally and returns the correct bytes.
    let bytes = store.resolve_chunk(shared).unwrap();
    assert_eq!(bytes.len(), m1.chunks[0].size as usize);
    assert_eq!(digstore_crypto::sha256(&bytes), shared);
}

#[test]
fn resolve_unknown_chunk_errors() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(1)).unwrap();
    store.stage_file("a.txt", b"x").unwrap();
    let _r = store.commit().unwrap();
    let bogus = Bytes32::from([0xCDu8; 32]);
    let err = store.resolve_chunk(bogus).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::ChunkNotFound(_)));
}
```
> This test references `digstore_crypto::sha256` directly; add `digstore-crypto` as a `[dev-dependencies]` entry too if the integration test cannot see the normal dependency. Append to `Cargo.toml` under `[dev-dependencies]`: `digstore-crypto = { path = "../digstore-crypto" }`.
- [ ] Add the dev-dependency line. In `crates/digstore-store/Cargo.toml`, under `[dev-dependencies]`, add `digstore-crypto = { path = "../digstore-crypto" }` (so the integration test can call `digstore_crypto::sha256`).
- [ ] Run: `cargo test -p digstore-store --test commit_flow resolve_chunk_reads_deduplicated_chunk_across_generations`
  Expected: FAIL — `no method named resolve_chunk found for struct Store`.
- [ ] In `src/store.rs`, add the `resolve_chunk` method to the `impl<C: Clock> Store<C>` block:
```rust
    /// Resolve a chunk's bytes by content hash across ALL generation chunk dirs.
    /// Chunk bytes are content-addressed and stored once globally (§8.2), so a
    /// chunk introduced by an earlier generation lives only under that
    /// generation's `chunks/` dir; later generations referencing it have a
    /// sparse `chunks/`. Returns `ChunkNotFound` if no generation holds it.
    pub fn resolve_chunk(&self, hash: Bytes32) -> Result<Vec<u8>> {
        let gens = self.paths.generations_dir();
        if gens.exists() {
            let name = hash.to_hex();
            for entry in std::fs::read_dir(&gens)? {
                let candidate = entry?.path().join("chunks").join(&name);
                if candidate.exists() {
                    return Ok(std::fs::read(&candidate)?);
                }
            }
        }
        Err(StoreError::ChunkNotFound(hash.to_hex()))
    }
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow resolve_chunk_reads_deduplicated_chunk_across_generations resolve_unknown_chunk_errors`
  Expected: PASS — 2 passed.
- [ ] Commit:
```
git add crates/digstore-store/Cargo.toml crates/digstore-store/src/store.rs crates/digstore-store/tests/commit_flow.rs
git commit -m "feat(store): Store::resolve_chunk for global content-addressed chunk resolution (§8.2)"
```

---

## Task 15 — `Store::log` and `Store::diff`

**Files:**
- Modify: `crates/digstore-store/src/store.rs`
- Test: extend `crates/digstore-store/tests/commit_flow.rs`

`log` (§20.4) returns generations in order (the root history). `diff(a, b)` (§20.4) loads two generations by root hash and returns a `GenerationDiff`.

Steps:

- [ ] Append the failing tests to `crates/digstore-store/tests/commit_flow.rs`:
```rust
#[test]
fn log_lists_generations_in_order() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    store.stage_file("a.txt", b"first").unwrap();
    let _r0 = store.commit().unwrap();
    store.stage_file("b.txt", b"second").unwrap();
    let _r1 = store.commit().unwrap();

    let log = store.log().unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, 0);
    assert_eq!(log[1].id, 1);
}

#[test]
fn diff_between_two_generations_reports_added_key() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    store.stage_file("a.txt", b"alpha content here padded out").unwrap();
    let r0 = store.commit().unwrap();
    store.stage_file("a.txt", b"alpha content here padded out").unwrap();
    store.stage_file("b.txt", b"a new resource entirely").unwrap();
    let r1 = store.commit().unwrap();

    let d = store.diff(r0, r1).unwrap();
    assert_eq!(d.keys_added, vec!["b.txt".to_string()]);
    assert!(d.keys_removed.is_empty());
}

#[test]
fn diff_unknown_root_errors() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    store.stage_file("a.txt", b"x").unwrap();
    let r0 = store.commit().unwrap();
    let bogus = Bytes32::from([0xEEu8; 32]);
    let err = store.diff(r0, bogus).unwrap_err();
    assert!(matches!(err, digstore_store::StoreError::GenerationNotFound(_)));
}
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow log_lists_generations_in_order`
  Expected: FAIL — `no method named log found for struct Store`.
- [ ] In `src/store.rs`, add the methods to the `impl<C: Clock> Store<C>` block:
```rust
    /// Generations in chronological order (§20.4 `log`). Alias of root history.
    pub fn log(&self) -> Result<Vec<GenerationState>> {
        self.root_history()
    }

    /// Load a generation manifest by its root hash.
    pub fn generation_manifest(&self, root: Bytes32) -> Result<GenerationManifest> {
        let path = self.paths.generation_manifest(&root.to_hex());
        if !path.exists() {
            return Err(StoreError::GenerationNotFound(root.to_hex()));
        }
        GenerationManifest::read_from(path)
    }

    /// Diff two generations by root hash (§20.4 `diff`).
    pub fn diff(&self, a: Bytes32, b: Bytes32) -> Result<crate::diff::GenerationDiff> {
        let ma = self.generation_manifest(a)?;
        let mb = self.generation_manifest(b)?;
        Ok(crate::diff::GenerationDiff::between(&ma, &mb))
    }
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow log_lists_generations_in_order diff_between_two_generations_reports_added_key diff_unknown_root_errors`
  Expected: PASS — 3 passed.
- [ ] Commit:
```
git add crates/digstore-store/src/store.rs crates/digstore-store/tests/commit_flow.rs
git commit -m "feat(store): Store::log and Store::diff over generations (§20.4)"
```

---

## Task 16 — `current_root` / `roothash_history` accessors + module path

**Files:**
- Modify: `crates/digstore-store/src/store.rs`
- Test: extend `crates/digstore-store/tests/commit_flow.rs`

These accessors feed the compiler and remote layers: the current head root, the full ordered root-hash list (the source for the guest's `get_roothash_history`), and the expected compiled-module path for a given root.

Steps:

- [ ] Append the failing test to `crates/digstore-store/tests/commit_flow.rs`:
```rust
#[test]
fn current_root_and_history_accessors() {
    let dir = tempdir().unwrap();
    let mut store = Store::init(config(dir.path()), FixedClock::new(10)).unwrap();
    assert!(store.current_root().unwrap().is_none());

    store.stage_file("a.txt", b"one").unwrap();
    let r0 = store.commit().unwrap();
    store.stage_file("b.txt", b"two").unwrap();
    let r1 = store.commit().unwrap();

    assert_eq!(store.current_root().unwrap(), Some(r1));
    assert_eq!(store.roothash_history().unwrap(), vec![r0, r1]);

    let sid_hex = "44".repeat(32);
    let expected = dir
        .path()
        .join("modules")
        .join(format!("{sid_hex}-{}.wasm", r1.to_hex()));
    assert_eq!(store.module_path(r1), expected);
}
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow current_root_and_history_accessors`
  Expected: FAIL — `no method named current_root found for struct Store`.
- [ ] In `src/store.rs`, add the accessors to the `impl<C: Clock> Store<C>` block:
```rust
    /// The current head root hash, or `None` if no generation has been committed.
    pub fn current_root(&self) -> Result<Option<Bytes32>> {
        Ok(RootHistory::open(self.paths.history_file())?
            .head()?
            .map(|g| g.root))
    }

    /// All root hashes in chronological order — the source for the guest's
    /// `get_roothash_history` export (§4.3). Consumed by `digstore-guest`.
    pub fn roothash_history(&self) -> Result<Vec<Bytes32>> {
        Ok(self.root_history()?.into_iter().map(|g| g.root).collect())
    }

    /// Deterministic path of the compiled module for a given root (§4.4):
    /// `{store_id}-{root}.wasm` under `modules/`. Consumed by `digstore-compiler`.
    pub fn module_path(&self, root: Bytes32) -> std::path::PathBuf {
        self.paths.module_file(&root.to_hex())
    }
```
- [ ] Run: `cargo test -p digstore-store --test commit_flow current_root_and_history_accessors`
  Expected: PASS.
- [ ] Commit:
```
git add crates/digstore-store/src/store.rs crates/digstore-store/tests/commit_flow.rs
git commit -m "feat(store): current_root, roothash_history, and module_path accessors"
```

---

## Task 17 — Full crate test pass, clippy, fmt

**Files:**
- (verification only; no source change expected)

Steps:

- [ ] Run: `cargo test -p digstore-store`
  Expected: PASS — all unit tests (`error`, `clock`, `paths`, `config`, `staging`, `chunkstore`, `generation`, `history`, `diff`) and both integration suites (`layout`, `commit_flow`) green. Summary lines resemble `test result: ok. N passed; 0 failed` for each binary.
- [ ] Run: `cargo clippy -p digstore-store --all-targets -- -D warnings`
  Expected: PASS — `Finished` with no warnings. If clippy reports a minor lint (e.g., `needless_borrow`), fix it minimally and re-run until clean.
- [ ] Run: `cargo fmt -p digstore-store -- --check`
  Expected: PASS (no diff). If it fails, run `cargo fmt -p digstore-store` and re-check.
- [ ] Confirm `src/lib.rs` declares exactly these modules and re-exports (the final public surface):
```rust
mod chunkstore;
mod clock;
mod config;
mod diff;
mod error;
mod generation;
mod history;
mod paths;
mod staging;
mod store;

pub use chunkstore::ChunkStore;
pub use clock::{Clock, FixedClock, SystemClock};
pub use config::{load_config, save_config};
pub use diff::GenerationDiff;
pub use error::{Result, StoreError};
pub use generation::{ChunkRef, GenerationManifest, KeyTableRecord};
pub use history::RootHistory;
pub use paths::StorePaths;
pub use staging::{StagedRecord, StagingArea};
pub use store::Store;
```
- [ ] Commit (only if fmt/clippy changed files):
```
git add crates/digstore-store/src
git commit -m "chore(store): pass clippy and fmt; finalize public re-exports"
```

---

## Definition of Done

Every **assigned** paper section maps to concrete, tested tasks:

- [ ] **§4.1 Store Configuration** — `StoreConfig`/`Visibility`/`SecretSalt` persisted and round-tripped via `config.toml`; private-store salt preserved; missing-salt rejected. (Tasks 4, 10)
- [ ] **§4.2 Store ID** — 32-byte store id curried into `StorePaths`, module filenames, and URN-derived `static_key`s; `store_id()` accessor. (Tasks 3, 10, 12)
- [ ] **§4.3 Generations and Root Hash** — `GenerationState` produced per commit; append-only **monotonic** `RootHistory`; `log`/`roothash_history`/`current_root`. (Tasks 8, 12, 15, 16)
- [ ] **§4.4 On-Disk Layout** — exact directory tree (`{store_id}.staging.bin`, `generations/{root}/manifest.json` + `chunks/{hash}`, `modules/{store_id}-{root}.wasm`, `config.toml`, `roots.log`) asserted path-by-path; sparse `chunks/` reconciled by `resolve_chunk`. (Tasks 3, 7, 10, 14)
- [ ] **§8.2 Generations (dedup + merkle + root)** — staged chunks deduplicated **globally** across all generations; per-generation merkle tree over chunk leaves; root recorded and appended; shared chunks stored once asserted by file count; global `resolve_chunk` reads deduplicated bytes; manifest `KeyTableRecord` projects to canonical `KeyTableEntry`. (Tasks 6, 7, 12, 13, 14)
- [ ] **§20.1 init** — `Store::init` generates layout + records visibility + writes config; refuses to clobber; `open` reloads. (Task 10)
- [ ] **§20.2 add** — `Store::add`/`stage_file`: path-relative-to-base becomes resource key; path-escape rejected; bytes appended to big-endian staging. (Task 11)
- [ ] **§20.3 commit (store mechanics)** — chunk, merkle, root recorded in history, generation dir written, staging cleared, deterministic root for fixed input, `static_key` matches client URL retrieval key; compilation invoked separately by `digstore-compiler` over the produced generation dir. (Tasks 12, 13)
- [ ] **§20.4 log/diff (store mechanics, adjacent to §20.3)** — `Store::log` lists generations in order; `Store::diff` reports chunk-set and resource-key deltas. (Tasks 9, 15)
- [ ] All TDD steps committed with conventional messages; `cargo test -p digstore-store`, `cargo clippy -- -D warnings`, and `cargo fmt --check` all green. (Task 17)

**Sections referenced but NOT owned here (values produced are consumed downstream):** §8.3 interleaved-pool ordering and §9.1 merkle construction feed `digstore-compiler`/`digstore-core`; §19.3 byte-identical compilation is owned by `digstore-compiler` (this crate only guarantees a deterministic root for fixed input). `get_roothash_history`/module emission are owned by `digstore-guest`/`digstore-compiler`; this crate supplies the deterministic on-disk generation they consume.

**Documented deviations surfaced in this crate:**
1. Staging-record and key-table framing use **big-endian** length prefixes (Chia streamable framing), per the locked decision, NOT the paper's "little-endian" note (Task 5).
2. URN `static_key` is **root-independent** (`root_hash: None`): a resource keeps the same lookup key across generations; asserted for client parity in Task 12.
3. The interleaved-pool *deterministic filler* (ChaCha20-keyed, deviation #2 of the project) and module emission are owned by `digstore-compiler`; this crate only produces the deterministic on-disk generation it consumes.

---

## Plan metadata

- **Crate:** digstore-store
- **Assigned paper sections:** 4.1,4.2,4.3,4.4,8.2,20.1,20.2,20.3(store mechanics)
- **Depends on:** digstore-core, digstore-chunker, digstore-crypto
- **Spec sections covered (claimed):** 4.1, 4.2, 4.3, 4.4, 8.2, 20.1, 20.2, 20.3, 20.4

### Public items exported (consumed by other crates)

```
pub enum StoreError { AlreadyExists(String), NotFound(String), InvalidConfig(String), CorruptStaging(String), GenerationNotFound(String), ChunkNotFound(String), NonMonotonicHistory { last: u64, got: u64 }, EmptyStaging, Manifest(String), Config(String), PathEscape(std::path::PathBuf), Io(std::io::Error) }
pub type Result<T> = std::result::Result<T, StoreError>;
pub trait Clock { fn unix_seconds(&self) -> u64; }
pub struct SystemClock; impl Clock for SystemClock
pub struct FixedClock; impl FixedClock { pub fn new(now: u64) -> Self; pub fn advance(&self, delta: u64); } impl Clock for FixedClock
pub struct StorePaths; impl StorePaths { pub fn new(data_dir: impl AsRef<std::path::Path>, store_id: digstore_core::Bytes32) -> Self; pub fn root(&self) -> std::path::PathBuf; pub fn config_file(&self) -> std::path::PathBuf; pub fn history_file(&self) -> std::path::PathBuf; pub fn staging_file(&self) -> std::path::PathBuf; pub fn generations_dir(&self) -> std::path::PathBuf; pub fn modules_dir(&self) -> std::path::PathBuf; pub fn generation_dir(&self, root_hex: &str) -> std::path::PathBuf; pub fn generation_manifest(&self, root_hex: &str) -> std::path::PathBuf; pub fn generation_chunks_dir(&self, root_hex: &str) -> std::path::PathBuf; pub fn chunk_file(&self, root_hex: &str, chunk_hash_hex: &str) -> std::path::PathBuf; pub fn module_file(&self, root_hex: &str) -> std::path::PathBuf; pub fn store_id_hex(&self) -> &str; }
pub fn save_config(path: impl AsRef<std::path::Path>, cfg: &digstore_core::StoreConfig) -> Result<()>
pub fn load_config(path: impl AsRef<std::path::Path>) -> Result<digstore_core::StoreConfig>
pub struct StagedRecord { pub resource_key: String, pub content: Vec<u8> }
pub struct StagingArea; impl StagingArea { pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self>; pub fn append(&mut self, resource_key: &str, content: &[u8]) -> Result<()>; pub fn records(&self) -> Result<Vec<StagedRecord>>; pub fn is_empty(&self) -> Result<bool>; pub fn clear(&mut self) -> Result<()>; }
pub struct ChunkStore; impl ChunkStore { pub fn new(chunks_dir: impl AsRef<std::path::Path>) -> Self; pub fn put(&self, hash: digstore_core::Bytes32, data: &[u8]) -> Result<bool>; pub fn contains(&self, hash: digstore_core::Bytes32) -> Result<bool>; pub fn get(&self, hash: digstore_core::Bytes32) -> Result<Vec<u8>>; pub fn count(&self) -> Result<usize>; }
pub struct ChunkRef { pub index: u32, pub hash: digstore_core::Bytes32, pub size: u64 }
pub struct KeyTableRecord { pub resource_key: String, pub static_key: digstore_core::Bytes32, pub generation: digstore_core::Bytes32, pub chunk_indices: Vec<u32>, pub total_size: u64 } impl KeyTableRecord { pub fn to_key_table_entry(&self) -> digstore_core::KeyTableEntry; }
pub struct GenerationManifest { pub schema_version: u32, pub generation_id: u64, pub root: digstore_core::Bytes32, pub timestamp: u64, pub chunks: Vec<ChunkRef>, pub key_table: Vec<KeyTableRecord> } impl GenerationManifest { pub fn to_json(&self) -> Result<String>; pub fn from_json(s: &str) -> Result<Self>; pub fn write_to(&self, path: impl AsRef<std::path::Path>) -> Result<()>; pub fn read_from(path: impl AsRef<std::path::Path>) -> Result<Self>; pub fn chunk_hashes(&self) -> std::collections::BTreeSet<digstore_core::Bytes32>; pub fn resource_keys(&self) -> std::collections::BTreeSet<String>; }
pub struct RootHistory; impl RootHistory { pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self>; pub fn entries(&self) -> Result<Vec<digstore_core::GenerationState>>; pub fn head(&self) -> Result<Option<digstore_core::GenerationState>>; pub fn next_id(&self) -> Result<u64>; pub fn append(&mut self, gen: &digstore_core::GenerationState) -> Result<()>; }
pub struct GenerationDiff { pub chunks_added: Vec<digstore_core::Bytes32>, pub chunks_removed: Vec<digstore_core::Bytes32>, pub keys_added: Vec<String>, pub keys_removed: Vec<String> } impl GenerationDiff { pub fn between(a: &GenerationManifest, b: &GenerationManifest) -> Self; pub fn is_empty(&self) -> bool; }
pub struct Store<C: Clock>; impl<C: Clock> Store<C> { pub fn init(config: digstore_core::StoreConfig, clock: C) -> Result<Self>; pub fn open(data_dir: impl AsRef<std::path::Path>, clock: C) -> Result<Self>; pub fn store_id(&self) -> digstore_core::Bytes32; pub fn config(&self) -> &digstore_core::StoreConfig; pub fn paths(&self) -> &StorePaths; pub fn root_history(&self) -> Result<Vec<digstore_core::GenerationState>>; pub fn stage_file(&mut self, resource_key: &str, bytes: &[u8]) -> Result<()>; pub fn add(&mut self, file: impl AsRef<std::path::Path>, base: impl AsRef<std::path::Path>) -> Result<()>; pub fn commit(&mut self) -> Result<digstore_core::Bytes32>; pub fn resolve_chunk(&self, hash: digstore_core::Bytes32) -> Result<Vec<u8>>; pub fn log(&self) -> Result<Vec<digstore_core::GenerationState>>; pub fn generation_manifest(&self, root: digstore_core::Bytes32) -> Result<GenerationManifest>; pub fn diff(&self, a: digstore_core::Bytes32, b: digstore_core::Bytes32) -> Result<GenerationDiff>; pub fn current_root(&self) -> Result<Option<digstore_core::Bytes32>>; pub fn roothash_history(&self) -> Result<Vec<digstore_core::Bytes32>>; pub fn module_path(&self, root: digstore_core::Bytes32) -> std::path::PathBuf; }
```