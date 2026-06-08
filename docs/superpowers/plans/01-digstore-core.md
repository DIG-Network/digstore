# digstore-core Implementation Plan

> **For agentic workers**: Execute this plan using the **REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`**. Each task is a bite-sized TDD cycle: write a failing test (full code shown), run it and confirm the expected FAIL, write the minimal implementation (full code shown), run the test and confirm PASS, then commit with the exact conventional-commit message. Do not skip the red phase. Do not batch multiple tasks into one commit. Every code block below is complete and compilable — type it verbatim.

**Goal**: Implement the foundation crate `digstore-core` providing every canonical newtype, the Chia-streamable big-endian codec, URN parsing/canonicalization/retrieval-key derivation, the WASM ABI helpers + error codes, the Merkle tree + inclusion-proof verifier, and all wire/config structs shared across the Digstore workspace.

**Architecture**: `digstore-core` is `no_std + alloc` by default with an opt-in `std` feature for host convenience; it has zero workspace dependencies so the guest (`wasm32-unknown-unknown`) and host both link it. It exposes hand-written `Encode`/`Decode` traits (big-endian Chia framing) implemented for every primitive and every wire struct, plus pure-Rust SHA-256 (via `sha2`, which is `no_std`) used by URN retrieval keys and Merkle hashing. Everything downstream (guest, compiler, host, remote) consumes the exact type names and signatures defined here.

**Tech Stack**: Rust 1.94 (edition 2021), `no_std + alloc`; `sha2` (no_std, `default-features=false`) for SHA-256; `serde` (derive, alloc; optional, behind the `serde` cargo feature for hex serde on newtypes); `serde_json` (NON-optional, `default-features=false, features=["alloc"]`) so `serde_json::Value` is available in both no_std and std builds for `MetadataManifest.custom`; `hex` (no_std) for newtype hex. Tests use the standard `cargo test` harness (host with `std`).

---

## File Structure

All paths under `crates/digstore-core/`.

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest: `no_std`-friendly deps, `std`/`serde` cargo features, `[lib]` name `digstore_core`. |
| `src/lib.rs` | Crate root: `#![no_std]` + `extern crate alloc`, module declarations, public re-exports. |
| `src/error.rs` | `ErrorCode` enum (repr i32) + `CoreError` for codec/parse failures. |
| `src/codec/mod.rs` | `Encode`/`Decode` traits, `Encoder`/`Decoder` cursors, `DecodeError`, big-endian framing doc. |
| `src/codec/primitives.rs` | `Encode`/`Decode` impls for `u8/u16/u32/u64`, `Option<T>`, `Vec<T>`, `String`, fixed `[u8;N]`. |
| `src/codec/section.rs` | Data-section header: magic `DIGS`, `format_version`, offset table encode/decode. |
| `src/bytes.rs` | `Bytes32`/`Bytes48`/`Bytes96` newtypes: hex encode/decode, serde, codec. |
| `src/hash.rs` | `sha256(&[u8]) -> Bytes32` thin wrapper over `sha2`. |
| `src/abi.rs` | `pack_ptr_len`/`unpack_ptr_len`/`is_error` const fns. |
| `src/urn.rs` | `Urn` struct, parser, `canonical()`, `retrieval_key()`. |
| `src/merkle.rs` | `MerkleTree` build, `ProofStep`, `MerkleProof`, inclusion verify. |
| `src/keytable.rs` | `KeyTableEntry`, `PathWalk`. |
| `src/manifest.rs` | `MetadataManifest`, `Author`. |
| `src/wire.rs` | `ContentResponse`, `ProofResponse`, `ExecutionProof`, `ChiaBlockRef`, `AttestationChallenge`, `AttestationResponse`, `AuthenticationInfo`. |
| `src/config.rs` | `StoreConfig`, `Visibility`, `SecretSalt`, `GenerationState`, `Generation`, `GenerationId`, `ChunkerConfig`, `HostImportsConfig`, `TrustedHostKey`, `CompilationResult`, `CompilationStats`, `CompilerError`. |
| `tests/error.rs` | `ErrorCode` discriminants + `from_i32`. |
| `tests/abi.rs` | pack/unpack/is_error golden tests. |
| `tests/codec_primitives.rs` | Fixed byte-vector fixtures for every primitive. |
| `tests/bytes.rs` | Newtype hex/codec + sha256 KAT. |
| `tests/urn.rs` | Table-driven URN parse/canonicalize/retrieval-key. |
| `tests/section_header.rs` | Data-section header encode/decode. |
| `tests/merkle.rs` | Single/even/odd/1000-leaf build + proof accept/reject + proof size. |
| `tests/keytable.rs` | `KeyTableEntry`/`PathWalk` round-trip. |
| `tests/manifest.rs` | `MetadataManifest`/`Author` round-trip incl. `serde_json::Value` custom. |
| `tests/wire_proof.rs` | `ChiaBlockRef`/`ExecutionProof`/`ProofResponse` round-trip. |
| `tests/wire_content.rs` | `ContentResponse`/attestation/`AuthenticationInfo` round-trip. |
| `tests/config.rs` | Config/generation/compiler types. |
| `tests/codec_structs.rs` | Aggregate struct round-trip sweep + golden frame fixtures. |

---

## Task 1 — Crate skeleton (`no_std` + features)

**Files:**
- Create: `crates/digstore-core/Cargo.toml`
- Create: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/src/{abi,bytes,config,error,hash,keytable,manifest,merkle,urn,wire}.rs`
- Create: `crates/digstore-core/src/codec/{mod,primitives,section}.rs`
- Modify: `Cargo.toml` (workspace root)

Steps:

- [ ] Create the workspace root `Cargo.toml` (if it does not yet exist) with the member list:
```toml
[workspace]
resolver = "2"
members = ["crates/digstore-core"]

[workspace.package]
edition = "2021"
version = "0.1.0"
license = "MIT"
```
- [ ] Create `crates/digstore-core/Cargo.toml`. NOTE the feature wiring: `default` enables BOTH `std` and the local `serde` cargo feature; `std` only flips the `*/std` flags of dependencies; `serde` activates the optional `serde` dependency; `serde_json` is a NON-optional `alloc` dependency so `serde_json::Value` is always available (the guest's no_std build still gets it):
```toml
[package]
name = "digstore-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
name = "digstore_core"
path = "src/lib.rs"

[features]
default = ["std", "serde"]
std = ["sha2/std", "hex/std", "serde_json/std"]
serde = ["dep:serde"]

[dependencies]
sha2 = { version = "0.10", default-features = false }
hex = { version = "0.4", default-features = false, features = ["alloc"] }
serde_json = { version = "1", default-features = false, features = ["alloc"] }
serde = { version = "1", default-features = false, features = ["derive", "alloc"], optional = true }
```
- [ ] Create `crates/digstore-core/src/lib.rs` with the no_std root and module declarations. For THIS task only, omit the `pub use` re-export block (re-exports are added incrementally as each module is implemented):
```rust
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

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
```
- [ ] Create the eleven leaf module files as stubs so the crate compiles: each of `src/abi.rs`, `src/bytes.rs`, `src/config.rs`, `src/error.rs`, `src/hash.rs`, `src/keytable.rs`, `src/manifest.rs`, `src/merkle.rs`, `src/urn.rs`, `src/wire.rs` containing only `//! stub`. Create `src/codec/mod.rs` containing:
```rust
//! stub
pub mod primitives;
pub mod section;
```
- [ ] Create `src/codec/primitives.rs` and `src/codec/section.rs`, each containing only `//! stub`.
- [ ] Run `cargo build -p digstore-core`. Expected: `Compiling digstore-core v0.1.0 ...` then `Finished \`dev\` profile`.
- [ ] Add the `wasm32-unknown-unknown` target if not present: `rustup target add wasm32-unknown-unknown`. Then verify the no_std wasm build: `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core Cargo.toml && git commit -m "chore(core): scaffold digstore-core no_std crate skeleton"`

---

## Task 2 — `ErrorCode` enum and `CoreError`

**Files:**
- Modify: `crates/digstore-core/src/error.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/error.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/error.rs`:
```rust
use digstore_core::ErrorCode;

#[test]
fn error_code_discriminants_match_spec() {
    assert_eq!(ErrorCode::GeneralError as i32, -1);
    assert_eq!(ErrorCode::InvalidParameter as i32, -2);
    assert_eq!(ErrorCode::BufferTooSmall as i32, -3);
    assert_eq!(ErrorCode::NoSession as i32, -100);
    assert_eq!(ErrorCode::SessionExpired as i32, -101);
    assert_eq!(ErrorCode::AttestationFailed as i32, -102);
    assert_eq!(ErrorCode::NetworkError as i32, -200);
    assert_eq!(ErrorCode::Timeout as i32, -203);
    assert_eq!(ErrorCode::NotFound as i32, -300);
    assert_eq!(ErrorCode::ValidationFailed as i32, -301);
}

#[test]
fn error_code_from_i32_roundtrips() {
    for code in [
        ErrorCode::GeneralError,
        ErrorCode::NoSession,
        ErrorCode::Timeout,
        ErrorCode::ValidationFailed,
    ] {
        assert_eq!(ErrorCode::from_i32(code as i32), Some(code));
    }
    assert_eq!(ErrorCode::from_i32(42), None);
}
```
- [ ] Run `cargo test -p digstore-core --test error`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::ErrorCode\``.
- [ ] Implement `crates/digstore-core/src/error.rs`:
```rust
//! Error codes shared across the WASM ABI and core failures.

use alloc::string::String;

/// ABI error codes returned across the host/guest boundary (negative i32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    GeneralError = -1,
    InvalidParameter = -2,
    BufferTooSmall = -3,
    NoSession = -100,
    SessionExpired = -101,
    AttestationFailed = -102,
    NetworkError = -200,
    Timeout = -203,
    NotFound = -300,
    ValidationFailed = -301,
}

impl ErrorCode {
    /// Recover an `ErrorCode` from its i32 discriminant, if it is a known code.
    pub const fn from_i32(value: i32) -> Option<ErrorCode> {
        match value {
            -1 => Some(ErrorCode::GeneralError),
            -2 => Some(ErrorCode::InvalidParameter),
            -3 => Some(ErrorCode::BufferTooSmall),
            -100 => Some(ErrorCode::NoSession),
            -101 => Some(ErrorCode::SessionExpired),
            -102 => Some(ErrorCode::AttestationFailed),
            -200 => Some(ErrorCode::NetworkError),
            -203 => Some(ErrorCode::Timeout),
            -300 => Some(ErrorCode::NotFound),
            -301 => Some(ErrorCode::ValidationFailed),
            _ => None,
        }
    }
}

/// Library-level error for parsing/codec/validation failures inside the core crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// A string could not be parsed (URN, hex, etc.).
    Parse(String),
    /// A codec decode failed.
    Decode(String),
    /// A value failed validation.
    Validation(String),
}
```
- [ ] Add re-export to `lib.rs` (insert after the `pub mod wire;` line):
```rust
pub use error::{CoreError, ErrorCode};
```
- [ ] Run `cargo test -p digstore-core --test error`. Expected PASS: `test result: ok. 2 passed; 0 failed`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add ErrorCode enum and CoreError"`

---

## Task 3 — ABI pack/unpack/is_error

