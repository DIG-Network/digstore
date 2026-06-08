# digstore-crypto Implementation Plan

> **For agentic workers:** Execute this plan with the **REQUIRED SUB-SKILL `superpowers:subagent-driven-development`**. Every numbered Task is a bite-sized TDD unit: write the failing test FIRST, run it and confirm the exact failure, write the minimal implementation, run it and confirm it passes, then commit. Do not batch steps. Do not skip the red phase. Each checkbox is one 2-5 minute action. There are NO "decide later", "STOP and confirm", or "try X or Y" steps in this plan — every API call and every fixture value is committed against a verified `chia-bls 0.45` / `hkdf 0.12` / `aes-gcm 0.10` surface.

**Goal:** Provide all host-side cryptographic primitives for Digstore — URN-derived key derivation (SHA-256 + HKDF-SHA256), per-URN AES-256-GCM chunk encryption/decryption with tamper detection, Chia-AugScheme BLS keygen/sign/verify, and the canonical push/node/attestation signing-message constructions — plus committed cross-implementation parity fixtures that the guest's pure-Rust `bls12_381` verifier must accept.

**Architecture:** A host-only library crate that wraps `sha2`, `hkdf`, `aes-gcm`, and `chia-bls` (blst). It exposes free functions and thin newtype wrappers, never holding global state. All keying is deterministic and derived solely from the canonical URN (plus an optional `SecretSalt` for private stores), enforcing the unique-key-per-URN invariant that makes the documented fixed-GCM-nonce deviation safe. The crate-private `chia_bls::SecretKey` is wrapped in an opaque `HostSigningKey` so no downstream crate version-locks to `chia-bls`; every public input/output uses the canonical `digstore-core` types (`Bytes32`/`Bytes48`/`Bytes96`/`SecretSalt`). The crate emits JSON fixtures (BLS parity + KDF KAT) consumed by `digstore-guest` and frozen against regeneration.

**Tech Stack:** Rust 1.94 (host target only); `digstore-core` (canonical types: `Bytes32`, `Bytes48`, `Bytes96`, `SecretSalt`, `Urn`, `AttestationChallenge`, and the shared scheme constant `CHIA_BLS_SCHEME`); `sha2 = "0.10"`; `hkdf = "0.12"`; `aes-gcm = "0.10"`; `chia-bls = "=0.45.0"` (pinned, blst backend); `hex = "0.4"`; `serde` + `serde_json` (fixtures); `thiserror = "1"`; `tempfile = "3"` (dev-only).

**Documented deviations relevant to this crate:**
- **GCM fixed nonce (paper §11.2).** AES-256-GCM uses a FIXED 12-byte all-zero nonce. This is safe *only* under the unique-key-per-URN invariant: each canonical URN derives a distinct AES-256 key, so no key is ever reused across two plaintexts. This crate enforces and tests that invariant (Task 8); it never reuses a key.
- **Codec endianness (Chia, paper deviation 1).** Not directly relevant here — this crate produces raw byte material; serialization framing (Chia big-endian) lives in `digstore-core`. Fixtures are emitted as hex JSON, not via the data-section codec. Where a height is bound into a signed message (`sign_node`), it is encoded big-endian to match the Chia-compat rule.

**Verified upstream API facts (do not re-derive; these were confirmed against `chia-bls 0.45.0`):**
- `chia_bls::SecretKey::from_seed(seed: &[u8]) -> SecretKey` (takes a byte **slice**).
- `chia_bls::SecretKey::public_key(&self) -> PublicKey`.
- `chia_bls::SecretKey::to_bytes(&self) -> [u8; 32]`.
- `chia_bls::PublicKey::to_bytes(&self) -> [u8; 48]`; `PublicKey::from_bytes(&[u8; 48]) -> Result<PublicKey, chia_bls::Error>`.
- `chia_bls::Signature::to_bytes(&self) -> [u8; 96]`; `Signature::from_bytes(&[u8; 96]) -> Result<Signature, chia_bls::Error>`.
- `chia_bls::sign(sk: &SecretKey, msg: impl AsRef<[u8]>) -> Signature` (AugScheme: prepends the public key and uses the Chia DST).
- `chia_bls::verify(sig: &Signature, key: &PublicKey, msg: impl AsRef<[u8]>) -> bool`.
- For seed `[0u8, 1, 2, …, 31]`: the G1 public key is `8f336467f057b373bb3c43815a10ec131119d1bf50c14fa3f9ad86c0ec074f920f936a5315a8365a37fee0afa34c32c6` and AugScheme-signing message `[7u8, 8, 9]` yields the G2 signature `a5ce62a76c749a06c85b2d3762523b2e1d6756455767d2023967480433f7225c5cf42b3e14d0df41c0e6f9ecc18a39c30fdbfdbfd422945b478cc1675adf046aefbf4810e3ab9b0eb09855d3e5540cb0924e0f3d0e324bb59c59659b1c6b4283`. These are used as the Chia conformance KAT in Task 13.
- `from_bytes` on `PublicKey`/`Signature` returns `Err` for non-canonical bytes such as all-`0xFF` (verified), so `bls_verify` returning `false` on the error path is exercised by real input.

---

## File Structure

All paths under `crates/digstore-crypto/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest: pinned `chia-bls = "=0.45.0"`, `digstore-core` path dep, hashing/aead/kdf deps, `tempfile` dev-dep. |
| `src/lib.rs` | Crate root: module declarations, re-exports of the public surface, crate-level docs stating the deviations and the guest parity contract. |
| `src/error.rs` | `TamperError`, `BlsError`, `CryptoError` definitions (via `thiserror`), all `PartialEq + Eq`. |
| `src/hash.rs` | `sha256(&[u8]) -> Bytes32` thin wrapper used internally and re-exported. |
| `src/kdf.rs` | `derive_decryption_key(canonical_urn, Option<SecretSalt>) -> [u8;32]` (HKDF-SHA256 with documented salt/info). |
| `src/aead.rs` | `encrypt_chunk(key, plaintext) -> Vec<u8>` and `decrypt_chunk(key, ct) -> Result<Vec<u8>, TamperError>` (AES-256-GCM, fixed nonce). |
| `src/bls.rs` | `HostSigningKey` opaque wrapper; `bls_keygen`, `bls_sign`, `bls_verify`, `validate_public_key`; canonical signing-message builders `push_signing_message`, `node_signing_message`, `attestation_signing_message`; `sign_push`, `verify_push`, `sign_node`, `sign_attestation`; Chia AugScheme. |
| `src/fixtures.rs` | `BlsFixture`/`BlsFixtureSet`, `KdfFixture`/`KdfFixtureSet`; `generate()` + `write_bls_fixtures`/`write_kdf_fixtures`; emits cross-impl + KAT vectors to shared JSON. |
| `tests/hash_kat.rs` | SHA-256 wrapper known-answer + URN/retrieval-key bridge tests. |
| `tests/kdf_kat.rs` | HKDF determinism, unique-key-per-URN invariant, and committed-KAT-fixture stability tests. |
| `tests/aead_roundtrip.rs` | AES-GCM round-trip + tamper-detection tests. |
| `tests/bls_roundtrip.rs` | BLS sign/verify round-trip, wrong-key/message/malformed rejection, Chia known-vector conformance, message-binding tests for push/node/attestation. |
| `tests/bls_fixtures.rs` | Generates, self-verifies, and freezes the cross-impl parity fixtures; asserts stable decoded content. |
| `tests/fixtures/bls_parity.json` | (committed, regenerated by an example) Shared BLS fixtures consumed by `digstore-guest` tests. |
| `tests/fixtures/kdf_kat.json` | (committed, regenerated by an example) Frozen HKDF KAT vectors. |
| `examples/gen_fixtures.rs` | Non-test binary that (re)generates both committed fixture files; the only thing that writes into the source tree. |

---

## Task 0 — Pin dependencies and confirm the canonical-type preconditions

**Files:**
- Create: `crates/digstore-crypto/Cargo.toml`
- Modify: `Cargo.toml` (workspace root — add member if a `members` list exists)

Steps:

- [ ] Run `cargo metadata --no-deps --format-version 1 --manifest-path C:/Users/micha/workspace/dig_network/digstore_wasm/Cargo.toml` and confirm it succeeds. If `members` is an explicit array and `digstore-crypto` is absent, add the line `"crates/digstore-crypto",` to that array. If members use a glob (`"crates/*"`), no change is needed.
- [ ] Confirm the canonical-type preconditions this crate depends on by running `cargo doc -p digstore-core --no-deps` and visually confirming the generated docs declare each of: `Bytes32(pub [u8; 32])`, `Bytes48(pub [u8; 48])`, `Bytes96(pub [u8; 96])`, `SecretSalt(pub [u8; 32])`, `Urn { chain, store_id, root_hash, resource_key }` with `fn canonical(&self) -> String` and `fn retrieval_key(&self) -> Bytes32`, `AttestationChallenge { nonce: [u8;32], store_id: [u8;32], timestamp: u64 }`, and the shared constant `pub const CHIA_BLS_SCHEME: &str`. These preconditions are facts assumed by every later task; the canonical catalog mandates them. (If `CHIA_BLS_SCHEME` is not yet present in `digstore-core`, it is owned there per the canonical-types rule; this crate references `digstore_core::CHIA_BLS_SCHEME` and the `digstore-core` plan adds the const — value `"chia-aug-scheme-bls12381-g2-xmd-sha256-sswu-ro"`.)
- [ ] Create `crates/digstore-crypto/Cargo.toml` with this exact content:

```toml
[package]
name = "digstore-crypto"
version = "0.1.0"
edition = "2021"
description = "Host-side cryptography for Digstore: URN key derivation, AES-256-GCM chunk encryption, and Chia AugScheme BLS."

