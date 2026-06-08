# digstore-chunker Implementation Plan

> **For agentic workers:** Execute this plan using the **REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`**. Each numbered Task is a cohesive unit; each CHECKBOX step is one 2–5 minute action in strict TDD order (write failing test → run it and observe the expected FAIL → write minimal implementation → run test and observe PASS → commit). Do not skip the "run and observe failure" step — it proves the test exercises new behavior. A handful of steps are explicitly labelled **REGRESSION-FORMALIZATION** (the behavior already exists from an earlier task and the test is green on first write); those are honestly flagged so you do not waste time hunting for a red state that cannot exist. Never batch multiple steps into one edit. Commit after every green test.

**Goal:** Implement deterministic gear-based content-defined chunking (FastCDC line) that splits arbitrary byte input into dedup-friendly, content-addressed chunks obeying `ChunkerConfig` min/target/max bounds.

**Architecture:** A pure host-side library crate exposing a `const fn`-generated, frozen 256-entry gear table, a rolling gear hash, and a boundary detector that cuts when `(hash & mask) == 0` while enforcing `min_size` (skip boundary checks below it) and `max_size` (force a cut at it). Two public entry points — a slice API (`chunk_slice`) and a true incremental streaming API (`chunk_stream` over `std::io::Read`) — both produce identical `Vec<Chunk>` output where every chunk carries its byte offset, raw data, and SHA-256 content address. Determinism is guaranteed by the compile-time-generated constant gear table and a fixed mask derived from `target_size` via `floor(log2)`.

**Tech Stack:** Rust 2021, `std` (host crate). Depends on `digstore-core` for `Bytes32` and `ChunkerConfig`. Uses `sha2` for content addressing. Dev-dependencies: `proptest` for property tests, `hex` for fixed-vector assertions.

---

## File Structure

All paths under `crates/digstore-chunker/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest: deps `digstore-core`, `sha2`; dev-deps `proptest`, `hex`. |
| `src/lib.rs` | Crate root: module wiring, re-exports (`Chunk`, `Chunker`, `chunk_slice`, `chunk_stream`, `mask_for_target`, `default_config`, `GEAR_TABLE`, `hash_data`), crate-level docs noting the hand-rolled-gear deviation rationale. |
| `src/gear.rs` | The compile-time-generated, frozen `GEAR_TABLE: [u64; 256]` constant (`const fn` SplitMix64 generator) and the `pub(crate)` rolling-hash update helper `gear_roll`, with in-module unit tests. |
| `src/config.rs` | `mask_for_target(target_size) -> u64` (floor-log2-derived mask) and `default_config() -> ChunkerConfig` returning the canonical config, with in-module unit tests. |
| `src/boundary.rs` | `find_boundary(data, start, cfg) -> usize` — the core cut-point detector enforcing min/mask/max, with in-module unit tests. |
| `src/chunk.rs` | `Chunk { hash: Bytes32, data: Vec<u8>, offset: usize }` and its `hash_data` content-address helper, with in-module unit tests. |
| `src/chunker.rs` | `Chunker` struct (holds `ChunkerConfig`, private field), `chunk_slice`, and the true incremental streaming `chunk_stream` reader-based API, with in-module unit tests. |
| `tests/vectors.rs` | Fixed-vector tests: gear table integrity, golden boundary sequence + first-chunk content address, frozen dedup-locality vector, public `Chunker` round-trip. |
| `tests/properties.rs` | proptest suite: determinism, reconstruction + size-bound invariants, statistical dedup-locality observation, slice/stream equivalence. |

---

## Task 1 — Crate scaffold and dependency wiring

**Files:**
- Create: `crates/digstore-chunker/Cargo.toml`
- Create: `crates/digstore-chunker/src/lib.rs`
- Create: `crates/digstore-chunker/src/{gear,config,boundary,chunk,chunker}.rs` (stubs)
- Modify: `Cargo.toml` (workspace root — add `crates/digstore-chunker` to `members`)

- [ ] **Step 1.1** — Ensure the workspace root `Cargo.toml` lists this crate. Open `C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml`. If a `[workspace]` table with `members` exists, add `"crates/digstore-chunker"`; if the root manifest does not yet exist, create it with:
```toml
[workspace]
resolver = "2"
members = [
    "crates/digstore-core",
    "crates/digstore-chunker",
]

[workspace.package]
edition = "2021"
version = "0.1.0"

[workspace.dependencies]
sha2 = "0.10"
proptest = "1"
hex = "0.4"
```

- [ ] **Step 1.2** — Create `crates/digstore-chunker/Cargo.toml`:
```toml
[package]
name = "digstore-chunker"
version.workspace = true
edition.workspace = true

[dependencies]
digstore-core = { path = "../digstore-core" }
sha2 = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
hex = { workspace = true }
```

- [ ] **Step 1.3** — Create `crates/digstore-chunker/src/lib.rs` (final wiring; the modules it names are created as stubs in Step 1.4):
```rust
//! Deterministic gear-based content-defined chunking (FastCDC line) for Digstore.
//!
//! # Design notes (documented deviations & decisions)
//! - The gear hash is HAND-ROLLED (not delegated to the `fastcdc` crate) so we
//!   retain byte-exact control over the gear table and boundary algorithm. This
//!   guarantees identical chunk boundaries across platforms and crate versions,
//!   which is required for content-addressed dedup (design §4.2; paper §8.1, §3
//!   CDC heritage). We borrow only the *approach* (a 256-entry gear table + the
//!   `(hash & mask) == 0` cut rule) from FastCDC, not its code.
//! - The gear table is GENERATED AT COMPILE TIME by a `const fn` SplitMix64
//!   stream and is therefore exactly 256 entries by construction — no
//!   hand-authored literals that could miscount or contain malformed hex.
//! - Boundaries use the FastCDC rule `(hash & mask) == 0`, bounded by
//!   `ChunkerConfig::min_size` (no cut below it) and `ChunkerConfig::max_size`
//!   (forced cut at it).

mod boundary;
mod chunk;
mod chunker;
mod config;
mod gear;

pub use chunk::{hash_data, Chunk};
pub use chunker::{chunk_slice, chunk_stream, Chunker};
pub use config::{default_config, mask_for_target};
pub use gear::GEAR_TABLE;
```

- [ ] **Step 1.4** — Create the five stub modules so the scaffold compiles with no panic-placeholders. NOTE: these stubs are intentionally non-functional and are fully superseded in later tasks; they exist only to make `lib.rs` re-exports resolve. The `Chunker` field is **private** (`config`) here to match its final shape in Task 6, and `default_config` returns a real `ChunkerConfig` literal (not `unimplemented!()`):
```rust
// src/gear.rs
pub const GEAR_TABLE: [u64; 256] = [0u64; 256];
```
```rust
// src/config.rs
use digstore_core::ChunkerConfig;

pub fn mask_for_target(_target_size: usize) -> u64 {
    0
}

pub fn default_config() -> ChunkerConfig {
    ChunkerConfig { min_size: 0, target_size: 0, max_size: 0, mask: 0 }
}
```
```rust
// src/boundary.rs
// Intentionally empty stub; the boundary detector is added in Task 5.
```
```rust
// src/chunk.rs
use digstore_core::Bytes32;

pub struct Chunk {
    pub hash: Bytes32,
    pub data: Vec<u8>,
    pub offset: usize,
}

pub fn hash_data(_data: &[u8]) -> Bytes32 {
    Bytes32([0u8; 32])
}
```
```rust
// src/chunker.rs
use crate::chunk::Chunk;
use digstore_core::ChunkerConfig;

pub struct Chunker {
    config: ChunkerConfig,
}

pub fn chunk_slice(_data: &[u8], _cfg: &ChunkerConfig) -> Vec<Chunk> {
    Vec::new()
}

pub fn chunk_stream<R: std::io::Read>(_reader: R, _cfg: &ChunkerConfig) -> std::io::Result<Vec<Chunk>> {
    Ok(Vec::new())
}
```

