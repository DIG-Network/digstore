# digstore-remote Implementation Plan

> **For agentic workers:** This plan is executed via **REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`**. Each task below is a sequence of bite-sized (2-5 min) TDD steps with explicit checkboxes. Do them strictly in order: write the failing test, run it and confirm the FAIL message, write the minimal implementation, run it and confirm PASS, then commit. Never batch steps. Never write implementation before its test. Reference only types/functions defined in this crate or in the canonical catalog (`digstore-core`, `digstore-store`, `digstore-host`, `digstore-crypto`).

**Goal:** Implement the full Digstore HTTPS remote protocol (paper §21) as an `axum` (tokio) server plus a `reqwest` client (`DigClient`) covering descriptor/roots/module/content/proof/delta endpoints, ETag caching, BLS-authenticated fast-forward push with pending-head support, and delta sync.

**Architecture:** `RemoteServer::new(backend)` builds an `axum::Router` over a `RemoteBackend` trait (implemented by an in-process adapter over `digstore-store` + `digstore-host`); handlers run the synchronous wasmtime host inside `tokio::task::spawn_blocking`. `DigClient` wraps a `reqwest::Client` and implements `clone`/`fetch`/`pull`/`push`, verifying modules and deltas against trusted roots using `digstore-core` merkle + `digstore-crypto` BLS. ETag is the generation root; push auth verifies a publisher BLS signature over `SHA-256(root)` bound to the store id against the store public key, fast-forward-only, with a pending-vs-served head distinction.

**Tech Stack:** Rust 2021, `axum` 0.7, `tokio` 1 (rt-multi-thread, macros), `tower` (`ServiceExt::oneshot` for tests), `reqwest` 0.12 (rustls), `serde`/`serde_json`, `bytes`, `http`, `thiserror`, plus workspace crates `digstore-core`, `digstore-store`, `digstore-host`, `digstore-crypto`.

---

## File Structure

All paths under `crates/digstore-remote/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest, dependencies, dev-dependencies, features |
| `src/lib.rs` | Crate root: re-exports `RemoteServer`, `DigClient`, `RemoteBackend`, error types, wire DTOs |
| `src/error.rs` | `RemoteError` (server-side) and `ClientError` (client-side) enums mapping to HTTP status codes |
| `src/wire.rs` | JSON request/response DTOs for descriptor, roots, content, proof, delta endpoints |
| `src/etag.rs` | ETag encode/decode (root hex), `If-None-Match` parsing/matching |
| `src/backend.rs` | `RemoteBackend` trait + `HeadState`/`PushOutcome`/`DeltaSet` types |
| `src/backend_inmem.rs` | `InMemoryBackend` test/reference backend (store map, head state, pending state) |
| `src/auth.rs` | Push authorization: BLS sig verify over `SHA-256(root)` bound to store id; bearer token check |
| `src/server.rs` | `RemoteServer`, `Router` construction, `AppState`, `spawn_blocking` bridge |
| `src/handlers/mod.rs` | Handler module aggregator |
| `src/handlers/descriptor.rs` | `GET /stores/{id}`, `GET /stores/{id}/roots` |
| `src/handlers/module.rs` | `HEAD/GET/PUT /stores/{id}/module` (ETag, 304, push, 201/202/409/413/422/401/403) |
| `src/handlers/content.rs` | `POST /stores/{id}/content` (decoy-safe, never 404 on miss) |
| `src/handlers/proof.rs` | `POST /stores/{id}/proof` |
| `src/handlers/delta.rs` | `GET /stores/{id}/delta?from=&to=`, `POST /stores/{id}/delta` |
| `src/client.rs` | `DigClient`: `clone`/`fetch`/`pull`/`push`, module+delta verification |
| `src/ratelimit.rs` | Token-bucket per-store rate limiter (429) |
| `tests/test_helpers.rs` | Shared test fixtures: keypairs, sample modules, backend builders, oneshot helper |
| `tests/descriptor.rs` | Integration tests for descriptor + roots endpoints |
| `tests/module.rs` | Integration tests for HEAD/GET module + ETag/304 |
| `tests/push.rs` | Integration tests for PUT module: auth, fast-forward, pending, size/validation |
| `tests/content.rs` | Integration tests for content + proof endpoints |
| `tests/delta.rs` | Integration tests for delta GET/POST |
| `tests/ratelimit.rs` | Integration test for 429 |
| `tests/client_roundtrip.rs` | End-to-end `DigClient` ↔ `RemoteServer` over a bound `tokio` listener |

---

## Task 1: Crate scaffolding and error types

**Files:**
- Create: `crates/digstore-remote/Cargo.toml`
- Create: `crates/digstore-remote/src/lib.rs`
- Create: `crates/digstore-remote/src/error.rs`
- Test: `crates/digstore-remote/src/error.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] Add `crates/digstore-remote` to the workspace `members` list in the root `Cargo.toml` (if a `[workspace] members = [...]` array exists, append `"crates/digstore-remote"`; if the workspace uses a glob `"crates/*"`, no change is needed — verify by reading the root `Cargo.toml`).
- [ ] Create `crates/digstore-remote/Cargo.toml` with the full dependency set:
```toml
[package]
name = "digstore-remote"
version = "0.1.0"
edition = "2021"

[dependencies]
digstore-core = { path = "../digstore-core" }
digstore-store = { path = "../digstore-store" }
digstore-host = { path = "../digstore-host" }
digstore-crypto = { path = "../digstore-crypto" }
axum = { version = "0.7", features = ["http1", "json", "tokio"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "sync"] }
tower = { version = "0.4", features = ["util"] }
http = "1"
http-body-util = "0.1"
bytes = "1"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
hex = "0.4"

[dev-dependencies]
tower = { version = "0.4", features = ["util"] }
```
- [ ] Create `crates/digstore-remote/src/lib.rs` with just module declarations and a doc comment:
```rust
//! Digstore HTTPS remote protocol (paper §21): axum server + reqwest client.
//!
//! Deviation note: the wire codec for store *content* is the Chia big-endian
//! streamable framing used everywhere in digstore-core; only the REST envelope
//! (descriptor/roots/delta metadata) is JSON for transport ergonomics.
pub mod error;

pub use error::{ClientError, RemoteError};
```
- [ ] Create `crates/digstore-remote/src/error.rs` with a failing test first:
```rust
use http::StatusCode;
use thiserror::Error;