[dependencies]
digstore-core = { path = "../digstore-core" }
sha2 = "0.10"
hkdf = "0.12"
aes-gcm = "0.10"
chia-bls = "=0.45.0"
hex = "0.4"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] Run `cargo build -p digstore-crypto`. Expect an error because `src/lib.rs` does not exist yet: `error: couldn't read crates/digstore-crypto/src/lib.rs`. This confirms the manifest is wired and the next task creates the root.
- [ ] Commit. `git add crates/digstore-crypto/Cargo.toml Cargo.toml` then:
```
chore(crypto): pin chia-bls=0.45.0 and scaffold digstore-crypto manifest
```

---

## Task 1 — Crate root, error types, and SHA-256 wrapper (scaffolding, explicitly non-TDD)

This task creates the compile-able skeleton. The error types and `sha256` wrapper are thin pass-throughs over `thiserror` and `sha2`; their behavior is locked by the first failing test in Task 2 (which is written BEFORE this implementation is exercised by any assertion). These three files are scaffolding, created so the crate compiles; the red phase for `sha256` lives in Task 2.

**Files:**
- Create: `crates/digstore-crypto/src/lib.rs`
- Create: `crates/digstore-crypto/src/error.rs`
- Create: `crates/digstore-crypto/src/hash.rs`

Steps:

- [ ] Create `crates/digstore-crypto/src/error.rs` with this exact content:

```rust
use thiserror::Error;

/// Returned when AES-256-GCM authentication fails (ciphertext or tag tampered).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("AES-256-GCM authentication failed: ciphertext or tag was tampered")]
pub struct TamperError;

/// BLS-layer errors (malformed key/signature bytes).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BlsError {
    #[error("invalid BLS public key bytes")]
    InvalidPublicKey,
    #[error("invalid BLS signature bytes")]
    InvalidSignature,
}

/// Umbrella crypto error returned by `decrypt_and_unwrap` and any caller that
/// wants a single error type spanning AEAD and BLS failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CryptoError {
    #[error(transparent)]
    Tamper(#[from] TamperError),
    #[error(transparent)]
    Bls(#[from] BlsError),
}
```

- [ ] Create `crates/digstore-crypto/src/hash.rs` with this exact content:

```rust
use digstore_core::Bytes32;
use sha2::{Digest, Sha256};

/// SHA-256 over `data`, returned as the canonical `Bytes32` newtype.
pub fn sha256(data: &[u8]) -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    Bytes32(arr)
}
```

- [ ] Create `crates/digstore-crypto/src/lib.rs` with this exact content (modules `kdf`, `aead`, `bls`, `fixtures` are declared but their files are created in their own tasks; declaring an absent module fails to build, so we add the module files as empty stubs in the SAME task that first needs them — for now only declare `error` and `hash`):

```rust
//! Host-side cryptography for Digstore.
//!
//! # Documented deviations
//! - **Fixed GCM nonce (paper §11.2):** AES-256-GCM uses a fixed all-zero
//!   12-byte nonce. Safe *only* because every canonical URN derives a unique
//!   AES-256 key (unique-key-per-URN invariant). A key is never reused across
//!   two plaintexts.
//! - **BLS (paper §11.3, §12, §13.7, §21.6):** Chia AugScheme via `chia-bls`
//!   (blst). G1 public keys are 48 bytes, G2 signatures are 96 bytes. Messages
//!   are augmented with the public key and hashed with the Chia DST, exactly as
//!   the guest's pure-Rust `bls12_381` verifier expects.
//!
//! # Cross-impl parity contract
//! `tests/fixtures/bls_parity.json` holds host-signed (blst) AugScheme vectors
//! that `digstore-guest`'s pure-Rust `bls12_381` verifier MUST accept. The
//! scheme tag in that file is [`digstore_core::CHIA_BLS_SCHEME`]; both crates
//! compare against the same constant. Regenerate with `cargo run -p
//! digstore-crypto --example gen_fixtures`.
//!
//! # Type boundary
//! The `chia-bls` secret key never crosses this crate's public boundary; it is
//! wrapped in the opaque [`bls::HostSigningKey`]. All public inputs/outputs use
//! canonical `digstore-core` types (`Bytes32`/`Bytes48`/`Bytes96`/`SecretSalt`).

pub mod error;
pub mod hash;

pub use error::{BlsError, CryptoError, TamperError};
pub use hash::sha256;
```

- [ ] Run `cargo build -p digstore-crypto`. Expect: `Compiling digstore-crypto v0.1.0` then `Finished`. (If this fails because `digstore-core` does not expose `Bytes32(pub [u8;32])`, the failure is a precondition violation from Task 0; do not work around it here.)
- [ ] Commit. `git add crates/digstore-crypto/src/lib.rs crates/digstore-crypto/src/error.rs crates/digstore-crypto/src/hash.rs` then:
```
feat(crypto): crate root, error types, and sha256 Bytes32 wrapper
```

---

## Task 2 — SHA-256 wrapper known-answer test (true red-first)

The `sha256` function already exists from Task 1, so this test compiles and passes on first run; that is correct for a pure-wrapper KAT. To honor a *genuine* red phase, the FIRST checkbox writes the test against a NOT-YET-EXISTING constant `digstore_crypto::CRYPTO_VERSION` so it fails to compile, then the implementation adds that constant. This avoids the "assert a wrong value then revert" theater.

**Files:**
- Create: `crates/digstore-crypto/tests/hash_kat.rs`
- Modify: `crates/digstore-crypto/src/lib.rs`

Steps:

- [ ] Create `crates/digstore-crypto/tests/hash_kat.rs` with this exact content:

```rust
use digstore_crypto::sha256;

