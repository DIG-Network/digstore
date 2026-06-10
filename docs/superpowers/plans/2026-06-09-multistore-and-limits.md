# Multi-Store Workspaces, Per-Store Cap, Memory Ceiling — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `.dig/` into a multi-store workspace with clean store-switching UX, a per-store 100 MB stage cap with always-visible capacity, a configurable per-store content root for build output, staging management, a commit-time URN manifest + URN preview, and a raised (256→2048 page / 128 MiB) module memory ceiling with a dynamic guest heap so stores up to the cap actually commit and serve.

**Architecture:** All workspace/multi-store/content-root/cap/manifest logic lives in `digstore-cli` (the CLI already owns the real commit + client crypto per its `store_ops.rs` architecture comment, so the store/core crates need **no** changes). The only cross-crate work is the memory-ceiling raise in `digstore-compiler`, `digstore-guest`, and `digstore-host`. `content_root` is stored in a new CLI-owned `workspace.toml`, **not** in `StoreConfig`. `MAX_STORE_BYTES` is a CLI constant.

**Tech Stack:** Rust (workspace, edition 2021, pinned toolchain 1.94.1), clap 4 (derive), `ignore`/`globset` (file walking), `anstream`/`anstyle` (color), `serde`/`toml`/`serde_json`, `assert_cmd` (CLI integration tests), `wat`/`wasm-encoder`/`wasmparser` (compiler), `wasmtime` (host). Guest is `no_std` wasm32 with a custom `#[global_allocator]`.

---

## Shared design decisions (read once, applies to every task)

- **Crate scope:** `digstore-cli`, `digstore-compiler`, `digstore-guest`, `digstore-host`, docs. Do **not** edit `digstore-core` or `digstore-store`.
- **`content_root`** is per-store and lives in `workspace.toml` under `[stores.<name>]`, never in `StoreConfig`/`config.toml`.
- **`workspace.toml`** layout (CLI-owned, sits at `.dig/workspace.toml`):
  ```toml
  active = "default"

  [stores.default]
  id = "ab12…64hex…"

  [stores.site]
  id = "cd34…64hex…"
  content_root = "dist"
  ```
- **`CliContext` final shape** (`crates/digstore-cli/src/context.rs`):
  ```rust
  #[derive(Debug, Clone)]
  pub struct CliContext {
      /// Per-store directory: `.dig/stores/<name>/` (workspace dir for workspace-only cmds).
      pub dig_dir: PathBuf,
      /// The `.dig/` workspace directory (skip target for walks; home of workspace.toml).
      pub workspace_dir: PathBuf,
      /// Resolved operating directory for `add`/`urn`/`status` scans (§2.8).
      pub op_dir: PathBuf,
      /// Selected store name, when a store is resolved (None for workspace-only cmds).
      pub store_name: Option<String>,
      pub json: bool,
      pub verbose: bool,
  }
  ```
  All existing helpers (`config_path`, `load_config`, `staging_path`, `modules_dir`, `generations_dir`, `history_path`, `salt_path`, `find_store_id`) keep operating on `dig_dir` unchanged.