/// Server-side error. Each variant maps to a §21.8 status code.
#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("unknown store")]
    UnknownStore,
    #[error("unknown root")]
    UnknownRoot,
    #[error("push not authorized: {0}")]
    Unauthorized(String),
    #[error("missing bearer token")]
    MissingBearer,
    #[error("non-fast-forward push")]
    NonFastForward,
    #[error("module too large: {0} bytes")]
    TooLarge(u64),
    #[error("module failed validation: {0}")]
    Validation(String),
    #[error("rate limited")]
    RateLimited,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl RemoteError {
    /// §21.8 status code mapping. Note: content miss is NEVER mapped here
    /// (it returns 200 with a decoy); only structural errors map.
    pub fn status(&self) -> StatusCode {
        match self {
            RemoteError::UnknownStore | RemoteError::UnknownRoot => StatusCode::NOT_FOUND,
            RemoteError::Unauthorized(_) => StatusCode::FORBIDDEN,
            RemoteError::MissingBearer => StatusCode::UNAUTHORIZED,
            RemoteError::NonFastForward => StatusCode::CONFLICT,
            RemoteError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            RemoteError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            RemoteError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            RemoteError::BadRequest(_) => StatusCode::BAD_REQUEST,
            RemoteError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Client-side error for DigClient operations.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http transport: {0}")]
    Transport(String),
    #[error("server returned status {0}")]
    Status(u16),
    #[error("verification failed: {0}")]
    Verification(String),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("non-fast-forward (409)")]
    NonFastForward,
    #[error("unauthorized ({0})")]
    Unauthorized(u16),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_matches_spec_21_8() {
        assert_eq!(RemoteError::UnknownStore.status(), StatusCode::NOT_FOUND);
        assert_eq!(RemoteError::UnknownRoot.status(), StatusCode::NOT_FOUND);
        assert_eq!(RemoteError::Unauthorized("bad sig".into()).status(), StatusCode::FORBIDDEN);
        assert_eq!(RemoteError::MissingBearer.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(RemoteError::NonFastForward.status(), StatusCode::CONFLICT);
        assert_eq!(RemoteError::TooLarge(1).status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(RemoteError::Validation("x".into()).status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(RemoteError::RateLimited.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
```
- [ ] Run `cargo test -p digstore-remote status_mapping_matches_spec_21_8`. Expected on first attempt: compile-then-PASS. If `http` feature mismatch causes `error[E0433]: failed to resolve: use of undeclared crate`, fix `Cargo.toml` deps and re-run. Expected final: `test error::tests::status_mapping_matches_spec_21_8 ... ok`.
- [ ] Commit: `git add crates/digstore-remote Cargo.toml && git commit -m "feat(remote): scaffold digstore-remote crate with §21.8 error/status mapping"`

---

## Task 2: ETag handling (§21.7)

**Files:**
- Create: `crates/digstore-remote/src/etag.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: `crates/digstore-remote/src/etag.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] Add `pub mod etag;` to `src/lib.rs`.
- [ ] Create `crates/digstore-remote/src/etag.rs` with a failing test first (`Bytes32` comes from `digstore-core`):
```rust
use digstore_core::Bytes32;

/// The module's ETag is its generation root (§21.7), rendered as a strong,
/// quoted hex string: `"<64-hex>"`.
pub fn etag_for_root(root: &Bytes32) -> String {
    format!("\"{}\"", root.to_hex())
}

/// Parse a single `If-None-Match` header value into a root, if it is a
/// well-formed quoted 64-hex strong tag. Returns None for `*`, weak tags,
/// or malformed values.
pub fn parse_if_none_match(header: &str) -> Option<Bytes32> {
    let trimmed = header.trim();
    let inner = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    Bytes32::from_hex(inner).ok()
}

/// Does the client's `If-None-Match` value match the current root? (=> 304)
pub fn matches_current(header: &str, current_root: &Bytes32) -> bool {
    parse_if_none_match(header).as_ref() == Some(current_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    fn root(b: u8) -> Bytes32 { Bytes32([b; 32]) }

    #[test]
    fn etag_is_quoted_hex_of_root() {
        let e = etag_for_root(&root(0xAB));
        assert_eq!(e, format!("\"{}\"", "ab".repeat(32)));
    }

    #[test]
    fn parse_round_trips_etag() {
        let r = root(0x07);
        let e = etag_for_root(&r);
        assert_eq!(parse_if_none_match(&e), Some(r));
    }

    #[test]
    fn matches_current_true_when_equal_false_when_not() {
        let r = root(0x10);
        assert!(matches_current(&etag_for_root(&r), &r));
        assert!(!matches_current(&etag_for_root(&root(0x11)), &r));
    }

    #[test]
    fn star_and_garbage_do_not_match() {
        let r = root(0x20);
        assert!(!matches_current("*", &r));
        assert!(!matches_current("\"nothex\"", &r));
        assert!(!matches_current("W/\"weak\"", &r));
    }
}
```
- [ ] Run `cargo test -p digstore-remote --lib etag`. Expected: 4 tests PASS (`test etag::tests::etag_is_quoted_hex_of_root ... ok`, etc). If `Bytes32::to_hex`/`from_hex` signatures differ, adjust the call sites to the canonical `digstore-core` API and re-run.
- [ ] Commit: `git add crates/digstore-remote/src/etag.rs crates/digstore-remote/src/lib.rs && git commit -m "feat(remote): ETag = root, If-None-Match parsing/matching (§21.7)"`

---

## Task 3: Wire DTOs for descriptor, roots, content, proof, delta

**Files:**
- Create: `crates/digstore-remote/src/wire.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: `crates/digstore-remote/src/wire.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] Add `pub mod wire;` to `src/lib.rs`.
- [ ] Create `crates/digstore-remote/src/wire.rs` with the JSON envelope DTOs and a failing serde round-trip test. Roots/hashes are hex strings on the JSON envelope; `ciphertext`/`proof` blobs are base64 strings; content/proof responses reuse `digstore-core` structs internally but are exposed over the wire as these DTOs:
```rust
use serde::{Deserialize, Serialize};

/// `GET /stores/{id}` — store descriptor (§21.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreDescriptor {
    /// Current served (confirmed) root, hex.
    pub current_root: String,
    /// Total served module size in bytes.
    pub size: u64,
    /// Store BLS G1 public key, 48-byte hex.
    pub public_key: String,
}

/// `GET /stores/{id}/roots` — linear root history, oldest→newest (§21.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootHistory {
    pub roots: Vec<RootEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootEntry {
    pub generation: u64,
    pub root: String,
    pub timestamp: u64,
}

/// `POST /stores/{id}/content` request body (§21.2): retrieval key + root + range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRequest {
    /// Retrieval key (SHA-256 of canonical URN), 32-byte hex.
    pub retrieval_key: String,
    /// Generation root to read against, 32-byte hex.
    pub root: String,
    /// Optional byte range [start,end) into the resource.
    pub range: Option<ByteRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

/// `POST /stores/{id}/content` response (§14.x shape; decoy identical on wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentEnvelope {
    /// base64(ciphertext bytes).
    pub ciphertext_b64: String,
    /// base64(custom-codec-encoded MerkleProof).
    pub merkle_proof_b64: String,
    /// 32-byte hex roothash the proof commits to.
    pub roothash: String,
}

/// `POST /stores/{id}/proof` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofRequest {
    pub retrieval_key: String,
    pub root: String,
}

/// `POST /stores/{id}/proof` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofEnvelope {
    /// base64(custom-codec-encoded ExecutionProof).
    pub proof_b64: String,
    pub roothash: String,
}

/// `GET /delta?from=&to=` / `POST /delta` response (§21.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaResponse {
    pub from: String,
    pub to: String,
    /// New chunks present in `to` and absent from `from`: hex hash -> base64 bytes.
    pub chunks: Vec<DeltaChunk>,
    /// Key-table entries changed/added between `from` and `to`.
    pub key_table_changes: Vec<KeyTableChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaChunk {
    pub hash: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTableChange {
    /// base64(custom-codec-encoded KeyTableEntry).
    pub entry_b64: String,
}

/// `POST /delta` request: client have-summary (§21.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaNegotiateRequest {
    pub to: String,
    /// Hex hashes of chunks the client already holds.
    pub have: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_json_round_trips() {
        let d = StoreDescriptor {
            current_root: "ab".repeat(32),
            size: 4096,
            public_key: "cd".repeat(48),
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: StoreDescriptor = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn content_request_range_optional() {
        let no_range = ContentRequest {
            retrieval_key: "00".repeat(32),
            root: "11".repeat(32),
            range: None,
        };
        let s = serde_json::to_string(&no_range).unwrap();
        assert!(s.contains("\"range\":null"));
        let back: ContentRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(no_range, back);
    }

    #[test]
    fn delta_response_round_trips() {
        let d = DeltaResponse {
            from: "00".repeat(32),
            to: "01".repeat(32),
            chunks: vec![DeltaChunk { hash: "aa".repeat(32), data_b64: "AAAA".into() }],
            key_table_changes: vec![KeyTableChange { entry_b64: "BBBB".into() }],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: DeltaResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }
}
```
- [ ] Run `cargo test -p digstore-remote --lib wire`. Expected: `test wire::tests::descriptor_json_round_trips ... ok` and two more PASS.
- [ ] Commit: `git add crates/digstore-remote/src/wire.rs crates/digstore-remote/src/lib.rs && git commit -m "feat(remote): JSON wire DTOs for descriptor/roots/content/proof/delta (§21.2)"`

---

## Task 4: RemoteBackend trait and supporting types

**Files:**
- Create: `crates/digstore-remote/src/backend.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: deferred to Task 5 (the in-memory impl exercises the trait)

Steps:

- [ ] Add `pub mod backend;` to `src/lib.rs` and re-export: `pub use backend::{RemoteBackend, HeadState, PushOutcome, PushMode, DeltaSet};`.
- [ ] Create `crates/digstore-remote/src/backend.rs`. This trait abstracts the store/host so the in-memory reference backend and the real `digstore-store`+`digstore-host` adapter are interchangeable. Note: `served_root` (confirmed head, §21.4) and `pending_root` are distinct.
```rust
use digstore_core::{Bytes32, Bytes48};
use crate::error::RemoteError;

/// The current head state of a store on the remote (§21.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    /// The confirmed generation the remote currently serves.
    pub served_root: Bytes32,
    /// A pushed-but-not-yet-advanced generation, if any (§21.4 pending).
    pub pending_root: Option<Bytes32>,
    /// Served module size in bytes.
    pub served_size: u64,
    /// Store BLS G1 public key.
    pub public_key: Bytes48,
}

/// One entry in the linear root history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRecord {
    pub generation: u64,
    pub root: Bytes32,
    pub timestamp: u64,
}

/// Whether a push advances the served head or stays pending (§21.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushMode {
    /// Advance the served head immediately (=> 201).
    Advance,
    /// Accept into pending state, do not advance served head (=> 202).
    Pending,
}

/// Result of an accepted push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// New generation is now the served head (201).
    Advanced,
    /// Accepted into pending state, head unchanged (202).
    Pending,
}

/// The chunk-set difference between two generations along linear ancestry (§21.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSet {
    pub from: Bytes32,
    pub to: Bytes32,
    /// (chunk hash, chunk bytes) for chunks in `to` not in `from`.
    pub new_chunks: Vec<(Bytes32, Vec<u8>)>,
    /// Custom-codec-encoded KeyTableEntry blobs changed between from..to.
    pub key_table_changes: Vec<Vec<u8>>,
}

/// Abstracts store storage + host serving for the remote (§18, §21).
/// Implementations: InMemoryBackend (tests/reference) and a digstore-store
/// + digstore-host adapter (production). All methods are synchronous; the
/// server runs them inside spawn_blocking because the host (wasmtime) is sync.
pub trait RemoteBackend: Send + Sync + 'static {
    /// Head state for a store, or UnknownStore.
    fn head_state(&self, store_id: &Bytes32) -> Result<HeadState, RemoteError>;

    /// Linear root history oldest→newest, or UnknownStore.
    fn root_history(&self, store_id: &Bytes32) -> Result<Vec<RootRecord>, RemoteError>;

    /// Raw module bytes for the served head, or UnknownStore/UnknownRoot.
    /// If `root` is Some, it must equal the served head root (else UnknownRoot).
    fn module_bytes(&self, store_id: &Bytes32, root: Option<&Bytes32>)
        -> Result<Vec<u8>, RemoteError>;

    /// Serve content for a retrieval key + root + optional range.
    /// MUST return Ok with a decoy on a retrieval miss (never 404 for content,
    /// §14.2/§21.8). Returns (ciphertext, encoded_merkle_proof, roothash).
    fn serve_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
        range: Option<(u64, u64)>,
    ) -> Result<(Vec<u8>, Vec<u8>, Bytes32), RemoteError>;

    /// Serve a proof for a retrieval key + root. Returns (encoded_proof, roothash).
    fn serve_proof(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
    ) -> Result<(Vec<u8>, Bytes32), RemoteError>;

    /// Accept a pushed module. The caller has ALREADY verified the BLS push
    /// signature and fast-forward eligibility; this only persists state.
    /// `parent` is the root the push claims to fast-forward from.
    fn accept_push(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module_bytes: &[u8],
        mode: PushMode,
    ) -> Result<PushOutcome, RemoteError>;

    /// Compute the delta from `from` to `to` along linear ancestry (§21.5).
    fn delta(&self, store_id: &Bytes32, from: &Bytes32, to: &Bytes32)
        -> Result<DeltaSet, RemoteError>;

    /// Negotiated delta from a client have-summary (§21.5 POST /delta).
    fn delta_from_have(
        &self,
        store_id: &Bytes32,
        to: &Bytes32,
        have: &[Bytes32],
    ) -> Result<DeltaSet, RemoteError>;

    /// Maximum accepted module size in bytes (§21.8 413).
    fn max_module_size(&self) -> u64;

    /// Whether a bearer token is required for push (§21.6).
    fn requires_bearer(&self, store_id: &Bytes32) -> bool;

    /// Validate a presented bearer token for a store (transport-level, §21.6).
    fn check_bearer(&self, store_id: &Bytes32, token: Option<&str>) -> bool;
}
```
- [ ] Run `cargo build -p digstore-remote`. Expected: builds clean (trait + types compile). If `Bytes48` is not re-exported from `digstore-core`, fix the `use`.
- [ ] Commit: `git add crates/digstore-remote/src/backend.rs crates/digstore-remote/src/lib.rs && git commit -m "feat(remote): RemoteBackend trait with served/pending head + delta (§21.4/§21.5)"`

---

## Task 5: InMemoryBackend reference implementation

**Files:**
- Create: `crates/digstore-remote/src/backend_inmem.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: `crates/digstore-remote/src/backend_inmem.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] Add `pub mod backend_inmem;` to `src/lib.rs` and re-export `pub use backend_inmem::InMemoryBackend;`.
- [ ] Create `crates/digstore-remote/src/backend_inmem.rs`. This is a deterministic, dependency-light reference backend used by every integration test. It stores per-store generations (root → module bytes + chunk set + key-table-change blobs), a served head pointer, and an optional pending pointer. Content miss returns a deterministic decoy (size from `retrieval_key`), never an error. Write the failing tests first:
```rust
use std::collections::BTreeMap;
use std::sync::Mutex;
use digstore_core::{Bytes32, Bytes48};
use crate::backend::{
    DeltaSet, HeadState, PushMode, PushOutcome, RemoteBackend, RootRecord,
};
use crate::error::RemoteError;

#[derive(Clone)]
struct Generation {
    parent: Option<Bytes32>,
    generation_no: u64,
    timestamp: u64,
    module: Vec<u8>,
    /// chunk hash -> chunk bytes present in this generation.
    chunks: BTreeMap<Bytes32, Vec<u8>>,
    /// encoded KeyTableEntry blobs introduced/changed at this generation.
    key_table_changes: Vec<Vec<u8>>,
    /// retrieval_key -> (ciphertext, encoded merkle proof) for real hits.
    content: BTreeMap<Bytes32, (Vec<u8>, Vec<u8>)>,
}

struct StoreState {
    public_key: Bytes48,
    served_root: Bytes32,
    pending_root: Option<Bytes32>,
    requires_bearer: bool,
    bearer_token: Option<String>,
    generations: BTreeMap<Bytes32, Generation>,
    /// served head ordering for root history.
    history_order: Vec<Bytes32>,
}

/// Deterministic in-memory backend for tests and as a reference implementation.
pub struct InMemoryBackend {
    stores: Mutex<BTreeMap<Bytes32, StoreState>>,
    max_module_size: u64,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        InMemoryBackend { stores: Mutex::new(BTreeMap::new()), max_module_size: 16 * 1024 * 1024 }
    }

    pub fn with_max_module_size(max: u64) -> Self {
        InMemoryBackend { stores: Mutex::new(BTreeMap::new()), max_module_size: max }
    }

    /// Register a store with a genesis generation.
    #[allow(clippy::too_many_arguments)]
    pub fn add_store(
        &self,
        store_id: Bytes32,
        public_key: Bytes48,
        genesis_root: Bytes32,
        module: Vec<u8>,
    ) {
        let mut g = Generation {
            parent: None,
            generation_no: 0,
            timestamp: 1_000,
            module,
            chunks: BTreeMap::new(),
            key_table_changes: Vec::new(),
            content: BTreeMap::new(),
        };
        // genesis carries no incremental chunk; tests add chunks explicitly.
        let _ = &mut g;
        let mut state = StoreState {
            public_key,
            served_root: genesis_root,
            pending_root: None,
            requires_bearer: false,
            bearer_token: None,
            generations: BTreeMap::new(),
            history_order: vec![genesis_root],
        };
        state.generations.insert(genesis_root, g);
        self.stores.lock().unwrap().insert(store_id, state);
    }

    /// Add a child generation directly (test helper, bypasses push auth).
    pub fn add_generation(
        &self,
        store_id: &Bytes32,
        parent: Bytes32,
        new_root: Bytes32,
        module: Vec<u8>,
        chunks: Vec<(Bytes32, Vec<u8>)>,
        key_table_changes: Vec<Vec<u8>>,
        advance: bool,
    ) {
        let mut stores = self.stores.lock().unwrap();
        let st = stores.get_mut(store_id).expect("store");
        let gen_no = st.generations.len() as u64;
        let gen = Generation {
            parent: Some(parent),
            generation_no: gen_no,
            timestamp: 1_000 + gen_no,
            module,
            chunks: chunks.into_iter().collect(),
            key_table_changes,
            content: BTreeMap::new(),
        };
        st.generations.insert(new_root, gen);
        if advance {
            st.served_root = new_root;
            st.history_order.push(new_root);
        }
    }

    /// Require a bearer token for push (§21.6).
    pub fn set_bearer(&self, store_id: &Bytes32, token: &str) {
        let mut stores = self.stores.lock().unwrap();
        if let Some(st) = stores.get_mut(store_id) {
            st.requires_bearer = true;
            st.bearer_token = Some(token.to_string());
        }
    }

    /// Insert a real content hit for a retrieval key at the served head.
    pub fn put_content(&self, store_id: &Bytes32, retrieval_key: Bytes32, ciphertext: Vec<u8>, proof: Vec<u8>) {
        let mut stores = self.stores.lock().unwrap();
        let st = stores.get_mut(store_id).expect("store");
        let root = st.served_root;
        let gen = st.generations.get_mut(&root).expect("gen");
        gen.content.insert(retrieval_key, (ciphertext, proof));
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self { Self::new() }
}

/// Deterministic decoy bytes: length from a logarithmic distribution keyed by
/// the retrieval key; bytes are a SHA-256 keystream over the key (§14.2).
fn decoy_bytes(retrieval_key: &Bytes32) -> Vec<u8> {
    use digstore_crypto::sha256;
    // length: 256..=64Ki, bucketed by the low bits of the key (log distribution).
    let bucket = (retrieval_key.0[0] % 8) as u32; // 0..7
    let len = 256usize << bucket; // 256, 512, ... 32768
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u32;
    while out.len() < len {
        let mut block = Vec::with_capacity(36);
        block.extend_from_slice(&retrieval_key.0);
        block.extend_from_slice(&counter.to_be_bytes());
        let h = sha256(&block);
        out.extend_from_slice(&h);
        counter += 1;
    }
    out.truncate(len);
    out
}

impl RemoteBackend for InMemoryBackend {
    fn head_state(&self, store_id: &Bytes32) -> Result<HeadState, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let served = st.generations.get(&st.served_root).ok_or(RemoteError::UnknownRoot)?;
        Ok(HeadState {
            served_root: st.served_root,
            pending_root: st.pending_root,
            served_size: served.module.len() as u64,
            public_key: st.public_key,
        })
    }

    fn root_history(&self, store_id: &Bytes32) -> Result<Vec<RootRecord>, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        Ok(st.history_order.iter().map(|r| {
            let g = &st.generations[r];
            RootRecord { generation: g.generation_no, root: *r, timestamp: g.timestamp }
        }).collect())
    }

    fn module_bytes(&self, store_id: &Bytes32, root: Option<&Bytes32>) -> Result<Vec<u8>, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let target = match root {
            Some(r) => *r,
            None => st.served_root,
        };
        let g = st.generations.get(&target).ok_or(RemoteError::UnknownRoot)?;
        // Only the served head is downloadable as the current module.
        if target != st.served_root {
            return Err(RemoteError::UnknownRoot);
        }
        Ok(g.module.clone())
    }

    fn serve_content(
        &self,
        store_id: &Bytes32,
        retrieval_key: &Bytes32,
        root: &Bytes32,
        range: Option<(u64, u64)>,
    ) -> Result<(Vec<u8>, Vec<u8>, Bytes32), RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let g = st.generations.get(root).ok_or(RemoteError::UnknownRoot)?;
        let (mut ct, proof) = match g.content.get(retrieval_key) {
            Some((ct, p)) => (ct.clone(), p.clone()),
            // Retrieval MISS: deterministic decoy, never 404 (§14.2/§21.8).
            None => (decoy_bytes(retrieval_key), Vec::new()),
        };
        if let Some((start, end)) = range {
            let s = (start as usize).min(ct.len());
            let e = (end as usize).min(ct.len());
            if s <= e { ct = ct[s..e].to_vec(); }
        }
        Ok((ct, proof, *root))
    }

    fn serve_proof(
        &self,
        store_id: &Bytes32,
        _retrieval_key: &Bytes32,
        root: &Bytes32,
    ) -> Result<(Vec<u8>, Bytes32), RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        st.generations.get(root).ok_or(RemoteError::UnknownRoot)?;
        // reference: empty proof blob; real backend returns ExecutionProof bytes.
        Ok((Vec::new(), *root))
    }

    fn accept_push(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module_bytes: &[u8],
        mode: PushMode,
    ) -> Result<PushOutcome, RemoteError> {
        let mut stores = self.stores.lock().unwrap();
        let st = stores.get_mut(store_id).ok_or(RemoteError::UnknownStore)?;
        let gen_no = st.generations.len() as u64;
        let gen = Generation {
            parent: Some(*parent),
            generation_no: gen_no,
            timestamp: 1_000 + gen_no,
            module: module_bytes.to_vec(),
            chunks: BTreeMap::new(),
            key_table_changes: Vec::new(),
            content: BTreeMap::new(),
        };
        st.generations.insert(*new_root, gen);
        match mode {
            PushMode::Advance => {
                st.served_root = *new_root;
                st.pending_root = None;
                st.history_order.push(*new_root);
                Ok(PushOutcome::Advanced)
            }
            PushMode::Pending => {
                st.pending_root = Some(*new_root);
                Ok(PushOutcome::Pending)
            }
        }
    }

    fn delta(&self, store_id: &Bytes32, from: &Bytes32, to: &Bytes32) -> Result<DeltaSet, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let from_gen = st.generations.get(from).ok_or(RemoteError::UnknownRoot)?;
        let to_gen = st.generations.get(to).ok_or(RemoteError::UnknownRoot)?;
        let new_chunks: Vec<(Bytes32, Vec<u8>)> = to_gen.chunks.iter()
            .filter(|(h, _)| !from_gen.chunks.contains_key(*h))
            .map(|(h, d)| (*h, d.clone()))
            .collect();
        Ok(DeltaSet {
            from: *from,
            to: *to,
            new_chunks,
            key_table_changes: to_gen.key_table_changes.clone(),
        })
    }

    fn delta_from_have(&self, store_id: &Bytes32, to: &Bytes32, have: &[Bytes32]) -> Result<DeltaSet, RemoteError> {
        let stores = self.stores.lock().unwrap();
        let st = stores.get(store_id).ok_or(RemoteError::UnknownStore)?;
        let to_gen = st.generations.get(to).ok_or(RemoteError::UnknownRoot)?;
        let have_set: std::collections::BTreeSet<&Bytes32> = have.iter().collect();
        let new_chunks: Vec<(Bytes32, Vec<u8>)> = to_gen.chunks.iter()
            .filter(|(h, _)| !have_set.contains(*h))
            .map(|(h, d)| (*h, d.clone()))
            .collect();
        Ok(DeltaSet {
            from: Bytes32([0u8; 32]),
            to: *to,
            new_chunks,
            key_table_changes: to_gen.key_table_changes.clone(),
        })
    }

    fn max_module_size(&self) -> u64 { self.max_module_size }

    fn requires_bearer(&self, store_id: &Bytes32) -> bool {
        let stores = self.stores.lock().unwrap();
        stores.get(store_id).map(|s| s.requires_bearer).unwrap_or(false)
    }

    fn check_bearer(&self, store_id: &Bytes32, token: Option<&str>) -> bool {
        let stores = self.stores.lock().unwrap();
        match stores.get(store_id) {
            Some(st) => match (&st.bearer_token, token) {
                (Some(expected), Some(given)) => expected == given,
                (Some(_), None) => false,
                (None, _) => true,
            },
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(x: u8) -> Bytes32 { Bytes32([x; 32]) }
    fn b48(x: u8) -> Bytes48 { Bytes48([x; 48]) }

    fn backend_with_one_store() -> (InMemoryBackend, Bytes32) {
        let be = InMemoryBackend::new();
        let id = b32(1);
        be.add_store(id, b48(2), b32(0x10), vec![0u8; 64]);
        (be, id)
    }

    #[test]
    fn head_state_returns_served_root_and_size() {
        let (be, id) = backend_with_one_store();
        let hs = be.head_state(&id).unwrap();
        assert_eq!(hs.served_root, b32(0x10));
        assert_eq!(hs.served_size, 64);
        assert_eq!(hs.pending_root, None);
    }

    #[test]
    fn head_state_unknown_store_errors() {
        let be = InMemoryBackend::new();
        assert!(matches!(be.head_state(&b32(9)), Err(RemoteError::UnknownStore)));
    }

    #[test]
    fn content_miss_returns_deterministic_decoy_never_error() {
        let (be, id) = backend_with_one_store();
        let key = b32(0x55);
        let (a, _, _) = be.serve_content(&id, &key, &b32(0x10), None).unwrap();
        let (b, _, _) = be.serve_content(&id, &key, &b32(0x10), None).unwrap();
        assert!(!a.is_empty());
        assert_eq!(a, b, "same miss must yield identical decoy bytes (§14.2)");
    }

    #[test]
    fn content_hit_returns_real_ciphertext() {
        let (be, id) = backend_with_one_store();
        let key = b32(0x77);
        be.put_content(&id, key, vec![1, 2, 3, 4], vec![9, 9]);
        let (ct, proof, root) = be.serve_content(&id, &key, &b32(0x10), None).unwrap();
        assert_eq!(ct, vec![1, 2, 3, 4]);
        assert_eq!(proof, vec![9, 9]);
        assert_eq!(root, b32(0x10));
    }

    #[test]
    fn pending_push_does_not_advance_served_head() {
        let (be, id) = backend_with_one_store();
        let out = be.accept_push(&id, &b32(0x10), &b32(0x20), &[0u8; 32], PushMode::Pending).unwrap();
        assert_eq!(out, PushOutcome::Pending);
        let hs = be.head_state(&id).unwrap();
        assert_eq!(hs.served_root, b32(0x10), "served head unchanged on pending (§21.4)");
        assert_eq!(hs.pending_root, Some(b32(0x20)));
    }

    #[test]
    fn advance_push_moves_served_head() {
        let (be, id) = backend_with_one_store();
        let out = be.accept_push(&id, &b32(0x10), &b32(0x30), &[0u8; 48], PushMode::Advance).unwrap();
        assert_eq!(out, PushOutcome::Advanced);
        let hs = be.head_state(&id).unwrap();
        assert_eq!(hs.served_root, b32(0x30));
        assert_eq!(hs.served_size, 48);
    }

    #[test]
    fn delta_returns_only_new_chunks() {
        let (be, id) = backend_with_one_store();
        // genesis (0x10) has no chunks; add a child with two chunks.
        be.add_generation(&id, b32(0x10), b32(0x40),
            vec![0u8; 10],
            vec![(b32(0xA1), vec![1]), (b32(0xA2), vec![2])],
            vec![vec![7, 7]],
            true);
        let d = be.delta(&id, &b32(0x10), &b32(0x40)).unwrap();
        assert_eq!(d.new_chunks.len(), 2);
        assert_eq!(d.key_table_changes, vec![vec![7, 7]]);
    }

    #[test]
    fn bearer_required_rejects_wrong_token() {
        let (be, id) = backend_with_one_store();
        be.set_bearer(&id, "secret");
        assert!(be.requires_bearer(&id));
        assert!(be.check_bearer(&id, Some("secret")));
        assert!(!be.check_bearer(&id, Some("wrong")));
        assert!(!be.check_bearer(&id, None));
    }
}
```
- [ ] Run `cargo test -p digstore-remote --lib backend_inmem`. Expected first attempt: a FAIL if `digstore_crypto::sha256` has a different name/signature — the compile error `error[E0425]: cannot find function 'sha256' in crate 'digstore_crypto'` tells you the exact symbol to fix. Adjust the `use`/call to the canonical `digstore-crypto` hashing API, then re-run.
- [ ] After fixing, run `cargo test -p digstore-remote --lib backend_inmem` again. Expected: 8 tests PASS (`content_miss_returns_deterministic_decoy_never_error ... ok`, etc).
- [ ] Commit: `git add crates/digstore-remote/src/backend_inmem.rs crates/digstore-remote/src/lib.rs && git commit -m "feat(remote): InMemoryBackend reference (decoy-safe content, pending head, delta)"`

---

## Task 6: Push authorization (§21.6)

**Files:**
- Create: `crates/digstore-remote/src/auth.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: `crates/digstore-remote/src/auth.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] Add `pub mod auth;` to `src/lib.rs` and re-export `pub use auth::{verify_push_signature, push_signing_message, PushAuth};`.
- [ ] Create `crates/digstore-remote/src/auth.rs`. The publisher signs `SHA-256(root)` bound to the store id with the store BLS key; the remote verifies against the store public key. We define the signing message as `SHA-256(store_id || SHA-256(root))` to bind the root to the store id, then verify the BLS G2 signature over that message digest using `digstore-crypto`'s Chia AugScheme verify. Write failing tests first:
```rust
use digstore_core::{Bytes32, Bytes48, Bytes96};
use digstore_crypto::sha256;

/// The 32-byte message a publisher signs for a push (§21.6):
/// SHA-256( store_id || SHA-256(root) ), binding the pushed root to the store.
pub fn push_signing_message(store_id: &Bytes32, root: &Bytes32) -> [u8; 32] {
    let inner = sha256(&root.0);
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(&store_id.0);
    buf.extend_from_slice(&inner);
    sha256(&buf)
}

/// Parsed push-authorization inputs extracted from request headers/body.
#[derive(Debug, Clone)]
pub struct PushAuth {
    pub signature: Bytes96,
    pub bearer: Option<String>,
}

/// Verify the publisher BLS signature over the push message (§21.6).
/// Uses the Chia AugScheme verify in digstore-crypto. Returns true on valid.
pub fn verify_push_signature(
    store_public_key: &Bytes48,
    store_id: &Bytes32,
    root: &Bytes32,
    signature: &Bytes96,
) -> bool {
    let msg = push_signing_message(store_id, root);
    digstore_crypto::bls_verify(&store_public_key.0, &msg, &signature.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(x: u8) -> Bytes32 { Bytes32([x; 32]) }

    #[test]
    fn message_binds_store_id_and_root() {
        let m1 = push_signing_message(&b32(1), &b32(2));
        let m2 = push_signing_message(&b32(9), &b32(2)); // different store id
        let m3 = push_signing_message(&b32(1), &b32(3)); // different root
        assert_ne!(m1, m2);
        assert_ne!(m1, m3);
        // deterministic
        assert_eq!(m1, push_signing_message(&b32(1), &b32(2)));
    }

    #[test]
    fn valid_signature_verifies_and_tamper_fails() {
        // host-side keygen + sign (Chia AugScheme) via digstore-crypto.
        let (sk, pk) = digstore_crypto::bls_keygen(&[7u8; 32]);
        let store_id = b32(0x11);
        let root = b32(0x22);
        let msg = push_signing_message(&store_id, &root);
        let sig = digstore_crypto::bls_sign(&sk, &msg);

        let pk48 = Bytes48(pk);
        let sig96 = Bytes96(sig);
        assert!(verify_push_signature(&pk48, &store_id, &root, &sig96));

        // tamper: a different root must not verify under the same signature.
        assert!(!verify_push_signature(&pk48, &store_id, &b32(0x23), &sig96));
    }
}
```
- [ ] Run `cargo test -p digstore-remote --lib auth`. Expected first attempt: FAIL with `error[E0425]: cannot find function 'bls_keygen'/'bls_sign'/'bls_verify' in crate 'digstore_crypto'` if the canonical names differ. Inspect `digstore-crypto`'s public BLS API (`cargo doc -p digstore-crypto --no-deps` or read `crates/digstore-crypto/src/lib.rs`), then align the function names/signatures (keygen, sign, verify using Chia AugScheme; pubkey 48B, sig 96B) and re-run.
- [ ] After fixing, run `cargo test -p digstore-remote --lib auth` again. Expected: `test auth::tests::message_binds_store_id_and_root ... ok` and `valid_signature_verifies_and_tamper_fails ... ok`.
- [ ] Commit: `git add crates/digstore-remote/src/auth.rs crates/digstore-remote/src/lib.rs && git commit -m "feat(remote): BLS push auth — sign SHA-256(root) bound to store id (§21.6)"`

---

## Task 7: Rate limiter (§21.8 429)

**Files:**
- Create: `crates/digstore-remote/src/ratelimit.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: `crates/digstore-remote/src/ratelimit.rs` (inline `#[cfg(test)]`)

Steps:

- [ ] Add `pub mod ratelimit;` to `src/lib.rs` and re-export `pub use ratelimit::RateLimiter;`.
- [ ] Create `crates/digstore-remote/src/ratelimit.rs` with a deterministic token-bucket keyed by store id. Tests must not depend on wall-clock, so the bucket exposes a `try_acquire` that decrements a fixed budget; refill is a separate explicit call. Write failing tests first:
```rust
use std::collections::BTreeMap;
use std::sync::Mutex;
use digstore_core::Bytes32;

/// Simple per-store token bucket. Deterministic: `refill` adds tokens up to
/// capacity; `try_acquire` consumes one token, returning false (=> 429) when
/// the bucket is empty.
pub struct RateLimiter {
    capacity: u32,
    buckets: Mutex<BTreeMap<Bytes32, u32>>,
}

impl RateLimiter {
    pub fn new(capacity: u32) -> Self {
        RateLimiter { capacity, buckets: Mutex::new(BTreeMap::new()) }
    }

    /// Attempt to consume one token for a store. False => rate limited.
    pub fn try_acquire(&self, store_id: &Bytes32) -> bool {
        let mut b = self.buckets.lock().unwrap();
        let tokens = b.entry(*store_id).or_insert(self.capacity);
        if *tokens == 0 {
            false
        } else {
            *tokens -= 1;
            true
        }
    }

    /// Refill a store's bucket to capacity (called on a timer in production).
    pub fn refill(&self, store_id: &Bytes32) {
        let mut b = self.buckets.lock().unwrap();
        b.insert(*store_id, self.capacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn b32(x: u8) -> Bytes32 { Bytes32([x; 32]) }

    #[test]
    fn allows_up_to_capacity_then_limits() {
        let rl = RateLimiter::new(2);
        let id = b32(1);
        assert!(rl.try_acquire(&id));
        assert!(rl.try_acquire(&id));
        assert!(!rl.try_acquire(&id), "third call must be rate limited (429)");
    }

    #[test]
    fn refill_restores_budget() {
        let rl = RateLimiter::new(1);
        let id = b32(2);
        assert!(rl.try_acquire(&id));
        assert!(!rl.try_acquire(&id));
        rl.refill(&id);
        assert!(rl.try_acquire(&id));
    }

    #[test]
    fn buckets_are_per_store() {
        let rl = RateLimiter::new(1);
        assert!(rl.try_acquire(&b32(1)));
        assert!(rl.try_acquire(&b32(2)), "different store has its own bucket");
    }
}
```
- [ ] Run `cargo test -p digstore-remote --lib ratelimit`. Expected: 3 tests PASS.
- [ ] Commit: `git add crates/digstore-remote/src/ratelimit.rs crates/digstore-remote/src/lib.rs && git commit -m "feat(remote): per-store token-bucket rate limiter (§21.8 429)"`

---

## Task 8: Server skeleton, AppState, and router (with descriptor + roots handlers)

**Files:**
- Create: `crates/digstore-remote/src/server.rs`
- Create: `crates/digstore-remote/src/handlers/mod.rs`
- Create: `crates/digstore-remote/src/handlers/descriptor.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Create: `crates/digstore-remote/tests/test_helpers.rs`
- Create: `crates/digstore-remote/tests/descriptor.rs`

Steps:

- [ ] Add to `src/lib.rs`: `pub mod backend; pub mod server; pub mod handlers; pub mod ratelimit; pub mod auth;` (ensure all are present) and `pub use server::{RemoteServer, AppState};`.
- [ ] Create `crates/digstore-remote/src/server.rs` with `AppState`, the `RemoteServer` builder, the `spawn_blocking` helper, and the router wiring descriptor routes. Path params parse store id from hex.
```rust
use std::sync::Arc;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use digstore_core::Bytes32;
use crate::backend::RemoteBackend;
use crate::error::RemoteError;
use crate::ratelimit::RateLimiter;
use crate::wire::{RootEntry, RootHistory, StoreDescriptor};

/// Shared server state behind an Arc; cloned into every handler.
#[derive(Clone)]
pub struct AppState {
    pub backend: Arc<dyn RemoteBackend>,
    pub rate_limiter: Arc<RateLimiter>,
}

/// The Digstore remote server. Wraps an axum Router over a RemoteBackend.
pub struct RemoteServer {
    state: AppState,
}

impl RemoteServer {
    pub fn new(backend: Arc<dyn RemoteBackend>) -> Self {
        RemoteServer {
            state: AppState {
                backend,
                rate_limiter: Arc::new(RateLimiter::new(10_000)),
            },
        }
    }

    pub fn with_rate_limiter(backend: Arc<dyn RemoteBackend>, rl: Arc<RateLimiter>) -> Self {
        RemoteServer { state: AppState { backend, rate_limiter: rl } }
    }

    /// Build the axum Router exposing the full §21.2 surface.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/stores/:id", get(crate::handlers::descriptor::get_descriptor))
            .route("/stores/:id/roots", get(crate::handlers::descriptor::get_roots))
            .route(
                "/stores/:id/module",
                get(crate::handlers::module::get_module)
                    .head(crate::handlers::module::head_module)
                    .put(crate::handlers::module::put_module),
            )
            .route("/stores/:id/content", post(crate::handlers::content::post_content))
            .route("/stores/:id/proof", post(crate::handlers::proof::post_proof))
            .route(
                "/stores/:id/delta",
                get(crate::handlers::delta::get_delta).post(crate::handlers::delta::post_delta),
            )
            .with_state(self.state.clone())
    }
}

/// Parse a hex store id from a path parameter, or 400.
pub fn parse_store_id(s: &str) -> Result<Bytes32, RemoteError> {
    Bytes32::from_hex(s).map_err(|_| RemoteError::BadRequest("bad store id".into()))
}

/// Run a synchronous backend call off the async runtime (wasmtime is sync, §18).
pub async fn run_blocking<T, F>(f: F) -> Result<T, RemoteError>
where
    F: FnOnce() -> Result<T, RemoteError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| RemoteError::Internal(format!("join: {e}")))?
}

impl IntoResponse for RemoteError {
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

// re-export the bits handlers need at crate root usage sites.
pub(crate) use axum::{extract::Path as AxPath, extract::State as AxState};
```
> Note: the small `pub(crate) use` re-export line above is illustrative; if it triggers an unused-import warning, delete it — handlers import `axum` types directly.
- [ ] Create `crates/digstore-remote/src/handlers/mod.rs`:
```rust
pub mod content;
pub mod delta;
pub mod descriptor;
pub mod module;
pub mod proof;
```
- [ ] Create stub handler modules so the router compiles. Create `crates/digstore-remote/src/handlers/module.rs`, `content.rs`, `proof.rs`, `delta.rs` each with a single `unimplemented`-free stub returning 500 for now (they are filled in later tasks). For `module.rs`:
```rust
use axum::{extract::{Path, State}, http::StatusCode, response::Response};
use crate::server::AppState;

pub async fn get_module(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response_stub()
}
pub async fn head_module(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response_stub()
}
pub async fn put_module(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response_stub()
}

// helper trait to keep stubs terse; removed when handlers are implemented.
trait StubResponse { fn into_response_stub(self) -> Response; }
impl StubResponse for StatusCode {
    fn into_response_stub(self) -> Response {
        use axum::response::IntoResponse;
        self.into_response()
    }
}
```
  Create the same minimal stub shape for `content.rs` (`post_content`), `proof.rs` (`post_proof`), and `delta.rs` (`get_delta`, `post_delta`) — each a `pub async fn name(State(_s): State<AppState>, Path(_id): Path<String>) -> Response { StatusCode::NOT_IMPLEMENTED.into_response_stub() }` with the same local `StubResponse` trait.
- [ ] Create `crates/digstore-remote/src/handlers/descriptor.rs` with the real descriptor + roots handlers:
```rust
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use digstore_core::Bytes48;
use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{RootEntry, RootHistory, StoreDescriptor};

pub async fn get_descriptor(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.head_state(&store_id)).await;
    match res {
        Ok(hs) => {
            let body = StoreDescriptor {
                current_root: hs.served_root.to_hex(),
                size: hs.served_size,
                public_key: hex::encode(pubkey_bytes(&hs.public_key)),
            };
            Json(body).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn get_roots(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.root_history(&store_id)).await;
    match res {
        Ok(records) => {
            let roots = records.into_iter().map(|r| RootEntry {
                generation: r.generation,
                root: r.root.to_hex(),
                timestamp: r.timestamp,
            }).collect();
            Json(RootHistory { roots }).into_response()
        }
        Err(e) => e.into_response(),
    }
}

fn pubkey_bytes(pk: &Bytes48) -> [u8; 48] { pk.0 }
```
- [ ] Create `crates/digstore-remote/tests/test_helpers.rs` with shared fixtures (allow unused since shared across test files):
```rust
#![allow(dead_code)]
use std::sync::Arc;
use axum::Router;
use digstore_core::{Bytes32, Bytes48, Bytes96};
use digstore_remote::{InMemoryBackend, RemoteServer};

pub fn b32(x: u8) -> Bytes32 { Bytes32([x; 32]) }
pub fn b48(x: u8) -> Bytes48 { Bytes48([x; 48]) }
pub fn b96(x: u8) -> Bytes96 { Bytes96([x; 96]) }

/// A backend with one store registered at genesis root 0x10, pk 0x02,
/// 64-byte module. Returns (backend Arc, store id, store id hex).
pub fn one_store() -> (Arc<InMemoryBackend>, Bytes32, String) {
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(1);
    be.add_store(id, b48(2), b32(0x10), vec![0u8; 64]);
    (be, id, id.to_hex())
}

pub fn router_for(be: Arc<InMemoryBackend>) -> Router {
    RemoteServer::new(be).router()
}
```
- [ ] Create `crates/digstore-remote/tests/descriptor.rs` with the failing integration tests (tower `oneshot`):
```rust
mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use digstore_remote::wire::{RootHistory, StoreDescriptor};

#[tokio::test]
async fn get_descriptor_returns_root_size_pubkey() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let resp = app
        .oneshot(Request::builder().uri(format!("/stores/{id_hex}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let d: StoreDescriptor = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(d.current_root, "10".repeat(32));
    assert_eq!(d.size, 64);
    assert_eq!(d.public_key, "02".repeat(48));
}

#[tokio::test]
async fn get_descriptor_unknown_store_is_404() {
    let (be, _id, _hex) = one_store();
    let app = router_for(be);
    let unknown = "99".repeat(32);
    let resp = app
        .oneshot(Request::builder().uri(format!("/stores/{unknown}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_roots_lists_history() {
    let (be, id, id_hex) = one_store();
    be.add_generation(&id, b32(0x10), b32(0x11), vec![0u8; 8], vec![], vec![], true);
    let app = router_for(be);
    let resp = app
        .oneshot(Request::builder().uri(format!("/stores/{id_hex}/roots")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let h: RootHistory = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(h.roots.len(), 2);
    assert_eq!(h.roots[0].root, "10".repeat(32));
    assert_eq!(h.roots[1].root, "11".repeat(32));
}
```
- [ ] Make `wire` and `backend` modules reachable from tests: confirm `src/lib.rs` has `pub mod wire;` and `pub use backend_inmem::InMemoryBackend;` (added earlier). The test references `digstore_remote::wire::...`.
- [ ] Run `cargo test -p digstore-remote --test descriptor`. Expected first compile FAIL with `error[E0599]: no method named 'into_response_stub'` only if a stub typo exists — fix stub modules. Once compiling, expected first run FAIL only if signatures mismatch (e.g. `to_hex`). After alignment, expected: `get_descriptor_returns_root_size_pubkey ... ok`, `get_descriptor_unknown_store_is_404 ... ok`, `get_roots_lists_history ... ok`.
- [ ] Commit: `git add crates/digstore-remote/src crates/digstore-remote/tests && git commit -m "feat(remote): axum server skeleton + descriptor/roots handlers (§21.2/§21.3)"`

---

## Task 9: Module HEAD and GET with ETag/304 (§21.2, §21.7)

**Files:**
- Modify: `crates/digstore-remote/src/handlers/module.rs`
- Create: `crates/digstore-remote/tests/module.rs`

Steps:

- [ ] Create `crates/digstore-remote/tests/module.rs` with failing tests first:
```rust
mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn head_module_sets_etag_and_size_no_body() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let resp = app
        .oneshot(Request::builder().method(Method::HEAD)
            .uri(format!("/stores/{id_hex}/module")).body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let etag = resp.headers().get(header::ETAG).unwrap().to_str().unwrap().to_string();
    assert_eq!(etag, format!("\"{}\"", "10".repeat(32)));
    assert_eq!(resp.headers().get(header::CONTENT_LENGTH).unwrap().to_str().unwrap(), "64");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty(), "HEAD has no body");
}

#[tokio::test]
async fn get_module_returns_wasm_bytes_with_etag() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let resp = app
        .oneshot(Request::builder().uri(format!("/stores/{id_hex}/module")).body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap(), "application/wasm");
    assert_eq!(resp.headers().get(header::ETAG).unwrap().to_str().unwrap(), format!("\"{}\"", "10".repeat(32)));
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.len(), 64);
}

#[tokio::test]
async fn if_none_match_current_root_returns_304() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let etag = format!("\"{}\"", "10".repeat(32));
    let resp = app
        .oneshot(Request::builder()
            .uri(format!("/stores/{id_hex}/module"))
            .header(header::IF_NONE_MATCH, etag)
            .body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty());
}

#[tokio::test]
async fn if_none_match_stale_root_returns_200() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let stale = format!("\"{}\"", "ff".repeat(32));
    let resp = app
        .oneshot(Request::builder()
            .uri(format!("/stores/{id_hex}/module"))
            .header(header::IF_NONE_MATCH, stale)
            .body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_module_unknown_store_404() {
    let (be, _id, _hex) = one_store();
    let app = router_for(be);
    let unknown = "aa".repeat(32);
    let resp = app
        .oneshot(Request::builder().uri(format!("/stores/{unknown}/module")).body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```
- [ ] Run `cargo test -p digstore-remote --test module`. Expected FAIL: handlers still return 501 (`NOT_IMPLEMENTED`), so e.g. `head_module_sets_etag_and_size_no_body` fails `assert_eq!(resp.status(), 200)` with `left: 501 right: 200`.
- [ ] Replace `get_module`/`head_module` in `crates/digstore-remote/src/handlers/module.rs` with the real implementations (keep `put_module` as the stub for now):
```rust
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use crate::etag::{etag_for_root, matches_current};
use crate::server::{parse_store_id, run_blocking, AppState};

pub async fn head_module(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.head_state(&store_id)).await;
    match res {
        Ok(hs) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::ETAG, etag_for_root(&hs.served_root).parse().unwrap());
            headers.insert(header::CONTENT_LENGTH, hs.served_size.to_string().parse().unwrap());
            headers.insert(header::CONTENT_TYPE, "application/wasm".parse().unwrap());
            (StatusCode::OK, headers).into_response()
        }
        Err(e) => e.into_response(),
    }
}

pub async fn get_module(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();
    let head = match run_blocking({
        let b = backend.clone();
        move || b.head_state(&store_id)
    }).await {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    // §21.7: If-None-Match equal to current root -> 304.
    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if matches_current(inm, &head.served_root) {
            return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag_for_root(&head.served_root))]).into_response();
        }
    }

    let res = run_blocking(move || backend.module_bytes(&store_id, None)).await;
    match res {
        Ok(bytes) => {
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/wasm".to_string()),
                    (header::ETAG, etag_for_root(&head.served_root)),
                ],
                Body::from(bytes),
            ).into_response()
        }
        Err(e) => e.into_response(),
    }
}

// put_module remains the existing stub until Task 10.
pub async fn put_module(State(_s): State<AppState>, Path(_id): Path<String>) -> Response {
    StatusCode::NOT_IMPLEMENTED.into_response()
}
```
  Remove the now-unused local `StubResponse` trait from this file.
- [ ] Run `cargo test -p digstore-remote --test module`. Expected: 5 tests PASS (`head_module_sets_etag_and_size_no_body ... ok`, `if_none_match_current_root_returns_304 ... ok`, etc).
- [ ] Commit: `git add crates/digstore-remote/src/handlers/module.rs crates/digstore-remote/tests/module.rs && git commit -m "feat(remote): HEAD/GET module with application/wasm + ETag/304 (§21.2/§21.7)"`

---

## Task 10: PUT module — push auth, fast-forward, pending, size/validation (§21.4, §21.6, §21.8)

**Files:**
- Modify: `crates/digstore-remote/src/handlers/module.rs`
- Modify: `crates/digstore-remote/src/server.rs` (define push header names + body shape)
- Create: `crates/digstore-remote/tests/push.rs`

Push wire contract (documented here, used by client in Task 14): the publisher PUTs the raw module bytes as the body with these headers:
- `X-Dig-Parent: <64-hex>` — the root this push fast-forwards from (must equal served head).
- `X-Dig-Root: <64-hex>` — the new root being pushed.
- `X-Dig-Signature: <192-hex>` — BLS G2 signature (96 bytes) over `push_signing_message(store_id, root)`.
- `X-Dig-Push-Mode: advance|pending` — optional, default `advance` (§21.4).
- `Authorization: Bearer <token>` — optional bearer (§21.6).

Steps:

- [ ] Create `crates/digstore-remote/tests/push.rs` with failing tests first. The helper signs with `digstore-crypto` keygen/sign and registers the matching pubkey on the store:
```rust
mod test_helpers;
use test_helpers::*;

use std::sync::Arc;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;
use digstore_core::{Bytes32, Bytes48, Bytes96};
use digstore_remote::{auth::push_signing_message, InMemoryBackend, RemoteServer};

/// Build a store whose public key is a real BLS key; return (backend, id, id_hex, secret_key).
fn signed_store() -> (Arc<InMemoryBackend>, Bytes32, String, Vec<u8>) {
    let (sk, pk) = digstore_crypto::bls_keygen(&[42u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(1);
    be.add_store(id, Bytes48(pk), b32(0x10), vec![0u8; 64]);
    (be, id, id.to_hex(), sk)
}

fn put_req(id_hex: &str, parent: &str, root: &str, sig_hex: &str, mode: Option<&str>, bearer: Option<&str>, body: Vec<u8>) -> Request<Body> {
    let mut b = Request::builder().method(Method::PUT)
        .uri(format!("/stores/{id_hex}/module"))
        .header("X-Dig-Parent", parent)
        .header("X-Dig-Root", root)
        .header("X-Dig-Signature", sig_hex);
    if let Some(m) = mode { b = b.header("X-Dig-Push-Mode", m); }
    if let Some(t) = bearer { b = b.header("Authorization", format!("Bearer {t}")); }
    b.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn valid_push_advances_head_201() {
    let (be, id, id_hex, sk) = signed_store();
    let new_root = b32(0x20);
    let msg = push_signing_message(&id, &new_root);
    let sig = digstore_crypto::bls_sign(&sk, &msg);
    let app = RemoteServer::new(be.clone()).router();
    let resp = app.oneshot(put_req(&id_hex, &"10".repeat(32), &new_root.to_hex(),
        &hex::encode(sig), Some("advance"), None, vec![1u8; 80])).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let hs = be.head_state(&id).unwrap();
    assert_eq!(hs.served_root, new_root, "served head advanced");
    assert_eq!(hs.served_size, 80);
}

#[tokio::test]
async fn bad_signature_is_403() {
    let (be, id, id_hex, _sk) = signed_store();
    let new_root = b32(0x20);
    let app = RemoteServer::new(be.clone()).router();
    let bad_sig = Bytes96([0xCD; 96]);
    let resp = app.oneshot(put_req(&id_hex, &"10".repeat(32), &new_root.to_hex(),
        &hex::encode(bad_sig.0), Some("advance"), None, vec![1u8; 80])).await.unwrap();
    assert!(resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::UNAUTHORIZED);
    assert_eq!(be.head_state(&id).unwrap().served_root, b32(0x10), "head not advanced on bad sig");
}

#[tokio::test]
async fn missing_bearer_when_required_is_401() {
    let (be, id, id_hex, sk) = signed_store();
    be.set_bearer(&id, "tok");
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&id, &new_root));
    let app = RemoteServer::new(be.clone()).router();
    // valid sig but NO bearer header -> 401
    let resp = app.oneshot(put_req(&id_hex, &"10".repeat(32), &new_root.to_hex(),
        &hex::encode(sig), Some("advance"), None, vec![1u8; 80])).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn correct_bearer_and_sig_is_201() {
    let (be, id, id_hex, sk) = signed_store();
    be.set_bearer(&id, "tok");
    let new_root = b32(0x21);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&id, &new_root));
    let app = RemoteServer::new(be.clone()).router();
    let resp = app.oneshot(put_req(&id_hex, &"10".repeat(32), &new_root.to_hex(),
        &hex::encode(sig), Some("advance"), Some("tok"), vec![1u8; 80])).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn non_fast_forward_is_409() {
    let (be, id, id_hex, sk) = signed_store();
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&id, &new_root));
    let app = RemoteServer::new(be.clone()).router();
    // parent does NOT match served head 0x10 -> 409
    let resp = app.oneshot(put_req(&id_hex, &"ee".repeat(32), &new_root.to_hex(),
        &hex::encode(sig), Some("advance"), None, vec![1u8; 80])).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(be.head_state(&id).unwrap().served_root, b32(0x10));
}

#[tokio::test]
async fn pending_push_is_202_and_served_head_unchanged() {
    let (be, id, id_hex, sk) = signed_store();
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&id, &new_root));
    let app = RemoteServer::new(be.clone()).router();
    let resp = app.oneshot(put_req(&id_hex, &"10".repeat(32), &new_root.to_hex(),
        &hex::encode(sig), Some("pending"), None, vec![1u8; 80])).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let hs = be.head_state(&id).unwrap();
    assert_eq!(hs.served_root, b32(0x10), "served head unchanged on pending (§21.4)");
    assert_eq!(hs.pending_root, Some(new_root));
}

