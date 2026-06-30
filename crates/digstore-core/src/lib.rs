#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod abi;
pub mod bytes;
pub mod capsule;
pub mod codec;
pub mod config;
pub mod crypto;
pub mod datasection;
pub mod error;
pub mod hash;
pub mod keytable;
pub mod manifest;
pub mod merkle;
pub mod tombstone;
pub mod urn;
pub mod urn_grammar;
pub mod wire;

pub use abi::{is_error, pack_ptr_len, unpack_ptr_len};
pub use bytes::{Bytes32, Bytes48, Bytes96};
pub use capsule::Capsule;
pub use codec::{Decode, DecodeError, Decoder, Encode, Encoder};
pub use crypto::{decrypt_chunk, derive_decryption_key, encrypt_chunk};
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
pub use merkle::{resource_leaf, MerkleProof, MerkleTree, ProofStep};
pub use tombstone::{RevocationReason, Tombstone, TombstoneScope};
pub use urn::Urn;

/// The canonical chain tag for Digstore URNs (mainnet-only; paper §1/§10). The
/// SINGLE definition shared by the producer (`digstore-cli`/`digstore-store`), the
/// host, and the browser verifier (`dig-client-wasm`) — the value every layer puts
/// in `Urn.chain`, so the retrieval key derived from the URN can never skew.
pub const CHAIN: &str = "chia";

/// Conventional default-view resource key when a URN carries no resource path
/// (paper §8.5 social conventions): the landing page a bare store URL resolves to.
/// Shared by the CLI producer and the browser verifier.
pub const DEFAULT_RESOURCE_KEY: &str = "index.html";
pub use wire::{
    AttestationChallenge, AttestationResponse, AuthenticationInfo, ChiaBlockRef, ContentResponse,
    ExecutionProof, ProofPrelude, ProofResponse, ATTEST_DST,
};