- **Operating-directory precedence (§2.8):** `-C/--cwd` flag → store `content_root` (joined to project root) → CWD. `project_root = workspace_dir.parent()`.
- **`MAX_STORE_BYTES = 128_000_000`** (128 MB, decimal) — the plaintext stage cap. **Lives in `digstore-core`** (e.g. `digstore-core/src/config.rs`) as the single source of truth, because the compiler needs it as the **filler budget** and the host/CLI need it for limits. The CLI references `digstore_core::MAX_STORE_BYTES` (reused by `init_store`, the `add` cap check, the `commit` defensive check, `remote_ops::clone`). This supersedes the original plan's "no core edits / CLI-local 100 MB" decision — the uniform-filler model (below) ties the cap to the compiler, so a shared constant is required.
- **Uniform module size (filler model):** the compiler pads every module's injected data blob to a **fixed budget** so all stores compile to the **same module size** regardless of content; a store at 100% of the cap needs ~no filler. This replaces the old `next_pow2(content)`-**append** logic in `pipeline.rs::next_filler_bucket` (which roughly doubled the module). Filler = `max(0, FIXED_BLOB_LEN − actual_blob_len_without_filler)`. `FIXED_BLOB_LEN` covers a max-cap store's ciphertext + key-table + merkle + headers (≈ 128 MiB; choose a page-aligned value ≥ the worst case at the cap).
- **Memory ceiling:** `MAX_MEMORY_PAGES = 6144` (**384 MiB**) — sized to hold the fixed ~128 MiB data region **plus** a serve-time heap allocation for a worst-case single max-size resource (`digstore_core::serving::concat_output` copies the resource's chunk ciphertexts into a fresh `Vec`, up to ~122 MiB) **plus** response framing headroom. Derived from `MAX_STORE_BYTES` in `digstore-core` and consumed by compiler `template::MAX_MEMORY_PAGES` (+ `inject.rs`, which imports it — delete its private `CEILING_PAGES`) and host `MAX_MEMORY_BYTES = MAX_MEMORY_PAGES * WASM_PAGE_SIZE`. Validated end-to-end by a near-cap full-store serve stress test (A5); trim toward 4096 only if the test shows the headroom is unused.
- **The golden fixture `golden_data_section.hex` IS regenerated in Task A5** (the filler bytes/length change with the uniform-filler model). It is NOT touched by A1–A4 (the ceiling raise alone does not change the data-section blob).
- **Guest WASM rebuild:** after any `digstore-guest` change, rebuild `target/wasm32-unknown-unknown/release/digstore_guest.wasm` (`cargo build -p digstore-guest --target wasm32-unknown-unknown --release`) before running `digstore-compiler`/`digstore-cli` tests that embed it.
- **Commit discipline:** every task ends with a commit. Branch is `feat/digstore-multistore` (already created). End commit messages with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- **Disk note:** this machine hits disk-full LNK errors when `target/` balloons. If a build fails with LNK1318/os error 112, it is disk, not code — clear `target/debug/incremental`.

---

## File Structure

**Created:**
- `crates/digstore-cli/src/workspace.rs` — `WorkspaceToml`/`StoreEntryToml` serde, `Workspace` runtime type (load/save/discover/create/migrate/resolve/register/set_active/set_content_root).
- `crates/digstore-cli/src/commands/stores.rs` — `digstore stores`.
- `crates/digstore-cli/src/commands/use_store.rs` — `digstore use <name>` (file is `use_store.rs`; `use` is a keyword).
- `crates/digstore-cli/src/commands/dir.rs` — `digstore dir [<path>]`.
- `crates/digstore-cli/src/commands/unstage.rs` — `digstore unstage`.
- `crates/digstore-cli/src/commands/staged.rs` — `digstore staged`.
- `crates/digstore-cli/src/commands/urn.rs` — `digstore urn [PATHS]`.
- `crates/digstore-cli/tests/cli_multistore.rs` — assert_cmd integration tests for the new UX.
- `crates/digstore-compiler/tests/large_data_section.rs` — >8 MiB self-serve test (mechanism) + near-cap `#[ignore]` stress test.

**Modified:**
- `digstore-host/src/config.rs` (constant + 3 tests), `tests/bounds.rs` (comment).
- `digstore-compiler/src/template.rs` (const + comments + 3 unit tests), `src/inject.rs` (use shared const), `fixtures/digstore_guest_template.wat` (`1 2048`), `tests/inject.rs` (2 asserts).
- `digstore-guest/src/allocator.rs` (dynamic heap + tests).
- `digstore-cli/src/context.rs`, `src/cli.rs`, `src/commands/mod.rs`, `src/main.rs`, `src/lib.rs`, `src/ops/store_ops.rs`, `src/ops/walk.rs`, `src/ops/remote_ops.rs`, `src/ui/mod.rs`, `src/output.rs`, `src/commands/init.rs`, `src/commands/add.rs`, `src/commands/status.rs`, `src/commands/commit.rs`, `tests/common/mod.rs`.
- `docs/whitepaper/digstore-whitepaper.md` + re-render; `SECURITY.md`.

---

## Phase A — Memory ceiling (compiler + guest + host)

> Independent of the CLI work. Do this phase first: it is what originally broke `commit` ("data section needs 1981 pages but §5.1 memory ceiling is 256"). Each task is TDD against existing tests that hard-code 256.

### Task A1: Host memory ceiling 256 → 2048 pages

**Files:**
- Modify: `crates/digstore-host/src/config.rs:9` (const) and its `#[cfg(test)] mod tests`
- Modify: `crates/digstore-host/tests/bounds.rs:84-97` (comment only)

- [ ] **Step 1: Update the three failing unit tests first (they encode the old ceiling).** In `crates/digstore-host/src/config.rs` tests:

```rust
    #[test]
    fn defaults_match_spec() {
        let l = ExecutionLimits::default();
        assert_eq!(l.memory_bytes_max, 128 * 1024 * 1024);
        assert_eq!(l.timeout, Duration::from_secs(5));
        assert!(l.fuel >= 1_000_000_000);
    }

    #[test]
    fn pages_helper_matches_bytes() {
        let l = ExecutionLimits::default();
        assert_eq!(l.memory_pages_max(), 2048);
    }

    #[test]
    fn consts_match_spec() {
        assert_eq!(WASM_PAGE_SIZE, 64 * 1024);
        assert_eq!(MAX_MEMORY_BYTES, 128 * 1024 * 1024);
    }
```

- [ ] **Step 2: Run them to confirm they fail.** Run: `cargo test -p digstore-host --lib config`. Expected: FAIL (`memory_bytes_max` is 16 MiB, `256`).

- [ ] **Step 3: Raise the constant.** `crates/digstore-host/src/config.rs:9`:

```rust
/// Hard ceiling matching the guest's declared max (2048 pages = 128 MiB, §18.2).
pub const MAX_MEMORY_BYTES: usize = 2048 * WASM_PAGE_SIZE;
```

- [ ] **Step 4: Fix the stale comment in `tests/bounds.rs:86`** (`memory_ceiling_allows_within_limit`): change `// 256 pages, room for +200` to `// 2048 pages, room for +200`. (The grow.wat grows 200 pages from 1 → 201 ≤ 2048; assertion stays green.)

- [ ] **Step 5: Run host tests.** Run: `cargo test -p digstore-host`. Expected: PASS.

- [ ] **Step 6: Commit.**
```bash
git add crates/digstore-host
git commit -m "feat(host): raise module memory ceiling 256→2048 pages (128 MiB)"
```

### Task A2: Compiler memory ceiling 256 → 2048; unify the constant

**Files:**
- Modify: `crates/digstore-compiler/src/template.rs:6` (const), `:175-186` (`assert_memory_ceiling` strings), `:147-158` (`load_template` comment), unit tests at `:280-297` and `:341-359`
- Modify: `crates/digstore-compiler/src/inject.rs:210` (delete `CEILING_PAGES`), `:57-79` (use `template::MAX_MEMORY_PAGES`)
- Modify: `crates/digstore-compiler/fixtures/digstore_guest_template.wat:13`
- Modify: `crates/digstore-compiler/tests/inject.rs:247-272` (two `Some(256)`/WAT updates)

- [ ] **Step 1: Update the unit tests in `template.rs` first.** Replace the three tests' literals:

`emitted_module_with_memory_max_exactly_ceiling_passes` (`:291-297`):
```rust
    #[test]
    fn emitted_module_with_memory_max_exactly_ceiling_passes() {
        let bytes = full_abi_module(r#"(memory (export "memory") 1 2048)"#);
        assert_memory_ceiling(&bytes).expect("2048 is the ceiling");
        let t = load_template(baked_template_bytes()).expect("baked template valid");
        assert_eq!(t.memory_max_pages, Some(MAX_MEMORY_PAGES));
    }
```

`emitted_module_with_memory_max_under_ceiling_fails_ceiling_check` (`:280-288`) — comment fix only:
```rust
        // §5.1: the module-declared cap is EXACTLY 2048 pages (128 MiB).
        let bytes = full_abi_module(r#"(memory (export "memory") 1 128)"#);
```

`template_with_memory_max_over_ceiling_is_rejected` (`:341-359`) — bump the over-ceiling value:
```rust
        // Full ABI but max pages 2049 (> 2048) -> rejected.
        let watsrc = r#"(module
          (memory (export "memory") 1 2049)
          ...
          )"#;
```
(Keep the rest of that WAT block byte-for-byte; only `257`→`2049` and the comment change.)

- [ ] **Step 2: Update the two integration tests in `tests/inject.rs`.**
`inject_normalizes_unbounded_memory_to_ceiling` (`:247-261`):
```rust
        assert_eq!(max, Some(2048), "§5.1 emitted module must declare maximum 2048");
```
`inject_preserves_ceiling_memory_max` (`:265-272`):
```rust
    let watsrc = r#"(module (memory (export "memory") 4 2048))"#;
    let template = wat::parse_str(watsrc).unwrap();
    let blob = b"DIGS";
    let out = inject_data_section(&template, blob, 0x10).expect("inject ok");
    let (_, max) = memory_limits(&out);
    assert_eq!(max, Some(2048));
```

- [ ] **Step 3: Run to confirm failures.** Run: `cargo test -p digstore-compiler template:: ` and `cargo test -p digstore-compiler --test inject`. Expected: FAIL (ceiling still 256; baked template still `1 256`).

- [ ] **Step 4: Raise `MAX_MEMORY_PAGES` and fix human labels** in `template.rs:6` and `assert_memory_ceiling` (`:175-186`):
```rust
/// Maximum linear-memory pages the served module may declare (§5.1: 128 MiB ceiling).
pub const MAX_MEMORY_PAGES: u64 = 2048;
```
```rust
pub fn assert_memory_ceiling(module: &[u8]) -> Result<()> {
    let t = load_template(module)?;
    match t.memory_max_pages {
        Some(max) if max == MAX_MEMORY_PAGES => Ok(()),
        Some(max) => Err(CompilerError::Validation(format!(
            "emitted module memory max {max} pages must equal §5.1 ceiling {MAX_MEMORY_PAGES} (128 MiB)"
        ))),
        None => Err(CompilerError::Validation(format!(
            "emitted module must declare memory maximum {MAX_MEMORY_PAGES} pages (§5.1 128 MiB ceiling)"
        ))),
    }
}
```
Update the `load_template` comment block (`:147-158`) `Some(256)`/`16 MiB` → `Some(2048)`/`128 MiB`.

- [ ] **Step 5: Unify the inject.rs constant.** In `crates/digstore-compiler/src/inject.rs` delete the private `const CEILING_PAGES: u64 = 256;` (`:210`) and replace its two uses (`:63` guard, `:69` `Some(CEILING_PAGES)`) with the shared `crate::template::MAX_MEMORY_PAGES`. Update the inline comment to `128 MiB = 2048 pages`. Result (the `MemorySection` arm):
```rust
            Payload::MemorySection(reader) => {
                let mut mem = MemorySection::new();
                for m in reader {
                    let m = m.map_err(|e| CompilerError::InvalidTemplate(e.to_string()))?;
                    let min = m.initial.max(needed_pages);
                    // §5.1: the emitted module always declares the 128 MiB ceiling
                    // (`maximum: Some(2048)`) regardless of the template's declared max.
                    if needed_pages > crate::template::MAX_MEMORY_PAGES {
                        return Err(CompilerError::Validation(format!(
                            "data section needs {needed_pages} pages but §5.1 memory ceiling is {}",
                            crate::template::MAX_MEMORY_PAGES
                        )));
                    }
                    mem.memory(MemoryType {
                        minimum: min,
                        maximum: Some(crate::template::MAX_MEMORY_PAGES),
                        memory64: m.memory64,
                        shared: m.shared,
                        page_size_log2: None,
                    });
                }
                module.section(&mem);
            }
```
(Confirm `template` is reachable from `inject.rs`; both are modules of the same crate. If `MAX_MEMORY_PAGES` is not already `pub` at crate level, it is `pub` in `template.rs` so `crate::template::MAX_MEMORY_PAGES` resolves.)

- [ ] **Step 6: Update the baked template** `crates/digstore-compiler/fixtures/digstore_guest_template.wat:13`:
```wat
  (memory (export "memory") 1 2048)
```

- [ ] **Step 7: Rebuild and run compiler tests.** Run: `cargo test -p digstore-compiler`. Expected: PASS (build.rs re-bakes the template; `assert_memory_ceiling` now matches 2048). If `self_serving.rs` / `data_section_golden.rs` need the guest wasm, they are unaffected by this task (golden unchanged).

- [ ] **Step 8: Commit.**
```bash
git add crates/digstore-compiler
git commit -m "feat(compiler): raise memory ceiling to 2048 pages; unify CEILING_PAGES into template::MAX_MEMORY_PAGES"
```

### Task A3: Guest dynamic heap above the data section

**Files:**
- Modify: `crates/digstore-guest/src/allocator.rs` (HEAP_BASE/HEAP_END, `BumpAllocator::new`, `bump`, tests)

**Design:** `next == 0` is the "uninitialized" sentinel. On wasm32, first `bump` reads the `DIGS` header at `DIGS_DATA_OFFSET` (mirroring `datasection::embedded`), computes `heap_base = align_up(DIGS_DATA_OFFSET + total_blob_len, 64KiB)`, and CAS-installs it; absent/invalid magic → fixed 8 MiB fallback. The `HEAP_END` hard cap is removed; OOM is signaled only by `ensure_memory` (i.e. `memory.grow`) failing. On native (test) builds there is no real linear memory, so the init path returns the fixed 8 MiB base without dereferencing memory.

- [ ] **Step 1: Write the new/updated tests first.** Replace the three existing tests in `allocator.rs`'s `mod tests` with:

```rust
    #[test]
    fn fallback_base_is_8mib_without_a_digs_header() {
        // Native build: no DIGS magic in memory, so the fixed fallback applies.
        let a = BumpAllocator::new();
        let p1 = a.bump(Layout::from_size_align(64, 8).unwrap());
        assert!(!p1.is_null());
        assert!((p1 as usize) >= FALLBACK_HEAP_BASE);
        assert!(FALLBACK_HEAP_BASE > digstore_core::datasection::DIGS_DATA_OFFSET as usize);
    }

    #[test]
    fn bump_returns_distinct_aligned_pointers() {
        let a = BumpAllocator::new();
        let p1 = a.bump(Layout::from_size_align(64, 8).unwrap()) as usize;
        let p2 = a.bump(Layout::from_size_align(64, 8).unwrap()) as usize;
        assert_ne!(p1, p2);
        assert!(p2 >= p1 + 64);
        assert_eq!(p1 % 8, 0);
        assert_eq!(p2 % 8, 0);
    }

    #[test]
    fn heap_base_from_header_sits_above_the_blob() {
        // Unit-test the pure computation: given a total blob length, the base is
        // page-aligned and strictly above DIGS_DATA_OFFSET + total_len.
        let off = digstore_core::datasection::DIGS_DATA_OFFSET as usize;
        let total_len = 10 * 1024 * 1024; // 10 MiB blob (over the old 8 MiB base)
        let base = heap_base_from_total_len(total_len);
        assert!(base >= off + total_len);
        assert_eq!(base % 65536, 0);
    }
```

- [ ] **Step 2: Run to confirm failure.** Run: `cargo test -p digstore-guest --lib`. Expected: FAIL (`FALLBACK_HEAP_BASE`, `heap_base_from_total_len` don't exist; `HEAP_END` removed).

- [ ] **Step 3: Rewrite the allocator.** In `crates/digstore-guest/src/allocator.rs`:

Replace the constants:
```rust
/// Fixed fallback heap base used on native (test) builds and when no `DIGS`
/// header is present in linear memory. Above the data-section window so heap
/// growth can never corrupt an (absent) blob (contract D2). 8 MiB.
pub const FALLBACK_HEAP_BASE: usize = 8 * 1024 * 1024;
```
(Delete `HEAP_BASE` and `HEAP_END`.)

Add a pure helper (available to all targets, used by the test and the wasm init path):
```rust
/// `heap_base = align_up(DIGS_DATA_OFFSET + total_blob_len, 64 KiB)`.
#[inline]
fn heap_base_from_total_len(total_len: usize) -> usize {
    let off = digstore_core::datasection::DIGS_DATA_OFFSET as usize;
    let end = off + total_len;
    let page = 64 * 1024;
    end.div_ceil(page) * page
}
```

`new()` initializes the sentinel:
```rust
    pub const fn new() -> Self {
        BumpAllocator { next: AtomicUsize::new(0) } // 0 = uninitialized
    }
```

Add the wasm32 header reader (mirrors `datasection::embedded`, header-only read):
```rust
    /// Compute the dynamic heap base by reading the injected `DIGS` header at
    /// `DIGS_DATA_OFFSET`. Returns the fallback if magic/version are absent.
    #[cfg(target_arch = "wasm32")]
    fn resolve_heap_base() -> usize {
        use digstore_core::datasection::DIGS_DATA_OFFSET;
        const HEADER_LEN: usize = 9; // magic(4)+version(1)+count(u32 BE)
        const ROW_LEN: usize = 10;   // id(u16)+offset(u32)+len(u32)
        unsafe {
            let base = DIGS_DATA_OFFSET as *const u8;
            let header = core::slice::from_raw_parts(base, HEADER_LEN);
            if &header[0..4] != b"DIGS" || header[4] != 1 {
                return FALLBACK_HEAP_BASE;
            }
            let count = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;
            let table_len = match count.checked_mul(ROW_LEN).and_then(|t| t.checked_add(HEADER_LEN)) {
                Some(n) => n,
                None => return FALLBACK_HEAP_BASE,
            };
            let rows = core::slice::from_raw_parts(base, table_len);
            let mut total_len = table_len;
            for i in 0..count {
                let p = HEADER_LEN + i * ROW_LEN;
                let offset = u32::from_be_bytes([rows[p+2], rows[p+3], rows[p+4], rows[p+5]]) as usize;
                let len = u32::from_be_bytes([rows[p+6], rows[p+7], rows[p+8], rows[p+9]]) as usize;
                match offset.checked_add(len) {
                    Some(end) if end > total_len => total_len = end,
                    Some(_) => {}
                    None => return FALLBACK_HEAP_BASE,
                }
            }
            heap_base_from_total_len(total_len)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_heap_base() -> usize {
        FALLBACK_HEAP_BASE
    }
```

Rewrite `bump` to initialize lazily and drop the `HEAP_END` cap:
```rust
    pub fn bump(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(1);
        let size = layout.size();
        loop {
            let mut cur = self.next.load(Ordering::Relaxed);
            if cur == 0 {
                // First allocation: install the dynamic base. Losers re-read.
                let base = Self::resolve_heap_base();
                match self.next.compare_exchange(0, base, Ordering::SeqCst, Ordering::Relaxed) {
                    Ok(_) => cur = base,
                    Err(observed) => cur = observed,
                }
            }
            let aligned = (cur + align - 1) & !(align - 1);
            let end = match aligned.checked_add(size) {
                Some(e) => e,
                None => return core::ptr::null_mut(),
            };
            if self
                .next
                .compare_exchange(cur, end, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                if !Self::ensure_memory(end) {
                    return core::ptr::null_mut();
                }
                return aligned as *mut u8;
            }
        }
    }
```
Keep `ensure_memory` (wasm32) and the native stub unchanged. Update the module-doc comment that mentions the fixed `1 MiB .. 8 MiB` window to describe the dynamic base. (Also fix the stale `1 MiB` comment in `crates/digstore-compiler/src/pipeline.rs:24` → `2 MiB` while nearby — optional cleanup; if touched, include it in this commit.)

- [ ] **Step 4: Run guest tests.** Run: `cargo test -p digstore-guest`. Expected: PASS.

- [ ] **Step 5: Rebuild the guest wasm** (downstream tests embed it):
```bash
cargo build -p digstore-guest --target wasm32-unknown-unknown --release --locked
```

- [ ] **Step 6: Run the self-serve suites that consume the guest wasm.** Run: `cargo test -p digstore-compiler --test self_serving` and `cargo test -p digstore-cli --test adv_self_serve`. Expected: PASS (these are the D6 self-serve guarantees; the dynamic base must not change served bytes for small blobs).

- [ ] **Step 7: Commit.**
```bash
git add crates/digstore-guest crates/digstore-compiler/src/pipeline.rs
git commit -m "feat(guest): dynamic heap base above the data section; remove fixed 16 MiB cap (D2 for any blob size)"
```

### Task A4: Large-data-section serve test (mechanism + near-cap stress)

**Files:**
- Create: `crates/digstore-compiler/tests/large_data_section.rs`

- [ ] **Step 1: Write a >8 MiB self-serve test** modeled on `self_serving.rs::real_compiled_module_serves_itself_with_verifying_proof`. It compiles a real module whose injected data section exceeds the old 8 MiB heap base (e.g. a single ~12 MiB resource), instantiates `HostRuntime` with `ExecutionLimits::default()` (now 128 MiB), serves the resource, and asserts the proof verifies to the trusted root and the client GCM-decrypts to the original bytes. Reuse the `GUEST_WASM` path and helpers from `self_serving.rs` (copy the harness; do not refactor the existing file).

```rust
// Mirror the harness in tests/self_serving.rs. Build a ~12 MiB resource so the
// injected data section is well above the old 8 MiB fixed heap base.
const GUEST_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-unknown-unknown/release/digstore_guest.wasm"
);

#[test]
fn module_with_data_section_over_8mib_serves_and_verifies() {
    let payload = vec![0xABu8; 12 * 1024 * 1024];
    // ... build store_ops-style generation OR use the compiler harness from
    // self_serving.rs to inject {payload} as one resource, compile, instantiate
    // HostRuntime, serve by retrieval key, assert proof.verify(root) && client
    // GCM-open == payload. (Replicate the exact construction used in
    // self_serving.rs; do not import its private helpers.)
}
```

- [ ] **Step 2: Write a near-cap `#[ignore]` stress test** that pushes close to the 100 MB cap to validate the 128 MiB ceiling has enough headroom end-to-end (this is the real validator of the 2048-page choice; `#[ignore]` because it is slow/heavy):
```rust
#[test]
#[ignore = "stress: ~90 MB resource validates the 128 MiB ceiling headroom; run with --ignored"]
fn near_cap_resource_serves_within_the_128mib_ceiling() {
    let payload = vec![0x5Au8; 90 * 1024 * 1024];
    // Same flow as above. If this OOMs, the single knob to raise is
    // template::MAX_MEMORY_PAGES (and the host MAX_MEMORY_BYTES) — everything
    // else derives from it. Document any failure as a finding.
}
```

- [ ] **Step 3: Run the non-ignored test.** Run: `cargo test -p digstore-compiler --test large_data_section`. Expected: PASS.

- [ ] **Step 4: Run the stress test explicitly to validate headroom.** Run: `cargo test -p digstore-compiler --test large_data_section -- --ignored near_cap`. Expected: PASS. If it OOMs, STOP and report — this is the headroom finding the spec's 128 MiB choice depends on.

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-compiler/tests/large_data_section.rs
git commit -m "test(compiler): serve a >8 MiB data section + near-cap ceiling-headroom stress test"
```

### Task A5: Uniform-size filler + final 384 MiB ceiling + shared cap constant

> Supersedes the interim ceiling (2048) committed in A1–A3. Resolves the headroom finding: the old `next_pow2`-**append** filler made a module ≈ 2–3× its content; a store could not reach the cap within a sane ceiling. New model: **every module is one fixed size** (filler pads small stores up to the budget), and the ceiling is sized to also serve a worst-case single max resource.

**Files:**
- Modify: `crates/digstore-core/src/config.rs` (add `MAX_STORE_BYTES`; optionally `MAX_MODULE_MEMORY_PAGES`)
- Modify: `crates/digstore-compiler/src/pipeline.rs` (`next_filler_bucket` → pad-to-fixed-budget), `src/template.rs` (`MAX_MEMORY_PAGES = 6144`), `src/inject.rs` (uses `template::MAX_MEMORY_PAGES`)
- Modify: `crates/digstore-compiler/fixtures/digstore_guest_template.wat:13` (`1 6144`)
- Modify: `crates/digstore-host/src/config.rs` (`MAX_MEMORY_BYTES = 6144 * WASM_PAGE_SIZE`) + its tests
- Regenerate: `crates/digstore-compiler/tests/fixtures/golden_data_section.hex`
- Modify: ceiling/filler unit + integration tests (`template.rs`, `tests/inject.rs`, `self_serving.rs`, host `config.rs`/`bounds.rs`) to the new constants
- Modify: `crates/digstore-compiler/tests/large_data_section.rs` (near-cap test → a real ~120 MB full store served end-to-end)

- [ ] **Step 1: Add the shared cap constant** in `digstore-core/src/config.rs`:
```rust
/// Per-store hard cap on staged plaintext content (§3). 128 MB, decimal.
/// Single source of truth: the CLI enforces it at stage/commit, the compiler
/// uses it as the uniform-filler budget, the host derives its memory bound.
pub const MAX_STORE_BYTES: u64 = 128_000_000;
```
Re-export from the crate root if the crate uses a prelude/`pub use` pattern (match existing conventions). No other field/struct changes.

- [ ] **Step 2: Replace the filler with pad-to-fixed-budget** in `pipeline.rs`. Delete `next_filler_bucket` (the power-of-two stepper). Compute the blob length WITHOUT filler (headers + key table + chunk pool + merkle), then set `filler_len = FIXED_BLOB_LEN.saturating_sub(blob_len_without_filler)`, where `FIXED_BLOB_LEN` is a page-aligned constant ≥ the worst-case blob for a `MAX_STORE_BYTES` store. Define it explicitly and document the derivation:
```rust
/// Every module's injected data blob is padded to exactly this length so all
/// stores are byte-for-byte the same size (§8.3 size obfuscation: the module
/// size reveals nothing about content size). Must be >= the largest blob a
/// MAX_STORE_BYTES store can produce (ciphertext + key table + merkle + header).
/// 128 MiB comfortably covers a 128 MB (=122.07 MiB) store + metadata headroom.
const FIXED_BLOB_LEN: usize = 128 * 1024 * 1024;
```
If `blob_len_without_filler > FIXED_BLOB_LEN` (should be impossible under the cap), return `CompilerError::Validation("content exceeds the uniform-size budget")` rather than truncating. Filler bytes remain the deterministic ChaCha20 stream (`deterministic_filler`), just a computed length.

- [ ] **Step 3: Raise the ceiling constants.** `template.rs`: `pub const MAX_MEMORY_PAGES: u64 = 6144;` (128 MiB human labels → 384 MiB). `inject.rs`: already uses `template::MAX_MEMORY_PAGES` after A2 — no change beyond the value flowing through. `host/src/config.rs`: `MAX_MEMORY_BYTES = 6144 * WASM_PAGE_SIZE` (and its `defaults_match_spec`/`pages_helper_matches_bytes`/`consts_match_spec` tests → 384 MiB / 6144). Baked template `fixtures/digstore_guest_template.wat:13` → `(memory (export "memory") 1 6144)`. Update every compiler/host test literal that A1–A3 set to 2048 → 6144, and the over-ceiling rejection test → `6145`.

- [ ] **Step 4: Regenerate the golden fixture.** Run the data-section golden test, capture the new hex (the filler length changed), and update `tests/fixtures/golden_data_section.hex`. Confirm `data_section_golden.rs` passes. (Document in the commit that the regen is due to the uniform-filler length, not a format change.)

- [ ] **Step 5: Rewrite the near-cap test** in `large_data_section.rs` to build a real ~120 MB store (just under the 128 MB cap), compile, instantiate `HostRuntime` (default 384 MiB), serve the largest resource, and assert the proof verifies + client GCM-decrypt recovers the bytes. Keep it `#[ignore]` (heavy) but make it the authoritative ceiling validator. Also add/keep a small uniform-size assertion: two stores with very different content sizes compile to **identical module size**.

- [ ] **Step 6: Rebuild guest wasm + run the full compiler/host/guest suites + the self-serve D6 suites.** Run: `cargo build -p digstore-guest --target wasm32-unknown-unknown --release --locked`; `cargo test -p digstore-host`; `cargo test -p digstore-compiler`; `cargo test -p digstore-compiler --test self_serving`; `cargo test -p digstore-cli --test adv_self_serve`; `cargo test -p digstore-compiler --test large_data_section -- --include-ignored`. Expected: PASS. If the ~120 MB serve OOMs at 384 MiB, report it (do not silently raise further — it would indicate `concat_output` double-copies and the ceiling needs ~512 MiB; flag as a finding).

- [ ] **Step 7: Commit.**
```bash
git add crates/digstore-core crates/digstore-compiler crates/digstore-host
git commit -m "feat: uniform-size module filler + 384 MiB ceiling; MAX_STORE_BYTES=128MB in digstore-core"
```

> **Phase E note:** the whitepaper §8.3 must be rewritten from "coarse power-of-two size bucket" to "every module is padded to one fixed size (uniform), revealing nothing about content size"; §5.1/§18.2 memory numbers → 384 MiB. Folded into Task E1.

---

## Phase B — CLI workspace + multi-store core

### Task B1: `Workspace` module (workspace.toml, resolution, migration)

**Files:**
- Create: `crates/digstore-cli/src/workspace.rs`
- Modify: `crates/digstore-cli/src/lib.rs` (add `pub mod workspace;`)

- [ ] **Step 1: Write the unit tests first** (in `workspace.rs` `#[cfg(test)] mod tests`, using `tempfile`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn id(n: u8) -> String { format!("{:064x}", n) }

    #[test]
    fn name_validation_accepts_and_rejects() {
        for ok in ["default", "site", "a.b_c-1"] { assert!(validate_store_name(ok).is_ok(), "{ok}"); }
        for bad in ["", ".", "..", "a/b", "a\\b", "a b"] { assert!(validate_store_name(bad).is_err(), "{bad}"); }
    }

    #[test]
    fn toml_round_trip_preserves_active_and_content_root() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        std::fs::create_dir_all(&dig).unwrap();
        let mut ws = Workspace { dir: dig.clone(), active: None, stores: Default::default() };
        ws.register("default", &id(1), None).unwrap();
        ws.register("site", &id(2), Some("dist".into())).unwrap();
        ws.set_active("site").unwrap();
        ws.save().unwrap();
        let re = Workspace::load(&dig).unwrap();
        assert_eq!(re.active.as_deref(), Some("site"));
        assert_eq!(re.content_root("site"), Some("dist".to_string()));
        assert_eq!(re.content_root("default"), None);
    }

    #[test]
    fn selection_precedence_flag_then_active_then_single_then_error() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        std::fs::create_dir_all(&dig).unwrap();
        let mut ws = Workspace { dir: dig, active: None, stores: Default::default() };
        // none yet -> error
        assert!(ws.resolve_store_name(None).is_err());
        // single implicit
        ws.register("only", &id(1), None).unwrap();
        assert_eq!(ws.resolve_store_name(None).unwrap(), "only");
        // two -> needs active or flag
        ws.register("two", &id(2), None).unwrap();
        assert!(ws.resolve_store_name(None).is_err());
        ws.set_active("two").unwrap();
        assert_eq!(ws.resolve_store_name(None).unwrap(), "two");
        // explicit flag wins, unknown flag errors
        assert_eq!(ws.resolve_store_name(Some("only")).unwrap(), "only");
        assert!(ws.resolve_store_name(Some("nope")).is_err());
    }

    #[test]
    fn migrate_moves_legacy_single_store_into_default() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        std::fs::create_dir_all(&dig).unwrap();
        // legacy: config.toml directly under .dig, no stores/, no workspace.toml
        std::fs::write(dig.join("config.toml"),
            format!("store_id = \"{}\"\ndata_dir = \".\"\nmax_size = 1000\nvisibility = \"public\"\n", id(7))).unwrap();
        std::fs::write(dig.join("roots.log"), b"").unwrap();
        let ws = Workspace::load_or_migrate(&dig).unwrap();
        assert_eq!(ws.active.as_deref(), Some("default"));
        assert!(dig.join("stores/default/config.toml").exists());
        assert!(!dig.join("config.toml").exists());
        assert!(dig.join("workspace.toml").exists());
    }
}
```

- [ ] **Step 2: Run to confirm failure.** Run: `cargo test -p digstore-cli --lib workspace`. Expected: FAIL (module doesn't exist).

- [ ] **Step 3: Implement `workspace.rs`.**

```rust
//! The `.dig/` workspace: a registry of named stores plus the active selection.
//! CLI-owned (the store/core crates know nothing about names or content roots).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspaceToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default)]
    stores: BTreeMap<String, StoreEntryToml>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreEntryToml {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_root: Option<String>,
}