#[tokio::test]
async fn oversized_module_is_413() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[7u8; 32]);
    let be = Arc::new(InMemoryBackend::with_max_module_size(16));
    let id = b32(1);
    be.add_store(id, Bytes48(pk), b32(0x10), vec![0u8; 8]);
    let new_root = b32(0x20);
    let sig = digstore_crypto::bls_sign(&sk, &push_signing_message(&id, &new_root));
    let app = RemoteServer::new(be.clone()).router();
    let resp = app.oneshot(put_req(&id.to_hex(), &"10".repeat(32), &new_root.to_hex(),
        &hex::encode(sig), Some("advance"), None, vec![1u8; 100])).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn malformed_signature_header_is_422_or_403() {
    let (be, _id, id_hex, _sk) = signed_store();
    let app = RemoteServer::new(be).router();
    let resp = app.oneshot(put_req(&id_hex, &"10".repeat(32), &"20".repeat(32),
        "not-hex", Some("advance"), None, vec![1u8; 8])).await.unwrap();
    assert!(resp.status() == StatusCode::UNPROCESSABLE_ENTITY || resp.status() == StatusCode::FORBIDDEN);
}
```
- [ ] Add `digstore-crypto` and `hex` to `[dev-dependencies]` in `Cargo.toml` so tests can keygen/sign:
```toml
[dev-dependencies]
tower = { version = "0.4", features = ["util"] }
digstore-crypto = { path = "../digstore-crypto" }
hex = "0.4"
http-body-util = "0.1"
```
- [ ] Run `cargo test -p digstore-remote --test push`. Expected FAIL: stub `put_module` returns 501; first assertion fails `left: 501 right: 201`.
- [ ] Implement `put_module` in `crates/digstore-remote/src/handlers/module.rs`. Order of checks (§21.8 precedence): parse store id (400) → store exists / load head (404) → parse headers parent/root/sig (422 on malformed) → bearer required & valid (401) → size limit (413) → BLS signature valid (403) → fast-forward parent==served head (409) → accept push (201 advance / 202 pending). Add the body extraction via `axum::body::Bytes`:
```rust
use axum::body::Bytes;
use digstore_core::{Bytes32, Bytes96};
use crate::auth::verify_push_signature;
use crate::backend::PushMode;
use crate::error::RemoteError;