**Files:**
- Modify: `crates/digstore-core/src/abi.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/abi.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/abi.rs`:
```rust
use digstore_core::{is_error, pack_ptr_len, unpack_ptr_len};

#[test]
fn pack_unpack_roundtrip() {
    let cases = [(0u32, 0u32), (1, 64), (0x0001_0000, 0xFFFF), (0xDEAD_BEEF, 1024)];
    for (ptr, len) in cases {
        let packed = pack_ptr_len(ptr, len);
        assert_eq!(unpack_ptr_len(packed), (ptr, len));
    }
}

#[test]
fn pack_layout_is_high_ptr_low_len() {
    // ptr=1, len=2 => (1<<32)|2
    assert_eq!(pack_ptr_len(1, 2), (1i64 << 32) | 2);
}

#[test]
fn is_error_sentinel() {
    // Error sentinel: len == 0 && (ptr as i32) < 0.
    // ptr=0x8000_0000 has high bit set => negative i32, len 0 => error.
    let err = pack_ptr_len(0x8000_0000, 0);
    assert!(is_error(err));
    // A normal zero-length success at ptr=0 is NOT an error.
    assert!(!is_error(pack_ptr_len(0, 0)));
    // Non-zero length is never an error regardless of ptr.
    assert!(!is_error(pack_ptr_len(0x8000_0000, 5)));
    // An ErrorCode packed as ptr with len 0 is an error (codes are negative i32).
    let code_packed = pack_ptr_len((-1i32) as u32, 0);
    assert!(is_error(code_packed));
}
```
- [ ] Run `cargo test -p digstore-core --test abi`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::pack_ptr_len\``.
- [ ] Implement `crates/digstore-core/src/abi.rs`:
```rust
//! WASM ABI helpers: pack/unpack a (ptr, len) pair into an i64 return value.

/// Pack a 32-bit pointer and length into a single i64: `(ptr << 32) | len`.
pub const fn pack_ptr_len(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | (len as i64)
}

/// Split a packed i64 back into `(ptr, len)`.
pub const fn unpack_ptr_len(packed: i64) -> (u32, u32) {
    let ptr = (packed >> 32) as u32;
    let len = (packed & 0xFFFF_FFFF) as u32;
    (ptr, len)
}

/// An error sentinel has `len == 0` and the pointer reinterpreted as i32 is negative.
pub const fn is_error(packed: i64) -> bool {
    let (ptr, len) = unpack_ptr_len(packed);
    len == 0 && (ptr as i32) < 0
}
```
- [ ] Add re-export to `lib.rs` (after the `pub use error::{...};` line):
```rust
pub use abi::{is_error, pack_ptr_len, unpack_ptr_len};
```
- [ ] Run `cargo test -p digstore-core --test abi`. Expected PASS: `test result: ok. 3 passed; 0 failed`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add ABI pack/unpack/is_error helpers"`

---

## Task 4 — Codec traits and cursors

**Files:**
- Modify: `crates/digstore-core/src/codec/mod.rs`
- Modify: `crates/digstore-core/src/lib.rs`

Steps:

- [ ] Replace `crates/digstore-core/src/codec/mod.rs` (currently the stub + two `pub mod` lines) with the trait + cursor definitions. Note the module doc records **documented deviation 1**: the codec is BIG-ENDIAN (Chia STREAMABLE), not the paper's little-endian note:
```rust
//! Chia-streamable codec: BIG-ENDIAN fixed-width framing.
//!
//! DOCUMENTED DEVIATION 1: This codec is BIG-ENDIAN (Chia STREAMABLE),
//! NOT the paper's little-endian note. Chia compatibility wins.
//!
//! Framing rules (Chia STREAMABLE):
//! - `uintN`/`intN`: fixed-width big-endian.
//! - `Option<T>`: 1 tag byte (0=None, 1=Some) then `T`.
//! - `Vec<T>`: 4-byte BE count, then each item.
//! - `String`: 4-byte BE byte-length, then UTF-8 bytes.
//! - `Bytes32/48/96`: raw bytes, no length prefix.

pub mod primitives;
pub mod section;

use alloc::string::String;
use alloc::vec::Vec;

/// Error produced while decoding a byte stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Reached end of input before the value was fully read.
    UnexpectedEof,
    /// A tag/discriminant byte was out of range.
    InvalidTag(u8),
    /// UTF-8 validation failed for a String.
    InvalidUtf8,
    /// A magic / version / structural mismatch.
    Invalid(&'static str),
}

/// Append-only big-endian writer.
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder { buf: Vec::new() }
    }
    /// Write raw bytes with no length prefix.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
    /// Consume the encoder and return the accumulated bytes.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
    /// Current length of the buffer (used by the section offset table).
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// Forward-only big-endian reader over a borrowed slice.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Decoder { data, pos: 0 }
    }
    /// Read exactly `n` raw bytes.
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }
    /// Bytes remaining.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

/// Serialize `self` into an `Encoder` using big-endian Chia framing.
pub trait Encode {
    fn encode(&self, enc: &mut Encoder);
    /// Convenience: encode into a fresh `Vec<u8>`.
    fn to_bytes(&self) -> Vec<u8> {
        let mut enc = Encoder::new();
        self.encode(&mut enc);
        enc.finish()
    }
}

/// Deserialize `Self` from a `Decoder`.
pub trait Decode: Sized {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError>;
    /// Convenience: decode from a complete byte slice (does not require full consumption).
    fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut dec = Decoder::new(bytes);
        Self::decode(&mut dec)
    }
}

/// Helper used by `String`/manifest decode to validate UTF-8 in no_std.
pub(crate) fn utf8_from(bytes: &[u8]) -> Result<String, DecodeError> {
    core::str::from_utf8(bytes)
        .map(|s| s.into())
        .map_err(|_| DecodeError::InvalidUtf8)
}
```
- [ ] Add re-export to `lib.rs` (after the `pub use abi::{...};` line):
```rust
pub use codec::{Decode, DecodeError, Decoder, Encode, Encoder};
```
- [ ] Run `cargo build -p digstore-core`. Expected: `Finished` (traits + cursors compile; `primitives.rs`/`section.rs` are still `//! stub`).
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add Encode/Decode codec traits and cursors"`

---

## Task 5 — Codec primitives with fixed byte fixtures

**Files:**
- Modify: `crates/digstore-core/src/codec/primitives.rs`
- Create: `crates/digstore-core/tests/codec_primitives.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/codec_primitives.rs` verbatim (integration tests compile as a separate `std` crate, so `Vec`, `String`, and `vec!` come from the std prelude — the only import needed is the codec traits):
```rust
use digstore_core::codec::{Decode, Encode};

#[test]
fn u8_fixture() {
    assert_eq!(0x07u8.to_bytes(), vec![0x07]);
    assert_eq!(u8::from_bytes(&[0xFF]).unwrap(), 0xFF);
}

#[test]
fn u16_be_fixture() {
    assert_eq!(0x0102u16.to_bytes(), vec![0x01, 0x02]);
    assert_eq!(u16::from_bytes(&[0xAB, 0xCD]).unwrap(), 0xABCD);
}

#[test]
fn u32_be_fixture() {
    assert_eq!(0x01020304u32.to_bytes(), vec![0x01, 0x02, 0x03, 0x04]);
    assert_eq!(u32::from_bytes(&[0x00, 0x00, 0x01, 0x00]).unwrap(), 256);
}

#[test]
fn u64_be_fixture() {
    assert_eq!(
        0x0102030405060708u64.to_bytes(),
        vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
    );
}

#[test]
fn option_none_is_zero_tag() {
    let v: Option<u32> = None;
    assert_eq!(v.to_bytes(), vec![0x00]);
}

#[test]
fn option_some_is_one_tag_then_value() {
    let v: Option<u32> = Some(0x01020304);
    assert_eq!(v.to_bytes(), vec![0x01, 0x01, 0x02, 0x03, 0x04]);
    assert_eq!(Option::<u32>::from_bytes(&[0x01, 0x00, 0x00, 0x00, 0x09]).unwrap(), Some(9));
    assert_eq!(Option::<u32>::from_bytes(&[0x00]).unwrap(), None);
}

#[test]
fn option_invalid_tag_rejected() {
    assert!(Option::<u32>::from_bytes(&[0x02]).is_err());
}

#[test]
fn vec_count_prefixed_be() {
    let v: Vec<u16> = vec![0x0102, 0x0304];
    // 4-byte BE count (2) then two u16.
    assert_eq!(v.to_bytes(), vec![0, 0, 0, 2, 0x01, 0x02, 0x03, 0x04]);
    assert_eq!(Vec::<u16>::from_bytes(&[0, 0, 0, 0]).unwrap(), Vec::<u16>::new());
}

#[test]
fn string_len_prefixed_utf8() {
    let s = String::from("dig");
    assert_eq!(s.to_bytes(), vec![0, 0, 0, 3, b'd', b'i', b'g']);
    assert_eq!(String::from_bytes(&[0, 0, 0, 0]).unwrap(), String::new());
}

#[test]
fn fixed_array_raw_no_prefix() {
    let a: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
    assert_eq!(a.to_bytes(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(<[u8; 4]>::from_bytes(&[1, 2, 3, 4]).unwrap(), [1, 2, 3, 4]);
}

#[test]
fn eof_is_error() {
    assert!(u32::from_bytes(&[0x00, 0x01]).is_err());
}
```
- [ ] Run `cargo test -p digstore-core --test codec_primitives`. Expected FAIL: `error[E0599]: no method named \`to_bytes\` found for type \`u8\`` (the `Encode` impls do not exist yet).
- [ ] Implement `crates/digstore-core/src/codec/primitives.rs` (replace the `//! stub`):
```rust
//! Big-endian Chia framing for primitive types.

use super::{utf8_from, Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::string::String;
use alloc::vec::Vec;

macro_rules! impl_uint {
    ($t:ty, $n:expr) => {
        impl Encode for $t {
            fn encode(&self, enc: &mut Encoder) {
                enc.write_bytes(&self.to_be_bytes());
            }
        }
        impl Decode for $t {
            fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = dec.read_bytes($n)?;
                let mut arr = [0u8; $n];
                arr.copy_from_slice(bytes);
                Ok(<$t>::from_be_bytes(arr))
            }
        }
    };
}

impl_uint!(u8, 1);
impl_uint!(u16, 2);
impl_uint!(u32, 4);
impl_uint!(u64, 8);

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, enc: &mut Encoder) {
        match self {
            None => enc.write_bytes(&[0u8]),
            Some(v) => {
                enc.write_bytes(&[1u8]);
                v.encode(enc);
            }
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let tag = dec.read_bytes(1)?[0];
        match tag {
            0 => Ok(None),
            1 => Ok(Some(T::decode(dec)?)),
            other => Err(DecodeError::InvalidTag(other)),
        }
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, enc: &mut Encoder) {
        (self.len() as u32).encode(enc);
        for item in self {
            item.encode(enc);
        }
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let count = u32::decode(dec)? as usize;
        let mut out = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            out.push(T::decode(dec)?);
        }
        Ok(out)
    }
}

impl Encode for String {
    fn encode(&self, enc: &mut Encoder) {
        let bytes = self.as_bytes();
        (bytes.len() as u32).encode(enc);
        enc.write_bytes(bytes);
    }
}

impl Decode for String {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let len = u32::decode(dec)? as usize;
        let bytes = dec.read_bytes(len)?;
        utf8_from(bytes)
    }
}

macro_rules! impl_fixed_array {
    ($n:expr) => {
        impl Encode for [u8; $n] {
            fn encode(&self, enc: &mut Encoder) {
                enc.write_bytes(self);
            }
        }
        impl Decode for [u8; $n] {
            fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = dec.read_bytes($n)?;
                let mut arr = [0u8; $n];
                arr.copy_from_slice(bytes);
                Ok(arr)
            }
        }
    };
}

impl_fixed_array!(4); // used by tests + small fixed fields
impl_fixed_array!(32);
impl_fixed_array!(48);
impl_fixed_array!(96);
```
- [ ] Run `cargo test -p digstore-core --test codec_primitives`. Expected PASS: `test result: ok. 11 passed; 0 failed`.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add big-endian codec primitive impls"`

---

## Task 6 — `Bytes32`/`Bytes48`/`Bytes96` newtypes + hash