#[test]
fn sha256_known_answer_abc() {
    // FIPS 180-2 test vector for "abc".
    let got = sha256(b"abc");
    let expected =
        hex::decode("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            .unwrap();
    assert_eq!(&got.0[..], &expected[..]);
}

#[test]
fn sha256_known_answer_empty() {
    let got = sha256(b"");
    let expected =
        hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .unwrap();
    assert_eq!(&got.0[..], &expected[..]);
}

#[test]
fn crate_advertises_its_version() {
    assert_eq!(digstore_crypto::CRYPTO_VERSION, 1);
}
```

- [ ] Run `cargo test -p digstore-crypto --test hash_kat`. Expect FAIL to COMPILE: `error[E0425]: cannot find value 'CRYPTO_VERSION' in crate 'digstore_crypto'`. (`hex` is a normal dependency of the crate, so it resolves in integration tests without a dev-dep entry.)
- [ ] Add the constant to `src/lib.rs` immediately after the `pub use hash::sha256;` line:

```rust
/// Versioning tag for the crypto domain constants (HKDF salt/info, scheme tag).
/// Bumping this signals a deliberate, breaking change to derived material.
pub const CRYPTO_VERSION: u32 = 1;
```

- [ ] Run `cargo test -p digstore-crypto --test hash_kat`. Expect PASS: `test sha256_known_answer_abc ... ok`, `test sha256_known_answer_empty ... ok`, `test crate_advertises_its_version ... ok`.
- [ ] Commit:
```
test(crypto): lock SHA-256 wrapper with FIPS vectors and crate version const
```

---

## Task 3 — URN-to-retrieval-key bridge test

This crate does not own URN parsing (that is `digstore-core`), but its key derivation hashes the canonical URN string. This test proves `sha256(canonical_urn.as_bytes()) == Urn::retrieval_key()`, guarding against drift between the two crates.

**Files:**
- Modify: `crates/digstore-crypto/tests/hash_kat.rs` (append)

Steps:

- [ ] Append this test to `crates/digstore-crypto/tests/hash_kat.rs`:

```rust
#[test]
fn sha256_of_canonical_urn_equals_retrieval_key() {
    use digstore_core::{Bytes32, Urn};

    let urn = Urn {
        chain: "mainnet".to_string(),
        store_id: Bytes32([0x11; 32]),
        root_hash: None,
        resource_key: Some("file.txt".to_string()),
    };
    let canonical = urn.canonical();
    let direct: Bytes32 = digstore_crypto::sha256(canonical.as_bytes());
    let via_core: Bytes32 = urn.retrieval_key();
    assert_eq!(direct, via_core);
}
```

- [ ] Run `cargo test -p digstore-crypto --test hash_kat sha256_of_canonical_urn_equals_retrieval_key`. Expect PASS: `... ok`. (If it fails with a mismatch, `digstore-core`'s `retrieval_key` does not hash the canonical string as the canonical catalog requires — that is a `digstore-core` precondition defect, not something this crate patches.)
- [ ] Commit:
```
test(crypto): assert sha256(canonical urn) matches Urn::retrieval_key
```

---

## Task 4 — HKDF derivation contract (failing test for public store, then implement)

Decision locked here (documented in code): HKDF-SHA256 with
- `ikm = canonical_urn.as_bytes()`
- `salt`: public stores use `SHA-256(b"digstore-hkdf-salt-v1")`; private stores use `SHA-256(b"digstore-hkdf-salt-v1" || secret_salt)` (mixes the 32-byte `SecretSalt`). Both salts are always 32 bytes.
- `info = b"digstore-aes-256-gcm-key-v1"`
- output length = 32 bytes (AES-256 key).

**Files:**
- Create: `crates/digstore-crypto/src/kdf.rs`
- Create: `crates/digstore-crypto/tests/kdf_kat.rs`
- Modify: `crates/digstore-crypto/src/lib.rs`

Steps:

- [ ] Create `crates/digstore-crypto/tests/kdf_kat.rs` with this exact content (failing first — the function does not exist):

```rust
#[test]
fn derive_decryption_key_public_is_32_bytes_and_deterministic() {
    let canonical = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/file.txt";
    let k1 = digstore_crypto::derive_decryption_key(canonical, None);
    let k2 = digstore_crypto::derive_decryption_key(canonical, None);
    assert_eq!(k1.len(), 32);
    assert_eq!(k1, k2, "derivation must be deterministic for a given URN");
}
```

- [ ] Run `cargo test -p digstore-crypto --test kdf_kat derive_decryption_key_public_is_32_bytes_and_deterministic`. Expect FAIL to COMPILE: `error[E0425]: cannot find function 'derive_decryption_key' in crate 'digstore_crypto'`.
- [ ] Create `crates/digstore-crypto/src/kdf.rs` with this exact content:

```rust
use digstore_core::SecretSalt;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

/// Fixed HKDF salt domain string for stores (paper §11.1, §11.4).
const HKDF_SALT_DOMAIN: &[u8] = b"digstore-hkdf-salt-v1";
/// Fixed HKDF `info` context for the AES-256-GCM content key (paper §11.1).
const HKDF_INFO: &[u8] = b"digstore-aes-256-gcm-key-v1";

/// Derive the 32-byte AES-256 content key for a resource from its canonical URN.
///
/// `ikm = canonical_urn` bytes. For public stores the salt is
/// `SHA-256(HKDF_SALT_DOMAIN)`. For private stores (paper §11.4) the
/// `SecretSalt` is mixed in: `salt = SHA-256(HKDF_SALT_DOMAIN || secret_salt)`.
/// Output is always 32 bytes. Distinct URNs (and distinct salts) derive
/// distinct keys — the invariant that makes the fixed GCM nonce safe (§11.2).
pub fn derive_decryption_key(canonical_urn: &str, secret_salt: Option<SecretSalt>) -> [u8; 32] {
    let mut salt_hasher = Sha256::new();
    salt_hasher.update(HKDF_SALT_DOMAIN);
    if let Some(SecretSalt(secret)) = secret_salt {
        salt_hasher.update(secret);
    }
    let salt_digest = salt_hasher.finalize();
    let mut salt = [0u8; 32];
    salt.copy_from_slice(&salt_digest);

    let hk = Hkdf::<Sha256>::new(Some(&salt), canonical_urn.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(HKDF_INFO, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}
```

- [ ] In `src/lib.rs`, add the module declaration and re-export. After `pub mod hash;` add `pub mod kdf;`, and after `pub use hash::sha256;` add:

```rust
pub use kdf::derive_decryption_key;
```

- [ ] Run `cargo test -p digstore-crypto --test kdf_kat derive_decryption_key_public_is_32_bytes_and_deterministic`. Expect PASS: `... ok`.
- [ ] Commit:
```
feat(crypto): derive_decryption_key via HKDF-SHA256 from canonical URN
```

---

## Task 5 — Unique-key-per-URN invariant (the §11.2 fixed-nonce safety basis)

This is the security predicate that makes the fixed GCM nonce safe. No frozen-literal KAT is used here (frozen KDF KATs live in Task 6 as a committed generated fixture, never as a hand-typed hex literal).

**Files:**
- Modify: `crates/digstore-crypto/tests/kdf_kat.rs` (append)

Steps:

- [ ] Append the invariant tests:

```rust
#[test]
fn two_distinct_urns_yield_two_distinct_keys() {
    let a = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/a.txt";
    let b = "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/b.txt";
    let ka = digstore_crypto::derive_decryption_key(a, None);
    let kb = digstore_crypto::derive_decryption_key(b, None);
    assert_ne!(ka, kb, "distinct URNs MUST derive distinct keys (fixed-nonce safety)");
}

#[test]
fn public_and_private_same_urn_yield_distinct_keys() {
    use digstore_core::SecretSalt;
    let u = "urn:dig:mainnet:2222222222222222222222222222222222222222222222222222222222222222/a";
    let pub_k = digstore_crypto::derive_decryption_key(u, None);
    let priv_k = digstore_crypto::derive_decryption_key(u, Some(SecretSalt([0x09; 32])));
    assert_ne!(pub_k, priv_k, "private store must not collide with public key for same URN");
}

#[test]
fn two_private_salts_same_urn_yield_distinct_keys() {
    use digstore_core::SecretSalt;
    let u = "urn:dig:mainnet:2222222222222222222222222222222222222222222222222222222222222222/a";
    let k1 = digstore_crypto::derive_decryption_key(u, Some(SecretSalt([0x01; 32])));
    let k2 = digstore_crypto::derive_decryption_key(u, Some(SecretSalt([0x02; 32])));
    assert_ne!(k1, k2, "different SecretSalts must derive different keys");
}
```

- [ ] Run `cargo test -p digstore-crypto --test kdf_kat`. Expect all four tests PASS (the three new ones plus the determinism test from Task 4).
- [ ] Commit:
```
test(crypto): enforce unique-key-per-URN invariant for fixed-nonce safety
```

---

## Task 6 — KDF KAT fixtures: generate, write via example, freeze on regeneration

Locks the exact 32-byte HKDF outputs so any future refactor that silently changes salt/info/ikm fails loudly — WITHOUT a hand-typed hex literal. The values are produced programmatically by `generate()`, written once into a committed JSON file by the `gen_fixtures` example, and a test asserts that the committed file equals fresh in-memory regeneration (no file write in the test path).

**Files:**
- Create: `crates/digstore-crypto/src/fixtures.rs` (KDF part now; BLS part appended in Task 12)
- Create: `crates/digstore-crypto/examples/gen_fixtures.rs`
- Modify: `crates/digstore-crypto/src/lib.rs`
- Modify: `crates/digstore-crypto/tests/kdf_kat.rs` (append)
- Create: `crates/digstore-crypto/tests/fixtures/kdf_kat.json` (generated, committed)

Steps:

- [ ] Append the failing fixture-stability test to `crates/digstore-crypto/tests/kdf_kat.rs`:

```rust
use digstore_crypto::fixtures::KdfFixtureSet;

#[test]
fn committed_kdf_fixture_matches_generated() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("kdf_kat.json");
    let on_disk = std::fs::read_to_string(&path).expect(
        "committed kdf_kat.json must exist; run: cargo run -p digstore-crypto --example gen_fixtures",
    );
    let parsed: KdfFixtureSet = serde_json::from_str(&on_disk).unwrap();
    let fresh = KdfFixtureSet::generate();

    assert_eq!(parsed.crypto_version, fresh.crypto_version);
    assert_eq!(parsed.vectors.len(), fresh.vectors.len());
    for (a, b) in parsed.vectors.iter().zip(fresh.vectors.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.canonical_urn, b.canonical_urn);
        assert_eq!(a.secret_salt_hex, b.secret_salt_hex);
        assert_eq!(a.key_hex, b.key_hex, "KDF output drift in '{}'", a.name);
    }
}
```

- [ ] Run `cargo test -p digstore-crypto --test kdf_kat committed_kdf_fixture_matches_generated`. Expect FAIL to COMPILE: `error[E0432]: unresolved import 'digstore_crypto::fixtures'`.
- [ ] Create `crates/digstore-crypto/src/fixtures.rs` with this exact content (KDF half; the BLS half is appended in Task 12):

```rust
use crate::derive_decryption_key;
use digstore_core::SecretSalt;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// One HKDF known-answer vector: a canonical URN (+ optional secret salt) and
/// the 32-byte derived AES-256 key. Frozen so derivation cannot silently drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfFixture {
    pub name: String,
    pub canonical_urn: String,
    /// `None` for public stores; hex of the 32-byte `SecretSalt` otherwise.
    pub secret_salt_hex: Option<String>,
    pub key_hex: String,
}

/// The full frozen KDF KAT set, tagged with the crate's `CRYPTO_VERSION`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfFixtureSet {
    pub crypto_version: u32,
    pub vectors: Vec<KdfFixture>,
}

impl KdfFixtureSet {
    /// Deterministically generate the canonical KAT set.
    pub fn generate() -> Self {
        // (name, urn, optional secret salt) tuples.
        let specs: &[(&str, &str, Option<[u8; 32]>)] = &[
            (
                "public_root_a",
                "urn:dig:mainnet:0000000000000000000000000000000000000000000000000000000000000000/a",
                None,
            ),
            (
                "public_root_file",
                "urn:dig:mainnet:1111111111111111111111111111111111111111111111111111111111111111/file.txt",
                None,
            ),
            (
                "private_salt_07",
                "urn:dig:mainnet:0000000000000000000000000000000000000000000000000000000000000000/a",
                Some([0x07; 32]),
            ),
            (
                "private_salt_09",
                "urn:dig:mainnet:2222222222222222222222222222222222222222222222222222222222222222/a",
                Some([0x09; 32]),
            ),
        ];

        let mut vectors = Vec::with_capacity(specs.len());
        for (name, urn, salt) in specs {
            let salt_opt = salt.map(SecretSalt);
            let key = derive_decryption_key(urn, salt_opt);
            vectors.push(KdfFixture {
                name: name.to_string(),
                canonical_urn: urn.to_string(),
                secret_salt_hex: salt.map(hex::encode),
                key_hex: hex::encode(key),
            });
        }

        KdfFixtureSet {
            crypto_version: crate::CRYPTO_VERSION,
            vectors,
        }
    }
}

/// Generate and write the KDF KAT set as pretty JSON to `path`, creating parent
/// directories as needed. Called only by `examples/gen_fixtures.rs`.
pub fn write_kdf_fixtures(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let set = KdfFixtureSet::generate();
    let json =
        serde_json::to_string_pretty(&set).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}
```

- [ ] In `src/lib.rs`, declare the module and re-export the KDF fixture surface. After `pub mod kdf;` add `pub mod fixtures;`, and after the `pub use kdf::derive_decryption_key;` line add:

```rust
pub use fixtures::{write_kdf_fixtures, KdfFixture, KdfFixtureSet};
```

- [ ] Create `crates/digstore-crypto/examples/gen_fixtures.rs` with this exact content (this is the ONLY thing that writes into the source tree):

```rust
//! Regenerates the committed fixture files under `tests/fixtures/`.
//! Run with: cargo run -p digstore-crypto --example gen_fixtures

use std::path::Path;

