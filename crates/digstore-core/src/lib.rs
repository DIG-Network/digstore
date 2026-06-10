#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod abi;
pub mod bytes;
pub mod codec;
pub mod config;
pub mod datasection;
pub mod error;
pub mod hash;
pub mod keytable;
pub mod manifest;
pub mod merkle;
pub mod urn;
pub mod wire;

pub use abi::{is_error, pack_ptr_len, unpack_ptr_len};
pub use bytes::{Bytes32, Bytes48, Bytes96};
pub use codec::{Decode, DecodeError, Decoder, Encode, Encoder};
pub use error::{CoreError, ErrorCode};
pub use hash::sha256;

/// Alias module so `digstore_core::types::Bytes32` resolves (host/guest use this path).
pub mod types {
    pub use crate::bytes::{Bytes32, Bytes48, Bytes96};
}

/// CONVENTIONS C9: single source of truth for the serving-output byte ordering.
/// Both `digstore-guest` (`get_content`) and `digstore-prover`
/// (`ServingInputs::output_bytes`) call this so re-execution matches what was
/// served (deviation #3, `program_hash` binding).
pub mod serving {
    use alloc::vec::Vec;

    /// Concatenate chunk byte-slices in the given order (simple ordered concat).
    pub fn concat_output(chunks_in_order: &[&[u8]]) -> Vec<u8> {
        let total: usize = chunks_in_order.iter().map(|c| c.len()).sum();
        let mut out = Vec::with_capacity(total);
        for chunk in chunks_in_order {
            out.extend_from_slice(chunk);
        }
        out
    }
}

#[cfg(feature = "std")]
pub use config::CompilationResult;
pub use config::{
    ChunkerConfig, CompilationStats, CompilerError, Generation, GenerationId, GenerationState,
    HostImportsConfig, SecretSalt, StoreConfig, TrustedHostKey, Visibility, MAX_STORE_BYTES,
};
pub use keytable::{KeyTableEntry, PathWalk};
pub use manifest::{Author, MetadataManifest};
pub use merkle::{MerkleProof, MerkleTree, ProofStep};
pub use urn::Urn;
pub use wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ChiaBlockRef, ContentResponse,
    ExecutionProof, ProofPrelude, ProofResponse, ATTEST_DST,
};
