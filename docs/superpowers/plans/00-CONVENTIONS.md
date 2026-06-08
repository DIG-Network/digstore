# Digstore Implementation Conventions (BINDING)

> **Precedence:** This document **overrides** the per-crate plan files wherever they diverge. It resolves every cross-crate interface issue found by the global coverage critic. Read this before implementing any crate. All crates depend on these decisions.

Dependency order (acyclic, build bottom-up):
```
digstore-core
  → digstore-chunker, digstore-crypto
    → digstore-store
      → digstore-guest, digstore-prover
        → digstore-host
          → digstore-compiler, digstore-remote
            → digstore-cli
```

---

## C1. BLOCKER — `digstore-crypto::bls` module (BLS key types)

`digstore-host` and `digstore-prover` consume BLS key types from crypto. Crypto **MUST** export a public `bls` module with these exact names:

```rust
// crates/digstore-crypto/src/bls.rs  → pub mod bls;
pub struct SecretKey(/* blst SecretKey */);     // host-side signing key (chia-bls/blst)
pub struct PublicKey(/* blst PublicKey, G1 */);  // 48-byte G1
pub struct Signature(/* blst Signature, G2 */);  // 96-byte G2

pub type BlsSecretKey = SecretKey;   // alias used by digstore-host
pub type BlsPublicKey = PublicKey;

impl SecretKey {
    pub fn from_seed(seed: &[u8]) -> Self;
    pub fn public_key(&self) -> PublicKey;
    pub fn sign(&self, msg: &[u8]) -> Signature;        // AugScheme
}
impl PublicKey {
    pub fn to_bytes(&self) -> digstore_core::Bytes48;
    pub fn from_bytes(b: &digstore_core::Bytes48) -> Result<Self, CryptoError>;
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool;  // AugScheme
}
impl Signature {
    pub fn to_bytes(&self) -> digstore_core::Bytes96;
    pub fn from_bytes(b: &digstore_core::Bytes96) -> Result<Self, CryptoError>;
}
```

- The opaque `HostSigningKey` referenced in the crypto draft is **removed**; use `bls::SecretKey` everywhere.
- `digstore-host`: `HostDeps.bls_secret: digstore_crypto::bls::SecretKey`, `HostKeys.bls_secret: digstore_crypto::bls::SecretKey`.
- `digstore-prover`: `MockProver::new(node_secret: bls::SecretKey, node_pubkey: bls::PublicKey, ...)`, same for `Risc0Prover::new` and `HardwareAttestProver::new` (`enclave_secret: bls::SecretKey`).
- The guest does **not** use this module (it verifies with pure-Rust `bls12_381`); cross-impl parity fixtures (C7) prove the two agree.

## C2. MAJOR — `digstore-core` module-path convention

**Canonical convention: submodule paths are public AND flat re-exports are provided.** `digstore-core/src/lib.rs` declares all modules `pub` and re-exports primary types at the crate root. Additionally provide a `types` alias module for the byte newtypes (consumers reference `digstore_core::types::Bytes32`).

```rust
// crates/digstore-core/src/lib.rs
pub mod abi;
pub mod bytes;
pub mod codec;
pub mod config;
pub mod error;
pub mod hash;
pub mod keytable;
pub mod manifest;
pub mod merkle;
pub mod urn;
pub mod wire;

/// Alias module so `digstore_core::types::Bytes32` resolves (host/guest use this path).
pub mod types {
    pub use crate::bytes::{Bytes32, Bytes48, Bytes96};
}

// flat re-exports (convenience)
pub use abi::{is_error, pack_ptr_len, unpack_ptr_len};
pub use bytes::{Bytes32, Bytes48, Bytes96};
pub use config::{ChunkerConfig, CompilationResult, CompilationStats, CompilerError,
                 Generation, GenerationId, GenerationState, HostImportsConfig,
                 SecretSalt, StoreConfig, TrustedHostKey, Visibility};
pub use error::ErrorCode;          // ErrorCode lives in error.rs; ALSO re-exported as abi::ErrorCode
pub use keytable::{KeyTableEntry, PathWalk};
pub use manifest::{Author, MetadataManifest};
pub use merkle::{MerkleProof, MerkleTree, ProofStep};
pub use urn::Urn;
pub use wire::{AttestationChallenge, AttestationResponse, AuthenticationInfo,
               ChiaBlockRef, ContentResponse, ExecutionProof, ProofResponse};
```

These paths are therefore all valid and MUST resolve: `digstore_core::types::{Bytes32,Bytes48,Bytes96}`, `digstore_core::config::HostImportsConfig`, `digstore_core::abi::ErrorCode` (add `pub use crate::error::ErrorCode;` inside `abi.rs`), `digstore_core::merkle::MerkleTree`, plus all flat forms.

## C3. MAJOR — ExecutionProof generation ownership (guest vs host vs prover)

The **guest cannot generate an `ExecutionProof`** (no prover, no ChainSource, no node signing key inside wasm32; §13.3 says the *serving node* proves, not the module). Contract:

- **Guest `get_proof(req)`** returns a serialized **`ProofPrelude`** (defined in `digstore-core::wire`):
  ```rust
  pub struct ProofPrelude {
      pub roothash: Bytes32,
      pub output_commitment: Bytes32, // SHA-256 of the served output bytes (same bytes get_content returns)
      pub serving_digest: Bytes32,    // commitment over (retrieval_key, ordered chunk indices)
  }
  ```
