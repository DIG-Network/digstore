# digstore-prover Implementation Plan

> **For agentic workers:** This plan is executed with **superpowers:subagent-driven-development** (REQUIRED SUB-SKILL). Each numbered step is one bite-sized (2-5 min) action. Follow strict TDD: write a failing test (full code shown), run it and observe the exact expected FAIL line, write the minimal implementation (full code shown), run the test and observe PASS, then commit with the exact conventional-commit message given. Do not batch steps. Do not skip the red phase. Steps explicitly tagged **GUARD** are regression tests for behavior already implemented in an earlier task — they are expected to PASS on first run and carry NO impl step; they are clearly distinguished from RED steps and must never be mislabeled as TDD red→green cycles.

**Goal:** Implement digstore execution proofs and Chia chain anchoring (§13) behind `Prover`/`Verifier`/`ChainSource` traits, with a deterministic `MockProver` default, a feature-gated `Risc0Prover`, a feature-gated `HardwareAttestProver`, and `CoinsetChainSource` + `MockChainSource`.

**Architecture:** A trait layer (`Prover`, `Verifier`, `ChainSource`) decouples proof generation/verification from backend. The default `MockProver`/`MockVerifier` produce and check a deterministic SHA-256 commitment chain that cryptographically binds `program_hash`, `public_input`, `public_output`, and `roothash` so every other crate's tests can build and verify `ExecutionProof`s without a ZK toolchain. `Risc0Prover`/`Risc0Verifier` (feature `risc0`) re-execute the deterministic serving computation in a risc0 guest and commit `(program_hash, public_input_hash, roothash, public_output)`; `HardwareAttestProver`/`HardwareVerifier` (feature `hardware-attest`) carry a TEE/HSM attestation in the proof bytes. `CoinsetChainSource` hits `https://api.coinset.org` and walks down to the previous transaction block for timestamps; `MockChainSource` is deterministic for tests.

**Tech Stack:** Rust (host-side, `std`); `digstore-core` (canonical types `Bytes32`/`Bytes48`/`Bytes96`/`ExecutionProof`/`ChiaBlockRef`/`ProofResponse`, codec `Encode`/`Decode`, `Bytes32::to_hex`/`from_hex`); `digstore-crypto` (`sha256`, `bls` sign/verify wrappers, Chia AugScheme); `reqwest` (blocking) + `serde`/`serde_json` for `CoinsetChainSource`; `thiserror` for errors; `risc0-zkvm` + `risc0-build` + `bincode` (optional, feature `risc0`).

> **Pinned upstream contract (resolves all "use whichever core exposes" hedges).** This crate compiles against these exact signatures from its dependencies; if `digstore-core`/`digstore-crypto` differ, fix them THERE, do not hedge here:
> - `digstore_core::Bytes32::to_hex(&self) -> String` and `digstore_core::Bytes32::from_hex(&str) -> Result<Bytes32, digstore_core::HexError>`. Same `to_hex`/`from_hex` on `Bytes48`/`Bytes96`. Tuple field `.0` is the raw `[u8; N]`. There is NO `digstore_core::hex::encode/decode` free function — always use the newtype methods.
> - `digstore_core::codec::Encode::encode(&self, out: &mut Vec<u8>)` (infallible, appends) and `digstore_core::codec::Decode::decode(input: &mut &[u8]) -> Result<Self, digstore_core::codec::CodecError>` (advances the slice). `ChiaBlockRef` implements both with the Chia-streamable big-endian layout: `header_hash` raw 32 bytes, `height` u32 BE, `timestamp` u64 BE (44 bytes total).
> - `digstore_crypto::sha256(&[u8]) -> [u8; 32]`.
> - `digstore_crypto::bls` module (Chia AugScheme, augmentation handled INSIDE sign/verify — callers pass the raw message): `SecretKey::from_seed(&[u8; 32]) -> SecretKey`; `SecretKey::public_key(&self) -> PublicKey`; `PublicKey::to_bytes(&self) -> [u8; 48]`; `bls::sign(&SecretKey, msg: &[u8]) -> Signature`; `Signature::to_bytes(&self) -> [u8; 96]`; `bls::verify(pubkey: &[u8; 48], msg: &[u8], sig: &[u8; 96]) -> bool`.

---

## File Structure

All paths under `crates/digstore-prover/`.

| Path | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest; deps + `risc0` / `hardware-attest` features (final, complete in Task 1) |
| `build.rs` | risc0 methods embed (no-op unless `risc0` feature on) |
| `src/lib.rs` | Crate root; re-exports; module wiring; crate-level docs incl. deviation #3 |
| `src/error.rs` | `ProverError` enum (thiserror) + `Result` alias |
| `src/serving_inputs.rs` | `ServingInputs` struct + `output_bytes` + `compute_public_output` (roothash-bound) |
| `src/prover.rs` | `Prover` + `Verifier` traits (incl. `verify_response`, `verify_with_nonce`) |
| `src/chain.rs` | `ChainSource` trait |
| `src/commitment.rs` | `public_input` build/parse (strict-length), signing-message build, mock commitment helper |
| `src/mock.rs` | `MockProver` + `MockVerifier` (default backend) + `DEFAULT_FRESHNESS_WINDOW_SECS` |
| `src/mock_chain.rs` | `MockChainSource` (deterministic peak + freshness) |
| `src/coinset.rs` | `CoinsetChainSource` (reqwest → api.coinset.org, tx-block walk-down) + DTOs + parsers |
| `src/risc0_backend.rs` | `Risc0Prover` + `Risc0Verifier` (feature `risc0`) |
| `src/hardware.rs` | `HardwareAttestProver` + `HardwareVerifier` (feature `hardware-attest`) |
| `guest/Cargo.toml` | risc0 guest crate manifest |
| `guest/src/main.rs` | risc0 guest: re-executes serving computation, commits the journal |
| `tests/mock_roundtrip.rs` | MockProver/MockVerifier prove→verify happy path |
| `tests/roothash_binding.rs` | §13.4 proof is cryptographically bound to roothash (different-trusted-root rejected) |
| `tests/nonce_binding.rs` | §13.5 nonce-A proof rejected against nonce-B request |
| `tests/chain_freshness.rs` | §13.8 block outside/inside freshness window (GUARD) |
| `tests/node_attribution.rs` | §13.7 node BLS signature accept/reject (GUARD) |
| `tests/program_hash.rs` | program_hash mismatch rejected (GUARD) |
| `tests/public_output.rs` | public_output mismatch rejected (GUARD) |
| `tests/coinset_parse.rs` | Parse recorded coinset.org JSON fixtures → `ChiaBlockRef`, incl. tx-block walk-down |
| `tests/trait_object_safety.rs` | `Box<dyn Prover/Verifier/ChainSource>` usable (GUARD) |
| `tests/fixtures/get_blockchain_state.json` | Recorded coinset.org peak fixture (transaction block) |
| `tests/fixtures/get_block_record_by_height_notx.json` | Non-transaction block (no timestamp) fixture |
| `tests/fixtures/get_block_record_prev_tx.json` | Previous transaction block fixture (walk-down target) |
| `tests/hardware_attest.rs` | §13.6 hardware-attest prove→verify + tamper reject (feature `hardware-attest`) |
| `tests/risc0_smoke.rs` | §13.1-13.4 risc0 prove→verify smoke (feature `risc0`, `#[ignore]`) |

---

## Task 1 — Crate scaffold, final manifest, error type

**Files:**
- Modify: workspace root `Cargo.toml`
- Create: `crates/digstore-prover/Cargo.toml`
- Create: `crates/digstore-prover/build.rs`
- Create: `crates/digstore-prover/src/lib.rs`
- Create: `crates/digstore-prover/src/error.rs`
- Test: inline `#[cfg(test)]` in `error.rs`

Steps:

- [ ] **1.1 Add crate to the workspace.** Edit the workspace root `crates/../Cargo.toml` (`C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml`): in the `[workspace]` table add `"crates/digstore-prover"` to the `members` array (create `[workspace]` with `resolver = "2"` and `members = [ ... ]` if absent).

- [ ] **1.2 Write the FINAL complete `Cargo.toml`.** Create `crates/digstore-prover/Cargo.toml` with every dependency and feature wiring in one shot (no later manifest edits). Note every optional dep is referenced by the `risc0` feature, so the manifest parses cleanly even on a default build:
```toml
[package]
name = "digstore-prover"
version = "0.1.0"
edition = "2021"
description = "Execution proofs and Chia chain anchoring for digstore (§13)."

[dependencies]
digstore-core = { path = "../digstore-core" }
digstore-crypto = { path = "../digstore-crypto" }
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }

# risc0 backend (real ZK) — all optional, all referenced by the `risc0` feature below
risc0-zkvm = { version = "1.2", optional = true }
digstore-guest-risc0 = { path = "guest", optional = true }
bincode = { version = "1", optional = true }

[build-dependencies]
risc0-build = { version = "1.2", optional = true }

[features]
default = []
risc0 = ["dep:risc0-zkvm", "dep:digstore-guest-risc0", "dep:bincode", "dep:risc0-build"]
hardware-attest = []

[dev-dependencies]
serde_json = "1"

[package.metadata.risc0]
methods = ["guest"]
```

- [ ] **1.3 Write `build.rs`.** Create `crates/digstore-prover/build.rs`. The body is gated on `CARGO_FEATURE_RISC0` (which Cargo sets and `cfg(feature = ...)` reads in build scripts), so a default build is a true no-op and never requires `risc0-build`:
```rust
fn main() {
    #[cfg(feature = "risc0")]
    risc0_build::embed_methods();
}
```

- [ ] **1.4 Write the crate root.** Create `crates/digstore-prover/src/lib.rs`:
```rust
//! digstore-prover — execution proofs (§13) and Chia chain anchoring.
//!
//! # Documented deviation #3 (paper §13)
//! risc0 proves a faithful re-execution of the *deterministic serving
//! computation* (resolve retrieval key -> key-table lookup -> gather +
//! concatenate chunk ciphertext -> commit output), NOT wasmtime opcodes.
//! `program_hash = SHA-256(module_bytes)`. The [`mock::MockProver`] is the
//! default backend so the rest of the system is fully functional while the
//! real risc0 circuit matures.

pub mod error;
pub mod serving_inputs;
pub mod prover;
pub mod chain;
pub mod commitment;
pub mod mock;
pub mod mock_chain;
pub mod coinset;

#[cfg(feature = "risc0")]
pub mod risc0_backend;

#[cfg(feature = "hardware-attest")]
pub mod hardware;

pub use error::{ProverError, Result};
pub use serving_inputs::ServingInputs;
pub use prover::{Prover, Verifier};
pub use chain::ChainSource;
pub use mock::{MockProver, MockVerifier, DEFAULT_FRESHNESS_WINDOW_SECS};
pub use mock_chain::MockChainSource;
pub use coinset::CoinsetChainSource;
pub use commitment::{build_public_input, parse_public_input, signing_message, NONCE_LEN};
```

- [ ] **1.5 RED: error-type display test.** Create `crates/digstore-prover/src/error.rs` with the impl AND its test in one file (the impl is genuinely new code, the test asserts its Display contract):
```rust
use thiserror::Error;

/// Errors produced by provers, verifiers, and chain sources (§13).
#[derive(Debug, Error)]
pub enum ProverError {
    #[error("program hash mismatch: expected {expected}, got {actual}")]
    ProgramHashMismatch { expected: String, actual: String },
    #[error("public output mismatch")]
    PublicOutputMismatch,
    #[error("public input commitment mismatch")]
    PublicInputMismatch,
    #[error("nonce binding mismatch: proof bound to a different request")]
    NonceMismatch,
    #[error("response roothash {0} is not in the trusted set")]
    UntrustedRoot(String),
    #[error("proof is bound to roothash {bound}, but response asserts {asserted}")]
    RootBindingMismatch { bound: String, asserted: String },
    #[error("node signature verification failed")]
    NodeSignatureInvalid,
    #[error("chain block outside freshness window: block ts {block_ts}, now {now}, window {window}s")]
    BlockTooOld { block_ts: u64, now: u64, window: u64 },
    #[error("chain block timestamp {0} is in the future relative to now {1}")]
    BlockInFuture(u64, u64),
    #[error("block not found on trusted chain: {0}")]
    BlockNotOnChain(String),
    #[error("zk proof verification failed: {0}")]
    ZkProofInvalid(String),
    #[error("hardware attestation verification failed: {0}")]
    AttestationInvalid(String),
    #[error("proving backend failure: {0}")]
    Backend(String),
    #[error("codec error: {0}")]
    Codec(String),
    #[error("chain RPC error: {0}")]
    ChainRpc(String),
}

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, ProverError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_is_descriptive() {
        let e = ProverError::ProgramHashMismatch { expected: "aa".into(), actual: "bb".into() };
        assert_eq!(e.to_string(), "program hash mismatch: expected aa, got bb");
    }

    #[test]
    fn root_binding_error_display() {
        let e = ProverError::RootBindingMismatch { bound: "aa".into(), asserted: "bb".into() };
        assert_eq!(e.to_string(), "proof is bound to roothash aa, but response asserts bb");
    }
}
```
Run: `cargo test -p digstore-prover error_display_is_descriptive`
Expected FAIL (RED): the crate does not yet compile because `serving_inputs`, `prover`, `chain`, `commitment`, `mock`, `mock_chain`, `coinset` are declared in `lib.rs` but not created. Exact first line:
```
error[E0583]: file not found for module `serving_inputs`
```
This confirms wiring is correct; the modules are created in the next tasks. (We deliberately keep this RED until the dependent modules exist; the error-type code itself is complete.)