/// Runtime view of `workspace.toml`.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub dir: PathBuf, // the .dig/ directory
    pub active: Option<String>,
    pub stores: BTreeMap<String, StoreEntry>,
}

#[derive(Debug, Clone)]
pub struct StoreEntry {
    pub id: String,
    pub content_root: Option<String>,
}

/// Store names: non-empty, only `[A-Za-z0-9._-]`, not `.`/`..`, no separators.
pub fn validate_store_name(name: &str) -> Result<(), CliError> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(CliError::InvalidArgument(format!(
            "invalid store name '{name}': use letters, digits, '.', '_', '-' (no path separators)"
        )))
    }
}

impl Workspace {
    fn toml_path(dir: &Path) -> PathBuf { dir.join("workspace.toml") }

    /// Load an existing workspace (workspace.toml must exist).
    pub fn load(dir: &Path) -> Result<Self, CliError> {
        let path = Self::toml_path(dir);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CliError::Other(anyhow::anyhow!("read workspace.toml: {e}")))?;
        let wt: WorkspaceToml = toml::from_str(&text)
            .map_err(|e| CliError::Other(anyhow::anyhow!("parse workspace.toml: {e}")))?;
        Ok(Workspace {
            dir: dir.to_path_buf(),
            active: wt.active,
            stores: wt
                .stores
                .into_iter()
                .map(|(k, v)| (k, StoreEntry { id: v.id, content_root: v.content_root }))
                .collect(),
        })
    }

    /// Load, migrating a legacy single-store `.dig/` (config.toml at the root,
    /// no `stores/`, no workspace.toml) into `stores/default/` first.
    pub fn load_or_migrate(dir: &Path) -> Result<Self, CliError> {
        if Self::toml_path(dir).exists() {
            return Self::load(dir);
        }
        if dir.join("config.toml").exists() && !dir.join("stores").exists() {
            return Self::migrate_legacy(dir);
        }
        // Fresh/empty workspace.
        Ok(Workspace { dir: dir.to_path_buf(), active: None, stores: BTreeMap::new() })
    }

    fn migrate_legacy(dir: &Path) -> Result<Self, CliError> {
        let dest = dir.join("stores").join("default");
        std::fs::create_dir_all(&dest)
            .map_err(|e| CliError::Other(anyhow::anyhow!("migrate mkdir: {e}")))?;
        // Move every entry currently directly under .dig/ (except the new stores/ dir)
        // into stores/default/.
        for entry in std::fs::read_dir(dir)
            .map_err(|e| CliError::Other(anyhow::anyhow!("migrate scan: {e}")))?
        {
            let entry = entry.map_err(|e| CliError::Other(anyhow::anyhow!("migrate entry: {e}")))?;
            let name = entry.file_name();
            if name == "stores" || name == "workspace.toml" {
                continue;
            }
            let to = dest.join(&name);
            std::fs::rename(entry.path(), &to)
                .map_err(|e| CliError::Other(anyhow::anyhow!("migrate move {name:?}: {e}")))?;
        }
        let cfg = digstore_store::load_config(dest.join("config.toml"))
            .map_err(|e| CliError::Other(anyhow::anyhow!("migrate read config: {e}")))?;
        let mut ws = Workspace { dir: dir.to_path_buf(), active: None, stores: BTreeMap::new() };
        ws.register("default", &cfg.store_id.to_hex(), None)?;
        ws.set_active("default")?;
        ws.save()?;
        Ok(ws)
    }

    pub fn save(&self) -> Result<(), CliError> {
        let wt = WorkspaceToml {
            active: self.active.clone(),
            stores: self
                .stores
                .iter()
                .map(|(k, v)| (k.clone(), StoreEntryToml { id: v.id.clone(), content_root: v.content_root.clone() }))
                .collect(),
        };
        let text = toml::to_string_pretty(&wt)
            .map_err(|e| CliError::Other(anyhow::anyhow!("serialize workspace.toml: {e}")))?;
        std::fs::write(Self::toml_path(&self.dir), text)
            .map_err(|e| CliError::Other(anyhow::anyhow!("write workspace.toml: {e}")))
    }

    pub fn register(&mut self, name: &str, id_hex: &str, content_root: Option<String>) -> Result<(), CliError> {
        validate_store_name(name)?;
        if self.stores.contains_key(name) {
            return Err(CliError::InvalidArgument(format!("store '{name}' already exists")));
        }
        self.stores.insert(name.to_string(), StoreEntry { id: id_hex.to_string(), content_root });
        Ok(())
    }

    pub fn set_active(&mut self, name: &str) -> Result<(), CliError> {
        if !self.stores.contains_key(name) {
            return Err(CliError::InvalidArgument(format!("unknown store '{name}'")));
        }
        self.active = Some(name.to_string());
        Ok(())
    }

    pub fn set_content_root(&mut self, name: &str, root: Option<String>) -> Result<(), CliError> {
        let e = self
            .stores
            .get_mut(name)
            .ok_or_else(|| CliError::InvalidArgument(format!("unknown store '{name}'")))?;
        e.content_root = root;
        Ok(())
    }

    pub fn content_root(&self, name: &str) -> Option<String> {
        self.stores.get(name).and_then(|e| e.content_root.clone())
    }

    pub fn store_dir(&self, name: &str) -> PathBuf {
        self.dir.join("stores").join(name)
    }

    /// §2.3 precedence: explicit flag > active > single > error.
    pub fn resolve_store_name(&self, flag: Option<&str>) -> Result<String, CliError> {
        if let Some(name) = flag {
            if self.stores.contains_key(name) {
                return Ok(name.to_string());
            }
            return Err(CliError::InvalidArgument(format!("unknown store '{name}'")));
        }
        if let Some(active) = &self.active {
            if self.stores.contains_key(active) {
                return Ok(active.clone());
            }
        }
        if self.stores.len() == 1 {
            return Ok(self.stores.keys().next().unwrap().clone());
        }
        Err(CliError::InvalidArgument(
            "no store selected: use --store <name>, set one with `digstore use <name>`, or create one with `digstore init <name>`".into(),
        ))
    }
}
```
Add `pub mod workspace;` to `crates/digstore-cli/src/lib.rs`. (Confirm `tempfile` is a dev-dependency of `digstore-cli`; it is used by existing CLI tests.)

- [ ] **Step 4: Run.** Run: `cargo test -p digstore-cli --lib workspace`. Expected: PASS.

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/workspace.rs crates/digstore-cli/src/lib.rs
git commit -m "feat(cli): workspace.toml registry with store selection + legacy migration"
```