**Files:**
- Modify: `crates/digstore-core/src/bytes.rs`
- Modify: `crates/digstore-core/src/hash.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/bytes.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/bytes.rs` (SHA-256 KATs confirmed against the reference: empty -> `e3b0c4...b855`, "abc" -> `ba7816bf...15ad`):
```rust
use digstore_core::bytes::{Bytes32, Bytes48, Bytes96};
use digstore_core::codec::{Decode, Encode};
use digstore_core::sha256;

#[test]
fn bytes32_hex_roundtrip() {
    let b = Bytes32([0xAB; 32]);
    let hex = b.to_hex();
    assert_eq!(hex.len(), 64);
    assert_eq!(Bytes32::from_hex(&hex).unwrap(), b);
}

#[test]
fn bytes32_from_hex_rejects_wrong_length() {
    assert!(Bytes32::from_hex("abcd").is_err());
}

#[test]
fn bytes32_codec_is_raw_32_bytes() {
    let b = Bytes32([7u8; 32]);
    let enc = b.to_bytes();
    assert_eq!(enc.len(), 32);
    assert_eq!(enc, vec![7u8; 32]);
    assert_eq!(Bytes32::from_bytes(&enc).unwrap(), b);
}

#[test]
fn bytes48_and_96_codec_lengths() {
    assert_eq!(Bytes48([1u8; 48]).to_bytes().len(), 48);
    assert_eq!(Bytes96([2u8; 96]).to_bytes().len(), 96);
    assert_eq!(Bytes48::from_bytes(&[3u8; 48]).unwrap(), Bytes48([3u8; 48]));
    assert_eq!(Bytes96::from_bytes(&[4u8; 96]).unwrap(), Bytes96([4u8; 96]));
}

#[test]
fn sha256_known_answer_empty() {
    let out = sha256(b"");
    assert_eq!(
        out.to_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_known_answer_abc() {
    let out = sha256(b"abc");
    assert_eq!(
        out.to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
```
- [ ] Run `cargo test -p digstore-core --test bytes`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::bytes::Bytes32\``.
- [ ] Implement `crates/digstore-core/src/hash.rs`:
```rust
//! SHA-256 helper (no_std via the `sha2` crate).

use crate::bytes::Bytes32;
use sha2::{Digest, Sha256};

/// Compute SHA-256 over `data` and wrap it in a `Bytes32`.
pub fn sha256(data: &[u8]) -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    Bytes32(arr)
}
```
- [ ] Implement `crates/digstore-core/src/bytes.rs`:
```rust
//! Fixed-width byte newtypes with hex + codec + optional serde.

use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::error::CoreError;
use alloc::format;
use alloc::string::String;

macro_rules! bytes_newtype {
    ($name:ident, $n:expr) => {
        /// Fixed-width byte container (raw bytes on the wire, no length prefix).
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub [u8; $n]);

        impl $name {
            pub const LEN: usize = $n;

            /// Lowercase hex (no `0x` prefix).
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }

            /// Parse from lowercase/uppercase hex; must be exactly `2*LEN` chars.
            pub fn from_hex(s: &str) -> Result<Self, CoreError> {
                let bytes = hex::decode(s)
                    .map_err(|e| CoreError::Parse(format!("hex: {e}")))?;
                if bytes.len() != $n {
                    return Err(CoreError::Parse(format!(
                        "expected {} bytes, got {}",
                        $n,
                        bytes.len()
                    )));
                }
                let mut arr = [0u8; $n];
                arr.copy_from_slice(&bytes);
                Ok($name(arr))
            }

            pub fn as_bytes(&self) -> &[u8; $n] {
                &self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.to_hex())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name([0u8; $n])
            }
        }

        impl Encode for $name {
            fn encode(&self, enc: &mut Encoder) {
                enc.write_bytes(&self.0);
            }
        }

        impl Decode for $name {
            fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = dec.read_bytes($n)?;
                let mut arr = [0u8; $n];
                arr.copy_from_slice(bytes);
                Ok($name(arr))
            }
        }

        #[cfg(feature = "serde")]
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_hex())
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::from_hex(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

bytes_newtype!(Bytes32, 32);
bytes_newtype!(Bytes48, 48);
bytes_newtype!(Bytes96, 96);
```
- [ ] Add re-exports to `lib.rs` (after the `pub use codec::{...};` line):
```rust
pub use bytes::{Bytes32, Bytes48, Bytes96};
pub use hash::sha256;
```
- [ ] Run `cargo test -p digstore-core --test bytes`. Expected PASS: `test result: ok. 6 passed; 0 failed`.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add Bytes32/48/96 newtypes, hex, codec, sha256"`

---

## Task 7 — URN parse / canonicalize / retrieval_key (paper 6.1, 6.5)

**Files:**
- Modify: `crates/digstore-core/src/urn.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/urn.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/urn.rs` (table-driven, covers omitted rootHash / resourceKey + canonicalization + retrieval-key KAT):
```rust
use digstore_core::sha256;
use digstore_core::urn::Urn;
use digstore_core::Bytes32;

fn store_id() -> Bytes32 {
    Bytes32([0x11; 32])
}
fn root_hash() -> Bytes32 {
    Bytes32([0x22; 32])
}

#[test]
fn parse_full_urn() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let s = format!("urn:dig:mainnet:{sid}:{rh}/path/to/file.txt");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.chain, "mainnet");
    assert_eq!(urn.store_id, store_id());
    assert_eq!(urn.root_hash, Some(root_hash()));
    assert_eq!(urn.resource_key.as_deref(), Some("path/to/file.txt"));
}

#[test]
fn parse_omitted_roothash_and_resource() {
    let sid = store_id().to_hex();
    let s = format!("urn:dig:testnet:{sid}");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.chain, "testnet");
    assert_eq!(urn.root_hash, None);
    assert_eq!(urn.resource_key, None);
}

#[test]
fn parse_resource_without_roothash() {
    let sid = store_id().to_hex();
    let s = format!("urn:dig:mainnet:{sid}/readme.md");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.root_hash, None);
    assert_eq!(urn.resource_key.as_deref(), Some("readme.md"));
}

#[test]
fn parse_roothash_without_resource() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let s = format!("urn:dig:mainnet:{sid}:{rh}");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.root_hash, Some(root_hash()));
    assert_eq!(urn.resource_key, None);
}

#[test]
fn canonical_roundtrips_parse() {
    let sid = store_id().to_hex();
    let rh = root_hash().to_hex();
    let s = format!("urn:dig:mainnet:{sid}:{rh}/a/b");
    let urn = Urn::parse(&s).unwrap();
    assert_eq!(urn.canonical(), s);
    // Re-parsing the canonical form yields an equal URN.
    assert_eq!(Urn::parse(&urn.canonical()).unwrap(), urn);
}

#[test]
fn canonical_omits_absent_fields() {
    let sid = store_id().to_hex();
    let urn = Urn {
        chain: "mainnet".into(),
        store_id: store_id(),
        root_hash: None,
        resource_key: None,
    };
    assert_eq!(urn.canonical(), format!("urn:dig:mainnet:{sid}"));
}

#[test]
fn retrieval_key_is_sha256_of_canonical() {
    let sid = store_id().to_hex();
    let urn = Urn {
        chain: "mainnet".into(),
        store_id: store_id(),
        root_hash: None,
        resource_key: None,
    };
    let expected = sha256(format!("urn:dig:mainnet:{sid}").as_bytes());
    assert_eq!(urn.retrieval_key(), expected);
}

#[test]
fn parse_rejects_bad_scheme() {
    assert!(Urn::parse("urn:other:mainnet:00").is_err());
    assert!(Urn::parse("not-a-urn").is_err());
    assert!(Urn::parse("urn:dig:mainnet").is_err()); // missing store id
}

#[test]
fn parse_rejects_bad_store_id_hex() {
    assert!(Urn::parse("urn:dig:mainnet:zz").is_err());
}
```
- [ ] Run `cargo test -p digstore-core --test urn`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::urn::Urn\``.
- [ ] Implement `crates/digstore-core/src/urn.rs`:
```rust
//! URN parsing, canonicalization and retrieval-key derivation (paper 6.1, 6.5).
//!
//! Format: `urn:dig:<chain>:<storeID>[:<rootHash>][/<resourceKey>]`
//! - `retrieval_key = SHA-256(canonical())`

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::error::CoreError;
use crate::hash::sha256;
use alloc::format;
use alloc::string::{String, ToString};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Urn {
    pub chain: String,
    pub store_id: Bytes32,
    pub root_hash: Option<Bytes32>,
    pub resource_key: Option<String>,
}

impl Urn {
    /// Parse a URN string. Accepts omitted rootHash and/or resourceKey.
    pub fn parse(input: &str) -> Result<Urn, CoreError> {
        let rest = input
            .strip_prefix("urn:dig:")
            .ok_or_else(|| CoreError::Parse("missing 'urn:dig:' prefix".to_string()))?;

        // Split off the optional resource path at the FIRST '/'.
        let (head, resource_key) = match rest.split_once('/') {
            Some((h, r)) => (h, Some(r.to_string())),
            None => (rest, None),
        };

        // head = <chain>:<storeID>[:<rootHash>]
        let mut parts = head.split(':');
        let chain = parts
            .next()
            .filter(|c| !c.is_empty())
            .ok_or_else(|| CoreError::Parse("missing chain".to_string()))?
            .to_string();
        let store_id_hex = parts
            .next()
            .ok_or_else(|| CoreError::Parse("missing store id".to_string()))?;
        let store_id = Bytes32::from_hex(store_id_hex)?;
        let root_hash = match parts.next() {
            Some(rh) => Some(Bytes32::from_hex(rh)?),
            None => None,
        };
        if parts.next().is_some() {
            return Err(CoreError::Parse("too many ':' segments".to_string()));
        }

        Ok(Urn {
            chain,
            store_id,
            root_hash,
            resource_key,
        })
    }

    /// Render the canonical URN string.
    pub fn canonical(&self) -> String {
        let mut s = format!("urn:dig:{}:{}", self.chain, self.store_id.to_hex());
        if let Some(rh) = &self.root_hash {
            s.push(':');
            s.push_str(&rh.to_hex());
        }
        if let Some(rk) = &self.resource_key {
            s.push('/');
            s.push_str(rk);
        }
        s
    }

    /// `retrieval_key = SHA-256(canonical())`.
    pub fn retrieval_key(&self) -> Bytes32 {
        sha256(self.canonical().as_bytes())
    }
}

impl Encode for Urn {
    fn encode(&self, enc: &mut Encoder) {
        self.chain.encode(enc);
        self.store_id.encode(enc);
        self.root_hash.encode(enc);
        self.resource_key.encode(enc);
    }
}

impl Decode for Urn {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Urn {
            chain: String::decode(dec)?,
            store_id: Bytes32::decode(dec)?,
            root_hash: Option::<Bytes32>::decode(dec)?,
            resource_key: Option::<String>::decode(dec)?,
        })
    }
}
```
- [ ] Add re-export to `lib.rs` (after the `pub use hash::sha256;` line):
```rust
pub use urn::Urn;
```
- [ ] Run `cargo test -p digstore-core --test urn`. Expected PASS: `test result: ok. 9 passed; 0 failed`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add URN parse/canonical/retrieval_key (paper 6.1,6.5)"`

---

## Task 8 — Data-section header (magic DIGS, version, offset table)