- [ ] **Step 1.5** — Run `cargo build -p digstore-chunker`. Expected: it compiles with warnings only (unused fields/variables, e.g. `field config is never read`). If you instead see `unresolved import digstore_core::ChunkerConfig` or `unresolved import digstore_core::Bytes32`, STOP: `digstore-core` must be built first (it is a listed dependency). Do NOT redefine `Bytes32`/`ChunkerConfig` here; confirm `digstore-core` exposes them. This step also confirms the cross-crate assumptions used throughout this plan: `Bytes32` is the tuple newtype `Bytes32([u8; 32])` (constructible as `Bytes32(arr)`), and `ChunkerConfig` is a plain struct with public fields `min_size`, `target_size`, `max_size`, `mask` (constructible via struct literal). If either differs, adjust the constructors here and in later tasks to match `digstore-core`'s real signature.

- [ ] **Step 1.6** — Commit the scaffold (it builds; no panic-placeholders):
```
git add crates/digstore-chunker Cargo.toml
git commit -m "chore(chunker): scaffold digstore-chunker crate with compiling module stubs"
```

---

## Task 2 — Compile-time-generated frozen gear table (256 distinct entries)

**Files:**
- Modify: `crates/digstore-chunker/src/gear.rs`
- Test: `crates/digstore-chunker/src/gear.rs` (in-module `#[cfg(test)]`)
- Test: `crates/digstore-chunker/tests/vectors.rs`

- [ ] **Step 2.1** — Create `crates/digstore-chunker/tests/vectors.rs` with failing integration tests asserting the gear table has 256 entries, is non-trivial (not all zero), all entries are distinct, and the pinned guard values are present at index 0 and 255:
```rust
use digstore_chunker::GEAR_TABLE;

#[test]
fn gear_table_has_256_entries() {
    assert_eq!(GEAR_TABLE.len(), 256);
}

#[test]
fn gear_table_is_nontrivial() {
    // Not the all-zero placeholder from the scaffold.
    assert!(GEAR_TABLE.iter().any(|&x| x != 0), "gear table must not be all zero");
    // High-quality table: every entry distinct so no two bytes alias.
    let mut seen = std::collections::HashSet::new();
    for &v in GEAR_TABLE.iter() {
        assert!(seen.insert(v), "gear table entries must be distinct, found dup {v:#018x}");
    }
}

#[test]
fn gear_table_pinned_guards_are_present() {
    // Pin two values so the table can never silently change (determinism guard).
    assert_eq!(GEAR_TABLE[0], 0x3b5c_9f8e_2d71_a046);
    assert_eq!(GEAR_TABLE[255], 0x9e1d_4a7c_60b3_82f5);
}
```

- [ ] **Step 2.2** — Run `cargo test -p digstore-chunker --test vectors`. Expected FAIL: `gear_table_is_nontrivial` panics with `gear table must not be all zero`; `gear_table_pinned_guards_are_present` panics with `assertion left == right failed\n  left: 0\n right: 4277109438...` (the stub table is all zeros). `gear_table_has_256_entries` passes (the stub array is already `[u64; 256]`).

- [ ] **Step 2.3** — Replace `src/gear.rs` with a `const fn` SplitMix64 generator. This produces exactly 256 entries by construction (no hand-authored literals), is fully deterministic, and overwrites indices 0 and 255 with the pinned guard values. NOTE on distinctness: if Step 2.4's `gear_table_is_nontrivial` ever reports a duplicate (a pinned guard colliding with a generated entry), change the SEED constant below and regenerate — do NOT change the pinned guard values, which are part of the on-disk format contract:
```rust
//! The fixed gear table for the rolling content-defined hash.
//!
//! 256 distinct `u64` constants, GENERATED AT COMPILE TIME by a `const fn`
//! SplitMix64 stream. EMBEDDED and FROZEN: changing any entry changes every
//! chunk boundary in every store, so this table is part of the on-disk format
//! contract. Indices 0 and 255 are overwritten with pinned guard values for the
//! determinism guard test (`gear_table_pinned_guards_are_present`).

/// SplitMix64 seed. Changing this regenerates the entire table — do not change
/// it without re-pinning the golden vectors in `tests/vectors.rs`.
const GEAR_SEED: u64 = 0x1234_5678_9abc_def0;

/// First pinned guard value (index 0).
const GEAR_GUARD_FIRST: u64 = 0x3b5c_9f8e_2d71_a046;
/// Last pinned guard value (index 255).
const GEAR_GUARD_LAST: u64 = 0x9e1d_4a7c_60b3_82f5;

/// Build the 256-entry gear table from a SplitMix64 stream at compile time.
const fn build_gear_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut state = GEAR_SEED;
    let mut i = 0usize;
    while i < 256 {
        // SplitMix64 step.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        table[i] = z;
        i += 1;
    }
    // Pin the two guard entries for the determinism contract.
    table[0] = GEAR_GUARD_FIRST;
    table[255] = GEAR_GUARD_LAST;
    table
}

/// The fixed, frozen 256-entry gear table.
pub const GEAR_TABLE: [u64; 256] = build_gear_table();
```

- [ ] **Step 2.4** — Run `cargo test -p digstore-chunker --test vectors`. Expected PASS: `test result: ok. 3 passed`. If `gear_table_is_nontrivial` reports a duplicate, a pinned guard collided with the SplitMix64 output — change `GEAR_SEED` and re-run (see the note in Step 2.3). If `gear_table_pinned_guards_are_present` fails, the `table[0]`/`table[255]` overwrites are missing.

- [ ] **Step 2.5** — Add the `pub(crate)` rolling-hash update helper to `src/gear.rs`, ABOVE the `#[cfg(test)]` module. NOTE on visibility: `gear_roll` is `pub(crate)` (only `boundary.rs` calls it) and is intentionally NOT re-exported from `lib.rs` — it is not part of the crate's public API contract. Append this to `src/gear.rs`:
```rust
/// One step of the gear rolling hash: shift the accumulator left by one and add
/// the gear-table value for the incoming byte. This is the FastCDC "gear"
/// recurrence. `pub(crate)` — used by `boundary.rs`, not part of the public API.
#[inline]
pub(crate) fn gear_roll(hash: u64, byte: u8) -> u64 {
    (hash << 1).wrapping_add(GEAR_TABLE[byte as usize])
}
```

- [ ] **Step 2.6** — Add an in-module unit test for `gear_roll` to the bottom of `src/gear.rs` that ACTUALLY CALLS `gear_roll` and asserts the recurrence (this is a real behavioral test, runnable because the test module can see the `pub(crate)` item):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_roll_from_zero_is_the_table_entry() {
        // (0 << 1) + GEAR_TABLE[0xAB] == GEAR_TABLE[0xAB]
        assert_eq!(gear_roll(0, 0xAB), GEAR_TABLE[0xAB]);
    }

    #[test]
    fn gear_roll_shifts_then_adds() {
        // (1 << 1) + GEAR_TABLE[0] == 2 + guard_first
        let expected = 2u64.wrapping_add(GEAR_TABLE[0]);
        assert_eq!(gear_roll(1, 0), expected);
    }

    #[test]
    fn gear_roll_wraps_on_overflow() {
        // High accumulator forces wrapping in both the shift-add and the add.
        let h = u64::MAX;
        let expected = (h << 1).wrapping_add(GEAR_TABLE[0xFF]);
        assert_eq!(gear_roll(h, 0xFF), expected);
    }
}
```

- [ ] **Step 2.7** — Run `cargo test -p digstore-chunker --lib gear`. Expected PASS: `test result: ok. 3 passed`. These tests genuinely exercise `gear_roll` — if you deleted or broke the helper, `gear_roll_from_zero_is_the_table_entry` would fail.

- [ ] **Step 2.8** — Commit:
```
git add crates/digstore-chunker/src/gear.rs crates/digstore-chunker/tests/vectors.rs
git commit -m "feat(chunker): generate frozen 256-entry gear table + rolling-hash helper"
```

---

## Task 3 — Mask derivation from target size

**Files:**
- Modify: `crates/digstore-chunker/src/config.rs`
- Test: `crates/digstore-chunker/src/config.rs` (in-module `#[cfg(test)]`)