fn header_str<'a>(h: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn parse_b32(s: &str) -> Result<Bytes32, RemoteError> {
    Bytes32::from_hex(s).map_err(|_| RemoteError::Validation("bad hex root".into()))
}

fn parse_sig(s: &str) -> Result<Bytes96, RemoteError> {
    let raw = hex::decode(s).map_err(|_| RemoteError::Validation("bad sig hex".into()))?;
    let arr: [u8; 96] = raw.try_into().map_err(|_| RemoteError::Validation("sig must be 96 bytes".into()))?;
    Ok(Bytes96(arr))
}

pub async fn put_module(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let backend = s.backend.clone();

    // 404 if store unknown.
    let head = match run_blocking({ let b = backend.clone(); move || b.head_state(&store_id) }).await {
        Ok(h) => h,
        Err(e) => return e.into_response(),
    };

    // 422 on malformed required headers.
    let (parent, root, sig) = match (
        header_str(&headers, "X-Dig-Parent"),
        header_str(&headers, "X-Dig-Root"),
        header_str(&headers, "X-Dig-Signature"),
    ) {
        (Some(p), Some(r), Some(sg)) => {
            match (parse_b32(p), parse_b32(r), parse_sig(sg)) {
                (Ok(p), Ok(r), Ok(sg)) => (p, r, sg),
                _ => return RemoteError::Validation("malformed push headers".into()).into_response(),
            }
        }
        _ => return RemoteError::Validation("missing push headers".into()).into_response(),
    };

    // 401 if bearer required but missing/invalid.
    let bearer = header_str(&headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string());
    {
        let backend = backend.clone();
        let sid = store_id;
        let needs = run_blocking(move || Ok(backend.requires_bearer(&sid))).await.unwrap_or(false);
        if needs {
            let backend = s.backend.clone();
            let sid = store_id;
            let bclone = bearer.clone();
            let ok = run_blocking(move || Ok(backend.check_bearer(&sid, bclone.as_deref()))).await.unwrap_or(false);
            if !ok {
                return RemoteError::MissingBearer.into_response();
            }
        }
    }

    // 413 if oversized.
    let max = run_blocking({ let b = backend.clone(); move || Ok(b.max_module_size()) }).await.unwrap_or(0);
    if body.len() as u64 > max {
        return RemoteError::TooLarge(body.len() as u64).into_response();
    }

    // 403 if BLS signature invalid.
    if !verify_push_signature(&head.public_key, &store_id, &root, &sig) {
        return RemoteError::Unauthorized("bad BLS signature".into()).into_response();
    }

    // 409 if not a fast-forward of the served head.
    if parent != head.served_root {
        return RemoteError::NonFastForward.into_response();
    }

    let mode = match header_str(&headers, "X-Dig-Push-Mode") {
        Some("pending") => PushMode::Pending,
        _ => PushMode::Advance,
    };

    let body_vec = body.to_vec();
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.accept_push(&store_id, &parent, &root, &body_vec, mode)).await;
    match res {
        Ok(crate::backend::PushOutcome::Advanced) => {
            (StatusCode::CREATED, [(header::ETAG, etag_for_root(&root))]).into_response()
        }
        Ok(crate::backend::PushOutcome::Pending) => StatusCode::ACCEPTED.into_response(),
        Err(e) => e.into_response(),
    }
}
```
- [ ] Run `cargo test -p digstore-remote --test push`. Expected: 8 tests PASS (`valid_push_advances_head_201 ... ok`, `non_fast_forward_is_409 ... ok`, `pending_push_is_202_and_served_head_unchanged ... ok`, etc).
- [ ] Commit: `git add crates/digstore-remote/src/handlers/module.rs crates/digstore-remote/tests/push.rs crates/digstore-remote/Cargo.toml && git commit -m "feat(remote): PUT module push — BLS auth, fast-forward, pending head, 413/422 (§21.4/§21.6/§21.8)"`

---

## Task 11: POST content (decoy-safe, never 404) and POST proof (§21.2, §14.2)

**Files:**
- Modify: `crates/digstore-remote/src/handlers/content.rs`
- Modify: `crates/digstore-remote/src/handlers/proof.rs`
- Create: `crates/digstore-remote/tests/content.rs`

Steps:

- [ ] Create `crates/digstore-remote/tests/content.rs` with failing tests first:
```rust
mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use digstore_remote::wire::{ContentEnvelope, ContentRequest, ProofEnvelope, ProofRequest};

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder().method(Method::POST).uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn content_hit_returns_real_ciphertext_200() {
    let (be, id, id_hex) = one_store();
    be.put_content(&id, b32(0x55), vec![10, 20, 30], vec![1, 1]);
    let app = router_for(be);
    let req = ContentRequest { retrieval_key: "55".repeat(32), root: "10".repeat(32), range: None };
    let resp = app.oneshot(post_json(&format!("/stores/{id_hex}/content"), &serde_json::to_string(&req).unwrap())).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ContentEnvelope = serde_json::from_slice(&bytes).unwrap();
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD.decode(env.ciphertext_b64).unwrap();
    assert_eq!(ct, vec![10, 20, 30]);
    assert_eq!(env.roothash, "10".repeat(32));
}