### Task B2: `CliContext` refactor (workspace_dir, op_dir, store_name)

**Files:**
- Modify: `crates/digstore-cli/src/context.rs`

- [ ] **Step 1: Write tests** (in `context.rs` `mod tests`) for the new constructors and op-dir precedence:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn for_store_points_dig_dir_at_store_subdir() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        let ctx = CliContext::for_store(dig.clone(), "site", None, dir.path().to_path_buf(), false, false);
        assert_eq!(ctx.dig_dir, dig.join("stores").join("site"));
        assert_eq!(ctx.workspace_dir, dig);
        assert_eq!(ctx.store_name.as_deref(), Some("site"));
    }

    #[test]
    fn op_dir_precedence_cwd_flag_over_content_root() {
        let dir = TempDir::new().unwrap();
        let project = dir.path();
        let dig = project.join(".dig");
        // content_root = "dist" -> op_dir = project/dist
        let a = CliContext::for_store_with_op(dig.clone(), "s", Some("dist".into()), None, project.to_path_buf(), false, false);
        assert_eq!(a.op_dir, project.join("dist"));
        // -C override wins (absolute)
        let abs = project.join("other");
        let b = CliContext::for_store_with_op(dig.clone(), "s", Some("dist".into()), Some(abs.clone()), project.to_path_buf(), false, false);
        assert_eq!(b.op_dir, abs);
    }
}
```

- [ ] **Step 2: Run.** `cargo test -p digstore-cli --lib context`. Expected: FAIL (new fields/ctors missing).

- [ ] **Step 3: Implement.** Update the struct to the final shape (see Shared design). Keep `discover_dig_dir`/`cwd_dig_dir` private fns unchanged. Replace `resolve`/`resolve_init` usage with constructors the dispatcher will call:

```rust
impl CliContext {
    /// Discover the `.dig/` workspace by walking up from CWD (or `explicit`).
    pub fn discover_workspace(explicit: Option<PathBuf>) -> PathBuf {
        explicit.or_else(Self::discover_dig_dir).unwrap_or_else(Self::cwd_dig_dir)
    }

    /// Workspace for `init`: anchored to CWD/.dig (or `explicit`), no walk-up.
    pub fn init_workspace(explicit: Option<PathBuf>) -> PathBuf {
        explicit.unwrap_or_else(Self::cwd_dig_dir)
    }

    /// Per-store context with op_dir defaulting to CWD (used before content_root is known).
    pub fn for_store(
        workspace_dir: PathBuf,
        name: &str,
        cwd_flag: Option<PathBuf>,
        cwd: PathBuf,
        json: bool,
        verbose: bool,
    ) -> Self {
        Self::for_store_with_op(workspace_dir, name, None, cwd_flag, cwd, json, verbose)
    }

    /// Per-store context resolving op_dir per §2.8: cwd_flag > content_root(joined to project root) > cwd.
    #[allow(clippy::too_many_arguments)]
    pub fn for_store_with_op(
        workspace_dir: PathBuf,
        name: &str,
        content_root: Option<String>,
        cwd_flag: Option<PathBuf>,
        cwd: PathBuf,
        json: bool,
        verbose: bool,
    ) -> Self {
        let dig_dir = workspace_dir.join("stores").join(name);
        let project_root = workspace_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| cwd.clone());
        let op_dir = match cwd_flag {
            Some(p) if p.is_absolute() => p,
            Some(p) => cwd.join(p),
            None => match content_root {
                Some(cr) => project_root.join(cr),
                None => cwd,
            },
        };
        CliContext {
            dig_dir,
            workspace_dir,
            op_dir,
            store_name: Some(name.to_string()),
            json,
            verbose,
        }
    }

    /// Workspace-only context (stores/use): no store resolved.
    pub fn workspace_only(workspace_dir: PathBuf, json: bool, verbose: bool) -> Self {
        CliContext {
            dig_dir: workspace_dir.clone(),
            workspace_dir,
            op_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            store_name: None,
            json,
            verbose,
        }
    }
}
```
Keep all existing path helpers (`config_path`, `load_config`, `staging_path`, etc.) exactly as-is — they operate on `dig_dir`.

- [ ] **Step 4: Run.** `cargo test -p digstore-cli --lib context`. Expected: PASS. (The crate will not fully build until B3 updates the callers of the old `resolve`/`resolve_init` — that is expected; this task's unit tests compile via `--lib` only if the rest of the crate compiles. If the crate doesn't compile yet, defer the full run to B3 and verify with `cargo check -p digstore-cli` showing only the expected dispatch/test call-site errors.)

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/context.rs
git commit -m "refactor(cli): CliContext carries workspace_dir, op_dir, store_name (dig_dir stays = store dir)"
```

### Task B3: Global flags + dispatch refactor (+ migration call)

**Files:**
- Modify: `crates/digstore-cli/src/cli.rs` (global flags, new Command variants + Args structs)
- Modify: `crates/digstore-cli/src/commands/mod.rs` (dispatch)

- [ ] **Step 1: Add the global flags and new command variants** in `cli.rs`. Add to `Cli`:
```rust
    /// Operate on a specific store by name (overrides the active store).
    #[arg(long = "store", global = true)]
    pub store_name: Option<String>,
    /// Operating directory for add/urn/status (overrides the store's content root).
    #[arg(short = 'C', long = "cwd", global = true)]
    pub cwd: Option<PathBuf>,
```
Add the new `Command` variants and Args structs:
```rust
    Stores(StoresArgs),
    Use(UseArgs),
    Dir(DirArgs),
    Unstage(UnstageArgs),
    Staged(StagedArgs),
    Urn(UrnArgs),
```
```rust
#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore stores")]
pub struct StoresArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore use site")]
pub struct UseArgs { pub name: String }

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore dir\n  digstore dir dist")]
pub struct DirArgs { pub path: Option<PathBuf> }

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore unstage")]
pub struct UnstageArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore staged")]
pub struct StagedArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore urn -A\n  digstore urn css/app.css\n  digstore urn file --root <hex>")]
pub struct UrnArgs {
    pub paths: Vec<PathBuf>,
    #[arg(short = 'A', long)]
    pub all: bool,
    #[arg(long)]
    pub root: Option<String>,
}
```
Update `InitArgs` to add a positional name and rename `--data_dir` to `--dir`:
```rust
#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore init\n  digstore init site --dir dist\n  digstore init --private")]
pub struct InitArgs {
    /// Store name (default: "default").
    pub name: Option<String>,
    #[arg(long)]
    pub private: bool,
    /// Content root (the build-output directory this store captures).
    #[arg(long)]
    pub dir: Option<String>,
}
```