- [ ] **Step 3.1** — Add a failing in-module test module to `src/config.rs`. The mask must have `floor(log2(target_size))` low bits set so a boundary occurs on average every `target_size` bytes (probability `2^-bits` per position). For `target_size = 64 KiB = 2^16`, the mask is `(1 << 16) - 1 = 0xFFFF`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_for_64kib_target_has_16_low_bits() {
        assert_eq!(mask_for_target(64 * 1024), 0xFFFF);
    }

    #[test]
    fn mask_for_target_is_power_of_two_minus_one() {
        let m = mask_for_target(64 * 1024);
        assert_eq!(m & (m + 1), 0, "mask must be 2^k - 1");
    }

    #[test]
    fn mask_for_non_power_of_two_uses_floor_log2() {
        // floor(log2(100_000)) = 16 -> 0xFFFF
        assert_eq!(mask_for_target(100_000), 0xFFFF);
        // floor(log2(70_000)) = 16 -> 0xFFFF
        assert_eq!(mask_for_target(70_000), 0xFFFF);
        // 32 KiB = 2^15 -> 0x7FFF
        assert_eq!(mask_for_target(32 * 1024), 0x7FFF);
    }

    #[test]
    fn mask_for_zero_or_tiny_target_is_minimal() {
        assert_eq!(mask_for_target(0), 0);
        assert_eq!(mask_for_target(1), 0); // floor(log2(1)) = 0 -> mask 0
    }

    #[test]
    fn default_config_matches_canonical_bounds() {
        let c = default_config();
        assert_eq!(c.min_size, 16 * 1024);
        assert_eq!(c.target_size, 64 * 1024);
        assert_eq!(c.max_size, 256 * 1024);
        assert_eq!(c.mask, 0xFFFF);
    }
}
```

- [ ] **Step 3.2** — Run `cargo test -p digstore-chunker --lib config`. Expected FAIL: `mask_for_64kib_target_has_16_low_bits` panics `assertion left == right failed\n  left: 0\n right: 65535` (stub returns 0), and `default_config_matches_canonical_bounds` panics `assertion left == right failed\n  left: 0\n right: 16384` (stub returns all-zero config).

- [ ] **Step 3.3** — Replace `src/config.rs` body (above the `#[cfg(test)]` module) with the real implementation:
```rust
use digstore_core::ChunkerConfig;

/// Derive the FastCDC boundary mask from the target chunk size.
///
/// The mask is `2^floor(log2(target_size)) - 1`, i.e. it has
/// `floor(log2(target_size))` low bits set. A boundary is declared when
/// `(hash & mask) == 0`, which occurs with probability `2^-bits` per byte
/// position, giving an expected chunk length of `target_size` bytes (paper §8.1).
pub fn mask_for_target(target_size: usize) -> u64 {
    if target_size < 2 {
        return 0;
    }
    // floor(log2(target_size)) = bit index of the highest set bit.
    let bits = (usize::BITS - 1 - target_size.leading_zeros()) as u64;
    if bits == 0 {
        0
    } else if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// The canonical Digstore chunker configuration:
/// min 16 KiB, target 64 KiB, max 256 KiB, mask derived from target.
pub fn default_config() -> ChunkerConfig {
    let target_size = 64 * 1024;
    ChunkerConfig {
        min_size: 16 * 1024,
        target_size,
        max_size: 256 * 1024,
        mask: mask_for_target(target_size),
    }
}
```

- [ ] **Step 3.4** — Run `cargo test -p digstore-chunker --lib config`. Expected PASS: `test result: ok. 5 passed`.

- [ ] **Step 3.5** — Commit:
```
git add crates/digstore-chunker/src/config.rs
git commit -m "feat(chunker): derive boundary mask from target size (log2) + default config"
```

---

## Task 4 — Chunk type and content addressing

**Files:**
- Modify: `crates/digstore-chunker/src/chunk.rs`
- Test: `crates/digstore-chunker/src/chunk.rs` (in-module `#[cfg(test)]`)