#[tokio::test]
async fn content_miss_returns_200_decoy_never_404() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let req = ContentRequest { retrieval_key: "ab".repeat(32), root: "10".repeat(32), range: None };
    let resp = app.oneshot(post_json(&format!("/stores/{id_hex}/content"), &serde_json::to_string(&req).unwrap())).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "content miss must be 200 decoy, never 404 (§21.8/§14.2)");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ContentEnvelope = serde_json::from_slice(&bytes).unwrap();
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD.decode(env.ciphertext_b64).unwrap();
    assert!(!ct.is_empty(), "decoy has real-looking bytes");
}

#[tokio::test]
async fn content_miss_is_deterministic_same_key_same_bytes() {
    let (be, _id, id_hex) = one_store();
    let app1 = router_for(be.clone());
    let app2 = router_for(be);
    let req = ContentRequest { retrieval_key: "cc".repeat(32), root: "10".repeat(32), range: None };
    let body = serde_json::to_string(&req).unwrap();
    let r1 = app1.oneshot(post_json(&format!("/stores/{id_hex}/content"), &body)).await.unwrap();
    let r2 = app2.oneshot(post_json(&format!("/stores/{id_hex}/content"), &body)).await.unwrap();
    let b1 = r1.into_body().collect().await.unwrap().to_bytes();
    let b2 = r2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(b1, b2, "same miss -> identical decoy (§14.2)");
}

#[tokio::test]
async fn content_unknown_store_404() {
    let (be, _id, _hex) = one_store();
    let app = router_for(be);
    let unknown = "99".repeat(32);
    let req = ContentRequest { retrieval_key: "55".repeat(32), root: "10".repeat(32), range: None };
    let resp = app.oneshot(post_json(&format!("/stores/{unknown}/content"), &serde_json::to_string(&req).unwrap())).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "unknown store IS 404 (only content miss is exempt)");
}

#[tokio::test]
async fn content_range_slices_ciphertext() {
    let (be, id, id_hex) = one_store();
    be.put_content(&id, b32(0x55), vec![0,1,2,3,4,5,6,7], vec![]);
    let app = router_for(be);
    let req = ContentRequest {
        retrieval_key: "55".repeat(32),
        root: "10".repeat(32),
        range: Some(digstore_remote::wire::ByteRange { start: 2, end: 5 }),
    };
    let resp = app.oneshot(post_json(&format!("/stores/{id_hex}/content"), &serde_json::to_string(&req).unwrap())).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ContentEnvelope = serde_json::from_slice(&bytes).unwrap();
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD.decode(env.ciphertext_b64).unwrap();
    assert_eq!(ct, vec![2, 3, 4]);
}

#[tokio::test]
async fn proof_returns_200_with_roothash() {
    let (be, _id, id_hex) = one_store();
    let app = router_for(be);
    let req = ProofRequest { retrieval_key: "55".repeat(32), root: "10".repeat(32) };
    let resp = app.oneshot(post_json(&format!("/stores/{id_hex}/proof"), &serde_json::to_string(&req).unwrap())).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let env: ProofEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(env.roothash, "10".repeat(32));
}
```
- [ ] Add `base64` to `[dependencies]` and `[dev-dependencies]` in `Cargo.toml`:
```toml
base64 = "0.22"
```
  (add under `[dependencies]`; also add `base64 = "0.22"` to `[dev-dependencies]`).
- [ ] Run `cargo test -p digstore-remote --test content`. Expected FAIL: stub handlers return 501; `content_hit_returns_real_ciphertext_200` fails `left: 501 right: 200`.
- [ ] Implement `post_content` in `crates/digstore-remote/src/handlers/content.rs`:
```rust
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use digstore_core::Bytes32;
use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{ContentEnvelope, ContentRequest};

