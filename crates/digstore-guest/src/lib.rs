//! # digstore-guest
//!
//! The served WASM logic. Documented deviations enforced here:
//! 1. Codec is BIG-ENDIAN (Chia streamable framing), NOT the paper's little-endian
//!    note (§5.3). Chia compatibility wins.
//! 2. Decoy/cover streams use deterministic ChaCha20 keyed by SHA-256 so identical
//!    inputs yield identical bytes (§19.3 determinism), interpreting the paper's
//!    "random filler" as "deterministically pseudo-random".
//! 3. The guest VERIFIES BLS with pure-Rust bls12_381 (AugScheme); it never signs
//!    and never decrypts. Node proof signatures are produced by the host.
//! 4. CONVENTIONS C3: `get_proof` returns a serialized `ProofPrelude`, NOT a
//!    finished `ExecutionProof` (the guest cannot make ZK proofs in wasm).
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod allocator;
pub mod host;

// Wasm-only ABI surface. Pure logic modules below are always compiled.
#[cfg(target_arch = "wasm32")]
pub mod abi;
#[cfg(target_arch = "wasm32")]
pub mod data_stub;
#[cfg(target_arch = "wasm32")]
pub mod imports;

pub mod attestation;
pub mod content;
pub mod datasection;
pub mod decoy;
pub mod jwt;
pub mod metadata;
pub mod obfuscation_hooks;
pub mod oblivious;
pub mod packing;
pub mod proof;
pub mod request;
pub mod session;
pub mod temporal;

// On wasm with no std, supply a panic handler.
#[cfg(all(target_arch = "wasm32", not(feature = "std")))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