**Files:**
- Modify: `crates/digstore-core/src/codec/section.rs`
- Create: `crates/digstore-core/tests/section_header.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/section_header.rs`:
```rust
use digstore_core::codec::section::{SectionEntry, SectionHeader, DIGS_MAGIC, FORMAT_VERSION};
use digstore_core::codec::{Decode, Encode};

#[test]
fn header_starts_with_magic_and_version() {
    let header = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![
            SectionEntry { id: 1, offset: 100, length: 50 },
            SectionEntry { id: 2, offset: 150, length: 25 },
        ],
    };
    let bytes = header.to_bytes();
    assert_eq!(&bytes[0..4], DIGS_MAGIC);
    assert_eq!(bytes[4], FORMAT_VERSION);
}

#[test]
fn header_offset_table_roundtrip() {
    let header = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![
            SectionEntry { id: 7, offset: 0, length: 4096 },
            SectionEntry { id: 9, offset: 4096, length: 1024 },
        ],
    };
    let bytes = header.to_bytes();
    let decoded = SectionHeader::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, header);
}

#[test]
fn header_rejects_bad_magic() {
    let mut bytes = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![],
    }
    .to_bytes();
    bytes[0] = b'X';
    assert!(SectionHeader::from_bytes(&bytes).is_err());
}

#[test]
fn header_rejects_unknown_version() {
    let mut bytes = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![],
    }
    .to_bytes();
    bytes[4] = 99;
    assert!(SectionHeader::from_bytes(&bytes).is_err());
}

#[test]
fn lookup_finds_section_by_id() {
    let header = SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![
            SectionEntry { id: 3, offset: 10, length: 20 },
            SectionEntry { id: 5, offset: 30, length: 40 },
        ],
    };
    assert_eq!(header.find(5), Some((30, 40)));
    assert_eq!(header.find(99), None);
}
```
- [ ] Run `cargo test -p digstore-core --test section_header`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::codec::section::SectionHeader\``.
- [ ] Implement `crates/digstore-core/src/codec/section.rs` (replace the `//! stub`):
```rust
//! Data-section header: magic `DIGS`, u8 format_version=1, then an offset table.

use super::{Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::vec::Vec;

/// Magic bytes at the start of a Digstore data section.
pub const DIGS_MAGIC: &[u8; 4] = b"DIGS";
/// Current data-section format version.
pub const FORMAT_VERSION: u8 = 1;

/// One entry in the section offset table: a logical section id plus its
/// byte offset and length within the surrounding data blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionEntry {
    pub id: u32,
    pub offset: u64,
    pub length: u64,
}

impl Encode for SectionEntry {
    fn encode(&self, enc: &mut Encoder) {
        self.id.encode(enc);
        self.offset.encode(enc);
        self.length.encode(enc);
    }
}

impl Decode for SectionEntry {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(SectionEntry {
            id: u32::decode(dec)?,
            offset: u64::decode(dec)?,
            length: u64::decode(dec)?,
        })
    }
}

/// Data-section header: magic + version + offset table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionHeader {
    pub format_version: u8,
    pub entries: Vec<SectionEntry>,
}

impl SectionHeader {
    /// Look up `(offset, length)` for a section id.
    pub fn find(&self, id: u32) -> Option<(u64, u64)> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| (e.offset, e.length))
    }
}

impl Encode for SectionHeader {
    fn encode(&self, enc: &mut Encoder) {
        enc.write_bytes(DIGS_MAGIC);
        self.format_version.encode(enc);
        self.entries.encode(enc); // Vec<SectionEntry>: 4-byte BE count then entries
    }
}

impl Decode for SectionHeader {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let magic = dec.read_bytes(4)?;
        if magic != DIGS_MAGIC {
            return Err(DecodeError::Invalid("bad DIGS magic"));
        }
        let format_version = u8::decode(dec)?;
        if format_version != FORMAT_VERSION {
            return Err(DecodeError::Invalid("unknown format version"));
        }
        let entries = Vec::<SectionEntry>::decode(dec)?;
        Ok(SectionHeader {
            format_version,
            entries,
        })
    }
}
```
- [ ] Run `cargo test -p digstore-core --test section_header`. Expected PASS: `test result: ok. 5 passed; 0 failed`.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add DIGS data-section header + offset table"`

---

## Task 9 — Merkle tree build + inclusion-proof verify (paper 7.1, 7.2, 7.3)

**Files:**
- Modify: `crates/digstore-core/src/merkle.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/merkle.rs`

Algorithm (canonical): `leaf = SHA-256(chunk)`; `node = SHA-256(left || right)`; an **odd node is carried up unchanged** to the next level; `root = generation root`. Proof path is bottom-up `ProofStep { hash, is_left }` where `is_left` marks whether the sibling is on the left.

Steps:

- [ ] Write failing test `crates/digstore-core/tests/merkle.rs`:
```rust
use digstore_core::merkle::MerkleTree;
use digstore_core::sha256;
use digstore_core::Bytes32;

fn chunks(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; 8]).collect()
}

#[test]
fn single_leaf_root_is_leaf_hash() {
    let data = vec![vec![1u8, 2, 3]];
    let tree = MerkleTree::build(&data);
    assert_eq!(tree.root(), sha256(&[1u8, 2, 3]));
}

#[test]
fn two_leaves_root_is_parent_hash() {
    let a = vec![0xAAu8];
    let b = vec![0xBBu8];
    let tree = MerkleTree::build(&[a.clone(), b.clone()]);
    let la = sha256(&a);
    let lb = sha256(&b);
    let mut cat = Vec::new();
    cat.extend_from_slice(&la.0);
    cat.extend_from_slice(&lb.0);
    assert_eq!(tree.root(), sha256(&cat));
}

#[test]
fn odd_leaf_is_carried_up() {
    // 3 leaves: level0 = [l0,l1,l2]; level1 = [h(l0||l1), l2]; root = h(level1_0 || l2).
    let data = chunks(3);
    let tree = MerkleTree::build(&data);
    let l: Vec<Bytes32> = data.iter().map(|c| sha256(c)).collect();
    let mut p01 = Vec::new();
    p01.extend_from_slice(&l[0].0);
    p01.extend_from_slice(&l[1].0);
    let n01 = sha256(&p01);
    let mut top = Vec::new();
    top.extend_from_slice(&n01.0);
    top.extend_from_slice(&l[2].0); // odd carried up unchanged
    assert_eq!(tree.root(), sha256(&top));
}

#[test]
fn inclusion_proof_accepts_each_leaf() {
    let data = chunks(8);
    let tree = MerkleTree::build(&data);
    for (i, c) in data.iter().enumerate() {
        let proof = tree.prove(i).unwrap();
        assert_eq!(proof.leaf, sha256(c));
        assert_eq!(proof.root, tree.root());
        assert!(proof.verify());
    }
}

#[test]
fn inclusion_proof_rejects_tampered_leaf() {
    let data = chunks(8);
    let tree = MerkleTree::build(&data);
    let mut proof = tree.prove(3).unwrap();
    proof.leaf = Bytes32([0xFF; 32]);
    assert!(!proof.verify());
}

#[test]
fn inclusion_proof_rejects_tampered_path() {
    let data = chunks(8);
    let tree = MerkleTree::build(&data);
    let mut proof = tree.prove(3).unwrap();
    proof.path[0].hash = Bytes32([0x00; 32]);
    assert!(!proof.verify());
}

#[test]
fn proof_size_is_ceil_log2_n() {
    // The proof for leaf index 0 always traverses the full left spine,
    // so its path length equals ceil(log2 n) for every n (carry rule included).
    for n in [1usize, 2, 3, 4, 5, 8, 16, 17, 1000] {
        let data = chunks(n);
        let tree = MerkleTree::build(&data);
        let proof = tree.prove(0).unwrap();
        assert_eq!(proof.path.len(), ceil_log2(n), "n={n}");
    }
}

#[test]
fn thousand_leaf_all_proofs_verify() {
    let data = chunks(1000);
    let tree = MerkleTree::build(&data);
    for i in (0..1000).step_by(37) {
        assert!(tree.prove(i).unwrap().verify());
    }
}

#[test]
fn prove_out_of_range_is_none() {
    let tree = MerkleTree::build(&chunks(4));
    assert!(tree.prove(4).is_none());
}

fn ceil_log2(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let mut levels = 0;
    let mut count = n;
    while count > 1 {
        count = (count + 1) / 2;
        levels += 1;
    }
    levels
}
```
- [ ] Run `cargo test -p digstore-core --test merkle`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::merkle::MerkleTree\``.
- [ ] Implement `crates/digstore-core/src/merkle.rs`:
```rust
//! Merkle tree build + inclusion proof verify (paper 7.1, 7.2, 7.3).
//!
//! - `leaf = SHA-256(chunk)`
//! - `node = SHA-256(left || right)`
//! - an odd node is carried up unchanged
//! - `root = generation root`

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::hash::sha256;
use alloc::vec::Vec;

/// One step on a bottom-up inclusion path: the sibling hash and whether that
/// sibling sits on the LEFT of the current node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofStep {
    pub hash: Bytes32,
    pub is_left: bool,
}

impl Encode for ProofStep {
    fn encode(&self, enc: &mut Encoder) {
        self.hash.encode(enc);
        (self.is_left as u8).encode(enc);
    }
}

impl Decode for ProofStep {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let hash = Bytes32::decode(dec)?;
        let flag = u8::decode(dec)?;
        let is_left = match flag {
            0 => false,
            1 => true,
            other => return Err(DecodeError::InvalidTag(other)),
        };
        Ok(ProofStep { hash, is_left })
    }
}

/// A complete inclusion proof from a leaf up to the generation root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf: Bytes32,
    pub path: Vec<ProofStep>,
    pub root: Bytes32,
}

impl MerkleProof {
    /// Recompute the root from `leaf` + `path` and compare to `root`.
    pub fn verify(&self) -> bool {
        let mut acc = self.leaf;
        for step in &self.path {
            acc = if step.is_left {
                hash_pair(&step.hash, &acc)
            } else {
                hash_pair(&acc, &step.hash)
            };
        }
        acc == self.root
    }
}

impl Encode for MerkleProof {
    fn encode(&self, enc: &mut Encoder) {
        self.leaf.encode(enc);
        self.path.encode(enc);
        self.root.encode(enc);
    }
}

impl Decode for MerkleProof {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(MerkleProof {
            leaf: Bytes32::decode(dec)?,
            path: Vec::<ProofStep>::decode(dec)?,
            root: Bytes32::decode(dec)?,
        })
    }
}

fn hash_pair(left: &Bytes32, right: &Bytes32) -> Bytes32 {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&left.0);
    buf[32..].copy_from_slice(&right.0);
    sha256(&buf)
}

/// A built Merkle tree retaining every level so proofs can be generated.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    /// `levels[0]` are leaves; the last level is the single-element root.
    levels: Vec<Vec<Bytes32>>,
}

impl MerkleTree {
    /// Build a tree from raw chunk byte-slices (`leaf = SHA-256(chunk)`).
    pub fn build(chunks: &[Vec<u8>]) -> MerkleTree {
        let leaves: Vec<Bytes32> = chunks.iter().map(|c| sha256(c)).collect();
        Self::from_leaves(leaves)
    }

    /// Build a tree directly from precomputed leaf hashes.
    pub fn from_leaves(leaves: Vec<Bytes32>) -> MerkleTree {
        let mut levels: Vec<Vec<Bytes32>> = Vec::new();
        let first = if leaves.is_empty() {
            // Empty tree: root is SHA-256 of nothing, kept as a single level.
            let mut v = Vec::new();
            v.push(sha256(&[]));
            v
        } else {
            leaves
        };
        levels.push(first);

        while levels.last().map(|l| l.len()).unwrap_or(0) > 1 {
            let prev = levels.last().unwrap();
            let mut next = Vec::with_capacity((prev.len() + 1) / 2);
            let mut i = 0;
            while i < prev.len() {
                if i + 1 < prev.len() {
                    next.push(hash_pair(&prev[i], &prev[i + 1]));
                    i += 2;
                } else {
                    // Odd node carried up unchanged.
                    next.push(prev[i]);
                    i += 1;
                }
            }
            levels.push(next);
        }
        MerkleTree { levels }
    }

    /// The generation root (last level, single element).
    pub fn root(&self) -> Bytes32 {
        *self.levels.last().unwrap().last().unwrap()
    }

    /// Number of leaves.
    pub fn leaf_count(&self) -> usize {
        self.levels[0].len()
    }

    /// Generate an inclusion proof for leaf `index`, or `None` if out of range.
    pub fn prove(&self, index: usize) -> Option<MerkleProof> {
        if index >= self.leaf_count() {
            return None;
        }
        let leaf = self.levels[0][index];
        let mut path = Vec::new();
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            if idx % 2 == 0 {
                // Right sibling exists unless this is a carried-up odd node.
                if idx + 1 < level.len() {
                    path.push(ProofStep {
                        hash: level[idx + 1],
                        is_left: false,
                    });
                }
                // else: node carried up unchanged, no step added.
            } else {
                // Left sibling always exists.
                path.push(ProofStep {
                    hash: level[idx - 1],
                    is_left: true,
                });
            }
            idx /= 2;
        }
        Some(MerkleProof {
            leaf,
            path,
            root: self.root(),
        })
    }
}
```
- [ ] Add re-export to `lib.rs` (after the `pub use urn::Urn;` line):
```rust
pub use merkle::{MerkleProof, MerkleTree, ProofStep};
```
- [ ] Run `cargo test -p digstore-core --test merkle`. Expected PASS: `test result: ok. 9 passed; 0 failed`. The `proof_size_is_ceil_log2_n` test proves only leaf index 0, whose path length equals `ceil(log2 n)` for every tested `n` under the carry rule; `thousand_leaf_all_proofs_verify` and `inclusion_proof_accepts_each_leaf` confirm all indices produce verifiable proofs.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add Merkle tree build + inclusion proof verify (paper 7.1-7.3)"`