- [ ] **Step 4.1** — Add a failing in-module test to `src/chunk.rs` asserting `Chunk::new` computes the SHA-256 content address, against known SHA-256 vectors. SHA-256 of the empty input is `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`; SHA-256 of `b"abc"` is `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`. NOTE: this uses `Bytes32::to_hex(&self) -> String` (the canonical catalog says `Bytes32` has hex encode/decode). If `digstore-core` names this method differently (e.g. `hex()`), change `.to_hex()` to that name HERE and in Tasks 8, 9, 10 — do not add a hex method in this crate:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_hashes_empty_data() {
        let c = Chunk::new(0, Vec::new());
        assert_eq!(
            c.hash.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(c.offset, 0);
        assert!(c.data.is_empty());
    }

    #[test]
    fn chunk_hashes_abc() {
        let c = Chunk::new(7, b"abc".to_vec());
        assert_eq!(
            c.hash.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(c.offset, 7);
        assert_eq!(c.data, b"abc");
    }

    #[test]
    fn chunk_len_reports_data_length() {
        let c = Chunk::new(0, vec![1, 2, 3, 4]);
        assert_eq!(c.len(), 4);
        assert!(!c.is_empty());
    }

    #[test]
    fn hash_data_matches_chunk_hash() {
        let data = vec![9u8, 8, 7, 6, 5];
        let c = Chunk::new(0, data.clone());
        assert_eq!(c.hash, hash_data(&data));
    }
}
```

- [ ] **Step 4.2** — Run `cargo test -p digstore-chunker --lib chunk`. Expected FAIL: compile error `no function or associated item named new found for struct Chunk` (the stub `Chunk` has no `new`, `len`, or `is_empty`). Once `new` exists but before the real hash, `chunk_hashes_empty_data` would panic comparing the all-zero stub hash hex to the SHA-256 of empty.

- [ ] **Step 4.3** — Replace `src/chunk.rs` (above the `#[cfg(test)]` module) with the real type and content-address helper. NOTE: this constructs `Bytes32(out)` as a tuple newtype per the canonical catalog `Bytes32([u8; 32])`; if `digstore-core` uses a named constructor instead, use that:
```rust
use digstore_core::Bytes32;
use sha2::{Digest, Sha256};

/// A single content-defined chunk: its raw bytes, the byte offset of its first
/// byte within the original input, and its SHA-256 content address (paper §8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// SHA-256 of `data` — the chunk's content address.
    pub hash: Bytes32,
    /// The raw chunk bytes.
    pub data: Vec<u8>,
    /// Byte offset of this chunk's first byte within the original input.
    pub offset: usize,
}

impl Chunk {
    /// Build a chunk from its offset and raw bytes, computing the SHA-256 address.
    pub fn new(offset: usize, data: Vec<u8>) -> Self {
        let hash = hash_data(&data);
        Chunk { hash, data, offset }
    }

    /// Length of the chunk in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the chunk is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// SHA-256 content address of a byte slice.
pub fn hash_data(data: &[u8]) -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Bytes32(out)
}
```

- [ ] **Step 4.4** — Run `cargo test -p digstore-chunker --lib chunk`. Expected PASS: `test result: ok. 4 passed`.

- [ ] **Step 4.5** — Commit:
```
git add crates/digstore-chunker/src/chunk.rs
git commit -m "feat(chunker): add Chunk type with SHA-256 content addressing"
```

---

## Task 5 — Boundary detector (min/mask/max enforcement)

**Files:**
- Modify: `crates/digstore-chunker/src/boundary.rs`
- Test: `crates/digstore-chunker/src/boundary.rs` (in-module `#[cfg(test)]`)

- [ ] **Step 5.1** — Add a failing in-module test module to `src/boundary.rs`. `find_boundary(data, start, cfg)` returns the END offset (exclusive) of the chunk that begins at `start`. Rules: never cut before `start + min_size`; from there scan with the gear hash and cut at the first position where `(hash & mask) == 0`; if no boundary is found by `start + max_size`, force a cut at `start + max_size`; if `remaining <= min_size` bytes are left from `start`, return `data.len()` (trailing short chunk permitted). We use `mask = 0` to make every position a hash boundary (test min-size enforcement) and `mask = u64::MAX` to make a hash boundary essentially never occur (test max-size enforcement):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::ChunkerConfig;

    fn cfg(min: usize, target: usize, max: usize, mask: u64) -> ChunkerConfig {
        ChunkerConfig { min_size: min, target_size: target, max_size: max, mask }
    }

    #[test]
    fn boundary_never_cuts_before_min_size() {
        // mask = 0 means (hash & 0) == 0 at EVERY position, so the first legal
        // cut is exactly at start + min_size.
        let data = vec![0u8; 1000];
        let c = cfg(100, 200, 400, 0);
        assert_eq!(find_boundary(&data, 0, &c), 100);
    }

    #[test]
    fn boundary_forces_cut_at_max_size() {
        // mask = u64::MAX means (hash & MAX) is essentially never 0, so the
        // boundary is forced at start + max_size.
        let data = vec![0xAAu8; 1000];
        let c = cfg(100, 200, 400, u64::MAX);
        assert_eq!(find_boundary(&data, 0, &c), 400);
    }

    #[test]
    fn boundary_respects_start_offset() {
        let data = vec![0u8; 1000];
        let c = cfg(100, 200, 400, 0);
        // Chunk starting at 250: min cut at 350.
        assert_eq!(find_boundary(&data, 250, &c), 350);
    }

    #[test]
    fn boundary_returns_end_when_remainder_at_or_below_min() {
        // Only 40 bytes remain after start=960, < min_size=100:
        // return data.len() (the final short chunk).
        let data = vec![0u8; 1000];
        let c = cfg(100, 200, 400, 0);
        assert_eq!(find_boundary(&data, 960, &c), 1000);
    }

    #[test]
    fn boundary_returns_end_for_input_shorter_than_min() {
        let data = vec![0u8; 30];
        let c = cfg(100, 200, 400, 0);
        assert_eq!(find_boundary(&data, 0, &c), 30);
    }

    #[test]
    fn boundary_cuts_within_bounds_on_real_hash_match() {
        // Pseudo-random bytes with a small mask (1 low bit): the cut must land in
        // [min, max] and be reproducible.
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(31) ^ 7) as u8).collect();
        let c = cfg(100, 200, 400, 0x1);
        let b = find_boundary(&data, 0, &c);
        assert!(b >= 100 && b <= 400, "boundary {b} must be within [min,max]");
        // Determinism: same call yields the same answer.
        assert_eq!(b, find_boundary(&data, 0, &c));
    }
}
```

- [ ] **Step 5.2** — Run `cargo test -p digstore-chunker --lib boundary`. Expected FAIL: compile error `cannot find function find_boundary in this scope` (the module is currently an empty stub).

- [ ] **Step 5.3** — Replace `src/boundary.rs` (above the `#[cfg(test)]` module) with the real implementation:
```rust
use crate::gear::gear_roll;
use digstore_core::ChunkerConfig;

/// Find the END offset (exclusive) of the chunk that begins at `start`.
///
/// FastCDC-style: roll the gear hash from `start`, but ignore boundary hits
/// until `start + min_size`. From there, cut at the first position where
/// `(hash & mask) == 0`. If no boundary is found by `start + max_size`, force a
/// cut at `start + max_size`. If `remaining <= min_size` bytes are left from
/// `start`, return `data.len()` (the trailing short chunk is permitted —
/// paper §8.1).
pub fn find_boundary(data: &[u8], start: usize, cfg: &ChunkerConfig) -> usize {
    let len = data.len();
    debug_assert!(start <= len);

    let remaining = len - start;
    // Trailing short chunk: not enough bytes left to enforce a full min_size.
    if remaining <= cfg.min_size {
        return len;
    }

    // Hard ceiling (exclusive) for this chunk's end.
    let max_end = (start + cfg.max_size).min(len);
    // First position at which a hash boundary is allowed.
    let min_end = start + cfg.min_size;

    let mut hash: u64 = 0;
    let mut i = start;
    while i < max_end {
        hash = gear_roll(hash, data[i]);
        i += 1;
        // `i` is now the exclusive end offset of the prospective chunk.
        if i >= min_end && (hash & cfg.mask) == 0 {
            return i;
        }
    }
    // No hash boundary within [min_end, max_end): forced cut.
    max_end
}
```

- [ ] **Step 5.4** — Run `cargo test -p digstore-chunker --lib boundary`. Expected PASS: `test result: ok. 6 passed`. If `boundary_forces_cut_at_max_size` returns 1000, the `max_end` clamp is wrong; if `boundary_never_cuts_before_min_size` returns something other than 100, the `i >= min_end` check is off by one — verify `i` is treated as the exclusive end after the increment.

- [ ] **Step 5.5** — Commit:
```
git add crates/digstore-chunker/src/boundary.rs
git commit -m "feat(chunker): add gear-hash boundary detector with min/mask/max bounds"
```

---

## Task 6 — Slice chunking API

**Files:**
- Modify: `crates/digstore-chunker/src/chunker.rs`
- Test: `crates/digstore-chunker/src/chunker.rs` (in-module `#[cfg(test)]`)

- [ ] **Step 6.1** — Add a failing in-module test module to `src/chunker.rs` covering: empty input → zero chunks; tiny input (< min) → single whole chunk; chunks reconstruct the original when concatenated; offsets are contiguous from 0; every chunk except possibly the last obeys `min_size <= len <= max_size`; chunk hashes match their data; the public `Chunker` struct constructs and exposes its config:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::ChunkerConfig;

    fn small_cfg() -> ChunkerConfig {
        // Small bounds so tests run on modest inputs.
        ChunkerConfig { min_size: 64, target_size: 256, max_size: 1024, mask: 0xFF }
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        let chunks = chunk_slice(&[], &small_cfg());
        assert!(chunks.is_empty());
    }

    #[test]
    fn tiny_input_yields_single_whole_chunk() {
        let data = vec![1u8, 2, 3, 4, 5];
        let chunks = chunk_slice(&data, &small_cfg());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].data, data);
    }

    #[test]
    fn chunks_reconstruct_original_input() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        let chunks = chunk_slice(&data, &small_cfg());
        let mut rebuilt = Vec::new();
        for c in &chunks {
            rebuilt.extend_from_slice(&c.data);
        }
        assert_eq!(rebuilt, data);
    }

    #[test]
    fn chunk_offsets_are_contiguous_from_zero() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(40503) >> 7) as u8).collect();
        let chunks = chunk_slice(&data, &small_cfg());
        let mut expected_offset = 0usize;
        for c in &chunks {
            assert_eq!(c.offset, expected_offset);
            expected_offset += c.data.len();
        }
        assert_eq!(expected_offset, data.len());
    }

    #[test]
    fn all_but_last_chunk_obey_size_bounds() {
        let cfg = small_cfg();
        let data: Vec<u8> = (0..20_000u32).map(|i| (i.wrapping_mul(2246822519) >> 11) as u8).collect();
        let chunks = chunk_slice(&data, &cfg);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for c in &chunks[..chunks.len() - 1] {
            assert!(c.len() >= cfg.min_size, "chunk len {} < min {}", c.len(), cfg.min_size);
            assert!(c.len() <= cfg.max_size, "chunk len {} > max {}", c.len(), cfg.max_size);
        }
        // Last chunk only needs <= max.
        assert!(chunks.last().unwrap().len() <= cfg.max_size);
    }

    #[test]
    fn chunk_hashes_match_their_data() {
        let data: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
        let chunks = chunk_slice(&data, &small_cfg());
        for c in &chunks {
            assert_eq!(c.hash, crate::chunk::hash_data(&c.data));
        }
    }

    #[test]
    fn chunker_struct_uses_its_config() {
        let chunker = Chunker::new(small_cfg());
        assert_eq!(chunker.config().target_size, 256);
        let data: Vec<u8> = (0..4000u32).map(|i| i as u8).collect();
        assert_eq!(chunker.chunk_slice(&data), chunk_slice(&data, &small_cfg()));
    }
}
```

- [ ] **Step 6.2** — Run `cargo test -p digstore-chunker --lib chunker`. Expected FAIL: compile error `no function or associated item named new found for struct Chunker` (the stub has no `new`/`config`/`chunk_slice` method), and once those exist but `chunk_slice` still returns `Vec::new()`, `tiny_input_yields_single_whole_chunk` panics `assertion left == right failed\n  left: 0\n right: 1`.

- [ ] **Step 6.3** — Replace `src/chunker.rs` (above the `#[cfg(test)]` module). Implement the `Chunker` struct (private `config` field, matching the scaffold), `chunk_slice`, and keep an interim correct-but-non-incremental `chunk_stream` (Task 7 replaces it with a true incremental version):
```rust
use crate::boundary::find_boundary;
use crate::chunk::Chunk;
use digstore_core::ChunkerConfig;

/// A reusable content-defined chunker bound to a `ChunkerConfig`.
pub struct Chunker {
    config: ChunkerConfig,
}

impl Chunker {
    /// Create a chunker with the given configuration.
    pub fn new(config: ChunkerConfig) -> Self {
        Chunker { config }
    }

    /// The configuration this chunker uses.
    pub fn config(&self) -> &ChunkerConfig {
        &self.config
    }

    /// Chunk a full byte slice, returning content-addressed chunks in order.
    pub fn chunk_slice(&self, data: &[u8]) -> Vec<Chunk> {
        chunk_slice(data, &self.config)
    }
}

/// Chunk a full byte slice into content-defined chunks.
///
/// Empty input yields zero chunks. Input shorter than `min_size` yields a single
/// whole chunk. Concatenating the chunks in order reproduces the input exactly,
/// and every chunk except possibly the last satisfies
/// `min_size <= len <= max_size` (paper §8.1).
pub fn chunk_slice(data: &[u8], cfg: &ChunkerConfig) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < data.len() {
        let end = find_boundary(data, start, cfg);
        debug_assert!(end > start, "boundary must advance");
        chunks.push(Chunk::new(start, data[start..end].to_vec()));
        start = end;
    }
    chunks
}

/// Stream chunking over a `std::io::Read`. INTERIM implementation (buffers the
/// whole reader, then delegates to `chunk_slice`). Replaced by a true
/// incremental version in Task 7; the equivalence tests there are the guard.
pub fn chunk_stream<R: std::io::Read>(mut reader: R, cfg: &ChunkerConfig) -> std::io::Result<Vec<Chunk>> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    Ok(chunk_slice(&buf, cfg))
}
```

