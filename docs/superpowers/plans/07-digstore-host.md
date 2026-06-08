# digstore-host Implementation Plan

> **For agentic workers:** Execute this plan using the **REQUIRED SUB-SKILL `superpowers:subagent-driven-development`**. Each numbered Task is a bite-sized TDD cycle: write the failing test, run it and confirm the expected failure, write the minimal implementation, run the test and confirm it passes, then commit with the exact conventional-commit message shown. Do not batch steps. Do not skip the red phase. One checkbox = one 2-5 minute action.

**Goal:** Implement `dig-host`, the wasmtime-backed runtime that instantiates a compiled Digstore WASM module, wires the eight `dig_host` import functions, enforces execution bounds (wall-clock timeout, memory ceiling, fuel), and drives the alloc → write-request → call-export → unpack → read → dealloc serve flow without ever decrypting or inspecting payloads.

**Architecture:** A `HostRuntime` owns a sync `wasmtime::Engine`, a validated `Module`, and a `Store<RuntimeState>` carrying a `HostState` (shared return buffer, BLS keys, injectable `Clock`, session table, `ChainSource`, `Prover`, CSPRNG, swappable `AttestationBackend`). The eight `dig_host` imports are registered on a `Linker<RuntimeState>` and read/write the guest's linear memory via the exported `memory` plus the guest's `alloc`/`host_read_return_buffer` contract. Higher-level methods (`get_store_id`, `serve_content`, `serve_proof`, …) marshal arguments through `pack_ptr_len`/`unpack_ptr_len`/`is_error` and treat returned bytes as opaque.

**Tech Stack:** Rust (edition 2021), `wasmtime` (sync engine, epoch interruption + fuel + `StoreLimits`), `reqwest` (blocking, for `jwks_fetch`), `rand`/`rand_chacha` (CSPRNG for `host_random_bytes`), `digstore-core` (canonical types, ABI helpers, codec), `digstore-crypto` (BLS sign + SHA-256), `digstore-prover` (`Prover`, `ChainSource` traits). Test fixtures are tiny hand-written WASM modules compiled from `.wat` via `wat`, plus the real `digstore-guest` template for the gated integration test.

---

## File Structure

All paths under `crates/digstore-host/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest: deps on wasmtime, reqwest (blocking), rand, rand_chacha, thiserror, digstore-core, digstore-crypto, digstore-prover; dev-deps wat, httpmock. |
| `src/lib.rs` | Crate root: module declarations, re-exports (`HostRuntime`, `HostDeps`, `HostState`, `Clock`, `SystemClock`, `FixedClock`, `HostError`, `ExecutionLimits`, `Session`, `SessionTable`, `AttestationBackend`, `BlsAttestationBackend`, consts). |
| `src/error.rs` | `HostError` enum (thiserror) covering wasmtime, validation, ABI-error, timeout, fuel, memory, import failures; maps guest `ErrorCode` sentinels. |
| `src/config.rs` | `ExecutionLimits { timeout, memory_bytes_max, fuel }` with spec defaults (16 MiB ceiling, fuel, 5s timeout) + page constants. |
| `src/clock.rs` | `Clock` trait (`now_unix_secs() -> u64`); `SystemClock`; `FixedClock` (injectable, deterministic). |
| `src/session.rs` | `Session { nonce, store_id, established_at, expires_at }`, `SessionTable` (new/establish/is_valid/active_store_id/clear with expiry). |
| `src/random.rs` | `HostRng` wrapper over `ChaCha20Rng`; `fill(count, max)` capped at `max_random_bytes`. |
| `src/teehook.rs` | §13.6 TEE-alternative attestation hook: `AttestationBackend` trait, default `BlsAttestationBackend`, `SharedBackend = Arc<dyn AttestationBackend>`. (Defined early so `HostState`/`HostRuntime` use the FINAL API.) |
| `src/state.rs` | `HostState` (return buffer, config, BLS keys, attestation backend, clock, session table, chain source, prover, rng, instance id) + `ReturnBuffer` write/grow helper + `HostKeys`. |
| `src/memory.rs` | Guest-memory helpers: read N bytes at ptr, write bytes at ptr (bounds-checked). |
| `src/imports.rs` | The eight `dig_host` import closures registered on the `Linker<RuntimeState>`. |
| `src/runtime.rs` | `HostRuntime` + `HostDeps` + `RuntimeState` + `EpochTicker`: engine/module setup, epoch+fuel+limits config, instantiate, typed export calls, serve flow (`serve_content`, `serve_proof`). |
| `tests/common/mod.rs` | Shared `test_deps(FixedClock) -> HostDeps` helper for integration tests. |
| `tests/fixtures/wat/echo.wat` | Tiny module: `alloc`/`dealloc`/`memory`/`init`/`get_store_id` returning fixed 32 bytes, for instantiation tests. |
| `tests/fixtures/wat/spin.wat` | Module whose `get_store_id` runs an infinite loop, for timeout/fuel tests. |
| `tests/fixtures/wat/grow.wat` | Module whose export calls `memory.grow`, for memory-limit tests. |
| `tests/fixtures/wat/return_buffer.wat` | Module that fills then copies the return buffer back, for buffer round-trip. |
| `tests/fixtures/wat/import_probe.wat` | Module exporting one function per import that calls the import and returns its result, for per-import tests. |
| `tests/fixtures/wat/serve_echo.wat` | Module whose `get_content`/`get_proof` echo the request bytes, for serve flow. |
| `tests/fixtures/wat/serve_err.wat` | Module whose `get_content`/`get_proof` return an error sentinel, for sentinel propagation. |
| `tests/fixtures/build_fixture.md` | Documents how the real `digstore-guest` `sample.wasm` integration fixture is produced. |
| `tests/instantiate.rs` | Integration: instantiate echo fixture, call `get_store_id`. |
| `tests/bounds.rs` | Integration: timeout, fuel exhaustion, memory ceiling (positive + negative). |
| `tests/imports_unit.rs` | Integration: each import probed (time, random cap, pubkey, attestation, session gating, clock determinism). |
| `tests/jwks_mock.rs` | Integration: `jwks_fetch` NoSession before session, succeeds after (httpmock). |
| `tests/serve_flow.rs` | Integration: `serve_content`/`serve_proof` round-trip + error-sentinel propagation. |
| `tests/return_buffer.rs` | Integration: return-buffer round-trip and grow-on-demand. |
| `tests/e2e_guest.rs` | Integration: serve against the real digstore-guest fixture (`#[ignore]`, run with `--ignored`). |

---

## Task 1 — Crate manifest and skeleton

**Files:**
- Create `crates/digstore-host/Cargo.toml`
- Create `crates/digstore-host/src/lib.rs`
- Create stub `src/*.rs` module files
- Modify workspace root `Cargo.toml`

Steps:

- [ ] **Write the manifest.** Create `crates/digstore-host/Cargo.toml`:
```toml
[package]
name = "digstore-host"
version = "0.1.0"
edition = "2021"

[dependencies]
digstore-core = { path = "../digstore-core" }
digstore-crypto = { path = "../digstore-crypto" }
digstore-prover = { path = "../digstore-prover" }
wasmtime = { version = "27", default-features = false, features = ["cranelift", "runtime", "gc"] }
reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }
rand = "0.8"
rand_chacha = "0.3"
thiserror = "1"

[dev-dependencies]
wat = "1"
httpmock = "0.7"
```

- [ ] **Add the crate to the workspace.** Modify the workspace root `Cargo.toml` (`C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml`) to include `"crates/digstore-host"` in `members` (create the `[workspace]` table with `members` and `resolver = "2"` if not present).

- [ ] **Write the crate root with module declarations only.** Create `crates/digstore-host/src/lib.rs`:
```rust
//! `dig-host`: wasmtime runtime for serving compiled Digstore WASM modules.
//!
//! Implements the host side of the `dig_host` import module (paper §6.3, §12),
//! the shared return buffer (§6.4), execution bounds (§18.2), the import
//! dispatch / state threading (§18.1, §18.3), the serve flow (§18.4), and the
//! swappable TEE-alternative attestation hook (§13.6). The host NEVER decrypts
//! or inspects served payloads.

mod clock;
mod config;
mod error;
mod imports;
mod memory;
mod random;
mod runtime;
mod session;
mod state;
mod teehook;

// re-exports added as modules land
```

- [ ] **Create empty stub module files so the crate compiles.** Create each of `clock.rs`, `config.rs`, `error.rs`, `imports.rs`, `memory.rs`, `random.rs`, `runtime.rs`, `session.rs`, `state.rs`, `teehook.rs` under `crates/digstore-host/src/` containing only a single doc-comment line (e.g. `//! placeholder, filled in a later task`).

- [ ] **Run the build to confirm the skeleton compiles.** `cargo build -p digstore-host`. Expected: dependency crates compile first, then `Compiling digstore-host v0.1.0` and `Finished`.

- [ ] **Commit.** `git add crates/digstore-host/Cargo.toml crates/digstore-host/src Cargo.toml` then `git commit -m "chore(host): scaffold digstore-host crate skeleton"`.

---

## Task 2 — `Clock` trait with injectable deterministic clock

**Files:**
- Modify `crates/digstore-host/src/clock.rs`
- Modify `crates/digstore-host/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/clock.rs`

Steps:

- [ ] **Write the failing test.** Add to the bottom of `crates/digstore-host/src/clock.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_injected_time() {
        let clock = FixedClock::new(1_700_000_000);
        assert_eq!(clock.now_unix_secs(), 1_700_000_000);
    }

    #[test]
    fn fixed_clock_can_advance() {
        let clock = FixedClock::new(100);
        clock.advance(50);
        assert_eq!(clock.now_unix_secs(), 150);
    }

    #[test]
    fn fixed_clock_set_overwrites() {
        let clock = FixedClock::new(100);
        clock.set(999);
        assert_eq!(clock.now_unix_secs(), 999);
    }

    #[test]
    fn system_clock_is_after_2020() {
        let clock = SystemClock;
        assert!(clock.now_unix_secs() > 1_577_836_800);
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host clock`. Expected: `error[E0433]: failed to resolve` / `cannot find type FixedClock in this scope`.

- [ ] **Write the minimal implementation.** Put above the test module in `crates/digstore-host/src/clock.rs`:
```rust
//! Injectable wall-clock source for `host_get_current_time` (§12).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source of the current Unix time in seconds. Injectable so tests are
/// deterministic and temporal-key checks (§16) are reproducible.
pub trait Clock: Send + Sync + 'static {
    fn now_unix_secs(&self) -> u64;
}

/// Production clock backed by the OS wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Deterministic clock for tests; time only moves when `advance`/`set` is called.
/// Clone shares the same underlying counter.
#[derive(Debug, Clone)]
pub struct FixedClock(Arc<AtomicU64>);

impl FixedClock {
    pub fn new(secs: u64) -> Self {
        FixedClock(Arc::new(AtomicU64::new(secs)))
    }
    pub fn advance(&self, secs: u64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
    pub fn set(&self, secs: u64) {
        self.0.store(secs, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_unix_secs(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host clock`. Expected: `test result: ok. 4 passed`.

- [ ] **Enable the re-export.** In `src/lib.rs` replace the `// re-exports added as modules land` line with `pub use clock::{Clock, FixedClock, SystemClock};`.

- [ ] **Commit.** `git add crates/digstore-host/src/clock.rs crates/digstore-host/src/lib.rs` then `git commit -m "feat(host): add injectable Clock with FixedClock and SystemClock"`.

---

## Task 3 — `ExecutionLimits` config and defaults

**Files:**
- Modify `crates/digstore-host/src/config.rs`
- Modify `crates/digstore-host/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/config.rs`

Steps:

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn defaults_match_spec() {
        let l = ExecutionLimits::default();
        assert_eq!(l.memory_bytes_max, 16 * 1024 * 1024);
        assert_eq!(l.timeout, Duration::from_secs(5));
        assert!(l.fuel >= 1_000_000_000);
    }

    #[test]
    fn pages_helper_matches_bytes() {
        let l = ExecutionLimits::default();
        assert_eq!(l.memory_pages_max(), 256);
    }

    #[test]
    fn consts_match_spec() {
        assert_eq!(WASM_PAGE_SIZE, 64 * 1024);
        assert_eq!(MAX_MEMORY_BYTES, 16 * 1024 * 1024);
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host config`. Expected: `cannot find type ExecutionLimits in this scope`.

- [ ] **Write the minimal implementation.** Put above the test module in `crates/digstore-host/src/config.rs`:
```rust
//! Execution bounds (§18.2): wall-clock timeout, outer memory ceiling, fuel.

use std::time::Duration;

/// Page size of WASM linear memory.
pub const WASM_PAGE_SIZE: usize = 64 * 1024;

/// Hard ceiling matching the guest's declared max (256 pages = 16 MiB, §18.2).
pub const MAX_MEMORY_BYTES: usize = 256 * WASM_PAGE_SIZE;

#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    /// Wall-clock budget for a single export call (enforced via epoch interruption).
    pub timeout: Duration,
    /// Outer linear-memory ceiling in bytes (StoreLimits, §18.2).
    pub memory_bytes_max: usize,
    /// Fuel budget for a single export call.
    pub fuel: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        ExecutionLimits {
            timeout: Duration::from_secs(5),
            memory_bytes_max: MAX_MEMORY_BYTES,
            fuel: 5_000_000_000,
        }
    }
}