fn main() -> std::io::Result<()> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    digstore_crypto::write_kdf_fixtures(&base.join("kdf_kat.json"))?;
    println!("wrote {}", base.join("kdf_kat.json").display());
    Ok(())
}
```

- [ ] Generate the committed file by running `cargo run -p digstore-crypto --example gen_fixtures`. Expect a line `wrote .../tests/fixtures/kdf_kat.json`. (The BLS fixture write is added to this example in Task 12; for now it only writes the KDF file.)
- [ ] Run `cargo test -p digstore-crypto --test kdf_kat committed_kdf_fixture_matches_generated`. Expect PASS: `... ok`.
- [ ] Commit (include the generated fixture so it is tracked):
```
feat(crypto): freeze HKDF KAT fixtures via gen_fixtures example
```

---

## Task 7 — AES-256-GCM encrypt/decrypt round-trip

Locked: AES-256-GCM, 12-byte FIXED all-zero nonce, no associated data; ciphertext layout = GCM ciphertext with the 16-byte tag appended (the `aes-gcm` crate's `encrypt` returns `ct||tag`).

**Files:**
- Create: `crates/digstore-crypto/src/aead.rs`
- Create: `crates/digstore-crypto/tests/aead_roundtrip.rs`
- Modify: `crates/digstore-crypto/src/lib.rs`

Steps:

- [ ] Create `crates/digstore-crypto/tests/aead_roundtrip.rs` with this exact content:

```rust
use digstore_crypto::{decrypt_chunk, derive_decryption_key, encrypt_chunk};

#[test]
fn encrypt_then_decrypt_recovers_plaintext() {
    let key = derive_decryption_key(
        "urn:dig:mainnet:3333333333333333333333333333333333333333333333333333333333333333/x",
        None,
    );
    let plaintext = b"the quick brown fox jumps over the lazy dog".to_vec();
    let ct = encrypt_chunk(&key, &plaintext);
    assert_ne!(ct, plaintext, "ciphertext must differ from plaintext");
    assert_eq!(ct.len(), plaintext.len() + 16, "ct must carry a 16-byte GCM tag");
    let recovered = decrypt_chunk(&key, &ct).expect("authentic ciphertext must decrypt");
    assert_eq!(recovered, plaintext);
}

#[test]
fn empty_plaintext_roundtrips() {
    let key = [0x42u8; 32];
    let ct = encrypt_chunk(&key, b"");
    assert_eq!(ct.len(), 16, "empty plaintext yields just the 16-byte tag");
    let recovered = decrypt_chunk(&key, &ct).expect("authentic empty ciphertext decrypts");
    assert!(recovered.is_empty());
}
```

- [ ] Run `cargo test -p digstore-crypto --test aead_roundtrip`. Expect FAIL to COMPILE: `error[E0432]: unresolved imports 'digstore_crypto::decrypt_chunk', 'digstore_crypto::encrypt_chunk'`.
- [ ] Create `crates/digstore-crypto/src/aead.rs` with this exact content:

```rust
use crate::error::TamperError;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};

/// Fixed 12-byte GCM nonce (paper §11.2). Safe ONLY under the unique-key-per-URN
/// invariant: each canonical URN derives a distinct key, so no key is ever
/// reused across two plaintexts. See crate-level docs.
const FIXED_NONCE: [u8; 12] = [0u8; 12];

/// Encrypt a chunk with AES-256-GCM under the per-URN `key`.
///
/// Returns `ciphertext || tag` (the `aes-gcm` crate appends the 16-byte tag).
pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher
        .encrypt(nonce, plaintext)
        .expect("AES-256-GCM encryption is infallible for in-memory plaintext")
}

/// Decrypt and authenticate a chunk. A failed GCM tag check is a tamper error.
pub fn decrypt_chunk(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, TamperError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&FIXED_NONCE);
    cipher.decrypt(nonce, ciphertext).map_err(|_| TamperError)
}
```

- [ ] In `src/lib.rs`, declare the module and re-export. After `pub mod fixtures;` add `pub mod aead;`, and add to the re-export block:

```rust
pub use aead::{decrypt_chunk, encrypt_chunk};
```

- [ ] Run `cargo test -p digstore-crypto --test aead_roundtrip`. Expect PASS: `test encrypt_then_decrypt_recovers_plaintext ... ok` and `test empty_plaintext_roundtrips ... ok`.
- [ ] Commit:
```
feat(crypto): AES-256-GCM encrypt_chunk/decrypt_chunk with fixed nonce
```

---

## Task 8 — AES-256-GCM tamper detection

**Files:**
- Modify: `crates/digstore-crypto/tests/aead_roundtrip.rs` (append)

Steps:

- [ ] Append the tamper tests:

```rust
use digstore_crypto::TamperError;

#[test]
fn flipping_a_ciphertext_byte_fails_authentication() {
    let key = [0x11u8; 32];
    let plaintext = b"sensitive payload".to_vec();
    let mut ct = encrypt_chunk(&key, &plaintext);
    ct[0] ^= 0x01; // index 0 is within the body for non-empty plaintext
    let err = decrypt_chunk(&key, &ct).unwrap_err();
    assert_eq!(err, TamperError);
}

#[test]
fn flipping_a_tag_byte_fails_authentication() {
    let key = [0x22u8; 32];
    let plaintext = b"sensitive payload".to_vec();
    let mut ct = encrypt_chunk(&key, &plaintext);
    let last = ct.len() - 1; // within the 16-byte tag
    ct[last] ^= 0x80;
    let err = decrypt_chunk(&key, &ct).unwrap_err();
    assert_eq!(err, TamperError);
}

#[test]
fn wrong_key_fails_authentication() {
    let key = [0x33u8; 32];
    let wrong = [0x44u8; 32];
    let ct = encrypt_chunk(&key, b"hello");
    let err = decrypt_chunk(&wrong, &ct).unwrap_err();
    assert_eq!(err, TamperError);
}

#[test]
fn truncated_ciphertext_fails() {
    let key = [0x55u8; 32];
    let ct = encrypt_chunk(&key, b"hello world");
    let truncated = &ct[..ct.len() - 4];
    let err = decrypt_chunk(&key, truncated).unwrap_err();
    assert_eq!(err, TamperError);
}
```

- [ ] Run `cargo test -p digstore-crypto --test aead_roundtrip`. Expect all six tests (two from Task 7, four new) PASS.
- [ ] Commit:
```
test(crypto): AES-256-GCM rejects tampered ciphertext, tag, wrong key, truncation
```

---

## Task 9 — BLS keygen, opaque signing key, and pubkey validation

Type-boundary decision (resolves the "leaked `chia_bls` types" defect): the secret key is wrapped in an opaque `HostSigningKey(chia_bls::SecretKey)`. `bls_keygen` returns `(HostSigningKey, Bytes48)`; `bls_sign` takes `&HostSigningKey`; no downstream crate ever names a `chia_bls` type. Public-key validation returns `Result<(), BlsError>` (not a leaked `chia_bls::PublicKey`).

Verified API (Task 0 probe): `SecretKey::from_seed(&[u8])`, `sk.public_key()`, `PublicKey::to_bytes() -> [u8;48]`, `PublicKey::from_bytes(&[u8;48]) -> Result`, `Signature::to_bytes() -> [u8;96]`, `Signature::from_bytes(&[u8;96]) -> Result`, free fns `chia_bls::sign(&sk, msg) -> Signature` and `chia_bls::verify(&sig, &pk, msg) -> bool`.

**Files:**
- Create: `crates/digstore-crypto/src/bls.rs`
- Create: `crates/digstore-crypto/tests/bls_roundtrip.rs`
- Modify: `crates/digstore-crypto/src/lib.rs`

Steps:

- [ ] Create `crates/digstore-crypto/tests/bls_roundtrip.rs` with this exact content (failing first):

```rust
use digstore_crypto::{bls_keygen, validate_public_key};

#[test]
fn keygen_is_deterministic_and_pubkey_validates() {
    let seed = [0xABu8; 32];
    let (_sk1, pk1) = bls_keygen(&seed);
    let (_sk2, pk2) = bls_keygen(&seed);
    assert_eq!(pk1, pk2, "same seed must yield same public key");
    // A meaningful check (length is a compile-time constant on Bytes48, so we
    // assert the key is canonical/parseable and not all-zero instead).
    assert_ne!(pk1.0, [0u8; 48], "public key must not be all-zero");
    assert!(validate_public_key(&pk1).is_ok(), "keygen output must be a valid G1 point");
}

#[test]
fn distinct_seeds_yield_distinct_pubkeys() {
    let (_s1, p1) = bls_keygen(&[0x01u8; 32]);
    let (_s2, p2) = bls_keygen(&[0x02u8; 32]);
    assert_ne!(p1, p2);
}

#[test]
fn validate_public_key_rejects_non_canonical_bytes() {
    use digstore_core::Bytes48;
    use digstore_crypto::BlsError;
    let bogus = Bytes48([0xFFu8; 48]);
    assert_eq!(validate_public_key(&bogus), Err(BlsError::InvalidPublicKey));
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip`. Expect FAIL to COMPILE: `error[E0432]: unresolved import 'digstore_crypto::bls_keygen'`.
- [ ] Create `crates/digstore-crypto/src/bls.rs` with this exact content (keygen + validation + the sign/verify primitives used by later tasks):

```rust
use crate::error::BlsError;
use chia_bls::{sign as aug_sign, verify as aug_verify, PublicKey, SecretKey, Signature};
use digstore_core::{Bytes32, Bytes48, Bytes96};

/// Opaque host signing key. Wraps `chia_bls::SecretKey` so that the `chia-bls`
/// type never crosses this crate's public boundary; downstream crates hold only
/// canonical `digstore-core` types plus this opaque handle.
pub struct HostSigningKey(SecretKey);

impl HostSigningKey {
    /// Deterministically derive a host signing key from a 32-byte seed.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        HostSigningKey(SecretKey::from_seed(seed))
    }

    /// The 48-byte G1 public key for this signing key.
    pub fn public_key(&self) -> Bytes48 {
        Bytes48(self.0.public_key().to_bytes())
    }
}

/// Deterministically derive a BLS keypair from a 32-byte seed (Chia keygen).
/// Returns the opaque signing key and the 48-byte G1 public key.
pub fn bls_keygen(seed: &[u8; 32]) -> (HostSigningKey, Bytes48) {
    let sk = HostSigningKey::from_seed(seed);
    let pk = sk.public_key();
    (sk, pk)
}

