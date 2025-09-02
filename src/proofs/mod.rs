//! Merkle proof system for Digstore Min
//!
//! This module provides functionality for generating and verifying merkle proofs
//! for any data item, byte range, or layer in the repository.

pub mod incremental;
pub mod merkle;
pub mod proof;

// Re-export commonly used items
pub use incremental::{IncrementalMerkleBuilder, IndexCache, StreamingLayerWriter};
pub use merkle::{DigstoreProof, MerkleTree};
pub use proof::{Proof, ProofElement, ProofGenerator, ProofMetadata, ProofPosition, ProofTarget};