- [ ] **Step 2: Refactor `dispatch`** in `commands/mod.rs` to: build Ui → resolve/migrate workspace → branch workspace-only vs store-scoped → build the right `CliContext`. Declare the new modules.
```rust
pub mod stores;
pub mod use_store;
pub mod dir;
pub mod unstage;
pub mod staged;
pub mod urn;
```
```rust
pub fn dispatch(cli: Cli) -> Result<(), CliError> {
    let ui = crate::ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.verbose);
    let cwd = std::env::current_dir().map_err(|e| CliError::Other(e.into()))?;

    // `init` anchors to CWD; everything else discovers the workspace by walking up.
    let workspace_dir = if matches!(cli.command, Command::Init(_)) {
        CliContext::init_workspace(cli.dig_dir.clone())
    } else {
        CliContext::discover_workspace(cli.dig_dir.clone())
    };

    // init creates the workspace itself; all other commands load (and migrate) it.
    match cli.command {
        Command::Init(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return init::run(&ctx, &ui, a);
        }
        Command::Stores(a) => {
            let ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return stores::run(&ctx, &ui, &ws, a);
        }
        Command::Use(a) => {
            let mut ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return use_store::run(&ctx, &ui, &mut ws, a);
        }
        _ => {}
    }

    // Store-scoped commands: resolve the workspace, the store name, and op_dir.
    let ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
    let name = ws.resolve_store_name(cli.store_name.as_deref())?;
    let content_root = ws.content_root(&name);
    let ctx = CliContext::for_store_with_op(
        workspace_dir, &name, content_root, cli.cwd.clone(), cwd, cli.json, cli.verbose,
    );

    match cli.command {
        Command::Add(a) => add::run(&ctx, &ui, a),
        Command::Commit(a) => commit::run(&ctx, &ui, a),
        Command::Status(a) => status::run(&ctx, &ui, a),
        Command::Log(a) => log::run(&ctx, &ui, a),
        Command::Diff(a) => diff::run(&ctx, &ui, a),
        Command::Checkout(a) => checkout::run(&ctx, &ui, a),
        Command::Cat(a) => cat::run(&ctx, &ui, a),
        Command::Dir(a) => dir::run(&ctx, &ui, a),
        Command::Unstage(a) => unstage::run(&ctx, &ui, a),
        Command::Staged(a) => staged::run(&ctx, &ui, a),
        Command::Urn(a) => urn::run(&ctx, &ui, a),
        Command::Remote(a) => remote::run(&ctx, &ui, a),
        Command::Clone(a) => clone::run(&ctx, &ui, a),
        Command::Push(a) => push::run(&ctx, &ui, a),
        Command::Pull(a) => pull::run(&ctx, &ui, a),
        Command::Init(_) | Command::Stores(_) | Command::Use(_) => unreachable!("handled above"),
    }
}
```
(`use_store::run`, `dir::run`, `unstage::run`, `staged::run`, `urn::run`, `stores::run` are created in later tasks; this task may stub them as `unimplemented!()` modules that compile, or — preferred — implement B4–B7/D1–D4 before running the full build. To keep TDD green per task, create the module files now with a `pub fn run(...) -> Result<(), CliError> { unimplemented!() }` matching the signature, and fill them in their own tasks.)

- [ ] **Step 3: Create compile-stub module files** so the crate builds: `stores.rs`, `use_store.rs`, `dir.rs`, `unstage.rs`, `staged.rs`, `urn.rs`, each with the correct `run` signature returning `unimplemented!()`. (Each is fully implemented in its own task below; the stub keeps the build green between tasks.)

- [ ] **Step 4: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS (init still uses old `init_store` signature — update in B4; if init breaks here, temporarily keep init calling the old path and finish in B4. Prefer to do B4 immediately after.)

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/cli.rs crates/digstore-cli/src/commands
git commit -m "feat(cli): --store/-C global flags, new subcommand variants, workspace-aware dispatch + migration"
```

### Task B4: `init [name] [--dir]` writes workspace.toml; 100 MB default

**Files:**
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (`MAX_STORE_BYTES`, `init_store` signature)
- Modify: `crates/digstore-cli/src/commands/init.rs`
- Modify: `crates/digstore-cli/src/ops/remote_ops.rs:196` (clone default → 100 MB)

- [ ] **Step 1: Add the constant and change `init_store`** in `store_ops.rs`. At the top of the file:
```rust
/// Per-store hard cap on staged content (§3). 100 MB, decimal.
pub const MAX_STORE_BYTES: u64 = 100_000_000;
```
Change `init_store` to take the store directory it should create plus the name, and to set `max_size = MAX_STORE_BYTES`. Since `ctx.dig_dir` is now `.dig/stores/<name>/`, `init_store` writes there directly. Replace the 1 GiB literal (`:131`) with `MAX_STORE_BYTES`. The `data_dir` field stays `ctx.dig_dir.display()` (it is the store's own dir; **content_root is separate, in workspace.toml**). Keep `ensure_dig_gitignored` but call it on `ctx.workspace_dir` (so `.dig/` is ignored once at workspace level).

Replace `:104-108` and `:131` and `:143`:
```rust
    let dd = data_dir.unwrap_or_else(|| ctx.dig_dir.display().to_string());
    let cfg = StoreConfig {
        store_id,
        data_dir: dd,
        max_size: MAX_STORE_BYTES,
        visibility,
    };
```
```rust
    // Git convenience: ignore the workspace `.dig/` once.
    ensure_dig_gitignored(&ctx.workspace_dir);
```
(The `init_store` signature stays `(ctx, private, data_dir: Option<String>)`; the caller now passes the per-store ctx with `dig_dir = .dig/stores/<name>/`. Confirm `ctx.config_path().exists()` guard still points at the per-store config.)

- [ ] **Step 2: Update `remote_ops.rs:196`** clone default:
```rust
        max_size: crate::ops::store_ops::MAX_STORE_BYTES,
```

- [ ] **Step 3: Rewrite `init::run`** to create the store dir, run `init_store`, then register in workspace.toml + set active if first + record content_root.
```rust
pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: InitArgs) -> Result<(), CliError> {
    let name = args.name.clone().unwrap_or_else(|| "default".to_string());
    crate::workspace::validate_store_name(&name)?;

    // Load or create the workspace (migrating a legacy single-store layout first).
    let mut ws = crate::workspace::Workspace::load_or_migrate(&ctx.workspace_dir)?;
    if ws.stores.contains_key(&name) {
        return Err(CliError::InvalidArgument(format!("store '{name}' already exists")));
    }

    // Per-store context for init_store (dig_dir = .dig/stores/<name>/).
    let store_dir = ws.store_dir(&name);
    std::fs::create_dir_all(&store_dir).map_err(|e| CliError::Other(e.into()))?;
    let store_ctx = CliContext {
        dig_dir: store_dir,
        workspace_dir: ctx.workspace_dir.clone(),
        op_dir: ctx.op_dir.clone(),
        store_name: Some(name.clone()),
        json: ctx.json,
        verbose: ctx.verbose,
    };
    let res = store_ops::init_store(&store_ctx, args.private, None)?;

    let first = ws.stores.is_empty();
    ws.register(&name, &res.store_id.to_hex(), args.dir.clone())?;
    if first {
        ws.set_active(&name)?;
    }
    ws.save()?;

    let content_root = args.dir.clone().unwrap_or_else(|| ".".to_string());
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "store": name,
            "store_id": res.store_id.to_hex(),
            "host_public_key": res.host_public_key.to_hex(),
            "content_root": args.dir,
            "active": first,
        }));
    } else {
        ui.success(format!("Initialized store '{}' ({})", name, res.store_id.to_hex()));
        ui.line(format!("  content root: {content_root}"));
        if first { ui.line("  set as active store"); }
        ui.line(format!("  trusted host key: {}", res.host_public_key.to_hex()));
        ui.hint("digstore add -A");
    }
    Ok(())
}
```

- [ ] **Step 4: Build + run existing init tests** (they will be updated in F1; for now confirm compile). Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/ops/store_ops.rs crates/digstore-cli/src/ops/remote_ops.rs crates/digstore-cli/src/commands/init.rs
git commit -m "feat(cli): init [name] [--dir] registers store in workspace.toml; 100 MB default cap"
```

### Task B5: `digstore stores` (list)

**Files:**
- Modify: `crates/digstore-cli/src/commands/stores.rs` (replace stub)

- [ ] **Step 1: Implement `stores::run(ctx, ui, ws, _args)`.** List each store: `*` marker on active, name, short id, content root, current root (short) or `(empty)`, used/limit. Compute per-store staged bytes by opening each store's staging area; compute current root via the store's `roots.log` head; `--json` emits the full array.
```rust
use crate::context::CliContext;
use crate::ui::Ui;
use crate::workspace::Workspace;

pub fn run(_ctx: &CliContext, ui: &Ui, ws: &Workspace, _args: crate::cli::StoresArgs) -> Result<(), CliError> {
    #[derive(serde::Serialize)]
    struct Row<'a> {
        name: &'a str,
        store_id: &'a str,
        active: bool,
        content_root: Option<String>,
        current_root: Option<String>,
        staged_bytes: u64,
        limit_bytes: u64,
    }
    let mut rows = Vec::new();
    for (name, entry) in &ws.stores {
        let store_dir = ws.store_dir(name);
        let staged_bytes = staged_total_for_dir(&store_dir, &entry.id);
        let current_root = current_root_for_dir(&store_dir);
        rows.push(Row {
            name,
            store_id: &entry.id,
            active: ws.active.as_deref() == Some(name.as_str()),
            content_root: entry.content_root.clone(),
            current_root,
            staged_bytes,
            limit_bytes: crate::ops::store_ops::MAX_STORE_BYTES,
        });
    }
    if ui.json() {
        ui.emit_json(&rows);
        return Ok(());
    }
    if rows.is_empty() {
        ui.line("no stores; create one with `digstore init`");
        return Ok(());
    }
    for r in &rows {
        let star = if r.active { "*" } else { " " };
        let root = r.current_root.as_deref().map(|h| &h[..h.len().min(12)]).unwrap_or("(empty)");
        let cr = r.content_root.clone().unwrap_or_else(|| ".".into());
        ui.line(format!(
            "{star} {:<16} {}…  root {}  dir {}  {}",
            r.name,
            &r.store_id[..r.store_id.len().min(8)],
            root,
            cr,
            crate::ui::human_capacity(r.staged_bytes, r.limit_bytes),
        ));
    }
    Ok(())
}
```
Add two small file-level helpers in this module: `staged_total_for_dir(store_dir, id_hex) -> u64` (open `<store_dir>/<id>.staging.bin` via `digstore_store::StagingArea`, sum `records()` content lengths, 0 on error) and `current_root_for_dir(store_dir) -> Option<String>` (open `<store_dir>/roots.log` via `digstore_store::RootHistory`, return `head()` hex). `human_capacity` is added in Task C3 — if C3 is not yet done, inline a `format!("{}/{} MB", …)` placeholder and replace it in C3. (Order C3 before B5 if executing strictly; otherwise the placeholder is fine.)

- [ ] **Step 2: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 3: Commit.**
```bash
git add crates/digstore-cli/src/commands/stores.rs
git commit -m "feat(cli): `digstore stores` lists stores with active marker, root, content root, capacity"
```

### Task B6: `digstore use <name>` (set active)

**Files:**
- Modify: `crates/digstore-cli/src/commands/use_store.rs` (replace stub)

- [ ] **Step 1: Implement.**
```rust
use crate::context::CliContext;
use crate::ui::Ui;
use crate::workspace::Workspace;

pub fn run(_ctx: &CliContext, ui: &Ui, ws: &mut Workspace, args: crate::cli::UseArgs) -> Result<(), CliError> {
    ws.set_active(&args.name)?;
    ws.save()?;
    let cr = ws.content_root(&args.name).unwrap_or_else(|| ".".into());
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "active": args.name, "content_root": ws.content_root(&args.name) }));
    } else {
        ui.success(format!("active store is now '{}'", args.name));
        ui.line(format!("  content root: {cr}"));
    }
    Ok(())
}
```

- [ ] **Step 2: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 3: Commit.**
```bash
git add crates/digstore-cli/src/commands/use_store.rs
git commit -m "feat(cli): `digstore use <name>` sets the active store"
```

### Task B7: `digstore dir [<path>]` (show/set content root)

**Files:**
- Modify: `crates/digstore-cli/src/commands/dir.rs` (replace stub)