/// Sign `msg` under the Chia AugScheme (public key prepended, Chia DST).
/// Returns the 96-byte G2 signature.
pub fn bls_sign(sk: &HostSigningKey, msg: &[u8]) -> Bytes96 {
    let sig: Signature = aug_sign(&sk.0, msg);
    Bytes96(sig.to_bytes())
}

/// Verify a 96-byte AugScheme signature against a 48-byte public key and message.
/// Returns `false` on any malformed input or invalid signature.
pub fn bls_verify(pk: &Bytes48, msg: &[u8], sig: &Bytes96) -> bool {
    let pk = match PublicKey::from_bytes(&pk.0) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let sig = match Signature::from_bytes(&sig.0) {
        Ok(s) => s,
        Err(_) => return false,
    };
    aug_verify(&sig, &pk, msg)
}

/// Validate that `pk` is a canonical G1 public key, surfacing a typed error.
/// Returns `Ok(())` if parseable, `Err(BlsError::InvalidPublicKey)` otherwise.
/// Does not leak the `chia_bls::PublicKey` type across the crate boundary.
pub fn validate_public_key(pk: &Bytes48) -> Result<(), BlsError> {
    PublicKey::from_bytes(&pk.0)
        .map(|_| ())
        .map_err(|_| BlsError::InvalidPublicKey)
}

/// Canonical push-authorization signing message (paper §21.6):
/// `SHA-256(root) || store_id` (64 bytes). Exported so the remote crate verifies
/// with byte-identical input.
pub fn push_signing_message(root: &Bytes32, store_id: &Bytes32) -> Vec<u8> {
    let root_hash = crate::hash::sha256(&root.0);
    let mut msg = Vec::with_capacity(64);
    msg.extend_from_slice(&root_hash.0);
    msg.extend_from_slice(&store_id.0);
    msg
}

/// Canonical node execution-proof signing message (paper §13.7, §16).
///
/// Binds the attestation-relevant fields of `ExecutionProof` so the node
/// signature authenticates the program, the committed output, the block anchor,
/// AND the public input — not just `proof || public_input`:
///   `program_hash(32) || public_output(32) || chia_header_hash(32)
///    || height_be(4) || public_input(var)`
/// `height` is encoded big-endian (Chia-compat rule).
pub fn node_signing_message(
    program_hash: &Bytes32,
    public_output: &Bytes32,
    chia_header_hash: &Bytes32,
    height: u32,
    public_input: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(100 + public_input.len());
    msg.extend_from_slice(&program_hash.0);
    msg.extend_from_slice(&public_output.0);
    msg.extend_from_slice(&chia_header_hash.0);
    msg.extend_from_slice(&height.to_be_bytes());
    msg.extend_from_slice(public_input);
    msg
}

/// Canonical attestation signing message (paper §12):
/// `nonce(32) || store_id(32) || timestamp_be(8)` (72 bytes).
pub fn attestation_signing_message(
    nonce: &[u8; 32],
    store_id: &[u8; 32],
    timestamp: u64,
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(72);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(store_id);
    msg.extend_from_slice(&timestamp.to_be_bytes());
    msg
}
```

- [ ] In `src/lib.rs`, declare the module and re-export. After `pub mod aead;` add `pub mod bls;`, and add to the re-export block:

```rust
pub use bls::{
    attestation_signing_message, bls_keygen, bls_sign, bls_verify, node_signing_message,
    push_signing_message, validate_public_key, HostSigningKey,
};
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip`. Expect PASS: `keygen_is_deterministic_and_pubkey_validates ... ok`, `distinct_seeds_yield_distinct_pubkeys ... ok`, `validate_public_key_rejects_non_canonical_bytes ... ok`.
- [ ] Commit:
```
feat(crypto): opaque HostSigningKey, bls_keygen/sign/verify, signing-message builders
```

---

## Task 10 — BLS sign/verify round-trip and rejection paths

**Files:**
- Modify: `crates/digstore-crypto/tests/bls_roundtrip.rs` (append)

Steps:

- [ ] Append round-trip and rejection tests:

```rust
use digstore_crypto::{bls_sign, bls_verify};

#[test]
fn sign_then_verify_round_trip() {
    let (sk, pk) = bls_keygen(&[0x10u8; 32]);
    let msg = b"digstore execution proof payload";
    let sig = bls_sign(&sk, msg);
    assert!(bls_verify(&pk, msg, &sig), "valid signature must verify");
}

#[test]
fn verify_rejects_wrong_public_key() {
    let (sk, _pk) = bls_keygen(&[0x20u8; 32]);
    let (_sk2, other_pk) = bls_keygen(&[0x21u8; 32]);
    let msg = b"message";
    let sig = bls_sign(&sk, msg);
    assert!(!bls_verify(&other_pk, msg, &sig), "wrong key must not verify");
}

#[test]
fn verify_rejects_wrong_message() {
    let (sk, pk) = bls_keygen(&[0x30u8; 32]);
    let sig = bls_sign(&sk, b"original");
    assert!(!bls_verify(&pk, b"tampered", &sig), "altered message must not verify");
}

#[test]
fn verify_rejects_malformed_signature_bytes() {
    use digstore_core::Bytes96;
    let (_sk, pk) = bls_keygen(&[0x40u8; 32]);
    let bogus = Bytes96([0xFFu8; 96]);
    assert!(!bls_verify(&pk, b"x", &bogus), "non-canonical sig bytes must not verify");
}

#[test]
fn verify_rejects_malformed_public_key_bytes() {
    use digstore_core::{Bytes48, Bytes96};
    let (sk, _pk) = bls_keygen(&[0x41u8; 32]);
    let sig = bls_sign(&sk, b"x");
    let bogus_pk = Bytes48([0xFFu8; 48]);
    let sig96 = Bytes96(sig.0);
    assert!(!bls_verify(&bogus_pk, b"x", &sig96), "non-canonical pk bytes must not verify");
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip`. Expect all PASS (the `from_bytes` error path returning `false` is exercised by the two malformed-bytes tests — verified that `from_bytes(&[0xFF; N])` returns `Err`).
- [ ] Commit:
```
test(crypto): BLS round-trip plus wrong-key/message and malformed-bytes rejection
```

---

## Task 11 — Chia AugScheme known-answer conformance (real frozen vectors)

Locks AugScheme correctness against a real, computed Chia reference vector (NOT a placeholder, NOT a half-remembered prefix). For seed `[0,1,…,31]` the G1 pubkey is `8f336467…` and signing `[7,8,9]` yields the G2 signature `a5ce62a7…` — both produced by `chia-bls 0.45` and committed here as exact literals.

**Files:**
- Modify: `crates/digstore-crypto/tests/bls_roundtrip.rs` (append)

Steps:

- [ ] Append the conformance test with the real frozen hex literals:

```rust
#[test]
fn chia_aug_scheme_known_vector() {
    use digstore_core::{Bytes48, Bytes96};

    // Seed = [0, 1, 2, ..., 31].
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = i as u8;
    }
    let (sk, pk) = bls_keygen(&seed);

    // Real chia-bls 0.45 AugScheme reference values for this seed.
    let expected_pk = hex::decode(
        "8f336467f057b373bb3c43815a10ec131119d1bf50c14fa3f9ad86c0ec074f920f936a5315a8365a37fee0afa34c32c6",
    )
    .unwrap();
    assert_eq!(&pk.0[..], &expected_pk[..], "G1 pubkey must match Chia reference");

    let msg = [7u8, 8, 9];
    let sig = bls_sign(&sk, &msg);
    let expected_sig = hex::decode(
        "a5ce62a76c749a06c85b2d3762523b2e1d6756455767d2023967480433f7225c5cf42b3e14d0df41c0e6f9ecc18a39c30fdbfdbfd422945b478cc1675adf046aefbf4810e3ab9b0eb09855d3e5540cb0924e0f3d0e324bb59c59659b1c6b4283",
    )
    .unwrap();
    assert_eq!(&sig.0[..], &expected_sig[..], "AugScheme G2 sig must match Chia reference");

    // The frozen vector must self-verify through our verifier.
    let pk48 = Bytes48(expected_pk.try_into().unwrap());
    let sig96 = Bytes96(expected_sig.try_into().unwrap());
    assert!(bls_verify(&pk48, &msg, &sig96));
    // And must NOT verify a different message (binding sanity).
    assert!(!bls_verify(&pk48, &[9u8, 9, 9], &sig96));
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip chia_aug_scheme_known_vector`. Expect PASS: `... ok`.
- [ ] Commit:
```
test(crypto): freeze real Chia AugScheme known-answer vector (pubkey + signature)
```

---

## Task 12 — Cross-impl BLS parity fixtures (generate, freeze, document for guest)

Emits `(name, seed, msg, pk, sig)` vectors signed here with blst, plus the shared scheme tag `digstore_core::CHIA_BLS_SCHEME`, into `tests/fixtures/bls_parity.json` — the file `digstore-guest` loads and verifies with pure-Rust `bls12_381`. The write happens ONLY in the `gen_fixtures` example; the test compares the committed file to in-memory regeneration (no test-time write into the source tree).

**Files:**
- Modify: `crates/digstore-crypto/src/fixtures.rs` (append BLS half)
- Modify: `crates/digstore-crypto/src/lib.rs`
- Modify: `crates/digstore-crypto/examples/gen_fixtures.rs`
- Create: `crates/digstore-crypto/tests/bls_fixtures.rs`
- Create: `crates/digstore-crypto/tests/fixtures/bls_parity.json` (generated, committed)

Steps:

- [ ] Create `crates/digstore-crypto/tests/bls_fixtures.rs` with this exact content (failing first):

```rust
use digstore_crypto::fixtures::BlsFixtureSet;

#[test]
fn fixture_set_self_verifies_and_tags_scheme() {
    let set = BlsFixtureSet::generate();
    assert_eq!(set.scheme, digstore_core::CHIA_BLS_SCHEME, "scheme tag must be the shared const");
    assert!(!set.vectors.is_empty(), "must emit at least one vector");
    for v in &set.vectors {
        let pk = digstore_core::Bytes48(hex::decode(&v.pubkey_hex).unwrap().try_into().unwrap());
        let sig = digstore_core::Bytes96(hex::decode(&v.signature_hex).unwrap().try_into().unwrap());
        let msg = hex::decode(&v.message_hex).unwrap();
        assert!(
            digstore_crypto::bls_verify(&pk, &msg, &sig),
            "fixture '{}' must verify under blst signer side",
            v.name
        );
    }
}

#[test]
fn committed_bls_fixture_matches_generated() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bls_parity.json");
    let on_disk = std::fs::read_to_string(&path).expect(
        "committed bls_parity.json must exist; run: cargo run -p digstore-crypto --example gen_fixtures",
    );
    let parsed: BlsFixtureSet = serde_json::from_str(&on_disk).unwrap();
    let fresh = BlsFixtureSet::generate();

    assert_eq!(parsed.scheme, fresh.scheme);
    assert_eq!(parsed.vectors.len(), fresh.vectors.len());
    for (a, b) in parsed.vectors.iter().zip(fresh.vectors.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.seed_hex, b.seed_hex);
        assert_eq!(a.message_hex, b.message_hex);
        assert_eq!(a.pubkey_hex, b.pubkey_hex, "pubkey drift in '{}'", a.name);
        assert_eq!(a.signature_hex, b.signature_hex, "sig drift in '{}'", a.name);
    }
}