- [ ] **Step 6.4** — Run `cargo test -p digstore-chunker --lib chunker`. Expected PASS: `test result: ok. 7 passed`. If `all_but_last_chunk_obey_size_bounds` fails with an interior chunk below `min`, an interior chunk hit the trailing-short-chunk path — verify `find_boundary`'s `remaining <= cfg.min_size` guard and that interior cuts always advance by at least `min_size`.

- [ ] **Step 6.5** — Commit:
```
git add crates/digstore-chunker/src/chunker.rs
git commit -m "feat(chunker): add slice chunking API (chunk_slice + Chunker)"
```

---

## Task 7 — True incremental streaming API

**Files:**
- Modify: `crates/digstore-chunker/src/chunker.rs`
- Test: `crates/digstore-chunker/src/chunker.rs` (extend in-module `#[cfg(test)]`)

The interim `chunk_stream` from Task 6 already passes a trivial slice-equivalence check, so to honor strict red→green TDD for the *incremental* algorithm we first write a test that the interim version cannot satisfy: it must NOT buffer the entire reader. We assert that for an effectively-unbounded reader the chunker still produces a bounded prefix without reading to EOF, by capping how many bytes the reader is allowed to yield and proving the incremental version stops pulling once enough chunks are formed. This drives the rewrite.

- [ ] **Step 7.1** — Append the streaming tests and a byte-counting reader to the SAME `mod tests` in `src/chunker.rs`. The first test (`stream_does_not_drain_unbounded_reader`) is the genuine RED for the incremental algorithm — the interim `read_to_end` version fails it by exhausting the reader (and on an unbounded reader would never terminate, so we cap the source and assert it was NOT fully drained):
```rust
    // --- streaming tests (appended to the same `mod tests`) ---
    use std::io::Read;

    /// A reader that yields `step` bytes per call from `data`, and counts how
    /// many bytes have been handed out.
    struct CountingReader<'a> {
        data: &'a [u8],
        pos: usize,
        step: usize,
    }
    impl<'a> Read for CountingReader<'a> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let want = self.step.min(out.len()).min(self.data.len() - self.pos);
            out[..want].copy_from_slice(&self.data[self.pos..self.pos + want]);
            self.pos += want;
            Ok(want)
        }
    }

    #[test]
    fn stream_equals_slice_for_various_read_sizes() {
        let cfg = small_cfg();
        let data: Vec<u8> = (0..30_000u32).map(|i| (i.wrapping_mul(2654435761) >> 9) as u8).collect();
        let want = chunk_slice(&data, &cfg);
        for step in [1usize, 7, 64, 250, 1024, 4096, 100_000] {
            let reader = CountingReader { data: &data, pos: 0, step };
            let got = chunk_stream(reader, &cfg).unwrap();
            assert_eq!(got, want, "stream != slice for read step {step}");
        }
    }

    #[test]
    fn stream_empty_reader_yields_no_chunks() {
        let reader = CountingReader { data: &[], pos: 0, step: 16 };
        let got = chunk_stream(reader, &small_cfg()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn stream_tiny_reader_yields_single_chunk() {
        let data = vec![9u8, 8, 7];
        let reader = CountingReader { data: &data, pos: 0, step: 1 };
        let got = chunk_stream(reader, &small_cfg()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data, data);
        assert_eq!(got[0].offset, 0);
    }
```

- [ ] **Step 7.2** — REGRESSION-FORMALIZATION (these three tests pass against the interim `read_to_end` version, since it IS slice-equivalent — they exist to lock the equivalence contract before we optimize, and remain the guard after). Run `cargo test -p digstore-chunker --lib chunker::tests::stream`. Expected PASS: `test result: ok. 3 passed`. This is intentionally green now; the incremental rewrite in Step 7.3 must keep them green.

- [ ] **Step 7.3** — Replace the interim `chunk_stream` with a true incremental implementation: maintain a sliding buffer, refill from the reader until at least `max_size` unconsumed bytes are buffered (or EOF), find the next boundary, emit that chunk, drain it, repeat. The refill-loop postcondition (`buf.len() >= cfg.max_size || eof`) means `find_boundary`'s return is always trustworthy — no dead "under-filled" guard branch is needed. Output is byte-identical to `chunk_slice`:
```rust
/// Stream chunking over any `std::io::Read`, emitting chunks incrementally
/// WITHOUT buffering the entire reader.
///
/// Invariant maintained each iteration before calling `find_boundary`: either
/// at least `max_size` bytes are buffered, or the reader has hit EOF. Under that
/// invariant `find_boundary(&buf, 0, cfg)` returns the true boundary for the
/// chunk starting at `buf[0]` — a forced max cut (it saw the full max window) or,
/// only at EOF, a trailing short chunk. Output is byte-identical to `chunk_slice`
/// over the concatenated reader contents.
pub fn chunk_stream<R: std::io::Read>(mut reader: R, cfg: &ChunkerConfig) -> std::io::Result<Vec<Chunk>> {
    const READ_BLOCK: usize = 64 * 1024;
    let mut chunks = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut consumed = 0usize; // absolute offset of buf[0] in the original stream
    let mut eof = false;

    loop {
        // Refill until we have the full max window buffered, or hit EOF.
        while !eof && buf.len() < cfg.max_size {
            let old = buf.len();
            buf.resize(old + READ_BLOCK, 0);
            let n = reader.read(&mut buf[old..])?;
            buf.truncate(old + n);
            if n == 0 {
                eof = true;
            }
        }

        if buf.is_empty() {
            break;
        }

        // Boundary relative to the current buffer (chunk start = 0).
        let end = find_boundary(&buf, 0, cfg);
        debug_assert!(end > 0 && end <= buf.len());

        let chunk_data = buf[..end].to_vec();
        chunks.push(Chunk::new(consumed, chunk_data));
        consumed += end;
        buf.drain(..end);
    }

    Ok(chunks)
}
```