impl ExecutionLimits {
    pub fn memory_pages_max(&self) -> usize {
        self.memory_bytes_max / WASM_PAGE_SIZE
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host config`. Expected: `test result: ok. 3 passed`.

- [ ] **Enable the re-export.** In `src/lib.rs` add `pub use config::{ExecutionLimits, MAX_MEMORY_BYTES, WASM_PAGE_SIZE};`.

- [ ] **Commit.** `git add crates/digstore-host/src/config.rs crates/digstore-host/src/lib.rs` then `git commit -m "feat(host): add ExecutionLimits with 16MiB ceiling and fuel defaults"`.

---

## Task 4 — `HostError` enum

**Files:**
- Modify `crates/digstore-host/src/error.rs`
- Modify `crates/digstore-host/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/error.rs`

Steps:

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/error.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::abi::ErrorCode;

    #[test]
    fn from_error_code_maps_no_session() {
        let e = HostError::from_guest_code(ErrorCode::NoSession as i32);
        assert!(matches!(e, HostError::GuestError(ErrorCode::NoSession)));
    }

    #[test]
    fn from_error_code_maps_not_found() {
        let e = HostError::from_guest_code(ErrorCode::NotFound as i32);
        assert!(matches!(e, HostError::GuestError(ErrorCode::NotFound)));
    }

    #[test]
    fn unknown_code_is_general() {
        let e = HostError::from_guest_code(-9999);
        assert!(matches!(e, HostError::GuestError(ErrorCode::GeneralError)));
    }

    #[test]
    fn timeout_displays() {
        assert_eq!(HostError::Timeout.to_string(), "execution timed out");
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host error`. Expected: `cannot find type HostError`.

- [ ] **Write the minimal implementation.** Above the test module in `crates/digstore-host/src/error.rs`:
```rust
//! Error type for the host runtime.

use digstore_core::abi::ErrorCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("wasmtime error: {0}")]
    Wasmtime(String),

    #[error("module validation failed: {0}")]
    Validation(String),

    #[error("guest export returned error code: {0:?}")]
    GuestError(ErrorCode),

    #[error("execution timed out")]
    Timeout,

    #[error("execution ran out of fuel")]
    OutOfFuel,

    #[error("guest exceeded memory ceiling")]
    MemoryLimit,

    #[error("missing required export: {0}")]
    MissingExport(&'static str),

    #[error("guest memory access out of bounds")]
    OutOfBounds,

    #[error("return buffer overflow: needed {needed}, max {max}")]
    ReturnBufferOverflow { needed: usize, max: usize },

    #[error("http error: {0}")]
    Http(String),
}

impl HostError {
    /// Map a negative guest sentinel (`is_error` true) to a `HostError`.
    /// Unknown / unmapped codes collapse to `GeneralError`.
    pub fn from_guest_code(code: i32) -> Self {
        let mapped = match code {
            c if c == ErrorCode::GeneralError as i32 => ErrorCode::GeneralError,
            c if c == ErrorCode::InvalidParameter as i32 => ErrorCode::InvalidParameter,
            c if c == ErrorCode::BufferTooSmall as i32 => ErrorCode::BufferTooSmall,
            c if c == ErrorCode::NoSession as i32 => ErrorCode::NoSession,
            c if c == ErrorCode::SessionExpired as i32 => ErrorCode::SessionExpired,
            c if c == ErrorCode::AttestationFailed as i32 => ErrorCode::AttestationFailed,
            c if c == ErrorCode::NetworkError as i32 => ErrorCode::NetworkError,
            c if c == ErrorCode::Timeout as i32 => ErrorCode::Timeout,
            c if c == ErrorCode::NotFound as i32 => ErrorCode::NotFound,
            c if c == ErrorCode::ValidationFailed as i32 => ErrorCode::ValidationFailed,
            _ => ErrorCode::GeneralError,
        };
        HostError::GuestError(mapped)
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host error`. Expected: `test result: ok. 4 passed`. (If `digstore_core`'s `ErrorCode` is exported at a different path than `digstore_core::abi`, adjust the import to the real path; the type name `ErrorCode` is fixed by the canonical catalog.)

- [ ] **Enable the re-export.** In `src/lib.rs` add `pub use error::HostError;`.

- [ ] **Commit.** `git add crates/digstore-host/src/error.rs crates/digstore-host/src/lib.rs` then `git commit -m "feat(host): add HostError with guest ErrorCode mapping"`.

---

## Task 5 — CSPRNG wrapper with `max_random_bytes` cap

**Files:**
- Modify `crates/digstore-host/src/random.rs`
- Test: inline `#[cfg(test)]` in `src/random.rs`

Steps:

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/random.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_returns_requested_count_under_cap() {
        let mut rng = HostRng::from_seed([7u8; 32]);
        let out = rng.fill(64, 1024).unwrap();
        assert_eq!(out.len(), 64);
    }

    #[test]
    fn fill_over_cap_is_rejected() {
        let mut rng = HostRng::from_seed([7u8; 32]);
        assert!(rng.fill(2048, 1024).is_none());
    }

    #[test]
    fn seeded_rng_is_deterministic() {
        let mut a = HostRng::from_seed([1u8; 32]);
        let mut b = HostRng::from_seed([1u8; 32]);
        assert_eq!(a.fill(32, 1024), b.fill(32, 1024));
    }

    #[test]
    fn distinct_seeds_differ() {
        let mut a = HostRng::from_seed([1u8; 32]);
        let mut b = HostRng::from_seed([2u8; 32]);
        assert_ne!(a.fill(32, 1024), b.fill(32, 1024));
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host random`. Expected: `cannot find type HostRng`.

- [ ] **Write the minimal implementation.** Above the test module in `crates/digstore-host/src/random.rs`:
```rust
//! CSPRNG backing `host_random_bytes` (§12). Capped at `max_random_bytes`.
//! Seedable so the oblivious-access cover reads (§14.3) are reproducible in tests.

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;

pub struct HostRng {
    inner: ChaCha20Rng,
}

impl HostRng {
    /// Production constructor seeded from OS entropy.
    pub fn from_entropy() -> Self {
        HostRng {
            inner: ChaCha20Rng::from_entropy(),
        }
    }

    /// Deterministic constructor for tests.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        HostRng {
            inner: ChaCha20Rng::from_seed(seed),
        }
    }

    /// Produce `count` random bytes, or `None` if `count > max` (cap enforced).
    pub fn fill(&mut self, count: usize, max: usize) -> Option<Vec<u8>> {
        if count > max {
            return None;
        }
        let mut buf = vec![0u8; count];
        self.inner.fill_bytes(&mut buf);
        Some(buf)
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host random`. Expected: `test result: ok. 4 passed`.

- [ ] **Commit.** `git add crates/digstore-host/src/random.rs` then `git commit -m "feat(host): add capped CSPRNG wrapper for host_random_bytes"`.

---

## Task 6 — `Session` and `SessionTable` with expiry

**Files:**
- Modify `crates/digstore-host/src/session.rs`
- Modify `crates/digstore-host/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/session.rs`

Steps:

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/session.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_session_is_invalid() {
        let table = SessionTable::new();
        assert!(!table.is_valid(100));
    }

    #[test]
    fn established_session_is_valid_before_expiry() {
        let mut table = SessionTable::new();
        table.establish([9u8; 32], [3u8; 32], 100, 60);
        assert!(table.is_valid(120));
        assert_eq!(table.active_store_id(120), Some([3u8; 32]));
    }

    #[test]
    fn session_expires_after_ttl() {
        let mut table = SessionTable::new();
        table.establish([9u8; 32], [3u8; 32], 100, 60);
        assert!(!table.is_valid(161));
        assert_eq!(table.active_store_id(161), None);
    }

    #[test]
    fn clear_removes_session() {
        let mut table = SessionTable::new();
        table.establish([9u8; 32], [3u8; 32], 100, 60);
        table.clear();
        assert!(!table.is_valid(120));
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host session`. Expected: `cannot find type SessionTable`.

- [ ] **Write the minimal implementation.** Above the test module in `crates/digstore-host/src/session.rs`:
```rust
//! Attestation-gated session state (§12). A session is established after a
//! successful attestation handshake and gates `jwks_fetch` (§6.3).

#[derive(Debug, Clone, Copy)]
pub struct Session {
    pub nonce: [u8; 32],
    pub store_id: [u8; 32],
    pub established_at: u64,
    pub expires_at: u64,
}

impl Session {
    pub fn is_valid_at(&self, now: u64) -> bool {
        now < self.expires_at
    }
}

#[derive(Debug, Default)]
pub struct SessionTable {
    current: Option<Session>,
}

impl SessionTable {
    pub fn new() -> Self {
        SessionTable { current: None }
    }

    /// Establish (or replace) the active session with a TTL in seconds.
    pub fn establish(&mut self, nonce: [u8; 32], store_id: [u8; 32], now: u64, ttl_secs: u64) {
        self.current = Some(Session {
            nonce,
            store_id,
            established_at: now,
            expires_at: now.saturating_add(ttl_secs),
        });
    }

    pub fn is_valid(&self, now: u64) -> bool {
        self.current.map(|s| s.is_valid_at(now)).unwrap_or(false)
    }

    pub fn active_store_id(&self, now: u64) -> Option<[u8; 32]> {
        self.current
            .filter(|s| s.is_valid_at(now))
            .map(|s| s.store_id)
    }

    pub fn clear(&mut self) {
        self.current = None;
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host session`. Expected: `test result: ok. 4 passed`.

- [ ] **Enable the re-export.** In `src/lib.rs` add `pub use session::{Session, SessionTable};`.

- [ ] **Commit.** `git add crates/digstore-host/src/session.rs crates/digstore-host/src/lib.rs` then `git commit -m "feat(host): add session table with expiry for jwks gating"`.

---

## Task 7 — §13.6 TEE-alternative attestation hook (`AttestationBackend`)

§13.6: the attestation mechanism behind `host_create_attestation` must be swappable for a hardware-attestation backend without changing the import wiring. Default = BLS over the challenge (Chia AugScheme). This is defined **before** `HostState`/`HostRuntime` so those types use the FINAL `SharedBackend = Arc<dyn AttestationBackend>` directly — there is no temporary alias anywhere in this plan.

**Files:**
- Modify `crates/digstore-host/src/teehook.rs`
- Modify `crates/digstore-host/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/teehook.rs`

Steps:

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/teehook.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::types::{Bytes48, Bytes96};

    struct ConstBackend;
    impl AttestationBackend for ConstBackend {
        fn attest(&self, _challenge: &[u8]) -> Result<Bytes96, crate::error::HostError> {
            Ok(Bytes96([0x5Au8; 96]))
        }
        fn public_key(&self) -> Bytes48 {
            Bytes48([0x11u8; 48])
        }
    }

    #[test]
    fn custom_backend_signs() {
        let b = ConstBackend;
        let sig = b.attest(b"challenge").unwrap();
        assert_eq!(sig.0, [0x5Au8; 96]);
        assert_eq!(b.public_key().0, [0x11u8; 48]);
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host teehook`. Expected: `cannot find trait AttestationBackend`.

- [ ] **Write the trait + BLS default backend (FINAL `SharedBackend`).** Put above the test module in `crates/digstore-host/src/teehook.rs`:
```rust
//! §13.6 TEE / hardware-attestation alternative hook.
//!
//! `host_create_attestation` delegates to an `AttestationBackend`. The default
//! `BlsAttestationBackend` signs the challenge with the host BLS secret (Chia
//! AugScheme). A hardware-attestation backend can replace it behind the same
//! import surface without touching the linker wiring (§13.6).

use crate::error::HostError;
use digstore_core::types::{Bytes48, Bytes96};
use digstore_crypto::bls::BlsSecretKey;
use std::sync::Arc;

/// Pluggable attestation backend (§13.6). Default is BLS; a TEE backend can
/// drop in here without changing the import surface.
pub trait AttestationBackend: Send + Sync + 'static {
    /// Produce a 96-byte attestation signature over the challenge bytes.
    fn attest(&self, challenge: &[u8]) -> Result<Bytes96, HostError>;
    /// The attesting public key (48-byte BLS G1 for the BLS backend).
    fn public_key(&self) -> Bytes48;
}

/// Shared backend handle carried on `HostState`.
pub type SharedBackend = Arc<dyn AttestationBackend>;

/// Default backend: BLS sign over the challenge (Chia AugScheme).
pub struct BlsAttestationBackend {
    secret: BlsSecretKey,
    public: Bytes48,
}

impl BlsAttestationBackend {
    pub fn new(secret: BlsSecretKey, public: Bytes48) -> Self {
        BlsAttestationBackend { secret, public }
    }
}

impl AttestationBackend for BlsAttestationBackend {
    fn attest(&self, challenge: &[u8]) -> Result<Bytes96, HostError> {
        Ok(digstore_crypto::bls::sign(&self.secret, challenge))
    }
    fn public_key(&self) -> Bytes48 {
        self.public
    }
}
```

> `digstore_crypto::bls::sign(&sk, msg) -> Bytes96` must use Chia AugScheme (the canonical decision); if the real crate names this function differently, adjust only this call — the type names `BlsSecretKey`/`Bytes96`/`Bytes48` are fixed by the canonical catalog.

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host teehook`. Expected: `test result: ok. 1 passed`.

- [ ] **Enable the re-export.** In `src/lib.rs` add `pub use teehook::{AttestationBackend, BlsAttestationBackend};`.

- [ ] **Commit.** `git add crates/digstore-host/src/teehook.rs crates/digstore-host/src/lib.rs` then `git commit -m "feat(host): add swappable AttestationBackend (13.6 TEE hook)"`.

---

## Task 8 — `HostState` with shared return buffer

The return buffer (§6.4): host writes import results into a buffer of `return_buffer_capacity` (default 64 KiB), grows up to `max_return_buffer_size` (16 MiB), and the guest copies it out via `host_read_return_buffer`. `HostState` references the FINAL `SharedBackend` from Task 7.

**Files:**
- Modify `crates/digstore-host/src/state.rs`
- Modify `crates/digstore-host/src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/state.rs`

Steps:

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/state.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::config::HostImportsConfig;

    fn cfg() -> HostImportsConfig {
        HostImportsConfig {
            return_buffer_capacity: 64 * 1024,
            max_return_buffer_size: 16 * 1024 * 1024,
            max_random_bytes: 1024,
            host_version: "dig-host-test/0.1".to_string(),
        }
    }

    #[test]
    fn set_return_then_read_round_trips() {
        let mut rb = ReturnBuffer::new(&cfg());
        let written = rb.set(&[1, 2, 3, 4]).unwrap();
        assert_eq!(written, 4);
        assert_eq!(rb.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn buffer_grows_past_initial_capacity() {
        let mut rb = ReturnBuffer::new(&cfg());
        let big = vec![7u8; 128 * 1024];
        let written = rb.set(&big).unwrap();
        assert_eq!(written, 128 * 1024);
        assert_eq!(rb.as_slice().len(), 128 * 1024);
    }

    #[test]
    fn buffer_rejects_over_max() {
        let mut rb = ReturnBuffer::new(&cfg());
        let too_big = vec![0u8; 16 * 1024 * 1024 + 1];
        let err = rb.set(&too_big).unwrap_err();
        assert!(matches!(err, crate::error::HostError::ReturnBufferOverflow { .. }));
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host state`. Expected: `cannot find type ReturnBuffer`.

- [ ] **Write the minimal implementation.** Above the test module in `crates/digstore-host/src/state.rs`:
```rust
//! Per-instance host state shared with the `dig_host` imports (§6.4, §12, §18.3).

use crate::clock::Clock;
use crate::error::HostError;
use crate::random::HostRng;
use crate::session::SessionTable;
use crate::teehook::SharedBackend;
use digstore_core::config::HostImportsConfig;
use digstore_core::types::{Bytes32, Bytes48, Bytes96};
use digstore_crypto::bls::BlsSecretKey;
use digstore_prover::{ChainSource, Prover};
use std::sync::Arc;

/// Growable shared return buffer (§6.4): the single channel imports use to
/// hand variable-length results back to the guest.
pub struct ReturnBuffer {
    bytes: Vec<u8>,
    max: usize,
}

impl ReturnBuffer {
    pub fn new(cfg: &HostImportsConfig) -> Self {
        ReturnBuffer {
            bytes: Vec::with_capacity(cfg.return_buffer_capacity),
            max: cfg.max_return_buffer_size,
        }
    }

    /// Replace buffer contents; returns the number of bytes written, or
    /// `ReturnBufferOverflow` if it exceeds `max_return_buffer_size`.
    pub fn set(&mut self, data: &[u8]) -> Result<usize, HostError> {
        if data.len() > self.max {
            return Err(HostError::ReturnBufferOverflow {
                needed: data.len(),
                max: self.max,
            });
        }
        self.bytes.clear();
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

/// Host BLS key material used for attestation and node-proof signing (§12).
pub struct HostKeys {
    pub bls_secret: BlsSecretKey,
    pub bls_public: Bytes48,
}

/// State threaded through every `dig_host` import call (§18.3).
pub struct HostState {
    pub store_id: Bytes32,
    pub config: HostImportsConfig,
    pub return_buffer: ReturnBuffer,
    pub keys: Arc<HostKeys>,
    pub attestation: SharedBackend,
    pub clock: Arc<dyn Clock>,
    pub sessions: SessionTable,
    pub chain: Arc<dyn ChainSource>,
    pub prover: Arc<dyn Prover>,
    pub rng: HostRng,
    pub instance_id: Bytes32,
    /// Reqwest request budget for blocking host I/O (jwks_fetch), seeded from
    /// ExecutionLimits.timeout. Epoch interruption does NOT cover blocking host
    /// calls, so jwks is bounded by this independent request timeout (§18.2 note).
    pub http_timeout_secs: u64,
    /// Set by attestation so the serve flow can record the last signature.
    pub last_signature: Option<Bytes96>,
}
```

> The test only constructs `ReturnBuffer`, so `HostState`'s dependency-crate references (`BlsSecretKey`, `ChainSource`, `Prover`, `SharedBackend`) must merely compile. `SharedBackend` already exists in its final form from Task 7. If a dependency-crate path differs from the assumed `digstore_crypto::bls` / `digstore_prover` roots, adjust the `use` lines; the type names are fixed by the canonical catalog.

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host state`. Expected: `test result: ok. 3 passed`.

- [ ] **Enable the re-export.** In `src/lib.rs` add `pub use state::HostState;`.

- [ ] **Commit.** `git add crates/digstore-host/src/state.rs crates/digstore-host/src/lib.rs` then `git commit -m "feat(host): add HostState and growable shared return buffer"`.

---

## Task 9 — Guest-memory helpers (read/write)

**Files:**
- Modify `crates/digstore-host/src/memory.rs`
- Create `crates/digstore-host/tests/fixtures/wat/echo.wat`
- Test: inline `#[cfg(test)]` in `src/memory.rs`

Steps:

- [ ] **Create the echo fixture.** Create `crates/digstore-host/tests/fixtures/wat/echo.wat`:
```wat
(module
  (memory (export "memory") 1 256)
  (global $bump (mut i32) (i32.const 1024))
  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $size)))
    (local.get $ptr))
  (func (export "dealloc") (param $ptr i32) (param $size i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; get_store_id: writes 32 bytes of 0xAB at ptr 256, returns pack_ptr_len(256, 32).
  (func (export "get_store_id") (result i64)
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $done
      (loop $l
        (br_if $done (i32.ge_u (local.get $i) (i32.const 32)))
        (i32.store8 (i32.add (i32.const 256) (local.get $i)) (i32.const 0xAB))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $l)))
    (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const 32)))
)
```

- [ ] **Write the failing test.** Add to `crates/digstore-host/src/memory.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Instance, Module, Store};

    fn fixture() -> (Store<()>, Memory) {
        let engine = Engine::default();
        let wat = include_str!("../tests/fixtures/wat/echo.wat");
        let module = Module::new(&engine, wat).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[]).unwrap();
        let mem = instance.get_memory(&mut store, "memory").unwrap();
        (store, mem)
    }

    #[test]
    fn write_and_read_round_trip() {
        let (mut store, mem) = fixture();
        write_bytes(&mut store, &mem, 512, &[1, 2, 3, 4, 5]).unwrap();
        let got = read_bytes(&store, &mem, 512, 5).unwrap();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn read_out_of_bounds_errors() {
        let (store, mem) = fixture();
        let err = read_bytes(&store, &mem, u32::MAX, 16).unwrap_err();
        assert!(matches!(err, crate::error::HostError::OutOfBounds));
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host memory`. Expected: `cannot find function write_bytes`.

- [ ] **Write the minimal implementation.** Above the test module in `crates/digstore-host/src/memory.rs`:
```rust
//! Helpers for reading/writing the guest's linear memory (§6.4).

use crate::error::HostError;
use wasmtime::{AsContext, AsContextMut, Memory};

/// Read `len` bytes at `ptr` from guest memory.
pub fn read_bytes(
    store: impl AsContext,
    mem: &Memory,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, HostError> {
    let data = mem.data(&store);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or(HostError::OutOfBounds)?;
    if end > data.len() {
        return Err(HostError::OutOfBounds);
    }
    Ok(data[start..end].to_vec())
}

/// Write `bytes` at `ptr` into guest memory.
pub fn write_bytes(
    mut store: impl AsContextMut,
    mem: &Memory,
    ptr: u32,
    bytes: &[u8],
) -> Result<(), HostError> {
    let data = mem.data_mut(&mut store);
    let start = ptr as usize;
    let end = start
        .checked_add(bytes.len())
        .ok_or(HostError::OutOfBounds)?;
    if end > data.len() {
        return Err(HostError::OutOfBounds);
    }
    data[start..end].copy_from_slice(bytes);
    Ok(())
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host memory`. Expected: `test result: ok. 2 passed`.

- [ ] **Commit.** `git add crates/digstore-host/src/memory.rs crates/digstore-host/tests/fixtures/wat/echo.wat` then `git commit -m "feat(host): add guest linear-memory read/write helpers"`.

---

## Task 10 — Engine/Module setup and basic instantiation (§18.1)

This task wires the engine (fuel + epoch enabled), `Module::validate` + instantiate, required-export lookup, and `init()`. The outer memory ceiling (StoreLimits) is deliberately NOT wired here — it gets its own RED-before-GREEN cycle in Task 14.

**Files:**
- Modify `crates/digstore-host/src/runtime.rs`, `src/imports.rs`, `src/lib.rs`
- Create `crates/digstore-host/tests/common/mod.rs`
- Create `crates/digstore-host/tests/instantiate.rs`

Steps:

- [ ] **Write the failing integration test.** Create `crates/digstore-host/tests/instantiate.rs`:
```rust
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

#[test]
fn instantiate_echo_and_call_get_store_id() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/echo.wat")).unwrap();
    let cfg = HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    };
    let mut rt = HostRuntime::new(
        &module_bytes,
        cfg,
        ExecutionLimits::default(),
        test_deps(FixedClock::new(1_700_000_000)),
    )
    .unwrap();

    let id = rt.get_store_id().unwrap();
    assert_eq!(id, vec![0xABu8; 32]);
}
```

- [ ] **Create the shared test helper.** Create `crates/digstore-host/tests/common/mod.rs`:
```rust
use digstore_core::types::{Bytes32, Bytes48};
use digstore_crypto::bls::{derive_public, BlsSecretKey};
use digstore_host::HostDeps;
use digstore_host::FixedClock;
use digstore_prover::mock::{MockChainSource, MockProver};
use std::sync::Arc;

/// Build HostDeps with a deterministic BLS key, mock chain, and mock prover.
/// `clock` is shared (FixedClock clones share their counter) so tests can advance it.
pub fn test_deps(clock: FixedClock) -> HostDeps {
    let sk = BlsSecretKey::from_seed(&[42u8; 32]);
    let pk: Bytes48 = derive_public(&sk);
    HostDeps {
        store_id: Bytes32([0u8; 32]),
        bls_secret: sk,
        bls_public: pk,
        clock: Arc::new(clock),
        chain: Arc::new(MockChainSource::default()),
        prover: Arc::new(MockProver::default()),
        rng_seed: Some([99u8; 32]),
        instance_id: Bytes32([1u8; 32]),
        attestation: None,
    }
}
```

> If digstore-crypto / digstore-prover expose differently named constructors than `BlsSecretKey::from_seed` / `derive_public` / `mock::MockChainSource` / `mock::MockProver`, adjust this helper only; the rest of the plan depends on `HostDeps` fields, not on those constructors.

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test instantiate`. Expected: `cannot find type HostDeps` / `no method named get_store_id`.

- [ ] **Write the minimal runtime.** Put in `crates/digstore-host/src/runtime.rs`:
```rust
//! `HostRuntime`: wasmtime engine + module + serve flow (§18).

use crate::clock::Clock;
use crate::config::ExecutionLimits;
use crate::error::HostError;
use crate::memory::read_bytes;
use crate::random::HostRng;
use crate::session::SessionTable;
use crate::state::{HostKeys, HostState, ReturnBuffer};
use crate::teehook::{BlsAttestationBackend, SharedBackend};
use digstore_core::abi::{is_error, unpack_ptr_len};
use digstore_core::config::HostImportsConfig;
use digstore_core::types::{Bytes32, Bytes48};
use digstore_crypto::bls::BlsSecretKey;
use digstore_prover::{ChainSource, Prover};
use std::sync::Arc;
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

/// Dependencies injected into a runtime: BLS keys, clock, chain, prover, rng.
pub struct HostDeps {
    pub store_id: Bytes32,
    pub bls_secret: BlsSecretKey,
    pub bls_public: Bytes48,
    pub clock: Arc<dyn Clock>,
    pub chain: Arc<dyn ChainSource>,
    pub prover: Arc<dyn Prover>,
    /// `Some(seed)` => deterministic rng (tests); `None` => OS entropy.
    pub rng_seed: Option<[u8; 32]>,
    pub instance_id: Bytes32,
    /// `None` => default BLS attestation backend built from the BLS keys (§13.6).
    pub attestation: Option<SharedBackend>,
}

/// Combined per-store host state. The wasmtime resource limiter is added in Task 14.
pub struct RuntimeState {
    pub host: HostState,
}

pub struct HostRuntime {
    store: Store<RuntimeState>,
    instance: Instance,
    memory: Memory,
    limits_cfg: ExecutionLimits,
}

impl HostRuntime {
    pub fn new(
        module_bytes: &[u8],
        config: HostImportsConfig,
        limits: ExecutionLimits,
        deps: HostDeps,
    ) -> Result<Self, HostError> {
        let mut wcfg = wasmtime::Config::new();
        wcfg.consume_fuel(true);
        wcfg.epoch_interruption(true);
        let engine = Engine::new(&wcfg).map_err(|e| HostError::Wasmtime(e.to_string()))?;

        Module::validate(&engine, module_bytes)
            .map_err(|e| HostError::Validation(e.to_string()))?;
        let module = Module::new(&engine, module_bytes)
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;

        let rng = match deps.rng_seed {
            Some(s) => HostRng::from_seed(s),
            None => HostRng::from_entropy(),
        };

        // Build the attestation backend (clones the secret) so the keys can
        // then be moved into HostKeys (§13.6 default = BLS backend).
        let attestation: SharedBackend = match deps.attestation {
            Some(b) => b,
            None => Arc::new(BlsAttestationBackend::new(
                deps.bls_secret.clone(),
                deps.bls_public,
            )),
        };

        let host = HostState {
            store_id: deps.store_id,
            config: config.clone(),
            return_buffer: ReturnBuffer::new(&config),
            keys: Arc::new(HostKeys {
                bls_secret: deps.bls_secret,
                bls_public: deps.bls_public,
            }),
            attestation,
            clock: deps.clock,
            sessions: SessionTable::new(),
            chain: deps.chain,
            prover: deps.prover,
            rng,
            instance_id: deps.instance_id,
            http_timeout_secs: limits.timeout.as_secs().max(1),
            last_signature: None,
        };

        let mut store = Store::new(&engine, RuntimeState { host });

        let mut linker: Linker<RuntimeState> = Linker::new(&engine);
        crate::imports::register(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(HostError::MissingExport("memory"))?;

        if let Ok(init) = instance.get_typed_func::<(), i32>(&mut store, "init") {
            // arm bounds even for init so a malicious init cannot hang setup.
            let _ = store.set_fuel(limits.fuel);
            let _ = init.call(&mut store, ());
        }

        Ok(HostRuntime {
            store,
            instance,
            memory,
            limits_cfg: limits,
        })
    }

    /// Set the per-export-call fuel budget. Epoch deadline is added in Task 12.
    /// NOTE: bounds are armed PER export call (alloc, serve, dealloc each get
    /// their own budget); the serve flow is not a single combined budget (§18.2).
    fn arm_bounds(&mut self) {
        let _ = self.store.set_fuel(self.limits_cfg.fuel);
    }

    fn map_trap(e: wasmtime::Error) -> HostError {
        HostError::Wasmtime(e.to_string())
    }

    /// Unpack a packed ptr/len, check the error sentinel, and read the bytes.
    fn unpack_and_read(&mut self, packed: i64) -> Result<Vec<u8>, HostError> {
        if is_error(packed) {
            let (ptr, _len) = unpack_ptr_len(packed);
            return Err(HostError::from_guest_code(ptr as i32));
        }
        let (ptr, len) = unpack_ptr_len(packed);
        read_bytes(&self.store, &self.memory, ptr, len)
    }

    fn data_export(&mut self, name: &'static str) -> Result<Vec<u8>, HostError> {
        let func: TypedFunc<(), i64> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport(name))?;
        self.arm_bounds();
        let packed = func.call(&mut self.store, ()).map_err(Self::map_trap)?;
        self.unpack_and_read(packed)
    }

    pub fn get_store_id(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_store_id")
    }

    pub fn get_current_roothash(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_current_roothash")
    }
}
```

- [ ] **Add a no-op imports registrar.** Put in `crates/digstore-host/src/imports.rs`:
```rust
//! Registration of the eight `dig_host` import functions (§6.3, §12, §18.3).

use crate::error::HostError;
use crate::runtime::RuntimeState;
use wasmtime::Linker;

pub fn register(_linker: &mut Linker<RuntimeState>) -> Result<(), HostError> {
    // Imports added in Task 11. The echo fixture imports nothing, so an empty
    // linker instantiates it successfully.
    Ok(())
}
```

- [ ] **Enable the re-export.** In `src/lib.rs` add `pub use runtime::{HostDeps, HostRuntime};`.

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test instantiate`. Expected: `test instantiate_echo_and_call_get_store_id ... ok`.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/src/imports.rs crates/digstore-host/src/lib.rs crates/digstore-host/tests/instantiate.rs crates/digstore-host/tests/common/mod.rs` then `git commit -m "feat(host): instantiate validated module and call data exports"`.

---

## Task 11 — Wire the eight `dig_host` imports

Each import returns `i32` (`>=0` = length written to the return buffer, `<0` = error sentinel), except `host_get_current_time` which returns `i64`.

**Files:**
- Modify `crates/digstore-host/src/imports.rs`, `src/runtime.rs`
- Create `crates/digstore-host/tests/fixtures/wat/import_probe.wat`
- Create `crates/digstore-host/tests/imports_unit.rs`

### 11a — `host_get_current_time` and `host_random_bytes`

- [ ] **Create the import-probe fixture.** Create `crates/digstore-host/tests/fixtures/wat/import_probe.wat`:
```wat
(module
  (import "dig_host" "host_get_public_key" (func $hpk (result i32)))
  (import "dig_host" "host_create_attestation" (func $hca (param i32) (result i32)))
  (import "dig_host" "host_establish_session" (func $hes (param i32) (result i32)))
  (import "dig_host" "host_verify_session" (func $hvs (result i32)))
  (import "dig_host" "jwks_fetch" (func $jf (param i32 i32) (result i32)))
  (import "dig_host" "host_get_current_time" (func $hct (result i64)))
  (import "dig_host" "host_random_bytes" (func $hrb (param i32) (result i32)))
  (import "dig_host" "host_read_return_buffer" (func $hrr (param i32) (result i32)))
  (memory (export "memory") 4 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  (func (export "probe_time") (result i64) (call $hct))
  (func (export "probe_random") (param $n i32) (result i32) (call $hrb (local.get $n)))
  (func (export "probe_pubkey") (result i32) (call $hpk))
  (func (export "probe_attest") (param $p i32) (result i32) (call $hca (local.get $p)))
  (func (export "probe_establish") (param $p i32) (result i32) (call $hes (local.get $p)))
  (func (export "probe_verify") (result i32) (call $hvs))
  (func (export "probe_jwks") (param $p i32) (param $l i32) (result i32)
    (call $jf (local.get $p) (local.get $l)))
  (func (export "probe_read") (param $d i32) (result i32) (call $hrr (local.get $d)))
)
```

- [ ] **Write the failing test for time + random.** Create `crates/digstore-host/tests/imports_unit.rs`:
```rust
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

fn probe_runtime(clock: FixedClock) -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(clock)).unwrap()
}

#[test]
fn host_time_returns_injected_clock() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let t = rt.call_i64_export("probe_time").unwrap();
    assert_eq!(t, 1_700_000_000);
}

#[test]
fn host_random_under_cap_writes_buffer() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export_1("probe_random", 64).unwrap();
    assert_eq!(n, 64);
}

#[test]
fn host_random_over_cap_errors() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export_1("probe_random", 2048).unwrap();
    assert!(n < 0);
}
```

- [ ] **Add the typed test-call helpers to the runtime.** Append a new `impl HostRuntime { ... }` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn call_i64_export(&mut self, name: &str) -> Result<i64, HostError> {
        let f: TypedFunc<(), i64> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i64-export"))?;
        self.arm_bounds();
        f.call(&mut self.store, ()).map_err(Self::map_trap)
    }

    pub fn call_i32_export_1(&mut self, name: &str, arg: i32) -> Result<i32, HostError> {
        let f: TypedFunc<i32, i32> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i32-export-1"))?;
        self.arm_bounds();
        f.call(&mut self.store, arg).map_err(Self::map_trap)
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test imports_unit host_`. Expected FAIL: `host_get_current_time`/`host_random_bytes` are unresolved imports, so `linker.instantiate` returns `unknown import`; `HostRuntime::new(...).unwrap()` panics with `called Result::unwrap() on an Err value: Wasmtime("...unknown import...")`.

- [ ] **Implement all eight imports (real bodies for time/random, stubs for the rest).** Replace `register` in `crates/digstore-host/src/imports.rs`:
```rust
//! Registration of the eight `dig_host` import functions (§6.3, §12, §18.3).

use crate::error::HostError;
use crate::runtime::RuntimeState;
use digstore_core::abi::ErrorCode;
use wasmtime::{Caller, Linker};

pub fn register(linker: &mut Linker<RuntimeState>) -> Result<(), HostError> {
    let m = "dig_host";

    // host_get_current_time() -> i64 (§12). Injectable Clock.
    linker
        .func_wrap(m, "host_get_current_time", |caller: Caller<'_, RuntimeState>| -> i64 {
            caller.data().host.clock.now_unix_secs() as i64
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_random_bytes(count) -> i32 length written, or InvalidParameter (§12).
    linker
        .func_wrap(
            m,
            "host_random_bytes",
            |mut caller: Caller<'_, RuntimeState>, count: i32| -> i32 {
                if count < 0 {
                    return ErrorCode::InvalidParameter as i32;
                }
                let max = caller.data().host.config.max_random_bytes as usize;
                let state = &mut caller.data_mut().host;
                match state.rng.fill(count as usize, max) {
                    Some(bytes) => match state.return_buffer.set(&bytes) {
                        Ok(n) => n as i32,
                        Err(_) => ErrorCode::GeneralError as i32,
                    },
                    None => ErrorCode::InvalidParameter as i32,
                }
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // --- temporary stubs, replaced in 11b–11e ---
    for name in ["host_get_public_key", "host_verify_session"] {
        linker
            .func_wrap(m, name, |_c: Caller<'_, RuntimeState>| -> i32 { 0 })
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;
    }
    for name in ["host_create_attestation", "host_establish_session", "host_read_return_buffer"] {
        linker
            .func_wrap(m, name, |_c: Caller<'_, RuntimeState>, _p: i32| -> i32 {
                ErrorCode::GeneralError as i32
            })
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;
    }
    linker
        .func_wrap(m, "jwks_fetch", |_c: Caller<'_, RuntimeState>, _p: i32, _l: i32| -> i32 {
            ErrorCode::NoSession as i32
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    Ok(())
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test imports_unit host_`. Expected: `host_time_returns_injected_clock ... ok`, `host_random_under_cap_writes_buffer ... ok`, `host_random_over_cap_errors ... ok`.

- [ ] **Commit.** `git add crates/digstore-host/src/imports.rs crates/digstore-host/src/runtime.rs crates/digstore-host/tests/imports_unit.rs crates/digstore-host/tests/fixtures/wat/import_probe.wat` then `git commit -m "feat(host): wire host_get_current_time and host_random_bytes imports"`.

### 11b — `host_read_return_buffer`

- [ ] **Write the failing test.** Add to `tests/imports_unit.rs`:
```rust
#[test]
fn read_return_buffer_copies_into_guest() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export_1("probe_random", 16).unwrap();
    assert_eq!(n, 16);
    let copied = rt.call_i32_export_1("probe_read", 2048).unwrap();
    assert_eq!(copied, 16);
    let mem = rt.read_guest(2048, 16).unwrap();
    assert_eq!(mem.len(), 16);
}
```

- [ ] **Add the `read_guest` accessor.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn read_guest(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, HostError> {
        crate::memory::read_bytes(&self.store, &self.memory, ptr, len)
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test imports_unit read_return_buffer`. Expected FAIL: stub returns `GeneralError` (-1), so `assert_eq!(copied, 16)` fails with `left: -1, right: 16`.

- [ ] **Implement `host_read_return_buffer`.** In `imports.rs` remove `host_read_return_buffer` from the stub loop and add (place before `Ok(())`):
```rust
    // host_read_return_buffer(dest_ptr) -> i32 bytes copied (§6.4).
    linker
        .func_wrap(
            m,
            "host_read_return_buffer",
            |mut caller: Caller<'_, RuntimeState>, dest_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let buf = caller.data().host.return_buffer.as_slice().to_vec();
                let data = mem.data_mut(&mut caller);
                let start = dest_ptr as usize;
                let end = match start.checked_add(buf.len()) {
                    Some(e) => e,
                    None => return ErrorCode::InvalidParameter as i32,
                };
                if end > data.len() {
                    return ErrorCode::BufferTooSmall as i32;
                }
                data[start..end].copy_from_slice(&buf);
                buf.len() as i32
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test imports_unit read_return_buffer`. Expected: `read_return_buffer_copies_into_guest ... ok`.

- [ ] **Commit.** `git add crates/digstore-host/src/imports.rs crates/digstore-host/src/runtime.rs crates/digstore-host/tests/imports_unit.rs` then `git commit -m "feat(host): implement host_read_return_buffer memcpy into guest memory"`.

### 11c — `host_get_public_key`

- [ ] **Write the failing test.** Add to `tests/imports_unit.rs`:
```rust
#[test]
fn host_public_key_returns_48_bytes() {
    let mut rt = probe_runtime(FixedClock::new(100));
    let n = rt.call_i32_export("probe_pubkey").unwrap();
    assert_eq!(n, 48);
}
```

- [ ] **Add the no-arg i32 helper.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn call_i32_export(&mut self, name: &str) -> Result<i32, HostError> {
        let f: TypedFunc<(), i32> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i32-export-0"))?;
        self.arm_bounds();
        f.call(&mut self.store, ()).map_err(Self::map_trap)
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test imports_unit host_public_key`. Expected FAIL: stub returns `0`, so `assert_eq!(n, 48)` fails with `left: 0, right: 48`.

- [ ] **Implement `host_get_public_key`.** In `imports.rs` remove `host_get_public_key` from the i32-zero stub loop and add:
```rust
    // host_get_public_key() -> i32 length (48 bytes BLS G1) written (§12).
    linker
        .func_wrap(m, "host_get_public_key", |mut caller: Caller<'_, RuntimeState>| -> i32 {
            let pk = caller.data().host.keys.bls_public.0; // [u8; 48]
            match caller.data_mut().host.return_buffer.set(&pk) {
                Ok(n) => n as i32,
                Err(_) => ErrorCode::GeneralError as i32,
            }
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test imports_unit host_public_key`. Expected: `host_public_key_returns_48_bytes ... ok`.

- [ ] **Commit.** `git add crates/digstore-host/src/imports.rs crates/digstore-host/src/runtime.rs crates/digstore-host/tests/imports_unit.rs` then `git commit -m "feat(host): implement host_get_public_key import"`.

### 11d — `host_create_attestation`, `host_establish_session`, `host_verify_session`

`host_create_attestation(challenge_ptr)` reads `AttestationChallenge` (nonce[32] ‖ store_id[32] ‖ timestamp u64 BE) from guest memory, signs the raw challenge bytes via the attestation backend (§13.6), writes `AttestationResponse` (host_public_key[48] ‖ host_instance_id[32] ‖ signature[96]) into the return buffer, returns its length. `host_establish_session(challenge_ptr)` records a session keyed by nonce+store_id with a TTL; `host_verify_session()` returns 1/0. Because Task 7 already defined the FINAL `AttestationBackend` with `public_key()`, the body below uses `state.attestation.public_key()` directly — no temporary substitution.

- [ ] **Write the failing tests.** Add to `tests/imports_unit.rs`:
```rust
const CHALLENGE_LEN: usize = 32 + 32 + 8;
const ATTESTATION_LEN: usize = 48 + 32 + 96;

fn write_challenge(rt: &mut HostRuntime, ptr: u32) {
    let mut challenge = vec![0u8; CHALLENGE_LEN];
    challenge[0..32].fill(0x01);
    challenge[32..64].fill(0x02);
    challenge[64..72].copy_from_slice(&1_700_000_000u64.to_be_bytes());
    rt.write_guest(ptr, &challenge).unwrap();
}

#[test]
fn create_attestation_writes_response() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    write_challenge(&mut rt, 4096);
    let n = rt.call_i32_export_1("probe_attest", 4096).unwrap();
    assert_eq!(n as usize, ATTESTATION_LEN);
    let resp = rt.read_return_buffer_copy().unwrap();
    assert_eq!(resp.len(), ATTESTATION_LEN);
}

#[test]
fn establish_then_verify_session() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    write_challenge(&mut rt, 4096);
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), 0);
    let r = rt.call_i32_export_1("probe_establish", 4096).unwrap();
    assert!(r >= 0);
    assert_eq!(rt.call_i32_export("probe_verify").unwrap(), 1);
}
```

- [ ] **Add `write_guest` and `read_return_buffer_copy` helpers.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn write_guest(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), HostError> {
        crate::memory::write_bytes(&mut self.store, &self.memory, ptr, bytes)
    }

    pub fn read_return_buffer_copy(&mut self) -> Result<Vec<u8>, HostError> {
        Ok(self.store.data().host.return_buffer.as_slice().to_vec())
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test imports_unit attestation session`. Expected FAIL: stubs return `GeneralError`/`0`, so the length and verify assertions fail (`create_attestation_writes_response`: `left: -1, right: 176`; `establish_then_verify_session`: second verify `left: 0, right: 1`).

- [ ] **Implement attestation + session imports.** In `imports.rs` remove `host_create_attestation`/`host_establish_session` from the i32-arg stub loop and `host_verify_session` from the i32-zero loop, then add (place before `Ok(())`):
```rust
    const CHALLENGE_LEN: usize = 32 + 32 + 8;
    const SESSION_TTL_SECS: u64 = 300;

    // host_create_attestation(challenge_ptr) -> i32 length of AttestationResponse (§12, §13.6).
    linker
        .func_wrap(
            m,
            "host_create_attestation",
            |mut caller: Caller<'_, RuntimeState>, challenge_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let data = mem.data(&caller);
                let start = challenge_ptr as usize;
                let end = match start.checked_add(CHALLENGE_LEN) {
                    Some(e) if e <= data.len() => e,
                    _ => return ErrorCode::InvalidParameter as i32,
                };
                let challenge = data[start..end].to_vec();
                let state = &mut caller.data_mut().host;
                let sig = match state.attestation.attest(&challenge) {
                    Ok(s) => s,
                    Err(_) => return ErrorCode::AttestationFailed as i32,
                };
                let pk = state.attestation.public_key();
                let mut resp = Vec::with_capacity(48 + 32 + 96);
                resp.extend_from_slice(&pk.0);
                resp.extend_from_slice(&state.instance_id.0);
                resp.extend_from_slice(&sig.0);
                state.last_signature = Some(sig);
                match state.return_buffer.set(&resp) {
                    Ok(n) => n as i32,
                    Err(_) => ErrorCode::GeneralError as i32,
                }
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_establish_session(challenge_ptr) -> i32 (>=0 ok) (§12).
    linker
        .func_wrap(
            m,
            "host_establish_session",
            |mut caller: Caller<'_, RuntimeState>, challenge_ptr: i32| -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let data = mem.data(&caller);
                let start = challenge_ptr as usize;
                let end = match start.checked_add(CHALLENGE_LEN) {
                    Some(e) if e <= data.len() => e,
                    _ => return ErrorCode::InvalidParameter as i32,
                };
                let mut nonce = [0u8; 32];
                let mut store_id = [0u8; 32];
                nonce.copy_from_slice(&data[start..start + 32]);
                store_id.copy_from_slice(&data[start + 32..start + 64]);
                let now = caller.data().host.clock.now_unix_secs();
                caller
                    .data_mut()
                    .host
                    .sessions
                    .establish(nonce, store_id, now, SESSION_TTL_SECS);
                0
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;

    // host_verify_session() -> i32 (1 valid / 0 invalid) (§12).
    linker
        .func_wrap(m, "host_verify_session", |caller: Caller<'_, RuntimeState>| -> i32 {
            let now = caller.data().host.clock.now_unix_secs();
            if caller.data().host.sessions.is_valid(now) { 1 } else { 0 }
        })
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test imports_unit attestation session`. Expected: `create_attestation_writes_response ... ok`, `establish_then_verify_session ... ok`. (Both route through the default `BlsAttestationBackend` because `test_deps` passes `attestation: None`.)

- [ ] **Commit.** `git add crates/digstore-host/src/imports.rs crates/digstore-host/src/runtime.rs crates/digstore-host/tests/imports_unit.rs` then `git commit -m "feat(host): implement attestation signing and session establish/verify imports"`.

### 11e — `jwks_fetch` gating (NoSession before session)

This step implements ONLY the session-gating branch of `jwks_fetch` and tests it. The HTTP success path is introduced with its own RED test in Task 15.

- [ ] **Write the failing test (gating).** Add to `tests/imports_unit.rs`:
```rust
#[test]
fn jwks_fetch_blocked_without_session() {
    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let url = b"http://127.0.0.1:1/jwks.json";
    rt.write_guest(5000, url).unwrap();
    let r = rt.call_i32_export_2("probe_jwks", 5000, url.len() as i32).unwrap();
    assert_eq!(r, -100); // ErrorCode::NoSession
}
```

- [ ] **Add the two-i32-arg helper.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn call_i32_export_2(&mut self, name: &str, a: i32, b: i32) -> Result<i32, HostError> {
        let f: TypedFunc<(i32, i32), i32> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i32-export-2"))?;
        self.arm_bounds();
        f.call(&mut self.store, (a, b)).map_err(Self::map_trap)
    }
}
```

- [ ] **Run it (expect FAIL — force a true red).** The current `jwks_fetch` stub already returns `NoSession`, which would make this test pass without exercising real gating logic. To force a genuine RED, temporarily change the stub body to return `0` instead of `ErrorCode::NoSession as i32`, then `cargo test -p digstore-host --test imports_unit jwks_fetch_blocked`. Expected FAIL: `left: 0, right: -100`.

- [ ] **Implement the gating-only `jwks_fetch`.** In `imports.rs` remove the temporary `jwks_fetch` stub and add (place before `Ok(())`) the gating-only body. The HTTP success path is added in Task 15:
```rust
    // jwks_fetch(url_ptr, url_len) -> i32. SESSION-GATED (§6.3).
    // Gating only here; the HTTP success path is added in Task 15.
    linker
        .func_wrap(
            m,
            "jwks_fetch",
            |caller: Caller<'_, RuntimeState>, _url_ptr: i32, _url_len: i32| -> i32 {
                let now = caller.data().host.clock.now_unix_secs();
                if !caller.data().host.sessions.is_valid(now) {
                    return ErrorCode::NoSession as i32;
                }
                // Session valid but fetch not yet implemented; Task 15 fills this in.
                ErrorCode::NetworkError as i32
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;
```

- [ ] **Run it (expect PASS) and re-run the whole import suite.** `cargo test -p digstore-host --test imports_unit jwks_fetch_blocked` then `cargo test -p digstore-host --test imports_unit`. Expected: gating test and all prior import tests green.

- [ ] **Commit.** `git add crates/digstore-host/src/imports.rs crates/digstore-host/src/runtime.rs crates/digstore-host/tests/imports_unit.rs` then `git commit -m "feat(host): session-gate jwks_fetch (NoSession before session)"`.

---

## Task 12 — Execution bounds: wall-clock timeout AND fuel (§18.2)

Both the wall-clock timeout (epoch interruption) and fuel exhaustion get their own RED-before-GREEN cycle here. We build the timeout test first, implement the epoch ticker and the `Interrupt` arm of `map_trap`, then add the fuel test as a second RED that forces the `OutOfFuel` arm.

**Files:**
- Modify `crates/digstore-host/src/runtime.rs`
- Create `crates/digstore-host/tests/fixtures/wat/spin.wat`
- Create `crates/digstore-host/tests/bounds.rs`

### 12a — Wall-clock timeout via epoch interruption

- [ ] **Create the spin fixture.** Create `crates/digstore-host/tests/fixtures/wat/spin.wat`:
```wat
(module
  (memory (export "memory") 1 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  (func (export "get_store_id") (result i64)
    (loop $l (br $l))
    (i64.const 0))
)
```

- [ ] **Write the failing test.** Create `crates/digstore-host/tests/bounds.rs`:
```rust
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostError, HostRuntime};
use std::time::Duration;

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

#[test]
fn timeout_terminates_runaway_export() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/spin.wat")).unwrap();
    let mut limits = ExecutionLimits::default();
    limits.timeout = Duration::from_millis(300);
    limits.fuel = u64::MAX; // isolate: prove TIMEOUT triggers, not fuel
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let start = std::time::Instant::now();
    let err = rt.get_store_id().unwrap_err();
    assert!(matches!(err, HostError::Timeout), "expected Timeout, got {err:?}");
    assert!(start.elapsed() < Duration::from_secs(3));
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test bounds timeout`. Expected FAIL: with no ticker the call hangs; the test harness times out, or once the ticker exists but the trap isn't mapped it returns `HostError::Wasmtime(..)` not `Timeout`. Confirm it does not return `Timeout`.

- [ ] **Implement the epoch ticker, deadline, and Interrupt trap mapping.** In `runtime.rs` add imports and the `EpochTicker` near the top (after the existing `use` lines):
```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: Engine, period: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                std::thread::sleep(period);
                engine.increment_epoch();
            }
        });
        EpochTicker { stop, handle: Some(handle) }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
```
Add a `_ticker: EpochTicker` field to `HostRuntime`:
```rust
pub struct HostRuntime {
    store: Store<RuntimeState>,
    instance: Instance,
    memory: Memory,
    limits_cfg: ExecutionLimits,
    _ticker: EpochTicker,
}
```
In `new`, after `let engine = ...`, create the ticker:
```rust
        let period = (limits.timeout / 2).max(Duration::from_millis(10));
        let ticker = EpochTicker::start(engine.clone(), period);
```
and set `_ticker: ticker,` in the returned `Ok(HostRuntime { ... })`. Update `arm_bounds` to also set the epoch deadline, and replace `map_trap` to recognize the timeout (Interrupt) trap (the fuel arm is added in 12b):
```rust
    fn arm_bounds(&mut self) {
        let _ = self.store.set_fuel(self.limits_cfg.fuel);
        // Deadline = 2 epoch ticks (the ticker fires every timeout/2).
        // NOTE: bounds are armed per export call, not once per serve sequence (§18.2).
        self.store.set_epoch_deadline(2);
    }

    fn map_trap(e: wasmtime::Error) -> HostError {
        use wasmtime::Trap;
        if let Some(trap) = e.downcast_ref::<Trap>() {
            if let Trap::Interrupt = trap {
                return HostError::Timeout;
            }
        }
        HostError::Wasmtime(e.to_string())
    }
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test bounds timeout`. Expected: `timeout_terminates_runaway_export ... ok` within ~300-600 ms. Re-run prior suites: `cargo test -p digstore-host --test imports_unit` and `--test instantiate` still green.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/tests/fixtures/wat/spin.wat crates/digstore-host/tests/bounds.rs` then `git commit -m "feat(host): enforce wall-clock timeout via epoch interruption"`.

### 12b — Fuel exhaustion bound

- [ ] **Write the failing test.** Add to `tests/bounds.rs`:
```rust
#[test]
fn fuel_exhaustion_terminates_export() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/spin.wat")).unwrap();
    let mut limits = ExecutionLimits::default();
    limits.timeout = Duration::from_secs(30); // isolate: prove FUEL triggers, not timeout
    limits.fuel = 1_000_000;
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let err = rt.get_store_id().unwrap_err();
    assert!(matches!(err, HostError::OutOfFuel), "expected OutOfFuel, got {err:?}");
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test bounds fuel`. Expected FAIL: `map_trap` currently only handles `Trap::Interrupt`, so an exhausted-fuel trap falls through to `HostError::Wasmtime(..)`, and the test fails with `expected OutOfFuel, got Wasmtime("all fuel consumed by WebAssembly")`.

- [ ] **Add the `OutOfFuel` arm to `map_trap`.** In `runtime.rs` extend `map_trap`:
```rust
    fn map_trap(e: wasmtime::Error) -> HostError {
        use wasmtime::Trap;
        if let Some(trap) = e.downcast_ref::<Trap>() {
            match trap {
                Trap::Interrupt => return HostError::Timeout,
                Trap::OutOfFuel => return HostError::OutOfFuel,
                _ => {}
            }
        }
        HostError::Wasmtime(e.to_string())
    }
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test bounds fuel`. Expected: `fuel_exhaustion_terminates_export ... ok`. Re-run `cargo test -p digstore-host --test bounds timeout` to confirm the Interrupt path still maps to `Timeout`.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/tests/bounds.rs` then `git commit -m "feat(host): map fuel exhaustion trap to OutOfFuel"`.

---

## Task 13 — Outer memory ceiling (StoreLimits, §18.2)

§18.2: the host enforces a memory ceiling. We add the limiter with a RED-before-GREEN cycle: first a test that a small ceiling refuses an oversized `memory.grow` (RED, because Task 10 did not wire the limiter), then wire `StoreLimitsBuilder` + `store.limiter`, then GREEN.

**Files:**
- Create `crates/digstore-host/tests/fixtures/wat/grow.wat`
- Modify `crates/digstore-host/src/runtime.rs`
- Modify `crates/digstore-host/tests/bounds.rs`

Steps:

- [ ] **Create the grow fixture.** Create `crates/digstore-host/tests/fixtures/wat/grow.wat`:
```wat
(module
  (memory (export "memory") 1 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; get_store_id: try to grow by 200 pages; if grow returns -1, trap.
  (func (export "get_store_id") (result i64)
    (if (i32.eq (memory.grow (i32.const 200)) (i32.const -1))
      (then (unreachable)))
    (i64.const 0))
)
```

- [ ] **Write the failing test.** Add to `tests/bounds.rs`:
```rust
#[test]
fn memory_ceiling_blocks_oversized_grow() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/grow.wat")).unwrap();
    let mut limits = ExecutionLimits::default();
    limits.memory_bytes_max = 64 * 64 * 1024; // 64 pages = 4 MiB
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let err = rt.get_store_id().unwrap_err();
    assert!(
        matches!(err, HostError::Wasmtime(_) | HostError::MemoryLimit),
        "expected memory-limit-induced trap, got {err:?}"
    );
}

#[test]
fn memory_ceiling_allows_within_limit() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/grow.wat")).unwrap();
    let limits = ExecutionLimits::default(); // 256 pages, room for +200
    let mut rt = HostRuntime::new(&module_bytes, cfg(), limits, test_deps(FixedClock::new(100))).unwrap();
    let out = rt.get_store_id().unwrap(); // grow(200) from 1 page = 201 <= 256
    assert!(out.is_empty()); // pack_ptr_len(0,0) -> empty read
}
```

> `get_store_id` returns `i64.const 0` = `pack_ptr_len(0,0)`. `is_error(0)` is false (ptr=0 not `<0`), so `unpack_ptr_len` yields (0,0) and `read_bytes(.., 0, 0)` returns an empty `Vec` — validating the within-limit path.

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test bounds memory_ceiling`. Expected FAIL: no limiter is wired yet, so the 4 MiB ceiling is not enforced; `memory.grow(200)` from 1 page (201 pages = ~12.9 MiB, under the module's own 256-page max) succeeds, the guest does NOT trap, `get_store_id` returns `Ok(empty)`, and `memory_ceiling_blocks_oversized_grow` panics with `called Result::unwrap_err() on an Ok value`.

- [ ] **Wire the StoreLimits limiter.** In `runtime.rs` update the imports line that pulls from `wasmtime` to include `StoreLimits, StoreLimitsBuilder`:
```rust
use wasmtime::{
    Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};
```
Change `RuntimeState` to carry the limiter:
```rust
pub struct RuntimeState {
    pub host: HostState,
    pub limits: StoreLimits,
}
```
In `new`, after computing `let store_limits = ...`, build the store with the limiter and register it. Replace the existing `let mut store = Store::new(&engine, RuntimeState { host });` with:
```rust
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.memory_bytes_max)
            .build();

        let mut store = Store::new(&engine, RuntimeState { host, limits: store_limits });
        store.limiter(|s| &mut s.limits);
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test bounds memory_ceiling`. Expected: both `memory_ceiling_blocks_oversized_grow ... ok` and `memory_ceiling_allows_within_limit ... ok`. Re-run the full bounds file `cargo test -p digstore-host --test bounds` (timeout + fuel + memory) all green.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/tests/fixtures/wat/grow.wat crates/digstore-host/tests/bounds.rs` then `git commit -m "feat(host): enforce outer memory ceiling via StoreLimits"`.

---

## Task 14 — `jwks_fetch` HTTP success path + mock server (§6.3)

§6.3: once a session is established, `jwks_fetch` performs the blocking GET and returns the body length. This is the RED-before-GREEN cycle for the HTTP success behavior: the Task 11e gating-only body returns `NetworkError` after a valid session, so a success test fails first; then we add the reqwest GET + buffer write. The request timeout is derived from `HostState::http_timeout_secs` (seeded from `ExecutionLimits.timeout`), NOT a hard-coded constant.

> **§18.2 deviation note (documented):** epoch interruption only interrupts WASM execution, not blocking host calls. A hung jwks endpoint is therefore bounded by the reqwest request timeout (derived from `ExecutionLimits.timeout`), independent of the export wall-clock bound. This is intentional and surfaced via `HostState::http_timeout_secs`.

**Files:**
- Modify `crates/digstore-host/src/imports.rs`
- Create `crates/digstore-host/tests/jwks_mock.rs`

Steps:

- [ ] **Write the failing test.** Create `crates/digstore-host/tests/jwks_mock.rs`:
```rust
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};
use httpmock::prelude::*;

mod common;
use common::test_deps;

const CHALLENGE_LEN: usize = 32 + 32 + 8;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

fn probe_runtime(clock: FixedClock) -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(clock)).unwrap()
}

#[test]
fn jwks_fetch_nosession_then_success() {
    let server = MockServer::start();
    let body = br#"{"keys":[]}"#;
    let mock = server.mock(|when, then| {
        when.method(GET).path("/jwks.json");
        then.status(200).body(body);
    });

    let mut rt = probe_runtime(FixedClock::new(1_700_000_000));
    let url = format!("{}/jwks.json", server.base_url());
    rt.write_guest(5000, url.as_bytes()).unwrap();

    // 1. NoSession before any session.
    let r0 = rt.call_i32_export_2("probe_jwks", 5000, url.len() as i32).unwrap();
    assert_eq!(r0, -100);

    // 2. Establish a session.
    let mut challenge = vec![0u8; CHALLENGE_LEN];
    challenge[0..32].fill(0x01);
    challenge[32..64].fill(0x02);
    challenge[64..72].copy_from_slice(&1_700_000_000u64.to_be_bytes());
    rt.write_guest(4096, &challenge).unwrap();
    assert!(rt.call_i32_export_1("probe_establish", 4096).unwrap() >= 0);

    // 3. Now the fetch succeeds and returns the body length.
    let r1 = rt.call_i32_export_2("probe_jwks", 5000, url.len() as i32).unwrap();
    assert_eq!(r1 as usize, body.len());
    mock.assert();

    let got = rt.read_return_buffer_copy().unwrap();
    assert_eq!(&got, body);
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test jwks_mock`. Expected FAIL: after establishing a session the Task 11e gating-only body returns `ErrorCode::NetworkError` (-200), so `assert_eq!(r1 as usize, body.len())` fails (`r1 = -200`, cast to `usize` is huge / assertion mismatch) and `mock.assert()` would fail because no request was sent.

- [ ] **Implement the HTTP success path.** In `imports.rs` replace the entire `jwks_fetch` registration body (the gating-only one from Task 11e) with the full implementation:
```rust
    // jwks_fetch(url_ptr, url_len) -> i32. SESSION-GATED (§6.3).
    // NOTE (§18.2): epoch interruption does not cover blocking host I/O; this
    // call is bounded by its own reqwest timeout, derived from ExecutionLimits.
    linker
        .func_wrap(
            m,
            "jwks_fetch",
            |mut caller: Caller<'_, RuntimeState>, url_ptr: i32, url_len: i32| -> i32 {
                let now = caller.data().host.clock.now_unix_secs();
                if !caller.data().host.sessions.is_valid(now) {
                    return ErrorCode::NoSession as i32;
                }
                let timeout_secs = caller.data().host.http_timeout_secs;
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(mem) => mem,
                    None => return ErrorCode::GeneralError as i32,
                };
                let data = mem.data(&caller);
                let start = url_ptr as usize;
                let end = match start.checked_add(url_len as usize) {
                    Some(e) if e <= data.len() => e,
                    _ => return ErrorCode::InvalidParameter as i32,
                };
                let url = match std::str::from_utf8(&data[start..end]) {
                    Ok(u) => u.to_string(),
                    Err(_) => return ErrorCode::InvalidParameter as i32,
                };
                let resp = match reqwest::blocking::Client::new()
                    .get(&url)
                    .timeout(std::time::Duration::from_secs(timeout_secs))
                    .send()
                {
                    Ok(r) => r,
                    Err(e) if e.is_timeout() => return ErrorCode::Timeout as i32,
                    Err(_) => return ErrorCode::NetworkError as i32,
                };
                let body = match resp.bytes() {
                    Ok(b) => b,
                    Err(_) => return ErrorCode::NetworkError as i32,
                };
                match caller.data_mut().host.return_buffer.set(&body) {
                    Ok(n) => n as i32,
                    Err(_) => ErrorCode::GeneralError as i32,
                }
            },
        )
        .map_err(|e| HostError::Wasmtime(e.to_string()))?;
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test jwks_mock`. Expected: `jwks_fetch_nosession_then_success ... ok`. Re-run `cargo test -p digstore-host --test imports_unit jwks_fetch_blocked` to confirm gating still holds. If `r1` is negative, confirm the 300s session TTL covers the fixed clock and the loopback mock is reachable from `reqwest::blocking`.

- [ ] **Commit.** `git add crates/digstore-host/src/imports.rs crates/digstore-host/tests/jwks_mock.rs` then `git commit -m "feat(host): jwks_fetch HTTP success path via blocking reqwest (mock-tested)"`.

---

## Task 15 — Return-buffer round-trip + grow-on-demand integration (§6.4)

§6.4: validate the full buffer contract through a fixture that fills the host buffer via an import, copies it into guest memory via `host_read_return_buffer`, including growth past the 64 KiB initial capacity.

**Files:**
- Create `crates/digstore-host/tests/fixtures/wat/return_buffer.wat`
- Create `crates/digstore-host/tests/return_buffer.rs`
- Modify `crates/digstore-host/src/runtime.rs`

Steps:

- [ ] **Create the return-buffer fixture.** Create `crates/digstore-host/tests/fixtures/wat/return_buffer.wat`:
```wat
(module
  (import "dig_host" "host_random_bytes" (func $hrb (param i32) (result i32)))
  (import "dig_host" "host_read_return_buffer" (func $hrr (param i32) (result i32)))
  (memory (export "memory") 8 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; fill_and_read(n): random(n) -> buffer; copy buffer into mem@131072; return packed.
  (func (export "fill_and_read") (param $n i32) (result i64)
    (local $w i32) (local $copied i32)
    (local.set $w (call $hrb (local.get $n)))
    (if (i32.lt_s (local.get $w) (i32.const 0))
      (then (return (i64.shl (i64.extend_i32_s (local.get $w)) (i64.const 32)))))
    (local.set $copied (call $hrr (i32.const 131072)))
    (i64.or
      (i64.shl (i64.const 131072) (i64.const 32))
      (i64.extend_i32_u (local.get $copied))))
)
```

> Memory starts at 8 pages (512 KiB) so copying up to 128 KiB at offset 131072 is in bounds.

- [ ] **Write the failing test.** Create `crates/digstore-host/tests/return_buffer.rs`:
```rust
use digstore_core::abi::unpack_ptr_len;
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 256 * 1024, // raise cap so we can request 128 KiB
        host_version: "dig-host-test/0.1".to_string(),
    }
}

fn rb_rt() -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/return_buffer.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(100))).unwrap()
}

#[test]
fn small_buffer_round_trip() {
    let mut rt = rb_rt();
    let packed = rt.call_i64_export_1("fill_and_read", 100).unwrap();
    let (ptr, len) = unpack_ptr_len(packed);
    assert_eq!(len, 100);
    let bytes = rt.read_guest(ptr, len).unwrap();
    assert_eq!(bytes.len(), 100);
}

#[test]
fn buffer_grows_past_initial_capacity() {
    let mut rt = rb_rt();
    let packed = rt.call_i64_export_1("fill_and_read", 128 * 1024).unwrap();
    let (ptr, len) = unpack_ptr_len(packed);
    assert_eq!(len, 128 * 1024);
    let bytes = rt.read_guest(ptr, len).unwrap();
    assert_eq!(bytes.len(), 128 * 1024);
}
```

- [ ] **Add the one-i32-arg i64 helper.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn call_i64_export_1(&mut self, name: &str, arg: i32) -> Result<i64, HostError> {
        let f: TypedFunc<i32, i64> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i64-export-1"))?;
        self.arm_bounds();
        f.call(&mut self.store, arg).map_err(Self::map_trap)
    }
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test return_buffer`. Expected FAIL: `no method named call_i64_export_1 found` — the helper does not exist until the previous step compiles. (If run before adding the helper, expect a compile error; after adding the helper the tests compile and exercise Tasks 8/11a/11b. If the grow case returns `BufferTooSmall` (-3) from `host_read_return_buffer`, increase the fixture's initial memory pages.)

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test return_buffer`. Expected: both `small_buffer_round_trip ... ok` and `buffer_grows_past_initial_capacity ... ok`.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/tests/fixtures/wat/return_buffer.wat crates/digstore-host/tests/return_buffer.rs` then `git commit -m "test(host): return-buffer round-trip and grow-on-demand"`.

---

## Task 16 — Serve flow: `serve_content` and `serve_proof` (§18.4)

Serve flow: `alloc(req_len)` → `write_bytes(request)` → `get_content/get_proof(ptr,len)` → `is_error`? else `unpack_ptr_len` → `read_bytes(out)` → `dealloc(req_ptr,req_len)`. Request and response are opaque (never decrypted). `serve_via` arms bounds before each sub-call, so alloc/serve/dealloc are each independently bounded (per §18.2 — not one combined budget).

**Files:**
- Modify `crates/digstore-host/src/runtime.rs`
- Create `crates/digstore-host/tests/fixtures/wat/serve_echo.wat`
- Create `crates/digstore-host/tests/serve_flow.rs`

Steps:

- [ ] **Create the echo serve fixture.** Create `crates/digstore-host/tests/fixtures/wat/serve_echo.wat`:
```wat
(module
  (memory (export "memory") 2 256)
  (global $bump (mut i32) (i32.const 8192))
  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $size)))
    (local.get $ptr))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; copy [req_ptr, req_ptr+len) -> [65536, 65536+len); return pack_ptr_len(65536, len)
  (func $echo (param $req_ptr i32) (param $req_len i32) (result i64)
    (local $out i32) (local $i i32)
    (local.set $out (i32.const 65536))
    (local.set $i (i32.const 0))
    (block $done
      (loop $l
        (br_if $done (i32.ge_u (local.get $i) (local.get $req_len)))
        (i32.store8
          (i32.add (local.get $out) (local.get $i))
          (i32.load8_u (i32.add (local.get $req_ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $l)))
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
      (i64.extend_i32_u (local.get $req_len))))
  (func (export "get_content") (param $p i32) (param $l i32) (result i64)
    (call $echo (local.get $p) (local.get $l)))
  (func (export "get_proof") (param $p i32) (param $l i32) (result i64)
    (call $echo (local.get $p) (local.get $l)))
)
```

> Memory starts at 2 pages (128 KiB) so writing the echo output at offset 65536 is in bounds for the test request sizes (≤ 1 KiB).

- [ ] **Write the failing test.** Create `crates/digstore-host/tests/serve_flow.rs`:
```rust
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};

mod common;
use common::test_deps;

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

fn echo_rt() -> HostRuntime {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/serve_echo.wat")).unwrap();
    HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(100))).unwrap()
}

#[test]
fn serve_content_round_trips_request_bytes() {
    let mut rt = echo_rt();
    let req = b"retrieval-key-and-root-and-range-bytes".to_vec();
    let out = rt.serve_content(&req).unwrap();
    assert_eq!(out, req);
}

#[test]
fn serve_proof_round_trips_request_bytes() {
    let mut rt = echo_rt();
    let req = vec![0xCDu8; 1024];
    let out = rt.serve_proof(&req).unwrap();
    assert_eq!(out, req);
}

#[test]
fn serve_content_empty_request_is_ok() {
    let mut rt = echo_rt();
    let out = rt.serve_content(&[]).unwrap();
    assert!(out.is_empty());
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test serve_flow`. Expected: `no method named serve_content found`.

- [ ] **Implement the serve flow.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    /// §18.4 serve flow for content. Treats request/response as opaque bytes;
    /// the host NEVER decrypts or inspects the payload.
    pub fn serve_content(&mut self, request: &[u8]) -> Result<Vec<u8>, HostError> {
        self.serve_via("get_content", request)
    }

    /// §18.4 serve flow for proofs.
    pub fn serve_proof(&mut self, request: &[u8]) -> Result<Vec<u8>, HostError> {
        self.serve_via("get_proof", request)
    }

    fn serve_via(&mut self, export: &'static str, request: &[u8]) -> Result<Vec<u8>, HostError> {
        // 1. alloc(req_len) — bounds armed per sub-call (§18.2).
        let alloc: TypedFunc<i32, i32> = self
            .instance
            .get_typed_func(&mut self.store, "alloc")
            .map_err(|_| HostError::MissingExport("alloc"))?;
        self.arm_bounds();
        let req_ptr = alloc
            .call(&mut self.store, request.len() as i32)
            .map_err(Self::map_trap)?;

        // 2. write request bytes
        crate::memory::write_bytes(&mut self.store, &self.memory, req_ptr as u32, request)?;

        // 3. call get_content/get_proof(ptr, len)
        let serve: TypedFunc<(i32, i32), i64> = self
            .instance
            .get_typed_func(&mut self.store, export)
            .map_err(|_| HostError::MissingExport(export))?;
        self.arm_bounds();
        let packed = serve
            .call(&mut self.store, (req_ptr, request.len() as i32))
            .map_err(Self::map_trap)?;

        // 4-5. is_error? else unpack + read
        let out = self.unpack_and_read(packed);

        // 6. dealloc(req_ptr, req_len) — best effort.
        if let Ok(dealloc) = self
            .instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, "dealloc")
        {
            self.arm_bounds();
            let _ = dealloc.call(&mut self.store, (req_ptr, request.len() as i32));
        }

        out
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test serve_flow`. Expected: 3 tests `ok`. Confirm `cargo build -p digstore-host`.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/tests/fixtures/wat/serve_echo.wat crates/digstore-host/tests/serve_flow.rs` then `git commit -m "feat(host): implement serve_content/serve_proof flow per 18.4"`.

---

## Task 17 — Error-sentinel propagation in serve flow (§18.4)

`serve_*` must surface a guest error sentinel (`is_error(packed)` true: `len==0 && (ptr as i32) < 0`) as `HostError::GuestError(...)`. This is the RED-before-GREEN cycle for the error branch of `unpack_and_read` as exercised through the serve flow: a fixture whose `get_content` returns a `NotFound` sentinel must surface `GuestError(NotFound)`.

**Files:**
- Create `crates/digstore-host/tests/fixtures/wat/serve_err.wat`
- Modify `crates/digstore-host/tests/serve_flow.rs`

Steps:

- [ ] **Create the error-returning serve fixture.** Create `crates/digstore-host/tests/fixtures/wat/serve_err.wat`:
```wat
(module
  (memory (export "memory") 1 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; pack_ptr_len(ptr=-300, len=0): ((-300) << 32) | 0  (NotFound sentinel)
  (func (export "get_content") (param i32) (param i32) (result i64)
    (i64.shl (i64.const -300) (i64.const 32)))
  (func (export "get_proof") (param i32) (param i32) (result i64)
    (i64.shl (i64.const -300) (i64.const 32)))
)
```

> `(-300 as i64) << 32` puts `-300` in the high word and zero in the low word. `unpack_ptr_len` reads `ptr = (packed >> 32) as u32 = (-300) as u32`, `len = packed as u32 = 0`. `is_error = (len==0) && (ptr as i32) < 0 = true`. `from_guest_code(ptr as i32) = from_guest_code(-300) = NotFound`.

- [ ] **Write the failing test.** Add to `tests/serve_flow.rs`:
```rust
use digstore_core::abi::ErrorCode;
use digstore_host::HostError;

#[test]
fn serve_content_propagates_guest_error_sentinel() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/serve_err.wat")).unwrap();
    let mut rt = HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(100))).unwrap();
    let err = rt.serve_content(b"anything").unwrap_err();
    assert!(
        matches!(err, HostError::GuestError(ErrorCode::NotFound)),
        "expected GuestError(NotFound), got {err:?}"
    );
}

#[test]
fn serve_proof_propagates_guest_error_sentinel() {
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/serve_err.wat")).unwrap();
    let mut rt = HostRuntime::new(&module_bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(100))).unwrap();
    let err = rt.serve_proof(b"anything").unwrap_err();
    assert!(
        matches!(err, HostError::GuestError(ErrorCode::NotFound)),
        "expected GuestError(NotFound), got {err:?}"
    );
}
```

- [ ] **Run it (expect FAIL — confirm the fixture is wired, not a stale pass).** `cargo test -p digstore-host --test serve_flow serve_content_propagates`. Because `unpack_and_read` already maps sentinels (added in Task 10), this test could pass immediately. To make this a genuine RED that proves the new `serve_err.wat` fixture and `from_guest_code` casting path are exercised, FIRST run with a deliberately-wrong expectation by temporarily asserting `HostError::GuestError(ErrorCode::GeneralError)` instead of `NotFound`, confirm FAIL (`expected GuestError(GeneralError), got GuestError(NotFound)`), then restore the assertion to `NotFound`.

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test serve_flow serve_content_propagates serve_proof_propagates`. Expected: both `ok`. If they fail, confirm `unpack_ptr_len`/`is_error` in digstore-core match the canonical definitions and that `from_guest_code` casts `ptr as i32`.

- [ ] **Commit.** `git add crates/digstore-host/tests/fixtures/wat/serve_err.wat crates/digstore-host/tests/serve_flow.rs` then `git commit -m "test(host): serve flow propagates guest error sentinels"`.

---

## Task 18 — Clock-injection determinism through the full stack (§16 support)

§16 relies on `host_get_current_time` being injectable and deterministic; advancing a `FixedClock` between calls must change what the guest sees.

**Files:**
- Modify `crates/digstore-host/tests/imports_unit.rs`

Steps:

- [ ] **Write the failing test.** Add to `tests/imports_unit.rs`:
```rust
#[test]
fn clock_advance_is_observed_by_guest() {
    let clock = FixedClock::new(1_000);
    let module_bytes = wat::parse_str(include_str!("fixtures/wat/import_probe.wat")).unwrap();
    let mut rt = HostRuntime::new(
        &module_bytes,
        cfg(),
        ExecutionLimits::default(),
        test_deps(clock.clone()),
    )
    .unwrap();
    assert_eq!(rt.call_i64_export("probe_time").unwrap(), 1_000);
    clock.advance(500);
    assert_eq!(rt.call_i64_export("probe_time").unwrap(), 1_500);
}
```

- [ ] **Run it (expect FAIL — force a true red).** Because `test_deps` already wraps the passed `FixedClock` (clones share one `Arc<AtomicU64>`), this would pass immediately. To force a genuine RED that proves the shared-handle wiring, FIRST temporarily change the test to pass `test_deps(FixedClock::new(1_000))` (a SEPARATE clock, not `clock.clone()`); the second assertion then reads `1_000` instead of `1_500`. Run `cargo test -p digstore-host --test imports_unit clock_advance`, confirm FAIL (`left: 1000, right: 1500`), then restore `test_deps(clock.clone())`.

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test imports_unit clock_advance`. Expected: `clock_advance_is_observed_by_guest ... ok`. (No production code changes; this proves the Task 2 + Task 11a injection path is deterministic.)

- [ ] **Commit.** `git add crates/digstore-host/tests/imports_unit.rs` then `git commit -m "test(host): clock injection is deterministic and observed by the guest"`.

---

## Task 19 — Remaining data-export wrappers (§18.4 ABI surface)

The guest exports several no-arg data-returning functions beyond `get_store_id`/`get_current_roothash`. Wire the rest so the runtime exposes the full ABI surface.

**Files:**
- Modify `crates/digstore-host/src/runtime.rs`
- Modify `crates/digstore-host/tests/imports_unit.rs`

Steps:

- [ ] **Write the failing test.** Add to `tests/imports_unit.rs` (the import_probe fixture does NOT export these, so calling them returns `MissingExport`, which is the assertion):
```rust
use digstore_host::HostError;

#[test]
fn missing_data_exports_report_missing_export() {
    let mut rt = probe_runtime(FixedClock::new(100));
    // import_probe.wat exports none of these; the wrappers must compile and
    // surface MissingExport rather than panicking.
    assert!(matches!(rt.get_public_key().unwrap_err(), HostError::MissingExport(_)));
    assert!(matches!(rt.get_roothash_history().unwrap_err(), HostError::MissingExport(_)));
    assert!(matches!(rt.get_metadata().unwrap_err(), HostError::MissingExport(_)));
    assert!(matches!(rt.get_authentication_info().unwrap_err(), HostError::MissingExport(_)));
}
```

- [ ] **Run it (expect FAIL).** `cargo test -p digstore-host --test imports_unit missing_data_exports`. Expected: `no method named get_public_key found for struct HostRuntime`.

- [ ] **Add the remaining data-export wrappers.** Append to an `impl HostRuntime` block in `runtime.rs`:
```rust
impl HostRuntime {
    pub fn get_public_key(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_public_key")
    }
    pub fn get_roothash_history(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_roothash_history")
    }
    pub fn get_metadata(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_metadata")
    }
    pub fn get_authentication_info(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_authentication_info")
    }
}
```

- [ ] **Run it (expect PASS).** `cargo test -p digstore-host --test imports_unit missing_data_exports`. Expected: `missing_data_exports_report_missing_export ... ok`.

- [ ] **Commit.** `git add crates/digstore-host/src/runtime.rs crates/digstore-host/tests/imports_unit.rs` then `git commit -m "feat(host): expose remaining data-export wrappers"`.

---

## Task 20 — End-to-end serve against the real `digstore-guest` template (gated)

§18.4 integration: instantiate the actually-compiled guest template and drive `get_store_id`, `get_current_roothash`, `get_public_key`, and `serve_content`, proving the real ABI surface works through the host. These tests are honestly DEFERRED: they are `#[ignore]`-marked so the default suite does not claim a green cycle for behavior that needs an external fixture, and they panic loudly if `DIGSTORE_E2E=1` is set but the fixture is missing.

**Files:**
- Create `crates/digstore-host/tests/e2e_guest.rs`
- Create `crates/digstore-host/tests/fixtures/build_fixture.md`

Steps:

- [ ] **Document the fixture build.** Create `crates/digstore-host/tests/fixtures/build_fixture.md` describing: `cargo build -p digstore-guest --target wasm32-unknown-unknown --release`, then run `digstore-compiler` over a tiny seeded store (store_id all-`0x00`, one resource `hello.txt` with known plaintext) to write `crates/digstore-host/tests/fixtures/sample.wasm` and the matching content request to `crates/digstore-host/tests/fixtures/hello_request.bin`. Record the expected `store_id`, `roothash`, and retrieval-key request bytes in this file. Note: run these gated tests with `cargo test -p digstore-host --test e2e_guest -- --ignored` once the fixture exists.

- [ ] **Write the gated test.** Create `crates/digstore-host/tests/e2e_guest.rs`:
```rust
use digstore_core::config::HostImportsConfig;
use digstore_host::{ExecutionLimits, FixedClock, HostRuntime};
use std::path::Path;

mod common;
use common::test_deps;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.wasm");

fn cfg() -> HostImportsConfig {
    HostImportsConfig {
        return_buffer_capacity: 64 * 1024,
        max_return_buffer_size: 16 * 1024 * 1024,
        max_random_bytes: 1024,
        host_version: "dig-host-test/0.1".to_string(),
    }
}

/// Load the real guest fixture. If the fixture is missing AND the operator
/// explicitly opted in via DIGSTORE_E2E=1, panic loudly (the build step was
/// skipped). Otherwise (manual `--ignored` run without the fixture) panic with
/// a clear message — these tests are `#[ignore]` so they never run in the
/// default suite and never produce a false green.
fn load() -> HostRuntime {
    if !Path::new(FIXTURE).exists() {
        panic!(
            "e2e fixture missing: {FIXTURE} not built. See tests/fixtures/build_fixture.md \
             (cargo build -p digstore-guest --target wasm32-unknown-unknown --release, \
             then run digstore-compiler)."
        );
    }
    let bytes = std::fs::read(FIXTURE).unwrap();
    HostRuntime::new(&bytes, cfg(), ExecutionLimits::default(), test_deps(FixedClock::new(1_700_000_000))).unwrap()
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_get_store_id_is_32_bytes() {
    let mut rt = load();
    assert_eq!(rt.get_store_id().unwrap().len(), 32);
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_current_roothash_is_32_bytes() {
    let mut rt = load();
    assert_eq!(rt.get_current_roothash().unwrap().len(), 32);
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_public_key_is_48_bytes() {
    let mut rt = load();
    assert_eq!(rt.get_public_key().unwrap().len(), 48);
}

#[test]
#[ignore = "requires sample.wasm; run with --ignored after building the guest fixture"]
fn guest_serve_content_returns_response() {
    let mut rt = load();
    let req = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hello_request.bin")).unwrap();
    let out = rt.serve_content(&req).unwrap();
    assert!(!out.is_empty(), "content response must be non-empty (real or decoy)");
}
```

- [ ] **Run it (expect default skip).** `cargo test -p digstore-host --test e2e_guest`. Expected: all four tests report `ignored` (e.g. `test result: ok. 0 passed; 0 failed; 4 ignored`). This is honest: the default suite makes no assertion about behavior that requires the external fixture.

- [ ] **Document the post-fixture run.** Once `digstore-guest`/`digstore-compiler` land and `sample.wasm`/`hello_request.bin` are generated, run `cargo test -p digstore-host --test e2e_guest -- --ignored` and expect all four assertions to pass. If the fixture is absent at that point, `load()` panics with the build instructions rather than silently passing.

- [ ] **Commit.** `git add crates/digstore-host/tests/e2e_guest.rs crates/digstore-host/tests/fixtures/build_fixture.md` then `git commit -m "test(host): gated end-to-end serve against real guest fixture"`.

---

## Task 21 — Full-suite green + clippy gate

**Files:**
- None (verification + any small fixups)

Steps:

- [ ] **Run the entire crate test suite.** `cargo test -p digstore-host`. Expected: every unit module (clock/config/error/random/session/state/memory/teehook) plus integration suites (instantiate/bounds/imports_unit/jwks_mock/serve_flow/return_buffer) report `ok`, and `e2e_guest` reports `4 ignored`. Capture the final `test result: ok. N passed; 0 failed` lines.

- [ ] **Run clippy with denied warnings.** `cargo clippy -p digstore-host --all-targets -- -D warnings`. Expected: `Finished` with no warnings. Fix any lints (unused imports from stub phases, needless `.clone()`, `match` that should be `matches!`).

- [ ] **Run a release build.** `cargo build -p digstore-host --release`. Expected: `Finished release [optimized]`.

- [ ] **Commit any fixups.** `git add -A crates/digstore-host` then `git commit -m "chore(host): clippy clean and full-suite green"`.

---

## Definition of Done

| Paper section | Covered by | Verified by |
|---------------|-----------|-------------|
| **6.3** dig_host imports (all eight) + session-gated jwks_fetch | Tasks 11a–11e, 14 | `tests/imports_unit.rs`, `tests/jwks_mock.rs` |
| **6.4** shared return buffer (64 KiB default, 16 MiB max, grow-on-demand, `host_read_return_buffer`) | Tasks 8, 11b, 15 | `src/state.rs` units, `tests/return_buffer.rs` |
| **18.1** engine setup, `Module::validate` + instantiate, required-export wiring, `init()` | Task 10 | `tests/instantiate.rs` |
| **18.2** execution bounds: wall-clock timeout (epoch), fuel metering, outer memory ceiling (StoreLimits); each RED-before-GREEN | Tasks 12a, 12b, 13 | `tests/bounds.rs` (timeout, fuel, memory; positive + negative controls) |
| **18.3** import dispatch / state threading (`Linker<RuntimeState>`, per-call `HostState`) | Tasks 8, 10, 11a–11e | `tests/imports_unit.rs` |
| **18.4** serve flow: alloc → write → call → is_error → unpack → read → dealloc; never decrypts; `get_content` + `get_proof`; full data-export ABI surface | Tasks 16, 17, 19, 20 | `tests/serve_flow.rs`, `tests/imports_unit.rs`, `tests/e2e_guest.rs` (gated) |
| **12 (host-side imports)** host pubkey, attestation signing, session establish/verify, session-gated jwks_fetch, injectable clock, capped CSPRNG | Tasks 2, 5, 6, 11a, 11c, 11d, 11e, 14, 18 | `tests/imports_unit.rs`, `tests/jwks_mock.rs`, unit tests |
| **13.6** TEE-alternative attestation hook (swappable `AttestationBackend`, default BLS, defined before `HostState`) | Tasks 7, 11d | `src/teehook.rs` unit + `tests/imports_unit.rs` attestation |

The crate is **done** when `cargo test -p digstore-host` is fully green (with `e2e_guest` reporting `ignored`), `cargo clippy -p digstore-host --all-targets -- -D warnings` is clean, `cargo build -p digstore-host --release` succeeds, and the `e2e_guest` suite passes its real assertions when run with `-- --ignored` after the `digstore-guest` fixture module is built.

**Documented deviations surfaced in this crate:**
- **§18.2 (blocking host I/O):** epoch interruption only bounds WASM execution, not blocking host calls; `jwks_fetch` is bounded by its own reqwest request timeout derived from `ExecutionLimits.timeout` (via `HostState::http_timeout_secs`), independent of the export wall-clock bound. Stated in Task 14.
- **§18.2 (per-call bounds):** the serve flow arms fuel + epoch bounds before each of alloc/serve/dealloc, so each export sub-call is independently bounded rather than sharing one combined budget. Stated in Task 16.
- **Deviation #3 (risc0 proof scope):** this crate never re-implements proving; it carries the injected `Prover` handle through `HostState` (from `HostDeps`) for the proof exports to use.

---

## Plan metadata

- **Crate:** digstore-host
- **Assigned paper sections:** 6.3,6.4,18.1,18.2,18.3,18.4,12(host-side imports),13.6(TEE alt hook)
- **Depends on:** digstore-core, digstore-crypto, digstore-prover
- **Spec sections covered (claimed):** 6.3, 6.4, 12, 13.6, 18.1, 18.2, 18.3, 18.4

### Public items exported (consumed by other crates)

```
pub struct HostRuntime
impl HostRuntime { pub fn new(module_bytes: &[u8], config: digstore_core::config::HostImportsConfig, limits: ExecutionLimits, deps: HostDeps) -> Result<Self, HostError> }
impl HostRuntime { pub fn get_store_id(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn get_current_roothash(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn get_roothash_history(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn get_public_key(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn get_metadata(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn get_authentication_info(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn serve_content(&mut self, request: &[u8]) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn serve_proof(&mut self, request: &[u8]) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn read_guest(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn write_guest(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), HostError> }
impl HostRuntime { pub fn read_return_buffer_copy(&mut self) -> Result<Vec<u8>, HostError> }
impl HostRuntime { pub fn call_i64_export(&mut self, name: &str) -> Result<i64, HostError> }
impl HostRuntime { pub fn call_i64_export_1(&mut self, name: &str, arg: i32) -> Result<i64, HostError> }
impl HostRuntime { pub fn call_i32_export(&mut self, name: &str) -> Result<i32, HostError> }
impl HostRuntime { pub fn call_i32_export_1(&mut self, name: &str, arg: i32) -> Result<i32, HostError> }
impl HostRuntime { pub fn call_i32_export_2(&mut self, name: &str, a: i32, b: i32) -> Result<i32, HostError> }
pub struct HostDeps { pub store_id: digstore_core::types::Bytes32, pub bls_secret: digstore_crypto::bls::BlsSecretKey, pub bls_public: digstore_core::types::Bytes48, pub clock: std::sync::Arc<dyn Clock>, pub chain: std::sync::Arc<dyn digstore_prover::ChainSource>, pub prover: std::sync::Arc<dyn digstore_prover::Prover>, pub rng_seed: Option<[u8;32]>, pub instance_id: digstore_core::types::Bytes32, pub attestation: Option<SharedBackend> }
pub struct RuntimeState { pub host: HostState, pub limits: wasmtime::StoreLimits }
pub struct HostState { pub store_id: digstore_core::types::Bytes32, pub config: digstore_core::config::HostImportsConfig, pub return_buffer: ReturnBuffer, pub keys: std::sync::Arc<HostKeys>, pub attestation: SharedBackend, pub clock: std::sync::Arc<dyn Clock>, pub sessions: SessionTable, pub chain: std::sync::Arc<dyn digstore_prover::ChainSource>, pub prover: std::sync::Arc<dyn digstore_prover::Prover>, pub rng: HostRng, pub instance_id: digstore_core::types::Bytes32, pub http_timeout_secs: u64, pub last_signature: Option<digstore_core::types::Bytes96> }
pub struct HostKeys { pub bls_secret: digstore_crypto::bls::BlsSecretKey, pub bls_public: digstore_core::types::Bytes48 }
pub struct ReturnBuffer
impl ReturnBuffer { pub fn new(cfg: &digstore_core::config::HostImportsConfig) -> Self }
impl ReturnBuffer { pub fn set(&mut self, data: &[u8]) -> Result<usize, HostError> }
impl ReturnBuffer { pub fn as_slice(&self) -> &[u8] }
pub enum HostError { Wasmtime(String), Validation(String), GuestError(digstore_core::abi::ErrorCode), Timeout, OutOfFuel, MemoryLimit, MissingExport(&'static str), OutOfBounds, ReturnBufferOverflow { needed: usize, max: usize }, Http(String) }
impl HostError { pub fn from_guest_code(code: i32) -> Self }
pub struct ExecutionLimits { pub timeout: std::time::Duration, pub memory_bytes_max: usize, pub fuel: u64 }
impl Default for ExecutionLimits
impl ExecutionLimits { pub fn memory_pages_max(&self) -> usize }
pub const WASM_PAGE_SIZE: usize = 64 * 1024
pub const MAX_MEMORY_BYTES: usize = 256 * 64 * 1024
pub trait Clock: Send + Sync + 'static { fn now_unix_secs(&self) -> u64; }
pub struct SystemClock; impl Clock for SystemClock
pub struct FixedClock(std::sync::Arc<std::sync::atomic::AtomicU64>)
impl FixedClock { pub fn new(secs: u64) -> Self; pub fn advance(&self, secs: u64); pub fn set(&self, secs: u64) }
impl Clock for FixedClock
pub struct Session { pub nonce: [u8;32], pub store_id: [u8;32], pub established_at: u64, pub expires_at: u64 }
impl Session { pub fn is_valid_at(&self, now: u64) -> bool }
pub struct SessionTable
impl SessionTable { pub fn new() -> Self; pub fn establish(&mut self, nonce: [u8;32], store_id: [u8;32], now: u64, ttl_secs: u64); pub fn is_valid(&self, now: u64) -> bool; pub fn active_store_id(&self, now: u64) -> Option<[u8;32]>; pub fn clear(&mut self) }
pub struct HostRng
impl HostRng { pub fn from_entropy() -> Self; pub fn from_seed(seed: [u8;32]) -> Self; pub fn fill(&mut self, count: usize, max: usize) -> Option<Vec<u8>> }
pub trait AttestationBackend: Send + Sync + 'static { fn attest(&self, challenge: &[u8]) -> Result<digstore_core::types::Bytes96, HostError>; fn public_key(&self) -> digstore_core::types::Bytes48; }
pub type SharedBackend = std::sync::Arc<dyn AttestationBackend>
pub struct BlsAttestationBackend
impl BlsAttestationBackend { pub fn new(secret: digstore_crypto::bls::BlsSecretKey, public: digstore_core::types::Bytes48) -> Self }
impl AttestationBackend for BlsAttestationBackend
```