#[test]
fn write_path_is_idempotent_in_tempdir() {
    // Exercise write_bls_fixtures WITHOUT touching the source tree.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bls_parity.json");
    digstore_crypto::write_bls_fixtures(&path).expect("write fixtures to tempdir");
    let first = std::fs::read_to_string(&path).unwrap();
    digstore_crypto::write_bls_fixtures(&path).expect("rewrite is deterministic");
    let second = std::fs::read_to_string(&path).unwrap();
    assert_eq!(first, second, "fixture generation must be byte-stable");
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_fixtures`. Expect FAIL to COMPILE: `error[E0432]: unresolved import 'digstore_crypto::fixtures::BlsFixtureSet'`.
- [ ] Append the BLS half to `crates/digstore-crypto/src/fixtures.rs`:

```rust
use crate::{bls_keygen, bls_sign};

/// One cross-implementation parity vector: a message and the host-side (blst)
/// AugScheme public key + signature. The guest's pure-Rust `bls12_381` verifier
/// must accept every vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlsFixture {
    pub name: String,
    pub seed_hex: String,
    pub message_hex: String,
    pub pubkey_hex: String,
    pub signature_hex: String,
}

/// The full set of parity vectors, tagged with the shared scheme constant so
/// the guest asserts it is verifying the same scheme (Chia AugScheme).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlsFixtureSet {
    pub scheme: String,
    pub vectors: Vec<BlsFixture>,
}

impl BlsFixtureSet {
    /// Deterministically generate the canonical parity set.
    pub fn generate() -> Self {
        // (name, seed, message): empty, short, node-proof shape, push shape.
        let specs: &[(&str, [u8; 32], Vec<u8>)] = &[
            ("empty_message", [0x01; 32], vec![]),
            ("short_message", [0x02; 32], b"digstore".to_vec()),
            ("node_proof_shape", [0x03; 32], {
                let mut m = vec![0u8; 64];
                m.extend_from_slice(&[0xAB; 8]);
                m
            }),
            ("push_shape", [0x04; 32], {
                let mut m = vec![0x11; 32];
                m.extend_from_slice(&[0x22; 32]);
                m
            }),
        ];

        let mut vectors = Vec::with_capacity(specs.len());
        for (name, seed, msg) in specs {
            let (sk, pk) = bls_keygen(seed);
            let sig = bls_sign(&sk, msg);
            vectors.push(BlsFixture {
                name: name.to_string(),
                seed_hex: hex::encode(seed),
                message_hex: hex::encode(msg),
                pubkey_hex: hex::encode(pk.0),
                signature_hex: hex::encode(sig.0),
            });
        }

        BlsFixtureSet {
            scheme: digstore_core::CHIA_BLS_SCHEME.to_string(),
            vectors,
        }
    }
}

/// Generate the canonical parity set and write it as pretty JSON to `path`,
/// creating parent directories as needed. Called only by the gen_fixtures example.
pub fn write_bls_fixtures(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let set = BlsFixtureSet::generate();
    let json =
        serde_json::to_string_pretty(&set).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}