- [ ] **Step 1: Implement.** With no arg, print the selected store's content root (resolved op_dir + the stored value); with a path, persist it to workspace.toml. Warn (don't error) if the path doesn't exist yet.
```rust
use crate::context::CliContext;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: crate::cli::DirArgs) -> Result<(), CliError> {
    let name = ctx.store_name.clone().ok_or_else(|| CliError::InvalidArgument("no store selected".into()))?;
    let mut ws = crate::workspace::Workspace::load_or_migrate(&ctx.workspace_dir)?;
    match args.path {
        None => {
            let cr = ws.content_root(&name).unwrap_or_else(|| ".".into());
            if ui.json() {
                ui.emit_json(&serde_json::json!({ "store": name, "content_root": ws.content_root(&name), "operating_dir": ctx.op_dir.display().to_string() }));
            } else {
                ui.line(format!("content root: {cr}"));
            }
        }
        Some(p) => {
            let value = p.to_string_lossy().replace('\\', "/");
            ws.set_content_root(&name, Some(value.clone()))?;
            ws.save()?;
            let project_root = ctx.workspace_dir.parent().map(|x| x.to_path_buf()).unwrap_or_default();
            if !project_root.join(&value).exists() {
                ui.line(format!("note: '{value}' does not exist yet (build output dirs are often created later)"));
            }
            if ui.json() {
                ui.emit_json(&serde_json::json!({ "store": name, "content_root": value }));
            } else {
                ui.success(format!("content root for '{name}' set to '{value}'"));
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 3: Commit.**
```bash
git add crates/digstore-cli/src/commands/dir.rs
git commit -m "feat(cli): `digstore dir [path]` shows/sets the per-store content root"
```

---

## Phase C — Content-root scoping, 100 MB cap, capacity surfacing

### Task C1: Walk scoping (skip workspace dir; reject escapes)

**Files:**
- Modify: `crates/digstore-cli/src/ops/walk.rs`

- [ ] **Step 1: Write tests** (in `walk.rs` `mod tests`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn resolve_all_skips_the_workspace_dir_even_when_op_dir_is_project_root() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".dig/stores/default")).unwrap();
        std::fs::write(root.join(".dig/workspace.toml"), b"").unwrap();
        std::fs::write(root.join("a.txt"), b"hi").unwrap();
        let out = resolve_all(root, &root.join(".dig"));
        let keys: Vec<_> = out.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains(&"a.txt"));
        assert!(!keys.iter().any(|k| k.starts_with(".dig")));
    }

    #[test]
    fn resolve_arg_rejects_paths_escaping_the_operating_dir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("dist");
        std::fs::create_dir_all(&root).unwrap();
        let mut out = Vec::new();
        let err = resolve_arg(&root, &root.join(".dig"), "../secret.txt", &mut out);
        assert!(err.is_err());
    }
}
```

- [ ] **Step 2: Run.** `cargo test -p digstore-cli --lib walk`. Expected: FAIL (signatures differ; no escape guard).

- [ ] **Step 3: Implement.** Add a `skip: &Path` parameter to `walk_dir`, `resolve_all`, `resolve_arg`; skip any path under `skip`; reject escapes in `resolve_arg`.
```rust
fn walk_dir(root: &Path, skip: &Path, dir: &Path, out: &mut Vec<Resolved>) {
    let mut wb = WalkBuilder::new(dir);
    wb.hidden(false).git_ignore(true).git_global(true).git_exclude(true)
        .add_custom_ignore_filename(".digignore");
    for entry in wb.build().flatten() {
        let p = entry.path();
        if p.starts_with(skip) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            out.push(Resolved { path: p.to_path_buf(), key: key_for(root, p) });
        }
    }
}

pub fn resolve_all(root: &Path, skip: &Path) -> Vec<Resolved> {
    let mut out = Vec::new();
    walk_dir(root, skip, root, &mut out);
    out
}

pub fn resolve_arg(root: &Path, skip: &Path, arg: &str, out: &mut Vec<Resolved>) -> Result<(), String> {
    let as_path = root.join(arg);
    // Reject paths that escape the operating directory (§2.8).
    let within = |p: &Path| -> bool {
        match (p.canonicalize(), root.canonicalize()) {
            (Ok(pc), Ok(rc)) => pc.starts_with(&rc),
            // Not-yet-existing path: fall back to lexical containment.
            _ => !arg.split(['/', '\\']).any(|seg| seg == ".."),
        }
    };
    if as_path.is_file() {
        if !within(&as_path) {
            return Err(format!("'{arg}' is outside the operating directory"));
        }
        out.push(Resolved { path: as_path.clone(), key: key_for(root, &as_path) });
        return Ok(());
    }
    if as_path.is_dir() {
        if !within(&as_path) {
            return Err(format!("'{arg}' is outside the operating directory"));
        }
        walk_dir(root, skip, &as_path, out);
        return Ok(());
    }
    // Glob relative to root.
    let glob = Glob::new(arg).map_err(|e| format!("bad pattern '{arg}': {e}"))?.compile_matcher();
    let mut all = Vec::new();
    walk_dir(root, skip, root, &mut all);
    let before = out.len();
    for r in all {
        if glob.is_match(&r.key) {
            out.push(r);
        }
    }
    if out.len() == before {
        return Err(format!("no files matched '{arg}'"));
    }
    Ok(())
}
```

- [ ] **Step 4: Run.** `cargo test -p digstore-cli --lib walk`. Expected: PASS (callers updated in C2/C5/D4; crate may not fully build until then — verify with `cargo check` showing only expected call-site errors, fixed immediately in C2).

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/ops/walk.rs
git commit -m "feat(cli): walk scopes to operating dir, skips workspace dir, rejects escapes"
```

### Task C2: `add` — content-root scope + atomic 100 MB cap + capacity in outcome

**Files:**
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (`AddOutcome`, `add_files`)

- [ ] **Step 1: Write tests** (in `store_ops.rs` `mod tests`, mirroring existing add tests with the new `op_dir`):
```rust
    #[test]
    fn add_over_cap_stages_nothing_and_errors() {
        let (ctx, _td) = test_store_ctx(); // helper that inits a store; see existing tests
        // Write two files whose combined size exceeds MAX_STORE_BYTES via a tiny cap override is not possible
        // (cap is constant), so instead assert the arithmetic path: stage a file, then attempt a batch that
        // would exceed the cap using a stubbed large file. Use a temp file sized to exceed the remaining room.
        // (Implementer: size the fixture to MAX_STORE_BYTES+1 using a sparse/zero buffer written to op_dir.)
    }

    #[test]
    fn add_outcome_reports_staged_total_and_limit() {
        let (ctx, _td) = test_store_ctx();
        std::fs::write(ctx.op_dir.join("a.txt"), b"hello").unwrap();
        let out = add_files(&ctx, &[], true, false, None).unwrap();
        assert_eq!(out.limit_bytes, MAX_STORE_BYTES);
        assert_eq!(out.staged_bytes, 5);
    }
```
(If a `test_store_ctx` helper does not exist, add one in the test module that builds a `CliContext` via `Workspace`/`init_store` in a `TempDir` with `op_dir = project root`.)

- [ ] **Step 2: Run.** `cargo test -p digstore-cli --lib store_ops::tests::add`. Expected: FAIL (no cap check; `AddOutcome` lacks fields).

- [ ] **Step 3: Extend `AddOutcome` and rewrite `add_files`.**
```rust
pub struct AddOutcome {
    pub staged: Vec<(String, u64)>,
    pub unchanged: usize,
    pub dry_run: bool,
    pub staged_bytes: u64, // total staged after this add (or projected, for dry-run)
    pub limit_bytes: u64,
}
```
```rust
pub fn add_files(
    ctx: &CliContext,
    paths: &[PathBuf],
    all: bool,
    dry_run: bool,
    key: Option<String>,
) -> Result<AddOutcome, CliError> {
    use crate::ops::walk::{self, Resolved};

    let cfg = ctx.load_config()?;
    let root = ctx.op_dir.clone();
    let skip = ctx.workspace_dir.clone();

    let mut resolved: Vec<Resolved> = Vec::new();
    if all {
        resolved = walk::resolve_all(&root, &skip);
    } else {
        for p in paths {
            let arg = p.to_string_lossy();
            walk::resolve_arg(&root, &skip, &arg, &mut resolved).map_err(CliError::InvalidArgument)?;
        }
    }
    resolved.sort_by(|a, b| a.key.cmp(&b.key));
    resolved.dedup_by(|a, b| a.key == b.key);

    if key.is_some() && resolved.len() != 1 {
        return Err(CliError::InvalidArgument("--key requires exactly one file path".into()));
    }

    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let already: HashMap<String, Vec<u8>> = staging
        .records()
        .map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))??
        .into_iter()
        .map(|r| (r.resource_key, r.content))
        .collect();
    let already_bytes: u64 = already.values().map(|c| c.len() as u64).sum();

    // Read all incoming, decide new vs unchanged, and pre-sum — ATOMIC cap check.
    struct Incoming { key: String, data: Vec<u8> }
    let mut incoming: Vec<Incoming> = Vec::new();
    let mut unchanged = 0usize;
    for r in resolved {
        let data = fs::read(&r.path).map_err(|e| CliError::Other(e.into()))?;
        let effective_key = key.clone().unwrap_or_else(|| r.key.clone());
        if already.get(&effective_key).map(|c| c == &data).unwrap_or(false) {
            unchanged += 1;
            continue;
        }
        incoming.push(Incoming { key: effective_key, data });
    }
    let incoming_bytes: u64 = incoming.iter().map(|i| i.data.len() as u64).sum();
    let cap = if cfg.max_size == 0 { MAX_STORE_BYTES } else { cfg.max_size };
    let projected = already_bytes + incoming_bytes;
    if projected > cap {
        let store = ctx.store_name.clone().unwrap_or_else(|| "this".into());
        return Err(CliError::InvalidArgument(format!(
            "staging would reach {} MB, over the {} store's {} MB limit ({} MB free); stage fewer files or create another store (digstore init <name2>)",
            mb(projected), store, mb(cap), mb(cap.saturating_sub(already_bytes))
        )));
    }

    let mut outcome = AddOutcome {
        staged: Vec::new(),
        unchanged,
        dry_run,
        staged_bytes: projected,
        limit_bytes: cap,
    };
    if !dry_run {
        for i in incoming {
            let size = i.data.len() as u64;
            staging.append(&i.key, &i.data).map_err(|e| CliError::Other(anyhow::anyhow!("stage: {e}")))?;
            outcome.staged.push((i.key, size));
        }
    } else {
        for i in incoming {
            outcome.staged.push((i.key.clone(), i.data.len() as u64));
        }
        // dry-run: staged_bytes is the projected total if these were applied.
    }
    Ok(outcome)
}

/// Decimal MB, one decimal place.
fn mb(bytes: u64) -> String { format!("{:.1}", bytes as f64 / 1_000_000.0) }
```

- [ ] **Step 4: Run.** `cargo test -p digstore-cli --lib store_ops`. Expected: PASS.

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): add scopes to op_dir, enforces atomic 100 MB cap, reports capacity"
```

### Task C3: `Ui` capacity rendering

**Files:**
- Modify: `crates/digstore-cli/src/ui/mod.rs`

- [ ] **Step 1: Write tests** (in `ui/mod.rs` `mod tests`):
```rust
    #[test]
    fn human_capacity_is_plain_when_no_color() {
        let s = human_capacity(47_200_000, 100_000_000);
        assert!(s.contains("47.2 MB"));
        assert!(s.contains("52.8 MB free"));
        assert!(s.contains("100.0 MB"));
    }
```

- [ ] **Step 2: Run.** `cargo test -p digstore-cli --lib ui`. Expected: FAIL.

- [ ] **Step 3: Implement.** Add a free function `human_capacity` (the plain numeric string, reused by `stores`) and a `Ui::capacity` method (numbers always unless `--json`; bar only when `color && !quiet`):
```rust
/// Plain capacity string: "47.2 MB staged · 52.8 MB free of 100.0 MB".
pub fn human_capacity(staged: u64, limit: u64) -> String {
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
    let free = limit.saturating_sub(staged);
    format!("{} staged · {} free of {}", mb(staged), mb(free), mb(limit))
}

impl Ui {
    pub fn capacity(&self, staged: u64, limit: u64) {
        if self.json {
            return;
        }
        let nums = human_capacity(staged, limit);
        if self.color && !self.quiet {
            let width = 18usize;
            let filled = if limit == 0 { 0 } else { ((staged as u128 * width as u128) / limit as u128) as usize };
            let filled = filled.min(width);
            let bar: String = core::iter::repeat('#').take(filled)
                .chain(core::iter::repeat('·').take(width - filled)).collect();
            let mut o = self.out();
            let _ = writeln!(o, "  {nums}  [{bar}]");
        } else {
            let mut o = self.out();
            let _ = writeln!(o, "  {nums}");
        }
    }
}
```

