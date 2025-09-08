//! Merkle proof system for Digstore Min
//!
//! This module provides functionality for generating and verifying merkle proofs
//! for any data item, byte range, or layer in the repository.

pub mod incremental;
pub mod merkle;
pub mod proof;
pub mod size_proof;
pub mod ultra_compressed_proof;

// Re-export commonly used items
pub use incremental::{IncrementalMerkleBuilder, IndexCache, StreamingLayerWriter};
pub use merkle::{DigstoreProof, MerkleTree};
pub use proof::{Proof, ProofElement, ProofGenerator, ProofMetadata, ProofPosition, ProofTarget};
pub use size_proof::{
    verify_archive_size_proof, verify_compressed_hex_proof, ArchiveSizeProof, CompressedSizeProof,
    IntegrityProofs,
};
pub use ultra_compressed_proof::{calculate_minimum_theoretical_size, UltraCompressedProof};