```

- [ ] In `src/lib.rs`, extend the fixtures re-export. Replace the existing line `pub use fixtures::{write_kdf_fixtures, KdfFixture, KdfFixtureSet};` with:

```rust
pub use fixtures::{
    write_bls_fixtures, write_kdf_fixtures, BlsFixture, BlsFixtureSet, KdfFixture, KdfFixtureSet,
};
```

- [ ] Extend `examples/gen_fixtures.rs` to also write the BLS file. Replace its `main` body with:

```rust
fn main() -> std::io::Result<()> {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    digstore_crypto::write_kdf_fixtures(&base.join("kdf_kat.json"))?;
    digstore_crypto::write_bls_fixtures(&base.join("bls_parity.json"))?;
    println!("wrote {}", base.join("kdf_kat.json").display());
    println!("wrote {}", base.join("bls_parity.json").display());
    Ok(())
}
```

- [ ] Generate the committed BLS fixture by running `cargo run -p digstore-crypto --example gen_fixtures`. Expect two `wrote ...` lines including `bls_parity.json`.
- [ ] Run `cargo test -p digstore-crypto --test bls_fixtures`. Expect all three tests PASS: `fixture_set_self_verifies_and_tags_scheme ... ok`, `committed_bls_fixture_matches_generated ... ok`, `write_path_is_idempotent_in_tempdir ... ok`.
- [ ] Confirm both fixture files are tracked: run `git status --short crates/digstore-crypto/tests/fixtures/`. Both `bls_parity.json` and `kdf_kat.json` must be staged/committed, not gitignored.
- [ ] Commit (include the generated fixture so `digstore-guest` consumes it without a build step):
```
feat(crypto): emit cross-impl BLS parity fixtures consumed by digstore-guest
```

---

## Task 13 — Push-authorization signing + verification (§21.6)

`sign_push` and `verify_push` both build their message from the single canonical `push_signing_message`, so the remote crate verifies with byte-identical input. Per the locked REST spec: publisher signs `SHA-256(root)` bound to `store_id`; remote verifies vs the store public key.

**Files:**
- Modify: `crates/digstore-crypto/src/bls.rs` (append `sign_push`, `verify_push`)
- Modify: `crates/digstore-crypto/src/lib.rs`
- Modify: `crates/digstore-crypto/tests/bls_roundtrip.rs` (append)

Steps:

- [ ] Append the failing binding test to `crates/digstore-crypto/tests/bls_roundtrip.rs`:

```rust
#[test]
fn sign_push_then_verify_push_round_trip_and_binding() {
    use digstore_core::Bytes32;
    use digstore_crypto::{push_signing_message, sha256, sign_push, verify_push};

    let (sk, pk) = bls_keygen(&[0x50u8; 32]);
    let root = Bytes32([0xAAu8; 32]);
    let store_id = Bytes32([0xBBu8; 32]);

    let sig = sign_push(&sk, &root, &store_id);
    assert!(verify_push(&pk, &root, &store_id, &sig), "push sig must verify with verify_push");

    // The exact signed message is SHA-256(root) || store_id.
    let mut expected = Vec::new();
    expected.extend_from_slice(&sha256(&root.0).0);
    expected.extend_from_slice(&store_id.0);
    assert_eq!(push_signing_message(&root, &store_id), expected);

    // Wrong store_id must not verify (binding to store).
    let other_store = Bytes32([0xCCu8; 32]);
    assert!(!verify_push(&pk, &root, &other_store, &sig));
    // Wrong root must not verify.
    let other_root = Bytes32([0xDDu8; 32]);
    assert!(!verify_push(&pk, &other_root, &store_id, &sig));
    // Signing over the RAW root (not its hash) would not verify.
    let mut raw = Vec::new();
    raw.extend_from_slice(&root.0);
    raw.extend_from_slice(&store_id.0);
    assert!(!digstore_crypto::bls_verify(&pk, &raw, &sig));
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip sign_push`. Expect FAIL to COMPILE: `error[E0432]: unresolved imports 'digstore_crypto::sign_push', 'digstore_crypto::verify_push'`.
- [ ] Append to `crates/digstore-crypto/src/bls.rs`:

```rust
/// Push-authorization signature (paper §21.6): sign `SHA-256(root) || store_id`
/// with the store's BLS key. Uses the canonical [`push_signing_message`].
pub fn sign_push(store_sk: &HostSigningKey, root: &Bytes32, store_id: &Bytes32) -> Bytes96 {
    bls_sign(store_sk, &push_signing_message(root, store_id))
}

/// Verify a push-authorization signature against the store public key, using the
/// byte-identical canonical message. The remote crate calls THIS to authorize a
/// push (paper §21.6, 401/403 on failure).
pub fn verify_push(
    store_pk: &Bytes48,
    root: &Bytes32,
    store_id: &Bytes32,
    sig: &Bytes96,
) -> bool {
    bls_verify(store_pk, &push_signing_message(root, store_id), sig)
}
```

- [ ] In `src/lib.rs`, extend the bls re-export to include `sign_push, verify_push`. Replace the `pub use bls::{ ... };` block with:

```rust
pub use bls::{
    attestation_signing_message, bls_keygen, bls_sign, bls_verify, node_signing_message,
    push_signing_message, sign_push, validate_public_key, verify_push, HostSigningKey,
};
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip sign_push`. Expect PASS: `... ok`.
- [ ] Commit:
```
feat(crypto): sign_push/verify_push over canonical SHA-256(root)||store_id (§21.6)
```

---

## Task 14 — Node execution-proof signing (§13.7) binding the full attestation

`sign_node` binds program_hash, public_output, the Chia block anchor (header_hash + big-endian height), and public_input — addressing the review finding that signing only `proof||public_input` omitted the anchor and committed output. The message is built by the canonical `node_signing_message`.

**Files:**
- Modify: `crates/digstore-crypto/src/bls.rs` (append `sign_node`)
- Modify: `crates/digstore-crypto/src/lib.rs`
- Modify: `crates/digstore-crypto/tests/bls_roundtrip.rs` (append)

Steps:

- [ ] Append the failing binding test:

```rust
#[test]
fn sign_node_binds_program_output_anchor_and_input() {
    use digstore_core::Bytes32;
    use digstore_crypto::{bls_verify, node_signing_message, sign_node};

    let (sk, pk) = bls_keygen(&[0x60u8; 32]);
    let program_hash = Bytes32([0x01u8; 32]);
    let public_output = Bytes32([0x02u8; 32]);
    let header_hash = Bytes32([0x03u8; 32]);
    let height: u32 = 0x00ABCDEF;
    let public_input = vec![9u8, 8, 7];

    let sig = sign_node(&sk, &program_hash, &public_output, &header_hash, height, &public_input);

    // Verifies against the canonical message.
    let msg = node_signing_message(&program_hash, &public_output, &header_hash, height, &public_input);
    assert!(bls_verify(&pk, &msg, &sig));

    // height is big-endian: a different height must not verify.
    let wrong_height = node_signing_message(&program_hash, &public_output, &header_hash, height + 1, &public_input);
    assert!(!bls_verify(&pk, &wrong_height, &sig));

    // Changing the bound output must not verify.
    let other_output = Bytes32([0x99u8; 32]);
    let wrong_out = node_signing_message(&program_hash, &other_output, &header_hash, height, &public_input);
    assert!(!bls_verify(&pk, &wrong_out, &sig));

    // Changing the anchor (header_hash) must not verify.
    let other_anchor = Bytes32([0x77u8; 32]);
    let wrong_anchor = node_signing_message(&program_hash, &public_output, &other_anchor, height, &public_input);
    assert!(!bls_verify(&pk, &wrong_anchor, &sig));
}

#[test]
fn node_signing_message_layout_is_exact() {
    use digstore_core::Bytes32;
    use digstore_crypto::node_signing_message;
    let pg = Bytes32([0x01u8; 32]);
    let out = Bytes32([0x02u8; 32]);
    let hdr = Bytes32([0x03u8; 32]);
    let height: u32 = 0x01020304;
    let pi = vec![0xEE, 0xFF];
    let msg = node_signing_message(&pg, &out, &hdr, height, &pi);
    // 32 + 32 + 32 + 4 + 2 = 102 bytes.
    assert_eq!(msg.len(), 102);
    assert_eq!(&msg[0..32], &[0x01u8; 32]);
    assert_eq!(&msg[32..64], &[0x02u8; 32]);
    assert_eq!(&msg[64..96], &[0x03u8; 32]);
    assert_eq!(&msg[96..100], &[0x01, 0x02, 0x03, 0x04]); // big-endian height
    assert_eq!(&msg[100..102], &[0xEE, 0xFF]);
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip sign_node`. Expect FAIL to COMPILE: `error[E0425]: cannot find function 'sign_node' in crate 'digstore_crypto'` (the layout test compiles because `node_signing_message` already exists from Task 9, but the file fails to build overall due to the missing `sign_node`).
- [ ] Append to `crates/digstore-crypto/src/bls.rs`:

```rust
/// Node execution-proof signature (paper §13.7, §16). Signs the canonical
/// [`node_signing_message`] binding program_hash, public_output, the Chia block
/// anchor (header_hash + big-endian height), and public_input. The verifier
/// (host/guest) reconstructs the same message from the `ExecutionProof` fields.
#[allow(clippy::too_many_arguments)]
pub fn sign_node(
    node_sk: &HostSigningKey,
    program_hash: &Bytes32,
    public_output: &Bytes32,
    chia_header_hash: &Bytes32,
    height: u32,
    public_input: &[u8],
) -> Bytes96 {
    let msg =
        node_signing_message(program_hash, public_output, chia_header_hash, height, public_input);
    bls_sign(node_sk, &msg)
}
```

- [ ] In `src/lib.rs`, add `sign_node` to the bls re-export. Replace the `pub use bls::{ ... };` block with:

```rust
pub use bls::{
    attestation_signing_message, bls_keygen, bls_sign, bls_verify, node_signing_message,
    push_signing_message, sign_node, sign_push, validate_public_key, verify_push, HostSigningKey,
};
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip`. Expect all PASS, including `sign_node_binds_program_output_anchor_and_input` and `node_signing_message_layout_is_exact`.
- [ ] Commit:
```
feat(crypto): sign_node binds program_hash/output/anchor/input (§13.7,§16)
```

---

## Task 15 — Host attestation signing (§12)

Provides the host-sign side of attestation: `sign_attestation(host_sk, &AttestationChallenge) -> Bytes96` over the canonical `attestation_signing_message` (`nonce || store_id || timestamp_be`). This makes the §12 coverage concrete and tested rather than a generic `bls_sign` claim.

**Files:**
- Modify: `crates/digstore-crypto/src/bls.rs` (append `sign_attestation`)
- Modify: `crates/digstore-crypto/src/lib.rs`
- Modify: `crates/digstore-crypto/tests/bls_roundtrip.rs` (append)

Steps:

- [ ] Append the failing attestation test:

```rust
#[test]
fn sign_attestation_binds_nonce_store_and_timestamp() {
    use digstore_core::AttestationChallenge;
    use digstore_crypto::{attestation_signing_message, bls_verify, sign_attestation};

    let (sk, pk) = bls_keygen(&[0x70u8; 32]);
    let challenge = AttestationChallenge {
        nonce: [0x5A; 32],
        store_id: [0x6B; 32],
        timestamp: 0x0102_0304_0506_0708,
    };

    let sig = sign_attestation(&sk, &challenge);

    let msg = attestation_signing_message(&challenge.nonce, &challenge.store_id, challenge.timestamp);
    assert!(bls_verify(&pk, &msg, &sig));

    // Layout: 32 + 32 + 8 = 72 bytes, timestamp big-endian.
    assert_eq!(msg.len(), 72);
    assert_eq!(&msg[0..32], &[0x5Au8; 32]);
    assert_eq!(&msg[32..64], &[0x6Bu8; 32]);
    assert_eq!(&msg[64..72], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);

    // A different nonce must not verify.
    let wrong = attestation_signing_message(&[0x00; 32], &challenge.store_id, challenge.timestamp);
    assert!(!bls_verify(&pk, &wrong, &sig));
}
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip sign_attestation`. Expect FAIL to COMPILE: `error[E0425]: cannot find function 'sign_attestation' in crate 'digstore_crypto'`.
- [ ] Append to `crates/digstore-crypto/src/bls.rs` (add the import for the challenge type at the top of the file's existing `use` block by replacing `use digstore_core::{Bytes32, Bytes48, Bytes96};` with `use digstore_core::{AttestationChallenge, Bytes32, Bytes48, Bytes96};`, then append the function):

```rust
/// Host attestation signature (paper §12). Signs the canonical
/// [`attestation_signing_message`] over the challenge's
/// `nonce || store_id || timestamp_be`. The 48-byte host public key and this
/// 96-byte signature populate `AttestationResponse`.
pub fn sign_attestation(host_sk: &HostSigningKey, challenge: &AttestationChallenge) -> Bytes96 {
    let msg = attestation_signing_message(
        &challenge.nonce,
        &challenge.store_id,
        challenge.timestamp,
    );
    bls_sign(host_sk, &msg)
}
```

- [ ] In `src/lib.rs`, add `sign_attestation` to the bls re-export. Replace the `pub use bls::{ ... };` block with:

```rust
pub use bls::{
    attestation_signing_message, bls_keygen, bls_sign, bls_verify, node_signing_message,
    push_signing_message, sign_attestation, sign_node, sign_push, validate_public_key,
    verify_push, HostSigningKey,
};
```

- [ ] Run `cargo test -p digstore-crypto --test bls_roundtrip sign_attestation`. Expect PASS: `... ok`.
- [ ] Commit:
```
feat(crypto): sign_attestation over nonce||store_id||timestamp_be (§12)
```

---

## Task 16 — CryptoError is exercised by a real API (`decrypt_and_unwrap`)

Resolves the "dead public surface" finding: `CryptoError` must be returned by at least one public function. We add `decrypt_and_unwrap`, a convenience that decrypts a chunk and validates a co-supplied public key, returning the unified `CryptoError` so callers (host) can handle AEAD and BLS failures with one type.

**Files:**
- Modify: `crates/digstore-crypto/src/lib.rs` (add `decrypt_and_unwrap`)
- Create: `crates/digstore-crypto/tests/crypto_error.rs`

Steps:

- [ ] Create `crates/digstore-crypto/tests/crypto_error.rs` with this exact content (failing first):

```rust
use digstore_crypto::{
    bls_keygen, decrypt_and_unwrap, encrypt_chunk, BlsError, CryptoError, TamperError,
};
use digstore_core::Bytes48;

#[test]
fn decrypt_and_unwrap_ok_path() {
    let key = [0x21u8; 32];
    let (_sk, pk) = bls_keygen(&[0x80u8; 32]);
    let ct = encrypt_chunk(&key, b"payload");
    let out = decrypt_and_unwrap(&key, &ct, &pk).expect("valid key + valid pk");
    assert_eq!(out, b"payload");
}

#[test]
fn decrypt_and_unwrap_surfaces_tamper_error() {
    let key = [0x21u8; 32];
    let (_sk, pk) = bls_keygen(&[0x80u8; 32]);
    let mut ct = encrypt_chunk(&key, b"payload");
    ct[0] ^= 0x01;
    let err = decrypt_and_unwrap(&key, &ct, &pk).unwrap_err();
    assert_eq!(err, CryptoError::Tamper(TamperError));
}

#[test]
fn decrypt_and_unwrap_surfaces_bls_error() {
    let key = [0x21u8; 32];
    let ct = encrypt_chunk(&key, b"payload");
    let bad_pk = Bytes48([0xFFu8; 48]);
    let err = decrypt_and_unwrap(&key, &ct, &bad_pk).unwrap_err();
    assert_eq!(err, CryptoError::Bls(BlsError::InvalidPublicKey));
}
```

- [ ] Run `cargo test -p digstore-crypto --test crypto_error`. Expect FAIL to COMPILE: `error[E0432]: unresolved import 'digstore_crypto::decrypt_and_unwrap'`.
- [ ] Add `decrypt_and_unwrap` to `src/lib.rs`, after the re-export block:

```rust
use digstore_core::Bytes48;

/// Decrypt a chunk AND validate the accompanying store/host public key in one
/// call, returning the unified [`CryptoError`]. The public key is validated
/// (canonical G1) before the plaintext is returned, so a caller that holds both
/// a ciphertext and an unverified key gets a single error type spanning AEAD
/// (`TamperError`) and BLS (`BlsError`) failures.
pub fn decrypt_and_unwrap(
    key: &[u8; 32],
    ciphertext: &[u8],
    public_key: &Bytes48,
) -> Result<Vec<u8>, CryptoError> {
    validate_public_key(public_key)?;
    let plaintext = decrypt_chunk(key, ciphertext)?;
    Ok(plaintext)
}
```

- [ ] Run `cargo test -p digstore-crypto --test crypto_error`. Expect all three tests PASS.
- [ ] Commit:
```
feat(crypto): decrypt_and_unwrap returns unified CryptoError (AEAD+BLS)
```

---

## Task 17 — Full crate gate: build, all tests, clippy, docs

**Files:** none created; verification only.

Steps:

- [ ] Run `cargo build -p digstore-crypto`. Expect `Finished`.
- [ ] Run `cargo build -p digstore-crypto --examples`. Expect `Finished` (the `gen_fixtures` example compiles).
- [ ] Run `cargo test -p digstore-crypto`. Expect a green summary across `hash_kat`, `kdf_kat`, `aead_roundtrip`, `bls_roundtrip`, `bls_fixtures`, and `crypto_error`: `test result: ok. N passed; 0 failed`.
- [ ] Re-run the generator and confirm the working tree is clean (proves committed fixtures match generation): run `cargo run -p digstore-crypto --example gen_fixtures` then `git status --short crates/digstore-crypto/tests/fixtures/`. Expect NO output (no diff). If a fixture file shows as modified, the committed copy is stale — re-commit the regenerated file.
- [ ] Run `cargo clippy -p digstore-crypto --all-targets -- -D warnings`. Fix any lints (e.g., needless borrows). Expect clean exit.
- [ ] Run `cargo doc -p digstore-crypto --no-deps`. Expect docs build with no broken intra-doc links.
- [ ] Commit (only if clippy/doc fixes were applied):
```
chore(crypto): satisfy clippy -D warnings and clean doc build
```

---

## Definition of Done

Each assigned paper section maps to concrete tasks; all checkboxes above are complete and `cargo test -p digstore-crypto` is green.

- [ ] **§11.1 Key derivation (HKDF-SHA256 from canonical URN):** Tasks 3, 4, 6 — `derive_decryption_key`, URN/retrieval-key bridge, frozen HKDF KAT fixture (committed, regenerated by `gen_fixtures`).
- [ ] **§11.2 AES-256-GCM with fixed nonce + tamper detection:** Tasks 7, 8 — `encrypt_chunk`/`decrypt_chunk`, GCM tag verification, `TamperError`. Deviation documented in code and crate docs; safety basis in Task 5.
- [ ] **§11.3 BLS (Chia AugScheme, G1 48B / G2 96B):** Tasks 9, 10, 11 — opaque `HostSigningKey`, `bls_keygen`/`bls_sign`/`bls_verify`, round-trip, rejection paths, and the REAL frozen Chia AugScheme conformance vector.
- [ ] **§11.4 Private-store key separation (SecretSalt mixing):** Tasks 4, 5, 6 — private-store salt derivation, public-vs-private and salt-vs-salt distinctness, frozen private KAT vectors.
- [ ] **§12 host-sign side (attestation signature):** Task 15 — `sign_attestation` over the canonical `attestation_signing_message` (`nonce || store_id || timestamp_be`), with exact-layout and binding tests; AugScheme conformance via Task 11.
- [ ] **§13.7 Node proof signature:** Task 14 — `sign_node` / `node_signing_message` binding program_hash, public_output, Chia anchor (header_hash + big-endian height), and public_input; exact-layout and four binding tests.
- [ ] **§21.6 Push signature:** Task 13 — `sign_push` / `verify_push` over the canonical `push_signing_message` (`SHA-256(root) || store_id`); shared message builder so the remote crate verifies byte-identically; round-trip and binding tests.
- [ ] **Cross-impl parity (cross-cutting):** Task 12 — `bls_parity.json` generated by blst, tagged with the shared `digstore_core::CHIA_BLS_SCHEME`, self-verified, frozen against regeneration, written only by the `gen_fixtures` example (tempdir for the write-path test), and documented for `digstore-guest`.
- [ ] **Unique-key-per-URN invariant (safety basis for §11.2 fixed nonce):** Task 5 — distinct URNs/salts derive distinct keys.
- [ ] **Type-boundary integrity:** Tasks 9, 16 — no `chia-bls` type crosses the public API (`HostSigningKey` is opaque; `validate_public_key` returns `Result<(), BlsError>`), and `CryptoError` is a live return type of `decrypt_and_unwrap`.
- [ ] **Full gate green:** Task 17 — build, examples, all tests, regeneration is a clean no-op, clippy `-D warnings`, docs.

---

## Plan metadata

- **Crate:** digstore-crypto
- **Assigned paper sections:** 11.1,11.2,11.3,11.4,12(host-sign side),13.7(node sig),21.6(push sig)
- **Depends on:** digstore-core
- **Spec sections covered (claimed):** 11.1, 11.2, 11.3, 11.4, 12, 13.7, 16, 21.6

### Public items exported (consumed by other crates)

```
pub fn sha256(data: &[u8]) -> digstore_core::Bytes32
pub const CRYPTO_VERSION: u32
pub fn derive_decryption_key(canonical_urn: &str, secret_salt: Option<digstore_core::SecretSalt>) -> [u8; 32]
pub fn encrypt_chunk(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8>
pub fn decrypt_chunk(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, digstore_crypto::TamperError>
pub fn decrypt_and_unwrap(key: &[u8; 32], ciphertext: &[u8], public_key: &digstore_core::Bytes48) -> Result<Vec<u8>, digstore_crypto::CryptoError>
pub struct HostSigningKey(/* opaque, wraps chia_bls::SecretKey */)
impl HostSigningKey { pub fn from_seed(seed: &[u8; 32]) -> HostSigningKey; pub fn public_key(&self) -> digstore_core::Bytes48 }
pub fn bls_keygen(seed: &[u8; 32]) -> (digstore_crypto::HostSigningKey, digstore_core::Bytes48)
pub fn bls_sign(sk: &digstore_crypto::HostSigningKey, msg: &[u8]) -> digstore_core::Bytes96
pub fn bls_verify(pk: &digstore_core::Bytes48, msg: &[u8], sig: &digstore_core::Bytes96) -> bool
pub fn validate_public_key(pk: &digstore_core::Bytes48) -> Result<(), digstore_crypto::BlsError>
pub fn push_signing_message(root: &digstore_core::Bytes32, store_id: &digstore_core::Bytes32) -> Vec<u8>
pub fn node_signing_message(program_hash: &digstore_core::Bytes32, public_output: &digstore_core::Bytes32, chia_header_hash: &digstore_core::Bytes32, height: u32, public_input: &[u8]) -> Vec<u8>
pub fn attestation_signing_message(nonce: &[u8; 32], store_id: &[u8; 32], timestamp: u64) -> Vec<u8>
pub fn sign_push(store_sk: &digstore_crypto::HostSigningKey, root: &digstore_core::Bytes32, store_id: &digstore_core::Bytes32) -> digstore_core::Bytes96
pub fn verify_push(store_pk: &digstore_core::Bytes48, root: &digstore_core::Bytes32, store_id: &digstore_core::Bytes32, sig: &digstore_core::Bytes96) -> bool
pub fn sign_node(node_sk: &digstore_crypto::HostSigningKey, program_hash: &digstore_core::Bytes32, public_output: &digstore_core::Bytes32, chia_header_hash: &digstore_core::Bytes32, height: u32, public_input: &[u8]) -> digstore_core::Bytes96
pub fn sign_attestation(host_sk: &digstore_crypto::HostSigningKey, challenge: &digstore_core::AttestationChallenge) -> digstore_core::Bytes96
pub struct TamperError; (Debug, Clone, PartialEq, Eq, thiserror::Error)
pub enum BlsError { InvalidPublicKey, InvalidSignature } (Debug, Clone, PartialEq, Eq, thiserror::Error)
pub enum CryptoError { Tamper(TamperError), Bls(BlsError) } (Debug, Clone, PartialEq, Eq, thiserror::Error)
pub struct BlsFixture { pub name: String, pub seed_hex: String, pub message_hex: String, pub pubkey_hex: String, pub signature_hex: String } (Serialize, Deserialize)
pub struct BlsFixtureSet { pub scheme: String, pub vectors: Vec<BlsFixture> } ; impl BlsFixtureSet { pub fn generate() -> BlsFixtureSet }
pub fn write_bls_fixtures(path: &std::path::Path) -> std::io::Result<()>
pub struct KdfFixture { pub name: String, pub canonical_urn: String, pub secret_salt_hex: Option<String>, pub key_hex: String } (Serialize, Deserialize)
pub struct KdfFixtureSet { pub crypto_version: u32, pub vectors: Vec<KdfFixture> } ; impl KdfFixtureSet { pub fn generate() -> KdfFixtureSet }
pub fn write_kdf_fixtures(path: &std::path::Path) -> std::io::Result<()>
```