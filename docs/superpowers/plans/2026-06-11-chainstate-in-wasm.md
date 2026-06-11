# Chainstate-in-WASM Implementation Plan (Phase A: embed + read)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Embed a store's on-chain anchor pointer (network, launcher id, current coin id, height, tx id, coinset-url hint) into the compiled `.dig` WASM module's data section, and provide a reader so any app can extract the chain pointer from the module bytes alone.

**Architecture:** Add a new backward-compatible `SectionId::ChainState` to `digstore-core`'s data-section format with a `ChainState` struct + codec + `read_chain_state` reader. Thread an `Option<ChainState>` through `digstore-compiler` (emit it, and preserve it across a trusted-key rekey). The CLI builds the `ChainState` at `commit` finalize (post-confirmation, when the launcher/coin/height are known) and surfaces it via `anchor status` + a new `anchor inspect <module>`.

**Tech Stack:** Rust. `digstore-core` (datasection codec), `digstore-compiler` (pipeline/data_section), `digstore-cli` (commit + anchor commands). Spec: `docs/superpowers/specs/2026-06-11-chainstate-in-wasm-design.md`.

**Scope note:** This plan is **Phase A only** (the self-describing locator). **Phase B** (make `clone`/`pull` verify the served root against the on-chain singleton, closing SECURITY.md residual #6) is a sequenced follow-up plan, authored after Phase A lands — its exact shape depends on this plan's reader API plus a close read of `digstore-cli/src/ops/remote_ops.rs` and the mock-chain test seam.

**Conventions (all tasks):** TDD. Commits are conventional, SSH-signed, **NO `Co-Authored-By` trailer**. If a build panics about a missing `digstore_guest.wasm`, run `cargo build -p digstore-guest --target wasm32-unknown-unknown --release` first. Never touch mainnet / `.testcredentials`; CLI tests use the `DIGSTORE_ANCHOR_MOCK` seam.

---

## File structure

- `crates/digstore-core/src/datasection.rs` — **modify**: add `SectionId::ChainState = 12`, the `ChainState` struct + `encode`/`decode`, and `read_chain_state`. One responsibility: the in-module chainstate format.
- `crates/digstore-compiler/src/data_section.rs` — **modify**: `DataSectionInputs.chain_state: Option<ChainState>`; emit it in `encode_data_section` (before Filler); preserve it in `rekey_module_trusted`.
- `crates/digstore-compiler/src/pipeline.rs` — **modify**: `Compiler::compile` gains a `chain_state: Option<ChainState>` param, set on `DataSectionInputs`.
- `crates/digstore-cli/src/ops/store_ops.rs` — **modify**: `finalize_commit`/`compile_module` build + pass the `ChainState`.
- `crates/digstore-cli/src/commands/anchor.rs` — **modify**: `anchor status` reads the module's embedded `ChainState`; add the `inspect` sub-action.
- `crates/digstore-cli/src/cli.rs` — **modify**: `AnchorAction::Inspect { module: PathBuf }`.

---

## Task A1: `ChainState` type, section id, codec, and reader (digstore-core)

**Files:**
- Modify: `crates/digstore-core/src/datasection.rs`
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Add the section id.** In the `SectionId` enum add a variant AFTER `Filler`:

```rust
pub enum SectionId {
    StoreId = 1,
    CurrentRoot = 2,
    RootHistory = 3,
    PublicKey = 4,
    TrustedKeys = 5,
    Metadata = 6,
    AuthInfo = 7,
    KeyTable = 8,
    ChunkPool = 9,
    MerkleNodes = 10,
    Filler = 11,
    ChainState = 12,
}
```

- [ ] **Step 2: Write the failing codec round-trip test.** Add to the test module:

```rust
#[test]
fn chain_state_round_trips() {
    let cs = ChainState {
        version: 1,
        network: "mainnet".to_string(),
        launcher_id: Bytes32([0xAB; 32]),
        coin_id: Bytes32([0xCD; 32]),
        confirmed_height: 8_854_632,
        tx_id: "deadbeef".to_string(),
        coinset_url: "https://api.coinset.org".to_string(),
    };
    let bytes = cs.encode();
    let back = ChainState::decode(&bytes).expect("decode");
    assert_eq!(back, cs);
}

#[test]
fn chain_state_decode_rejects_truncated() {
    assert!(ChainState::decode(&[1u8, 0, 0]).is_err());
}

#[test]
fn read_chain_state_absent_is_none() {
    // A blob with one unrelated section and no ChainState.
    let blob = encode_blob(&[(SectionId::StoreId as u16, vec![7u8; 32])]);
    assert!(read_chain_state(&blob).unwrap().is_none());
}

#[test]
fn read_chain_state_present_round_trips() {
    let cs = ChainState {
        version: 1,
        network: "mainnet".into(),
        launcher_id: Bytes32([1; 32]),
        coin_id: Bytes32([2; 32]),
        confirmed_height: 42,
        tx_id: String::new(),
        coinset_url: "https://api.coinset.org".into(),
    };
    let blob = encode_blob(&[
        (SectionId::StoreId as u16, cs.launcher_id.0.to_vec()),
        (SectionId::ChainState as u16, cs.encode()),
    ]);
    assert_eq!(read_chain_state(&blob).unwrap().unwrap(), cs);
}
```

- [ ] **Step 3: Run the tests to verify they fail.**

Run: `cargo test -p digstore-core datasection::tests::chain_state -- --nocapture`
Expected: FAIL to compile (`ChainState` / `read_chain_state` not found).

- [ ] **Step 4: Implement `ChainState` + codec + reader.** Add near `SectionId` (the codec is a self-contained explicit byte layout: `version(u8) | network(str) | launcher_id(32) | coin_id(32) | confirmed_height(u32 BE) | tx_id(str) | coinset_url(str)`, where `str = len(u32 BE) || utf8 bytes`):

```rust
/// On-chain anchor pointer embedded in a compiled module's data section
/// (`SectionId::ChainState`). Lets any reader locate the store's singleton on
/// Chia from the module bytes alone. `coinset_url` is a transport HINT only —
/// callers override it with local config; it can go stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainState {
    pub version: u8,
    pub network: String,
    pub launcher_id: Bytes32,
    pub coin_id: Bytes32,
    pub confirmed_height: u32,
    pub tx_id: String,
    pub coinset_url: String,
}

impl ChainState {
    /// Current encoding version.
    pub const VERSION: u8 = 1;

    pub fn encode(&self) -> Vec<u8> {
        fn put_str(out: &mut Vec<u8>, s: &str) {
            out.extend_from_slice(&(s.len() as u32).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        let mut out = Vec::new();
        out.push(self.version);
        put_str(&mut out, &self.network);
        out.extend_from_slice(&self.launcher_id.0);
        out.extend_from_slice(&self.coin_id.0);
        out.extend_from_slice(&self.confirmed_height.to_be_bytes());
        put_str(&mut out, &self.tx_id);
        put_str(&mut out, &self.coinset_url);
        out
    }

    pub fn decode(buf: &[u8]) -> Result<ChainState, DecodeError> {
        struct R<'a> { b: &'a [u8], pos: usize }
        impl<'a> R<'a> {
            fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
                let end = self.pos.checked_add(n).ok_or(DecodeError::UnexpectedEof)?;
                if end > self.b.len() { return Err(DecodeError::UnexpectedEof); }
                let s = &self.b[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            fn u8(&mut self) -> Result<u8, DecodeError> { Ok(self.take(1)?[0]) }
            fn u32(&mut self) -> Result<u32, DecodeError> {
                let s = self.take(4)?;
                Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
            }
            fn b32(&mut self) -> Result<Bytes32, DecodeError> {
                let s = self.take(32)?;
                let mut a = [0u8; 32]; a.copy_from_slice(s); Ok(Bytes32(a))
            }
            fn s(&mut self) -> Result<String, DecodeError> {
                let n = self.u32()? as usize;
                let s = self.take(n)?;
                String::from_utf8(s.to_vec()).map_err(|_| DecodeError::Invalid("ChainState: bad utf8"))
            }
        }
        let mut r = R { b: buf, pos: 0 };
        let version = r.u8()?;
        let network = r.s()?;
        let launcher_id = r.b32()?;
        let coin_id = r.b32()?;
        let confirmed_height = r.u32()?;
        let tx_id = r.s()?;
        let coinset_url = r.s()?;
        Ok(ChainState { version, network, launcher_id, coin_id, confirmed_height, tx_id, coinset_url })
    }
}

/// Decode the embedded `ChainState` from a module data-section blob, if present.
/// Returns `Ok(None)` for older modules that carry no `ChainState` section.
pub fn read_chain_state(blob: &[u8]) -> Result<Option<ChainState>, DecodeError> {
    let view = DataView::parse(blob)?;
    match view.section(SectionId::ChainState) {
        Some(body) => Ok(Some(ChainState::decode(body)?)),
        None => Ok(None),
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `cargo test -p digstore-core datasection -- --nocapture`
Expected: PASS (all four new tests + existing ones).

- [ ] **Step 6: Export.** Ensure `ChainState` and `read_chain_state` are reachable from `digstore_core` (the `datasection` module is public; add `pub use` re-exports in `digstore-core/src/lib.rs` if the crate re-exports other datasection items — match the existing pattern). Run `cargo build -p digstore-core`.

- [ ] **Step 7: Commit.**

```bash
git add crates/digstore-core/src/datasection.rs crates/digstore-core/src/lib.rs
git commit -m "feat(core): ChainState data section + read_chain_state"
```

---

## Task A2: compiler emits + preserves `ChainState`

**Files:**
- Modify: `crates/digstore-compiler/src/data_section.rs` (`DataSectionInputs`, `encode_data_section`, `rekey_module_trusted`)
- Modify: `crates/digstore-compiler/src/pipeline.rs` (`Compiler::compile` signature)
- Test: `crates/digstore-compiler/src/data_section.rs` test module

- [ ] **Step 1: Write the failing emit test.** In the `data_section.rs` test module (the `inputs()` helper builds a `DataSectionInputs`):

```rust
#[test]
fn encode_emits_chain_state_when_present_and_filler_stays_last() {
    use digstore_core::datasection::{read_chain_state, ChainState, DataView, SectionId};
    let mut inp = inputs();
    let cs = ChainState {
        version: 1,
        network: "mainnet".into(),
        launcher_id: inp.store_id,
        coin_id: digstore_core::Bytes32([9u8; 32]),
        confirmed_height: 1234,
        tx_id: String::new(),
        coinset_url: "https://api.coinset.org".into(),
    };
    inp.chain_state = Some(cs.clone());
    let blob = encode_data_section(&inp);
    // ChainState round-trips out of the blob.
    assert_eq!(read_chain_state(&blob).unwrap().unwrap(), cs);
    // Filler remains the trailing section (highest offset), so uniform-size
    // padding still works.
    let view = DataView::parse(&blob).unwrap();
    let filler_off = view_section_offset(&view, SectionId::Filler);
    let chain_off = view_section_offset(&view, SectionId::ChainState);
    assert!(filler_off > chain_off, "Filler must be encoded after ChainState");
}

#[test]
fn encode_without_chain_state_has_no_section() {
    use digstore_core::datasection::read_chain_state;
    let inp = inputs(); // chain_state defaults to None
    let blob = encode_data_section(&inp);
    assert!(read_chain_state(&blob).unwrap().is_none());
}
```

Add this small test helper to the test module (offsets are needed to assert ordering; `DataView` exposes `section` bodies — derive offsets by pointer arithmetic against the blob):

```rust
fn view_section_offset(view: &digstore_core::datasection::DataView, id: digstore_core::datasection::SectionId) -> usize {
    let body = view.section(id).expect("section present");
    // body is a slice into the same blob; its start offset is its distance
    // from the blob start. DataView exposes the raw blob via `total_len`, but
    // not the base ptr; instead assert ordering by re-parsing offsets:
    // simplest: compare the section bodies' as_ptr() — both point into `blob`.
    body.as_ptr() as usize
}
```

- [ ] **Step 2: Run to verify failure.**

Run: `cargo test -p digstore-compiler data_section::tests::encode_emits_chain_state -- --nocapture`
Expected: FAIL to compile (`inp.chain_state` field doesn't exist).

- [ ] **Step 3: Add the field + default.** In `DataSectionInputs` add:

```rust
    /// Optional on-chain anchor pointer embedded as `SectionId::ChainState`.
    pub chain_state: Option<digstore_core::datasection::ChainState>,
```

Update the test `inputs()` helper to set `chain_state: None`. Update any OTHER construction site of `DataSectionInputs` (search the crate) to set `chain_state: None`.

- [ ] **Step 4: Emit it in `encode_data_section`.** In `encode_data_section`, after the existing sections are pushed and BEFORE the `Filler` section is appended, insert:

```rust
    if let Some(cs) = &i.chain_state {
        sections.push((SectionId::ChainState as u16, cs.encode()));
    }
```

(Locate the exact point: the `Filler` entry must be pushed last. If filler is appended in `pipeline.rs` rather than here, push ChainState in `encode_data_section` before returning and ensure pipeline still appends filler after — verify Filler ends up last by the test above.)

- [ ] **Step 5: Preserve it across rekey.** In `rekey_module_trusted` (the trusted-key swap that rebuilds the blob over a fixed `IDS` list), make `ChainState` an OPTIONAL passthrough: when rebuilding, if the source `view.section(SectionId::ChainState)` is `Some(body)`, include `(SectionId::ChainState as u16, body.to_vec())` in the rebuilt sections (before Filler); if `None`, omit it. Do NOT add `ChainState` to a fixed required-id array that would error when absent — keep it conditional so older modules still rekey.

- [ ] **Step 6: Thread through `Compiler::compile`.** In `pipeline.rs`, add a parameter to `compile`:

```rust
    pub fn compile<G: GenerationView>(
        config: &CompilerConfig,
        store_id: Bytes32,
        store_pubkey: Bytes48,
        generations: &[G],
        manifest: MetadataManifest,
        auth_info: AuthenticationInfo,
        trusted_keys: &[TrustedHostKey],
        chain_state: Option<digstore_core::datasection::ChainState>,
    ) -> Result<...> {
```

and set `chain_state` on the `DataSectionInputs { ... }` literal (replace the implicit default). Update every caller of `Compiler::compile` IN THIS CRATE (tests, examples) to pass `None`.

- [ ] **Step 7: Run the tests.**

Run: `cargo test -p digstore-compiler -- --nocapture`
Expected: PASS (new tests + existing; existing pass `None`).

- [ ] **Step 8: Run clippy + commit.**

Run: `cargo clippy -p digstore-compiler --all-targets`
Expected: clean.

```bash
git add crates/digstore-compiler/src/data_section.rs crates/digstore-compiler/src/pipeline.rs
git commit -m "feat(compiler): emit + preserve ChainState data section"
```

---

## Task A3: CLI embeds `ChainState` at commit finalize

**Files:**
- Modify: `crates/digstore-cli/src/ops/store_ops.rs` (`compile_module`, `finalize_commit`, and the `PreparedCommit`/commit path so the chain pointer is available)
- Modify: `crates/digstore-cli/src/commands/commit.rs` (pass the confirmed chain info into finalize)
- Test: `crates/digstore-cli/tests/cli_commit_log.rs`

> Context: `commands/commit.rs::run` already, on a confirmed anchor, has `launcher_id` (= store_id), the confirmed `coin_id`, the `ConfirmState::Confirmed { height }`, and `GlobalConfig` (for `coinset_url`). `finalize_commit(ctx, prepared)` then compiles the module via `compile_module`. The chain pointer must reach `compile_module`.

- [ ] **Step 1: Write the failing integration test.** In `tests/cli_commit_log.rs` (uses the `common::dig` seeded-mock seam):

```rust
#[test]
fn commit_embeds_chain_state_in_module() {
    let dir = common::tmp_dig();
    common::dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    common::dig(&dir).args(["add", "a.txt"]).assert().success();
    let out = common::dig(&dir).args(["--json", "commit", "-m", "x"]).output().unwrap();
    assert!(out.status.success());
    // Find the compiled module and decode its ChainState.
    let modules = common::store_dir(&dir).join("modules");
    let module = std::fs::read_dir(&modules).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "dig").unwrap_or(false))
        .expect("a .dig module");
    let bytes = std::fs::read(&module).unwrap();
    // Extract the data section the same way the compiler/verifier does, then read ChainState.
    let cs = digstore_cli::ops::store_ops::read_module_chain_state(&bytes)
        .expect("read")
        .expect("module carries ChainState");
    assert_eq!(cs.network, "mainnet");
    // store_id (launcher) is embedded and non-zero.
    assert_ne!(cs.launcher_id, digstore_core::Bytes32([0u8; 32]));
}
```

> Note: reading a module's data section needs the same `extract_data_section(module, DATA_SECTION_MEM_OFFSET)` the compiler uses. Expose a CLI helper `read_module_chain_state(module_bytes) -> Result<Option<ChainState>, CliError>` (Step 3) so tests and the `anchor` command share one path. If `digstore_compiler` does not publicly expose `extract_data_section` + `DATA_SECTION_MEM_OFFSET`, add a thin public wrapper to the compiler, e.g. `digstore_compiler::extract_data_section_blob(module) -> Result<Vec<u8>, CompilerError>`, and use it here.

- [ ] **Step 2: Run to verify failure.**

Run: `cargo test -p digstore-cli --test cli_commit_log commit_embeds_chain_state -- --nocapture`
Expected: FAIL (`read_module_chain_state` not found / module has no ChainState).

- [ ] **Step 3: Add the module reader helper.** In `store_ops.rs`:

```rust
/// Decode the embedded on-chain pointer from a compiled module's bytes, if any.
pub fn read_module_chain_state(
    module: &[u8],
) -> Result<Option<digstore_core::datasection::ChainState>, CliError> {
    let blob = digstore_compiler::extract_data_section_blob(module)
        .map_err(|e| CliError::Other(anyhow::anyhow!("extract data section: {e:?}")))?;
    digstore_core::datasection::read_chain_state(&blob)
        .map_err(|e| CliError::Other(anyhow::anyhow!("decode chain state: {e:?}")))
}
```

If `extract_data_section_blob` is not yet public in the compiler, add it there (a 3-line wrapper over the existing private `extract_data_section(module, DATA_SECTION_MEM_OFFSET)`), export from `digstore_compiler::lib`, then build.

- [ ] **Step 4: Build + pass the `ChainState` into compile.** Change `compile_module` to accept `chain_state: Option<ChainState>` and pass it as the new `Compiler::compile(..., chain_state)` argument. Build the `ChainState` in `commands/commit.rs` AFTER confirmation (where `launcher_id`, `coin_id`, `height`, and `GlobalConfig.coinset_url` are in scope) and thread it into `finalize_commit` → `compile_module`. Concretely:
  - Add a field to `PreparedCommit`? No — the chain info is known in `commit.rs`, not in `stage_to_root`. Instead give `finalize_commit` a `chain_state: Option<ChainState>` parameter and have `commit.rs` build it on the `Confirmed` arm:

```rust
// in commands/commit.rs, on ConfirmState::Confirmed { height }:
let cs = digstore_core::datasection::ChainState {
    version: digstore_core::datasection::ChainState::VERSION,
    network: "mainnet".to_string(),
    launcher_id, // chia Bytes32 -> core Bytes32 conversion as elsewhere (copy_from_slice)
    coin_id,     // same conversion
    confirmed_height: height,
    tx_id: state.last_tx_id.clone(), // best-effort; may be empty
    coinset_url: gcfg.coinset_url.clone(),
};
let outcome = store_ops::finalize_commit(ctx, prepared, Some(cs))?;
```

  Convert the chia `Bytes32` ids to `digstore_core::Bytes32` with the existing `copy_from_slice` idiom. Ensure `gcfg` (GlobalConfig) and `state` (AnchorState) are in scope at that point (they are — keys/gcfg from `prepare_anchor`, `state` loaded earlier).
  - `finalize_commit` passes `chain_state` to `compile_module`. The back-compat `store_ops::commit(ctx, msg)` wrapper calls `finalize_commit(ctx, prepared, None)`.

- [ ] **Step 5: Run the test.**

Run: `cargo test -p digstore-cli --test cli_commit_log commit_embeds_chain_state -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Full suite + clippy.**