- [ ] **Step 4: Run.** `cargo test -p digstore-cli --lib ui`. Expected: PASS. If Task B5 used a placeholder for capacity, replace it with `human_capacity` now.

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/ui/mod.rs
git commit -m "feat(cli): Ui::capacity + human_capacity (used/free/limit, bar on TTY)"
```

### Task C4: `add` handler shows capacity + JSON fields

**Files:**
- Modify: `crates/digstore-cli/src/commands/add.rs`

- [ ] **Step 1: Update the handler.** After the staged items (and unchanged line), render capacity; add `staged_bytes`/`limit_bytes` to JSON.
```rust
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "staged": outcome.staged.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            "unchanged": outcome.unchanged,
            "dry_run": outcome.dry_run,
            "staged_bytes": outcome.staged_bytes,
            "limit_bytes": outcome.limit_bytes,
        }));
        return Ok(());
    }
    let verb = if outcome.dry_run { "Would stage" } else { "Staged" };
    ui.verb(verb, format!("{} file(s)", outcome.staged.len()));
    for (k, _size) in &outcome.staged {
        ui.item(Marker::Staged, k);
    }
    if outcome.unchanged > 0 {
        ui.line(format!("  {} unchanged", outcome.unchanged));
    }
    ui.capacity(outcome.staged_bytes, outcome.limit_bytes);
    if !outcome.dry_run && !outcome.staged.is_empty() {
        ui.hint("digstore commit -m \"...\"");
    }
    Ok(())
```

- [ ] **Step 2: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 3: Commit.**
```bash
git add crates/digstore-cli/src/commands/add.rs
git commit -m "feat(cli): add prints capacity bar + emits staged/limit bytes in JSON"
```

### Task C5: `status` content-root scope + capacity header

**Files:**
- Modify: `crates/digstore-cli/src/output.rs` (`StatusView` + `render_status`)
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (`compute_status`, `status`)

- [ ] **Step 1: Add capacity fields to `StatusView`** and render them:
```rust
#[derive(Debug, Serialize)]
pub struct StatusView {
    pub root: Option<String>,
    pub staged: Vec<String>,
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
    pub staged_bytes: u64,
    pub limit_bytes: u64,
}
```
In `render_status`, after the generation-root line:
```rust
    ui.capacity(s.staged_bytes, s.limit_bytes);
```
Update the two existing `output.rs` unit tests to include the new fields (`staged_bytes: 0, limit_bytes: 100_000_000`).

- [ ] **Step 2: Update `compute_status` and `status`** in `store_ops.rs` to scan `ctx.op_dir` (skip `ctx.workspace_dir`) and fill the capacity fields from staging totals.
In `compute_status`: replace `let root_dir = ctx.dig_dir.parent()...` with `let root_dir = ctx.op_dir.clone();` and `walk::resolve_all(&root_dir)` → `walk::resolve_all(&root_dir, &ctx.workspace_dir)`. Compute `staged_bytes = staged_map.values().map(|c| c.len() as u64).sum()` and add `staged_bytes`, `limit_bytes: cfg.max_size.max_or(MAX_STORE_BYTES)` to the returned `StatusView`. (Use `if cfg.max_size == 0 { MAX_STORE_BYTES } else { cfg.max_size }`.) Apply the same two new fields to the lighter `status` fn.

- [ ] **Step 3: Run.** `cargo test -p digstore-cli --lib`. Expected: PASS.

- [ ] **Step 4: Commit.**
```bash
git add crates/digstore-cli/src/output.rs crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): status scans content root + shows capacity header"
```

### Task C6: `commit` defensive cap check

**Files:**
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (`commit`)

- [ ] **Step 1: Add the guard** right after the `records.is_empty()` check in `commit`:
```rust
    let cap = if cfg.max_size == 0 { MAX_STORE_BYTES } else { cfg.max_size };
    let staged_total: u64 = records.iter().map(|r| r.content.len() as u64).sum();
    if staged_total > cap {
        return Err(CliError::InvalidArgument(format!(
            "staged content is {:.1} MB, over the {:.1} MB limit; unstage some files (digstore unstage) before committing",
            staged_total as f64 / 1_000_000.0, cap as f64 / 1_000_000.0
        )));
    }
```

- [ ] **Step 2: Build + run.** Run: `cargo test -p digstore-cli --lib store_ops`. Expected: PASS.

- [ ] **Step 3: Commit.**
```bash
git add crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): defensive 100 MB cap check at commit"
```

---

## Phase D — Staging management + URN

### Task D1: `digstore unstage`

**Files:**
- Modify: `crates/digstore-cli/src/commands/unstage.rs` (replace stub)
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (add `clear_staging`)

- [ ] **Step 1: Add `clear_staging` op** in `store_ops.rs`:
```rust
/// Clear the selected store's staging area; returns how many entries were dropped.
pub fn clear_staging(ctx: &CliContext) -> Result<usize, CliError> {
    let cfg = ctx.load_config()?;
    let mut staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let n = staging.records().map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))??.len();
    staging.clear().map_err(|e| CliError::Other(anyhow::anyhow!("clear staging: {e}")))?;
    Ok(n)
}
```

- [ ] **Step 2: Implement the handler.**
```rust
use crate::context::CliContext;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, _args: crate::cli::UnstageArgs) -> Result<(), CliError> {
    let cleared = crate::ops::store_ops::clear_staging(ctx)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "cleared": cleared }));
    } else {
        ui.success(format!("cleared {cleared} staged entr{}", if cleared == 1 { "y" } else { "ies" }));
    }
    Ok(())
}
```

- [ ] **Step 3: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 4: Commit.**
```bash
git add crates/digstore-cli/src/commands/unstage.rs crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): `digstore unstage` clears the staging area"
```

### Task D2: `digstore staged`

**Files:**
- Modify: `crates/digstore-cli/src/commands/staged.rs` (replace stub)
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (add `list_staged`)

- [ ] **Step 1: Add `list_staged` op** returning `(Vec<(String,u64)>, total, limit)`:
```rust
pub fn list_staged(ctx: &CliContext) -> Result<(Vec<(String, u64)>, u64, u64), CliError> {
    let cfg = ctx.load_config()?;
    let staging = StagingArea::open(ctx.staging_path(&cfg.store_id))
        .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
    let mut entries: Vec<(String, u64)> = staging
        .records().map_err(|e| CliError::Other(anyhow::anyhow!("read staging: {e}")))??
        .into_iter().map(|r| (r.resource_key, r.content.len() as u64)).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let total: u64 = entries.iter().map(|(_, s)| *s).sum();
    let cap = if cfg.max_size == 0 { MAX_STORE_BYTES } else { cfg.max_size };
    Ok((entries, total, cap))
}
```

- [ ] **Step 2: Implement the handler** (per-file size, total, capacity footer; JSON shape per §5):
```rust
use crate::context::CliContext;
use crate::ui::{Marker, Ui};

