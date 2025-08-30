//! Proof generation and verification

use crate::core::{types::*, error::*};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Proof target specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProofTarget {
    /// Prove a file exists
    File {
        path: PathBuf,
        at: Option<Hash>,
    },
    /// Prove a byte range
    ByteRange {
        path: PathBuf,
        start: u64,
        end: u64,
        at: Option<Hash>,
    },
    /// Prove a layer exists
    Layer {
        layer_id: LayerId,
    },
    /// Prove a chunk exists
    Chunk {
        chunk_hash: ChunkHash,
    },
}

/// A single element in a merkle proof path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofElement {
    /// Hash of the sibling node
    pub hash: Hash,
    /// Whether the sibling is on the left or right
    pub position: ProofPosition,
}

/// Position of a sibling in a merkle proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProofPosition {
    Left,
    Right,
}

/// Complete merkle proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    /// Version of the proof format
    pub version: String,
    /// Type of proof
    pub proof_type: String,
    /// Target being proved
    pub target: ProofTarget,
    /// Root hash to verify against
    pub root: Hash,
    /// Proof path elements
    pub proof_path: Vec<ProofElement>,
    /// Additional metadata
    pub metadata: ProofMetadata,
}

/// Metadata included with proofs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofMetadata {
    /// Timestamp when proof was generated
    pub timestamp: i64,
    /// Layer number
    pub layer_number: Option<u64>,
    /// Store ID
    pub store_id: StoreId,
}

impl Proof {
    /// Verify this proof
    pub fn verify(&self) -> Result<bool> {
        // TODO: Implement proof verification
        todo!("Proof::verify not yet implemented")
    }

    /// Serialize proof to JSON
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(DigstoreError::JsonSerialization)
    }

    /// Deserialize proof from JSON
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(DigstoreError::JsonSerialization)
    }
}