pub async fn post_content(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ContentRequest>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let retrieval_key = match Bytes32::from_hex(&req.retrieval_key) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad retrieval key".into()).into_response(),
    };
    let root = match Bytes32::from_hex(&req.root) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad root".into()).into_response(),
    };
    let range = req.range.map(|r| (r.start, r.end));
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.serve_content(&store_id, &retrieval_key, &root, range)).await;
    match res {
        Ok((ct, proof, roothash)) => {
            let env = ContentEnvelope {
                ciphertext_b64: base64::engine::general_purpose::STANDARD.encode(ct),
                merkle_proof_b64: base64::engine::general_purpose::STANDARD.encode(proof),
                roothash: roothash.to_hex(),
            };
            Json(env).into_response()
        }
        Err(e) => e.into_response(),
    }
}
```
- [ ] Implement `post_proof` in `crates/digstore-remote/src/handlers/proof.rs`:
```rust
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use digstore_core::Bytes32;
use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{ProofEnvelope, ProofRequest};

pub async fn post_proof(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ProofRequest>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let retrieval_key = match Bytes32::from_hex(&req.retrieval_key) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad retrieval key".into()).into_response(),
    };
    let root = match Bytes32::from_hex(&req.root) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad root".into()).into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.serve_proof(&store_id, &retrieval_key, &root)).await;
    match res {
        Ok((proof, roothash)) => {
            let env = ProofEnvelope {
                proof_b64: base64::engine::general_purpose::STANDARD.encode(proof),
                roothash: roothash.to_hex(),
            };
            Json(env).into_response()
        }
        Err(e) => e.into_response(),
    }
}
```
- [ ] Run `cargo test -p digstore-remote --test content`. Expected: 6 tests PASS (including `content_miss_returns_200_decoy_never_404 ... ok` and `content_unknown_store_404 ... ok`).
- [ ] Commit: `git add crates/digstore-remote/src/handlers/content.rs crates/digstore-remote/src/handlers/proof.rs crates/digstore-remote/tests/content.rs crates/digstore-remote/Cargo.toml && git commit -m "feat(remote): POST content (200 decoy never 404) + POST proof (§21.2/§14.2)"`

---

## Task 12: GET/POST delta (§21.5)

**Files:**
- Modify: `crates/digstore-remote/src/handlers/delta.rs`
- Create: `crates/digstore-remote/tests/delta.rs`

Steps:

- [ ] Create `crates/digstore-remote/tests/delta.rs` with failing tests first:
```rust
mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use digstore_core::Bytes32;
use digstore_remote::wire::{DeltaNegotiateRequest, DeltaResponse};

/// store with genesis 0x10 (chunk A1) and child 0x40 (adds A2, A3) advanced.
fn store_with_two_gens() -> (std::sync::Arc<digstore_remote::InMemoryBackend>, Bytes32, String) {
    let (be, id, id_hex) = one_store();
    // seed genesis with one chunk by adding a child that supersedes it,
    // but simplest: add child 0x40 carrying two new chunks vs genesis (0 chunks).
    be.add_generation(&id, b32(0x10), b32(0x40),
        vec![0u8; 16],
        vec![(b32(0xA2), vec![2, 2]), (b32(0xA3), vec![3, 3, 3])],
        vec![vec![9, 9]],
        true);
    (be, id, id_hex)
}