Run: `cargo test -p digstore-cli` and `cargo clippy -p digstore-cli --all-targets`
Expected: all green (the back-compat `commit` wrapper keeps `ops_roundtrip`/`adv_*` callers working with `None`).

- [ ] **Step 7: Commit.**

```bash
git add crates/digstore-cli/src/ops/store_ops.rs crates/digstore-cli/src/commands/commit.rs crates/digstore-compiler/src/lib.rs crates/digstore-compiler/src/data_section.rs
git commit -m "feat(cli): embed ChainState in the module at commit finalize"
```

---

## Task A4: `anchor status` reads the module; new `anchor inspect <module>`

**Files:**
- Modify: `crates/digstore-cli/src/cli.rs` (`AnchorAction::Inspect`)
- Modify: `crates/digstore-cli/src/commands/anchor.rs`
- Test: `crates/digstore-cli/tests/cli_anchor.rs`

- [ ] **Step 1: Add the clap surface + parse test.** In `cli.rs`, extend `AnchorAction`:

```rust
#[derive(Debug, Subcommand)]
pub enum AnchorAction {
    /// Query the active store's on-chain anchor state.
    Status,
    /// Decode and print the embedded chain pointer of a module file.
    Inspect {
        /// Path to a compiled `.dig` module.
        module: std::path::PathBuf,
    },
}
```