pub fn run(ctx: &CliContext, ui: &Ui, _args: crate::cli::StagedArgs) -> Result<(), CliError> {
    let (entries, total, limit) = crate::ops::store_ops::list_staged(ctx)?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({
            "staged": entries.iter().map(|(k, s)| serde_json::json!({ "key": k, "size": s })).collect::<Vec<_>>(),
            "total_bytes": total,
            "limit_bytes": limit,
        }));
        return Ok(());
    }
    if entries.is_empty() {
        ui.line("nothing staged");
        ui.capacity(0, limit);
        return Ok(());
    }
    for (k, s) in &entries {
        ui.item(Marker::Staged, format!("{k}  ({:.1} MB)", *s as f64 / 1_000_000.0));
    }
    ui.capacity(total, limit);
    Ok(())
}
```
(Confirm `crate::ui::Marker` is re-exported; if not, use `crate::ui::theme::Marker`.)

- [ ] **Step 3: Build.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 4: Commit.**
```bash
git add crates/digstore-cli/src/commands/staged.rs crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): `digstore staged` lists entries + sizes + capacity"
```

### Task D3: URN manifest on commit (`urns.json` + `urns.txt`)

**Files:**
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (`commit`)

- [ ] **Step 1: Write a test** (in `store_ops.rs` `mod tests`) that commits a known file and asserts `urns.json` exists with the right key/URN/retrieval_key and `urns.txt` has a `key\turn` line:
```rust
    #[test]
    fn commit_writes_urn_manifest() {
        let (ctx, _td) = test_store_ctx();
        std::fs::write(ctx.op_dir.join("readme.md"), b"hi").unwrap();
        add_files(&ctx, &[], true, false, None).unwrap();
        let res = commit(&ctx, None).unwrap();
        let json = std::fs::read_to_string(ctx.dig_dir.join("urns.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["resources"][0]["key"], "readme.md");
        let urn = v["resources"][0]["urn"].as_str().unwrap();
        assert!(urn.contains(&res.roothash.to_hex()));
        assert!(urn.starts_with("urn:dig:chia:"));
        let txt = std::fs::read_to_string(ctx.dig_dir.join("urns.txt")).unwrap();
        assert!(txt.contains("readme.md\turn:dig:chia:"));
    }
```

- [ ] **Step 2: Run.** `cargo test -p digstore-cli --lib store_ops::tests::commit_writes_urn_manifest`. Expected: FAIL.

- [ ] **Step 3: Write the manifest** in `commit`, after `history.append(...)` (root is final) and **before** the existing `staging.clear()`. Use the already-built `manifest.key_table` (resource_key, static_key = rootless retrieval_key, total_size):
```rust
    // Local URN manifest (§6.1): the publisher's index of shareable URNs. Local
    // only — not embedded, not pushed. Root-pinned URN, rootless retrieval key.
    {
        #[derive(serde::Serialize)]
        struct UrnEntry { key: String, urn: String, retrieval_key: String, size: u64 }
        #[derive(serde::Serialize)]
        struct UrnManifest {
            store_id: String,
            store: Option<String>,
            root: String,
            generation: u64,
            resources: Vec<UrnEntry>,
        }
        let mut resources = Vec::with_capacity(manifest.key_table.len());
        let mut txt = String::new();
        for rec in &manifest.key_table {
            let pinned = digstore_core::urn::Urn {
                chain: "chia".to_string(),
                store_id: cfg.store_id,
                root_hash: Some(root),
                resource_key: Some(rec.resource_key.clone()),
            };
            let urn = pinned.canonical();
            txt.push_str(&format!("{}\t{}\n", rec.resource_key, urn));
            resources.push(UrnEntry {
                key: rec.resource_key.clone(),
                urn,
                retrieval_key: rec.static_key.to_hex(),
                size: rec.total_size,
            });
        }
        let out = UrnManifest {
            store_id: cfg.store_id.to_hex(),
            store: ctx.store_name.clone(),
            root: root_hex.clone(),
            generation: next_id,
            resources,
        };
        let json = serde_json::to_string_pretty(&out).map_err(|e| CliError::Other(e.into()))?;
        fs::write(ctx.dig_dir.join("urns.json"), json).map_err(|e| CliError::Other(e.into()))?;
        fs::write(ctx.dig_dir.join("urns.txt"), txt).map_err(|e| CliError::Other(e.into()))?;
    }
```
(Confirm `digstore_core::urn::Urn` is importable in this crate — `digstore-core` is already a dependency, and `canonical_resource_urn` uses `Urn` here. Use the existing import path.)

- [ ] **Step 4: Run.** `cargo test -p digstore-cli --lib store_ops::tests::commit_writes_urn_manifest`. Expected: PASS.

- [ ] **Step 5: Commit.**
```bash
git add crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): commit writes a local URN manifest (urns.json + urns.txt)"
```

### Task D4: `digstore urn [PATHS]` preview

**Files:**
- Modify: `crates/digstore-cli/src/commands/urn.rs` (replace stub)
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (add `preview_urns`)

- [ ] **Step 1: Add `preview_urns` op** mirroring `add` resolution (op_dir scope, content-root-relative keys), rootless by default, `--root` pins:
```rust
pub struct UrnPreview {
    pub path: String,
    pub key: String,
    pub urn: String,
    pub retrieval_key: String,
}

pub fn preview_urns(ctx: &CliContext, paths: &[PathBuf], all: bool, root_hex: Option<&str>) -> Result<Vec<UrnPreview>, CliError> {
    use crate::ops::walk::{self, Resolved};
    let cfg = ctx.load_config()?;
    let root = ctx.op_dir.clone();
    let skip = ctx.workspace_dir.clone();
    let mut resolved: Vec<Resolved> = Vec::new();
    if all {
        resolved = walk::resolve_all(&root, &skip);
    } else {
        for p in paths {
            walk::resolve_arg(&root, &skip, &p.to_string_lossy(), &mut resolved).map_err(CliError::InvalidArgument)?;
        }
    }
    resolved.sort_by(|a, b| a.key.cmp(&b.key));
    resolved.dedup_by(|a, b| a.key == b.key);

    let pinned_root = match root_hex {
        Some(h) => Some(digstore_core::Bytes32::from_hex(h).map_err(|_| CliError::InvalidArgument(format!("bad root hex: {h}")))?),
        None => None,
    };
    let mut out = Vec::new();
    for r in resolved {
        // Retrieval key is ALWAYS from the rootless canonical URN.
        let rootless = canonical_resource_urn(cfg.store_id, &r.key);
        let display = digstore_core::urn::Urn {
            chain: "chia".to_string(),
            store_id: cfg.store_id,
            root_hash: pinned_root,
            resource_key: Some(r.key.clone()),
        };
        out.push(UrnPreview {
            path: r.path.display().to_string(),
            key: r.key,
            urn: display.canonical(),
            retrieval_key: rootless.retrieval_key().to_hex(),
        });
    }
    Ok(out)
}
```

- [ ] **Step 2: Implement the handler.**
```rust
use crate::context::CliContext;
use crate::ui::Ui;

pub fn run(ctx: &CliContext, ui: &Ui, args: crate::cli::UrnArgs) -> Result<(), CliError> {
    if args.paths.is_empty() && !args.all {
        return Err(CliError::InvalidArgument("nothing to preview: pass paths or -A".into()));
    }
    let previews = crate::ops::store_ops::preview_urns(ctx, &args.paths, args.all, args.root.as_deref())?;
    if ui.json() {
        ui.emit_json(&previews.iter().map(|p| serde_json::json!({
            "path": p.path, "key": p.key, "urn": p.urn, "retrieval_key": p.retrieval_key,
        })).collect::<Vec<_>>());
        return Ok(());
    }
    for p in &previews {
        ui.line(format!("{}\t{}", p.key, p.urn));
    }
    Ok(())
}
```
(Make `UrnPreview` fields/struct and `preview_urns` `pub`. Add `serde::Serialize` to `UrnPreview` if emitting it directly instead of `json!`.)

- [ ] **Step 3: Build + smoke test.** Run: `cargo build -p digstore-cli`. Expected: PASS.

- [ ] **Step 4: Commit.**
```bash
git add crates/digstore-cli/src/commands/urn.rs crates/digstore-cli/src/ops/store_ops.rs
git commit -m "feat(cli): `digstore urn [PATHS]` previews content-root-relative URNs"
```

---

## Phase E — Docs

### Task E1: Whitepaper + PDF

**Files:**
- Modify: `docs/whitepaper/digstore-whitepaper.md`
- Re-render via `docs/whitepaper/render.py`

- [ ] **Step 1: Edit §4.4** — add "Multiple stores per project": a `.dig/` workspace holds named stores under `stores/<name>/`; `workspace.toml` tracks the active store + name→id registry + per-store content root; selection precedence `--store` > active > single. Note each store keeps its own staging/generations/modules.
- [ ] **Step 2: Edit §5.1** — module memory: data section up to the 100 MB store cap; module memory ceiling 2048 pages (128 MiB); the guest heap is placed dynamically **above** the data section (`align_up(DIGS_DATA_OFFSET + blob_len, 64 KiB)`), so heap growth never corrupts the embedded pool for any blob size (contract D2).
- [ ] **Step 3: Edit §18.2** — host outer memory limit default 128 MiB, operator-configurable via `ExecutionLimits.memory_bytes_max`.
- [ ] **Step 4: Add a short "Content root" note** — digstore captures build output; a store's content root (default CWD, settable to e.g. `dist/`) is the directory `add`/`urn` scan and the root that resource keys are relative to; keys are stable regardless of invocation directory.
- [ ] **Step 5: Re-render the PDF.** Run: `python docs/whitepaper/render.py` (Chrome headless). Expected: PDF regenerated.
- [ ] **Step 6: Commit.**
```bash
git add docs/whitepaper
git commit -m "docs(whitepaper): multi-store workspaces, content roots, 128 MiB ceiling + dynamic heap"
```

### Task E2: SECURITY.md

**Files:**
- Modify: `SECURITY.md`

- [ ] **Step 1: Add a note** — per-store 100 MB stage cap (enforced at `add`, defensively at `commit`); the module memory ceiling is raised to a configurable 128 MiB outer bound; the cap bounds the worst-case module size so the ceiling cannot be exceeded by content.
- [ ] **Step 2: Commit.**
```bash
git add SECURITY.md
git commit -m "docs(security): note per-store cap + configurable 128 MiB memory bound"
```

---

## Phase F — Integration tests + final gate

### Task F1: Fix existing CLI test helpers + add multi-store integration tests

**Files:**
- Modify: `crates/digstore-cli/tests/common/mod.rs` (`dig`, `store_id_and_root`)
- Modify: `crates/digstore-cli/tests/cli_init.rs`, `cli_add.rs`, `cli_status.rs` (path expectations)
- Modify: `crates/digstore-host/tests/dighost_serve.rs`, `crates/digstore-cli/tests/adv_self_serve.rs` (resolution call sites)
- Create: `crates/digstore-cli/tests/cli_multistore.rs`

- [ ] **Step 1: Update `common/mod.rs`.** `dig(dir)` still passes `--dig-dir <dir>/.dig`-equivalent — but since `--dig-dir` is the workspace dir, point it at `dir.path().join(".dig")`. Update `store_id_and_root` to read `stores/default/config.toml` (or parse `digstore stores --json`). Concretely:
```rust
pub fn dig(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("digstore").unwrap();
    cmd.arg("--dig-dir").arg(dir.path().join(".dig"));
    cmd
}
```
```rust
pub fn store_id_and_root(dir: &TempDir) -> (String, String) {
    let out = dig(dir).args(["log", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let root = v[0]["root"].as_str().unwrap().to_string();
    let cfg = std::fs::read_to_string(dir.path().join(".dig/stores/default/config.toml")).unwrap();
    let line = cfg.lines().find(|l| l.contains("store_id")).unwrap();
    let store_id = line.split('"').nth(1).unwrap().to_string();
    (store_id, root)
}
```
(These tests previously ran against the raw TempDir as the store dir. With the workspace model, `init` with no name creates `stores/default`. Verify each `cli_*` test still passes; adjust path assertions that read `config.toml` / `.gitignore` to the new locations: `.gitignore` is still at the project root = `dir.path()`, `config.toml` is now under `.dig/stores/default/`.)

- [ ] **Step 2: Update `dighost_serve.rs` + `adv_self_serve.rs`** `build_fixture()`/test setup. They call `CliContext::resolve(Some(td.path()), ...)` then `init_store(&ctx, false, None)` and read `ctx.dig_dir.join("signing_key.bin")`. Replace with the new flow that produces a per-store ctx whose `dig_dir = <ws>/stores/default`:
```rust
    let ws_dir = td.path().join(".dig");
    std::fs::create_dir_all(ws_dir.join("stores/default")).unwrap();
    let ctx = CliContext::for_store(ws_dir.clone(), "default", None, td.path().to_path_buf(), false, false);
    store_ops::init_store(&ctx, false, None).unwrap();
    // ctx.dig_dir == <ws>/stores/default — signing_key.bin lives there, as before.
```
(Keep the rest of each test identical; only the context construction changes. `ctx.op_dir` defaults to the tempdir; files added there get keys relative to it, same as before.)

- [ ] **Step 3: Run the high-risk suites.** Run: `cargo test -p digstore-host --test dighost_serve` and `cargo test -p digstore-cli --test adv_self_serve` and `cargo test -p digstore-cli --test adv_host_no_inspect`. Expected: PASS.

- [ ] **Step 4: Write `cli_multistore.rs`** (assert_cmd). Cover, each as its own `#[test]`, using `dig(&tmp)`:
  - `init a` + `init b` create `stores/a` and `stores/b`; `stores --json` lists both; `*` on the first (active).
  - `use b` then `stores --json` shows b active.
  - `--store a add <file>` targets a regardless of active; `staged --store a --json` shows it, `staged --store b --json` is empty (isolation).
  - over-cap: write a file > 100 MB (sparse zero buffer) and assert `add` exits non-zero with "over the … 100.0 MB limit" and `staged --json` total stays 0.
  - `unstage` empties staging (`staged --json` → empty).
  - `staged --json` reports `total_bytes` + `limit_bytes`.
  - content root: `init site --dir dist`; create `dist/css/app.css`; `--store site add -A` from the project root; `urn --store site css/app.css --json` shows key `css/app.css`; the same `urn` run with `-C dist` from elsewhere yields the identical URN.
  - migration: build a legacy `.dig/config.toml` layout by hand (init then move files up, or craft minimal), run any command, assert `stores --json` shows `default` and `.dig/stores/default/config.toml` exists.

  Example skeleton for one case:
```rust
#[test]
fn two_stores_list_and_switch() {
    let tmp = TempDir::new().unwrap();
    dig(&tmp).args(["init", "a"]).assert().success();
    dig(&tmp).args(["init", "b"]).assert().success();
    let out = dig(&tmp).args(["stores", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = v.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"a") && names.contains(&"b"));
    dig(&tmp).args(["use", "b"]).assert().success();
    let out = dig(&tmp).args(["stores", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let active = v.as_array().unwrap().iter().find(|r| r["active"] == true).unwrap();
    assert_eq!(active["name"], "b");
}
```

- [ ] **Step 5: Run.** Run: `cargo test -p digstore-cli`. Expected: PASS.

- [ ] **Step 6: Commit.**
```bash
git add crates/digstore-cli/tests crates/digstore-host/tests/dighost_serve.rs
git commit -m "test(cli): multi-store/cap/unstage/staged/content-root/urn/migration integration coverage"
```

### Task F2: Full gate

- [ ] **Step 1: Rebuild guest wasm** (final): `cargo build -p digstore-guest --target wasm32-unknown-unknown --release --locked`.
- [ ] **Step 2: Format.** Run: `cargo fmt --all`. Then `cargo fmt --all --check`. Expected: clean.
- [ ] **Step 3: Clippy (pinned).** Run: `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::default_constructed_unit_structs -A clippy::field_reassign_with_default`. Expected: no warnings. Fix any.
- [ ] **Step 4: Tests.** Run: `cargo test --workspace --locked`. Expected: PASS.
- [ ] **Step 5: Supply chain.** Run: `cargo deny check advisories bans sources`. Expected: PASS.
- [ ] **Step 6: Commit any fmt/clippy fixups.**
```bash
git add -A
git commit -m "chore: fmt + clippy clean across multi-store work"
```

---

## Self-Review notes (author)

- **Spec coverage:** §2.1–2.8 → B1–B7, C1, C5; §3 cap → C2/C6; §3.1 capacity → C3/C4/C5/B5/D2; §4 memory → A1–A4; §5 staging → D1/D2; §6 URN → D3/D4; §7 testing → F1/F2 + per-task tests; §8 out-of-scope respected (no sharding, no cross-store URNs, no per-path unstage).
- **Deliberate refinement vs spec:** `content_root` lives in `workspace.toml`, not `StoreConfig` (the store/crypto layer has no need for it) — keeps `digstore-core`/`digstore-store` untouched and matches the approved crate scope. `inject.rs` `CEILING_PAGES` is unified into `template::MAX_MEMORY_PAGES` (removes a documented drift). The golden hex is **not** regenerated (it does not encode the ceiling).
- **Known risk (flagged, not deferred):** 128 MiB ceiling headroom for a near-cap store is validated by the A4 `#[ignore]` stress test; if it OOMs, raise `template::MAX_MEMORY_PAGES` (+ host `MAX_MEMORY_BYTES`) — the single derived knob.
- **Type consistency:** `AddOutcome`/`StatusView` gain `staged_bytes`/`limit_bytes` (both u64); `Ui::capacity(staged,limit)` + `human_capacity(staged,limit)` consistent everywhere; `Workspace`/`CliContext` field names match across tasks.
