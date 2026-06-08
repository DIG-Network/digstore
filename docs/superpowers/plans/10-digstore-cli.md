# digstore-cli Implementation Plan

> **For agentic workers** — REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`. Execute this plan as a sequence of bite-sized TDD steps. Each checkbox is ONE 2–5 minute action. Do them strictly in order: write the failing test, run it and confirm the FAIL message, write the minimal implementation, run it and confirm the PASS, then commit. Never batch steps. Never collapse the red and green phases into one bullet. Never skip a red phase. Every type and function this crate calls from another crate is a fixed contract (see `depends_on` and the canonical catalog); this plan pins the exact expected signatures of those contracts. Do not invent alternative names and do not relitigate locked decisions.

**Goal:** Build the `digstore` binary — a git-style CLI (`init`, `add`, `commit`, `status`, `log`, `diff`, `checkout`, `cat`, `remote`, `clone`, `push`, `pull`) that orchestrates the store, compiler, host, and remote crates and performs all client-side decryption and merkle/proof verification.

**Architecture:** `digstore-cli` is a thin orchestration + presentation layer over the other crates. Commands map to `clap` subcommands; each subcommand calls a pure `ops::*` function (so the logic is unit-testable without spawning a process) which composes `digstore-store` (entity/generations/staging), `digstore-compiler` (module build at commit), `digstore-host` (a wasmtime instance to call `get_content`/`get_proof`), `digstore-remote` (clone/push/pull), and `digstore-crypto` (HKDF + AES-256-GCM client-side decrypt, merkle verify). The binary owns only argument parsing, exit codes, and human-readable output; all verification (per-chunk GCM tag, merkle-to-trusted-root §9.3, optional execution proof) happens client-side here, never in the module.

**Tech Stack:** Rust 2021 (pinned toolchain), `clap` v4 (derive), `anyhow` + `thiserror` for errors, `tokio` (current-thread) for the async `digstore-remote` client calls, `serde`/`serde_json` for config & manifest display, `tracing` + `tracing-subscriber` for `--verbose` logging. Tests use `assert_cmd` + `predicates` (end-to-end process tests) and `tempfile` (isolated `--dig-dir`).

> **Cross-crate contract pins (read before starting).** This crate does NOT define the following; they are owned by the named dependency crate and are referenced here by their exact, fixed signatures. If any signature below is not yet present in the dependency when you reach the step, STOP — the dependency plan is wrong, not this one.
>
> **`digstore-core`** (canonical types, single source of truth):
> - `Bytes32([u8;32])`, `Bytes48([u8;48])`, `Bytes96([u8;96])`, each `#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]` with `fn to_hex(&self) -> String` and `fn from_hex(s: &str) -> Result<Self, digstore_core::HexError>`.
> - `Urn { chain: String, store_id: Bytes32, root_hash: Option<Bytes32>, resource_key: Option<String> }` with `fn canonical(&self) -> String`, `fn retrieval_key(&self) -> Bytes32`, and `fn parse(s: &str) -> Result<Urn, digstore_core::UrnError>`.
> - `TrustedHostKey { public_key: [u8;48], label: String }` (ONE canonical definition; every crate re-uses THIS — never a per-crate copy). `#[derive(serde::Serialize, serde::Deserialize)]`.
> - `MerkleProof { leaf: Bytes32, path: Vec<ProofStep>, root: Bytes32 }`, `ProofStep { hash: Bytes32, is_left: bool }`.
> - `ContentRequest { retrieval_key: Bytes32, root: Option<Bytes32>, range: Option<(u64,u64)>, validity_window: Option<(u64,u64)> }` — the canonical `get_content`/`get_proof` request body (codec-encoded). Owned by digstore-core; the bare-retrieval-key wire format is NOT used.
> - `ContentResponse { ciphertext: Vec<u8>, merkle_proof: MerkleProof, roothash: Bytes32, chunks: Vec<ContentChunk> }`, `ContentChunk { ciphertext: Vec<u8>, proof: MerkleProof }` — `chunks` carries the per-chunk ciphertext+proof list (gather+concat is reconstructed client-side); `ciphertext` is the ordered concatenation of `chunks[*].ciphertext` for callers that do not need per-chunk verification. Decoy uses the same shape.
> - `ProofResponse { proof: ExecutionProof, roothash: Bytes32 }`, `ExecutionProof { program_hash: Bytes32, public_input: Vec<u8>, public_output: Bytes32, proof: Vec<u8>, chia_block: ChiaBlockRef, node_pubkey: Bytes48, node_signature: Bytes96 }`.
> - `GenerationState { id: u64, root: Bytes32, timestamp: u64 }` `#[derive(serde::Serialize, serde::Deserialize)]`.
> - `ErrorCode` enum (catalog values); `pack_ptr_len`/`unpack_ptr_len`/`is_error`.
> - Codec module `digstore_core::codec` exposing trait `Encode { fn encode(&self, out: &mut Vec<u8>); fn encode_to_vec(&self) -> Vec<u8> }` and trait `Decode: Sized { fn decode(input: &[u8]) -> Result<Self, digstore_core::codec::CodecError>; fn decode_from_slice(input: &[u8]) -> Result<Self, digstore_core::codec::CodecError> { Self::decode(input) } }`, implemented for all wire structs above. (Big-endian Chia framing — documented deviation #1.)
> - Canonical KDF constants (defined HERE so the compiler that ENCRYPTS and this CLI that DECRYPTS provably agree): `pub const HKDF_INFO: &[u8] = b"digstore-content-key-v1";` and `pub const HKDF_SALT_BASE: &[u8] = b"digstore-public-v1";`.
>
> **`digstore-crypto`** (host-side crypto; exact fns):
> - `fn sha256(data: &[u8]) -> [u8;32]`.
> - `fn hkdf_sha256_32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8;32]`.
> - `fn aes256gcm_seal(key: &[u8;32], plaintext: &[u8]) -> Vec<u8>` (fixed nonce — deviation #4).
> - `fn aes256gcm_open(key: &[u8;32], ciphertext: &[u8]) -> Result<Vec<u8>, digstore_crypto::AeadError>`.
> - Host-side BLS (chia-bls/blst, AugScheme), in module `digstore_crypto::host_bls`:
>   - `struct SecretKeyBytes(pub [u8;32])`.
>   - `fn keygen_os_rng() -> (SecretKeyBytes, digstore_core::Bytes48)` — fresh keypair from the OS CSPRNG; returns (secret, G1 public key 48B).
>   - `fn os_random_32() -> [u8;32]` — independent OS randomness (used for SecretSalt, NOT derived from the signing key).
>   - `fn secret_from_bytes(b: &[u8;32]) -> Result<SecretKeyBytes, digstore_crypto::BlsError>`.
>   - `fn sign_aug(sk: &SecretKeyBytes, msg: &[u8]) -> digstore_core::Bytes96` — AugScheme sign (prepends the pubkey to `msg` internally).
>
> **`digstore-store`** (host store + on-disk layout; exact API):
> - `StoreConfig { store_id: Bytes32, data_dir: String, max_size: u64, visibility: Visibility }`, `enum Visibility { Public, Private(SecretSalt) }`, `SecretSalt([u8;32])` — all `#[derive(serde::Serialize, serde::Deserialize)]`; the TOML encoding of `Visibility::Private` is an externally-tagged map (not load-bearing here — we surface the salt via a dedicated file, see Task 5).
> - `struct StagingArea` with: `fn load(path: &std::path::Path) -> Result<StagingArea, digstore_store::StoreError>` (returns an empty area if the file is empty), `fn save(&self, path: &std::path::Path) -> Result<(), digstore_store::StoreError>`, `fn empty() -> StagingArea`, `fn to_bytes(&self) -> Vec<u8>`, `fn is_empty(&self) -> bool`, `fn stage_resource(&mut self, key: &str, data: &[u8]) -> Result<usize, digstore_store::StoreError>` (chunks via digstore-chunker, returns chunk count), `fn resource_keys(&self) -> Vec<String>`, `fn staged_total_size(&self) -> u64`, `fn clear(&mut self)`, `fn seal_generation(&mut self, id: u64, timestamp: u64) -> Result<digstore_store::Generation, digstore_store::StoreError>`, `fn persist_generation(&self, generations_dir: &std::path::Path, gen: &digstore_store::Generation) -> Result<(), digstore_store::StoreError>`.
> - `Generation { state: GenerationState, tree: digstore_core::MerkleTree }` (re-exported `GenerationState`).
> - `struct GenerationManifest` (the per-generation on-disk manifest at `generations/{roothash}/manifest.json`, distinct from `MetadataManifest`), `#[derive(serde::Serialize, serde::Deserialize)]`, with `fn resource_keys(&self) -> Vec<String>` and `fn resource_digests(&self) -> std::collections::BTreeMap<String, Bytes32>` (each value = `Bytes32(sha256(concat(ordered chunk hashes)))`).
>
> **`digstore-compiler`** (exact API):
> - `struct Compiler;` with `fn new(trusted_keys: Vec<digstore_core::TrustedHostKey>) -> Result<Compiler, digstore_compiler::CompilerError>` (returns `Err(CompilerError::NoTrustedKeys)` if empty) and `fn compile(&self, cfg: &digstore_store::StoreConfig, gen: &digstore_store::Generation, modules_dir: &std::path::Path) -> Result<digstore_compiler::CompilationResult, digstore_compiler::CompilerError>`.
> - `CompilationResult { store_id: Bytes32, roothash: Bytes32, output_path: std::path::PathBuf, output_size: u64, stats: CompilationStats }`.
> - `fn template_guest_module_bytes() -> &'static [u8]` — the canonical compiled `digstore-guest` template (the artifact whose SHA-256 is the `program_hash` committed inside execution proofs — deviation #3). This is the SAME bytes for every store; the per-store data-injected module is NOT what `program_hash` covers.
>
> **`digstore-host`** (exact API; the host owns alloc/write/call/unpack/return-buffer/error-sentinel handling):
> - `struct Host;` with `fn new(module_bytes: &[u8], cfg: digstore_core::HostImportsConfig) -> Result<Host, digstore_host::HostError>`.
> - `fn call_get_content(&mut self, request: &[u8]) -> Result<Vec<u8>, digstore_host::HostError>` — encodes alloc/write/call `get_content`/`unpack_ptr_len`/read-return-buffer; on an `is_error` sentinel returns `Err(HostError::Guest(ErrorCode))`. The returned `Vec<u8>` is the raw codec bytes of the export's success payload.
> - `fn call_get_proof(&mut self, request: &[u8]) -> Result<Vec<u8>, digstore_host::HostError>` — same contract for `get_proof`.
> - `digstore_host::HostError` exposes `fn error_code(&self) -> Option<digstore_core::ErrorCode>`.
> - `HostImportsConfig` is the canonical type with a `Default` impl (`return_buffer_capacity: 64*1024`, `max_return_buffer_size: 16*1024*1024`, `max_random_bytes: 1024`, `host_version: <crate version>`).
>
> **`digstore-remote`** (exact client + test-server API):
> - `digstore_remote::client::RemoteClient` with `fn new(base_url: &str) -> Result<RemoteClient, digstore_remote::RemoteError>`, and async methods `get_descriptor(&self) -> Result<digstore_remote::StoreDescriptor, RemoteError>`, `get_roots(&self) -> Result<Vec<GenerationState>, RemoteError>`, `get_module(&self) -> Result<Vec<u8>, RemoteError>`, `head_module(&self) -> Result<digstore_remote::ModuleHead, RemoteError>`, `push_module(&self, store_id: &Bytes32, root: &Bytes32, module: &[u8], signature: &Bytes96) -> Result<digstore_remote::PushOutcome, RemoteError>`.
> - `digstore_remote::StoreDescriptor { store_id: Bytes32, root: Bytes32, size: u64, pubkey: Bytes48 }` (exact catalog fields — current root, size, pubkey; NO timestamp).
> - `digstore_remote::ModuleHead { exists: bool, size: u64, etag_root: Bytes32 }`.
> - `digstore_remote::PushOutcome { accepted: bool, pending: bool }` (pending = 202, head not advanced).
> - `enum RemoteError` with variants including `NonFastForward`, `Unauthorized`, `NotFound`, `Status(u16)`, `Network(String)` — match on the TYPED enum, never on Display strings.
> - `digstore_remote::testing::TestServer` with async `start_empty(store_id_hex: &str) -> TestServer`, async `start_with_module(store_id_hex: &str, root_hex: &str, module: &[u8]) -> TestServer`, and `fn base_url(&self) -> String`. The test server seeds `/roots` with one `GenerationState { id, root, timestamp }` per pushed/seeded module.

---

## File Structure

All paths under `crates/digstore-cli/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest; `[[bin]] name = "digstore"`; deps on the six digstore crates + clap/anyhow/tokio/serde/tracing; dev-deps assert_cmd/predicates/tempfile. |
| `src/main.rs` | Binary entry. Parses `Cli`, dispatches to `commands::*`, maps `CliError` → process exit code, installs tracing. |
| `src/lib.rs` | Library crate root (so e2e tests + unit tests share code). Declares all modules. |
| `src/cli.rs` | `clap` `Cli` struct + `Command` enum (one variant per verb) + global flags (`--dig-dir`, `--verbose`, `--json`). |
| `src/error.rs` | `CliError` enum (thiserror) + `ExitCode` mapping; `from_error_code` for upstream `ErrorCode`. |
| `src/context.rs` | `CliContext { dig_dir, json, verbose }`; resolves `~/.dig` or `--dig-dir`; locates current store + paths. |
| `src/output.rs` | Human + `--json` rendering helpers (status view, log list, diff). |
| `src/commands/mod.rs` | `dispatch(cli)` — matches `Command` → calls `ops`, formats result via `output`. |
| `src/commands/init.rs` … `pull.rs` | One module per verb; each a thin `run(ctx, args)`. |
| `src/ops/mod.rs` | Module declarations for the testable operation core. |
| `src/ops/store_ops.rs` | `init_store`, `add_path`, `commit`, `status`, `log`, `diff`, `checkout` helpers, `module_path_for`, history I/O. |
| `src/ops/serve.rs` | `serve_content` / `serve_proof`: spin a `digstore-host` instance over a module and call the export. |
| `src/ops/client_crypto.rs` | `derive_decryption_key` (§11.3/11.4) → per-chunk merkle verify (§9.3) → AES-256-GCM open → reassemble. |
| `src/ops/remote_ops.rs` | `clone_from`, `push_to`, `pull_from` over the reqwest client; typed error mapping. |
| `src/config.rs` | CLI-level config: remotes table (`remotes.toml`) read/write. |
| `tests/common/mod.rs` | E2E harness: `dig()` (assert_cmd `Command`), `tmp_dig()` (tempdir), store-id/root scrapers, wasm data-section corruption helper. |
| `tests/cli_init.rs` | E2E: init creates layout + a trusted key; help/json smoke. |
| `tests/cli_add_status.rs` | E2E: add stages; status shows it. |
| `tests/cli_commit_log.rs` | E2E: commit builds module + new root; log lists it. |
| `tests/cli_diff.rs` | E2E: diff between two generations. |
| `tests/cli_cat_roundtrip.rs` | E2E: add→commit→cat == input (single-chunk and multi-chunk); public-store miss decoy. |
| `tests/cli_private_salt.rs` | E2E: private store cat without salt → fails; with salt → plaintext (§11.4). |
| `tests/cli_checkout.rs` | E2E: checkout materializes a generation. |
| `tests/cli_tamper.rs` | E2E: tampered module data section → merkle/GCM verify fails on client (§9.3). |
| `tests/cli_remote_clone_push_pull.rs` | E2E: remote add + clone from test server + push fast-forward + pull advances. |

---

## Task 1 — Crate scaffold + manifest

**Files:**
- Modify root `C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml`
- Create `crates/digstore-cli/Cargo.toml`
- Create `crates/digstore-cli/src/lib.rs`
- Create `crates/digstore-cli/src/main.rs`

Steps:

- [ ] Add `"crates/digstore-cli"` to the workspace `members` array in `C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml`.
- [ ] Create `crates/digstore-cli/Cargo.toml`:
  ```toml
  [package]
  name = "digstore-cli"
  version = "0.1.0"
  edition = "2021"

  [[bin]]
  name = "digstore"
  path = "src/main.rs"

  [lib]
  name = "digstore_cli"
  path = "src/lib.rs"

  [dependencies]
  digstore-core = { path = "../digstore-core" }
  digstore-store = { path = "../digstore-store" }
  digstore-compiler = { path = "../digstore-compiler" }
  digstore-host = { path = "../digstore-host" }
  digstore-remote = { path = "../digstore-remote" }
  digstore-crypto = { path = "../digstore-crypto" }
  clap = { version = "4", features = ["derive"] }
  anyhow = "1"
  thiserror = "1"
  tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros"] }
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  toml = "0.8"
  tracing = "0.1"
  tracing-subscriber = { version = "0.3", features = ["env-filter"] }
  dirs = "5"

  [dev-dependencies]
  assert_cmd = "2"
  predicates = "3"
  tempfile = "3"
  ```
- [ ] Create `crates/digstore-cli/src/lib.rs` with exactly: `pub mod error;`
- [ ] Create `crates/digstore-cli/src/main.rs` with exactly: `fn main() { println!("digstore"); }`
- [ ] Create an empty `crates/digstore-cli/src/error.rs` (one line: `// CLI error type — implemented in Task 2`).
- [ ] Run `cargo build -p digstore-cli` — expect it to compile (it will fail only if a dependency crate is missing from the workspace; if so, that dependency's plan has not landed — STOP and surface that, do not stub it here). Expected on success: `Compiling digstore-cli v0.1.0 ... Finished`.
- [ ] Commit: `git add crates/digstore-cli Cargo.toml && git commit -m "chore(cli): scaffold digstore-cli crate manifest and entry points"`

---

## Task 2 — `CliError` + exit-code mapping (genuine red→green)

**Files:**
- Modify `crates/digstore-cli/src/error.rs`
- Test: inline `#[cfg(test)]` in `error.rs`

Steps:

- [ ] **RED (test only).** Replace `error.rs` with the type skeleton (bodies `todo!()`) plus the tests, so the tests fail at runtime not compile-time:
  ```rust
  //! CLI error type and process exit-code mapping.

  use digstore_core::ErrorCode;

  /// Top-level CLI error. Every command returns `Result<_, CliError>`.
  #[derive(Debug, thiserror::Error)]
  pub enum CliError {
      #[error("no digstore found at {0}; run `digstore init` first")]
      NoStore(String),
      #[error("invalid argument: {0}")]
      InvalidArgument(String),
      #[error("resource not found: {0}")]
      NotFound(String),
      #[error("verification failed: {0}")]
      VerificationFailed(String),
      #[error("network error: {0}")]
      Network(String),
      #[error("non-fast-forward: remote root has advanced")]
      NonFastForward,
      #[error("unauthorized: {0}")]
      Unauthorized(String),
      #[error(transparent)]
      Other(#[from] anyhow::Error),
  }

  impl CliError {
      /// Exit-code contract:
      /// 0 success | 1 other | 2 invalid-argument | 3 no-store | 4 not-found
      /// 5 verification-failed | 6 network | 7 non-fast-forward | 8 unauthorized.
      pub fn exit_code(&self) -> i32 {
          let _ = self;
          todo!("exit_code")
      }

      /// Map a canonical `digstore-core` ErrorCode (from a host/guest call) to a CliError.
      pub fn from_error_code(_code: ErrorCode, _ctx: &str) -> Self {
          todo!("from_error_code")
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn exit_codes_are_distinct_and_nonzero() {
          let errs = [
              CliError::NoStore("x".into()),
              CliError::InvalidArgument("x".into()),
              CliError::NotFound("x".into()),
              CliError::VerificationFailed("x".into()),
              CliError::Network("x".into()),
              CliError::NonFastForward,
              CliError::Unauthorized("x".into()),
          ];
          let mut codes: Vec<i32> = errs.iter().map(|e| e.exit_code()).collect();
          let n = codes.len();
          codes.sort_unstable();
          codes.dedup();
          assert_eq!(codes.len(), n, "exit codes must be distinct");
          assert!(codes.iter().all(|c| *c != 0), "exit codes must be nonzero");
      }

      #[test]
      fn maps_not_found_error_code() {
          let e = CliError::from_error_code(ErrorCode::NotFound, "urn:dig:...");
          assert!(matches!(e, CliError::NotFound(_)));
      }
  }
  ```
- [ ] Run `cargo test -p digstore-cli error::tests` — expect FAIL: `thread '...' panicked at 'not yet implemented: exit_code'`.
- [ ] **GREEN (minimal impl).** Replace the two `todo!()` bodies:
  ```rust
      pub fn exit_code(&self) -> i32 {
          match self {
              CliError::NoStore(_) => 3,
              CliError::InvalidArgument(_) => 2,
              CliError::NotFound(_) => 4,
              CliError::VerificationFailed(_) => 5,
              CliError::Network(_) => 6,
              CliError::NonFastForward => 7,
              CliError::Unauthorized(_) => 8,
              CliError::Other(_) => 1,
          }
      }

      pub fn from_error_code(code: ErrorCode, ctx: &str) -> Self {
          match code {
              ErrorCode::NotFound => CliError::NotFound(ctx.to_string()),
              ErrorCode::ValidationFailed => CliError::VerificationFailed(ctx.to_string()),
              ErrorCode::NetworkError | ErrorCode::Timeout => CliError::Network(ctx.to_string()),
              ErrorCode::NoSession | ErrorCode::SessionExpired => {
                  CliError::Unauthorized(ctx.to_string())
              }
              _ => CliError::InvalidArgument(ctx.to_string()),
          }
      }
  ```
- [ ] Run `cargo test -p digstore-cli error::tests` — expect PASS: `test result: ok. 2 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): CliError with distinct exit codes + ErrorCode mapping"`

---

## Task 3 — `CliContext`: dig-dir resolution & store discovery

**Files:**
- Create `crates/digstore-cli/src/context.rs`
- Modify `crates/digstore-cli/src/lib.rs` (add `pub mod context;`)
- Test: inline in `context.rs`

Steps:

- [ ] **RED.** Create `context.rs` with the struct + method signatures returning `todo!()`, plus tests:
  ```rust
  //! CLI execution context: where the store lives, output mode.

  use std::path::{Path, PathBuf};

  use digstore_core::Bytes32;
  use digstore_store::StoreConfig;

  use crate::error::CliError;

  #[derive(Debug, Clone)]
  pub struct CliContext {
      pub dig_dir: PathBuf,
      pub json: bool,
      pub verbose: bool,
  }

  impl CliContext {
      pub fn resolve(_explicit: Option<PathBuf>, _json: bool, _verbose: bool) -> Self {
          todo!("resolve")
      }
      pub fn config_path(&self) -> PathBuf { todo!("config_path") }
      pub fn load_config(&self) -> Result<StoreConfig, CliError> { todo!("load_config") }
      pub fn find_store_id(&self) -> Result<Bytes32, CliError> { todo!("find_store_id") }
      pub fn modules_dir(&self) -> PathBuf { todo!("modules_dir") }
      pub fn generations_dir(&self) -> PathBuf { todo!("generations_dir") }
      pub fn staging_path(&self, _store_id: &Bytes32) -> PathBuf { todo!("staging_path") }
      pub fn salt_path(&self) -> PathBuf { todo!("salt_path") }
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use tempfile::tempdir;

      #[test]
      fn explicit_dig_dir_is_used_verbatim() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          assert_eq!(ctx.dig_dir, td.path());
      }

      #[test]
      fn config_toml_path_is_under_dig_dir() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          assert_eq!(ctx.config_path(), td.path().join("config.toml"));
      }

      #[test]
      fn find_store_id_errors_when_no_config() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          assert!(ctx.find_store_id().is_err());
      }
  }
  ```
- [ ] Add `pub mod context;` to `lib.rs`.
- [ ] Run `cargo test -p digstore-cli context::tests` — expect FAIL: `panicked at 'not yet implemented: resolve'`.
- [ ] **GREEN.** Replace the `impl CliContext` body:
  ```rust
  impl CliContext {
      pub fn resolve(explicit: Option<PathBuf>, json: bool, verbose: bool) -> Self {
          let dig_dir = explicit.unwrap_or_else(|| {
              dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".dig")
          });
          CliContext { dig_dir, json, verbose }
      }

      pub fn config_path(&self) -> PathBuf {
          self.dig_dir.join("config.toml")
      }

      pub fn load_config(&self) -> Result<StoreConfig, CliError> {
          let path = self.config_path();
          if !path.exists() {
              return Err(CliError::NoStore(self.dig_dir.display().to_string()));
          }
          let text = std::fs::read_to_string(&path).map_err(|e| CliError::Other(e.into()))?;
          toml::from_str(&text).map_err(|e| CliError::Other(e.into()))
      }

      pub fn find_store_id(&self) -> Result<Bytes32, CliError> {
          Ok(self.load_config()?.store_id)
      }

      pub fn modules_dir(&self) -> PathBuf {
          self.dig_dir.join("modules")
      }

      pub fn generations_dir(&self) -> PathBuf {
          self.dig_dir.join("generations")
      }

      pub fn staging_path(&self, store_id: &Bytes32) -> PathBuf {
          self.dig_dir.join(format!("{}.staging.bin", store_id.to_hex()))
      }

      pub fn salt_path(&self) -> PathBuf {
          self.dig_dir.join("secret_salt.hex")
      }
  }
  ```
  Remove the now-unused `Path` import if the compiler warns (`use std::path::PathBuf;`).
- [ ] Run `cargo test -p digstore-cli context::tests` — expect PASS: `test result: ok. 3 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): CliContext for dig-dir resolution and store discovery"`

---

## Task 4 — clap CLI definition (`Cli` + `Command`)

**Files:**
- Create `crates/digstore-cli/src/cli.rs`
- Modify `crates/digstore-cli/src/lib.rs` (add `pub mod cli;`)
- Test: inline in `cli.rs`

Steps:

- [ ] **RED.** Create `cli.rs` with the full clap definition (clap derive needs the real structs to compile, so this is a compile-then-run red for the tests) and the test module:
  ```rust
  //! `clap` command-line surface for the `digstore` binary.

  use std::path::PathBuf;

  use clap::{Args, Parser, Subcommand};

  #[derive(Debug, Parser)]
  #[command(name = "digstore", version, about, long_about = None)]
  pub struct Cli {
      #[arg(long, global = true)]
      pub dig_dir: Option<PathBuf>,
      #[arg(long, global = true)]
      pub json: bool,
      #[arg(short, long, global = true)]
      pub verbose: bool,
      #[command(subcommand)]
      pub command: Command,
  }

  #[derive(Debug, Subcommand)]
  pub enum Command {
      Init(InitArgs),
      Add(AddArgs),
      Commit(CommitArgs),
      Status(StatusArgs),
      Log(LogArgs),
      Diff(DiffArgs),
      Checkout(CheckoutArgs),
      Cat(CatArgs),
      Remote(RemoteArgs),
      Clone(CloneArgs),
      Push(PushArgs),
      Pull(PullArgs),
  }

  #[derive(Debug, Args)]
  pub struct InitArgs {
      #[arg(long)]
      pub private: bool,
      #[arg(long)]
      pub data_dir: Option<String>,
  }

  #[derive(Debug, Args)]
  pub struct AddArgs {
      pub path: PathBuf,
      #[arg(long)]
      pub key: Option<String>,
  }

  #[derive(Debug, Args)]
  pub struct CommitArgs {
      #[arg(short, long)]
      pub message: Option<String>,
  }

  #[derive(Debug, Args)]
  pub struct StatusArgs {}

  #[derive(Debug, Args)]
  pub struct LogArgs {
      #[arg(short, long)]
      pub limit: Option<usize>,
  }

  #[derive(Debug, Args)]
  pub struct DiffArgs {
      pub from: String,
      pub to: String,
  }

  #[derive(Debug, Args)]
  pub struct CheckoutArgs {
      pub root: String,
      #[arg(long, short)]
      pub out: PathBuf,
      #[arg(long)]
      pub salt: Option<String>,
  }

  #[derive(Debug, Args)]
  pub struct CatArgs {
      pub urn: String,
      #[arg(long)]
      pub salt: Option<String>,
      #[arg(long)]
      pub verify_proof: bool,
  }

  #[derive(Debug, Args)]
  pub struct RemoteArgs {
      #[command(subcommand)]
      pub action: RemoteAction,
  }

  #[derive(Debug, Subcommand)]
  pub enum RemoteAction {
      Add { name: String, url: String },
      List,
      Remove { name: String },
  }

  #[derive(Debug, Args)]
  pub struct CloneArgs {
      pub source: String,
  }

  #[derive(Debug, Args)]
  pub struct PushArgs {
      #[arg(default_value = "origin")]
      pub remote: String,
  }

  #[derive(Debug, Args)]
  pub struct PullArgs {
      #[arg(default_value = "origin")]
      pub remote: String,
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use clap::Parser;

      #[test]
      fn parses_init() {
          let cli = Cli::try_parse_from(["digstore", "init"]).unwrap();
          assert!(matches!(cli.command, Command::Init(_)));
      }

      #[test]
      fn parses_add_path() {
          let cli = Cli::try_parse_from(["digstore", "add", "file.txt"]).unwrap();
          match cli.command {
              Command::Add(a) => assert_eq!(a.path.to_str().unwrap(), "file.txt"),
              _ => panic!("expected add"),
          }
      }

      #[test]
      fn parses_cat_urn() {
          let cli = Cli::try_parse_from(["digstore", "cat", "urn:dig:chia:abcd/readme"]).unwrap();
          match cli.command {
              Command::Cat(c) => assert_eq!(c.urn, "urn:dig:chia:abcd/readme"),
              _ => panic!("expected cat"),
          }
      }

      #[test]
      fn parses_remote_add_subcommand() {
          let cli = Cli::try_parse_from([
              "digstore", "remote", "add", "origin", "https://h/stores/x",
          ])
          .unwrap();
          match cli.command {
              Command::Remote(r) => match r.action {
                  RemoteAction::Add { name, url } => {
                      assert_eq!(name, "origin");
                      assert_eq!(url, "https://h/stores/x");
                  }
                  _ => panic!("expected remote add"),
              },
              _ => panic!("expected remote"),
          }
      }

      #[test]
      fn global_dig_dir_flag_before_subcommand() {
          let cli = Cli::try_parse_from(["digstore", "--dig-dir", "/tmp/d", "status"]).unwrap();
          assert_eq!(cli.dig_dir.unwrap().to_str().unwrap(), "/tmp/d");
      }

      #[test]
      fn global_json_flag_after_subcommand() {
          // proves global = true allows the flag to follow the subcommand
          let cli = Cli::try_parse_from(["digstore", "status", "--json"]).unwrap();
          assert!(cli.json);
      }

      #[test]
      fn private_salt_flag_on_cat() {
          let cli = Cli::try_parse_from([
              "digstore", "cat", "urn:dig:chia:abcd/r", "--salt",
              "0000000000000000000000000000000000000000000000000000000000000000",
          ])
          .unwrap();
          match cli.command {
              Command::Cat(c) => assert!(c.salt.is_some()),
              _ => panic!("expected cat"),
          }
      }
  }
  ```
- [ ] Add `pub mod cli;` to `lib.rs`.
- [ ] Run `cargo test -p digstore-cli cli::tests` — since the impl is the clap struct itself (data, not logic) and there is no separable behavior to stub, this is a structural module: expect PASS on first run: `test result: ok. 7 passed`. (If any test fails, the clap attributes are wrong — fix them, do not change the assertions.)
- [ ] Commit: `git add -A && git commit -m "feat(cli): clap Cli/Command for all git verbs incl. global-flag placement tests"`

---

## Task 5 — Output rendering (human + JSON)

**Files:**
- Create `crates/digstore-cli/src/output.rs`
- Modify `crates/digstore-cli/src/lib.rs` (add `pub mod output;`)
- Test: inline in `output.rs`

Steps:

- [ ] **RED.** Create `output.rs` with the view structs (data) + render fns returning `todo!()`, plus tests:
  ```rust
  //! Human + JSON rendering of command results.

  use serde::Serialize;

  #[derive(Debug, Serialize)]
  pub struct StatusView {
      pub root: Option<String>,
      pub staged: Vec<String>,
  }

  #[derive(Debug, Serialize)]
  pub struct LogEntry {
      pub id: u64,
      pub root: String,
      pub timestamp: u64,
  }

  #[derive(Debug, Serialize)]
  pub struct DiffEntry {
      pub resource_key: String,
      pub change: String, // "added" | "removed" | "modified"
  }

  pub fn render_status(_s: &StatusView, _json: bool) -> String { todo!("render_status") }
  pub fn render_log(_entries: &[LogEntry], _json: bool) -> String { todo!("render_log") }
  pub fn render_diff(_entries: &[DiffEntry], _json: bool) -> String { todo!("render_diff") }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn render_status_json_has_staged_count() {
          let s = StatusView { root: Some("ab".into()), staged: vec!["readme".into()] };
          let out = render_status(&s, true);
          assert!(out.contains("\"staged\""));
          assert!(out.contains("readme"));
      }

      #[test]
      fn render_status_human_lists_entries() {
          let s = StatusView { root: None, staged: vec!["a".into(), "b".into()] };
          let out = render_status(&s, false);
          assert!(out.contains("a"));
          assert!(out.contains("b"));
          assert!(out.to_lowercase().contains("staged"));
      }

      #[test]
      fn render_log_json_is_array() {
          let v = vec![LogEntry { id: 1, root: "aa".into(), timestamp: 100 }];
          let out = render_log(&v, true);
          assert!(out.trim_start().starts_with('['));
      }

      #[test]
      fn render_diff_human_uses_plus_for_added() {
          let v = vec![DiffEntry { resource_key: "b".into(), change: "added".into() }];
          let out = render_diff(&v, false);
          assert!(out.contains("+ b"));
      }
  }
  ```
- [ ] Add `pub mod output;` to `lib.rs`.
- [ ] Run `cargo test -p digstore-cli output::tests` — expect FAIL: `panicked at 'not yet implemented: render_status'`.
- [ ] **GREEN.** Replace the three render fns:
  ```rust
  pub fn render_status(s: &StatusView, json: bool) -> String {
      if json {
          return serde_json::to_string_pretty(s).expect("serialize status");
      }
      let mut out = String::new();
      match &s.root {
          Some(r) => out.push_str(&format!("On root {}\n", r)),
          None => out.push_str("No commits yet\n"),
      }
      if s.staged.is_empty() {
          out.push_str("nothing staged\n");
      } else {
          out.push_str("Staged for commit:\n");
          for e in &s.staged {
              out.push_str(&format!("  staged: {}\n", e));
          }
      }
      out
  }

  pub fn render_log(entries: &[LogEntry], json: bool) -> String {
      if json {
          return serde_json::to_string_pretty(entries).expect("serialize log");
      }
      let mut out = String::new();
      for e in entries {
          out.push_str(&format!("generation {}  root {}  ts {}\n", e.id, e.root, e.timestamp));
      }
      out
  }

  pub fn render_diff(entries: &[DiffEntry], json: bool) -> String {
      if json {
          return serde_json::to_string_pretty(entries).expect("serialize diff");
      }
      let mut out = String::new();
      for e in entries {
          let sign = match e.change.as_str() {
              "added" => '+',
              "removed" => '-',
              _ => '~',
          };
          out.push_str(&format!("{} {}\n", sign, e.resource_key));
      }
      out
  }
  ```
- [ ] Run `cargo test -p digstore-cli output::tests` — expect PASS: `test result: ok. 4 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): human + JSON output rendering for status/log/diff"`

---

## Task 6 — Command-module stubs + dispatch + `main` wiring

**Files:**
- Create `crates/digstore-cli/src/commands/mod.rs` and `src/commands/{init,add,commit,status,log,diff,checkout,cat,remote,clone,push,pull}.rs`
- Create empty `src/ops/mod.rs`, `src/config.rs`
- Modify `crates/digstore-cli/src/lib.rs`, `crates/digstore-cli/src/main.rs`

> This task wires the plumbing with NO behavior; each command stub returns a deterministic `not implemented` error so the binary compiles and dispatch is exercised. There is one behavioral assertion (an e2e smoke test) to guard the wiring.

Steps:

- [ ] Create `src/ops/mod.rs` containing exactly: `// ops modules added per task` (empty module body is valid).
- [ ] Create `src/config.rs` containing exactly: `// remotes config — implemented in Task 16`.
- [ ] Set `lib.rs` to the final module list:
  ```rust
  pub mod cli;
  pub mod commands;
  pub mod config;
  pub mod context;
  pub mod error;
  pub mod ops;
  pub mod output;
  ```
- [ ] Create each command stub. `src/commands/init.rs`:
  ```rust
  use crate::cli::InitArgs;
  use crate::context::CliContext;
  use crate::error::CliError;

  pub fn run(_ctx: &CliContext, _args: InitArgs) -> Result<(), CliError> {
      Err(CliError::Other(anyhow::anyhow!("init not implemented")))
  }
  ```
  Create the same shape for the other 11 files, substituting the arg type and message:
  - `add.rs` → `AddArgs`, "add not implemented"
  - `commit.rs` → `CommitArgs`, "commit not implemented"
  - `status.rs` → `StatusArgs`, "status not implemented"
  - `log.rs` → `LogArgs`, "log not implemented"
  - `diff.rs` → `DiffArgs`, "diff not implemented"
  - `checkout.rs` → `CheckoutArgs`, "checkout not implemented"
  - `cat.rs` → `CatArgs`, "cat not implemented"
  - `remote.rs` → `RemoteArgs`, "remote not implemented"
  - `clone.rs` → `CloneArgs`, "clone not implemented"
  - `push.rs` → `PushArgs`, "push not implemented"
  - `pull.rs` → `PullArgs`, "pull not implemented"
- [ ] Create `src/commands/mod.rs`:
  ```rust
  //! Command dispatch: clap `Command` -> `ops` -> `output`.

  use crate::cli::{Cli, Command};
  use crate::context::CliContext;
  use crate::error::CliError;

  pub mod init;
  pub mod add;
  pub mod commit;
  pub mod status;
  pub mod log;
  pub mod diff;
  pub mod checkout;
  pub mod cat;
  pub mod remote;
  pub mod clone;
  pub mod push;
  pub mod pull;

  pub fn dispatch(cli: Cli) -> Result<(), CliError> {
      let ctx = CliContext::resolve(cli.dig_dir.clone(), cli.json, cli.verbose);
      match cli.command {
          Command::Init(a) => init::run(&ctx, a),
          Command::Add(a) => add::run(&ctx, a),
          Command::Commit(a) => commit::run(&ctx, a),
          Command::Status(a) => status::run(&ctx, a),
          Command::Log(a) => log::run(&ctx, a),
          Command::Diff(a) => diff::run(&ctx, a),
          Command::Checkout(a) => checkout::run(&ctx, a),
          Command::Cat(a) => cat::run(&ctx, a),
          Command::Remote(a) => remote::run(&ctx, a),
          Command::Clone(a) => clone::run(&ctx, a),
          Command::Push(a) => push::run(&ctx, a),
          Command::Pull(a) => pull::run(&ctx, a),
      }
  }
  ```
- [ ] Replace `main.rs`:
  ```rust
  use clap::Parser;
  use digstore_cli::cli::Cli;
  use digstore_cli::commands;

  fn main() {
      let cli = Cli::parse();
      if cli.verbose {
          let _ = tracing_subscriber::fmt()
              .with_env_filter(
                  tracing_subscriber::EnvFilter::try_from_default_env()
                      .unwrap_or_else(|_| "digstore=debug".into()),
              )
              .try_init();
      }
      match commands::dispatch(cli) {
          Ok(()) => std::process::exit(0),
          Err(e) => {
              eprintln!("error: {e}");
              std::process::exit(e.exit_code());
          }
      }
  }
  ```
- [ ] Run `cargo build -p digstore-cli` — expect it to compile (all commands stubbed). Expected: `Finished`.
- [ ] **RED (e2e smoke).** Create `tests/common/mod.rs`:
  ```rust
  #![allow(dead_code)]
  use assert_cmd::Command;
  use tempfile::TempDir;

  pub fn dig(dir: &TempDir) -> Command {
      let mut cmd = Command::cargo_bin("digstore").unwrap();
      cmd.arg("--dig-dir").arg(dir.path());
      cmd
  }

  pub fn tmp_dig() -> TempDir {
      TempDir::new().unwrap()
  }

  /// Scrape store_id (hex) from config.toml and newest root (hex) from `log --json`.
  pub fn store_id_and_root(dir: &TempDir) -> (String, String) {
      let out = dig(dir).args(["log", "--json"]).output().unwrap();
      let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
      let root = v[0]["root"].as_str().unwrap().to_string();
      let cfg = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
      let line = cfg.lines().find(|l| l.contains("store_id")).unwrap();
      let store_id = line.split('"').nth(1).unwrap().to_string();
      (store_id, root)
  }
  ```
  Create `tests/cli_init.rs`:
  ```rust
  mod common;
  use common::{dig, tmp_dig};
  use predicates::prelude::*;

  #[test]
  fn help_lists_all_verbs() {
      let dir = tmp_dig();
      dig(&dir)
          .arg("--help")
          .assert()
          .success()
          .stdout(
              predicate::str::contains("init")
                  .and(predicate::str::contains("commit"))
                  .and(predicate::str::contains("cat"))
                  .and(predicate::str::contains("clone"))
                  .and(predicate::str::contains("push"))
                  .and(predicate::str::contains("pull")),
          );
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_init` — expect PASS (clap auto-generates `--help`): `test result: ok. 1 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): dispatch + stubbed commands + main wiring; help smoke test"`

---

## Task 7 — `init_store` + `init` command + E2E (§20.1)

> **Store-identity decision (documented):** `store_id = SHA-256(store BLS public key G1 bytes)`. The keypair is generated from the OS CSPRNG at init; the public key is recorded as the single trusted host key (so the compiler has ≥1 key). For a **private** store the `SecretSalt` is independent OS randomness (`os_random_32()`), NOT derived from the signing-key seed, and is surfaced to the user via `secret_salt.hex` (so `cat --salt` is scriptable without parsing TOML). A **cloned** store adopts the REMOTE store_id (see Task 17); local init store_id and a cloned store_id are unrelated by design — a clone never re-runs init.

**Files:**
- Create `crates/digstore-cli/src/ops/store_ops.rs`
- Modify `crates/digstore-cli/src/ops/mod.rs`, `crates/digstore-cli/src/commands/init.rs`
- Test: inline in `store_ops.rs`; `crates/digstore-cli/tests/cli_init.rs`

Steps:

- [ ] Add `pub mod store_ops;` to `ops/mod.rs`.
- [ ] **RED.** Create `store_ops.rs` with imports, the `InitResult` type, and `init_store` returning `todo!()`, plus two tests:
  ```rust
  //! Pure operations behind the local store git verbs.

  use std::fs;
  use std::path::{Path, PathBuf};

  use digstore_core::{Bytes32, Bytes48, GenerationState, TrustedHostKey, Urn};
  use digstore_core::codec::Encode;
  use digstore_store::{Generation, StoreConfig, SecretSalt, StagingArea, Visibility};

  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::output::{DiffEntry, LogEntry, StatusView};

  #[derive(Debug)]
  pub struct InitResult {
      pub store_id: Bytes32,
      pub host_public_key: Bytes48,
  }

  pub fn init_store(
      _ctx: &CliContext,
      _private: bool,
      _data_dir: Option<String>,
  ) -> Result<InitResult, CliError> {
      todo!("init_store")
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use tempfile::tempdir;

      #[test]
      fn init_creates_layout_and_config() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          let res = init_store(&ctx, false, None).unwrap();
          assert!(ctx.config_path().exists(), "config.toml written");
          assert!(ctx.modules_dir().exists(), "modules/ created");
          assert!(ctx.generations_dir().exists(), "generations/ created");
          assert!(td.path().join("trusted_keys.json").exists(), "trusted key written");
          assert!(td.path().join("signing_key.bin").exists(), "signing key written");
          assert_ne!(res.store_id, Bytes32([0u8; 32]));
      }

      #[test]
      fn init_private_records_secret_salt_file() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), true, false);
          init_store(&ctx, true, None).unwrap();
          let cfg = ctx.load_config().unwrap();
          assert!(matches!(cfg.visibility, Visibility::Private(_)));
          let salt_hex = std::fs::read_to_string(ctx.salt_path()).unwrap();
          assert_eq!(salt_hex.trim().len(), 64, "salt surfaced as 64-hex");
      }

      #[test]
      fn init_store_id_is_sha256_of_pubkey() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          let res = init_store(&ctx, false, None).unwrap();
          let expected = Bytes32(digstore_crypto::sha256(&res.host_public_key.0));
          assert_eq!(res.store_id, expected);
      }
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::init_creates_layout_and_config` — expect FAIL: `panicked at 'not yet implemented: init_store'`.
- [ ] **GREEN.** Replace `init_store`:
  ```rust
  pub fn init_store(
      ctx: &CliContext,
      private: bool,
      data_dir: Option<String>,
  ) -> Result<InitResult, CliError> {
      if ctx.config_path().exists() {
          return Err(CliError::InvalidArgument(format!(
              "store already initialized at {}",
              ctx.dig_dir.display()
          )));
      }
      fs::create_dir_all(&ctx.dig_dir).map_err(|e| CliError::Other(e.into()))?;
      fs::create_dir_all(ctx.modules_dir()).map_err(|e| CliError::Other(e.into()))?;
      fs::create_dir_all(ctx.generations_dir()).map_err(|e| CliError::Other(e.into()))?;

      // Host BLS keypair (chia AugScheme) from the OS CSPRNG.
      let (secret, host_public_key) = digstore_crypto::host_bls::keygen_os_rng();

      // Documented store-identity decision: store_id = SHA-256(store BLS public key).
      let store_id = Bytes32(digstore_crypto::sha256(&host_public_key.0));

      let visibility = if private {
          // SecretSalt is INDEPENDENT OS randomness, not derived from the signing key.
          Visibility::Private(SecretSalt(digstore_crypto::host_bls::os_random_32()))
      } else {
          Visibility::Public
      };

      let cfg = StoreConfig {
          store_id,
          data_dir: data_dir.unwrap_or_else(|| ctx.dig_dir.display().to_string()),
          max_size: 1024 * 1024 * 1024, // 1 GiB default ceiling (§20.2 enforcement)
          visibility: visibility.clone(),
      };
      fs::write(
          ctx.config_path(),
          toml::to_string_pretty(&cfg).map_err(|e| CliError::Other(e.into()))?,
      )
      .map_err(|e| CliError::Other(e.into()))?;

      // Persist signing key (host side, never embedded in modules).
      fs::write(ctx.dig_dir.join("signing_key.bin"), secret.0)
          .map_err(|e| CliError::Other(e.into()))?;

      // Surface SecretSalt deterministically for scripting `cat --salt`.
      if let Visibility::Private(salt) = &visibility {
          fs::write(ctx.salt_path(), Bytes32(salt.0).to_hex())
              .map_err(|e| CliError::Other(e.into()))?;
      }

      // Persist the trusted key set (the compiler reads this; canonical TrustedHostKey).
      let trusted = vec![TrustedHostKey {
          public_key: host_public_key.0,
          label: format!("dig-host-key-v1:{}", host_public_key.to_hex()),
      }];
      fs::write(
          ctx.dig_dir.join("trusted_keys.json"),
          serde_json::to_string_pretty(&trusted).map_err(|e| CliError::Other(e.into()))?,
      )
      .map_err(|e| CliError::Other(e.into()))?;

      // Empty staging.
      StagingArea::empty()
          .save(&ctx.staging_path(&store_id))
          .map_err(|e| CliError::Other(anyhow::anyhow!("save staging: {e}")))?;

      Ok(InitResult { store_id, host_public_key })
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests` — expect PASS: `test result: ok. 3 passed`.
- [ ] Implement `commands/init.rs`:
  ```rust
  use crate::cli::InitArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;

  pub fn run(ctx: &CliContext, args: InitArgs) -> Result<(), CliError> {
      let res = store_ops::init_store(ctx, args.private, args.data_dir)?;
      if ctx.json {
          println!(
              "{}",
              serde_json::json!({
                  "store_id": res.store_id.to_hex(),
                  "host_public_key": res.host_public_key.to_hex(),
              })
          );
      } else {
          println!("Initialized digstore {}", res.store_id.to_hex());
          println!("  dig dir: {}", ctx.dig_dir.display());
          println!("  trusted host key: {}", res.host_public_key.to_hex());
      }
      Ok(())
  }
  ```
- [ ] Add two e2e cases to `tests/cli_init.rs`:
  ```rust
  #[test]
  fn init_creates_store_and_trusted_key() {
      let dir = tmp_dig();
      dig(&dir)
          .arg("init")
          .assert()
          .success()
          .stdout(predicate::str::contains("Initialized digstore"));
      assert!(dir.path().join("config.toml").exists());
      assert!(dir.path().join("trusted_keys.json").exists());
      assert!(dir.path().join("modules").exists());
  }

  #[test]
  fn init_twice_fails_with_exit_2() {
      let dir = tmp_dig();
      dig(&dir).arg("init").assert().success();
      dig(&dir).arg("init").assert().failure().code(2);
  }

  #[test]
  fn init_json_emits_store_id() {
      let dir = tmp_dig();
      let out = dig(&dir).args(["--json", "init"]).output().unwrap();
      assert!(out.status.success());
      let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
      assert!(v["store_id"].as_str().is_some());
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_init` — expect PASS: `test result: ok. 4 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): init (layout + trusted key + store-id + salt file) with e2e (20.1)"`

---

## Task 8 — `log` + history I/O (§20.3 log)

**Files:**
- Modify `crates/digstore-cli/src/ops/store_ops.rs`, `crates/digstore-cli/src/commands/log.rs`
- Test: inline in `store_ops.rs`

> History lives at `~/.dig/history.json` as `Vec<GenerationState>` (canonical type, real generation ids). `log` and `current_root` read it.

Steps:

- [ ] **RED.** Add to `store_ops.rs` a test + the `log`/`current_root`/`append_history` signatures as `todo!()`:
  ```rust
  pub fn log(_ctx: &CliContext, _limit: Option<usize>) -> Result<Vec<LogEntry>, CliError> {
      todo!("log")
  }

  pub fn current_root(_ctx: &CliContext) -> Result<Option<Bytes32>, CliError> {
      todo!("current_root")
  }

  pub(crate) fn append_history(_ctx: &CliContext, _state: GenerationState) -> Result<(), CliError> {
      todo!("append_history")
  }
  ```
  Add inside `mod tests`:
  ```rust
      #[test]
      fn log_is_empty_before_any_commit() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          assert!(log(&ctx, None).unwrap().is_empty());
      }

      #[test]
      fn append_history_then_log_newest_first() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          append_history(&ctx, GenerationState { id: 1, root: Bytes32([1u8; 32]), timestamp: 10 }).unwrap();
          append_history(&ctx, GenerationState { id: 2, root: Bytes32([2u8; 32]), timestamp: 20 }).unwrap();
          let entries = log(&ctx, None).unwrap();
          assert_eq!(entries[0].id, 2, "newest first");
          assert_eq!(current_root(&ctx).unwrap(), Some(Bytes32([2u8; 32])));
      }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::log_is_empty_before_any_commit` — expect FAIL: `panicked at 'not yet implemented: log'`.
- [ ] **GREEN.** Replace the three fns:
  ```rust
  pub fn log(ctx: &CliContext, limit: Option<usize>) -> Result<Vec<LogEntry>, CliError> {
      let history_path = ctx.dig_dir.join("history.json");
      if !history_path.exists() {
          return Ok(Vec::new());
      }
      let text = fs::read_to_string(&history_path).map_err(|e| CliError::Other(e.into()))?;
      let mut states: Vec<GenerationState> =
          serde_json::from_str(&text).map_err(|e| CliError::Other(e.into()))?;
      states.sort_by(|a, b| b.id.cmp(&a.id));
      let iter = states.into_iter().map(|s| LogEntry {
          id: s.id,
          root: s.root.to_hex(),
          timestamp: s.timestamp,
      });
      Ok(match limit {
          Some(n) => iter.take(n).collect(),
          None => iter.collect(),
      })
  }

  pub fn current_root(ctx: &CliContext) -> Result<Option<Bytes32>, CliError> {
      let path = ctx.dig_dir.join("history.json");
      if !path.exists() {
          return Ok(None);
      }
      let text = fs::read_to_string(&path).map_err(|e| CliError::Other(e.into()))?;
      let states: Vec<GenerationState> =
          serde_json::from_str(&text).map_err(|e| CliError::Other(e.into()))?;
      Ok(states.iter().max_by_key(|s| s.id).map(|s| s.root))
  }

  pub(crate) fn append_history(ctx: &CliContext, state: GenerationState) -> Result<(), CliError> {
      let path = ctx.dig_dir.join("history.json");
      let mut states: Vec<GenerationState> = if path.exists() {
          serde_json::from_str(&fs::read_to_string(&path).map_err(|e| CliError::Other(e.into()))?)
              .map_err(|e| CliError::Other(e.into()))?
      } else {
          Vec::new()
      };
      states.push(state);
      fs::write(
          &path,
          serde_json::to_string_pretty(&states).map_err(|e| CliError::Other(e.into()))?,
      )
      .map_err(|e| CliError::Other(e.into()))
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::log_is_empty_before_any_commit store_ops::tests::append_history_then_log_newest_first` — expect PASS.
- [ ] Implement `commands/log.rs`:
  ```rust
  use crate::cli::LogArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;
  use crate::output;

  pub fn run(ctx: &CliContext, args: LogArgs) -> Result<(), CliError> {
      let entries = store_ops::log(ctx, args.limit)?;
      print!("{}", output::render_log(&entries, ctx.json));
      Ok(())
  }
  ```
- [ ] Run `cargo build -p digstore-cli` — expect compile.
- [ ] Commit: `git add -A && git commit -m "feat(cli): history.json I/O + log command (newest-first) (20.3)"`

---

## Task 9 — `add_path` (with `max_size` enforcement) + `status` + commands (§20.2)

**Files:**
- Modify `crates/digstore-cli/src/ops/store_ops.rs`, `crates/digstore-cli/src/commands/add.rs`, `crates/digstore-cli/src/commands/status.rs`
- Test: inline in `store_ops.rs`; `crates/digstore-cli/tests/cli_add_status.rs`

Steps:

- [ ] **RED.** Add `AddResult`, `add_path`, `status` as `todo!()` + tests:
  ```rust
  #[derive(Debug)]
  pub struct AddResult {
      pub resource_key: String,
      pub chunk_count: usize,
      pub total_size: u64,
  }

  pub fn add_path(_ctx: &CliContext, _path: &Path, _key: Option<String>) -> Result<AddResult, CliError> {
      todo!("add_path")
  }

  pub fn status(_ctx: &CliContext) -> Result<StatusView, CliError> {
      todo!("status")
  }
  ```
  Tests:
  ```rust
      #[test]
      fn add_path_stages_a_file_and_status_shows_it() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          let f = td.path().join("readme.txt");
          std::fs::write(&f, b"hello digstore").unwrap();
          let added = add_path(&ctx, &f, Some("readme".into())).unwrap();
          assert_eq!(added.resource_key, "readme");
          assert!(added.chunk_count >= 1);
          let s = status(&ctx).unwrap();
          assert!(s.staged.iter().any(|x| x == "readme"));
      }

      #[test]
      fn add_path_defaults_key_to_file_name() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          let f = td.path().join("notes.md");
          std::fs::write(&f, b"x").unwrap();
          let added = add_path(&ctx, &f, None).unwrap();
          assert_eq!(added.resource_key, "notes.md");
      }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::add_path_stages_a_file_and_status_shows_it` — expect FAIL: `panicked at 'not yet implemented: add_path'`.
- [ ] **GREEN.** Replace `add_path` + `status`:
  ```rust
  pub fn add_path(ctx: &CliContext, path: &Path, key: Option<String>) -> Result<AddResult, CliError> {
      let cfg = ctx.load_config()?;
      let data = fs::read(path).map_err(|e| CliError::Other(e.into()))?;
      let resource_key = key.unwrap_or_else(|| {
          path.file_name()
              .map(|n| n.to_string_lossy().into_owned())
              .unwrap_or_else(|| "unnamed".into())
      });
      let mut staging = StagingArea::load(&ctx.staging_path(&cfg.store_id))
          .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;

      // Enforce StoreConfig.max_size (§20.2): reject if staged total would exceed it.
      if cfg.max_size != 0 {
          let projected = staging.staged_total_size() + data.len() as u64;
          if projected > cfg.max_size {
              return Err(CliError::InvalidArgument(format!(
                  "staged size {} exceeds store max_size {}",
                  projected, cfg.max_size
              )));
          }
      }

      let chunk_count = staging
          .stage_resource(&resource_key, &data)
          .map_err(|e| CliError::Other(anyhow::anyhow!("stage: {e}")))?;
      staging
          .save(&ctx.staging_path(&cfg.store_id))
          .map_err(|e| CliError::Other(anyhow::anyhow!("save staging: {e}")))?;
      Ok(AddResult { resource_key, chunk_count, total_size: data.len() as u64 })
  }

  pub fn status(ctx: &CliContext) -> Result<StatusView, CliError> {
      let cfg = ctx.load_config()?;
      let staging = StagingArea::load(&ctx.staging_path(&cfg.store_id))
          .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
      let staged = staging.resource_keys();
      let root = current_root(ctx)?.map(|r| r.to_hex());
      Ok(StatusView { root, staged })
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::add_path_stages_a_file_and_status_shows_it store_ops::tests::add_path_defaults_key_to_file_name` — expect PASS.
- [ ] Implement `commands/add.rs`:
  ```rust
  use crate::cli::AddArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;

  pub fn run(ctx: &CliContext, args: AddArgs) -> Result<(), CliError> {
      let res = store_ops::add_path(ctx, &args.path, args.key)?;
      if ctx.json {
          println!(
              "{}",
              serde_json::json!({ "resource_key": res.resource_key, "chunks": res.chunk_count, "size": res.total_size })
          );
      } else {
          println!("staged {} ({} bytes, {} chunks)", res.resource_key, res.total_size, res.chunk_count);
      }
      Ok(())
  }
  ```
- [ ] Implement `commands/status.rs`:
  ```rust
  use crate::cli::StatusArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;
  use crate::output;

  pub fn run(ctx: &CliContext, _args: StatusArgs) -> Result<(), CliError> {
      let view = store_ops::status(ctx)?;
      print!("{}", output::render_status(&view, ctx.json));
      Ok(())
  }
  ```
- [ ] Create `tests/cli_add_status.rs`:
  ```rust
  mod common;
  use common::{dig, tmp_dig};
  use predicates::prelude::*;

  #[test]
  fn add_then_status_shows_staged() {
      let dir = tmp_dig();
      dig(&dir).arg("init").assert().success();
      let f = dir.path().join("readme.txt");
      std::fs::write(&f, b"hello digstore world").unwrap();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "readme"]).assert().success()
          .stdout(predicate::str::contains("staged readme"));
      dig(&dir).arg("status").assert().success()
          .stdout(predicate::str::contains("staged: readme"));
  }

  #[test]
  fn add_without_store_fails_exit_3() {
      let dir = tmp_dig();
      let f = dir.path().join("x.txt");
      std::fs::write(&f, b"x").unwrap();
      dig(&dir).args(["add"]).arg(&f).assert().failure().code(3);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_add_status` — expect PASS: `test result: ok. 2 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): add (with max_size enforcement) + status with e2e (20.2)"`

---

## Task 10 — `commit` (invoke compiler, single canonical TrustedHostKey) + E2E (§20.3)

**Files:**
- Modify `crates/digstore-cli/src/ops/store_ops.rs`, `crates/digstore-cli/src/commands/commit.rs`
- Test: inline in `store_ops.rs`; `crates/digstore-cli/tests/cli_commit_log.rs`

Steps:

- [ ] **RED.** Add `CommitOutcome`, `commit`, `load_trusted_keys`, `current_time` as `todo!()`/minimal + one test:
  ```rust
  #[derive(Debug)]
  pub struct CommitOutcome {
      pub roothash: Bytes32,
      pub output_path: PathBuf,
      pub output_size: u64,
  }

  pub fn commit(_ctx: &CliContext, _message: Option<String>) -> Result<CommitOutcome, CliError> {
      todo!("commit")
  }

  fn current_time() -> u64 {
      std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .map(|d| d.as_secs())
          .unwrap_or(0)
  }

  fn load_trusted_keys(ctx: &CliContext) -> Result<Vec<TrustedHostKey>, CliError> {
      let path = ctx.dig_dir.join("trusted_keys.json");
      let text = fs::read_to_string(&path).map_err(|e| CliError::Other(e.into()))?;
      serde_json::from_str(&text).map_err(|e| CliError::Other(e.into()))
  }
  ```
  Test:
  ```rust
      #[test]
      fn commit_builds_module_and_appends_root() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          let f = td.path().join("a.txt");
          std::fs::write(&f, b"alpha beta gamma delta").unwrap();
          add_path(&ctx, &f, Some("a".into())).unwrap();

          let res = commit(&ctx, Some("first".into())).unwrap();
          assert!(res.output_path.exists(), "module written to disk");
          let entries = log(&ctx, None).unwrap();
          assert_eq!(entries.len(), 1);
          assert_eq!(entries[0].root, res.roothash.to_hex());
      }

      #[test]
      fn commit_with_nothing_staged_errors() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          assert!(commit(&ctx, None).is_err());
      }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::commit_builds_module_and_appends_root` — expect FAIL: `panicked at 'not yet implemented: commit'`.
- [ ] **GREEN.** Replace `commit` (note: ONE canonical `TrustedHostKey` from digstore-core, written by init and read here — identical type, no aliasing):
  ```rust
  pub fn commit(ctx: &CliContext, _message: Option<String>) -> Result<CommitOutcome, CliError> {
      let cfg = ctx.load_config()?;

      let mut staging = StagingArea::load(&ctx.staging_path(&cfg.store_id))
          .map_err(|e| CliError::Other(anyhow::anyhow!("load staging: {e}")))?;
      if staging.is_empty() {
          return Err(CliError::InvalidArgument("nothing staged to commit".into()));
      }

      let next_id = log(ctx, Some(1))?.first().map(|e| e.id + 1).unwrap_or(1);
      let generation = staging
          .seal_generation(next_id, current_time())
          .map_err(|e| CliError::Other(anyhow::anyhow!("seal generation: {e}")))?;
      staging
          .persist_generation(&ctx.generations_dir(), &generation)
          .map_err(|e| CliError::Other(anyhow::anyhow!("persist generation: {e}")))?;

      // Compiler refuses an empty trusted set (CompilerError::NoTrustedKeys).
      let trusted = load_trusted_keys(ctx)?;
      let compiler = digstore_compiler::Compiler::new(trusted)
          .map_err(|e| CliError::Other(anyhow::anyhow!("compiler init: {e}")))?;
      let result = compiler
          .compile(&cfg, &generation, &ctx.modules_dir())
          .map_err(|e| CliError::Other(anyhow::anyhow!("compile failed: {e}")))?;

      append_history(
          ctx,
          GenerationState {
              id: generation.state.id,
              root: result.roothash,
              timestamp: generation.state.timestamp,
          },
      )?;

      staging.clear();
      staging
          .save(&ctx.staging_path(&cfg.store_id))
          .map_err(|e| CliError::Other(anyhow::anyhow!("save staging: {e}")))?;

      Ok(CommitOutcome {
          roothash: result.roothash,
          output_path: result.output_path,
          output_size: result.output_size,
      })
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::commit_builds_module_and_appends_root store_ops::tests::commit_with_nothing_staged_errors` — expect PASS.
- [ ] Implement `commands/commit.rs`:
  ```rust
  use crate::cli::CommitArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;

  pub fn run(ctx: &CliContext, args: CommitArgs) -> Result<(), CliError> {
      let res = store_ops::commit(ctx, args.message)?;
      if ctx.json {
          println!(
              "{}",
              serde_json::json!({ "root": res.roothash.to_hex(), "module": res.output_path.display().to_string(), "size": res.output_size })
          );
      } else {
          println!("committed root {}", res.roothash.to_hex());
          println!("  module: {} ({} bytes)", res.output_path.display(), res.output_size);
      }
      Ok(())
  }
  ```
- [ ] Create `tests/cli_commit_log.rs`:
  ```rust
  mod common;
  use common::{dig, tmp_dig};
  use predicates::prelude::*;

  #[test]
  fn commit_creates_module_and_log_lists_it() {
      let dir = tmp_dig();
      dig(&dir).arg("init").assert().success();
      let f = dir.path().join("a.txt");
      std::fs::write(&f, b"alpha beta gamma").unwrap();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "a"]).assert().success();
      dig(&dir).args(["commit", "-m", "first"]).assert().success()
          .stdout(predicate::str::contains("committed root"));
      let modules: Vec<_> = std::fs::read_dir(dir.path().join("modules"))
          .unwrap()
          .filter_map(|e| e.ok())
          .filter(|e| e.path().extension().map(|x| x == "wasm").unwrap_or(false))
          .collect();
      assert_eq!(modules.len(), 1);
      dig(&dir).args(["log"]).assert().success()
          .stdout(predicate::str::contains("generation 1"));
  }

  #[test]
  fn commit_with_nothing_staged_fails_exit_2() {
      let dir = tmp_dig();
      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["commit"]).assert().failure().code(2);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_commit_log` — expect PASS: `test result: ok. 2 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): commit (compiler invocation + history) with e2e (20.3)"`

---

## Task 11 — `module_path_for` helper

**Files:**
- Modify `crates/digstore-cli/src/ops/store_ops.rs`
- Test: inline in `store_ops.rs`

Steps:

- [ ] **RED.** Add `module_path_for` as `todo!()` + test:
  ```rust
  pub fn module_path_for(
      _ctx: &CliContext,
      _store_id: &Bytes32,
      _root: Option<Bytes32>,
  ) -> Result<PathBuf, CliError> {
      todo!("module_path_for")
  }
  ```
  Test:
  ```rust
      #[test]
      fn module_path_for_resolves_latest_when_root_omitted() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          let f = td.path().join("x.txt");
          std::fs::write(&f, b"data").unwrap();
          add_path(&ctx, &f, Some("x".into())).unwrap();
          let res = commit(&ctx, None).unwrap();
          let store_id = ctx.find_store_id().unwrap();
          let p = module_path_for(&ctx, &store_id, None).unwrap();
          assert!(p.ends_with(format!("{}-{}.wasm", store_id.to_hex(), res.roothash.to_hex())));
      }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::module_path_for_resolves_latest_when_root_omitted` — expect FAIL: `panicked at 'not yet implemented: module_path_for'`.
- [ ] **GREEN.**
  ```rust
  pub fn module_path_for(
      ctx: &CliContext,
      store_id: &Bytes32,
      root: Option<Bytes32>,
  ) -> Result<PathBuf, CliError> {
      let root = match root {
          Some(r) => r,
          None => current_root(ctx)?
              .ok_or_else(|| CliError::NotFound("no committed root; run `digstore commit`".into()))?,
      };
      let path = ctx
          .modules_dir()
          .join(format!("{}-{}.wasm", store_id.to_hex(), root.to_hex()));
      if !path.exists() {
          return Err(CliError::NotFound(format!("module for root {}", root.to_hex())));
      }
      Ok(path)
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::module_path_for_resolves_latest_when_root_omitted` — expect PASS.
- [ ] Commit: `git add -A && git commit -m "feat(cli): module_path_for resolves module by store+root"`

---

## Task 12 — `ops::serve` (host instance over a module; canonical `ContentRequest`)

**Files:**
- Create `crates/digstore-cli/src/ops/serve.rs`
- Modify `crates/digstore-cli/src/ops/mod.rs`
- Test: inline in `serve.rs`

> The request body is the canonical `digstore_core::ContentRequest` (owned by digstore-core), codec-encoded. It carries `retrieval_key` plus optional `root`, `range`, and `validity_window` (§16). This crate never invents a bare-key wire format.

Steps:

- [ ] Add `pub mod serve;` to `ops/mod.rs`.
- [ ] **RED.** Create `serve.rs` with `serve_content`/`serve_proof` as `todo!()` + a `request_for` helper + an integration-style test that uses a real committed module:
  ```rust
  //! Spin a digstore-host instance over a compiled module and call serving exports.

  use std::path::Path;

  use digstore_core::codec::{Decode, Encode};
  use digstore_core::{Bytes32, ContentRequest, ContentResponse, ExecutionProof, HostImportsConfig, ProofResponse, Urn};
  use digstore_host::Host;

  use crate::error::CliError;

  /// Build the canonical ContentRequest from a URN (owned by digstore-core).
  pub fn request_for(urn: &Urn) -> ContentRequest {
      ContentRequest {
          retrieval_key: urn.retrieval_key(),
          root: urn.root_hash,
          range: None,
          validity_window: None,
      }
  }

  pub fn serve_content(_module_path: &Path, _urn: &Urn) -> Result<ContentResponse, CliError> {
      todo!("serve_content")
  }

  pub fn serve_proof(_module_path: &Path, _urn: &Urn) -> Result<(ExecutionProof, Bytes32), CliError> {
      todo!("serve_proof")
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::context::CliContext;
      use crate::ops::store_ops;
      use tempfile::tempdir;

      fn setup_committed() -> (tempfile::TempDir, CliContext, Bytes32, Bytes32) {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          store_ops::init_store(&ctx, false, None).unwrap();
          let f = td.path().join("r.txt");
          std::fs::write(&f, b"the quick brown fox jumps over the lazy dog").unwrap();
          store_ops::add_path(&ctx, &f, Some("r".into())).unwrap();
          let res = store_ops::commit(&ctx, None).unwrap();
          let store_id = ctx.find_store_id().unwrap();
          (td, ctx, store_id, res.roothash)
      }

      #[test]
      fn serve_content_returns_chunks_and_root() {
          let (_td, ctx, store_id, root) = setup_committed();
          let urn = Urn {
              chain: "chia".into(),
              store_id,
              root_hash: Some(root),
              resource_key: Some("r".into()),
          };
          let module_path = ctx
              .modules_dir()
              .join(format!("{}-{}.wasm", store_id.to_hex(), root.to_hex()));
          let resp = serve_content(&module_path, &urn).unwrap();
          assert!(!resp.chunks.is_empty(), "at least one chunk returned");
          assert_eq!(resp.roothash, root);
      }
  }
  ```
- [ ] Run `cargo test -p digstore-cli serve::tests::serve_content_returns_chunks_and_root` — expect FAIL: `panicked at 'not yet implemented: serve_content'`.
- [ ] **GREEN.** Replace the two fns (Host maps `is_error` sentinels to `Err(HostError::Guest(code))`; we re-map to a `CliError`):
  ```rust
  pub fn serve_content(module_path: &Path, urn: &Urn) -> Result<ContentResponse, CliError> {
      let module_bytes = std::fs::read(module_path)
          .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
      let mut host = Host::new(&module_bytes, HostImportsConfig::default())
          .map_err(|e| CliError::Other(anyhow::anyhow!("host init: {e}")))?;
      let req = request_for(urn).encode_to_vec();
      let raw = host.call_get_content(&req).map_err(|e| match e.error_code() {
          Some(code) => CliError::from_error_code(code, &urn.canonical()),
          None => CliError::Other(anyhow::anyhow!("get_content: {e}")),
      })?;
      ContentResponse::decode_from_slice(&raw)
          .map_err(|e| CliError::VerificationFailed(format!("decode content: {e}")))
  }

  pub fn serve_proof(module_path: &Path, urn: &Urn) -> Result<(ExecutionProof, Bytes32), CliError> {
      let module_bytes = std::fs::read(module_path)
          .map_err(|_| CliError::NotFound(module_path.display().to_string()))?;
      let mut host = Host::new(&module_bytes, HostImportsConfig::default())
          .map_err(|e| CliError::Other(anyhow::anyhow!("host init: {e}")))?;
      let req = request_for(urn).encode_to_vec();
      let raw = host.call_get_proof(&req).map_err(|e| match e.error_code() {
          Some(code) => CliError::from_error_code(code, &urn.canonical()),
          None => CliError::Other(anyhow::anyhow!("get_proof: {e}")),
      })?;
      let resp = ProofResponse::decode_from_slice(&raw)
          .map_err(|e| CliError::VerificationFailed(format!("decode proof: {e}")))?;
      Ok((resp.proof, resp.roothash))
  }
  ```
- [ ] Run `cargo test -p digstore-cli serve::tests::serve_content_returns_chunks_and_root` — expect PASS.
- [ ] Commit: `git add -A && git commit -m "feat(cli): serve ops over host instance using canonical ContentRequest"`

---

## Task 13 — `client_crypto::derive_decryption_key` (§11.3/§11.4, shared KDF constants)

**Files:**
- Create `crates/digstore-cli/src/ops/client_crypto.rs`
- Modify `crates/digstore-cli/src/ops/mod.rs`
- Test: inline in `client_crypto.rs`

> The HKDF info/salt-base are the CANONICAL constants `digstore_core::HKDF_INFO` and `digstore_core::HKDF_SALT_BASE` (defined in digstore-core so the compiler that encrypts and this CLI that decrypts provably agree). Private stores append `SecretSalt` to the salt (§11.4).

Steps:

- [ ] Add `pub mod client_crypto;` to `ops/mod.rs`.
- [ ] **RED.** Create `client_crypto.rs` with `derive_decryption_key` as `todo!()` + tests:
  ```rust
  //! Client-side cryptography: key derivation, per-chunk merkle verify, AES-256-GCM open.
  //! All decryption happens HERE (CLIENT-SIDE); the module never decrypts.

  use digstore_core::{Bytes32, ContentResponse, MerkleProof, Urn};

  use crate::error::CliError;

  /// Derive the AES-256 key for a URN (§11.3). For private stores the SecretSalt is
  /// appended to the canonical salt base (§11.4); a wrong/missing salt yields a wrong
  /// key whose GCM tag will not verify. Uses the SHARED canonical constants so the
  /// compiler's encryption and this decryption agree.
  pub fn derive_decryption_key(_urn: &Urn, _secret_salt: Option<&[u8; 32]>) -> [u8; 32] {
      todo!("derive_decryption_key")
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      fn urn() -> Urn {
          Urn {
              chain: "chia".into(),
              store_id: Bytes32([7u8; 32]),
              root_hash: Some(Bytes32([9u8; 32])),
              resource_key: Some("readme".into()),
          }
      }

      #[test]
      fn key_is_deterministic_from_urn() {
          assert_eq!(derive_decryption_key(&urn(), None), derive_decryption_key(&urn(), None));
      }

      #[test]
      fn private_salt_changes_the_key() {
          let public = derive_decryption_key(&urn(), None);
          let private = derive_decryption_key(&urn(), Some(&[3u8; 32]));
          assert_ne!(public, private, "SecretSalt must change the derived key (11.4)");
      }

      #[test]
      fn key_is_32_bytes() {
          assert_eq!(derive_decryption_key(&urn(), None).len(), 32);
      }
  }
  ```
- [ ] Run `cargo test -p digstore-cli client_crypto::tests::key_is_deterministic_from_urn` — expect FAIL: `panicked at 'not yet implemented: derive_decryption_key'`.
- [ ] **GREEN.**
  ```rust
  pub fn derive_decryption_key(urn: &Urn, secret_salt: Option<&[u8; 32]>) -> [u8; 32] {
      let canonical = urn.canonical();
      let ikm = canonical.as_bytes();
      let mut salt = digstore_core::HKDF_SALT_BASE.to_vec();
      if let Some(s) = secret_salt {
          salt.extend_from_slice(s);
      }
      digstore_crypto::hkdf_sha256_32(ikm, &salt, digstore_core::HKDF_INFO)
  }
  ```
- [ ] Run `cargo test -p digstore-cli client_crypto::tests` — expect PASS: `test result: ok. 3 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): client HKDF key derivation w/ shared canonical constants (11.3/11.4)"`

---

## Task 14 — `client_crypto::verify_chunk_inclusion` against trusted root (§9.3)

**Files:**
- Modify `crates/digstore-cli/src/ops/client_crypto.rs`
- Test: inline in `client_crypto.rs`

Steps:

- [ ] **RED.** Add `verify_chunk_inclusion` as `todo!()` + tests:
  ```rust
  /// Verify (§9.3) that `chunk` is the proof's leaf, the path resolves to `proof.root`,
  /// and `proof.root == trusted_root`. leaf=SHA-256(chunk); node=SHA-256(left||right).
  pub fn verify_chunk_inclusion(
      _chunk: &[u8],
      _proof: &MerkleProof,
      _trusted_root: &Bytes32,
  ) -> Result<(), CliError> {
      todo!("verify_chunk_inclusion")
  }
  ```
  Tests:
  ```rust
      #[test]
      fn accepts_valid_single_leaf_and_rejects_wrong_root() {
          let chunk = b"hello".to_vec();
          let leaf = Bytes32(digstore_crypto::sha256(&chunk));
          let proof = MerkleProof { leaf, path: vec![], root: leaf };
          assert!(verify_chunk_inclusion(&chunk, &proof, &leaf).is_ok());
          let bad_root = Bytes32([0xAB; 32]);
          assert!(verify_chunk_inclusion(&chunk, &proof, &bad_root).is_err());
      }

      #[test]
      fn rejects_tampered_chunk() {
          let chunk = b"hello".to_vec();
          let leaf = Bytes32(digstore_crypto::sha256(&chunk));
          let proof = MerkleProof { leaf, path: vec![], root: leaf };
          assert!(verify_chunk_inclusion(b"hellp", &proof, &leaf).is_err());
      }

      #[test]
      fn verifies_two_leaf_path() {
          use digstore_core::ProofStep;
          let chunk = b"left-chunk".to_vec();
          let leaf = Bytes32(digstore_crypto::sha256(&chunk));
          let sibling = Bytes32([0x55; 32]);
          // node = SHA-256(leaf || sibling)  (leaf is left, sibling is right)
          let mut buf = [0u8; 64];
          buf[..32].copy_from_slice(&leaf.0);
          buf[32..].copy_from_slice(&sibling.0);
          let root = Bytes32(digstore_crypto::sha256(&buf));
          let proof = MerkleProof { leaf, path: vec![ProofStep { hash: sibling, is_left: false }], root };
          assert!(verify_chunk_inclusion(&chunk, &proof, &root).is_ok());
      }
  ```
- [ ] Run `cargo test -p digstore-cli client_crypto::tests::accepts_valid_single_leaf_and_rejects_wrong_root` — expect FAIL: `panicked at 'not yet implemented: verify_chunk_inclusion'`.
- [ ] **GREEN.**
  ```rust
  pub fn verify_chunk_inclusion(
      chunk: &[u8],
      proof: &MerkleProof,
      trusted_root: &Bytes32,
  ) -> Result<(), CliError> {
      let computed_leaf = Bytes32(digstore_crypto::sha256(chunk));
      if computed_leaf != proof.leaf {
          return Err(CliError::VerificationFailed("chunk does not match proof leaf (tampered chunk)".into()));
      }
      let mut acc = proof.leaf;
      for step in &proof.path {
          let mut buf = [0u8; 64];
          if step.is_left {
              buf[..32].copy_from_slice(&step.hash.0);
              buf[32..].copy_from_slice(&acc.0);
          } else {
              buf[..32].copy_from_slice(&acc.0);
              buf[32..].copy_from_slice(&step.hash.0);
          }
          acc = Bytes32(digstore_crypto::sha256(&buf));
      }
      if acc != proof.root {
          return Err(CliError::VerificationFailed("merkle path does not resolve to declared root".into()));
      }
      if &proof.root != trusted_root {
          return Err(CliError::VerificationFailed("merkle root does not match trusted root".into()));
      }
      Ok(())
  }
  ```
- [ ] Run `cargo test -p digstore-cli client_crypto::tests::accepts_valid_single_leaf_and_rejects_wrong_root client_crypto::tests::rejects_tampered_chunk client_crypto::tests::verifies_two_leaf_path` — expect PASS.
- [ ] Commit: `git add -A && git commit -m "feat(cli): client merkle inclusion verify against trusted root (9.3)"`

---

## Task 15 — `client_crypto::decrypt_and_verify` (per-chunk verify + AES-GCM open + reassemble)

**Files:**
- Modify `crates/digstore-cli/src/ops/client_crypto.rs`
- Test: inline in `client_crypto.rs`

> **Chunk model (resolved, not deferred):** a resource is N chunks. The host gathers and returns `ContentResponse.chunks: Vec<ContentChunk>`, each `{ ciphertext, proof }`. Each chunk ciphertext is an independent AES-256-GCM ciphertext (its own 16-byte tag) under the SAME URN key (fixed nonce; safe per deviation #4 because the key is unique per URN — every chunk of a resource shares the resource's URN key, and that key is never reused across a different plaintext resource). `decrypt_and_verify` iterates: verify each chunk leaf to the trusted root, AES-GCM-open each chunk, then concatenate the plaintext chunks in order.
>
> **Decoy behavior (resolved, §14.2):** a retrieval miss returns a decoy `ContentResponse` whose `chunks[*].proof.root` equals a fabricated root (NOT the trusted generation root). Therefore the FIRST verification gate that fails for a public-store miss is `verify_chunk_inclusion` (merkle root mismatch → `VerificationFailed`). On the wire the decoy is indistinguishable (success status, real-looking proof blob), but a client holding the URN and the trusted root detects it. Each test below pins which gate fires.

Steps:

- [ ] **RED.** Add `decrypt_and_verify` as `todo!()` + tests (single-chunk happy/tamper, and a fabricated-root decoy):
  ```rust
  /// Full client pipeline (§9.3 + §11): for each chunk verify its merkle proof against
  /// the trusted root, AES-256-GCM open it (tag verified), then concatenate in order.
  pub fn decrypt_and_verify(
      _resp: &ContentResponse,
      _urn: &Urn,
      _secret_salt: Option<&[u8; 32]>,
      _trusted_root: &Bytes32,
  ) -> Result<Vec<u8>, CliError> {
      todo!("decrypt_and_verify")
  }
  ```
  Tests:
  ```rust
      use digstore_core::ContentChunk;

      fn chunk_for(key: &[u8; 32], pt: &[u8]) -> ContentChunk {
          let ct = digstore_crypto::aes256gcm_seal(key, pt);
          let leaf = Bytes32(digstore_crypto::sha256(&ct));
          ContentChunk { ciphertext: ct, proof: MerkleProof { leaf, path: vec![], root: leaf } }
      }

      #[test]
      fn single_chunk_round_trips() {
          let urn = urn();
          let key = derive_decryption_key(&urn, None);
          let pt = b"the quick brown fox".to_vec();
          let c = chunk_for(&key, &pt);
          let root = c.proof.root;
          let resp = ContentResponse {
              ciphertext: c.ciphertext.clone(),
              merkle_proof: c.proof.clone(),
              roothash: root,
              chunks: vec![c],
          };
          assert_eq!(decrypt_and_verify(&resp, &urn, None, &root).unwrap(), pt);
      }

      #[test]
      fn wrong_trusted_root_fails_at_merkle_gate() {
          let urn = urn();
          let key = derive_decryption_key(&urn, None);
          let c = chunk_for(&key, b"data");
          let root = c.proof.root;
          let resp = ContentResponse {
              ciphertext: c.ciphertext.clone(),
              merkle_proof: c.proof.clone(),
              roothash: root,
              chunks: vec![c],
          };
          let err = decrypt_and_verify(&resp, &urn, None, &Bytes32([0xFF; 32])).unwrap_err();
          assert!(matches!(err, CliError::VerificationFailed(ref m) if m.contains("trusted root")));
      }

      #[test]
      fn tampered_ciphertext_fails_at_merkle_gate_first() {
          let urn = urn();
          let key = derive_decryption_key(&urn, None);
          let c = chunk_for(&key, b"data");
          let root = c.proof.root; // leaf computed over the ORIGINAL ciphertext
          let mut bad = c.clone();
          bad.ciphertext[0] ^= 0xFF; // now leaf != SHA-256(ciphertext)
          let resp = ContentResponse {
              ciphertext: bad.ciphertext.clone(),
              merkle_proof: bad.proof.clone(),
              roothash: root,
              chunks: vec![bad],
          };
          let err = decrypt_and_verify(&resp, &urn, None, &root).unwrap_err();
          assert!(matches!(err, CliError::VerificationFailed(ref m) if m.contains("tampered chunk")));
      }

      #[test]
      fn decoy_fabricated_root_fails_at_merkle_gate() {
          // §14.2: a miss returns a decoy whose proof.root is a fabricated value != trusted root.
          let urn = urn();
          let key = derive_decryption_key(&urn, None);
          let c = chunk_for(&key, b"decoy-bytes"); // proof.root == leaf (fabricated)
          let fabricated = c.proof.root;
          let trusted = Bytes32([0x11; 32]); // the real generation root the client trusts
          let resp = ContentResponse {
              ciphertext: c.ciphertext.clone(),
              merkle_proof: c.proof.clone(),
              roothash: fabricated,
              chunks: vec![c],
          };
          let err = decrypt_and_verify(&resp, &urn, None, &trusted).unwrap_err();
          assert!(matches!(err, CliError::VerificationFailed(ref m) if m.contains("trusted root")));
      }

      #[test]
      fn multi_chunk_round_trips_in_order() {
          let urn = urn();
          let key = derive_decryption_key(&urn, None);
          let c0 = chunk_for(&key, b"first-half-");
          let c1 = chunk_for(&key, b"second-half");
          // Two leaves -> a real 2-leaf root. node = SHA-256(leaf0 || leaf1).
          let mut buf = [0u8; 64];
          buf[..32].copy_from_slice(&c0.proof.leaf.0);
          buf[32..].copy_from_slice(&c1.proof.leaf.0);
          let root = Bytes32(digstore_crypto::sha256(&buf));
          let c0 = ContentChunk {
              ciphertext: c0.ciphertext,
              proof: MerkleProof { leaf: c0.proof.leaf, path: vec![digstore_core::ProofStep { hash: c1.proof.leaf, is_left: false }], root },
          };
          let c1 = ContentChunk {
              ciphertext: c1.ciphertext,
              proof: MerkleProof { leaf: c1.proof.leaf, path: vec![digstore_core::ProofStep { hash: c0.proof.leaf, is_left: true }], root },
          };
          let resp = ContentResponse {
              ciphertext: [c0.ciphertext.clone(), c1.ciphertext.clone()].concat(),
              merkle_proof: c0.proof.clone(),
              roothash: root,
              chunks: vec![c0, c1],
          };
          assert_eq!(decrypt_and_verify(&resp, &urn, None, &root).unwrap(), b"first-half-second-half");
      }
  ```
- [ ] Run `cargo test -p digstore-cli client_crypto::tests::single_chunk_round_trips` — expect FAIL: `panicked at 'not yet implemented: decrypt_and_verify'`.
- [ ] **GREEN.**
  ```rust
  pub fn decrypt_and_verify(
      resp: &ContentResponse,
      urn: &Urn,
      secret_salt: Option<&[u8; 32]>,
      trusted_root: &Bytes32,
  ) -> Result<Vec<u8>, CliError> {
      let key = derive_decryption_key(urn, secret_salt);
      let mut plaintext = Vec::new();
      for chunk in &resp.chunks {
          // 1) integrity: this chunk's ciphertext is committed under the trusted root.
          verify_chunk_inclusion(&chunk.ciphertext, &chunk.proof, trusted_root)?;
          // 2) confidentiality: open this chunk (tag verified inside).
          let pt = digstore_crypto::aes256gcm_open(&key, &chunk.ciphertext).map_err(|_| {
              CliError::VerificationFailed(
                  "AES-256-GCM tag verification failed (wrong key/salt or tampered ciphertext)".into(),
              )
          })?;
          plaintext.extend_from_slice(&pt);
      }
      Ok(plaintext)
  }
  ```
- [ ] Run `cargo test -p digstore-cli client_crypto::tests` — expect PASS: `test result: ok. 9 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): per-chunk decrypt_and_verify (merkle + AES-GCM) with decoy/multi-chunk tests (9.3/11/14.2)"`

---

## Task 16 — `cat` command + program_hash-over-template proof check (§20.7 cat, §9.3, §14.2)

**Files:**
- Modify `crates/digstore-cli/src/commands/cat.rs`
- Test: `crates/digstore-cli/tests/cli_cat_roundtrip.rs`

> **`--verify_proof` program_hash (resolved, deviation #3):** `program_hash` committed inside the execution proof is `SHA-256(template guest module bytes)`, NOT the per-store data-injected module. So the CLI compares `proof.program_hash` against `SHA-256(digstore_compiler::template_guest_module_bytes())`, never against the on-disk `.wasm`.

Steps:

- [ ] Implement `commands/cat.rs` (no separate unit test — the behavior is exercised by the round-trip e2e directly after; this is a thin composition of already-tested ops):
  ```rust
  use std::io::Write;

  use digstore_core::{Bytes32, Urn};

  use crate::cli::CatArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::{client_crypto, serve, store_ops};

  pub fn run(ctx: &CliContext, args: CatArgs) -> Result<(), CliError> {
      let urn = Urn::parse(&args.urn)
          .map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;

      let module_path = store_ops::module_path_for(ctx, &urn.store_id, urn.root_hash)?;

      // Trusted root: prefer the URN's root, else the current local root.
      let trusted_root: Bytes32 = match urn.root_hash {
          Some(r) => r,
          None => store_ops::current_root(ctx)?
              .ok_or_else(|| CliError::NotFound("no committed root".into()))?,
      };

      let resp = serve::serve_content(&module_path, &urn)?;

      if args.verify_proof {
          let (proof, root) = serve::serve_proof(&module_path, &urn)?;
          if root != trusted_root {
              return Err(CliError::VerificationFailed("proof root mismatch".into()));
          }
          // program_hash is over the TEMPLATE guest module, not the data-injected module.
          let expected = Bytes32(digstore_crypto::sha256(
              digstore_compiler::template_guest_module_bytes(),
          ));
          if proof.program_hash != expected {
              return Err(CliError::VerificationFailed("program hash mismatch".into()));
          }
      }

      let salt: Option<[u8; 32]> = match &args.salt {
          Some(hex) => Some(
              Bytes32::from_hex(hex)
                  .map_err(|_| CliError::InvalidArgument("salt must be 32-byte hex".into()))?
                  .0,
          ),
          None => None,
      };

      let plaintext = client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), &trusted_root)?;

      std::io::stdout().write_all(&plaintext).map_err(|e| CliError::Other(e.into()))?;
      Ok(())
  }
  ```
- [ ] **RED.** Create `tests/cli_cat_roundtrip.rs`:
  ```rust
  mod common;
  use common::{dig, store_id_and_root, tmp_dig};

  #[test]
  fn add_commit_cat_round_trips_public_store() {
      let dir = tmp_dig();
      let content = b"the quick brown fox jumps over the lazy dog 1234567890";
      let f = dir.path().join("doc.txt");
      std::fs::write(&f, content).unwrap();

      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "doc"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let (store_id, root) = store_id_and_root(&dir);
      let urn = format!("urn:dig:chia:{}:{}/doc", store_id, root);
      let out = dig(&dir).args(["cat", &urn]).output().unwrap();
      assert!(out.status.success(), "cat failed: {}", String::from_utf8_lossy(&out.stderr));
      assert_eq!(out.stdout, content, "cat must return original plaintext");
  }

  #[test]
  fn multi_chunk_resource_round_trips() {
      // >max_size (256 KiB) forces multiple chunks, exercising the per-chunk path.
      let dir = tmp_dig();
      let mut content = Vec::with_capacity(700 * 1024);
      for i in 0..(700 * 1024) {
          content.push((i % 251) as u8);
      }
      let f = dir.path().join("big.bin");
      std::fs::write(&f, &content).unwrap();

      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "big"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let (store_id, root) = store_id_and_root(&dir);
      let urn = format!("urn:dig:chia:{}:{}/big", store_id, root);
      let out = dig(&dir).args(["cat", &urn]).output().unwrap();
      assert!(out.status.success(), "cat failed: {}", String::from_utf8_lossy(&out.stderr));
      assert_eq!(out.stdout, content, "multi-chunk cat must reassemble exactly");
  }

  #[test]
  fn cat_unknown_resource_decoy_fails_verification_exit_5() {
      let dir = tmp_dig();
      let f = dir.path().join("doc.txt");
      std::fs::write(&f, b"real content here").unwrap();
      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "doc"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();
      let (store_id, root) = store_id_and_root(&dir);
      let urn = format!("urn:dig:chia:{}:{}/does-not-exist", store_id, root);
      // Decoy: wire success, fabricated proof root -> merkle gate fails -> exit 5.
      dig(&dir).args(["cat", &urn]).assert().failure().code(5);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_cat_roundtrip` — expect PASS: `test result: ok. 3 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): cat round-trip (single+multi-chunk) + decoy + program_hash-over-template (20.7/9.3/14.2)"`

---

## Task 17 — `checkout` command (client decrypt) + `list_generation_resources` (§20.5)

**Files:**
- Modify `crates/digstore-cli/src/ops/store_ops.rs`, `crates/digstore-cli/src/commands/checkout.rs`
- Test: inline in `store_ops.rs`; `crates/digstore-cli/tests/cli_checkout.rs`

Steps:

- [ ] **RED.** Add `list_generation_resources` as `todo!()` + test:
  ```rust
  pub fn list_generation_resources(_ctx: &CliContext, _root: &Bytes32) -> Result<Vec<String>, CliError> {
      todo!("list_generation_resources")
  }
  ```
  Test:
  ```rust
      #[test]
      fn list_generation_resources_returns_committed_keys() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();
          let f = td.path().join("a.txt");
          std::fs::write(&f, b"alpha").unwrap();
          add_path(&ctx, &f, Some("a".into())).unwrap();
          let res = commit(&ctx, None).unwrap();
          let keys = list_generation_resources(&ctx, &res.roothash).unwrap();
          assert!(keys.iter().any(|k| k == "a"));
      }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::list_generation_resources_returns_committed_keys` — expect FAIL: `panicked at 'not yet implemented: list_generation_resources'`.
- [ ] **GREEN.**
  ```rust
  pub fn list_generation_resources(ctx: &CliContext, root: &Bytes32) -> Result<Vec<String>, CliError> {
      let manifest_path = ctx.generations_dir().join(root.to_hex()).join("manifest.json");
      if !manifest_path.exists() {
          return Err(CliError::NotFound(format!("generation {}", root.to_hex())));
      }
      let text = fs::read_to_string(&manifest_path).map_err(|e| CliError::Other(e.into()))?;
      let manifest: digstore_store::GenerationManifest =
          serde_json::from_str(&text).map_err(|e| CliError::Other(e.into()))?;
      Ok(manifest.resource_keys())
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::list_generation_resources_returns_committed_keys` — expect PASS.
- [ ] Implement `commands/checkout.rs`:
  ```rust
  use std::fs;

  use digstore_core::{Bytes32, Urn};

  use crate::cli::CheckoutArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::{client_crypto, serve, store_ops};

  pub fn run(ctx: &CliContext, args: CheckoutArgs) -> Result<(), CliError> {
      let root = Bytes32::from_hex(&args.root)
          .map_err(|_| CliError::InvalidArgument("root must be 32-byte hex".into()))?;
      let store_id = ctx.find_store_id()?;
      let module_path = store_ops::module_path_for(ctx, &store_id, Some(root))?;

      let salt: Option<[u8; 32]> = match &args.salt {
          Some(hex) => Some(
              Bytes32::from_hex(hex)
                  .map_err(|_| CliError::InvalidArgument("salt must be 32-byte hex".into()))?
                  .0,
          ),
          None => None,
      };

      fs::create_dir_all(&args.out).map_err(|e| CliError::Other(e.into()))?;
      let keys = store_ops::list_generation_resources(ctx, &root)?;
      let mut count = 0usize;
      for key in keys {
          let urn = Urn {
              chain: "chia".into(),
              store_id,
              root_hash: Some(root),
              resource_key: Some(key.clone()),
          };
          let resp = serve::serve_content(&module_path, &urn)?;
          let plaintext = client_crypto::decrypt_and_verify(&resp, &urn, salt.as_ref(), &root)?;
          let dest = args.out.join(&key);
          if let Some(parent) = dest.parent() {
              fs::create_dir_all(parent).map_err(|e| CliError::Other(e.into()))?;
          }
          fs::write(&dest, &plaintext).map_err(|e| CliError::Other(e.into()))?;
          count += 1;
      }
      if ctx.json {
          println!("{}", serde_json::json!({ "root": root.to_hex(), "files": count }));
      } else {
          println!("checked out {} files from {} into {}", count, root.to_hex(), args.out.display());
      }
      Ok(())
  }
  ```
- [ ] **RED (e2e).** Create `tests/cli_checkout.rs`:
  ```rust
  mod common;
  use common::{dig, tmp_dig};

  fn root_hex(dir: &tempfile::TempDir) -> String {
      let out = dig(dir).args(["log", "--json"]).output().unwrap();
      let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
      v[0]["root"].as_str().unwrap().to_string()
  }

  #[test]
  fn checkout_materializes_generation() {
      let dir = tmp_dig();
      let content = b"materialize me please";
      let f = dir.path().join("file.txt");
      std::fs::write(&f, content).unwrap();
      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "file.txt"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let root = root_hex(&dir);
      let out_dir = dir.path().join("out");
      dig(&dir).args(["checkout", &root, "--out"]).arg(&out_dir).assert().success();
      assert_eq!(std::fs::read(out_dir.join("file.txt")).unwrap(), content);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_checkout` — expect PASS: `test result: ok. 1 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): checkout materializes a generation via client decrypt (20.5)"`

---

## Task 18 — `diff` between two generations (§20.4)

**Files:**
- Modify `crates/digstore-cli/src/ops/store_ops.rs`, `crates/digstore-cli/src/commands/diff.rs`
- Test: inline in `store_ops.rs`; `crates/digstore-cli/tests/cli_diff.rs`

Steps:

- [ ] **RED.** Add `diff` + `generation_resource_digests` as `todo!()` + test:
  ```rust
  pub fn diff(_ctx: &CliContext, _from: &Bytes32, _to: &Bytes32) -> Result<Vec<DiffEntry>, CliError> {
      todo!("diff")
  }
  ```
  Test:
  ```rust
      #[test]
      fn diff_reports_added_and_modified_resources() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          init_store(&ctx, false, None).unwrap();

          let f = td.path().join("a.txt");
          std::fs::write(&f, b"v1").unwrap();
          add_path(&ctx, &f, Some("a".into())).unwrap();
          let r1 = commit(&ctx, None).unwrap().roothash;

          std::fs::write(&f, b"v2-different").unwrap();
          add_path(&ctx, &f, Some("a".into())).unwrap();
          let g = td.path().join("b.txt");
          std::fs::write(&g, b"brand new").unwrap();
          add_path(&ctx, &g, Some("b".into())).unwrap();
          let r2 = commit(&ctx, None).unwrap().roothash;

          let d = diff(&ctx, &r1, &r2).unwrap();
          assert!(d.iter().any(|e| e.resource_key == "b" && e.change == "added"));
          assert!(d.iter().any(|e| e.resource_key == "a" && e.change == "modified"));
      }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::diff_reports_added_and_modified_resources` — expect FAIL: `panicked at 'not yet implemented: diff'`.
- [ ] **GREEN.**
  ```rust
  pub fn diff(ctx: &CliContext, from: &Bytes32, to: &Bytes32) -> Result<Vec<DiffEntry>, CliError> {
      let from_map = generation_resource_digests(ctx, from)?;
      let to_map = generation_resource_digests(ctx, to)?;
      let mut out = Vec::new();
      for (key, to_digest) in &to_map {
          match from_map.get(key) {
              None => out.push(DiffEntry { resource_key: key.clone(), change: "added".into() }),
              Some(from_digest) if from_digest != to_digest => {
                  out.push(DiffEntry { resource_key: key.clone(), change: "modified".into() })
              }
              Some(_) => {}
          }
      }
      for key in from_map.keys() {
          if !to_map.contains_key(key) {
              out.push(DiffEntry { resource_key: key.clone(), change: "removed".into() });
          }
      }
      out.sort_by(|a, b| a.resource_key.cmp(&b.resource_key));
      Ok(out)
  }

  fn generation_resource_digests(
      ctx: &CliContext,
      root: &Bytes32,
  ) -> Result<std::collections::BTreeMap<String, Bytes32>, CliError> {
      let manifest_path = ctx.generations_dir().join(root.to_hex()).join("manifest.json");
      if !manifest_path.exists() {
          return Err(CliError::NotFound(format!("generation {}", root.to_hex())));
      }
      let text = fs::read_to_string(&manifest_path).map_err(|e| CliError::Other(e.into()))?;
      let manifest: digstore_store::GenerationManifest =
          serde_json::from_str(&text).map_err(|e| CliError::Other(e.into()))?;
      Ok(manifest.resource_digests())
  }
  ```
- [ ] Run `cargo test -p digstore-cli store_ops::tests::diff_reports_added_and_modified_resources` — expect PASS.
- [ ] Implement `commands/diff.rs`:
  ```rust
  use digstore_core::Bytes32;

  use crate::cli::DiffArgs;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;
  use crate::output;

  pub fn run(ctx: &CliContext, args: DiffArgs) -> Result<(), CliError> {
      let from = Bytes32::from_hex(&args.from)
          .map_err(|_| CliError::InvalidArgument("from must be 32-byte hex".into()))?;
      let to = Bytes32::from_hex(&args.to)
          .map_err(|_| CliError::InvalidArgument("to must be 32-byte hex".into()))?;
      let entries = store_ops::diff(ctx, &from, &to)?;
      print!("{}", output::render_diff(&entries, ctx.json));
      Ok(())
  }
  ```
- [ ] **RED (e2e).** Create `tests/cli_diff.rs`:
  ```rust
  mod common;
  use common::{dig, tmp_dig};
  use predicates::prelude::*;

  fn roots(dir: &tempfile::TempDir) -> Vec<String> {
      let out = dig(dir).args(["log", "--json"]).output().unwrap();
      let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
      v.as_array().unwrap().iter().map(|e| e["root"].as_str().unwrap().to_string()).collect()
  }

  #[test]
  fn diff_two_generations_lists_changes() {
      let dir = tmp_dig();
      dig(&dir).arg("init").assert().success();
      let f = dir.path().join("a.txt");
      std::fs::write(&f, b"one").unwrap();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "a"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let g = dir.path().join("b.txt");
      std::fs::write(&g, b"two new").unwrap();
      dig(&dir).args(["add"]).arg(&g).args(["--key", "b"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let r = roots(&dir); // newest first: r[0]=to, r[1]=from
      dig(&dir).args(["diff", &r[1], &r[0]]).assert().success()
          .stdout(predicate::str::contains("+ b"));
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_diff` — expect PASS: `test result: ok. 1 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): diff between two generations (20.4)"`

---

## Task 19 — Remotes config + `remote` command (§20.6)

**Files:**
- Modify `crates/digstore-cli/src/config.rs`, `crates/digstore-cli/src/commands/remote.rs`
- Test: inline in `config.rs`; add a case to `crates/digstore-cli/tests/cli_remote_clone_push_pull.rs`

Steps:

- [ ] **RED.** Replace `config.rs` with the types + fns as `todo!()` + tests:
  ```rust
  //! CLI-level configuration: the remotes table (`remotes.toml`).

  use std::collections::BTreeMap;
  use std::fs;

  use serde::{Deserialize, Serialize};

  use crate::context::CliContext;
  use crate::error::CliError;

  #[derive(Debug, Default, Serialize, Deserialize)]
  struct RemotesFile {
      #[serde(default)]
      remotes: BTreeMap<String, String>,
  }

  fn remotes_path(ctx: &CliContext) -> std::path::PathBuf {
      ctx.dig_dir.join("remotes.toml")
  }

  pub fn add_remote(_ctx: &CliContext, _name: &str, _url: &str) -> Result<(), CliError> {
      todo!("add_remote")
  }
  pub fn remove_remote(_ctx: &CliContext, _name: &str) -> Result<(), CliError> {
      todo!("remove_remote")
  }
  pub fn list_remotes(_ctx: &CliContext) -> Result<BTreeMap<String, String>, CliError> {
      todo!("list_remotes")
  }
  pub fn resolve_remote_url(_ctx: &CliContext, _name: &str) -> Result<String, CliError> {
      todo!("resolve_remote_url")
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::ops::store_ops;
      use tempfile::tempdir;

      #[test]
      fn add_then_list_remote_persists() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          store_ops::init_store(&ctx, false, None).unwrap();
          add_remote(&ctx, "origin", "https://h/stores/x").unwrap();
          assert_eq!(
              list_remotes(&ctx).unwrap().get("origin").map(String::as_str),
              Some("https://h/stores/x")
          );
      }

      #[test]
      fn remove_remote_deletes_it() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          store_ops::init_store(&ctx, false, None).unwrap();
          add_remote(&ctx, "origin", "https://h").unwrap();
          remove_remote(&ctx, "origin").unwrap();
          assert!(list_remotes(&ctx).unwrap().is_empty());
      }

      #[test]
      fn resolve_remote_url_errors_for_unknown() {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          store_ops::init_store(&ctx, false, None).unwrap();
          assert!(resolve_remote_url(&ctx, "nope").is_err());
      }
  }
  ```
- [ ] Run `cargo test -p digstore-cli config::tests::add_then_list_remote_persists` — expect FAIL: `panicked at 'not yet implemented: add_remote'`.
- [ ] **GREEN.** Add the private I/O helpers and replace the four public fns:
  ```rust
  fn load(ctx: &CliContext) -> Result<RemotesFile, CliError> {
      let p = remotes_path(ctx);
      if !p.exists() {
          return Ok(RemotesFile::default());
      }
      let text = fs::read_to_string(&p).map_err(|e| CliError::Other(e.into()))?;
      toml::from_str(&text).map_err(|e| CliError::Other(e.into()))
  }

  fn save(ctx: &CliContext, f: &RemotesFile) -> Result<(), CliError> {
      let text = toml::to_string_pretty(f).map_err(|e| CliError::Other(e.into()))?;
      fs::write(remotes_path(ctx), text).map_err(|e| CliError::Other(e.into()))
  }

  pub fn add_remote(ctx: &CliContext, name: &str, url: &str) -> Result<(), CliError> {
      let mut f = load(ctx)?;
      f.remotes.insert(name.to_string(), url.to_string());
      save(ctx, &f)
  }

  pub fn remove_remote(ctx: &CliContext, name: &str) -> Result<(), CliError> {
      let mut f = load(ctx)?;
      if f.remotes.remove(name).is_none() {
          return Err(CliError::NotFound(format!("remote {name}")));
      }
      save(ctx, &f)
  }

  pub fn list_remotes(ctx: &CliContext) -> Result<BTreeMap<String, String>, CliError> {
      Ok(load(ctx)?.remotes)
  }

  pub fn resolve_remote_url(ctx: &CliContext, name: &str) -> Result<String, CliError> {
      list_remotes(ctx)?
          .get(name)
          .cloned()
          .ok_or_else(|| CliError::NotFound(format!("remote {name}")))
  }
  ```
- [ ] Run `cargo test -p digstore-cli config::tests` — expect PASS: `test result: ok. 3 passed`.
- [ ] Implement `commands/remote.rs`:
  ```rust
  use crate::cli::{RemoteAction, RemoteArgs};
  use crate::config;
  use crate::context::CliContext;
  use crate::error::CliError;

  pub fn run(ctx: &CliContext, args: RemoteArgs) -> Result<(), CliError> {
      match args.action {
          RemoteAction::Add { name, url } => {
              config::add_remote(ctx, &name, &url)?;
              if !ctx.json {
                  println!("added remote {name} -> {url}");
              }
          }
          RemoteAction::Remove { name } => {
              config::remove_remote(ctx, &name)?;
              if !ctx.json {
                  println!("removed remote {name}");
              }
          }
          RemoteAction::List => {
              let remotes = config::list_remotes(ctx)?;
              if ctx.json {
                  println!("{}", serde_json::to_string_pretty(&remotes).unwrap());
              } else {
                  for (name, url) in remotes {
                      println!("{name}\t{url}");
                  }
              }
          }
      }
      Ok(())
  }
  ```
- [ ] **RED (e2e).** Create `tests/cli_remote_clone_push_pull.rs` with the remote case (clone/push/pull cases added in Task 21):
  ```rust
  mod common;
  use common::{dig, tmp_dig};
  use predicates::prelude::*;

  #[test]
  fn remote_add_and_list_persists() {
      let dir = tmp_dig();
      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["remote", "add", "origin", "https://example/stores/abc"]).assert().success();
      dig(&dir).args(["remote", "list"]).assert().success()
          .stdout(predicate::str::contains("origin").and(predicate::str::contains("example")));
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_remote_clone_push_pull remote_add_and_list_persists` — expect PASS.
- [ ] Commit: `git add -A && git commit -m "feat(cli): remote add/list/remove with persisted remotes.toml (20.6)"`

---

## Task 20 — `remote_ops`: typed error mapping + `clone_from` (real verification, real ids) (§20.7, §21)

**Files:**
- Create `crates/digstore-cli/src/ops/remote_ops.rs`
- Modify `crates/digstore-cli/src/ops/mod.rs`
- Test: inline in `remote_ops.rs`

> Clone VERIFIES what it installs: it compares the downloaded module's HEAD `etag_root` (the remote's ETag=root) and the descriptor `root` for agreement, then writes. It records the REAL generation id/timestamp fetched from `/roots`, not a fabricated `id=1`. Error mapping matches the TYPED `RemoteError` enum, never Display strings.

Steps:

- [ ] Add `pub mod remote_ops;` to `ops/mod.rs`.
- [ ] **RED.** Create `remote_ops.rs` with `map_remote_err`, `CloneSummary`, `clone_from` (the latter `todo!()`) + a test booting the test server:
  ```rust
  //! Remote operations: clone, push, pull over the digstore-remote reqwest client.

  use std::fs;

  use digstore_core::{Bytes32, GenerationState};
  use digstore_remote::client::RemoteClient;
  use digstore_remote::RemoteError;
  use digstore_store::{StoreConfig, Visibility};

  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::store_ops;

  #[derive(Debug)]
  pub struct CloneSummary {
      pub store_id_hex: String,
      pub root_hex: String,
      pub module_size: u64,
  }

  /// Map the TYPED remote error enum to a CliError (never Display-string matching).
  pub(crate) fn map_remote_err(e: RemoteError) -> CliError {
      match e {
          RemoteError::NonFastForward => CliError::NonFastForward,
          RemoteError::Unauthorized => CliError::Unauthorized("remote rejected credentials".into()),
          RemoteError::NotFound => CliError::NotFound("remote resource".into()),
          RemoteError::Status(code) => CliError::Network(format!("remote status {code}")),
          RemoteError::Network(msg) => CliError::Network(msg),
      }
  }

  pub async fn clone_from(_ctx: &CliContext, _base_url: &str) -> Result<CloneSummary, CliError> {
      todo!("clone_from")
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use tempfile::tempdir;

      fn committed() -> (tempfile::TempDir, String, String, Vec<u8>) {
          let td = tempdir().unwrap();
          let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
          store_ops::init_store(&ctx, false, None).unwrap();
          let f = td.path().join("a.txt");
          std::fs::write(&f, b"clone me over the wire").unwrap();
          store_ops::add_path(&ctx, &f, Some("a".into())).unwrap();
          let res = store_ops::commit(&ctx, None).unwrap();
          let sid = ctx.find_store_id().unwrap();
          let module = std::fs::read(
              ctx.modules_dir().join(format!("{}-{}.wasm", sid.to_hex(), res.roothash.to_hex())),
          )
          .unwrap();
          (td, sid.to_hex(), res.roothash.to_hex(), module)
      }

      #[tokio::test(flavor = "current_thread")]
      async fn clone_fetches_and_verifies_module_into_new_dig_dir() {
          let (_src_td, store_id, root, module) = committed();
          let server =
              digstore_remote::testing::TestServer::start_with_module(&store_id, &root, &module).await;
          let base = server.base_url();

          let dst_td = tempdir().unwrap();
          let dst = CliContext::resolve(Some(dst_td.path().to_path_buf()), false, false);
          let url = format!("{base}/stores/{store_id}");
          let summary = clone_from(&dst, &url).await.unwrap();

          assert_eq!(summary.store_id_hex, store_id);
          assert_eq!(summary.root_hex, root);
          assert!(dst.modules_dir().join(format!("{store_id}-{root}.wasm")).exists());
      }
  }
  ```
- [ ] Run `cargo test -p digstore-cli remote_ops::tests::clone_fetches_and_verifies_module_into_new_dig_dir` — expect FAIL: `panicked at 'not yet implemented: clone_from'`.
- [ ] **GREEN.**
  ```rust
  pub async fn clone_from(ctx: &CliContext, base_url: &str) -> Result<CloneSummary, CliError> {
      if ctx.config_path().exists() {
          return Err(CliError::InvalidArgument(
              "dig dir already has a store; clone into an empty dir".into(),
          ));
      }
      let client = RemoteClient::new(base_url).map_err(map_remote_err)?;

      // 1) descriptor (current root, size, pubkey) and HEAD (ETag=root).
      let desc = client.get_descriptor().await.map_err(map_remote_err)?;
      let head = client.head_module().await.map_err(map_remote_err)?;
      if !head.exists {
          return Err(CliError::NotFound("remote module".into()));
      }
      // 2) the descriptor root and the module ETag root MUST agree before we trust anything.
      if head.etag_root != desc.root {
          return Err(CliError::VerificationFailed(
              "descriptor root and module ETag disagree".into(),
          ));
      }

      // 3) the REAL generation (id + timestamp) for this root from /roots.
      let roots = client.get_roots().await.map_err(map_remote_err)?;
      let gen_state = roots
          .into_iter()
          .find(|s| s.root == desc.root)
          .ok_or_else(|| CliError::VerificationFailed("root not present in remote /roots".into()))?;

      // 4) download module and verify its size matches what HEAD advertised.
      let module = client.get_module().await.map_err(map_remote_err)?;
      if module.len() as u64 != head.size {
          return Err(CliError::VerificationFailed("module size mismatch vs HEAD".into()));
      }

      // 5) install layout.
      fs::create_dir_all(&ctx.dig_dir).map_err(|e| CliError::Other(e.into()))?;
      fs::create_dir_all(ctx.modules_dir()).map_err(|e| CliError::Other(e.into()))?;
      fs::create_dir_all(ctx.generations_dir()).map_err(|e| CliError::Other(e.into()))?;

      let cfg = StoreConfig {
          store_id: desc.store_id,
          data_dir: ctx.dig_dir.display().to_string(),
          max_size: 1024 * 1024 * 1024,
          visibility: Visibility::Public,
      };
      fs::write(
          ctx.config_path(),
          toml::to_string_pretty(&cfg).map_err(|e| CliError::Other(e.into()))?,
      )
      .map_err(|e| CliError::Other(e.into()))?;

      let module_path = ctx
          .modules_dir()
          .join(format!("{}-{}.wasm", desc.store_id.to_hex(), desc.root.to_hex()));
      fs::write(&module_path, &module).map_err(|e| CliError::Other(e.into()))?;

      store_ops::append_history(
          ctx,
          GenerationState { id: gen_state.id, root: desc.root, timestamp: gen_state.timestamp },
      )?;

      Ok(CloneSummary {
          store_id_hex: desc.store_id.to_hex(),
          root_hex: desc.root.to_hex(),
          module_size: module.len() as u64,
      })
  }
  ```
- [ ] Run `cargo test -p digstore-cli remote_ops::tests::clone_fetches_and_verifies_module_into_new_dig_dir` — expect PASS.
- [ ] Commit: `git add -A && git commit -m "feat(cli): clone_from with real verification + real generation ids (20.7/21)"`

---

## Task 21 — `remote_ops::push_to` (exact push-auth message) + `push` command (§20.7, §21.6)

**Files:**
- Modify `crates/digstore-cli/src/ops/remote_ops.rs`, `crates/digstore-cli/src/commands/push.rs`
- Test: inline in `remote_ops.rs`

> **Push-auth signed message (pinned exactly):** `msg = SHA-256(store_id.0 ‖ root.0)`, signed with AugScheme using the store secret key. This MUST byte-match digstore-remote's verification (which loads the store public key from the descriptor and AugScheme-verifies the same `msg`). The signing key is `SecretKeyBytes` loaded via `secret_from_bytes` from the 32-byte `signing_key.bin`.

Steps:

- [ ] **RED.** Add a helper `push_auth_message` (pure, testable) + `push_to` (`todo!()`) and a unit test for the message construction:
  ```rust
  use digstore_core::Bytes96;

  /// The exact bytes signed for push-auth: SHA-256(store_id || root).
  pub(crate) fn push_auth_message(store_id: &Bytes32, root: &Bytes32) -> [u8; 32] {
      let mut buf = [0u8; 64];
      buf[..32].copy_from_slice(&store_id.0);
      buf[32..].copy_from_slice(&root.0);
      digstore_crypto::sha256(&buf)
  }

  pub async fn push_to(_ctx: &CliContext, _base_url: &str) -> Result<Bytes32, CliError> {
      todo!("push_to")
  }
  ```
  Add test:
  ```rust
      #[test]
      fn push_auth_message_is_sha256_of_store_id_concat_root() {
          let sid = Bytes32([1u8; 32]);
          let root = Bytes32([2u8; 32]);
          let mut expect = [0u8; 64];
          expect[..32].copy_from_slice(&sid.0);
          expect[32..].copy_from_slice(&root.0);
          assert_eq!(push_auth_message(&sid, &root), digstore_crypto::sha256(&expect));
      }
  ```
- [ ] Run `cargo test -p digstore-cli remote_ops::tests::push_auth_message_is_sha256_of_store_id_concat_root` — expect FAIL initially because `push_to`'s `todo!()` body has no test; the message test should compile and PASS once `push_auth_message` exists. Run it: expect PASS. (This is the green for the pure helper.)
- [ ] **GREEN (push_to).** Replace `push_to`:
  ```rust
  pub async fn push_to(ctx: &CliContext, base_url: &str) -> Result<Bytes32, CliError> {
      let cfg = ctx.load_config()?;
      let root = store_ops::current_root(ctx)?
          .ok_or_else(|| CliError::NotFound("no committed root to push".into()))?;
      let module_path = store_ops::module_path_for(ctx, &cfg.store_id, Some(root))?;
      let module = fs::read(&module_path).map_err(|e| CliError::Other(e.into()))?;

      // Load the store secret key (32 raw bytes) and AugScheme-sign the pinned message.
      let key_bytes = fs::read(ctx.dig_dir.join("signing_key.bin"))
          .map_err(|e| CliError::Other(e.into()))?;
      let mut arr = [0u8; 32];
      if key_bytes.len() != 32 {
          return Err(CliError::Other(anyhow::anyhow!("signing key must be 32 bytes")));
      }
      arr.copy_from_slice(&key_bytes);
      let sk = digstore_crypto::host_bls::secret_from_bytes(&arr)
          .map_err(|e| CliError::Other(anyhow::anyhow!("bad signing key: {e}")))?;
      let msg = push_auth_message(&cfg.store_id, &root);
      let signature: Bytes96 = digstore_crypto::host_bls::sign_aug(&sk, &msg);

      let client = RemoteClient::new(base_url).map_err(map_remote_err)?;
      client
          .push_module(&cfg.store_id, &root, &module, &signature)
          .await
          .map_err(map_remote_err)?;
      Ok(root)
  }
  ```
- [ ] Run `cargo build -p digstore-cli` — expect compile.
- [ ] Implement `commands/push.rs`:
  ```rust
  use crate::cli::PushArgs;
  use crate::config;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::remote_ops;

  pub fn run(ctx: &CliContext, args: PushArgs) -> Result<(), CliError> {
      let base = config::resolve_remote_url(ctx, &args.remote)?;
      let rt = tokio::runtime::Builder::new_current_thread()
          .enable_all()
          .build()
          .map_err(|e| CliError::Other(e.into()))?;
      let root = rt.block_on(remote_ops::push_to(ctx, &base))?;
      if ctx.json {
          println!("{}", serde_json::json!({ "pushed_root": root.to_hex() }));
      } else {
          println!("pushed root {} to {}", root.to_hex(), args.remote);
      }
      Ok(())
  }
  ```
- [ ] Commit: `git add -A && git commit -m "feat(cli): push_to with pinned AugScheme push-auth message + push command (20.7/21.6)"`

---

## Task 22 — `remote_ops::pull_from` (real ids from /roots) + `pull` command (§20.7, §21.4)

**Files:**
- Modify `crates/digstore-cli/src/ops/remote_ops.rs`, `crates/digstore-cli/src/commands/pull.rs`
- Test: inline in `remote_ops.rs`

Steps:

- [ ] **RED.** Add `pull_from` as `todo!()` + a test that pushes to an empty server then pulls into a clone:
  ```rust
  pub async fn pull_from(_ctx: &CliContext, _base_url: &str) -> Result<Bytes32, CliError> {
      todo!("pull_from")
  }
  ```
  Test:
  ```rust
      #[tokio::test(flavor = "current_thread")]
      async fn pull_advances_to_remote_root() {
          // build + push a source store
          let (src_td, store_id, root, module) = committed();
          let server =
              digstore_remote::testing::TestServer::start_with_module(&store_id, &root, &module).await;
          let base = server.base_url();
          let store_url = format!("{base}/stores/{store_id}");

          // clone into a fresh dir
          let dst_td = tempdir().unwrap();
          let dst = CliContext::resolve(Some(dst_td.path().to_path_buf()), false, false);
          clone_from(&dst, &store_url).await.unwrap();
          assert_eq!(store_ops::current_root(&dst).unwrap().unwrap().to_hex(), root);

          // pull when already up-to-date returns the same root
          let r = pull_from(&dst, &store_url).await.unwrap();
          assert_eq!(r.to_hex(), root);
          drop(src_td);
      }
  ```
- [ ] Run `cargo test -p digstore-cli remote_ops::tests::pull_advances_to_remote_root` — expect FAIL: `panicked at 'not yet implemented: pull_from'`.
- [ ] **GREEN.**
  ```rust
  pub async fn pull_from(ctx: &CliContext, base_url: &str) -> Result<Bytes32, CliError> {
      let cfg = ctx.load_config()?;
      let client = RemoteClient::new(base_url).map_err(map_remote_err)?;
      let desc = client.get_descriptor().await.map_err(map_remote_err)?;

      if store_ops::current_root(ctx)? == Some(desc.root) {
          return Ok(desc.root); // already up to date
      }

      // Verify the advertised module before installing (ETag=root agreement + size).
      let head = client.head_module().await.map_err(map_remote_err)?;
      if !head.exists || head.etag_root != desc.root {
          return Err(CliError::VerificationFailed("remote module ETag != descriptor root".into()));
      }
      let roots = client.get_roots().await.map_err(map_remote_err)?;
      let gen_state = roots
          .into_iter()
          .find(|s| s.root == desc.root)
          .ok_or_else(|| CliError::VerificationFailed("root not present in remote /roots".into()))?;
      let module = client.get_module().await.map_err(map_remote_err)?;
      if module.len() as u64 != head.size {
          return Err(CliError::VerificationFailed("module size mismatch vs HEAD".into()));
      }

      let module_path = ctx
          .modules_dir()
          .join(format!("{}-{}.wasm", cfg.store_id.to_hex(), desc.root.to_hex()));
      fs::write(&module_path, &module).map_err(|e| CliError::Other(e.into()))?;

      store_ops::append_history(
          ctx,
          GenerationState { id: gen_state.id, root: desc.root, timestamp: gen_state.timestamp },
      )?;
      Ok(desc.root)
  }
  ```
- [ ] Run `cargo test -p digstore-cli remote_ops::tests::pull_advances_to_remote_root` — expect PASS.
- [ ] Implement `commands/pull.rs`:
  ```rust
  use crate::cli::PullArgs;
  use crate::config;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::remote_ops;

  pub fn run(ctx: &CliContext, args: PullArgs) -> Result<(), CliError> {
      let base = config::resolve_remote_url(ctx, &args.remote)?;
      let rt = tokio::runtime::Builder::new_current_thread()
          .enable_all()
          .build()
          .map_err(|e| CliError::Other(e.into()))?;
      let root = rt.block_on(remote_ops::pull_from(ctx, &base))?;
      if ctx.json {
          println!("{}", serde_json::json!({ "root": root.to_hex() }));
      } else {
          println!("pulled; local root is now {}", root.to_hex());
      }
      Ok(())
  }
  ```
- [ ] Commit: `git add -A && git commit -m "feat(cli): pull_from (real ids from /roots) + pull command (20.7/21.4)"`

---

## Task 23 — `clone` command + full remote round-trip E2E (§20.7)

**Files:**
- Modify `crates/digstore-cli/src/commands/clone.rs`
- Test: add cases to `crates/digstore-cli/tests/cli_remote_clone_push_pull.rs`

Steps:

- [ ] Implement `commands/clone.rs` (resolves a URN to `origin`'s base + store path, or accepts a raw URL):
  ```rust
  use digstore_core::Urn;

  use crate::cli::CloneArgs;
  use crate::config;
  use crate::context::CliContext;
  use crate::error::CliError;
  use crate::ops::remote_ops;

  pub fn run(ctx: &CliContext, args: CloneArgs) -> Result<(), CliError> {
      let base_url = if args.source.starts_with("urn:dig:") {
          let urn = Urn::parse(&args.source)
              .map_err(|e| CliError::InvalidArgument(format!("bad urn: {e}")))?;
          let base = config::resolve_remote_url(ctx, "origin").map_err(|_| {
              CliError::InvalidArgument("cloning a URN requires a configured `origin` remote".into())
          })?;
          format!("{}/stores/{}", base.trim_end_matches('/'), urn.store_id.to_hex())
      } else {
          args.source.clone()
      };

      let rt = tokio::runtime::Builder::new_current_thread()
          .enable_all()
          .build()
          .map_err(|e| CliError::Other(e.into()))?;
      let summary = rt.block_on(remote_ops::clone_from(ctx, &base_url))?;

      if ctx.json {
          println!(
              "{}",
              serde_json::json!({ "store_id": summary.store_id_hex, "root": summary.root_hex, "module_size": summary.module_size })
          );
      } else {
          println!("cloned {} at root {} ({} bytes)", summary.store_id_hex, summary.root_hex, summary.module_size);
      }
      Ok(())
  }
  ```
- [ ] **RED (e2e).** Add to `tests/cli_remote_clone_push_pull.rs`:
  ```rust
  #[test]
  fn clone_then_cat_round_trips_from_remote() {
      let src = tmp_dig();
      let content = b"served from a remote digstore";
      let f = src.path().join("doc.txt");
      std::fs::write(&f, content).unwrap();
      dig(&src).arg("init").assert().success();
      dig(&src).args(["add"]).arg(&f).args(["--key", "doc"]).assert().success();
      dig(&src).args(["commit"]).assert().success();

      let (store_id, root) = common::store_id_and_root(&src);
      let module = std::fs::read(
          src.path().join("modules").join(format!("{store_id}-{root}.wasm")),
      )
      .unwrap();

      let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
      let base = rt.block_on(async {
          digstore_remote::testing::TestServer::start_with_module(&store_id, &root, &module)
              .await
              .base_url()
      });

      let dst = tmp_dig();
      let url = format!("{base}/stores/{store_id}");
      dig(&dst).args(["clone", &url]).assert().success();

      let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
      let cat = dig(&dst).args(["cat", &urn]).output().unwrap();
      assert!(cat.status.success());
      assert_eq!(cat.stdout, content);
      drop(rt);
  }

  #[test]
  fn push_fast_forward_then_pull_advances() {
      let src = tmp_dig();
      let f = src.path().join("a.txt");
      std::fs::write(&f, b"v1").unwrap();
      dig(&src).arg("init").assert().success();
      dig(&src).args(["add"]).arg(&f).args(["--key", "a"]).assert().success();
      dig(&src).args(["commit"]).assert().success();

      let (store_id, root1) = common::store_id_and_root(&src);

      let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
      let base = rt.block_on(async {
          digstore_remote::testing::TestServer::start_empty(&store_id).await.base_url()
      });
      let store_url = format!("{base}/stores/{store_id}");

      dig(&src).args(["remote", "add", "origin", &store_url]).assert().success();
      dig(&src).args(["push", "origin"]).assert().success();

      let dst = tmp_dig();
      dig(&dst).args(["clone", &store_url]).assert().success();

      std::fs::write(&f, b"v2-longer-content").unwrap();
      dig(&src).args(["add"]).arg(&f).args(["--key", "a"]).assert().success();
      dig(&src).args(["commit"]).assert().success();
      dig(&src).args(["push", "origin"]).assert().success();

      let (_sid2, root2) = common::store_id_and_root(&src);
      assert_ne!(root1, root2);

      dig(&dst).args(["remote", "add", "origin", &store_url]).assert().success();
      dig(&dst).args(["pull", "origin"]).assert().success();
      let outd: serde_json::Value =
          serde_json::from_slice(&dig(&dst).args(["log", "--json"]).output().unwrap().stdout).unwrap();
      assert_eq!(outd[0]["root"].as_str().unwrap(), root2);
      drop(rt);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_remote_clone_push_pull` — expect PASS: `test result: ok. 3 passed`.
- [ ] Commit: `git add -A && git commit -m "feat(cli): clone command + full clone/push/pull round-trip e2e (20.7)"`

---

## Task 24 — Private-store salt E2E (§11.4)

**Files:**
- Test: `crates/digstore-cli/tests/cli_private_salt.rs`

> The salt is read from the deterministic `secret_salt.hex` file written by `init --private` (Task 7), not scraped from TOML.

Steps:

- [ ] **RED (e2e).** Create `tests/cli_private_salt.rs`:
  ```rust
  mod common;
  use common::{dig, store_id_and_root, tmp_dig};

  #[test]
  fn private_cat_without_salt_fails_with_salt_succeeds() {
      let dir = tmp_dig();
      let content = b"secret private payload";
      let f = dir.path().join("s.txt");
      std::fs::write(&f, content).unwrap();

      dig(&dir).args(["init", "--private"]).assert().success();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "s"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let (store_id, root) = store_id_and_root(&dir);
      let urn = format!("urn:dig:chia:{}:{}/s", store_id, root);

      // WITHOUT salt -> wrong key -> AES-GCM tag fails -> exit 5.
      dig(&dir).args(["cat", &urn]).assert().failure().code(5);

      // WITH salt (read from the deterministic secret_salt.hex file) -> plaintext.
      let salt = std::fs::read_to_string(dir.path().join("secret_salt.hex")).unwrap();
      let salt = salt.trim();
      let out = dig(&dir).args(["cat", &urn, "--salt", salt]).output().unwrap();
      assert!(out.status.success(), "cat --salt failed: {}", String::from_utf8_lossy(&out.stderr));
      assert_eq!(out.stdout, content);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_private_salt` — expect PASS: `test result: ok. 1 passed`.

  > Note: for a private store, a real-hit chunk DOES chain to the trusted root (merkle gate passes), so the failing gate WITHOUT the salt is the AES-GCM tag (wrong key). This is distinct from a public-store miss (Task 16) where the merkle gate fails first — both surface as exit 5 but via different gates, as pinned in Task 15's unit tests.

- [ ] Commit: `git add -A && git commit -m "test(cli): private-store cat without/with salt via secret_salt.hex (11.4)"`

---

## Task 25 — Tamper E2E with deterministic data-section corruption (§9.3)

**Files:**
- Modify `crates/digstore-cli/tests/common/mod.rs` (add a data-section locator/corruptor)
- Test: `crates/digstore-cli/tests/cli_tamper.rs`

> The corruption deterministically targets the injected data section by locating the `DIGS` magic and flipping a byte inside a chunk-ciphertext region, so the host still instantiates the module (no code corruption) and the failure is a CLIENT-side merkle/GCM verification (exit 5), not a module-load error.

Steps:

- [ ] Add to `tests/common/mod.rs`:
  ```rust
  /// Locate the injected data section by its `DIGS` magic and flip one byte well past
  /// the header/offset-table, landing inside a chunk-ciphertext region. Deterministic.
  pub fn corrupt_data_section(module_path: &std::path::Path) {
      let mut bytes = std::fs::read(module_path).unwrap();
      let magic = b"DIGS";
      let start = bytes
          .windows(magic.len())
          .position(|w| w == magic)
          .expect("DIGS magic present in compiled module");
      // Skip magic(4) + version(1) + a generous offset-table allowance, then flip a byte.
      let target = start + 4 + 1 + 256;
      assert!(target < bytes.len(), "module too small to corrupt deterministically");
      bytes[target] ^= 0xFF;
      std::fs::write(module_path, &bytes).unwrap();
  }
  ```
- [ ] **RED (e2e).** Create `tests/cli_tamper.rs`:
  ```rust
  mod common;
  use common::{corrupt_data_section, dig, store_id_and_root, tmp_dig};

  #[test]
  fn tampered_data_section_fails_client_verification_exit_5() {
      let dir = tmp_dig();
      let f = dir.path().join("doc.txt");
      std::fs::write(&f, b"important verified content that spans one chunk").unwrap();
      dig(&dir).arg("init").assert().success();
      dig(&dir).args(["add"]).arg(&f).args(["--key", "doc"]).assert().success();
      dig(&dir).args(["commit"]).assert().success();

      let (store_id, root) = store_id_and_root(&dir);
      let module = dir.path().join("modules").join(format!("{store_id}-{root}.wasm"));
      corrupt_data_section(&module);

      let urn = format!("urn:dig:chia:{store_id}:{root}/doc");
      // Client merkle/GCM verification must fail -> exit 5 (VerificationFailed).
      dig(&dir).args(["cat", &urn]).assert().failure().code(5);
  }
  ```
- [ ] Run `cargo test -p digstore-cli --test cli_tamper` — expect PASS: `test result: ok. 1 passed`.

  > If this lands on a byte the host validator rejects at instantiation (which would surface as exit 1, not 5), the corruption offset is wrong relative to the data-section layout — adjust `target` to remain within the chunk pool per the compiler's section table. The contract is: corruption inside the chunk pool → client exit 5.

- [ ] Commit: `git add -A && git commit -m "test(cli): tampered data section fails client merkle/GCM verify, exit 5 (9.3)"`

---

## Task 26 — Workspace integration + clippy/fmt gate

**Files:**
- None new; modify only if lint fixes are required.

Steps:

- [ ] Run `cargo build --workspace` — expect the whole workspace compiles with `digstore-cli` wired in.
- [ ] Run `cargo test -p digstore-cli` — expect every unit + e2e test PASS across all test files.
- [ ] Run `cargo clippy -p digstore-cli --all-targets -- -D warnings` — fix any lints (needless clones, redundant `map_err`, unused imports) and re-run until clean.
- [ ] Run `cargo fmt -p digstore-cli -- --check` — if it reports a diff, run `cargo fmt -p digstore-cli` and re-check until clean.
- [ ] Commit: `git add -A && git commit -m "chore(cli): clippy + fmt clean; workspace integration green"`

---

## Definition of Done

The crate is done when every box below is checked, mapping each assigned paper section to its task(s):

- [ ] **§20.1 (init)** — `digstore init` creates the layout, config (with a nonzero default `max_size`), staging, signing key, `secret_salt.hex` for private stores, and ≥1 trusted host key; `store_id = SHA-256(BLS public key)`. (Task 7)
- [ ] **§20.2 (add)** — `digstore add <path>` stages + chunks a file under a resource key, enforcing `StoreConfig.max_size`; `status` shows it. (Task 9)
- [ ] **§20.3 (commit / status / log)** — `commit` invokes `digstore-compiler` with the single canonical `TrustedHostKey`, writes the module, appends the real generation id/root/timestamp; `log` lists generations newest-first; `status` shows staged-vs-committed. (Tasks 8, 9, 10)
- [ ] **§20.4 (diff)** — `diff <a> <b>` reports added/removed/modified resources between two generations via per-resource digests. (Task 18)
- [ ] **§20.5 (checkout)** — `checkout <root> --out` materializes a generation by serving + client-decrypting each resource. (Task 17)
- [ ] **§20.6 (remote)** — `remote add/list/remove` persists named remotes in `remotes.toml`. (Task 19)
- [ ] **§20.7 (clone / push / pull, cat)** — `cat <urn>` single- and multi-chunk add→commit→cat round trip; `clone` from a `digstore-remote` server (with ETag=root verification + real generation ids) then `cat`; `push` fast-forward (NonFastForward → exit 7) with the pinned AugScheme push-auth message; `pull` advances local root. (Tasks 16, 20, 21, 22, 23)
- [ ] **§11.3 (client decrypt)** — `derive_decryption_key` (HKDF-SHA256 over canonical URN, shared canonical constants) + per-chunk AES-256-GCM open with tag verification, client-side only; module never decrypts. (Tasks 13, 15)
- [ ] **§11.4 (private-store salt)** — private store cat WITHOUT salt fails at the GCM gate (exit 5); WITH the SecretSalt from `secret_salt.hex` succeeds. (Tasks 7, 13, 24)
- [ ] **§9.3 (client verify)** — `verify_chunk_inclusion` verifies each chunk's merkle proof to the trusted generation root; tampered data section → exit 5; optional `--verify_proof` compares `program_hash` against `SHA-256(template guest module)`. (Tasks 14, 15, 16, 25)
- [ ] **Decoys (§14.2) surfaced correctly** — a public-store retrieval miss returns wire-success with a fabricated proof root; the client detects it at the merkle gate (exit 5), indistinguishable from a real miss on the wire to a party without the trusted root. (Tasks 15, 16)
- [ ] **Exit codes + human/JSON output** — distinct nonzero exit codes per error class (2 invalid-arg, 3 no-store, 4 not-found, 5 verification, 6 network, 7 non-fast-forward, 8 unauthorized); `--json` available globally (placement proven by tests). (Tasks 2, 4, 5, 6)
- [ ] All unit + e2e tests pass; `cargo clippy -D warnings` and `cargo fmt --check` clean; `cargo build --workspace` green. (Task 26)


---

## Plan metadata

- **Crate:** digstore-cli
- **Assigned paper sections:** 20.1,20.2,20.3,20.4,20.5,20.6,20.7,11.3(client decrypt),9.3(client verify)
- **Depends on:** digstore-core, digstore-store, digstore-compiler, digstore-host, digstore-remote, digstore-crypto
- **Spec sections covered (claimed):** 20.1, 20.2, 20.3, 20.4, 20.5, 20.6, 20.7, 11.3, 11.4, 9.3, 14.2

### Public items exported (consumed by other crates)

```
pub fn digstore_cli::commands::dispatch(cli: digstore_cli::cli::Cli) -> Result<(), digstore_cli::error::CliError>
pub struct digstore_cli::cli::Cli { pub dig_dir: Option<std::path::PathBuf>, pub json: bool, pub verbose: bool, pub command: digstore_cli::cli::Command }
pub enum digstore_cli::cli::Command { Init(InitArgs), Add(AddArgs), Commit(CommitArgs), Status(StatusArgs), Log(LogArgs), Diff(DiffArgs), Checkout(CheckoutArgs), Cat(CatArgs), Remote(RemoteArgs), Clone(CloneArgs), Push(PushArgs), Pull(PullArgs) }
pub enum digstore_cli::error::CliError { NoStore(String), InvalidArgument(String), NotFound(String), VerificationFailed(String), Network(String), NonFastForward, Unauthorized(String), Other(anyhow::Error) }
pub fn digstore_cli::error::CliError::exit_code(&self) -> i32
pub fn digstore_cli::error::CliError::from_error_code(code: digstore_core::ErrorCode, ctx: &str) -> digstore_cli::error::CliError
pub struct digstore_cli::context::CliContext { pub dig_dir: std::path::PathBuf, pub json: bool, pub verbose: bool }
pub fn digstore_cli::context::CliContext::resolve(explicit: Option<std::path::PathBuf>, json: bool, verbose: bool) -> digstore_cli::context::CliContext
pub fn digstore_cli::context::CliContext::config_path(&self) -> std::path::PathBuf
pub fn digstore_cli::context::CliContext::load_config(&self) -> Result<digstore_store::StoreConfig, digstore_cli::error::CliError>
pub fn digstore_cli::context::CliContext::find_store_id(&self) -> Result<digstore_core::Bytes32, digstore_cli::error::CliError>
pub fn digstore_cli::context::CliContext::modules_dir(&self) -> std::path::PathBuf
pub fn digstore_cli::context::CliContext::generations_dir(&self) -> std::path::PathBuf
pub fn digstore_cli::context::CliContext::staging_path(&self, store_id: &digstore_core::Bytes32) -> std::path::PathBuf
pub fn digstore_cli::context::CliContext::salt_path(&self) -> std::path::PathBuf
pub fn digstore_cli::ops::store_ops::init_store(ctx: &CliContext, private: bool, data_dir: Option<String>) -> Result<digstore_cli::ops::store_ops::InitResult, CliError>
pub fn digstore_cli::ops::store_ops::add_path(ctx: &CliContext, path: &std::path::Path, key: Option<String>) -> Result<digstore_cli::ops::store_ops::AddResult, CliError>
pub fn digstore_cli::ops::store_ops::commit(ctx: &CliContext, message: Option<String>) -> Result<digstore_cli::ops::store_ops::CommitOutcome, CliError>
pub fn digstore_cli::ops::store_ops::status(ctx: &CliContext) -> Result<digstore_cli::output::StatusView, CliError>
pub fn digstore_cli::ops::store_ops::log(ctx: &CliContext, limit: Option<usize>) -> Result<Vec<digstore_cli::output::LogEntry>, CliError>
pub fn digstore_cli::ops::store_ops::current_root(ctx: &CliContext) -> Result<Option<digstore_core::Bytes32>, CliError>
pub fn digstore_cli::ops::store_ops::diff(ctx: &CliContext, from: &digstore_core::Bytes32, to: &digstore_core::Bytes32) -> Result<Vec<digstore_cli::output::DiffEntry>, CliError>
pub fn digstore_cli::ops::store_ops::module_path_for(ctx: &CliContext, store_id: &digstore_core::Bytes32, root: Option<digstore_core::Bytes32>) -> Result<std::path::PathBuf, CliError>
pub fn digstore_cli::ops::store_ops::list_generation_resources(ctx: &CliContext, root: &digstore_core::Bytes32) -> Result<Vec<String>, CliError>
pub fn digstore_cli::ops::serve::serve_content(module_path: &std::path::Path, urn: &digstore_core::Urn) -> Result<digstore_core::ContentResponse, CliError>
pub fn digstore_cli::ops::serve::serve_proof(module_path: &std::path::Path, urn: &digstore_core::Urn) -> Result<(digstore_core::ExecutionProof, digstore_core::Bytes32), CliError>
pub fn digstore_cli::ops::serve::request_for(urn: &digstore_core::Urn) -> digstore_core::ContentRequest
pub fn digstore_cli::ops::client_crypto::derive_decryption_key(urn: &digstore_core::Urn, secret_salt: Option<&[u8; 32]>) -> [u8; 32]
pub fn digstore_cli::ops::client_crypto::verify_chunk_inclusion(chunk: &[u8], proof: &digstore_core::MerkleProof, trusted_root: &digstore_core::Bytes32) -> Result<(), CliError>
pub fn digstore_cli::ops::client_crypto::decrypt_and_verify(resp: &digstore_core::ContentResponse, urn: &digstore_core::Urn, secret_salt: Option<&[u8; 32]>, trusted_root: &digstore_core::Bytes32) -> Result<Vec<u8>, CliError>
pub async fn digstore_cli::ops::remote_ops::clone_from(ctx: &CliContext, base_url: &str) -> Result<digstore_cli::ops::remote_ops::CloneSummary, CliError>
pub async fn digstore_cli::ops::remote_ops::push_to(ctx: &CliContext, base_url: &str) -> Result<digstore_core::Bytes32, CliError>
pub async fn digstore_cli::ops::remote_ops::pull_from(ctx: &CliContext, base_url: &str) -> Result<digstore_core::Bytes32, CliError>
pub fn digstore_cli::config::add_remote(ctx: &CliContext, name: &str, url: &str) -> Result<(), CliError>
pub fn digstore_cli::config::list_remotes(ctx: &CliContext) -> Result<std::collections::BTreeMap<String, String>, CliError>
pub fn digstore_cli::config::resolve_remote_url(ctx: &CliContext, name: &str) -> Result<String, CliError>
binary: digstore (clap) — verbs: init, add, commit, status, log, diff, checkout, cat, remote{add,list,remove}, clone, push, pull
```