Add a parse test:

```rust
#[test]
fn parses_anchor_inspect() {
    let cli = Cli::try_parse_from(["digstore", "anchor", "inspect", "x.dig"]).unwrap();
    match cli.command {
        Command::Anchor(a) => match a.action {
            Some(AnchorAction::Inspect { module }) => assert_eq!(module.to_str().unwrap(), "x.dig"),
            _ => panic!("expected inspect"),
        },
        _ => panic!("expected anchor"),
    }
}
```

- [ ] **Step 2: Write the failing behavior tests.** In `tests/cli_anchor.rs`:

```rust
#[test]
fn anchor_status_shows_module_chain_pointer() {
    let dir = common::tmp_dig();
    common::dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
    common::dig(&dir).args(["add", "a.txt"]).assert().success();
    common::dig(&dir).args(["commit", "-m", "x"]).assert().success();
    let out = common::dig(&dir).args(["--json", "anchor", "status"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // The module's embedded pointer is surfaced alongside anchor.toml state.
    assert_eq!(v["module_chain_state"]["network"].as_str(), Some("mainnet"));
    assert!(v["module_chain_state"]["coin_id"].as_str().is_some());
}

#[test]
fn anchor_inspect_dumps_a_module_pointer() {
    let dir = common::tmp_dig();
    common::dig(&dir).arg("init").assert().success();
    std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
    common::dig(&dir).args(["add", "a.txt"]).assert().success();
    common::dig(&dir).args(["commit", "-m", "x"]).assert().success();
    // Locate the compiled module.
    let modules = common::store_dir(&dir).join("modules");
    let module = std::fs::read_dir(&modules).unwrap().filter_map(|e| e.ok())
        .map(|e| e.path()).find(|p| p.extension().map(|x| x == "dig").unwrap_or(false)).unwrap();
    let out = common::dig(&dir).args(["--json", "anchor", "inspect", module.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["network"].as_str(), Some("mainnet"));
    assert!(v["launcher_id"].as_str().is_some());
}
```

