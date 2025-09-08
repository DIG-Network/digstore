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
pub use proof::{Proof, ProofPosition};