- [ ] **1.6 Commit.** `git add crates/digstore-prover/Cargo.toml crates/digstore-prover/build.rs crates/digstore-prover/src/lib.rs crates/digstore-prover/src/error.rs Cargo.toml` then `git commit -m "feat(prover): scaffold digstore-prover crate, final manifest, error type"`.

---

## Task 2 — ServingInputs + roothash-bound serving computation (§13.4 binding)

**Files:**
- Create: `crates/digstore-prover/src/serving_inputs.rs`
- Test: inline `#[cfg(test)]`

> **Blocker fix (roothash binding):** `compute_public_output` now hashes the roothash INTO the commitment: `public_output = SHA-256(roothash.0 || concat(chunk_ciphertext))`. This cryptographically binds the response to a specific generation root (§13.4), so a genuine proof can no longer be re-paired with a *different* trusted root. `output_bytes()` returns exactly the concatenated ciphertext and is reused by `compute_public_output` so it is exercised by every output test.

Steps:

- [ ] **2.1 RED: ServingInputs roothash-bound output test.** Create `crates/digstore-prover/src/serving_inputs.rs` with ONLY the `use` line and the test block:
```rust
use digstore_core::Bytes32;

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_crypto::sha256;

    fn inputs(root: [u8; 32], chunks: Vec<Vec<u8>>) -> ServingInputs {
        ServingInputs { retrieval_key: Bytes32([7u8; 32]), roothash: Bytes32(root), chunk_ciphertext: chunks }
    }

    #[test]
    fn public_output_binds_roothash_then_ciphertext() {
        let inp = inputs([9u8; 32], vec![vec![1, 2, 3], vec![4, 5]]);
        // commitment = SHA-256( roothash || concat(ciphertext) )
        let mut preimage = vec![9u8; 32];
        preimage.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(inp.compute_public_output(), Bytes32(sha256(&preimage)));
    }

    #[test]
    fn output_bytes_is_concatenated_ciphertext() {
        let inp = inputs([0u8; 32], vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]]);
        assert_eq!(inp.output_bytes(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn different_ciphertext_gives_different_output() {
        let a = inputs([9u8; 32], vec![vec![1, 2, 3]]);
        let b = inputs([9u8; 32], vec![vec![1, 2, 4]]);
        assert_ne!(a.compute_public_output(), b.compute_public_output());
    }

    #[test]
    fn different_roothash_gives_different_output() {
        let a = inputs([9u8; 32], vec![vec![1, 2, 3]]);
        let b = inputs([8u8; 32], vec![vec![1, 2, 3]]);
        assert_ne!(a.compute_public_output(), b.compute_public_output());
    }
}
```
Run: `cargo test -p digstore-prover public_output_binds_roothash_then_ciphertext`
Expected FAIL (RED):
```
error[E0422]: cannot find struct, variant or union type `ServingInputs` in this scope
```

- [ ] **2.2 GREEN: implement ServingInputs.** Insert above the `#[cfg(test)]` block (after the existing `use digstore_core::Bytes32;`):
```rust
use digstore_crypto::sha256;

/// Inputs to the deterministic serving computation that a proof attests
/// (deviation #3). The serving node resolves a retrieval key, looks it up in
/// the key table, gathers and concatenates the resource's chunk ciphertext,
/// and commits the result bound to the generation root. The risc0 guest
/// re-runs exactly this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingInputs {
    /// Resolved retrieval key for the request.
    pub retrieval_key: Bytes32,
    /// Generation root the response is bound to (§13.4).
    pub roothash: Bytes32,
    /// The gathered chunk ciphertext, in order, for the resolved resource.
    pub chunk_ciphertext: Vec<Vec<u8>>,
}

impl ServingInputs {
    /// The concatenated, in-order chunk ciphertext (the returned bytes).
    pub fn output_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in &self.chunk_ciphertext {
            out.extend_from_slice(chunk);
        }
        out
    }

    /// The serving computation's `public_output` commitment:
    /// `SHA-256( roothash || concat(chunk_ciphertext) )`. Binding the root
    /// into the commitment means a genuine proof cannot be re-paired with a
    /// different generation root (§13.4).
    pub fn compute_public_output(&self) -> Bytes32 {
        let mut preimage = Vec::with_capacity(32 + self.chunk_ciphertext.iter().map(|c| c.len()).sum::<usize>());
        preimage.extend_from_slice(&self.roothash.0);
        preimage.extend_from_slice(&self.output_bytes());
        Bytes32(sha256(&preimage))
    }
}
```
Run: `cargo test -p digstore-prover -- serving_inputs::tests`
Expected PASS: `test result: ok. 4 passed`.

- [ ] **2.3 Commit.** `git add crates/digstore-prover/src/serving_inputs.rs` then `git commit -m "feat(prover): ServingInputs with roothash-bound public_output (§13.4, deviation #3)"`.

---

## Task 3 — Commitment helpers: strict public_input codec + signing message

**Files:**
- Create: `crates/digstore-prover/src/commitment.rs`
- Test: inline `#[cfg(test)]`

> `public_input = client_nonce(32) || ChiaBlockRef(codec)`. `ChiaBlockRef` encodes via the pinned Chia-streamable codec (`header_hash` raw 32 bytes, `height` u32 BE, `timestamp` u64 BE = 44 bytes; total `public_input` = 76 bytes). The signing message is `proof || public_input` (§13.7). **Minor fix:** `parse_public_input` now rejects trailing bytes (over-length input).

Steps:

- [ ] **3.1 RED: build/parse round-trip + length strictness.** Create `crates/digstore-prover/src/commitment.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_core::codec::{Encode, Decode};
use crate::error::{ProverError, Result};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_block() -> ChiaBlockRef {
        ChiaBlockRef { header_hash: Bytes32([0xABu8; 32]), height: 5_000_000, timestamp: 1_900_000_000 }
    }

    #[test]
    fn public_input_round_trips() {
        let nonce = [0x11u8; 32];
        let block = sample_block();
        let pi = build_public_input(&nonce, &block);
        assert_eq!(pi.len(), 76); // 32 nonce + 32 header_hash + 4 height + 8 timestamp
        let (got_nonce, got_block) = parse_public_input(&pi).unwrap();
        assert_eq!(got_nonce, nonce);
        assert_eq!(got_block, block);
    }

    #[test]
    fn parse_rejects_short_input() {
        let short = vec![0u8; 10];
        assert!(matches!(parse_public_input(&short), Err(ProverError::Codec(_))));
    }

    #[test]
    fn parse_rejects_trailing_bytes() {
        let nonce = [0x11u8; 32];
        let mut pi = build_public_input(&nonce, &sample_block());
        pi.push(0xFF); // 77 bytes — one trailing byte
        assert!(matches!(parse_public_input(&pi), Err(ProverError::Codec(_))));
    }

    #[test]
    fn signing_message_is_proof_then_public_input() {
        let pi = vec![9u8; 76];
        let proof = vec![1u8, 2, 3];
        let msg = signing_message(&proof, &pi);
        assert_eq!(msg.len(), proof.len() + pi.len());
        assert_eq!(&msg[..3], &proof[..]);
        assert_eq!(&msg[3..], &pi[..]);
    }
}
```
Run: `cargo test -p digstore-prover public_input_round_trips`
Expected FAIL (RED):
```
error[E0425]: cannot find function `build_public_input` in this scope
```

- [ ] **3.2 GREEN: implement commitment helpers.** Insert above the test block (after the `use` lines):
```rust
/// Length of the client nonce that prefixes `public_input` (§13.5).
pub const NONCE_LEN: usize = 32;

/// Fixed encoded length of a `ChiaBlockRef` (header_hash 32 + height 4 + ts 8).
const CHIA_BLOCK_REF_LEN: usize = 44;

/// `public_input = client_nonce(32) || ChiaBlockRef(codec)`.
pub fn build_public_input(nonce: &[u8; 32], block: &ChiaBlockRef) -> Vec<u8> {
    let mut out = Vec::with_capacity(NONCE_LEN + CHIA_BLOCK_REF_LEN);
    out.extend_from_slice(nonce);
    block.encode(&mut out);
    out
}

/// Inverse of [`build_public_input`]. Rejects under- AND over-length input.
pub fn parse_public_input(bytes: &[u8]) -> Result<([u8; 32], ChiaBlockRef)> {
    if bytes.len() < NONCE_LEN {
        return Err(ProverError::Codec("public_input too short for nonce".into()));
    }
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&bytes[..NONCE_LEN]);
    let mut cursor: &[u8] = &bytes[NONCE_LEN..];
    let block = ChiaBlockRef::decode(&mut cursor)
        .map_err(|e| ProverError::Codec(format!("ChiaBlockRef decode: {e:?}")))?;
    if !cursor.is_empty() {
        return Err(ProverError::Codec(format!(
            "public_input has {} trailing bytes after ChiaBlockRef",
            cursor.len()
        )));
    }
    Ok((nonce, block))
}

/// The message a node signs for attribution: `proof || public_input` (§13.7).
pub fn signing_message(proof: &[u8], public_input: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(proof.len() + public_input.len());
    msg.extend_from_slice(proof);
    msg.extend_from_slice(public_input);
    msg
}
```
Run: `cargo test -p digstore-prover -- commitment::tests`
Expected PASS: `test result: ok. 4 passed`.

- [ ] **3.3 Commit.** `git add crates/digstore-prover/src/commitment.rs` then `git commit -m "feat(prover): strict public_input codec + node signing message helpers"`.

---

## Task 4 — Core traits: Prover, Verifier, ChainSource (committed as one unit)

**Files:**
- Create: `crates/digstore-prover/src/prover.rs`
- Create: `crates/digstore-prover/src/chain.rs`

> Traits only. `Verifier` carries the core `verify` plus two default-method helpers (`verify_response` for §13.4 root binding, `verify_with_nonce` for §13.5). All trait methods are object-safe (no generics, no `Self` by value). **TDD-violation fix:** chain.rs is committed in THIS task as part of the cohesive trait-definition unit.

Steps:

- [ ] **4.1 Write `Prover`/`Verifier` traits.** Create `crates/digstore-prover/src/prover.rs`:
```rust
use digstore_core::{Bytes32, ExecutionProof, ProofResponse};
use crate::serving_inputs::ServingInputs;
use crate::chain::ChainSource;
use crate::error::{ProverError, Result};

/// Produces an [`ExecutionProof`] for a serving run (§13.1-13.3).
///
/// `public_input` is `client_nonce(32) || ChiaBlockRef(codec)` (build via
/// [`crate::commitment::build_public_input`]). `serving_inputs` carries the
/// deterministic serving-computation inputs (deviation #3).
pub trait Prover {
    fn prove(
        &self,
        program_hash: Bytes32,
        public_input: &[u8],
        serving_inputs: &ServingInputs,
    ) -> Result<ExecutionProof>;
}

/// Verifies an [`ExecutionProof`] (§13.4-13.8): program-hash match, ZK /
/// attestation validity, output commitment, node BLS attribution, and chain
/// freshness via `chain`.
pub trait Verifier {
    fn verify(
        &self,
        proof: &ExecutionProof,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()>;

    /// Verify a full [`ProofResponse`] (§13.4): the inner proof must verify,
    /// the response's `roothash` must be in `trusted_roots`, AND that root
    /// must equal the root the proof is cryptographically bound to (recovered
    /// by recomputing the output commitment via `expected_output_bytes`).
    fn verify_response(
        &self,
        response: &ProofResponse,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        expected_output_bytes: &[u8],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        if !trusted_roots.contains(&response.roothash) {
            return Err(ProverError::UntrustedRoot(response.roothash.to_hex()));
        }
        // Recompute the bound commitment from the asserted root + returned bytes;
        // if the proof's committed output disagrees, the root binding is forged.
        let bound = bound_public_output(&response.roothash, expected_output_bytes);
        if bound != response.proof.public_output {
            return Err(ProverError::RootBindingMismatch {
                bound: bound.to_hex(),
                asserted: response.roothash.to_hex(),
            });
        }
        self.verify(&response.proof, expected_program_hash, trusted_roots, chain)
    }

    /// Verify a proof AND confirm it is bound to `expected_nonce` (§13.5).
    /// A proof for any other nonce is rejected, defeating replay.
    fn verify_with_nonce(
        &self,
        proof: &ExecutionProof,
        expected_nonce: &[u8; 32],
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        let (nonce, _block) = crate::commitment::parse_public_input(&proof.public_input)?;
        if &nonce != expected_nonce {
            return Err(ProverError::NonceMismatch);
        }
        self.verify(proof, expected_program_hash, trusted_roots, chain)
    }
}

/// Recompute the roothash-bound output commitment from the asserted root and
/// the returned bytes: `SHA-256( roothash || returned_bytes )`. Mirrors
/// [`ServingInputs::compute_public_output`].
pub fn bound_public_output(roothash: &Bytes32, output_bytes: &[u8]) -> Bytes32 {
    let mut preimage = Vec::with_capacity(32 + output_bytes.len());
    preimage.extend_from_slice(&roothash.0);
    preimage.extend_from_slice(output_bytes);
    Bytes32(digstore_crypto::sha256(&preimage))
}
```

- [ ] **4.2 Write `ChainSource` trait.** Create `crates/digstore-prover/src/chain.rs`:
```rust
use digstore_core::ChiaBlockRef;
use crate::error::Result;

/// A source of Chia chain state for anchoring proofs to wall-clock time
/// (§13.8). Implemented by [`crate::mock_chain::MockChainSource`] and
/// [`crate::coinset::CoinsetChainSource`].
pub trait ChainSource {
    /// The current peak block of the trusted chain.
    fn get_peak(&self) -> Result<ChiaBlockRef>;

    /// Confirm `block` is a real block on the trusted chain AND that its
    /// timestamp falls within `freshness_window_secs` of "now". Rejects blocks
    /// that are unknown, too old, or in the future.
    fn verify_block(&self, block: &ChiaBlockRef, freshness_window_secs: u64) -> Result<()>;
}
```

- [ ] **4.3 Build check.** Run: `cargo build -p digstore-prover`
Expected FAIL:
```
error[E0583]: file not found for module `mock`
```
This confirms `prover.rs`/`chain.rs` compile and the only remaining gaps are `mock`/`mock_chain`/`coinset`, created next.

- [ ] **4.4 Commit the trait-definition unit.** `git add crates/digstore-prover/src/prover.rs crates/digstore-prover/src/chain.rs` then `git commit -m "feat(prover): Prover/Verifier/ChainSource trait definitions"`.

---

## Task 5 — MockChainSource (§13.8 substrate)

**Files:**
- Create: `crates/digstore-prover/src/mock_chain.rs`
- Test: inline `#[cfg(test)]`

> Fully deterministic: holds a known set of blocks and a fixed "now". `verify_block` accepts iff the block is in the known set, `block.timestamp <= now`, and `now - block.timestamp <= window`.

Steps:

- [ ] **5.1 RED: peak + freshness behavior.** Create `crates/digstore-prover/src/mock_chain.rs`:
```rust
use std::collections::HashMap;
use digstore_core::{Bytes32, ChiaBlockRef};
use crate::chain::ChainSource;
use crate::error::{ProverError, Result};

#[cfg(test)]
mod tests {
    use super::*;

    fn block(h: u32, ts: u64, tag: u8) -> ChiaBlockRef {
        ChiaBlockRef { header_hash: Bytes32([tag; 32]), height: h, timestamp: ts }
    }

    #[test]
    fn get_peak_returns_configured_peak() {
        let peak = block(100, 1_000, 0x01);
        let src = MockChainSource::new(vec![peak.clone()], 1_000);
        assert_eq!(src.get_peak().unwrap(), peak);
    }

    #[test]
    fn verify_block_accepts_known_fresh_block() {
        let b = block(100, 990, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000);
        assert!(src.verify_block(&b, 60).is_ok());
    }

    #[test]
    fn verify_block_rejects_stale_block() {
        let b = block(100, 900, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000);
        assert!(matches!(src.verify_block(&b, 60).unwrap_err(), ProverError::BlockTooOld { .. }));
    }

    #[test]
    fn verify_block_rejects_unknown_block() {
        let src = MockChainSource::new(vec![block(100, 990, 0x01)], 1_000);
        let unknown = block(101, 995, 0x02);
        assert!(matches!(src.verify_block(&unknown, 60).unwrap_err(), ProverError::BlockNotOnChain(_)));
    }

    #[test]
    fn verify_block_rejects_future_block() {
        let b = block(100, 1_050, 0x01);
        let src = MockChainSource::new(vec![b.clone()], 1_000);
        assert!(matches!(src.verify_block(&b, 60).unwrap_err(), ProverError::BlockInFuture(_, _)));
    }
}
```
Run: `cargo test -p digstore-prover get_peak_returns_configured_peak`
Expected FAIL (RED):
```
error[E0433]: failed to resolve: use of undeclared type `MockChainSource`
```

- [ ] **5.2 GREEN: implement MockChainSource.** Insert above the test block:
```rust
/// Deterministic in-memory [`ChainSource`] for tests. Holds a fixed set of
/// known blocks (keyed by header hash) and a fixed `now`.
#[derive(Debug, Clone)]
pub struct MockChainSource {
    blocks: HashMap<[u8; 32], ChiaBlockRef>,
    peak: ChiaBlockRef,
    now: u64,
}

impl MockChainSource {
    /// `blocks[0]` is treated as the peak. `now` is the fixed wall clock.
    pub fn new(blocks: Vec<ChiaBlockRef>, now: u64) -> Self {
        assert!(!blocks.is_empty(), "MockChainSource needs at least one block");
        let peak = blocks[0].clone();
        let map = blocks.into_iter().map(|b| (b.header_hash.0, b)).collect();
        Self { blocks: map, peak, now }
    }

    /// Override the fixed "now" used for freshness checks.
    pub fn with_now(mut self, now: u64) -> Self {
        self.now = now;
        self
    }
}

impl ChainSource for MockChainSource {
    fn get_peak(&self) -> Result<ChiaBlockRef> {
        Ok(self.peak.clone())
    }

    fn verify_block(&self, block: &ChiaBlockRef, freshness_window_secs: u64) -> Result<()> {
        match self.blocks.get(&block.header_hash.0) {
            Some(b) if b == block => {}
            _ => return Err(ProverError::BlockNotOnChain(block.header_hash.to_hex())),
        }
        if block.timestamp > self.now {
            return Err(ProverError::BlockInFuture(block.timestamp, self.now));
        }
        if self.now - block.timestamp > freshness_window_secs {
            return Err(ProverError::BlockTooOld {
                block_ts: block.timestamp,
                now: self.now,
                window: freshness_window_secs,
            });
        }
        Ok(())
    }
}
```
Run: `cargo test -p digstore-prover -- mock_chain::tests`
Expected PASS: `test result: ok. 5 passed`.

- [ ] **5.3 Commit.** `git add crates/digstore-prover/src/mock_chain.rs` then `git commit -m "feat(prover): deterministic MockChainSource with freshness window (§13.8)"`.

---

## Task 6 — MockProver / MockVerifier prove→verify round-trip (§13.1-13.4, 13.7-13.8)

**Files:**
- Create: `crates/digstore-prover/src/mock.rs`
- Create: `crates/digstore-prover/tests/mock_roundtrip.rs`