- [ ] **Step 3: Run to verify failure.**

Run: `cargo test -p digstore-cli --test cli_anchor anchor_status_shows_module -- --nocapture`
Expected: FAIL (no `module_chain_state` field / `inspect` unimplemented).

- [ ] **Step 4: Implement.** In `commands/anchor.rs`:
  - In the `Status` path: after building the existing status object, load the current module via `store_ops::module_path_for(ctx, &store_id, None)` (best-effort; a store with no commit yet has no module → emit `module_chain_state: null`), read it with `store_ops::read_module_chain_state(&bytes)?`, and add a `module_chain_state` field to BOTH the human output (a few lines: network/launcher/coin/height) and the `--json` object (an object with `network, launcher_id, coin_id, confirmed_height, tx_id, coinset_url`, or `null` when absent).
  - Add the `Inspect { module }` arm: read the file, `read_module_chain_state(&bytes)?`; if `None` → `CliError::NotFound("module carries no chain state".into())`; else print human (network/launcher/coin/height/tx/url) or `--json` (the same object). This arm does NOT require the active store's anchor.toml — it works on any module path (but it is dispatched as a store-scoped command, which is fine; it ignores the store context except for `ui`).

  Helper to serialize a `ChainState` to JSON (shared by status + inspect):

```rust
fn chain_state_json(cs: &digstore_core::datasection::ChainState) -> serde_json::Value {
    serde_json::json!({
        "network": cs.network,
        "launcher_id": cs.launcher_id.to_hex(),
        "coin_id": cs.coin_id.to_hex(),
        "confirmed_height": cs.confirmed_height,
        "tx_id": cs.tx_id,
        "coinset_url": cs.coinset_url,
    })
}
```