---

## Task 10 — KeyTableEntry + PathWalk (paper 8.4)

**Files:**
- Modify: `crates/digstore-core/src/keytable.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/keytable.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/keytable.rs`:
```rust
use digstore_core::codec::{Decode, Encode};
use digstore_core::keytable::{KeyTableEntry, PathWalk};
use digstore_core::Bytes32;

#[test]
fn keytable_entry_roundtrip() {
    let e = KeyTableEntry {
        static_key: Bytes32([1; 32]),
        generation: Bytes32([2; 32]),
        chunk_indices: vec![0, 5, 9, 100],
        total_size: 4096,
    };
    let bytes = e.to_bytes();
    assert_eq!(KeyTableEntry::from_bytes(&bytes).unwrap(), e);
}

#[test]
fn keytable_entry_wire_layout() {
    let e = KeyTableEntry {
        static_key: Bytes32([0; 32]),
        generation: Bytes32([0; 32]),
        chunk_indices: vec![],
        total_size: 0,
    };
    let bytes = e.to_bytes();
    // 32 + 32 + 4(count=0) + 8(total_size) = 76 bytes
    assert_eq!(bytes.len(), 76);
}

#[test]
fn pathwalk_roundtrip_and_cursor() {
    let pw = PathWalk {
        resource_key: Bytes32([7; 32]),
        chunk_indices: vec![3, 4, 5],
        cursor: 1,
    };
    let bytes = pw.to_bytes();
    let decoded = PathWalk::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, pw);
    assert_eq!(decoded.cursor, 1);
}
```
- [ ] Run `cargo test -p digstore-core --test keytable`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::keytable::KeyTableEntry\``.
- [ ] Implement `crates/digstore-core/src/keytable.rs`:
```rust
//! Key-table entry and path-walk cursor (paper 8.4).

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::vec::Vec;

/// A key-table entry mapping a resource's static key + generation to its chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyTableEntry {
    pub static_key: Bytes32,
    pub generation: Bytes32,
    pub chunk_indices: Vec<u32>,
    pub total_size: u64,
}

impl Encode for KeyTableEntry {
    fn encode(&self, enc: &mut Encoder) {
        self.static_key.encode(enc);
        self.generation.encode(enc);
        self.chunk_indices.encode(enc);
        self.total_size.encode(enc);
    }
}

impl Decode for KeyTableEntry {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(KeyTableEntry {
            static_key: Bytes32::decode(dec)?,
            generation: Bytes32::decode(dec)?,
            chunk_indices: Vec::<u32>::decode(dec)?,
            total_size: u64::decode(dec)?,
        })
    }
}

/// A walk over a resource's chunk indices with a resumable cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathWalk {
    pub resource_key: Bytes32,
    pub chunk_indices: Vec<u32>,
    pub cursor: usize,
}

impl Encode for PathWalk {
    fn encode(&self, enc: &mut Encoder) {
        self.resource_key.encode(enc);
        self.chunk_indices.encode(enc);
        (self.cursor as u64).encode(enc);
    }
}

impl Decode for PathWalk {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let resource_key = Bytes32::decode(dec)?;
        let chunk_indices = Vec::<u32>::decode(dec)?;
        let cursor = u64::decode(dec)? as usize;
        Ok(PathWalk {
            resource_key,
            chunk_indices,
            cursor,
        })
    }
}
```
- [ ] Add re-export to `lib.rs` (after the `pub use merkle::{...};` line):
```rust
pub use keytable::{KeyTableEntry, PathWalk};
```
- [ ] Run `cargo test -p digstore-core --test keytable`. Expected PASS: `test result: ok. 3 passed; 0 failed`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add KeyTableEntry and PathWalk (paper 8.4)"`

---

## Task 11 — MetadataManifest + Author (paper 5.2 structs)

**Files:**
- Modify: `crates/digstore-core/src/manifest.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/manifest.rs`

Notes: `MetadataManifest.custom` is `BTreeMap<String, serde_json::Value>` UNCONDITIONALLY, matching the canonical catalog. `serde_json` is a non-optional `alloc` dependency, so `serde_json::Value`, `serde_json::to_string`, and `serde_json::from_str` are available in both the host (`std`) build and the guest (`no_std + alloc`) build. The `custom` map encodes each value as the JSON byte string of that value (length-prefixed UTF-8). `BTreeMap` gives deterministic iteration order so encoding is byte-stable. `links` is `BTreeMap<String, String>`.

Steps:

- [ ] Write failing test `crates/digstore-core/tests/manifest.rs` verbatim (integration test is a `std` crate; `BTreeMap` comes from `std::collections`, `serde_json` from the crate's non-optional dependency):
```rust
use digstore_core::codec::{Decode, Encode};
use digstore_core::manifest::{Author, MetadataManifest};
use std::collections::BTreeMap;

#[test]
fn manifest_roundtrip_minimal() {
    let m = MetadataManifest {
        schema_version: 1,
        name: "my-store".into(),
        version: None,
        description: None,
        authors: vec![],
        license: None,
        homepage: None,
        repository: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        content_type: None,
        links: BTreeMap::new(),
        custom: BTreeMap::new(),
    };
    let bytes = m.to_bytes();
    assert_eq!(MetadataManifest::from_bytes(&bytes).unwrap(), m);
}

#[test]
fn manifest_roundtrip_full() {
    let mut links = BTreeMap::new();
    links.insert("docs".to_string(), "https://example.com".to_string());
    let mut custom = BTreeMap::new();
    custom.insert("rating".to_string(), serde_json::json!(5));
    custom.insert("verified".to_string(), serde_json::json!(true));
    let m = MetadataManifest {
        schema_version: 2,
        name: "pkg".into(),
        version: Some("1.0.0".into()),
        description: Some("a package".into()),
        authors: vec![Author {
            name: "Alice".into(),
            handle: Some("@alice".into()),
            contact: None,
        }],
        license: Some("MIT".into()),
        homepage: Some("https://home".into()),
        repository: Some("https://repo".into()),
        keywords: vec!["a".into(), "b".into()],
        categories: vec!["tools".into()],
        icon: Some("icon.png".into()),
        content_type: Some("application/octet-stream".into()),
        links,
        custom,
    };
    let bytes = m.to_bytes();
    assert_eq!(MetadataManifest::from_bytes(&bytes).unwrap(), m);
}

#[test]
fn author_roundtrip() {
    let a = Author {
        name: "Bob".into(),
        handle: None,
        contact: Some("bob@x.io".into()),
    };
    let bytes = a.to_bytes();
    assert_eq!(Author::from_bytes(&bytes).unwrap(), a);
}
```
- [ ] Run `cargo test -p digstore-core --test manifest`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::manifest::MetadataManifest\`` (the manifest module is still `//! stub`).
- [ ] Implement `crates/digstore-core/src/manifest.rs`. The `custom` field is unconditionally `BTreeMap<String, serde_json::Value>`; under `std` the `BTreeMap` is `std::collections::BTreeMap`, otherwise `alloc::collections::BTreeMap` (these are the same type — `std` re-exports the `alloc` one):
```rust
//! Metadata manifest + author (paper 5.2 structs).
//!
//! `BTreeMap` is used so iteration/encode order is deterministic. `custom`
//! holds `serde_json::Value`, matching the canonical catalog exactly; the
//! `serde_json` dependency is non-optional (`alloc` feature) so `Value` is
//! available in both the host (`std`) and guest (`no_std + alloc`) builds.

use crate::codec::{utf8_from, Decode, DecodeError, Decoder, Encode, Encoder};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::collections::BTreeMap;
#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap;

/// One author of a store's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    pub handle: Option<String>,
    pub contact: Option<String>,
}

impl Encode for Author {
    fn encode(&self, enc: &mut Encoder) {
        self.name.encode(enc);
        self.handle.encode(enc);
        self.contact.encode(enc);
    }
}

impl Decode for Author {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Author {
            name: String::decode(dec)?,
            handle: Option::<String>::decode(dec)?,
            contact: Option::<String>::decode(dec)?,
        })
    }
}

/// Plaintext metadata manifest (NOT gated by session; served via get_metadata).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<Author>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    pub icon: Option<String>,
    pub content_type: Option<String>,
    pub links: BTreeMap<String, String>,
    pub custom: BTreeMap<String, serde_json::Value>,
}

/// Encode a `BTreeMap<String, String>` as 4-byte BE count then key/value strings.
fn encode_str_map(map: &BTreeMap<String, String>, enc: &mut Encoder) {
    (map.len() as u32).encode(enc);
    for (k, v) in map {
        k.encode(enc);
        v.encode(enc);
    }
}

fn decode_str_map(dec: &mut Decoder<'_>) -> Result<BTreeMap<String, String>, DecodeError> {
    let count = u32::decode(dec)? as usize;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let k = String::decode(dec)?;
        let v = String::decode(dec)?;
        map.insert(k, v);
    }
    Ok(map)
}

impl Encode for MetadataManifest {
    fn encode(&self, enc: &mut Encoder) {
        self.schema_version.encode(enc);
        self.name.encode(enc);
        self.version.encode(enc);
        self.description.encode(enc);
        self.authors.encode(enc);
        self.license.encode(enc);
        self.homepage.encode(enc);
        self.repository.encode(enc);
        self.keywords.encode(enc);
        self.categories.encode(enc);
        self.icon.encode(enc);
        self.content_type.encode(enc);
        encode_str_map(&self.links, enc);
        // custom: 4-byte BE count then (key string, json-text string).
        (self.custom.len() as u32).encode(enc);
        for (k, v) in &self.custom {
            k.encode(enc);
            let json = serde_json::to_string(v).unwrap_or_else(|_| "null".to_string());
            json.encode(enc);
        }
    }
}

impl Decode for MetadataManifest {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let schema_version = u32::decode(dec)?;
        let name = String::decode(dec)?;
        let version = Option::<String>::decode(dec)?;
        let description = Option::<String>::decode(dec)?;
        let authors = Vec::<Author>::decode(dec)?;
        let license = Option::<String>::decode(dec)?;
        let homepage = Option::<String>::decode(dec)?;
        let repository = Option::<String>::decode(dec)?;
        let keywords = Vec::<String>::decode(dec)?;
        let categories = Vec::<String>::decode(dec)?;
        let icon = Option::<String>::decode(dec)?;
        let content_type = Option::<String>::decode(dec)?;
        let links = decode_str_map(dec)?;

        let custom_count = u32::decode(dec)? as usize;
        let mut custom = BTreeMap::new();
        for _ in 0..custom_count {
            let k = String::decode(dec)?;
            let len = u32::decode(dec)? as usize;
            let raw = dec.read_bytes(len)?;
            let s = utf8_from(raw)?;
            let value: serde_json::Value =
                serde_json::from_str(&s).map_err(|_| DecodeError::Invalid("bad json"))?;
            custom.insert(k, value);
        }

        Ok(MetadataManifest {
            schema_version,
            name,
            version,
            description,
            authors,
            license,
            homepage,
            repository,
            keywords,
            categories,
            icon,
            content_type,
            links,
            custom,
        })
    }
}
```
- [ ] Add re-export to `lib.rs` (after the `pub use keytable::{...};` line):
```rust
pub use manifest::{Author, MetadataManifest};
```
- [ ] Run `cargo test -p digstore-core --test manifest`. Expected PASS: `test result: ok. 3 passed; 0 failed`.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished` (proves `serde_json::Value` + `to_string`/`from_str` work in the guest's no_std build).
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add MetadataManifest and Author (paper 5.2)"`

---

## Task 12 — Wire structs part A: ChiaBlockRef, ExecutionProof, ProofResponse (paper 9.1, 9.2)

**Files:**
- Modify: `crates/digstore-core/src/wire.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/wire_proof.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/wire_proof.rs`:
```rust
use digstore_core::codec::{Decode, Encode};
use digstore_core::wire::{ChiaBlockRef, ExecutionProof, ProofResponse};
use digstore_core::{Bytes32, Bytes48, Bytes96};

fn sample_proof() -> ExecutionProof {
    ExecutionProof {
        program_hash: Bytes32([1; 32]),
        public_input: vec![1, 2, 3],
        public_output: Bytes32([2; 32]),
        proof: vec![9, 9, 9, 9],
        chia_block: ChiaBlockRef {
            header_hash: Bytes32([3; 32]),
            height: 42,
            timestamp: 1_700_000_000,
        },
        node_pubkey: Bytes48([4; 48]),
        node_signature: Bytes96([5; 96]),
    }
}

#[test]
fn chia_block_ref_roundtrip() {
    let b = ChiaBlockRef {
        header_hash: Bytes32([7; 32]),
        height: 1000,
        timestamp: 1_650_000_000,
    };
    assert_eq!(ChiaBlockRef::from_bytes(&b.to_bytes()).unwrap(), b);
}

#[test]
fn execution_proof_roundtrip() {
    let p = sample_proof();
    assert_eq!(ExecutionProof::from_bytes(&p.to_bytes()).unwrap(), p);
}

#[test]
fn proof_response_roundtrip() {
    let r = ProofResponse {
        proof: sample_proof(),
        roothash: Bytes32([8; 32]),
    };
    assert_eq!(ProofResponse::from_bytes(&r.to_bytes()).unwrap(), r);
}
```
- [ ] Run `cargo test -p digstore-core --test wire_proof`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::wire::ExecutionProof\``.
- [ ] Implement the first half of `crates/digstore-core/src/wire.rs` (replace the `//! stub`):
```rust
//! Wire structs shared across host/guest/remote (paper 9.1, 9.2, 9.3, 9.5).

use crate::bytes::{Bytes32, Bytes48, Bytes96};
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::merkle::MerkleProof;
use alloc::string::String;
use alloc::vec::Vec;

/// Reference to a Chia block used to anchor a proof in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChiaBlockRef {
    pub header_hash: Bytes32,
    pub height: u32,
    pub timestamp: u64,
}

impl Encode for ChiaBlockRef {
    fn encode(&self, enc: &mut Encoder) {
        self.header_hash.encode(enc);
        self.height.encode(enc);
        self.timestamp.encode(enc);
    }
}

impl Decode for ChiaBlockRef {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ChiaBlockRef {
            header_hash: Bytes32::decode(dec)?,
            height: u32::decode(dec)?,
            timestamp: u64::decode(dec)?,
        })
    }
}

/// A ZK execution proof of a faithful re-execution of the serving computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProof {
    pub program_hash: Bytes32,
    pub public_input: Vec<u8>,
    pub public_output: Bytes32,
    pub proof: Vec<u8>,
    pub chia_block: ChiaBlockRef,
    pub node_pubkey: Bytes48,
    pub node_signature: Bytes96,
}

impl Encode for ExecutionProof {
    fn encode(&self, enc: &mut Encoder) {
        self.program_hash.encode(enc);
        self.public_input.encode(enc);
        self.public_output.encode(enc);
        self.proof.encode(enc);
        self.chia_block.encode(enc);
        self.node_pubkey.encode(enc);
        self.node_signature.encode(enc);
    }
}

impl Decode for ExecutionProof {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ExecutionProof {
            program_hash: Bytes32::decode(dec)?,
            public_input: Vec::<u8>::decode(dec)?,
            public_output: Bytes32::decode(dec)?,
            proof: Vec::<u8>::decode(dec)?,
            chia_block: ChiaBlockRef::decode(dec)?,
            node_pubkey: Bytes48::decode(dec)?,
            node_signature: Bytes96::decode(dec)?,
        })
    }
}

/// Response carrying an execution proof plus the active root hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofResponse {
    pub proof: ExecutionProof,
    pub roothash: Bytes32,
}

impl Encode for ProofResponse {
    fn encode(&self, enc: &mut Encoder) {
        self.proof.encode(enc);
        self.roothash.encode(enc);
    }
}

impl Decode for ProofResponse {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ProofResponse {
            proof: ExecutionProof::decode(dec)?,
            roothash: Bytes32::decode(dec)?,
        })
    }
}
```
- [ ] Note: `Vec<u8>` uses the blanket `Vec<T: Encode>` impl from Task 5 (4-byte BE count then each `u8`), so `public_input`/`proof` encode correctly. The `use alloc::string::String;` line is unused until Task 13 — to avoid an unused-import warning under `-D warnings`, this import is added in Task 13 along with `ContentResponse`. Remove the `use alloc::string::String;` line from the code above for THIS task (re-add it in Task 13).
- [ ] Add re-export to `lib.rs` (after the `pub use manifest::{...};` line):
```rust
pub use wire::{ChiaBlockRef, ExecutionProof, ProofResponse};
```
- [ ] Run `cargo test -p digstore-core --test wire_proof`. Expected PASS: `test result: ok. 3 passed; 0 failed`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add ChiaBlockRef/ExecutionProof/ProofResponse wire structs (paper 9.1-9.2)"`

---

## Task 13 — Wire structs part B: ContentResponse + attestation + AuthenticationInfo (paper 9.3, 9.5)

**Files:**
- Modify: `crates/digstore-core/src/wire.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/wire_content.rs`

Steps:

- [ ] Write failing test `crates/digstore-core/tests/wire_content.rs`:
```rust
use digstore_core::codec::{Decode, Encode};
use digstore_core::merkle::{MerkleProof, ProofStep};
use digstore_core::wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ContentResponse,
};
use digstore_core::Bytes32;

fn sample_merkle_proof() -> MerkleProof {
    MerkleProof {
        leaf: Bytes32([1; 32]),
        path: vec![ProofStep { hash: Bytes32([2; 32]), is_left: true }],
        root: Bytes32([3; 32]),
    }
}

#[test]
fn content_response_roundtrip() {
    let r = ContentResponse {
        ciphertext: vec![10, 20, 30, 40],
        merkle_proof: sample_merkle_proof(),
        roothash: Bytes32([4; 32]),
    };
    assert_eq!(ContentResponse::from_bytes(&r.to_bytes()).unwrap(), r);
}

#[test]
fn attestation_challenge_roundtrip() {
    let c = AttestationChallenge {
        nonce: [1; 32],
        store_id: [2; 32],
        timestamp: 999,
    };
    assert_eq!(AttestationChallenge::from_bytes(&c.to_bytes()).unwrap(), c);
}

#[test]
fn attestation_response_roundtrip() {
    let r = AttestationResponse {
        host_public_key: [3; 48],
        host_instance_id: [4; 32],
        signature: [5; 96],
    };
    assert_eq!(AttestationResponse::from_bytes(&r.to_bytes()).unwrap(), r);
}

#[test]
fn authentication_info_roundtrip() {
    let a = AuthenticationInfo {
        requires_session: true,
        requires_jwt: false,
        jwks_url: Some("https://issuer/.well-known/jwks.json".into()),
        accepted_algorithms: vec!["RS256".into(), "ES256".into()],
    };
    assert_eq!(AuthenticationInfo::from_bytes(&a.to_bytes()).unwrap(), a);
}
```
- [ ] Run `cargo test -p digstore-core --test wire_content`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::wire::ContentResponse\``.
- [ ] In `crates/digstore-core/src/wire.rs`, re-add the `use alloc::string::String;` import (it is now used by `AuthenticationInfo`). The top imports of the file should read:
```rust
use crate::bytes::{Bytes32, Bytes48, Bytes96};
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::merkle::MerkleProof;
use alloc::string::String;
use alloc::vec::Vec;
```
- [ ] Append to `crates/digstore-core/src/wire.rs` (after the `ProofResponse` impls):
```rust
/// Content (or decoy) response. Decoy uses this exact shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentResponse {
    pub ciphertext: Vec<u8>,
    pub merkle_proof: MerkleProof,
    pub roothash: Bytes32,
}

impl Encode for ContentResponse {
    fn encode(&self, enc: &mut Encoder) {
        self.ciphertext.encode(enc);
        self.merkle_proof.encode(enc);
        self.roothash.encode(enc);
    }
}

impl Decode for ContentResponse {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(ContentResponse {
            ciphertext: Vec::<u8>::decode(dec)?,
            merkle_proof: MerkleProof::decode(dec)?,
            roothash: Bytes32::decode(dec)?,
        })
    }
}

/// Challenge issued to a host during attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationChallenge {
    pub nonce: [u8; 32],
    pub store_id: [u8; 32],
    pub timestamp: u64,
}

impl Encode for AttestationChallenge {
    fn encode(&self, enc: &mut Encoder) {
        enc.write_bytes(&self.nonce);
        enc.write_bytes(&self.store_id);
        self.timestamp.encode(enc);
    }
}

impl Decode for AttestationChallenge {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let nonce = <[u8; 32]>::decode(dec)?;
        let store_id = <[u8; 32]>::decode(dec)?;
        let timestamp = u64::decode(dec)?;
        Ok(AttestationChallenge {
            nonce,
            store_id,
            timestamp,
        })
    }
}

/// A host's signed attestation response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationResponse {
    pub host_public_key: [u8; 48],
    pub host_instance_id: [u8; 32],
    pub signature: [u8; 96],
}

impl Encode for AttestationResponse {
    fn encode(&self, enc: &mut Encoder) {
        enc.write_bytes(&self.host_public_key);
        enc.write_bytes(&self.host_instance_id);
        enc.write_bytes(&self.signature);
    }
}

impl Decode for AttestationResponse {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let host_public_key = <[u8; 48]>::decode(dec)?;
        let host_instance_id = <[u8; 32]>::decode(dec)?;
        let signature = <[u8; 96]>::decode(dec)?;
        Ok(AttestationResponse {
            host_public_key,
            host_instance_id,
            signature,
        })
    }
}

/// Authentication requirements advertised by a store (get_authentication_info).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationInfo {
    pub requires_session: bool,
    pub requires_jwt: bool,
    pub jwks_url: Option<String>,
    pub accepted_algorithms: Vec<String>,
}

impl Encode for AuthenticationInfo {
    fn encode(&self, enc: &mut Encoder) {
        (self.requires_session as u8).encode(enc);
        (self.requires_jwt as u8).encode(enc);
        self.jwks_url.encode(enc);
        self.accepted_algorithms.encode(enc);
    }
}

impl Decode for AuthenticationInfo {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let requires_session = u8::decode(dec)? != 0;
        let requires_jwt = u8::decode(dec)? != 0;
        let jwks_url = Option::<String>::decode(dec)?;
        let accepted_algorithms = Vec::<String>::decode(dec)?;
        Ok(AuthenticationInfo {
            requires_session,
            requires_jwt,
            jwks_url,
            accepted_algorithms,
        })
    }
}
```
- [ ] Add re-export to `lib.rs` (extend the existing `pub use wire::{...};` line to also export the new types — replace the line from Task 12 with):
```rust
pub use wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ChiaBlockRef, ContentResponse,
    ExecutionProof, ProofResponse,
};
```
- [ ] Run `cargo test -p digstore-core --test wire_content`. Expected PASS: `test result: ok. 4 passed; 0 failed`.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add ContentResponse/attestation/AuthenticationInfo wire structs (paper 9.3,9.5)"`

---

## Task 14 — Config + generation + chunker + host-imports + trusted-key + compilation-result types (paper 5.2 structs)

**Files:**
- Modify: `crates/digstore-core/src/config.rs`
- Modify: `crates/digstore-core/src/lib.rs`
- Create: `crates/digstore-core/tests/config.rs`

Notes: `Generation` holds a `MerkleTree`; `CompilationResult` holds a `PathBuf` (std-only) and is therefore gated behind `#[cfg(feature = "std")]`. `ChunkerConfig`/`HostImportsConfig`/`TrustedHostKey`/`StoreConfig`/`GenerationState`/`CompilationStats`/`CompilerError`/`Visibility`/`SecretSalt`/`GenerationId` are `no_std`-safe. `TrustedHostKey.public_key` and `AttestationResponse.host_public_key` are raw `[u8; 48]` per the catalog, so `config.rs` imports only `Bytes32`.

Steps:

- [ ] Write failing test `crates/digstore-core/tests/config.rs`:
```rust
use digstore_core::config::{
    ChunkerConfig, CompilerError, GenerationId, GenerationState, HostImportsConfig, SecretSalt,
    StoreConfig, TrustedHostKey, Visibility,
};
use digstore_core::Bytes32;

#[test]
fn chunker_config_defaults() {
    let c = ChunkerConfig::default();
    assert_eq!(c.min_size, 16 * 1024);
    assert_eq!(c.target_size, 64 * 1024);
    assert_eq!(c.max_size, 256 * 1024);
}

#[test]
fn host_imports_config_defaults() {
    let h = HostImportsConfig::default();
    assert_eq!(h.return_buffer_capacity, 64 * 1024);
    assert_eq!(h.max_return_buffer_size, 16 * 1024 * 1024);
    assert_eq!(h.max_random_bytes, 1024);
}

#[test]
fn visibility_variants() {
    let pubv = Visibility::Public;
    let privv = Visibility::Private(SecretSalt([9; 32]));
    assert_ne!(pubv, privv);
    match privv {
        Visibility::Private(SecretSalt(s)) => assert_eq!(s, [9; 32]),
        _ => panic!("expected private"),
    }
}

#[test]
fn store_config_constructs() {
    let cfg = StoreConfig {
        store_id: Bytes32([1; 32]),
        data_dir: "/var/dig".into(),
        max_size: 1024,
        visibility: Visibility::Public,
    };
    assert_eq!(cfg.max_size, 1024);
}

#[test]
fn generation_state_and_id() {
    let id: GenerationId = 7;
    let gs = GenerationState {
        id,
        root: Bytes32([2; 32]),
        timestamp: 100,
    };
    assert_eq!(gs.id, 7);
}

#[test]
fn trusted_host_key_label_form() {
    let key = TrustedHostKey {
        public_key: [3; 48],
        label: "dig-host-key-v1:deadbeef".into(),
    };
    assert!(key.label.starts_with("dig-host-key-v1:"));
}

#[test]
fn compiler_error_no_trusted_keys() {
    let e = CompilerError::NoTrustedKeys;
    assert_eq!(e, CompilerError::NoTrustedKeys);
}
```
- [ ] Run `cargo test -p digstore-core --test config`. Expected FAIL: `error[E0432]: unresolved import \`digstore_core::config::ChunkerConfig\``.
- [ ] Implement `crates/digstore-core/src/config.rs`:
```rust
//! Store/generation/compiler configuration types (paper 5.2 structs).

use crate::bytes::Bytes32;
use crate::merkle::MerkleTree;
use alloc::string::String;

#[cfg(feature = "std")]
use std::path::PathBuf;

/// 32-byte secret salt mixed into private-store key derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretSalt(pub [u8; 32]);

/// Store visibility: public, or private with a secret salt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private(SecretSalt),
}

/// Static configuration for a store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConfig {
    pub store_id: Bytes32,
    pub data_dir: String,
    pub max_size: u64,
    pub visibility: Visibility,
}

/// Logical generation identifier.
pub type GenerationId = u64;

/// The committed state of a generation (no tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationState {
    pub id: u64,
    pub root: Bytes32,
    pub timestamp: u64,
}

/// A full generation: its state plus the built Merkle tree.
#[derive(Debug, Clone)]
pub struct Generation {
    pub state: GenerationState,
    pub tree: MerkleTree,
}

/// Content-defined chunker configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkerConfig {
    pub min_size: usize,
    pub target_size: usize,
    pub max_size: usize,
    pub mask: u64,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        ChunkerConfig {
            min_size: 16 * 1024,
            target_size: 64 * 1024,
            max_size: 256 * 1024,
            // Mask with ~16 bits set to target ~64KiB average chunks.
            mask: 0xFFFF,
        }
    }
}

/// Host imports configuration / limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostImportsConfig {
    pub return_buffer_capacity: usize,
    pub max_return_buffer_size: usize,
    pub max_random_bytes: u32,
    pub host_version: String,
}

impl Default for HostImportsConfig {
    fn default() -> Self {
        HostImportsConfig {
            return_buffer_capacity: 64 * 1024,
            max_return_buffer_size: 16 * 1024 * 1024,
            max_random_bytes: 1024,
            host_version: String::new(),
        }
    }
}

/// A trusted host BLS public key with its label (`dig-host-key-v1:<hex>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedHostKey {
    pub public_key: [u8; 48],
    pub label: String,
}

/// Statistics produced by a compilation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompilationStats {
    pub chunk_count: u64,
    pub total_bytes: u64,
    pub generation_count: u64,
}