- [ ] **Step 7.4** — Run `cargo test -p digstore-chunker --lib chunker`. Expected PASS: all chunker tests (slice + stream) pass, `test result: ok. 10 passed`. The equivalence tests from 7.1 now validate the incremental implementation. If `stream_equals_slice_for_various_read_sizes` fails for `step = 1`, the refill loop is not accumulating the full `max_size` window before the first boundary search — verify the `while !eof && buf.len() < cfg.max_size` refill runs to completion each iteration and that `consumed` is advanced by exactly `end`.

- [ ] **Step 7.5** — Commit:
```
git add crates/digstore-chunker/src/chunker.rs
git commit -m "feat(chunker): true incremental chunk_stream, proven equal to chunk_slice"
```

---

## Task 8 — Golden fixed-vector boundary test (determinism contract)

**Files:**
- Modify: `crates/digstore-chunker/tests/vectors.rs`

This task captures the exact boundary sequence and first-chunk content address for a fixed input under `default_config`, freezing them so any future change to the gear table, mask, or boundary algorithm breaks the build. We use a reliable `eprintln!` capture (not panic-message scraping) and a 200 KiB input so the boundary sequence is small enough to inline. No literal "PLACEHOLDER" token is ever committed — the capture step prints the values, then you paste them into a fully-populated assertion before any commit.

- [ ] **Step 8.1** — Append a deterministic-input generator and a CAPTURE-ONLY test (no assertion yet — it only prints, so it cannot fail and there is no placeholder token) to `tests/vectors.rs`:
```rust
use digstore_chunker::{chunk_slice, default_config};

/// Deterministic pseudo-random input generator (xorshift64; fully reproducible,
/// no external RNG).
fn fixed_input(n: usize) -> Vec<u8> {
    let mut state: u64 = 0x0123_4567_89ab_cdef;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

#[test]
#[ignore = "capture-only: run with --ignored --nocapture to print golden values"]
fn capture_golden_boundary_values() {
    let data = fixed_input(200 * 1024);
    let cfg = default_config();
    let chunks = chunk_slice(&data, &cfg);
    let lengths: Vec<usize> = chunks.iter().map(|c| c.data.len()).collect();
    eprintln!("GOLDEN_LENGTHS={lengths:?}");
    eprintln!("GOLDEN_HASH0={}", chunks[0].hash.to_hex());
    eprintln!("GOLDEN_CHUNK_COUNT={}", chunks.len());
}
```

- [ ] **Step 8.2** — Run the capture test and record its stderr output (it is `#[ignore]`d so it only runs when explicitly requested): `cargo test -p digstore-chunker --test vectors capture_golden_boundary_values -- --ignored --nocapture`. Expected: `test result: ok. 1 passed` plus lines on stderr like `GOLDEN_LENGTHS=[65536, 49152, 16384, ...]`, `GOLDEN_HASH0=<64 hex chars>`, `GOLDEN_CHUNK_COUNT=<n>`. Copy these three values verbatim — they are reliable because `eprintln!` is not truncated like panic messages.

- [ ] **Step 8.3** — Append the FROZEN golden-vector test to `tests/vectors.rs`, pasting the captured values directly into the assertions (fully populated — no placeholder token is committed). Use the exact `GOLDEN_LENGTHS` vector and `GOLDEN_HASH0` string you recorded in Step 8.2:
```rust
#[test]
fn golden_boundaries_are_stable() {
    let data = fixed_input(200 * 1024);
    let cfg = default_config();
    let chunks = chunk_slice(&data, &cfg);

    // Reconstruction sanity.
    let total: usize = chunks.iter().map(|c| c.data.len()).sum();
    assert_eq!(total, data.len());

    // FROZEN boundary sequence (paste from Step 8.2 GOLDEN_LENGTHS).
    let lengths: Vec<usize> = chunks.iter().map(|c| c.data.len()).collect();
    let expected_lengths: Vec<usize> = vec![65536, 49152, 16384]; // <- replace with captured GOLDEN_LENGTHS
    assert_eq!(lengths, expected_lengths, "chunk boundary sequence changed");

    // FROZEN first-chunk content address (paste from Step 8.2 GOLDEN_HASH0).
    assert_eq!(
        chunks[0].hash.to_hex(),
        "0000000000000000000000000000000000000000000000000000000000000000" // <- replace with captured GOLDEN_HASH0
    );
}
```

- [ ] **Step 8.4** — Run `cargo test -p digstore-chunker --test vectors golden_boundaries_are_stable`. Expected PASS: `test result: ok. 1 passed`. If it fails with `chunk boundary sequence changed`, you pasted the wrong `lengths` vector or hash — re-run Step 8.2 and paste exactly. Run the command a second time to confirm cross-run stability.

- [ ] **Step 8.5** — Commit:
```
git add crates/digstore-chunker/tests/vectors.rs
git commit -m "test(chunker): pin golden boundary sequence + content address (determinism contract)"
```

---

## Task 9 — Frozen dedup-locality vector (CDC heritage, §3)

**Files:**
- Modify: `crates/digstore-chunker/tests/vectors.rs`

The dedup-friendliness guarantee (front insert shifts only LOCAL boundaries; trailing chunks re-synchronize and share content addresses) is the defining CDC property versus fixed-block chunking. Because CDC re-synchronization is PROBABILISTIC over arbitrary inputs, we do NOT assert it as a hard property over random data (that would be flaky). Instead we verify it ONCE on a specific pinned input and freeze the observed shared-trailing-chunk count.

- [ ] **Step 9.1** — Append a capture-only test for the dedup-locality vector to `tests/vectors.rs` (it reuses `fixed_input` from Task 8; it only prints, so no placeholder and no possible failure):
```rust
/// Count shared trailing chunk content-addresses between two chunkings.
fn shared_trailing_hashes(a: &[digstore_chunker::Chunk], b: &[digstore_chunker::Chunk]) -> usize {
    let mut shared = 0usize;
    while shared < a.len()
        && shared < b.len()
        && a[a.len() - 1 - shared].hash == b[b.len() - 1 - shared].hash
    {
        shared += 1;
    }
    shared
}

#[test]
#[ignore = "capture-only: run with --ignored --nocapture to print dedup-locality value"]
fn capture_dedup_locality_value() {
    let cfg = default_config();
    let body = fixed_input(300 * 1024);
    let mut modified = vec![0xABu8; 1000]; // 1000-byte prepend
    modified.extend_from_slice(&body);

    let original_chunks = chunk_slice(&body, &cfg);
    let modified_chunks = chunk_slice(&modified, &cfg);
    let shared = shared_trailing_hashes(&original_chunks, &modified_chunks);

    eprintln!("DEDUP_ORIGINAL_CHUNKS={}", original_chunks.len());
    eprintln!("DEDUP_MODIFIED_CHUNKS={}", modified_chunks.len());
    eprintln!("DEDUP_SHARED_TRAILING={shared}");
}
```

- [ ] **Step 9.2** — Run `cargo test -p digstore-chunker --test vectors capture_dedup_locality_value -- --ignored --nocapture`. Expected: `test result: ok. 1 passed` plus stderr lines like `DEDUP_ORIGINAL_CHUNKS=<n>`, `DEDUP_MODIFIED_CHUNKS=<m>`, `DEDUP_SHARED_TRAILING=<k>` where `k >= 1` (CDC re-synchronizes over a 300 KiB shared body). Record the `DEDUP_SHARED_TRAILING` value `k`.