- [ ] **Step 5: Run the tests.**

Run: `cargo test -p digstore-cli --test cli_anchor` and the cli.rs parse test
Expected: PASS (all anchor tests, including the prior ones).

- [ ] **Step 6: Full suite + clippy + fmt.**

Run: `cargo test -p digstore-cli`, `cargo clippy -p digstore-cli --all-targets`, `cargo fmt`
Expected: green + clean + formatted.

- [ ] **Step 7: Commit.**

```bash
git add crates/digstore-cli/src/cli.rs crates/digstore-cli/src/commands/anchor.rs crates/digstore-cli/tests/cli_anchor.rs
git commit -m "feat(cli): anchor status reads module chain pointer; anchor inspect <module>"
```

---

## Task A5: workspace verification + docs

**Files:**
- Modify: `README.md` (one paragraph), `docs/superpowers/specs/2026-06-11-chainstate-in-wasm-design.md` (mark Phase A done)

- [ ] **Step 1: Full workspace verification.**

Run: `cargo build --workspace`
Run: `cargo test --workspace` — Expected: green (gated `e2e_mainnet` shows ignored, never run).
Run: `cargo clippy --workspace --all-targets` — Expected: clean.
Run: `cargo fmt --check` — Expected: clean (run `cargo fmt` if not).