/// Result of compiling a store into a serving module.
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilationResult {
    pub store_id: Bytes32,
    pub roothash: Bytes32,
    pub output_path: PathBuf,
    pub output_size: u64,
    pub stats: CompilationStats,
}

/// Errors raised by the compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerError {
    NoTrustedKeys,
    Io(String),
    Validation(String),
}
```
- [ ] Add re-exports to `lib.rs` (after the `pub use wire::{...};` block):
```rust
pub use config::{
    ChunkerConfig, CompilationStats, CompilerError, Generation, GenerationId, GenerationState,
    HostImportsConfig, SecretSalt, StoreConfig, TrustedHostKey, Visibility,
};
#[cfg(feature = "std")]
pub use config::CompilationResult;
```
- [ ] Run `cargo test -p digstore-core --test config`. Expected PASS: `test result: ok. 7 passed; 0 failed`.
- [ ] Run `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished` (`CompilationResult` excluded under no_std; everything else compiles).
- [ ] Commit: `git add crates/digstore-core && git commit -m "feat(core): add store/generation/compiler config types (paper 5.2)"`

---

## Task 15 — Cross-cutting struct round-trip sweep + golden fixtures

**Files:**
- Create: `crates/digstore-core/tests/codec_structs.rs`

This task adds a single aggregate test file that round-trips every codec-bearing struct (regression guard) plus byte-exact golden fixtures for the two most security-sensitive frames (`ContentResponse` empty + `ExecutionProof` field order).

Steps:

- [ ] Write test `crates/digstore-core/tests/codec_structs.rs`:
```rust
use digstore_core::codec::section::{SectionEntry, SectionHeader, FORMAT_VERSION};
use digstore_core::codec::{Decode, Encode};
use digstore_core::keytable::{KeyTableEntry, PathWalk};
use digstore_core::merkle::{MerkleProof, ProofStep};
use digstore_core::urn::Urn;
use digstore_core::wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ChiaBlockRef, ContentResponse,
    ExecutionProof, ProofResponse,
};
use digstore_core::{Bytes32, Bytes48, Bytes96};

fn assert_roundtrip<T: Encode + Decode + PartialEq + core::fmt::Debug>(value: T) {
    let bytes = value.to_bytes();
    let decoded = T::from_bytes(&bytes).expect("decode");
    assert_eq!(decoded, value);
}

#[test]
fn all_structs_roundtrip() {
    assert_roundtrip(Urn {
        chain: "mainnet".into(),
        store_id: Bytes32([1; 32]),
        root_hash: Some(Bytes32([2; 32])),
        resource_key: Some("a/b".into()),
    });
    assert_roundtrip(SectionHeader {
        format_version: FORMAT_VERSION,
        entries: vec![SectionEntry { id: 1, offset: 0, length: 8 }],
    });
    assert_roundtrip(KeyTableEntry {
        static_key: Bytes32([3; 32]),
        generation: Bytes32([4; 32]),
        chunk_indices: vec![1, 2, 3],
        total_size: 99,
    });
    assert_roundtrip(PathWalk {
        resource_key: Bytes32([5; 32]),
        chunk_indices: vec![7],
        cursor: 0,
    });
    let mp = MerkleProof {
        leaf: Bytes32([6; 32]),
        path: vec![ProofStep { hash: Bytes32([7; 32]), is_left: false }],
        root: Bytes32([8; 32]),
    };
    assert_roundtrip(mp.clone());
    assert_roundtrip(ChiaBlockRef {
        header_hash: Bytes32([9; 32]),
        height: 1,
        timestamp: 2,
    });
    let ep = ExecutionProof {
        program_hash: Bytes32([10; 32]),
        public_input: vec![1, 2],
        public_output: Bytes32([11; 32]),
        proof: vec![3, 4],
        chia_block: ChiaBlockRef {
            header_hash: Bytes32([12; 32]),
            height: 5,
            timestamp: 6,
        },
        node_pubkey: Bytes48([13; 48]),
        node_signature: Bytes96([14; 96]),
    };
    assert_roundtrip(ep.clone());
    assert_roundtrip(ProofResponse { proof: ep, roothash: Bytes32([15; 32]) });
    assert_roundtrip(ContentResponse {
        ciphertext: vec![1, 2, 3],
        merkle_proof: mp,
        roothash: Bytes32([16; 32]),
    });
    assert_roundtrip(AttestationChallenge { nonce: [1; 32], store_id: [2; 32], timestamp: 3 });
    assert_roundtrip(AttestationResponse {
        host_public_key: [4; 48],
        host_instance_id: [5; 32],
        signature: [6; 96],
    });
    assert_roundtrip(AuthenticationInfo {
        requires_session: true,
        requires_jwt: true,
        jwks_url: None,
        accepted_algorithms: vec!["RS256".into()],
    });
}

#[test]
fn content_response_empty_golden() {
    let r = ContentResponse {
        ciphertext: vec![],
        merkle_proof: MerkleProof {
            leaf: Bytes32([0; 32]),
            path: vec![],
            root: Bytes32([0; 32]),
        },
        roothash: Bytes32([0; 32]),
    };
    let bytes = r.to_bytes();
    // ciphertext: 4-byte count(0) = 0,0,0,0
    // merkle_proof.leaf: 32 zero bytes
    // merkle_proof.path: 4-byte count(0) = 0,0,0,0
    // merkle_proof.root: 32 zero bytes
    // roothash: 32 zero bytes
    assert_eq!(bytes.len(), 4 + 32 + 4 + 32 + 32);
    assert_eq!(&bytes[0..4], &[0, 0, 0, 0]);
}