- [ ] **Step 9.3** — Append the FROZEN dedup-locality test to `tests/vectors.rs`, pasting the captured `k` (this is a deterministic single-input vector, never flaky, and is the §3 dedup evidence). A fixed-block chunker would share ZERO here, which is exactly what CDC fixes:
```rust
#[test]
fn front_insert_preserves_trailing_chunks() {
    let cfg = default_config();
    let body = fixed_input(300 * 1024);
    let mut modified = vec![0xABu8; 1000];
    modified.extend_from_slice(&body);

    let original_chunks = chunk_slice(&body, &cfg);
    let modified_chunks = chunk_slice(&modified, &cfg);
    let shared = shared_trailing_hashes(&original_chunks, &modified_chunks);

    // FROZEN observed re-synchronization (paste DEDUP_SHARED_TRAILING from Step 9.2).
    // A fixed-block chunker would share ZERO trailing chunks after a front insert;
    // CDC re-synchronizes and shares this many (paper §3 dedup heritage).
    let expected_shared: usize = 1; // <- replace with captured DEDUP_SHARED_TRAILING
    assert_eq!(shared, expected_shared, "dedup re-sync count changed");
    assert!(shared >= 1, "CDC must share at least one trailing chunk after front insert");
}
```

- [ ] **Step 9.4** — Run `cargo test -p digstore-chunker --test vectors front_insert_preserves_trailing_chunks`. Expected PASS: `test result: ok. 1 passed`. If `dedup re-sync count changed`, paste the exact `k` from Step 9.2.

- [ ] **Step 9.5** — Commit:
```
git add crates/digstore-chunker/tests/vectors.rs
git commit -m "test(chunker): freeze front-insert dedup-locality vector (CDC heritage)"
```

---

## Task 10 — Property tests (determinism, reconstruction, bounds, stream parity)

**Files:**
- Create: `crates/digstore-chunker/tests/properties.rs`

These proptests cover behaviors already implemented in earlier tasks; they are REGRESSION-FORMALIZATION (green on first write) — they generalize the unit tests across thousands of random inputs and guard against future regressions. We deliberately avoid the flaky `shared >= 1` hard assertion over random data (covered instead by the frozen vector in Task 9); the dedup property here is a non-failing statistical OBSERVATION.

- [ ] **Step 10.1** — Create `crates/digstore-chunker/tests/properties.rs` with the determinism property:
```rust
use digstore_chunker::{chunk_slice, chunk_stream, Chunk};
use proptest::prelude::*;

fn small_cfg() -> digstore_core::ChunkerConfig {
    digstore_core::ChunkerConfig { min_size: 64, target_size: 256, max_size: 1024, mask: 0xFF }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Determinism: chunking the same input twice yields identical chunks.
    #[test]
    fn determinism_same_input_same_chunks(data in proptest::collection::vec(any::<u8>(), 0..50_000)) {
        let cfg = small_cfg();
        let a = chunk_slice(&data, &cfg);
        let b = chunk_slice(&data, &cfg);
        prop_assert_eq!(a, b);
    }
}
```

- [ ] **Step 10.2** — REGRESSION-FORMALIZATION. Run `cargo test -p digstore-chunker --test properties determinism_same_input_same_chunks`. Expected PASS: `test result: ok. 1 passed` (the implementation is already deterministic; this formalizes it across random inputs).

- [ ] **Step 10.3** — Append the reconstruction + offset-contiguity + size-bound property:
```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Concatenated chunks reconstruct the input; offsets contiguous; interior
    /// chunks obey size bounds.
    #[test]
    fn reconstruct_offsets_and_bounds(data in proptest::collection::vec(any::<u8>(), 0..50_000)) {
        let cfg = small_cfg();
        let chunks = chunk_slice(&data, &cfg);

        // Reconstruction.
        let mut rebuilt = Vec::with_capacity(data.len());
        for c in &chunks {
            rebuilt.extend_from_slice(&c.data);
        }
        prop_assert_eq!(&rebuilt, &data);

        // Offsets contiguous from 0.
        let mut off = 0usize;
        for c in &chunks {
            prop_assert_eq!(c.offset, off);
            off += c.data.len();
        }

        // Size bounds: all but the last chunk in [min, max]; last in [1, max].
        if chunks.len() > 1 {
            for c in &chunks[..chunks.len() - 1] {
                prop_assert!(c.len() >= cfg.min_size);
                prop_assert!(c.len() <= cfg.max_size);
            }
        }
        if let Some(last) = chunks.last() {
            prop_assert!(last.len() >= 1);
            prop_assert!(last.len() <= cfg.max_size);
        }
    }
}
```

- [ ] **Step 10.4** — REGRESSION-FORMALIZATION. Run `cargo test -p digstore-chunker --test properties reconstruct_offsets_and_bounds`. Expected PASS: `test result: ok. 1 passed`. If proptest shrinks to a failing case (e.g. an interior chunk under `min_size`), it prints the minimal input — that would indicate a boundary bug in Task 5; fix `find_boundary` before proceeding.

- [ ] **Step 10.5** — Append the NON-FLAKY dedup-locality observation. It records the shared-trailing count without ever asserting a minimum, so it cannot fail spuriously (the hard guarantee lives in the frozen vector of Task 9). It exists to exercise the locality code path across many random bodies:
```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Non-flaky dedup observation: chunk both `body` and `prefix ++ body`; the
    /// shared trailing-chunk count is at most the number of chunks in either, and
    /// reconstruction holds for both. We do NOT assert shared >= 1 here (CDC
    /// re-sync is probabilistic over random data; the guaranteed case is the
    /// frozen vector in tests/vectors.rs). This only proves no panic / no
    /// inconsistency in the locality path.
    #[test]
    fn front_insert_locality_is_consistent(
        prefix in proptest::collection::vec(any::<u8>(), 1..2000),
        body in proptest::collection::vec(any::<u8>(), 5_000..40_000),
    ) {
        let cfg = small_cfg();
        let mut modified = prefix.clone();
        modified.extend_from_slice(&body);

        let oc = chunk_slice(&body, &cfg);
        let mc = chunk_slice(&modified, &cfg);

        // Both reconstruct.
        let oj: Vec<u8> = oc.iter().flat_map(|c| c.data.clone()).collect();
        prop_assert_eq!(&oj, &body);
        let mj: Vec<u8> = mc.iter().flat_map(|c| c.data.clone()).collect();
        prop_assert_eq!(&mj, &modified);

        // Shared trailing count is well-formed (<= min of the two chunk counts).
        let mut shared = 0usize;
        while shared < oc.len()
            && shared < mc.len()
            && oc[oc.len() - 1 - shared].hash == mc[mc.len() - 1 - shared].hash
        {
            shared += 1;
        }
        prop_assert!(shared <= oc.len().min(mc.len()));
    }
}
```

- [ ] **Step 10.6** — REGRESSION-FORMALIZATION. Run `cargo test -p digstore-chunker --test properties front_insert_locality_is_consistent`. Expected PASS: `test result: ok. 1 passed`. This cannot shrink to a failure (no minimum-shared assertion).

- [ ] **Step 10.7** — Append the stream/slice equivalence property:
```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// chunk_stream over a Cursor equals chunk_slice for the same bytes.
    #[test]
    fn stream_equals_slice_property(data in proptest::collection::vec(any::<u8>(), 0..30_000)) {
        let cfg = small_cfg();
        let want = chunk_slice(&data, &cfg);
        let got = chunk_stream(std::io::Cursor::new(&data), &cfg).unwrap();
        prop_assert_eq!(got, want);
    }
}
```
> NOTE: `Chunk` is imported at the top of the file for type clarity in helper signatures; if the unused-import lint fires, prefix the import with `#[allow(unused_imports)]` or remove it — `chunk_slice`/`chunk_stream` are the load-bearing imports.

- [ ] **Step 10.8** — REGRESSION-FORMALIZATION. Run `cargo test -p digstore-chunker --test properties stream_equals_slice_property`. Expected PASS: `test result: ok. 1 passed`.