#[tokio::test]
async fn get_delta_returns_only_new_chunks_and_keytable_changes() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    let from = "10".repeat(32);
    let to = "40".repeat(32);
    let resp = app.oneshot(Request::builder()
        .uri(format!("/stores/{id_hex}/delta?from={from}&to={to}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let d: DeltaResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(d.from, from);
    assert_eq!(d.to, to);
    assert_eq!(d.chunks.len(), 2, "exactly the new chunks");
    assert_eq!(d.key_table_changes.len(), 1);
}

#[tokio::test]
async fn get_delta_missing_query_is_400() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    let resp = app.oneshot(Request::builder()
        .uri(format!("/stores/{id_hex}/delta"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_delta_unknown_root_is_404() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    let from = "10".repeat(32);
    let to = "ee".repeat(32);
    let resp = app.oneshot(Request::builder()
        .uri(format!("/stores/{id_hex}/delta?from={from}&to={to}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_delta_negotiates_from_have_summary() {
    let (be, _id, id_hex) = store_with_two_gens();
    let app = router_for(be);
    // client already holds A2 -> server returns only A3.
    let req = DeltaNegotiateRequest {
        to: "40".repeat(32),
        have: vec!["a2".repeat(32)],
    };
    let resp = app.oneshot(Request::builder().method(Method::POST)
        .uri(format!("/stores/{id_hex}/delta"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&req).unwrap())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let d: DeltaResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(d.chunks.len(), 1, "client had A2, only A3 returned");
    assert_eq!(d.chunks[0].hash, "a3".repeat(32));
}
```
- [ ] Run `cargo test -p digstore-remote --test delta`. Expected FAIL: stub handlers return 501; `get_delta_returns_only_new_chunks_and_keytable_changes` fails `left: 501 right: 200`.
- [ ] Implement `get_delta` and `post_delta` in `crates/digstore-remote/src/handlers/delta.rs`:
```rust
use std::collections::HashMap;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use digstore_core::Bytes32;
use crate::backend::DeltaSet;
use crate::error::RemoteError;
use crate::server::{parse_store_id, run_blocking, AppState};
use crate::wire::{DeltaChunk, DeltaNegotiateRequest, DeltaResponse, KeyTableChange};

fn to_wire(d: DeltaSet) -> DeltaResponse {
    DeltaResponse {
        from: d.from.to_hex(),
        to: d.to.to_hex(),
        chunks: d.new_chunks.into_iter().map(|(h, data)| DeltaChunk {
            hash: h.to_hex(),
            data_b64: base64::engine::general_purpose::STANDARD.encode(data),
        }).collect(),
        key_table_changes: d.key_table_changes.into_iter().map(|e| KeyTableChange {
            entry_b64: base64::engine::general_purpose::STANDARD.encode(e),
        }).collect(),
    }
}

pub async fn get_delta(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let (from, to) = match (q.get("from"), q.get("to")) {
        (Some(f), Some(t)) => match (Bytes32::from_hex(f), Bytes32::from_hex(t)) {
            (Ok(f), Ok(t)) => (f, t),
            _ => return RemoteError::BadRequest("bad from/to hex".into()).into_response(),
        },
        _ => return RemoteError::BadRequest("from and to required".into()).into_response(),
    };
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.delta(&store_id, &from, &to)).await;
    match res {
        Ok(d) => Json(to_wire(d)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn post_delta(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<DeltaNegotiateRequest>,
) -> Response {
    let store_id = match parse_store_id(&id) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let to = match Bytes32::from_hex(&req.to) {
        Ok(v) => v,
        Err(_) => return RemoteError::BadRequest("bad to hex".into()).into_response(),
    };
    let mut have = Vec::with_capacity(req.have.len());
    for h in &req.have {
        match Bytes32::from_hex(h) {
            Ok(v) => have.push(v),
            Err(_) => return RemoteError::BadRequest("bad have hex".into()).into_response(),
        }
    }
    let backend = s.backend.clone();
    let res = run_blocking(move || backend.delta_from_have(&store_id, &to, &have)).await;
    match res {
        Ok(d) => Json(to_wire(d)).into_response(),
        Err(e) => e.into_response(),
    }
}
```
- [ ] Run `cargo test -p digstore-remote --test delta`. Expected: 4 tests PASS (`get_delta_returns_only_new_chunks_and_keytable_changes ... ok`, `post_delta_negotiates_from_have_summary ... ok`, etc).
- [ ] Commit: `git add crates/digstore-remote/src/handlers/delta.rs crates/digstore-remote/tests/delta.rs && git commit -m "feat(remote): GET/POST delta — new chunks + key-table changes (§21.5)"`

---

## Task 13: Rate limiting middleware (§21.8 429)

**Files:**
- Modify: `crates/digstore-remote/src/server.rs`
- Create: `crates/digstore-remote/tests/ratelimit.rs`

Steps:

- [ ] Create `crates/digstore-remote/tests/ratelimit.rs` with failing test first. We construct a server whose limiter has capacity 1, then issue two descriptor GETs for the same store:
```rust
mod test_helpers;
use test_helpers::*;

use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use digstore_remote::{RateLimiter, RemoteServer};

#[tokio::test]
async fn second_request_for_store_is_429_when_capacity_one() {
    let (be, _id, id_hex) = one_store();
    let rl = Arc::new(RateLimiter::new(1));
    let app = RemoteServer::with_rate_limiter(be, rl).router();

    let r1 = app.clone().oneshot(
        Request::builder().uri(format!("/stores/{id_hex}")).body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(r1.status(), StatusCode::OK);

    let r2 = app.oneshot(
        Request::builder().uri(format!("/stores/{id_hex}")).body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS, "(§21.8 429)");
}

#[tokio::test]
async fn requests_to_distinct_stores_not_limited_together() {
    // two stores, capacity 1 each -> each gets one OK.
    let be = Arc::new(digstore_remote::InMemoryBackend::new());
    be.add_store(b32(1), b48(2), b32(0x10), vec![0u8; 8]);
    be.add_store(b32(3), b48(4), b32(0x20), vec![0u8; 8]);
    let rl = Arc::new(RateLimiter::new(1));
    let app = RemoteServer::with_rate_limiter(be, rl).router();

    let a = app.clone().oneshot(Request::builder().uri(format!("/stores/{}", b32(1).to_hex())).body(Body::empty()).unwrap()).await.unwrap();
    let b = app.oneshot(Request::builder().uri(format!("/stores/{}", b32(3).to_hex())).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(a.status(), StatusCode::OK);
    assert_eq!(b.status(), StatusCode::OK);
}
```
- [ ] Add `pub use ratelimit::RateLimiter;` and `pub use backend_inmem::InMemoryBackend;` to `src/lib.rs` if not already present (so tests can reach them).
- [ ] Run `cargo test -p digstore-remote --test ratelimit`. Expected FAIL: no limiting is wired yet; `second_request_for_store_is_429_when_capacity_one` fails `left: 200 right: 429`.
- [ ] Wire rate-limit enforcement as an axum middleware in `RemoteServer::router`. Add a `from_fn_with_state` layer that extracts the store id from the path and calls `rate_limiter.try_acquire`. Add to `server.rs`:
```rust
use axum::extract::Request as AxRequest;
use axum::middleware::{self, Next};

async fn rate_limit_mw(
    State(s): State<AppState>,
    req: AxRequest,
    next: Next,
) -> Response {
    // extract `{id}` segment after `/stores/`.
    let path = req.uri().path().to_string();
    if let Some(rest) = path.strip_prefix("/stores/") {
        let id_seg = rest.split('/').next().unwrap_or("");
        if let Ok(store_id) = Bytes32::from_hex(id_seg) {
            if !s.rate_limiter.try_acquire(&store_id) {
                return RemoteError::RateLimited.into_response();
            }
        }
    }
    next.run(req).await
}
```
  Then chain `.layer(middleware::from_fn_with_state(self.state.clone(), rate_limit_mw))` onto the `Router` in `router()` (after `.with_state(...)`; note `from_fn_with_state` clones state into the middleware).
- [ ] Run `cargo test -p digstore-remote --test ratelimit`. Expected: 2 tests PASS.
- [ ] Run the full suite to confirm no regressions: `cargo test -p digstore-remote`. Expected: all prior tests still PASS (the default `RemoteServer::new` uses a 10_000-capacity limiter, so existing single-call tests are unaffected).
- [ ] Commit: `git add crates/digstore-remote/src/server.rs crates/digstore-remote/src/lib.rs crates/digstore-remote/tests/ratelimit.rs && git commit -m "feat(remote): per-store rate-limit middleware (§21.8 429)"`

---

## Task 14: DigClient — fetch, clone, pull, push (§21.3, §21.4, §21.5, §21.6)

**Files:**
- Create: `crates/digstore-remote/src/client.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Create: `crates/digstore-remote/tests/client_roundtrip.rs`

Steps:

- [ ] Add `pub mod client;` to `src/lib.rs` and `pub use client::{DigClient, FetchInfo, PushResult, PullResult};`.
- [ ] Create `crates/digstore-remote/src/client.rs`. The client wraps `reqwest::Client` and a base URL. `fetch` GETs descriptor + roots; `clone` downloads the module and verifies its size/ETag; `pull` checks `If-None-Match`, downloads the new module (or, when `from` is known and a delta is preferred, GETs the delta), and verifies; `push` signs `push_signing_message` and PUTs the module with the auth headers. Verification of module integrity beyond size/ETag (full merkle-to-root) is delegated to a `verify` closure the caller supplies (so the client does not duplicate `digstore-store` compile logic); the default closure checks the served ETag matches the expected root.
```rust
use digstore_core::{Bytes32, Bytes48, Bytes96};
use crate::auth::push_signing_message;
use crate::error::ClientError;
use crate::etag::parse_if_none_match;
use crate::wire::{
    DeltaNegotiateRequest, DeltaResponse, RootHistory, StoreDescriptor,
};

/// Result of `fetch`: descriptor + root history (§21.3).
#[derive(Debug, Clone)]
pub struct FetchInfo {
    pub descriptor: StoreDescriptor,
    pub roots: RootHistory,
}

/// Result of `pull` (§21.4).
#[derive(Debug, Clone)]
pub enum PullResult {
    /// Local head already matched remote head (304).
    UpToDate,
    /// Downloaded a fresh module for `root` (size bytes).
    Module { root: Bytes32, bytes: Vec<u8> },
    /// Downloaded a delta to `root`.
    Delta { root: Bytes32, delta: DeltaResponse },
}

/// Result of `push` (§21.6 / §21.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    /// 201: served head advanced.
    Advanced,
    /// 202: accepted into pending state (§21.4).
    Pending,
}

/// HTTPS remote client: clone/fetch/pull/push (§21).
pub struct DigClient {
    base_url: String,
    http: reqwest::Client,
}

impl DigClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        DigClient { base_url: base_url.into().trim_end_matches('/').to_string(), http: reqwest::Client::new() }
    }

    pub fn with_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        DigClient { base_url: base_url.into().trim_end_matches('/').to_string(), http }
    }

    fn url(&self, path: &str) -> String { format!("{}{}", self.base_url, path) }

    /// §21.3 fetch: descriptor + root history only.
    pub async fn fetch(&self, store_id: &Bytes32) -> Result<FetchInfo, ClientError> {
        let id = store_id.to_hex();
        let d: StoreDescriptor = self.http.get(self.url(&format!("/stores/{id}")))
            .send().await.map_err(|e| ClientError::Transport(e.to_string()))?
            .error_for_status().map_err(|e| ClientError::Status(e.status().map(|s| s.as_u16()).unwrap_or(0)))?
            .json().await.map_err(|e| ClientError::Decode(e.to_string()))?;
        let roots: RootHistory = self.http.get(self.url(&format!("/stores/{id}/roots")))
            .send().await.map_err(|e| ClientError::Transport(e.to_string()))?
            .json().await.map_err(|e| ClientError::Decode(e.to_string()))?;
        Ok(FetchInfo { descriptor: d, roots })
    }

    /// §21.3 clone: download + verify the module. `verify` is called with
    /// (module_bytes, served_root) and must return Ok(()) when the module
    /// validates to that root (full merkle verification lives in the caller).
    pub async fn clone_store<V>(&self, store_id: &Bytes32, verify: V) -> Result<(Bytes32, Vec<u8>), ClientError>
    where
        V: FnOnce(&[u8], &Bytes32) -> Result<(), String>,
    {
        let id = store_id.to_hex();
        let resp = self.http.get(self.url(&format!("/stores/{id}/module")))
            .send().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        let etag = resp.headers().get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        let root = etag.as_deref().and_then(parse_if_none_match)
            .ok_or_else(|| ClientError::Verification("missing/invalid ETag".into()))?;
        let bytes = resp.bytes().await.map_err(|e| ClientError::Transport(e.to_string()))?.to_vec();
        verify(&bytes, &root).map_err(ClientError::Verification)?;
        Ok((root, bytes))
    }

    /// §21.4 pull: advance the local head. `local_root` is the client's current
    /// generation; If-None-Match short-circuits to UpToDate on 304. When
    /// `prefer_delta` and `local_root` is Some, attempt GET /delta first.
    pub async fn pull(
        &self,
        store_id: &Bytes32,
        local_root: Option<Bytes32>,
        prefer_delta: bool,
    ) -> Result<PullResult, ClientError> {
        let id = store_id.to_hex();
        // determine remote head.
        let desc = self.fetch(store_id).await?;
        let remote_root = Bytes32::from_hex(&desc.descriptor.current_root)
            .map_err(|_| ClientError::Decode("bad current_root".into()))?;
        if local_root == Some(remote_root) {
            return Ok(PullResult::UpToDate);
        }
        if prefer_delta {
            if let Some(from) = local_root {
                let from_h = from.to_hex();
                let to_h = remote_root.to_hex();
                let resp = self.http.get(self.url(&format!("/stores/{id}/delta?from={from_h}&to={to_h}")))
                    .send().await.map_err(|e| ClientError::Transport(e.to_string()))?;
                if resp.status().is_success() {
                    let delta: DeltaResponse = resp.json().await.map_err(|e| ClientError::Decode(e.to_string()))?;
                    return Ok(PullResult::Delta { root: remote_root, delta });
                }
                // fall through to full module on non-success delta.
            }
        }
        // full module download with conditional request.
        let mut req = self.http.get(self.url(&format!("/stores/{id}/module")));
        if let Some(lr) = local_root {
            req = req.header(reqwest::header::IF_NONE_MATCH, format!("\"{}\"", lr.to_hex()));
        }
        let resp = req.send().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        if resp.status().as_u16() == 304 {
            return Ok(PullResult::UpToDate);
        }
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        let bytes = resp.bytes().await.map_err(|e| ClientError::Transport(e.to_string()))?.to_vec();
        Ok(PullResult::Module { root: remote_root, bytes })
    }

    /// §21.6 push: sign SHA-256(root) bound to store id, PUT the module.
    /// `sign` is the caller's BLS signer over the 32-byte push message.
    pub async fn push<S>(
        &self,
        store_id: &Bytes32,
        parent: &Bytes32,
        new_root: &Bytes32,
        module: &[u8],
        pending: bool,
        bearer: Option<&str>,
        sign: S,
    ) -> Result<PushResult, ClientError>
    where
        S: FnOnce(&[u8; 32]) -> Bytes96,
    {
        let id = store_id.to_hex();
        let msg = push_signing_message(store_id, new_root);
        let sig = sign(&msg);
        let mut req = self.http.put(self.url(&format!("/stores/{id}/module")))
            .header("X-Dig-Parent", parent.to_hex())
            .header("X-Dig-Root", new_root.to_hex())
            .header("X-Dig-Signature", hex::encode(sig.0))
            .header("X-Dig-Push-Mode", if pending { "pending" } else { "advance" })
            .body(module.to_vec());
        if let Some(t) = bearer {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let resp = req.send().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        match resp.status().as_u16() {
            201 => Ok(PushResult::Advanced),
            202 => Ok(PushResult::Pending),
            401 | 403 => Err(ClientError::Unauthorized(resp.status().as_u16())),
            409 => Err(ClientError::NonFastForward),
            other => Err(ClientError::Status(other)),
        }
    }

    /// §21.5 negotiated delta from a have-summary.
    pub async fn negotiate_delta(
        &self,
        store_id: &Bytes32,
        to: &Bytes32,
        have: &[Bytes32],
    ) -> Result<DeltaResponse, ClientError> {
        let id = store_id.to_hex();
        let body = DeltaNegotiateRequest {
            to: to.to_hex(),
            have: have.iter().map(|h| h.to_hex()).collect(),
        };
        let resp = self.http.post(self.url(&format!("/stores/{id}/delta")))
            .json(&body).send().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        resp.json().await.map_err(|e| ClientError::Decode(e.to_string()))
    }
}

// keep Bytes48 referenced so the doc example compiles if added later.
#[allow(dead_code)]
fn _pubkey_marker(_pk: Bytes48) {}
```
- [ ] Create `crates/digstore-remote/tests/client_roundtrip.rs` end-to-end tests. These bind the server to an ephemeral `tokio::net::TcpListener`, serve in a background task, then drive `DigClient` against the real URL:
```rust
mod test_helpers;
use test_helpers::*;

use std::sync::Arc;
use digstore_core::{Bytes32, Bytes48, Bytes96};
use digstore_remote::{DigClient, InMemoryBackend, PullResult, PushResult, RemoteServer};

async fn spawn_server(be: Arc<InMemoryBackend>) -> String {
    let app = RemoteServer::new(be).router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn fetch_returns_descriptor_and_roots() {
    let (be, id, _hex) = one_store();
    be.add_generation(&id, b32(0x10), b32(0x11), vec![0u8; 8], vec![], vec![], true);
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let info = client.fetch(&id).await.unwrap();
    assert_eq!(info.descriptor.current_root, "11".repeat(32));
    assert_eq!(info.roots.roots.len(), 2);
}

#[tokio::test]
async fn clone_downloads_and_verifies_module() {
    let (be, id, _hex) = one_store();
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let (root, bytes) = client.clone_store(&id, |b, r| {
        if b.len() == 64 && *r == b32(0x10) { Ok(()) } else { Err("size mismatch".into()) }
    }).await.unwrap();
    assert_eq!(root, b32(0x10));
    assert_eq!(bytes.len(), 64);
}

#[tokio::test]
async fn pull_up_to_date_when_local_equals_head() {
    let (be, id, _hex) = one_store();
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client.pull(&id, Some(b32(0x10)), false).await.unwrap();
    assert!(matches!(res, PullResult::UpToDate));
}

#[tokio::test]
async fn pull_downloads_module_when_behind() {
    let (be, id, _hex) = one_store();
    be.add_generation(&id, b32(0x10), b32(0x12), vec![0u8; 32], vec![], vec![], true);
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client.pull(&id, Some(b32(0x10)), false).await.unwrap();
    match res {
        PullResult::Module { root, bytes } => {
            assert_eq!(root, b32(0x12));
            assert_eq!(bytes.len(), 32);
        }
        other => panic!("expected Module, got {other:?}"),
    }
}

#[tokio::test]
async fn pull_delta_path_returns_new_chunks() {
    let (be, id, _hex) = one_store();
    be.add_generation(&id, b32(0x10), b32(0x13), vec![0u8; 16],
        vec![(b32(0xB1), vec![1]), (b32(0xB2), vec![2])], vec![vec![5,5]], true);
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client.pull(&id, Some(b32(0x10)), true).await.unwrap();
    match res {
        PullResult::Delta { root, delta } => {
            assert_eq!(root, b32(0x13));
            assert_eq!(delta.chunks.len(), 2);
        }
        other => panic!("expected Delta, got {other:?}"),
    }
}

#[tokio::test]
async fn push_signs_and_advances_head() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[99u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(7);
    be.add_store(id, Bytes48(pk), b32(0x10), vec![0u8; 8]);
    let base = spawn_server(be.clone()).await;
    let client = DigClient::new(base);
    let new_root = b32(0x20);
    let res = client.push(&id, &b32(0x10), &new_root, &[1u8; 40], false, None, |msg| {
        Bytes96(digstore_crypto::bls_sign(&sk, msg))
    }).await.unwrap();
    assert_eq!(res, PushResult::Advanced);
}

#[tokio::test]
async fn push_pending_returns_pending_and_pull_sees_confirmed_not_pending() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[55u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(8);
    be.add_store(id, Bytes48(pk), b32(0x10), vec![0u8; 8]);
    let base = spawn_server(be.clone()).await;
    let client = DigClient::new(base);
    let pending_root = b32(0x20);
    let res = client.push(&id, &b32(0x10), &pending_root, &[1u8; 40], true, None, |msg| {
        Bytes96(digstore_crypto::bls_sign(&sk, msg))
    }).await.unwrap();
    assert_eq!(res, PushResult::Pending, "(§21.4 202)");
    // pull must still see the confirmed (genesis) head, NOT the pending root.
    let info = client.fetch(&id).await.unwrap();
    assert_eq!(info.descriptor.current_root, "10".repeat(32), "served head still confirmed (§21.4)");
}

#[tokio::test]
async fn push_non_fast_forward_is_client_error() {
    let (sk, pk) = digstore_crypto::bls_keygen(&[33u8; 32]);
    let be = Arc::new(InMemoryBackend::new());
    let id = b32(9);
    be.add_store(id, Bytes48(pk), b32(0x10), vec![0u8; 8]);
    let base = spawn_server(be).await;
    let client = DigClient::new(base);
    let res = client.push(&id, &b32(0xEE), &b32(0x20), &[1u8; 8], false, None, |msg| {
        Bytes96(digstore_crypto::bls_sign(&sk, msg))
    }).await;
    assert!(matches!(res, Err(digstore_remote::ClientError::NonFastForward)));
}
```
- [ ] Run `cargo test -p digstore-remote --test client_roundtrip`. Expected FAIL initially on compile if `client` module not exported; fix `lib.rs` re-exports. Then expected first run FAIL only if a `reqwest`/`axum::serve` feature is missing (`error[E0432]: unresolved import` or runtime connect error) — ensure `tokio` has `net` and `reqwest` has `json`+`rustls-tls`. After alignment, expected: 8 tests PASS (`push_pending_returns_pending_and_pull_sees_confirmed_not_pending ... ok`, etc).
- [ ] Commit: `git add crates/digstore-remote/src/client.rs crates/digstore-remote/src/lib.rs crates/digstore-remote/tests/client_roundtrip.rs && git commit -m "feat(remote): DigClient fetch/clone/pull/push over reqwest (§21.3-§21.6)"`

---

## Task 15: Production backend adapter over digstore-store + digstore-host

**Files:**
- Create: `crates/digstore-remote/src/backend_store.rs`
- Modify: `crates/digstore-remote/src/lib.rs`
- Test: `crates/digstore-remote/tests/backend_store.rs`

This adapter implements `RemoteBackend` against the real `digstore-store` (generations, on-disk modules, root history) and `digstore-host` (sync wasmtime serving of `get_content`/`get_proof`). It is the production backend; the in-memory one remains for unit tests.

Steps:

- [ ] Inspect the public surfaces of `digstore-store` and `digstore-host` so the adapter calls real APIs. Run `cargo doc -p digstore-store -p digstore-host --no-deps` and read `crates/digstore-store/src/lib.rs` and `crates/digstore-host/src/lib.rs`. Record the exact names for: opening a store by `StoreConfig`/`store_id`, listing `GenerationState` history, reading a generation's module bytes, the public key accessor, and the host "serve a request" entrypoint that returns `ContentResponse`/`ProofResponse` bytes. (These crates are dependencies; this step has no test — it informs the next steps.)
- [ ] Add `pub mod backend_store;` to `src/lib.rs` and `pub use backend_store::StoreBackend;`.
- [ ] Create `crates/digstore-remote/tests/backend_store.rs` with a failing test that builds a `StoreBackend` over a temp-dir store with one genesis generation and asserts `head_state`/`module_bytes` agree with what the store wrote. Write the test against the canonical `StoreConfig { store_id, data_dir, max_size, visibility }` and `GenerationState { id, root, timestamp }` types:
```rust
mod test_helpers;
use test_helpers::*;

use digstore_core::{Bytes32, StoreConfig, Visibility};
use digstore_remote::{backend::RemoteBackend, StoreBackend};

#[test]
fn head_state_matches_store_genesis() {
    let tmp = std::env::temp_dir().join(format!("digstore-remote-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store_id = b32(0x42);
    let config = StoreConfig {
        store_id,
        data_dir: tmp.to_string_lossy().to_string(),
        max_size: 16 * 1024 * 1024,
        visibility: Visibility::Public,
    };
    // build a store with one generation + module via the production helper.
    let backend = StoreBackend::initialize_for_test(config, vec![0u8; 128], b32(0x10))
        .expect("init store backend");
    let hs = backend.head_state(&store_id).unwrap();
    assert_eq!(hs.served_root, b32(0x10));
    assert_eq!(hs.served_size, 128);
}
```
> Note: `initialize_for_test` is a thin test constructor we add to `StoreBackend` that writes a genesis generation and module bytes through the real `digstore-store` API, so the test exercises the production storage path. If `digstore-store` exposes a more idiomatic constructor (verified in the first step of this task), use that exact API and rename the test helper accordingly — do not invent store-side functions.
- [ ] Run `cargo test -p digstore-remote --test backend_store head_state_matches_store_genesis`. Expected FAIL: `StoreBackend` not yet defined (`error[E0432]: unresolved import digstore_remote::StoreBackend`).
- [ ] Create `crates/digstore-remote/src/backend_store.rs` implementing `RemoteBackend`. Hold an `Arc<Mutex<digstore_store::Store>>` (or the canonical store handle) plus a `digstore_host` instance factory. `head_state` reads the latest `GenerationState` + module size + store public key. `module_bytes` reads the served generation's `.wasm` from disk. `serve_content`/`serve_proof` instantiate the host on the served module and invoke `get_content`/`get_proof` inside `spawn_blocking`-friendly synchronous calls, decoding the returned `ContentResponse`/`ProofResponse` via the `digstore-core` custom codec into `(ciphertext, encoded_merkle_proof, roothash)` / `(encoded_proof, roothash)`. `accept_push` writes the pushed module as a new generation (advance) or to a pending slot (pending) via the store API. `delta`/`delta_from_have` compute the chunk-set difference from the store's per-generation chunk index. Use the exact `digstore-store`/`digstore-host` function names recorded in step 1. Provide `StoreBackend::initialize_for_test(config, module_bytes, genesis_root)` that creates the store, writes a genesis generation, and persists the module. Map all not-found cases to `RemoteError::UnknownStore`/`UnknownRoot`, never panicking on missing input.
- [ ] Run `cargo test -p digstore-remote --test backend_store head_state_matches_store_genesis`. Expected: `test head_state_matches_store_genesis ... ok`.
- [ ] Add one more test in the same file asserting `serve_content` on a retrieval miss returns Ok with non-empty decoy bytes (delegating to the host's decoy path, §14.2), and run `cargo test -p digstore-remote --test backend_store`. Expected: 2 tests PASS.
- [ ] Commit: `git add crates/digstore-remote/src/backend_store.rs crates/digstore-remote/src/lib.rs crates/digstore-remote/tests/backend_store.rs && git commit -m "feat(remote): StoreBackend adapter over digstore-store + digstore-host (§18/§21)"`

---

## Task 16: Wire-indistinguishability and full-suite hardening

**Files:**
- Create: `crates/digstore-remote/tests/indistinguishability.rs`
- Modify: `crates/digstore-remote/src/handlers/content.rs` (only if a shape gap is found)

Steps:

- [ ] Create `crates/digstore-remote/tests/indistinguishability.rs` asserting a content hit and a content miss are structurally identical on the wire (same status, same JSON field set, comparable body shape) — the §14.2/§15 indistinguishability property as observed through the REST surface:
```rust
mod test_helpers;
use test_helpers::*;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use digstore_remote::wire::{ContentEnvelope, ContentRequest};

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder().method(Method::POST).uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn hit_and_miss_are_indistinguishable_on_wire() {
    let (be, id, id_hex) = one_store();
    // a real hit whose ciphertext length matches a plausible decoy bucket.
    be.put_content(&id, b32(0x55), vec![7u8; 512], vec![]);
    let app_hit = router_for(be.clone());
    let app_miss = router_for(be);

    let hit_req = ContentRequest { retrieval_key: "55".repeat(32), root: "10".repeat(32), range: None };
    let miss_req = ContentRequest { retrieval_key: "aa".repeat(32), root: "10".repeat(32), range: None };

    let rh = app_hit.oneshot(post_json(&format!("/stores/{id_hex}/content"), &serde_json::to_string(&hit_req).unwrap())).await.unwrap();
    let rm = app_miss.oneshot(post_json(&format!("/stores/{id_hex}/content"), &serde_json::to_string(&miss_req).unwrap())).await.unwrap();

    // same status (200), same content-type.
    assert_eq!(rh.status(), StatusCode::OK);
    assert_eq!(rm.status(), StatusCode::OK);
    assert_eq!(
        rh.headers().get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_string()),
        rm.headers().get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_string()),
    );

    // same JSON field set (envelope shape identical).
    let bh = rh.into_body().collect().await.unwrap().to_bytes();
    let bm = rm.into_body().collect().await.unwrap().to_bytes();
    let eh: ContentEnvelope = serde_json::from_slice(&bh).unwrap();
    let em: ContentEnvelope = serde_json::from_slice(&bm).unwrap();
    // both decode to the same struct shape with all three fields present.
    assert_eq!(eh.roothash, em.roothash);
    assert!(!eh.ciphertext_b64.is_empty());
    assert!(!em.ciphertext_b64.is_empty());
}
```
- [ ] Run `cargo test -p digstore-remote --test indistinguishability`. Expected: `test hit_and_miss_are_indistinguishable_on_wire ... ok`. If it fails because the miss path omits `merkle_proof_b64` differently than the hit, normalize the content handler so both always emit all three fields (a miss already emits an empty-but-present `merkle_proof_b64`).
- [ ] Run the entire crate suite: `cargo test -p digstore-remote`. Expected: all tests across `error`, `etag`, `wire`, `backend_inmem`, `auth`, `ratelimit` (lib), and `descriptor`, `module`, `push`, `content`, `delta`, `ratelimit`, `client_roundtrip`, `backend_store`, `indistinguishability` (integration) PASS. Sample tail: `test result: ok. N passed; 0 failed`.
- [ ] Run `cargo clippy -p digstore-remote --all-targets -- -D warnings`. Expected: no warnings; fix any (unused imports in handler stubs, needless clones).
- [ ] Commit: `git add crates/digstore-remote/tests/indistinguishability.rs crates/digstore-remote/src && git commit -m "test(remote): wire indistinguishability of content hit vs decoy + clippy clean (§14.2/§15)"`

---

## Definition of Done

A checklist mapping the crate's assigned paper sections to the tasks that satisfy them. Every box must be checked, with all `cargo test -p digstore-remote` green and `cargo clippy -p digstore-remote --all-targets -- -D warnings` clean.

- [ ] **§21.1 Overview** — remote is also a serving provider (inherits blindness): satisfied by `StoreBackend` reusing `digstore-host` serving (Task 15) and the decoy-safe content path (Task 11).
- [ ] **§21.2 REST Surface** — `GET /stores/{id}`, `GET /stores/{id}/roots`, `HEAD/GET/PUT /stores/{id}/module`, `POST /stores/{id}/content`, `POST /stores/{id}/proof`, `GET/POST /stores/{id}/delta`: Tasks 8 (descriptor/roots), 9 (HEAD/GET module), 10 (PUT module), 11 (content/proof), 12 (delta).
- [ ] **§21.3 Clone and Fetch** — `DigClient::fetch` (descriptor+roots only) and `clone_store` (download+verify): Task 14.
- [ ] **§21.4 Pull and Head Advancement** — served vs pending head; pull sees confirmed not pending; 202 on pending push: Tasks 5 (backend state), 10 (202 + served head unchanged), 14 (`pull` + pending-not-served e2e).
- [ ] **§21.5 Delta Sync** — `GET /delta?from=&to=` returns exactly new chunks + key-table changes; `POST /delta` negotiates from have-summary: Tasks 12 (handlers), 14 (`pull` delta path + `negotiate_delta`).
- [ ] **§21.6 Push** — publisher BLS signature over `SHA-256(root)` bound to store id verified vs store public key; optional bearer (401/403); fast-forward-only (409): Tasks 6 (auth verify), 10 (handler enforcement), 14 (`push` signs).
- [ ] **§21.7 ETags and Caching** — ETag = root; `If-None-Match` match → 304: Tasks 2 (etag logic), 9 (304 on module GET), 14 (client conditional pull).
- [ ] **§21.8 Status Codes** — 200/201/202/304/401/403/404(never for content)/409/413/422/429: Task 1 (mapping), 9 (304/404), 10 (201/202/401/403/409/413/422), 11 (200 decoy, 404 only for unknown store), 13 (429).
- [ ] Documented deviation stated in code (`src/lib.rs` doc): the REST envelope is JSON for transport ergonomics while store content/proof/key-table blobs remain Chia big-endian custom-codec encoded (consistent with the locked codec decision).
- [ ] `RemoteServer::new(backend)` and `DigClient` public API present and exercised by `tests/client_roundtrip.rs` end-to-end against a live `axum::serve` listener.


---

## Plan metadata

- **Crate:** digstore-remote
- **Assigned paper sections:** 21.1,21.2,21.3,21.4,21.5,21.6,21.7,21.8
- **Depends on:** digstore-core, digstore-store, digstore-host, digstore-crypto
- **Spec sections covered (claimed):** 21.1, 21.2, 21.3, 21.4, 21.5, 21.6, 21.7, 21.8

### Public items exported (consumed by other crates)

```
pub struct RemoteServer
impl RemoteServer { pub fn new(backend: std::sync::Arc<dyn RemoteBackend>) -> Self }
impl RemoteServer { pub fn with_rate_limiter(backend: std::sync::Arc<dyn RemoteBackend>, rl: std::sync::Arc<RateLimiter>) -> Self }
impl RemoteServer { pub fn router(&self) -> axum::Router }
#[derive(Clone)] pub struct AppState { pub backend: std::sync::Arc<dyn RemoteBackend>, pub rate_limiter: std::sync::Arc<RateLimiter> }
pub trait RemoteBackend: Send + Sync + 'static { fn head_state(&self, store_id: &Bytes32) -> Result<HeadState, RemoteError>; fn root_history(&self, store_id: &Bytes32) -> Result<Vec<RootRecord>, RemoteError>; fn module_bytes(&self, store_id: &Bytes32, root: Option<&Bytes32>) -> Result<Vec<u8>, RemoteError>; fn serve_content(&self, store_id: &Bytes32, retrieval_key: &Bytes32, root: &Bytes32, range: Option<(u64,u64)>) -> Result<(Vec<u8>, Vec<u8>, Bytes32), RemoteError>; fn serve_proof(&self, store_id: &Bytes32, retrieval_key: &Bytes32, root: &Bytes32) -> Result<(Vec<u8>, Bytes32), RemoteError>; fn accept_push(&self, store_id: &Bytes32, parent: &Bytes32, new_root: &Bytes32, module_bytes: &[u8], mode: PushMode) -> Result<PushOutcome, RemoteError>; fn delta(&self, store_id: &Bytes32, from: &Bytes32, to: &Bytes32) -> Result<DeltaSet, RemoteError>; fn delta_from_have(&self, store_id: &Bytes32, to: &Bytes32, have: &[Bytes32]) -> Result<DeltaSet, RemoteError>; fn max_module_size(&self) -> u64; fn requires_bearer(&self, store_id: &Bytes32) -> bool; fn check_bearer(&self, store_id: &Bytes32, token: Option<&str>) -> bool; }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct HeadState { pub served_root: Bytes32, pub pending_root: Option<Bytes32>, pub served_size: u64, pub public_key: Bytes48 }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct RootRecord { pub generation: u64, pub root: Bytes32, pub timestamp: u64 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum PushMode { Advance, Pending }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum PushOutcome { Advanced, Pending }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct DeltaSet { pub from: Bytes32, pub to: Bytes32, pub new_chunks: Vec<(Bytes32, Vec<u8>)>, pub key_table_changes: Vec<Vec<u8>> }
pub struct InMemoryBackend
impl InMemoryBackend { pub fn new() -> Self }
impl InMemoryBackend { pub fn with_max_module_size(max: u64) -> Self }
impl InMemoryBackend { pub fn add_store(&self, store_id: Bytes32, public_key: Bytes48, genesis_root: Bytes32, module: Vec<u8>) }
impl InMemoryBackend { pub fn add_generation(&self, store_id: &Bytes32, parent: Bytes32, new_root: Bytes32, module: Vec<u8>, chunks: Vec<(Bytes32, Vec<u8>)>, key_table_changes: Vec<Vec<u8>>, advance: bool) }
impl InMemoryBackend { pub fn set_bearer(&self, store_id: &Bytes32, token: &str) }
impl InMemoryBackend { pub fn put_content(&self, store_id: &Bytes32, retrieval_key: Bytes32, ciphertext: Vec<u8>, proof: Vec<u8>) }
pub struct StoreBackend
impl StoreBackend { pub fn initialize_for_test(config: StoreConfig, module_bytes: Vec<u8>, genesis_root: Bytes32) -> Result<Self, RemoteError> }
pub struct DigClient
impl DigClient { pub fn new(base_url: impl Into<String>) -> Self }
impl DigClient { pub fn with_client(base_url: impl Into<String>, http: reqwest::Client) -> Self }
impl DigClient { pub async fn fetch(&self, store_id: &Bytes32) -> Result<FetchInfo, ClientError> }
impl DigClient { pub async fn clone_store<V: FnOnce(&[u8], &Bytes32) -> Result<(), String>>(&self, store_id: &Bytes32, verify: V) -> Result<(Bytes32, Vec<u8>), ClientError> }
impl DigClient { pub async fn pull(&self, store_id: &Bytes32, local_root: Option<Bytes32>, prefer_delta: bool) -> Result<PullResult, ClientError> }
impl DigClient { pub async fn push<S: FnOnce(&[u8;32]) -> Bytes96>(&self, store_id: &Bytes32, parent: &Bytes32, new_root: &Bytes32, module: &[u8], pending: bool, bearer: Option<&str>, sign: S) -> Result<PushResult, ClientError> }
impl DigClient { pub async fn negotiate_delta(&self, store_id: &Bytes32, to: &Bytes32, have: &[Bytes32]) -> Result<DeltaResponse, ClientError> }
#[derive(Debug, Clone)] pub struct FetchInfo { pub descriptor: StoreDescriptor, pub roots: RootHistory }
#[derive(Debug, Clone)] pub enum PullResult { UpToDate, Module { root: Bytes32, bytes: Vec<u8> }, Delta { root: Bytes32, delta: DeltaResponse } }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum PushResult { Advanced, Pending }
pub struct RateLimiter
impl RateLimiter { pub fn new(capacity: u32) -> Self }
impl RateLimiter { pub fn try_acquire(&self, store_id: &Bytes32) -> bool }
impl RateLimiter { pub fn refill(&self, store_id: &Bytes32) }
pub fn etag_for_root(root: &Bytes32) -> String
pub fn parse_if_none_match(header: &str) -> Option<Bytes32>
pub fn matches_current(header: &str, current_root: &Bytes32) -> bool
pub fn push_signing_message(store_id: &Bytes32, root: &Bytes32) -> [u8; 32]
pub fn verify_push_signature(store_public_key: &Bytes48, store_id: &Bytes32, root: &Bytes32, signature: &Bytes96) -> bool
#[derive(Debug, Clone)] pub struct PushAuth { pub signature: Bytes96, pub bearer: Option<String> }
#[derive(Debug, Error)] pub enum RemoteError { UnknownStore, UnknownRoot, Unauthorized(String), MissingBearer, NonFastForward, TooLarge(u64), Validation(String), RateLimited, BadRequest(String), Internal(String) }
impl RemoteError { pub fn status(&self) -> http::StatusCode }
#[derive(Debug, Error)] pub enum ClientError { Transport(String), Status(u16), Verification(String), Decode(String), NonFastForward, Unauthorized(u16) }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct StoreDescriptor { pub current_root: String, pub size: u64, pub public_key: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct RootHistory { pub roots: Vec<RootEntry> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct RootEntry { pub generation: u64, pub root: String, pub timestamp: u64 }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct ContentRequest { pub retrieval_key: String, pub root: String, pub range: Option<ByteRange> }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] pub struct ByteRange { pub start: u64, pub end: u64 }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct ContentEnvelope { pub ciphertext_b64: String, pub merkle_proof_b64: String, pub roothash: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct ProofRequest { pub retrieval_key: String, pub root: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct ProofEnvelope { pub proof_b64: String, pub roothash: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct DeltaResponse { pub from: String, pub to: String, pub chunks: Vec<DeltaChunk>, pub key_table_changes: Vec<KeyTableChange> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct DeltaChunk { pub hash: String, pub data_b64: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct KeyTableChange { pub entry_b64: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct DeltaNegotiateRequest { pub to: String, pub have: Vec<String> }
```