#[test]
fn execution_proof_field_order_golden() {
    let ep = ExecutionProof {
        program_hash: Bytes32([0xAA; 32]),
        public_input: vec![],
        public_output: Bytes32([0xBB; 32]),
        proof: vec![],
        chia_block: ChiaBlockRef {
            header_hash: Bytes32([0xCC; 32]),
            height: 0,
            timestamp: 0,
        },
        node_pubkey: Bytes48([0xDD; 48]),
        node_signature: Bytes96([0xEE; 96]),
    };
    let bytes = ep.to_bytes();
    // program_hash first.
    assert_eq!(&bytes[0..32], &[0xAA; 32]);
    // then public_input length (0) BE.
    assert_eq!(&bytes[32..36], &[0, 0, 0, 0]);
    // then public_output (0xBB * 32).
    assert_eq!(&bytes[36..68], &[0xBB; 32]);
}
```
- [ ] Run `cargo test -p digstore-core --test codec_structs`. Expected PASS: `test result: ok. 3 passed; 0 failed`.
- [ ] Commit: `git add crates/digstore-core && git commit -m "test(core): add aggregate struct round-trip + golden frame fixtures"`

---

## Task 16 — Full workspace verification gate

**Files:**
- Test: all of `crates/digstore-core/tests/*`

Steps:

- [ ] Run the full default-feature suite: `cargo test -p digstore-core`. Expected: every test binary reports `ok`; the final summary line for each suite reads `test result: ok. N passed; 0 failed`.
- [ ] Run clippy clean under default features: `cargo clippy -p digstore-core --all-targets -- -D warnings`. Expected: `Finished` with no warnings. If a lint fires (e.g. `clippy::vec_init_then_push` in `from_leaves`, needless clone), fix it minimally and re-run until clean.
- [ ] Run clippy under no-default-features too (catches guest-path warnings): `cargo clippy -p digstore-core --no-default-features --lib -- -D warnings`. Expected: `Finished` with no warnings.
- [ ] Run the no_std wasm build to confirm the guest can link core: `cargo build -p digstore-core --no-default-features --target wasm32-unknown-unknown`. Expected: `Finished`.
- [ ] Run `cargo build -p digstore-core --no-default-features` (no_std on host target, no wasm) to confirm the `alloc`-only path compiles. Expected: `Finished`.
- [ ] Commit (if any clippy fixes were applied): `git add crates/digstore-core && git commit -m "chore(core): clippy clean + no_std build gate"`

---

## Definition of Done

| Paper section | Covered by | Verified |
|---------------|------------|----------|
| 5.2 (structs) | Tasks 10, 11, 12, 13, 14 (KeyTableEntry, MetadataManifest/Author, all wire structs, StoreConfig/Generation/Chunker/HostImports/TrustedHostKey/CompilationResult) | `cargo test -p digstore-core --test keytable --test manifest --test wire_proof --test wire_content --test config` |
| 6.1 (URN format) | Task 7 | `cargo test -p digstore-core --test urn` |
| 6.5 (retrieval key) | Task 7 (`retrieval_key = SHA-256(canonical)`, KAT) | `cargo test -p digstore-core --test urn retrieval_key_is_sha256_of_canonical` |
| 7.1 (Merkle leaves/build) | Task 9 | `cargo test -p digstore-core --test merkle` |
| 7.2 (Merkle nodes/odd-carry) | Task 9 (`odd_leaf_is_carried_up`) | same |
| 7.3 (inclusion proofs) | Task 9 (accept/reject/size) | same |
| 8.4 (key table / path walk) | Task 10 | `cargo test -p digstore-core --test keytable` |
| 9.1 (execution proof) | Task 12 | `cargo test -p digstore-core --test wire_proof` |
| 9.2 (proof response / Chia anchor) | Task 12 (`ChiaBlockRef`, `ProofResponse`) | same |
| 9.3 (content response shape) | Task 13 (`ContentResponse`) | `cargo test -p digstore-core --test wire_content` |
| 9.5 (attestation / auth info) | Task 13 (`AttestationChallenge`/`AttestationResponse`/`AuthenticationInfo`) | same |

Additional crate-wide DoD checklist:

- [ ] All canonical newtypes (`Bytes32/48/96`) implemented with hex + serde + codec (Task 6).
- [ ] SHA-256 KAT (empty, "abc") verified (Task 6).
- [ ] Codec primitives big-endian with fixed byte fixtures: `u8/u16/u32/u64`, `Option`, `Vec`, `String`, fixed arrays (Task 5).
- [ ] Data-section header `DIGS` + version + offset table encode/decode (Task 8).
- [ ] ABI `pack_ptr_len`/`unpack_ptr_len`/`is_error` golden tests incl. error sentinel (Task 3).
- [ ] `ErrorCode` discriminants match the canonical catalog exactly (Task 2).
- [ ] `MetadataManifest.custom` is `BTreeMap<String, serde_json::Value>` UNCONDITIONALLY (matches catalog; verified by the no_std wasm build in Task 11).
- [ ] Every codec-bearing struct round-trips (Task 15 aggregate sweep).
- [ ] Crate builds `--no-default-features --target wasm32-unknown-unknown` so the guest can link it (Tasks 1, 4, 5, 6, 8, 9, 11, 13, 14, 16).
- [ ] **Documented deviation 1 stated**: the codec is BIG-ENDIAN (Chia STREAMABLE), not the paper's little-endian note — recorded in `src/codec/mod.rs` module doc.
- [ ] `cargo clippy -p digstore-core --all-targets -- -D warnings` and `--no-default-features --lib -- -D warnings` are both clean (Task 16).


---

## Plan metadata

- **Crate:** digstore-core
- **Assigned paper sections:** 6.1,6.5,7.1,7.2,7.3,8.4,9.1,9.2,9.3,9.5,5.2(structs)
- **Depends on:** none
- **Spec sections covered (claimed):** 5.2, 6.1, 6.5, 7.1, 7.2, 7.3, 8.4, 9.1, 9.2, 9.3, 9.5

### Public items exported (consumed by other crates)

```
pub struct Bytes32(pub [u8; 32]);
pub struct Bytes48(pub [u8; 48]);
pub struct Bytes96(pub [u8; 96]);
impl Bytes32 { pub const LEN: usize; pub fn to_hex(&self) -> alloc::string::String; pub fn from_hex(s: &str) -> Result<Bytes32, CoreError>; pub fn as_bytes(&self) -> &[u8; 32]; }
impl Bytes48 { pub const LEN: usize; pub fn to_hex(&self) -> alloc::string::String; pub fn from_hex(s: &str) -> Result<Bytes48, CoreError>; pub fn as_bytes(&self) -> &[u8; 48]; }
impl Bytes96 { pub const LEN: usize; pub fn to_hex(&self) -> alloc::string::String; pub fn from_hex(s: &str) -> Result<Bytes96, CoreError>; pub fn as_bytes(&self) -> &[u8; 96]; }
pub fn sha256(data: &[u8]) -> Bytes32;
pub const fn pack_ptr_len(ptr: u32, len: u32) -> i64;
pub const fn unpack_ptr_len(packed: i64) -> (u32, u32);
pub const fn is_error(packed: i64) -> bool;
#[repr(i32)] pub enum ErrorCode { GeneralError = -1, InvalidParameter = -2, BufferTooSmall = -3, NoSession = -100, SessionExpired = -101, AttestationFailed = -102, NetworkError = -200, Timeout = -203, NotFound = -300, ValidationFailed = -301 }
impl ErrorCode { pub const fn from_i32(value: i32) -> Option<ErrorCode>; }
pub enum CoreError { Parse(alloc::string::String), Decode(alloc::string::String), Validation(alloc::string::String) }
pub enum DecodeError { UnexpectedEof, InvalidTag(u8), InvalidUtf8, Invalid(&'static str) }
pub struct Encoder { /* private */ }
impl Encoder { pub fn new() -> Self; pub fn write_bytes(&mut self, bytes: &[u8]); pub fn finish(self) -> alloc::vec::Vec<u8>; pub fn len(&self) -> usize; pub fn is_empty(&self) -> bool; }
pub struct Decoder<'a> { /* private */ }
impl<'a> Decoder<'a> { pub fn new(data: &'a [u8]) -> Self; pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError>; pub fn position(&self) -> usize; pub fn remaining(&self) -> usize; }
pub trait Encode { fn encode(&self, enc: &mut Encoder); fn to_bytes(&self) -> alloc::vec::Vec<u8>; }
pub trait Decode: Sized { fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError>; fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError>; }
impl Encode for u8/u16/u32/u64; impl Decode for u8/u16/u32/u64;
impl<T: Encode> Encode for Option<T>; impl<T: Decode> Decode for Option<T>;
impl<T: Encode> Encode for alloc::vec::Vec<T>; impl<T: Decode> Decode for alloc::vec::Vec<T>;
impl Encode for alloc::string::String; impl Decode for alloc::string::String;
impl Encode for [u8; 4]/[u8; 32]/[u8; 48]/[u8; 96]; impl Decode for [u8; 4]/[u8; 32]/[u8; 48]/[u8; 96];
pub const DIGS_MAGIC: &[u8; 4] = b"DIGS";
pub const FORMAT_VERSION: u8 = 1;
pub struct SectionEntry { pub id: u32, pub offset: u64, pub length: u64 }
pub struct SectionHeader { pub format_version: u8, pub entries: alloc::vec::Vec<SectionEntry> }
impl SectionHeader { pub fn find(&self, id: u32) -> Option<(u64, u64)>; }
pub struct Urn { pub chain: alloc::string::String, pub store_id: Bytes32, pub root_hash: Option<Bytes32>, pub resource_key: Option<alloc::string::String> }
impl Urn { pub fn parse(input: &str) -> Result<Urn, CoreError>; pub fn canonical(&self) -> alloc::string::String; pub fn retrieval_key(&self) -> Bytes32; }
pub struct ProofStep { pub hash: Bytes32, pub is_left: bool }
pub struct MerkleProof { pub leaf: Bytes32, pub path: alloc::vec::Vec<ProofStep>, pub root: Bytes32 }
impl MerkleProof { pub fn verify(&self) -> bool; }
pub struct MerkleTree { /* private */ }
impl MerkleTree { pub fn build(chunks: &[alloc::vec::Vec<u8>]) -> MerkleTree; pub fn from_leaves(leaves: alloc::vec::Vec<Bytes32>) -> MerkleTree; pub fn root(&self) -> Bytes32; pub fn leaf_count(&self) -> usize; pub fn prove(&self, index: usize) -> Option<MerkleProof>; }
pub struct KeyTableEntry { pub static_key: Bytes32, pub generation: Bytes32, pub chunk_indices: alloc::vec::Vec<u32>, pub total_size: u64 }
pub struct PathWalk { pub resource_key: Bytes32, pub chunk_indices: alloc::vec::Vec<u32>, pub cursor: usize }
pub struct Author { pub name: alloc::string::String, pub handle: Option<alloc::string::String>, pub contact: Option<alloc::string::String> }
pub struct MetadataManifest { pub schema_version: u32, pub name: alloc::string::String, pub version: Option<alloc::string::String>, pub description: Option<alloc::string::String>, pub authors: alloc::vec::Vec<Author>, pub license: Option<alloc::string::String>, pub homepage: Option<alloc::string::String>, pub repository: Option<alloc::string::String>, pub keywords: alloc::vec::Vec<alloc::string::String>, pub categories: alloc::vec::Vec<alloc::string::String>, pub icon: Option<alloc::string::String>, pub content_type: Option<alloc::string::String>, pub links: BTreeMap<alloc::string::String, alloc::string::String>, pub custom: BTreeMap<alloc::string::String, serde_json::Value> }
pub struct ChiaBlockRef { pub header_hash: Bytes32, pub height: u32, pub timestamp: u64 }
pub struct ExecutionProof { pub program_hash: Bytes32, pub public_input: alloc::vec::Vec<u8>, pub public_output: Bytes32, pub proof: alloc::vec::Vec<u8>, pub chia_block: ChiaBlockRef, pub node_pubkey: Bytes48, pub node_signature: Bytes96 }
pub struct ProofResponse { pub proof: ExecutionProof, pub roothash: Bytes32 }
pub struct ContentResponse { pub ciphertext: alloc::vec::Vec<u8>, pub merkle_proof: MerkleProof, pub roothash: Bytes32 }
pub struct AttestationChallenge { pub nonce: [u8; 32], pub store_id: [u8; 32], pub timestamp: u64 }
pub struct AttestationResponse { pub host_public_key: [u8; 48], pub host_instance_id: [u8; 32], pub signature: [u8; 96] }
pub struct AuthenticationInfo { pub requires_session: bool, pub requires_jwt: bool, pub jwks_url: Option<alloc::string::String>, pub accepted_algorithms: alloc::vec::Vec<alloc::string::String> }
pub struct SecretSalt(pub [u8; 32]);
pub enum Visibility { Public, Private(SecretSalt) }
pub struct StoreConfig { pub store_id: Bytes32, pub data_dir: alloc::string::String, pub max_size: u64, pub visibility: Visibility }
pub type GenerationId = u64;
pub struct GenerationState { pub id: u64, pub root: Bytes32, pub timestamp: u64 }
pub struct Generation { pub state: GenerationState, pub tree: MerkleTree }
pub struct ChunkerConfig { pub min_size: usize, pub target_size: usize, pub max_size: usize, pub mask: u64 } // Default: 16K/64K/256K/0xFFFF
pub struct HostImportsConfig { pub return_buffer_capacity: usize, pub max_return_buffer_size: usize, pub max_random_bytes: u32, pub host_version: alloc::string::String } // Default: 64K/16M/1024/empty
pub struct TrustedHostKey { pub public_key: [u8; 48], pub label: alloc::string::String }
pub struct CompilationStats { pub chunk_count: u64, pub total_bytes: u64, pub generation_count: u64 }
#[cfg(feature = "std")] pub struct CompilationResult { pub store_id: Bytes32, pub roothash: Bytes32, pub output_path: std::path::PathBuf, pub output_size: u64, pub stats: CompilationStats }
pub enum CompilerError { NoTrustedKeys, Io(alloc::string::String), Validation(alloc::string::String) }
```