> The mock proof is a SHA-256 commitment chain (deviation #3, default backend):
> `proof_bytes = SHA-256( b"digstore-mock-proof-v1" || program_hash || public_input || public_output )`.
> Because `public_output` already binds the roothash (Task 2), the proof transitively commits to the root. `node_signature` is a real BLS G2 sig over `signing_message(proof_bytes, public_input)` (§13.7), so attribution is genuinely checked even in the mock.

Steps:

- [ ] **6.1 RED: round-trip integration test.** Create `crates/digstore-prover/tests/mock_roundtrip.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs};
use digstore_crypto::bls;

fn block() -> ChiaBlockRef {
    ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 }
}

#[test]
fn mock_prove_verify_round_trip() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = block();

    let program_hash = Bytes32([0xAAu8; 32]);
    let roothash = Bytes32([0xBBu8; 32]);
    let nonce = [0x33u8; 32];
    let public_input = build_public_input(&nonce, &block);

    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash,
        chunk_ciphertext: vec![vec![0xDEu8, 0xAD], vec![0xBE, 0xEF]],
    };

    let prover = MockProver::new(sk, pk.clone(), block.clone());
    let proof = prover.prove(program_hash, &public_input, &serving).unwrap();

    assert_eq!(proof.node_pubkey.0, pk.to_bytes());
    assert_eq!(proof.chia_block, block);
    assert_eq!(proof.program_hash, program_hash);
    assert_eq!(proof.public_output, serving.compute_public_output());

    let chain = MockChainSource::new(vec![block.clone()], 1_000_030);
    MockVerifier::default()
        .verify(&proof, program_hash, &[roothash], &chain)
        .expect("genuine mock proof must verify");
}
```
Run: `cargo test -p digstore-prover --test mock_roundtrip`
Expected FAIL (RED):
```
error[E0432]: unresolved import `digstore_prover::MockProver`
```

- [ ] **6.2 GREEN: implement MockProver/MockVerifier.** Create `crates/digstore-prover/src/mock.rs`:
```rust
use digstore_core::{Bytes32, Bytes48, Bytes96, ExecutionProof, ChiaBlockRef};
use digstore_crypto::{sha256, bls};
use crate::serving_inputs::ServingInputs;
use crate::prover::{Prover, Verifier};
use crate::chain::ChainSource;
use crate::commitment::{parse_public_input, signing_message};
use crate::error::{ProverError, Result};

const MOCK_DOMAIN: &[u8] = b"digstore-mock-proof-v1";

/// Default freshness window for chain anchoring (10 minutes).
pub const DEFAULT_FRESHNESS_WINDOW_SECS: u64 = 600;

/// The mock commitment-chain proof bytes (deviation #3): a SHA-256 over the
/// full statement. Recomputed identically by the verifier.
fn mock_proof_bytes(program_hash: &Bytes32, public_input: &[u8], public_output: &Bytes32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MOCK_DOMAIN);
    buf.extend_from_slice(&program_hash.0);
    buf.extend_from_slice(public_input);
    buf.extend_from_slice(&public_output.0);
    sha256(&buf).to_vec()
}

/// Default deterministic prover. Emits a commitment-chain proof and a genuine
/// node BLS signature over `(proof || public_input)`.
pub struct MockProver {
    secret: bls::SecretKey,
    pubkey: bls::PublicKey,
    chia_block: ChiaBlockRef,
}

impl MockProver {
    pub fn new(secret: bls::SecretKey, pubkey: bls::PublicKey, chia_block: ChiaBlockRef) -> Self {
        Self { secret, pubkey, chia_block }
    }
}

impl Prover for MockProver {
    fn prove(
        &self,
        program_hash: Bytes32,
        public_input: &[u8],
        serving_inputs: &ServingInputs,
    ) -> Result<ExecutionProof> {
        let (_nonce, block) = parse_public_input(public_input)?;
        if block != self.chia_block {
            return Err(ProverError::Backend(
                "public_input block does not match prover's bound chia_block".into(),
            ));
        }
        let public_output = serving_inputs.compute_public_output();
        let proof = mock_proof_bytes(&program_hash, public_input, &public_output);
        let msg = signing_message(&proof, public_input);
        let sig = bls::sign(&self.secret, &msg);
        Ok(ExecutionProof {
            program_hash,
            public_input: public_input.to_vec(),
            public_output,
            proof,
            chia_block: self.chia_block.clone(),
            node_pubkey: Bytes48(self.pubkey.to_bytes()),
            node_signature: Bytes96(sig.to_bytes()),
        })
    }
}

/// Verifier for [`MockProver`] proofs.
#[derive(Debug, Default)]
pub struct MockVerifier;

impl Verifier for MockVerifier {
    fn verify(
        &self,
        proof: &ExecutionProof,
        expected_program_hash: Bytes32,
        trusted_roots: &[Bytes32],
        chain: &dyn ChainSource,
    ) -> Result<()> {
        // 1. program-hash match (§13.1, §13.4)
        if proof.program_hash != expected_program_hash {
            return Err(ProverError::ProgramHashMismatch {
                expected: expected_program_hash.to_hex(),
                actual: proof.program_hash.to_hex(),
            });
        }
        // 2. public_input parse; bound block must equal proof.chia_block (§13.8)
        let (_nonce, pi_block) = parse_public_input(&proof.public_input)?;
        if pi_block != proof.chia_block {
            return Err(ProverError::Codec("public_input block != proof.chia_block".into()));
        }
        // 3. recompute the mock commitment chain (deviation #3). Tampering
        //    public_output OR proof bytes surfaces here.
        let expected_proof = mock_proof_bytes(&proof.program_hash, &proof.public_input, &proof.public_output);
        if expected_proof != proof.proof {
            return Err(ProverError::ZkProofInvalid("mock commitment chain mismatch".into()));
        }
        // 4. node attribution: BLS over (proof || public_input) (§13.7)
        let msg = signing_message(&proof.proof, &proof.public_input);
        if !bls::verify(&proof.node_pubkey.0, &msg, &proof.node_signature.0) {
            return Err(ProverError::NodeSignatureInvalid);
        }
        // 5. require a non-empty trusted-root set; root *binding* is enforced
        //    in Verifier::verify_response against the asserted ProofResponse root.
        if trusted_roots.is_empty() {
            return Err(ProverError::UntrustedRoot("no trusted roots provided".into()));
        }
        // 6. chain freshness (§13.8)
        chain.verify_block(&proof.chia_block, DEFAULT_FRESHNESS_WINDOW_SECS)?;
        Ok(())
    }
}
```
Run: `cargo test -p digstore-prover --test mock_roundtrip`
Expected PASS: `test result: ok. 1 passed`.

- [ ] **6.3 Commit.** `git add crates/digstore-prover/src/mock.rs crates/digstore-prover/tests/mock_roundtrip.rs` then `git commit -m "feat(prover): MockProver/MockVerifier commitment-chain round-trip (§13.1-13.4, 13.7-13.8)"`.

---

## Task 7 — Roothash binding enforcement (§13.4)

**Files:**
- Create: `crates/digstore-prover/tests/roothash_binding.rs`

> **Blocker fix:** `verify_response` recomputes `SHA-256(asserted_root || returned_bytes)` and compares it to the proof's committed `public_output`. This proves the proof is *bound* to the asserted root, not merely that the asserted root is in the trusted set. The decisive test pairs a genuine proof (bound to root B) with a *different but trusted* root C and asserts rejection — impossible to detect under the old plan.

Steps:

- [ ] **7.1 RED: bound-root accepted; different-trusted-root rejected; untrusted rejected.** Create `crates/digstore-prover/tests/roothash_binding.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef, ProofResponse};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

fn make_response(roothash: Bytes32) -> (ProofResponse, Bytes32, ChiaBlockRef, Vec<u8>) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let program_hash = Bytes32([0xAAu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs {
        retrieval_key: Bytes32([1u8; 32]),
        roothash,
        chunk_ciphertext: vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]],
    };
    let returned = serving.output_bytes();
    let proof = MockProver::new(sk, pk, block.clone())
        .prove(program_hash, &public_input, &serving).unwrap();
    (ProofResponse { proof, roothash }, program_hash, block, returned)
}

#[test]
fn bound_roothash_is_accepted() {
    let root = Bytes32([0xBBu8; 32]);
    let (resp, ph, block, returned) = make_response(root);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    MockVerifier::default()
        .verify_response(&resp, ph, &[root], &returned, &chain)
        .expect("a proof bound to the asserted trusted root must verify");
}

#[test]
fn untrusted_roothash_is_rejected() {
    let root = Bytes32([0xBBu8; 32]);
    let (resp, ph, block, returned) = make_response(root);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    let other = Bytes32([0xCCu8; 32]);
    let err = MockVerifier::default()
        .verify_response(&resp, ph, &[other], &returned, &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::UntrustedRoot(_)));
}

#[test]
fn proof_bound_to_root_b_rejected_when_response_asserts_different_trusted_root_c() {
    // Genuine proof bound to root B; attacker swaps the response root to C,
    // and the verifier happens to trust BOTH B and C. The binding check must
    // still reject because public_output committed B, not C.
    let root_b = Bytes32([0xBBu8; 32]);
    let root_c = Bytes32([0xCCu8; 32]);
    let (mut resp, ph, block, returned) = make_response(root_b);
    resp.roothash = root_c; // forge the asserted root
    let chain = MockChainSource::new(vec![block], 1_000_030);
    let err = MockVerifier::default()
        .verify_response(&resp, ph, &[root_b, root_c], &returned, &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::RootBindingMismatch { .. }));
}
```
Run: `cargo test -p digstore-prover --test roothash_binding`
Expected: `bound_roothash_is_accepted` and `untrusted_roothash_is_rejected` PASS immediately (logic shipped in Task 4.1 `verify_response`); `proof_bound_to_root_b_rejected_...` is the decisive new assertion. All three should PASS because the binding logic is already in `verify_response`. If `proof_bound_to_root_b_rejected_...` FAILS, the binding is not enforced — fix `verify_response` in `prover.rs`. Expected on success: `test result: ok. 3 passed`.

- [ ] **7.2 Commit.** `git add crates/digstore-prover/tests/roothash_binding.rs` then `git commit -m "test(prover): roothash binding enforced in verify_response (§13.4)"`.

---

## Task 8 — Nonce binding rejection (§13.5)

**Files:**
- Create: `crates/digstore-prover/tests/nonce_binding.rs`

> `verify_with_nonce` (defined in Task 4.1) checks the proof's bound nonce equals the request's expected nonce. This task is the §13.5 behavior test for that already-defined default method.

Steps:

- [ ] **8.1 RED: nonce mismatch rejected, match accepted.** Create `crates/digstore-prover/tests/nonce_binding.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef, ExecutionProof};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

fn make_proof(nonce: [u8; 32]) -> (ExecutionProof, Bytes32, Bytes32, ChiaBlockRef) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let program_hash = Bytes32([0xAAu8; 32]);
    let roothash = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&nonce, &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash, chunk_ciphertext: vec![vec![9, 9, 9]] };
    let proof = MockProver::new(sk, pk, block.clone()).prove(program_hash, &public_input, &serving).unwrap();
    (proof, program_hash, roothash, block)
}

#[test]
fn proof_for_nonce_a_rejected_against_request_nonce_b() {
    let (proof, ph, root, block) = make_proof([0xA1u8; 32]);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    let err = MockVerifier::default()
        .verify_with_nonce(&proof, &[0xB2u8; 32], ph, &[root], &chain)
        .unwrap_err();
    assert!(matches!(err, ProverError::NonceMismatch));
}

#[test]
fn proof_for_nonce_a_accepted_against_request_nonce_a() {
    let nonce_a = [0xA1u8; 32];
    let (proof, ph, root, block) = make_proof(nonce_a);
    let chain = MockChainSource::new(vec![block], 1_000_030);
    MockVerifier::default()
        .verify_with_nonce(&proof, &nonce_a, ph, &[root], &chain)
        .expect("matching nonce must verify");
}
```
Run: `cargo test -p digstore-prover --test nonce_binding`
Expected: both PASS (the `verify_with_nonce` default method shipped in Task 4.1). If `proof_for_nonce_a_rejected_against_request_nonce_b` FAILS, fix `verify_with_nonce` in `prover.rs`. Expected on success: `test result: ok. 2 passed`.

- [ ] **8.2 Commit.** `git add crates/digstore-prover/tests/nonce_binding.rs` then `git commit -m "test(prover): nonce binding rejection in verify_with_nonce (§13.5)"`.

---

## Task 9 — Chain freshness window accept/reject (§13.8) [GUARD]

**Files:**
- Create: `crates/digstore-prover/tests/chain_freshness.rs`

> **GUARD / regression task.** The freshness behavior was implemented in Tasks 5 (`MockChainSource::verify_block`) and 6 (`MockVerifier::verify` step 6). These tests are NOT a TDD red→green cycle — they are expected to PASS on first run and carry no impl step. They lock the §13.8 behavior against future regressions.

Steps:

- [ ] **9.1 GUARD: block inside/outside window + unknown block.** Create `crates/digstore-prover/tests/chain_freshness.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef, ExecutionProof};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

fn proof_bound_to(block: &ChiaBlockRef) -> (ExecutionProof, Bytes32, Bytes32) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1]] };
    let proof = MockProver::new(sk, pk, block.clone()).prove(ph, &public_input, &serving).unwrap();
    (proof, ph, root)
}

#[test]
fn block_inside_freshness_window_accepted() {
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let (proof, ph, root) = proof_bound_to(&block);
    let chain = MockChainSource::new(vec![block], 1_000_300); // 300s < 600s window
    MockVerifier::default().verify(&proof, ph, &[root], &chain).expect("fresh block accepted");
}

#[test]
fn block_outside_freshness_window_rejected() {
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let (proof, ph, root) = proof_bound_to(&block);
    let chain = MockChainSource::new(vec![block], 1_000_700); // 700s > 600s window
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::BlockTooOld { .. }));
}

#[test]
fn block_unknown_to_chain_rejected() {
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let (proof, ph, root) = proof_bound_to(&block);
    let other = ChiaBlockRef { header_hash: Bytes32([0x66u8; 32]), height: 43, timestamp: 1_000_010 };
    let chain = MockChainSource::new(vec![other], 1_000_300);
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::BlockNotOnChain(_)));
}
```
Run: `cargo test -p digstore-prover --test chain_freshness`
Expected PASS (GUARD, no impl step): `test result: ok. 3 passed`. If any fails, the regression is in `MockVerifier::verify` step 6 or `MockChainSource::verify_block` — fix there.

- [ ] **9.2 Commit.** `git add crates/digstore-prover/tests/chain_freshness.rs` then `git commit -m "test(prover): GUARD chain freshness window accept/reject (§13.8)"`.

---

## Task 10 — Node attribution accept/reject (§13.7) [GUARD]

**Files:**
- Create: `crates/digstore-prover/tests/node_attribution.rs`

> **GUARD / regression task.** Node BLS attribution was implemented in Task 6 (`MockVerifier::verify` step 4). Expected to PASS on first run; no impl step.

Steps:

- [ ] **10.1 GUARD: genuine accepted; tampered sig rejected; wrong pubkey rejected.** Create `crates/digstore-prover/tests/node_attribution.rs`:
```rust
use digstore_core::{Bytes32, Bytes48, ChiaBlockRef, ExecutionProof};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

fn fresh_proof() -> (ExecutionProof, Bytes32, Bytes32, ChiaBlockRef) {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![5, 5]] };
    let proof = MockProver::new(sk, pk, block.clone()).prove(ph, &public_input, &serving).unwrap();
    (proof, ph, root, block)
}

#[test]
fn genuine_signature_attributes_to_node() {
    let (proof, ph, root, block) = fresh_proof();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap();
}

#[test]
fn tampered_signature_is_rejected() {
    let (mut proof, ph, root, block) = fresh_proof();
    proof.node_signature.0[0] ^= 0xFF;
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::NodeSignatureInvalid));
}

#[test]
fn wrong_pubkey_is_rejected() {
    let (mut proof, ph, root, block) = fresh_proof();
    let other_pk = bls::SecretKey::from_seed(&[99u8; 32]).public_key();
    proof.node_pubkey = Bytes48(other_pk.to_bytes());
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::NodeSignatureInvalid));
}
```
Run: `cargo test -p digstore-prover --test node_attribution`
Expected PASS (GUARD, no impl step): `test result: ok. 3 passed`. If `tampered_signature_is_rejected` fails, the BLS verify wiring is wrong — fix step 4 in `mock.rs`.

- [ ] **10.2 Commit.** `git add crates/digstore-prover/tests/node_attribution.rs` then `git commit -m "test(prover): GUARD node BLS attribution accept/reject (§13.7)"`.

---

## Task 11 — program_hash and public_output mismatch rejection (§13.4) [GUARD]

**Files:**
- Create: `crates/digstore-prover/tests/program_hash.rs`
- Create: `crates/digstore-prover/tests/public_output.rs`

> **GUARD / regression task.** Program-hash check (step 1) and commitment-chain recompute (step 3) shipped in Task 6. Expected to PASS on first run; no impl step.

Steps:

- [ ] **11.1 GUARD: program_hash mismatch rejected.** Create `crates/digstore-prover/tests/program_hash.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

#[test]
fn program_hash_mismatch_is_rejected() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let proven = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1]] };
    let proof = MockProver::new(sk, pk, block.clone()).prove(proven, &public_input, &serving).unwrap();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default().verify(&proof, Bytes32([0xCCu8; 32]), &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::ProgramHashMismatch { .. }));
}
```
Run: `cargo test -p digstore-prover --test program_hash`
Expected PASS (GUARD): `test result: ok. 1 passed`. If FAIL, fix step 1 in `mock.rs`.

- [ ] **11.2 GUARD: tampered public_output rejected.** Create `crates/digstore-prover/tests/public_output.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

#[test]
fn tampered_public_output_is_rejected() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let public_input = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1, 2, 3]] };
    let mut proof = MockProver::new(sk, pk, block.clone()).prove(ph, &public_input, &serving).unwrap();
    proof.public_output = Bytes32([0xEEu8; 32]); // tamper the committed output
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = MockVerifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    // In the mock, public_output feeds the commitment chain, so tampering it breaks
    // the recompute and surfaces as ZkProofInvalid. (Risc0Verifier surfaces the same
    // tamper as PublicOutputMismatch via the journal — both are valid §13.4 rejections.)
    assert!(matches!(err, ProverError::ZkProofInvalid(_)));
}

#[test]
fn output_bytes_differ_changes_commitment() {
    let a = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: Bytes32([2u8; 32]), chunk_ciphertext: vec![vec![1, 2, 3]] };
    let b = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: Bytes32([2u8; 32]), chunk_ciphertext: vec![vec![1, 2, 4]] };
    assert_ne!(a.compute_public_output(), b.compute_public_output());
}
```
Run: `cargo test -p digstore-prover --test public_output`
Expected PASS (GUARD): `test result: ok. 2 passed`. If FAIL, fix step 3 in `mock.rs`.

- [ ] **11.3 Commit.** `git add crates/digstore-prover/tests/program_hash.rs crates/digstore-prover/tests/public_output.rs` then `git commit -m "test(prover): GUARD program_hash + public_output mismatch rejection (§13.4)"`.

---

## Task 12 — CoinsetChainSource DTOs + tx-block walk-down parse (§13.8 live chain)

**Files:**
- Create: `crates/digstore-prover/tests/fixtures/get_blockchain_state.json`
- Create: `crates/digstore-prover/tests/fixtures/get_block_record_by_height_notx.json`
- Create: `crates/digstore-prover/tests/fixtures/get_block_record_prev_tx.json`
- Create: `crates/digstore-prover/src/coinset.rs`
- Create: `crates/digstore-prover/tests/coinset_parse.rs`

> coinset.org mirrors the Chia full-node RPC. `POST /get_blockchain_state` → `{"blockchain_state":{"peak":{...}},"success":true}`; the peak is a `BlockRecord` with `header_hash` (0x-hex), `height`, and `timestamp` (present only on **transaction blocks**; `null` otherwise). `POST /get_block_record_by_height` → `{"block_record":{...},"success":true}`.
>
> **Major fix (tx-block walk-down):** a `BlockRecord` with `timestamp == None` is NO LONGER an error. `record_to_ref_with_resolver` keeps the header hash/height and, when the timestamp is missing, follows `prev_transaction_block_height` (resolved by a `BlockRecordResolver` closure) until a timestamped block is found, using that block's timestamp for the ref. This is unit-tested with a non-transaction-block fixture and a prev-tx fixture, so the live §13.8 path's hard part is covered without a network call.

Steps:

- [ ] **12.1 Create the blockchain-state fixture (transaction block).** Create `crates/digstore-prover/tests/fixtures/get_blockchain_state.json`:
```json
{
  "blockchain_state": {
    "difficulty": 14680064,
    "peak": {
      "header_hash": "0xb5f2a7c1d3e4f5061728394a5b6c7d8e9f00112233445566778899aabbccddee",
      "height": 5421337,
      "weight": "123456789",
      "total_iters": "9876543210",
      "signage_point_index": 12,
      "farmer_puzzle_hash": "0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
      "timestamp": 1717804800,
      "prev_transaction_block_height": 5421330,
      "prev_transaction_block_hash": "0xaa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899"
    },
    "space": "39000000000000000000000",
    "sub_slot_iters": 578813952,
    "sync": { "sync_mode": false, "synced": true, "sync_tip_height": 0, "sync_progress_height": 0 }
  },
  "success": true
}
```

- [ ] **12.2 Create the non-transaction-block fixture (no timestamp).** Create `crates/digstore-prover/tests/fixtures/get_block_record_by_height_notx.json`:
```json
{
  "block_record": {
    "header_hash": "0xc6e3b8d2e4f5061728394a5b6c7d8e9f00112233445566778899aabbccddeeff",
    "height": 5421335,
    "timestamp": null,
    "prev_transaction_block_height": 5421330,
    "prev_transaction_block_hash": "0xaa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899"
  },
  "success": true
}
```

- [ ] **12.3 Create the previous-transaction-block fixture (walk-down target).** Create `crates/digstore-prover/tests/fixtures/get_block_record_prev_tx.json`:
```json
{
  "block_record": {
    "header_hash": "0xaa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899",
    "height": 5421330,
    "timestamp": 1717804620,
    "prev_transaction_block_height": 5421325,
    "prev_transaction_block_hash": "0xbb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899aabb"
  },
  "success": true
}
```

- [ ] **12.4 RED: parse fixtures (peak, tx walk-down, failure).** Create `crates/digstore-prover/tests/coinset_parse.rs`:
```rust
use digstore_prover::coinset::{
    parse_blockchain_state, parse_block_record_resolved,
    BlockchainStateResponse, BlockRecord, BlockRecordResponse,
};

const STATE_JSON: &str = include_str!("fixtures/get_blockchain_state.json");
const NOTX_JSON: &str = include_str!("fixtures/get_block_record_by_height_notx.json");
const PREVTX_JSON: &str = include_str!("fixtures/get_block_record_prev_tx.json");

#[test]
fn parses_blockchain_state_peak_into_chia_block_ref() {
    let resp: BlockchainStateResponse = serde_json::from_str(STATE_JSON).unwrap();
    let block = parse_blockchain_state(resp).unwrap();
    assert_eq!(block.height, 5_421_337);
    assert_eq!(block.timestamp, 1_717_804_800);
    assert_eq!(block.header_hash.0[0], 0xB5);
    assert_eq!(block.header_hash.0[31], 0xEE);
}

#[test]
fn non_transaction_block_walks_down_to_prev_tx_for_timestamp() {
    // The fetched record (height 5421335) has timestamp == null; the resolver
    // returns the prev transaction block (5421330) which carries a timestamp.
    let notx: BlockRecordResponse = serde_json::from_str(NOTX_JSON).unwrap();
    let prevtx: BlockRecordResponse = serde_json::from_str(PREVTX_JSON).unwrap();
    let prev_record: BlockRecord = prevtx.block_record.clone().unwrap();

    let block = parse_block_record_resolved(notx, &mut |height| {
        assert_eq!(height, 5_421_330); // walk-down follows prev_transaction_block_height
        Ok(prev_record.clone())
    })
    .unwrap();

    // Header hash/height stay the originally-requested record's; timestamp is
    // inherited from the nearest previous transaction block.
    assert_eq!(block.height, 5_421_335);
    assert_eq!(block.header_hash.0[0], 0xC6);
    assert_eq!(block.timestamp, 1_717_804_620);
}

#[test]
fn transaction_block_uses_its_own_timestamp_without_walking() {
    let prevtx: BlockRecordResponse = serde_json::from_str(PREVTX_JSON).unwrap();
    // Resolver must NOT be called for a block that already has a timestamp.
    let block = parse_block_record_resolved(prevtx, &mut |_h| {
        panic!("resolver must not be called for a transaction block");
    })
    .unwrap();
    assert_eq!(block.height, 5_421_330);
    assert_eq!(block.timestamp, 1_717_804_620);
}

#[test]
fn rejects_unsuccessful_response() {
    let bad = r#"{"success": false, "error": "boom"}"#;
    let resp: BlockchainStateResponse = serde_json::from_str(bad).unwrap();
    assert!(parse_blockchain_state(resp).is_err());
}
```
Run: `cargo test -p digstore-prover --test coinset_parse`
Expected FAIL (RED):
```
error[E0432]: unresolved import `digstore_prover::coinset::parse_block_record_resolved`
```

- [ ] **12.5 GREEN: implement coinset DTOs + walk-down parsers + live source.** Create `crates/digstore-prover/src/coinset.rs`:
```rust
use serde::Deserialize;
use digstore_core::{Bytes32, ChiaBlockRef};
use crate::chain::ChainSource;
use crate::error::{ProverError, Result};

/// A Chia `BlockRecord` (subset we need). `timestamp` is `None` on
/// non-transaction blocks; such blocks point at a previous transaction block.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRecord {
    pub header_hash: String,
    pub height: u32,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub prev_transaction_block_height: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainState {
    pub peak: Option<BlockRecord>,
}

/// Response of `POST /get_blockchain_state`.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainStateResponse {
    pub success: bool,
    #[serde(default)]
    pub blockchain_state: Option<BlockchainState>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response of `POST /get_block_record_by_height` and `/get_block_record`.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRecordResponse {
    pub success: bool,
    #[serde(default)]
    pub block_record: Option<BlockRecord>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A resolver that fetches a `BlockRecord` at a given height (used to walk down
/// to the previous transaction block when a record lacks a timestamp).
pub type BlockRecordResolver<'a> = dyn FnMut(u32) -> Result<BlockRecord> + 'a;

/// Decode a Chia `0x`-prefixed 32-byte hex string into [`Bytes32`].
fn parse_header_hash(s: &str) -> Result<Bytes32> {
    let hex = s.strip_prefix("0x").unwrap_or(s);
    Bytes32::from_hex(hex).map_err(|e| ProverError::ChainRpc(format!("bad header hash hex: {e:?}")))
}

/// Resolve a `BlockRecord` to a [`ChiaBlockRef`]. The header hash and height
/// come from `record`; if `record` has no timestamp it is a non-transaction
/// block, so we walk `prev_transaction_block_height` via `resolve` until a
/// timestamped block is found, and inherit that timestamp.
fn record_to_ref(record: BlockRecord, resolve: &mut BlockRecordResolver<'_>) -> Result<ChiaBlockRef> {
    let header_hash = parse_header_hash(&record.header_hash)?;
    let height = record.height;
    let timestamp = resolve_timestamp(&record, resolve)?;
    Ok(ChiaBlockRef { header_hash, height, timestamp })
}

/// Walk down to the nearest previous transaction block to obtain a timestamp.
fn resolve_timestamp(record: &BlockRecord, resolve: &mut BlockRecordResolver<'_>) -> Result<u64> {
    if let Some(ts) = record.timestamp {
        return Ok(ts);
    }
    let mut next = record.prev_transaction_block_height;
    // Bound the walk to avoid pathological loops.
    for _ in 0..256 {
        let h = next.ok_or_else(|| ProverError::ChainRpc(format!(
            "block at height {} has no timestamp and no prev_transaction_block_height",
            record.height
        )))?;
        let prev = resolve(h)?;
        if let Some(ts) = prev.timestamp {
            return Ok(ts);
        }
        next = prev.prev_transaction_block_height;
    }
    Err(ProverError::ChainRpc(format!(
        "could not resolve a transaction block timestamp for height {}",
        record.height
    )))
}

/// Extract the peak [`ChiaBlockRef`] from a blockchain-state response. The peak
/// is a transaction block (carries its own timestamp), so no resolver is used.
pub fn parse_blockchain_state(resp: BlockchainStateResponse) -> Result<ChiaBlockRef> {
    if !resp.success {
        return Err(ProverError::ChainRpc(resp.error.unwrap_or_else(|| "get_blockchain_state failed".into())));
    }
    let peak = resp.blockchain_state.and_then(|s| s.peak)
        .ok_or_else(|| ProverError::ChainRpc("no peak in blockchain_state".into()))?;
    record_to_ref(peak, &mut |_h| Err(ProverError::ChainRpc("peak unexpectedly lacked a timestamp".into())))
}

/// Extract a [`ChiaBlockRef`] from a block-record response, walking down to the
/// previous transaction block (via `resolve`) for the timestamp when needed.
pub fn parse_block_record_resolved(
    resp: BlockRecordResponse,
    resolve: &mut BlockRecordResolver<'_>,
) -> Result<ChiaBlockRef> {
    if !resp.success {
        return Err(ProverError::ChainRpc(resp.error.unwrap_or_else(|| "get_block_record failed".into())));
    }
    let rec = resp.block_record.ok_or_else(|| ProverError::ChainRpc("no block_record in response".into()))?;
    record_to_ref(rec, resolve)
}

/// Live [`ChainSource`] backed by the coinset.org Chia RPC mirror.
#[derive(Debug, Clone)]
pub struct CoinsetChainSource {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl Default for CoinsetChainSource {
    fn default() -> Self {
        Self::new("https://api.coinset.org")
    }
}

impl CoinsetChainSource {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), client: reqwest::blocking::Client::new() }
    }

    fn post_json(&self, endpoint: &str, body: String) -> Result<String> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint);
        let resp = self.client.post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .map_err(|e| ProverError::ChainRpc(format!("{endpoint} send: {e}")))?;
        resp.text().map_err(|e| ProverError::ChainRpc(format!("{endpoint} body: {e}")))
    }

    fn fetch_record(&self, height: u32) -> Result<BlockRecord> {
        let body = self.post_json("get_block_record_by_height", format!("{{\"height\": {height}}}"))?;
        let resp: BlockRecordResponse = serde_json::from_str(&body)
            .map_err(|e| ProverError::ChainRpc(format!("parse record: {e}")))?;
        if !resp.success {
            return Err(ProverError::ChainRpc(resp.error.unwrap_or_else(|| "get_block_record_by_height failed".into())));
        }
        resp.block_record.ok_or_else(|| ProverError::ChainRpc("no block_record in response".into()))
    }
}

impl ChainSource for CoinsetChainSource {
    fn get_peak(&self) -> Result<ChiaBlockRef> {
        let body = self.post_json("get_blockchain_state", "{}".into())?;
        let resp: BlockchainStateResponse = serde_json::from_str(&body)
            .map_err(|e| ProverError::ChainRpc(format!("parse state: {e}")))?;
        parse_blockchain_state(resp)
    }

    fn verify_block(&self, block: &ChiaBlockRef, freshness_window_secs: u64) -> Result<()> {
        // Confirm the block is on-chain: fetch the record at this height and
        // compare header hashes (walking down for a timestamp if needed).
        let body = self.post_json("get_block_record_by_height", format!("{{\"height\": {}}}", block.height))?;
        let resp: BlockRecordResponse = serde_json::from_str(&body)
            .map_err(|e| ProverError::ChainRpc(format!("parse record: {e}")))?;
        let on_chain = parse_block_record_resolved(resp, &mut |h| self.fetch_record(h))?;
        if on_chain.header_hash != block.header_hash {
            return Err(ProverError::BlockNotOnChain(block.header_hash.to_hex()));
        }
        // Freshness against the current peak's wall-clock timestamp.
        let now = self.get_peak()?.timestamp;
        if block.timestamp > now {
            return Err(ProverError::BlockInFuture(block.timestamp, now));
        }
        if now - block.timestamp > freshness_window_secs {
            return Err(ProverError::BlockTooOld { block_ts: block.timestamp, now, window: freshness_window_secs });
        }
        Ok(())
    }
}
```
Run: `cargo test -p digstore-prover --test coinset_parse`
Expected PASS: `test result: ok. 4 passed`.

- [ ] **12.6 Commit.** `git add crates/digstore-prover/src/coinset.rs crates/digstore-prover/tests/coinset_parse.rs crates/digstore-prover/tests/fixtures/` then `git commit -m "feat(prover): CoinsetChainSource with tx-block walk-down + fixture parse (§13.8)"`.

---

## Task 13 — Hardware-attestation alternative (§13.6) behind the same trait

**Files:**
- Create: `crates/digstore-prover/src/hardware.rs`
- Create: `crates/digstore-prover/tests/hardware_attest.rs`

> §13.6: where per-request ZK is prohibitive, a TEE/HSM attestation vouches for the same statement. `HardwareAttestProver`/`HardwareVerifier` (feature `hardware-attest`) implement the SAME `Prover`/`Verifier` traits. The attestation lives in `ExecutionProof.proof` (96 bytes): the enclave signs `SHA-256(b"digstore-tee-attest-v1" || program_hash || public_input || public_output)` with a BLS key. The verifier recomputes that digest and BLS-verifies under a configured trusted enclave key. Node attribution (`node_signature`) still applies.

Steps:

- [ ] **13.1 RED: hardware attest prove→verify + tamper/wrong-key reject.** Create `crates/digstore-prover/tests/hardware_attest.rs`:
```rust
#![cfg(feature = "hardware-attest")]
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::build_public_input;
use digstore_prover::hardware::{HardwareAttestProver, HardwareVerifier};
use digstore_prover::{Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

fn fixture() -> (bls::SecretKey, bls::PublicKey, bls::SecretKey, bls::PublicKey, ChiaBlockRef, Bytes32, Bytes32, Vec<u8>, ServingInputs) {
    let node_sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let node_pk = node_sk.public_key();
    let enclave_sk = bls::SecretKey::from_seed(&[42u8; 32]);
    let enclave_pk = enclave_sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1, 2, 3]] };
    (node_sk, node_pk, enclave_sk, enclave_pk, block, ph, root, pi, serving)
}

#[test]
fn hardware_attest_round_trip() {
    let (node_sk, node_pk, enclave_sk, enclave_pk, block, ph, root, pi, serving) = fixture();
    let proof = HardwareAttestProver::new(node_sk, node_pk, enclave_sk, block.clone()).prove(ph, &pi, &serving).unwrap();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    HardwareVerifier::new(enclave_pk.to_bytes()).verify(&proof, ph, &[root], &chain).unwrap();
}

#[test]
fn tampered_attestation_rejected() {
    let (node_sk, node_pk, enclave_sk, enclave_pk, block, ph, root, pi, serving) = fixture();
    let mut proof = HardwareAttestProver::new(node_sk, node_pk, enclave_sk, block.clone()).prove(ph, &pi, &serving).unwrap();
    proof.proof[0] ^= 0xFF;
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = HardwareVerifier::new(enclave_pk.to_bytes()).verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::AttestationInvalid(_)));
}

#[test]
fn wrong_enclave_key_rejected() {
    let (node_sk, node_pk, enclave_sk, _enclave_pk, block, ph, root, pi, serving) = fixture();
    let proof = HardwareAttestProver::new(node_sk, node_pk, enclave_sk, block.clone()).prove(ph, &pi, &serving).unwrap();
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let wrong = bls::SecretKey::from_seed(&[7u8; 32]).public_key().to_bytes();
    let err = HardwareVerifier::new(wrong).verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::AttestationInvalid(_)));
}
```
Run: `cargo test -p digstore-prover --features hardware-attest --test hardware_attest`
Expected FAIL (RED):
```
error[E0432]: unresolved import `digstore_prover::hardware`
```

- [ ] **13.2 GREEN: implement hardware backend.** Create `crates/digstore-prover/src/hardware.rs`:
```rust
use digstore_core::{Bytes32, Bytes48, Bytes96, ExecutionProof, ChiaBlockRef};
use digstore_crypto::{sha256, bls};
use crate::serving_inputs::ServingInputs;
use crate::prover::{Prover, Verifier};
use crate::chain::ChainSource;
use crate::commitment::{parse_public_input, signing_message};
use crate::error::{ProverError, Result};
use crate::mock::DEFAULT_FRESHNESS_WINDOW_SECS;

const TEE_DOMAIN: &[u8] = b"digstore-tee-attest-v1";

/// Digest the enclave signs to vouch for the serving statement (§13.6).
fn attest_digest(program_hash: &Bytes32, public_input: &[u8], public_output: &Bytes32) -> [u8; 32] {
    let mut buf = Vec::new();
    buf.extend_from_slice(TEE_DOMAIN);
    buf.extend_from_slice(&program_hash.0);
    buf.extend_from_slice(public_input);
    buf.extend_from_slice(&public_output.0);
    sha256(&buf)
}

/// §13.6 alternative: a TEE/HSM-attested run replaces the ZK proof. The
/// attestation (enclave BLS signature, 96 bytes) is carried in `proof`.
pub struct HardwareAttestProver {
    node_secret: bls::SecretKey,
    node_pubkey: bls::PublicKey,
    enclave_secret: bls::SecretKey,
    chia_block: ChiaBlockRef,
}

impl HardwareAttestProver {
    pub fn new(node_secret: bls::SecretKey, node_pubkey: bls::PublicKey, enclave_secret: bls::SecretKey, chia_block: ChiaBlockRef) -> Self {
        Self { node_secret, node_pubkey, enclave_secret, chia_block }
    }
}

impl Prover for HardwareAttestProver {
    fn prove(&self, program_hash: Bytes32, public_input: &[u8], serving_inputs: &ServingInputs) -> Result<ExecutionProof> {
        let (_nonce, block) = parse_public_input(public_input)?;
        if block != self.chia_block {
            return Err(ProverError::Backend("public_input block mismatch".into()));
        }
        let public_output = serving_inputs.compute_public_output();
        let digest = attest_digest(&program_hash, public_input, &public_output);
        let attestation = bls::sign(&self.enclave_secret, &digest);
        let proof = attestation.to_bytes().to_vec(); // 96 bytes
        let msg = signing_message(&proof, public_input);
        let node_sig = bls::sign(&self.node_secret, &msg);
        Ok(ExecutionProof {
            program_hash,
            public_input: public_input.to_vec(),
            public_output,
            proof,
            chia_block: self.chia_block.clone(),
            node_pubkey: Bytes48(self.node_pubkey.to_bytes()),
            node_signature: Bytes96(node_sig.to_bytes()),
        })
    }
}

/// Verifier for [`HardwareAttestProver`] proofs; configured with the trusted
/// enclave BLS public key.
pub struct HardwareVerifier {
    trusted_enclave_pubkey: [u8; 48],
}

impl HardwareVerifier {
    pub fn new(trusted_enclave_pubkey: [u8; 48]) -> Self {
        Self { trusted_enclave_pubkey }
    }
}

impl Verifier for HardwareVerifier {
    fn verify(&self, proof: &ExecutionProof, expected_program_hash: Bytes32, trusted_roots: &[Bytes32], chain: &dyn ChainSource) -> Result<()> {
        if proof.program_hash != expected_program_hash {
            return Err(ProverError::ProgramHashMismatch {
                expected: expected_program_hash.to_hex(),
                actual: proof.program_hash.to_hex(),
            });
        }
        let (_nonce, pi_block) = parse_public_input(&proof.public_input)?;
        if pi_block != proof.chia_block {
            return Err(ProverError::Codec("public_input block != proof.chia_block".into()));
        }
        if proof.proof.len() != 96 {
            return Err(ProverError::AttestationInvalid("attestation not 96 bytes".into()));
        }
        let mut sig = [0u8; 96];
        sig.copy_from_slice(&proof.proof);
        let digest = attest_digest(&proof.program_hash, &proof.public_input, &proof.public_output);
        if !bls::verify(&self.trusted_enclave_pubkey, &digest, &sig) {
            return Err(ProverError::AttestationInvalid("enclave signature invalid".into()));
        }
        let msg = signing_message(&proof.proof, &proof.public_input);
        if !bls::verify(&proof.node_pubkey.0, &msg, &proof.node_signature.0) {
            return Err(ProverError::NodeSignatureInvalid);
        }
        if trusted_roots.is_empty() {
            return Err(ProverError::UntrustedRoot("no trusted roots provided".into()));
        }
        chain.verify_block(&proof.chia_block, DEFAULT_FRESHNESS_WINDOW_SECS)?;
        Ok(())
    }
}
```
Run: `cargo test -p digstore-prover --features hardware-attest --test hardware_attest`
Expected PASS: `test result: ok. 3 passed`.

- [ ] **13.3 Confirm default build excludes it.** Run: `cargo build -p digstore-prover` (no `--features`).
Expected: `Finished` with no compilation of `hardware.rs`. Then `cargo test -p digstore-prover` stays green.

- [ ] **13.4 Commit.** `git add crates/digstore-prover/src/hardware.rs crates/digstore-prover/tests/hardware_attest.rs` then `git commit -m "feat(prover): hardware-attest alternative behind same trait (§13.6)"`.

---

## Task 14 — risc0 guest crate (deviation #3 serving computation)

**Files:**
- Create: `crates/digstore-prover/guest/Cargo.toml`
- Create: `crates/digstore-prover/guest/src/main.rs`

> The guest re-executes the deterministic serving computation and commits a journal of `(program_hash, public_input_hash, roothash, public_output)`. The wire input type `GuestInput` is shared by name/shape on both sides (minor fix: no ad-hoc tuple). `public_output` is computed identically to the host (`SHA-256(roothash || concat(chunks))`), so a host that lies about its output cannot produce a matching journal.

Steps:

- [ ] **14.1 Write the guest manifest.** Create `crates/digstore-prover/guest/Cargo.toml`:
```toml
[package]
name = "digstore-guest-risc0"
version = "0.1.0"
edition = "2021"

[dependencies]
risc0-zkvm = { version = "1.2", default-features = false, features = ["std"] }
serde = { version = "1", features = ["derive"] }
sha2 = "0.10"

[[bin]]
name = "digstore-serving-guest"
path = "src/main.rs"
```

- [ ] **14.2 Write the guest program.** Create `crates/digstore-prover/guest/src/main.rs`. `GuestInput` is the explicit shared wire type:
```rust
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Wire input from the host prover. MUST be byte-identical to the host-side
/// `GuestInput` in `risc0_backend.rs`.
#[derive(Serialize, Deserialize)]
struct GuestInput {
    program_hash: [u8; 32],
    public_input: Vec<u8>,
    roothash: [u8; 32],
    chunks: Vec<Vec<u8>>,
}

fn main() {
    let input: GuestInput = env::read();

    // Deterministic serving computation: gather + concatenate the ciphertext,
    // then commit SHA-256(roothash || concat) — identical to the host.
    let mut preimage = Vec::new();
    preimage.extend_from_slice(&input.roothash);
    for c in &input.chunks {
        preimage.extend_from_slice(c);
    }
    let public_output: [u8; 32] = Sha256::digest(&preimage).into();
    let public_input_hash: [u8; 32] = Sha256::digest(&input.public_input).into();

    // Journal: (program_hash, public_input_hash, roothash, public_output)
    env::commit(&(input.program_hash, public_input_hash, input.roothash, public_output));
}
```

- [ ] **14.3 Commit the guest.** `git add crates/digstore-prover/guest` then `git commit -m "feat(prover): risc0 guest re-executing the serving computation (deviation #3)"`.

---

## Task 15 — risc0 host backend + prove/verify smoke (feature `risc0`)

**Files:**
- Create: `crates/digstore-prover/src/risc0_backend.rs`
- Create: `crates/digstore-prover/tests/risc0_smoke.rs`

> The host `Risc0Prover` runs the guest via `default_prover()`, serializes the receipt into `proof`, and signs `(proof || public_input)`. `Risc0Verifier` checks the receipt, decodes the journal, and compares each committed field to the proof. **Minor fixes:** the shared `GuestInput` struct is used on both sides (no ad-hoc tuple); a journal public-input divergence returns the dedicated `PublicInputMismatch` (not `NonceMismatch`); hex uses `Bytes32::to_hex` consistently. The generated constants `DIGSTORE_SERVING_GUEST_ELF` and `DIGSTORE_SERVING_GUEST_ID` come from `risc0-build` embedding the `[[bin]]` named `digstore-serving-guest` (uppercased + `_ELF`/`_ID`).

Steps:

- [ ] **15.1 Write `Risc0Prover`/`Risc0Verifier`.** Create `crates/digstore-prover/src/risc0_backend.rs`:
```rust
use digstore_core::{Bytes32, Bytes48, Bytes96, ExecutionProof, ChiaBlockRef};
use digstore_crypto::{sha256, bls};
use risc0_zkvm::{default_prover, ExecutorEnv, Receipt};
use serde::{Serialize, Deserialize};
use crate::serving_inputs::ServingInputs;
use crate::prover::{Prover, Verifier};
use crate::chain::ChainSource;
use crate::commitment::{parse_public_input, signing_message};
use crate::error::{ProverError, Result};
use crate::mock::DEFAULT_FRESHNESS_WINDOW_SECS;

// Generated by risc0-build (build.rs `embed_methods`): for the guest bin named
// `digstore-serving-guest`, these are `DIGSTORE_SERVING_GUEST_ELF` (&[u8]) and
// `DIGSTORE_SERVING_GUEST_ID` ([u32; 8]).
include!(concat!(env!("OUT_DIR"), "/methods.rs"));

/// Wire input to the guest. MUST be byte-identical to the guest-side
/// `GuestInput` in `guest/src/main.rs`.
#[derive(Serialize, Deserialize)]
struct GuestInput {
    program_hash: [u8; 32],
    public_input: Vec<u8>,
    roothash: [u8; 32],
    chunks: Vec<Vec<u8>>,
}

/// Real ZK prover: runs the serving computation in a risc0 guest (§13.1-13.3).
pub struct Risc0Prover {
    node_secret: bls::SecretKey,
    node_pubkey: bls::PublicKey,
    chia_block: ChiaBlockRef,
}

impl Risc0Prover {
    pub fn new(node_secret: bls::SecretKey, node_pubkey: bls::PublicKey, chia_block: ChiaBlockRef) -> Self {
        Self { node_secret, node_pubkey, chia_block }
    }
}

impl Prover for Risc0Prover {
    fn prove(&self, program_hash: Bytes32, public_input: &[u8], serving_inputs: &ServingInputs) -> Result<ExecutionProof> {
        let (_nonce, block) = parse_public_input(public_input)?;
        if block != self.chia_block {
            return Err(ProverError::Backend("public_input block mismatch".into()));
        }
        let guest_input = GuestInput {
            program_hash: program_hash.0,
            public_input: public_input.to_vec(),
            roothash: serving_inputs.roothash.0,
            chunks: serving_inputs.chunk_ciphertext.clone(),
        };
        let env = ExecutorEnv::builder()
            .write(&guest_input)
            .map_err(|e| ProverError::Backend(format!("env write: {e}")))?
            .build()
            .map_err(|e| ProverError::Backend(format!("env build: {e}")))?;
        let receipt = default_prover()
            .prove(env, DIGSTORE_SERVING_GUEST_ELF)
            .map_err(|e| ProverError::Backend(format!("prove: {e}")))?
            .receipt;
        let proof = bincode::serialize(&receipt)
            .map_err(|e| ProverError::Backend(format!("receipt ser: {e}")))?;
        let public_output = serving_inputs.compute_public_output();
        let msg = signing_message(&proof, public_input);
        let node_sig = bls::sign(&self.node_secret, &msg);
        Ok(ExecutionProof {
            program_hash,
            public_input: public_input.to_vec(),
            public_output,
            proof,
            chia_block: self.chia_block.clone(),
            node_pubkey: Bytes48(self.node_pubkey.to_bytes()),
            node_signature: Bytes96(node_sig.to_bytes()),
        })
    }
}

/// Verifier for [`Risc0Prover`] proofs.
#[derive(Default)]
pub struct Risc0Verifier;

impl Verifier for Risc0Verifier {
    fn verify(&self, proof: &ExecutionProof, expected_program_hash: Bytes32, trusted_roots: &[Bytes32], chain: &dyn ChainSource) -> Result<()> {
        if proof.program_hash != expected_program_hash {
            return Err(ProverError::ProgramHashMismatch {
                expected: expected_program_hash.to_hex(),
                actual: proof.program_hash.to_hex(),
            });
        }
        let (_nonce, pi_block) = parse_public_input(&proof.public_input)?;
        if pi_block != proof.chia_block {
            return Err(ProverError::Codec("public_input block != proof.chia_block".into()));
        }
        let receipt: Receipt = bincode::deserialize(&proof.proof)
            .map_err(|e| ProverError::ZkProofInvalid(format!("receipt de: {e}")))?;
        receipt.verify(DIGSTORE_SERVING_GUEST_ID)
            .map_err(|e| ProverError::ZkProofInvalid(format!("receipt verify: {e}")))?;
        // Journal: (program_hash, public_input_hash, roothash, public_output)
        let (j_program_hash, j_pi_hash, _j_root, j_output): ([u8; 32], [u8; 32], [u8; 32], [u8; 32]) =
            receipt.journal.decode().map_err(|e| ProverError::ZkProofInvalid(format!("journal decode: {e}")))?;
        if j_program_hash != proof.program_hash.0 {
            return Err(ProverError::ProgramHashMismatch {
                expected: proof.program_hash.to_hex(),
                actual: Bytes32(j_program_hash).to_hex(),
            });
        }
        if j_pi_hash != sha256(&proof.public_input) {
            return Err(ProverError::PublicInputMismatch);
        }
        if j_output != proof.public_output.0 {
            return Err(ProverError::PublicOutputMismatch);
        }
        let msg = signing_message(&proof.proof, &proof.public_input);
        if !bls::verify(&proof.node_pubkey.0, &msg, &proof.node_signature.0) {
            return Err(ProverError::NodeSignatureInvalid);
        }
        if trusted_roots.is_empty() {
            return Err(ProverError::UntrustedRoot("no trusted roots provided".into()));
        }
        chain.verify_block(&proof.chia_block, DEFAULT_FRESHNESS_WINDOW_SECS)?;
        Ok(())
    }
}
```

- [ ] **15.2 Build the risc0 feature to resolve generated names.** Run: `cargo build -p digstore-prover --features risc0`
Expected: `Finished` (this triggers `build.rs` → `risc0_build::embed_methods()`, generating `$OUT_DIR/methods.rs` with `DIGSTORE_SERVING_GUEST_ELF` and `DIGSTORE_SERVING_GUEST_ID`, which the `include!` then resolves). If it fails with `cannot find value DIGSTORE_SERVING_GUEST_ELF`, the constant name does not match the guest bin name — set the guest `[[bin]]` `name = "digstore-serving-guest"` exactly as in Task 14.1 (the macro uppercases the bin name and appends `_ELF`/`_ID`). Do not commit yet.

- [ ] **15.3 RED: risc0 prove→verify smoke + tampered-output reject.** Create `crates/digstore-prover/tests/risc0_smoke.rs`:
```rust
#![cfg(feature = "risc0")]
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::build_public_input;
use digstore_prover::risc0_backend::{Risc0Prover, Risc0Verifier};
use digstore_prover::{Prover, Verifier, MockChainSource, ServingInputs, ProverError};
use digstore_crypto::bls;

/// Real risc0 prove->verify. Slow; opt in with `--ignored`. In dev mode
/// (`RISC0_DEV_MODE=1`) it runs in seconds. This is a REAL test, not a stub.
#[test]
#[ignore = "slow: real risc0 proving; run with --ignored or RISC0_DEV_MODE=1"]
fn risc0_prove_verify_smoke() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![0xDE, 0xAD], vec![0xBE, 0xEF]] };

    let proof = Risc0Prover::new(sk, pk, block.clone()).prove(ph, &pi, &serving).expect("risc0 proving must succeed");
    assert_eq!(proof.public_output, serving.compute_public_output());

    let chain = MockChainSource::new(vec![block], 1_000_100);
    Risc0Verifier::default().verify(&proof, ph, &[root], &chain).expect("risc0 proof must verify");
}

#[test]
#[ignore = "slow: real risc0 proving"]
fn risc0_tampered_output_rejected() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1, 2, 3]] };
    let mut proof = Risc0Prover::new(sk, pk, block.clone()).prove(ph, &pi, &serving).unwrap();
    proof.public_output = Bytes32([0xEEu8; 32]); // tamper claimed output
    let chain = MockChainSource::new(vec![block], 1_000_100);
    let err = Risc0Verifier::default().verify(&proof, ph, &[root], &chain).unwrap_err();
    assert!(matches!(err, ProverError::PublicOutputMismatch));
}
```
Run (PowerShell): `$env:RISC0_DEV_MODE=1; cargo test -p digstore-prover --features risc0 --test risc0_smoke -- --ignored`
Expected on first run BEFORE 15.2 succeeded: a compile error naming `DIGSTORE_SERVING_GUEST_ELF`. After 15.2 resolves the generated names, expected PASS: `test result: ok. 2 passed; 0 failed` (run as `--ignored`).

- [ ] **15.4 GREEN: confirm smoke passes in dev mode.** Run: `$env:RISC0_DEV_MODE=1; cargo test -p digstore-prover --features risc0 --test risc0_smoke -- --ignored`
Expected PASS: `test result: ok. 2 passed`. Then confirm the default build is unaffected: `cargo build -p digstore-prover` → `Finished`, and `cargo test -p digstore-prover` stays green (risc0 code not compiled).

- [ ] **15.5 Commit.** `git add crates/digstore-prover/src/risc0_backend.rs crates/digstore-prover/tests/risc0_smoke.rs` then `git commit -m "feat(prover): risc0 host backend + prove/verify smoke (§13.1-13.4, deviation #3)"`.

---

## Task 16 — Object-safety GUARD + full sweep

**Files:**
- Create: `crates/digstore-prover/tests/trait_object_safety.rs`

> **GUARD task.** The traits were authored object-safe in Task 4 (no generics, no `Self`-by-value). This test exercises `Box<dyn Prover/Verifier/ChainSource>` so the host/remote crates can rely on trait objects. Expected to PASS on first run; no impl step.

Steps:

- [ ] **16.1 GUARD: traits are object-safe.** Create `crates/digstore-prover/tests/trait_object_safety.rs`:
```rust
use digstore_core::{Bytes32, ChiaBlockRef};
use digstore_prover::{build_public_input, MockProver, MockVerifier, Prover, Verifier, ChainSource, MockChainSource, ServingInputs};
use digstore_crypto::bls;

#[test]
fn prover_and_verifier_are_object_safe() {
    let sk = bls::SecretKey::from_seed(&[7u8; 32]);
    let pk = sk.public_key();
    let block = ChiaBlockRef { header_hash: Bytes32([0x55u8; 32]), height: 42, timestamp: 1_000_000 };
    let prover: Box<dyn Prover> = Box::new(MockProver::new(sk, pk, block.clone()));
    let verifier: Box<dyn Verifier> = Box::new(MockVerifier::default());
    let chain: Box<dyn ChainSource> = Box::new(MockChainSource::new(vec![block.clone()], 1_000_100));

    let ph = Bytes32([0xAAu8; 32]);
    let root = Bytes32([0xBBu8; 32]);
    let pi = build_public_input(&[0x33u8; 32], &block);
    let serving = ServingInputs { retrieval_key: Bytes32([1u8; 32]), roothash: root, chunk_ciphertext: vec![vec![1]] };
    let proof = prover.prove(ph, &pi, &serving).unwrap();
    verifier.verify(&proof, ph, &[root], chain.as_ref()).unwrap();
}
```
Run: `cargo test -p digstore-prover --test trait_object_safety`
Expected PASS (GUARD): `test result: ok. 1 passed`. If it fails with "the trait cannot be made into an object", remove any offending generic from `prover.rs`/`chain.rs`.

- [ ] **16.2 Full default test sweep.** Run: `cargo test -p digstore-prover`
Expected: green across `error::tests`, `serving_inputs::tests`, `commitment::tests`, `mock_chain::tests`, `mock_roundtrip`, `roothash_binding`, `nonce_binding`, `chain_freshness`, `node_attribution`, `program_hash`, `public_output`, `coinset_parse`, `trait_object_safety` — every block ends `test result: ok.`.

- [ ] **16.3 Feature sweep.** Run these:
  - `cargo test -p digstore-prover --features hardware-attest` → hardware tests green.
  - `cargo build -p digstore-prover --features risc0` → `Finished` (compile success).
  - `$env:RISC0_DEV_MODE=1; cargo test -p digstore-prover --features risc0 --test risc0_smoke -- --ignored` → `test result: ok. 2 passed`.

- [ ] **16.4 Commit.** `git add crates/digstore-prover/tests/trait_object_safety.rs` then `git commit -m "test(prover): GUARD object-safe traits; full default + feature sweep green"`.

---

## Definition of Done

| Paper section | Covered by | Verified by test |
|---------------|-----------|------------------|
| **13.1 What Is Proven** | `ServingInputs` + `Prover` trait; statement = `program_hash` run produces `public_output` | `serving_inputs::tests`, `mock_roundtrip`, `risc0_smoke` |
| **13.2 Proof Structure** | Canonical `ExecutionProof`/`ChiaBlockRef`/`ProofResponse`; `public_input = nonce ‖ ChiaBlockRef(codec)` | `commitment::tests::public_input_round_trips`, `commitment::tests::parse_rejects_trailing_bytes` |
| **13.3 Proving Pipeline** | `MockProver::prove`, `Risc0Prover::prove`, `HardwareAttestProver::prove`; per-request, bound to public input + block | `mock_roundtrip`, `risc0_smoke`, `hardware_attest` |
| **13.4 Verification** | `Verifier::verify` (program-hash + commitment-chain/journal output) AND `verify_response` cryptographic roothash binding | `roothash_binding` (incl. different-trusted-root rejection), `program_hash`, `public_output` |
| **13.5 Nonce Binding** | `verify_with_nonce` rejects nonce-A proof against nonce-B request | `nonce_binding` |
| **13.6 Hardware Alternative** | `HardwareAttestProver`/`HardwareVerifier` behind the same `Prover`/`Verifier` traits, feature `hardware-attest` | `hardware_attest` |
| **13.7 Node Attribution** | `node_signature` = BLS over `proof ‖ public_input`, verified under `node_pubkey` | `node_attribution`, `mock_roundtrip` |
| **13.8 Chain-Anchored Freshness** | `ChainSource` trait; `MockChainSource` + `CoinsetChainSource` (tx-block walk-down); freshness window enforced in `verify` | `chain_freshness`, `coinset_parse` (incl. walk-down) |
| Deviation #3 (risc0 re-executes serving computation; `program_hash = SHA-256(module_bytes)`; mock default) | documented in `lib.rs`; implemented in `risc0_backend.rs` + `guest/src/main.rs` | `risc0_smoke` |

**Exit criteria (all must hold):**
- Every checkbox above is checked.
- `cargo test -p digstore-prover` is green on default features.
- `cargo test -p digstore-prover --features hardware-attest` is green.
- `cargo build -p digstore-prover --features risc0` compiles and `$env:RISC0_DEV_MODE=1; cargo test -p digstore-prover --features risc0 --test risc0_smoke -- --ignored` reports `test result: ok. 2 passed`.
- No placeholders: every test asserts a real condition; every impl step shows compilable code; `chain.rs` and the guest are committed; all hex uses `Bytes32::to_hex`/`from_hex`; the risc0 journal public-input divergence returns `PublicInputMismatch`; the manifest's optional deps are all referenced by the `risc0` feature; the coinset live path resolves timestamps via tx-block walk-down.


---

## Plan metadata

- **Crate:** digstore-prover
- **Assigned paper sections:** 13.1,13.2,13.3,13.4,13.5,13.6,13.7,13.8
- **Depends on:** digstore-core, digstore-crypto
- **Spec sections covered (claimed):** 13.1, 13.2, 13.3, 13.4, 13.5, 13.6, 13.7, 13.8

### Public items exported (consumed by other crates)

```
pub trait Prover { fn prove(&self, program_hash: digstore_core::Bytes32, public_input: &[u8], serving_inputs: &ServingInputs) -> Result<digstore_core::ExecutionProof>; }
pub trait Verifier { fn verify(&self, proof: &digstore_core::ExecutionProof, expected_program_hash: digstore_core::Bytes32, trusted_roots: &[digstore_core::Bytes32], chain: &dyn ChainSource) -> Result<()>; fn verify_response(&self, response: &digstore_core::ProofResponse, expected_program_hash: digstore_core::Bytes32, trusted_roots: &[digstore_core::Bytes32], expected_output_bytes: &[u8], chain: &dyn ChainSource) -> Result<()>; fn verify_with_nonce(&self, proof: &digstore_core::ExecutionProof, expected_nonce: &[u8;32], expected_program_hash: digstore_core::Bytes32, trusted_roots: &[digstore_core::Bytes32], chain: &dyn ChainSource) -> Result<()>; }
pub trait ChainSource { fn get_peak(&self) -> Result<digstore_core::ChiaBlockRef>; fn verify_block(&self, block: &digstore_core::ChiaBlockRef, freshness_window_secs: u64) -> Result<()>; }
pub struct ServingInputs { pub retrieval_key: digstore_core::Bytes32, pub roothash: digstore_core::Bytes32, pub chunk_ciphertext: Vec<Vec<u8>> }
impl ServingInputs { pub fn output_bytes(&self) -> Vec<u8>; pub fn compute_public_output(&self) -> digstore_core::Bytes32; }
pub fn build_public_input(nonce: &[u8;32], block: &digstore_core::ChiaBlockRef) -> Vec<u8>
pub fn parse_public_input(bytes: &[u8]) -> Result<([u8;32], digstore_core::ChiaBlockRef)>
pub fn signing_message(proof: &[u8], public_input: &[u8]) -> Vec<u8>
pub const NONCE_LEN: usize = 32
pub fn bound_public_output(roothash: &digstore_core::Bytes32, output_bytes: &[u8]) -> digstore_core::Bytes32
pub struct MockProver; impl MockProver { pub fn new(secret: digstore_crypto::bls::SecretKey, pubkey: digstore_crypto::bls::PublicKey, chia_block: digstore_core::ChiaBlockRef) -> Self; } impl Prover for MockProver
pub struct MockVerifier; impl Default for MockVerifier; impl Verifier for MockVerifier
pub const DEFAULT_FRESHNESS_WINDOW_SECS: u64 = 600
pub struct MockChainSource; impl MockChainSource { pub fn new(blocks: Vec<digstore_core::ChiaBlockRef>, now: u64) -> Self; pub fn with_now(self, now: u64) -> Self; } impl ChainSource for MockChainSource
pub struct CoinsetChainSource; impl CoinsetChainSource { pub fn new(base_url: impl Into<String>) -> Self; } impl Default for CoinsetChainSource; impl ChainSource for CoinsetChainSource
pub fn parse_blockchain_state(resp: coinset::BlockchainStateResponse) -> Result<digstore_core::ChiaBlockRef>
pub fn parse_block_record_resolved(resp: coinset::BlockRecordResponse, resolve: &mut coinset::BlockRecordResolver) -> Result<digstore_core::ChiaBlockRef>
pub type coinset::BlockRecordResolver<'a> = dyn FnMut(u32) -> Result<coinset::BlockRecord> + 'a
pub struct coinset::BlockRecord { pub header_hash: String, pub height: u32, pub timestamp: Option<u64>, pub prev_transaction_block_height: Option<u32> }
pub struct coinset::BlockchainStateResponse { pub success: bool, pub blockchain_state: Option<coinset::BlockchainState>, pub error: Option<String> }
pub struct coinset::BlockRecordResponse { pub success: bool, pub block_record: Option<coinset::BlockRecord>, pub error: Option<String> }
pub enum ProverError { ProgramHashMismatch{expected:String,actual:String}, PublicOutputMismatch, PublicInputMismatch, NonceMismatch, UntrustedRoot(String), RootBindingMismatch{bound:String,asserted:String}, NodeSignatureInvalid, BlockTooOld{block_ts:u64,now:u64,window:u64}, BlockInFuture(u64,u64), BlockNotOnChain(String), ZkProofInvalid(String), AttestationInvalid(String), Backend(String), Codec(String), ChainRpc(String) }
pub type Result<T> = core::result::Result<T, ProverError>
#[cfg(feature="risc0")] pub struct risc0_backend::Risc0Prover; impl risc0_backend::Risc0Prover { pub fn new(node_secret: digstore_crypto::bls::SecretKey, node_pubkey: digstore_crypto::bls::PublicKey, chia_block: digstore_core::ChiaBlockRef) -> Self; } impl Prover for risc0_backend::Risc0Prover
#[cfg(feature="risc0")] pub struct risc0_backend::Risc0Verifier; impl Default for risc0_backend::Risc0Verifier; impl Verifier for risc0_backend::Risc0Verifier
#[cfg(feature="hardware-attest")] pub struct hardware::HardwareAttestProver; impl hardware::HardwareAttestProver { pub fn new(node_secret: digstore_crypto::bls::SecretKey, node_pubkey: digstore_crypto::bls::PublicKey, enclave_secret: digstore_crypto::bls::SecretKey, chia_block: digstore_core::ChiaBlockRef) -> Self; } impl Prover for hardware::HardwareAttestProver
#[cfg(feature="hardware-attest")] pub struct hardware::HardwareVerifier; impl hardware::HardwareVerifier { pub fn new(trusted_enclave_pubkey: [u8;48]) -> Self; } impl Verifier for hardware::HardwareVerifier
```