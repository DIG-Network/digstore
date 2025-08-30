//! Merkle proof system for Digstore Min
//!
//! This module provides functionality for generating and verifying merkle proofs
//! for any data item, byte range, or layer in the repository.

pub mod merkle;
pub mod proof;

// Re-export commonly used items
pub use merkle::{MerkleTree, DigstoreProof};
pub use proof::{Proof, ProofTarget, ProofElement, ProofGenerator};