- **Host `serve_proof(req)`** calls the guest's `get_proof` to obtain the `ProofPrelude`, then invokes `digstore_prover::Prover::prove(program_hash, public_input, &ServingInputs{..})` to produce the full `ExecutionProof`, wraps in `ProofResponse { proof, roothash }`, signs `node_signature` via `digstore_crypto::bls`.
- The guest's `ProofOutcome` therefore carries `ProofPrelude`, **not** a finished `ExecutionProof`.
- `digstore-guest` does **not** depend on `digstore-prover`. `digstore-host` depends on both.

## C4. MINOR — section 16 (temporal) ownership
Temporal validity (`ValidityWindow`, `within_window`, decoy-on-expiry) lives **only in `digstore-guest`**. Remove §16 from `digstore-crypto`'s claimed sections. Crypto only provides `attestation_signing_message(challenge)` byte layout.

## C5. MINOR — section 20.4 partition
`digstore-store` owns the **mechanics**: `RootHistory`, `Store::log() -> Vec<GenerationState>`, `Store::diff(a, b) -> GenerationDiff`. `digstore-cli` owns **presentation**: formatting `log`/`status`/`diff` for the terminal. CLI calls store mechanics; neither reimplements the other.

## C6. MINOR — CompilationStats / CompilationResult / CompilerError home
Canonical home is **`digstore-core::config`** for `CompilationResult { store_id, roothash, output_path, output_size, stats }`, `CompilationStats { chunk_count, total_bytes, generation_count }`, and `CompilerError` (with `NoTrustedKeys`). The compiler's richer internal stats are a **separate, renamed** struct `digstore_compiler::CompilerStats { generation_count, unique_chunk_count, resource_count, pool_byte_len, data_section_byte_len, obfuscation_applied }` carried inside `CompilationResult.stats`? No — to avoid two `CompilationStats`: `CompilationResult.stats` is `digstore_core::config::CompilationStats`; the compiler additionally returns its detail as `CompilerStats` in a separate field `CompilationResult` is **not** modified — compiler returns `(CompilationResult, CompilerStats)` or exposes `CompilerStats` via its own result wrapper `CompileOutcome { result: CompilationResult, detail: CompilerStats }`. Do **not** declare a second type named `CompilationStats`.

## C7. push-signing message — single source of truth
Canonical signer is **`digstore_crypto`**:
```rust
pub fn push_signing_message(root: &Bytes32, store_id: &Bytes32) -> [u8; 32]; // = SHA-256(root || store_id)
pub fn sign_push(sk: &bls::SecretKey, root: &Bytes32, store_id: &Bytes32) -> Bytes96;
pub fn verify_push(pk: &bls::PublicKey, root: &Bytes32, store_id: &Bytes32, sig: &Bytes96) -> bool;
```
Argument order is **`(root, store_id)`** everywhere; message is the 32-byte SHA-256. `digstore-remote` and `digstore-cli` **delegate** to these (remote does not define its own). A shared test vector (fixed root, store_id → fixed message + fixed sig) lives in crypto fixtures and is re-checked by remote + cli tests.

## C8. cross-impl parity fixtures (BLS)
`digstore-crypto` emits a fixtures file `crates/digstore-crypto/tests/fixtures/bls_vectors.json` (msg hex, pubkey hex 48B, sig hex 96B) produced by blst. `digstore-guest` tests load the **same** file and assert its pure-Rust `bls12_381` verifier accepts every vector. CI fails if either side drifts.

## C9. guest↔prover serving-output parity
The bytes the guest concatenates for `get_content` (ordered chunk ciphertext) MUST equal `digstore_prover::ServingInputs::output_bytes()` ordering, so program re-execution (deviation #3, `program_hash` binding) matches what was served. Shared helper in `digstore-core`:
```rust
// digstore_core::serving
pub fn concat_output(chunks_in_order: &[&[u8]]) -> alloc::vec::Vec<u8>; // simple ordered concat, single definition
```
Both guest and prover call `concat_output`. A cross-crate test asserts equality.

## C10. CLI client crypto — no parallel KDF
`digstore-cli` calls `digstore_crypto::derive_decryption_key(canonical_urn, Option<&SecretSalt>) -> [u8;32]` (it depends on crypto). It does **not** define its own `derive_decryption_key`. CLI client-side: derive key → AES-256-GCM open (verify tag) → reassemble by chunk order → `MerkleProof::verify` against trusted root → optional `Verifier::verify`.

---

## Toolchain / workspace
- Pinned toolchain via `rust-toolchain.toml` (channel `stable`, components `rustfmt`, `clippy`, target `wasm32-unknown-unknown`).
- Single workspace `Cargo.toml`; each crate `crates/<name>/`.
- `digstore-core` is `no_std + alloc` (feature `std` for host). The guest builds it with `--no-default-features`.
- Every crate: `cargo test -p <crate>`; guest additionally `cargo build -p digstore-guest --target wasm32-unknown-unknown --release`.
- TDD: failing test → minimal impl → green → commit. Conventional commits. Frequent.