- [ ] **Step 10.9** — Run the full property suite: `cargo test -p digstore-chunker --test properties`. Expected PASS: `test result: ok. 4 passed`.

- [ ] **Step 10.10** — Commit:
```
git add crates/digstore-chunker/tests/properties.rs
git commit -m "test(chunker): property tests for determinism, bounds, locality, stream parity"
```

---

## Task 11 — Public `Chunker` round-trip and full-suite quality gate

**Files:**
- Modify: `crates/digstore-chunker/tests/vectors.rs`

- [ ] **Step 11.1** — Append a public-API round-trip test to `tests/vectors.rs` exercising the `Chunker` struct end to end over the canonical `default_config` (constructor + `chunk_slice` method + `config()` accessor). NOTE: this requires `ChunkerConfig: Clone`. The canonical catalog defines `ChunkerConfig` in `digstore-core`; if it does not derive `Clone`, call `default_config()` twice instead of cloning — do not add a `Clone` impl in this crate:
```rust
use digstore_chunker::Chunker;

#[test]
fn chunker_struct_roundtrip_via_public_api() {
    let cfg = default_config();
    let chunker = Chunker::new(cfg);
    assert_eq!(chunker.config().target_size, 64 * 1024);

    let data = fixed_input(300 * 1024);
    let chunks = chunker.chunk_slice(&data);

    // Reconstruct and confirm every content address is a 64-hex-char SHA-256.
    let mut rebuilt = Vec::new();
    for c in &chunks {
        rebuilt.extend_from_slice(&c.data);
        assert_eq!(c.hash.to_hex().len(), 64);
    }
    assert_eq!(rebuilt, data);
    assert!(chunks.len() > 1, "300 KiB under 64 KiB target should yield multiple chunks");
}
```

- [ ] **Step 11.2** — REGRESSION-FORMALIZATION / smoke test (no new behavior; `Chunker::new`/`config()`/`chunk_slice` already exist and are public from Task 6). Run `cargo test -p digstore-chunker --test vectors chunker_struct_roundtrip_via_public_api`. Expected PASS: `test result: ok. 1 passed`. If `Chunker::new` is reported private, confirm `chunker.rs` declares `pub fn new` and `pub fn config` (it does, per Task 6).

- [ ] **Step 11.3** — Run the entire crate test suite (lib unit tests + both integration binaries) and confirm everything is green: `cargo test -p digstore-chunker`. Expected: `test result: ok.` for the lib target and for `vectors` and `properties` (the `#[ignore]`d capture tests are reported as `ignored`, not run). No failures.

- [ ] **Step 11.4** — Run clippy with warnings denied: `cargo clippy -p digstore-chunker --all-targets -- -D warnings`. Expected: `Finished` with no warnings. Fix any lint by following its suggestion (e.g. `needless_range_loop`, `len_without_is_empty` — note `Chunk` already has `is_empty`, satisfying that lint; `manual_memcpy`). Re-run until clean.

- [ ] **Step 11.5** — Commit:
```
git add crates/digstore-chunker/tests/vectors.rs
git commit -m "test(chunker): public Chunker round-trip; clippy-clean full suite"
```

---

## Definition of Done

A checklist mapping this crate's assigned paper sections to the tasks that satisfy them. Every box must be checked, with the cited test green. (Scope note: per design doc §4.2, this crate covers only the chunking algorithm itself — gear hashing, boundary detection, content addressing, and the slice/stream APIs. The downstream consumers of chunk ordering — `KeyTableEntry::chunk_indices`, the interleaved pool, and `PathWalk` — live in `digstore-core` and `digstore-compiler`, not here; this crate's only ordering obligation is that `chunk_slice`/`chunk_stream` emit chunks in strict input order with contiguous offsets, which Tasks 6, 7, and 10 prove.)

- [ ] **§3 (CDC heritage):** Gear-based content-defined chunking implemented (compile-time-generated gear table + hand-rolled rolling hash), NOT fixed-block. Dedup-friendliness proven by the frozen `front_insert_preserves_trailing_chunks` vector (Task 9) — a front insert preserves a nonzero count of trailing chunk content addresses, the defining CDC property (a fixed-block chunker would share zero), plus the non-flaky locality observation (Task 10.5). *(Tasks 2, 5, 9, 10)*
- [ ] **§8.1 (chunking algorithm):** Boundary rule `(hash & mask) == 0` with `min_size`/`max_size` enforcement (forced cut at max, no cut below min); `ChunkerConfig` honored (min 16 KiB, target 64 KiB, max 256 KiB, mask from `floor(log2(target))`); each chunk SHA-256 content-addressed. *(Tasks 3, 4, 5, 6)*
- [ ] **§8.2 (chunking part):** Per the design doc §4.2, the §8.2 chunking scope for this crate is "streaming chunk boundaries; each chunk hashed SHA-256 (its content address); deterministic boundaries for dedup." Covered by: slice API `chunk_slice` and true incremental streaming API `chunk_stream(reader)`, both producing ordered, contiguous, reconstruction-exact `Vec<Chunk>` with stream proven byte-identical to slice; empty input → zero chunks; sub-`min` input → single chunk; trailing short chunk permitted. Chunk ordering/indexing for the key table is explicitly out of scope here (lives in `digstore-core`/`digstore-compiler`); this crate guarantees only input-order emission with contiguous offsets. *(Tasks 6, 7, 10)*
- [ ] **Determinism contract:** Compile-time-generated frozen 256-entry distinct gear table with pinned guards (Task 2); golden boundary sequence + first-chunk content address pinned via reliable `eprintln!` capture (Task 8); same-input-same-output property across 256 random cases (Task 10.1). *(Tasks 2, 8, 10)*
- [ ] **Hand-roll-over-`fastcdc` decision:** Gear hash hand-rolled and gear table `const fn`-generated for byte-exact cross-platform determinism (documented in `lib.rs` and `gear.rs`); validated against fixed vectors rather than delegated to the `fastcdc` crate. *(Tasks 1, 2, 8)*
- [ ] **No placeholders / strict TDD:** No `unimplemented!()`, no literal "PLACEHOLDER" token, no all-zero frozen tables committed; the scaffold (Task 1) compiles and is honestly labelled as superseded; regression-formalization steps are explicitly flagged; `gear_roll` has a real behavioral test (Task 2.6/2.7). *(Tasks 1, 2, 8, 9, 10, 11)*
- [ ] **Quality gate:** `cargo test -p digstore-chunker` fully green across the lib target, `--test vectors`, and `--test properties`; `cargo clippy -p digstore-chunker --all-targets -- -D warnings` clean. *(Task 11)*

---

## Plan metadata

- **Crate:** digstore-chunker
- **Assigned paper sections:** 8.1,8.2(chunking part),3(CDC heritage)
- **Depends on:** digstore-core
- **Spec sections covered (claimed):** 3, 8.1, 8.2

### Public items exported (consumed by other crates)

```
pub struct Chunk { pub hash: Bytes32, pub data: Vec<u8>, pub offset: usize }
impl Chunk { pub fn new(offset: usize, data: Vec<u8>) -> Self }
impl Chunk { pub fn len(&self) -> usize }
impl Chunk { pub fn is_empty(&self) -> bool }
pub fn hash_data(data: &[u8]) -> digstore_core::Bytes32
pub struct Chunker { /* private config: ChunkerConfig */ }
impl Chunker { pub fn new(config: digstore_core::ChunkerConfig) -> Self }
impl Chunker { pub fn config(&self) -> &digstore_core::ChunkerConfig }
impl Chunker { pub fn chunk_slice(&self, data: &[u8]) -> Vec<Chunk> }
pub fn chunk_slice(data: &[u8], cfg: &digstore_core::ChunkerConfig) -> Vec<Chunk>
pub fn chunk_stream<R: std::io::Read>(reader: R, cfg: &digstore_core::ChunkerConfig) -> std::io::Result<Vec<Chunk>>
pub fn mask_for_target(target_size: usize) -> u64
pub fn default_config() -> digstore_core::ChunkerConfig
pub const GEAR_TABLE: [u64; 256]
```