- [ ] **Step 2: README note.** In the README's on-chain anchoring section, add one short paragraph: the compiled module embeds its on-chain pointer (network + launcher/store id + current coin + height + coinset hint); `digstore anchor status` shows it and `digstore anchor inspect <module.dig>` dumps any module's pointer; the embedded coinset URL is a hint that local config overrides.

- [ ] **Step 3: Mark Phase A done in the spec** and note Phase B (chain-verified clone/pull) is the next plan.

- [ ] **Step 4: Commit.**

```bash
git add README.md docs/superpowers/specs/2026-06-11-chainstate-in-wasm-design.md
git commit -m "docs: README + spec note for embedded module chain pointer (Phase A)"
```

---

## Self-review (against the spec)

- **Spec §"Phase A — embed + read"**: core section (A1) ✔, compiler emit+preserve (A2) ✔, CLI commit embed (A3) ✔, reader/`anchor status`/`anchor inspect` (A4) ✔, backward-compat (A1/A2 tests assert no-ChainState modules parse/verify) ✔.
- **Fields**: network, launcher_id, coin_id, confirmed_height, tx_id, coinset_url all in `ChainState` (A1) ✔. `tx_id` best-effort/empty (A3 Step 4) ✔. coinset_url hint, config-overridable — embedding only; live calls keep using config (Phase B concern; noted) ✔.
- **Snapshot semantics**: embedded at commit finalize, per generation (A3) ✔.
- **`anchor.toml` unchanged** ✔ (no task modifies its schema).
- **Phase B** explicitly deferred to a follow-up plan ✔.
- **Placeholder scan**: every code step has concrete code; the one judgement point (exact insertion site for the Filler-last ordering, and whether `extract_data_section` needs a public wrapper) is specified with a fallback action, not a TODO.
- **Type consistency**: `ChainState` fields identical across A1/A2/A3/A4; `read_chain_state` (core) vs `read_module_chain_state` (CLI, operates on module bytes) are distinct, intentional names; `Compiler::compile` gains exactly one `chain_state: Option<ChainState>` param used consistently in A2/A3.
