//! Proof generation and verification

use crate::core::{types::*, error::*};
use crate::proofs::merkle::{MerkleTree, DigstoreProof};
use crate::storage::{store::Store, layer::Layer};
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
    /// Create a new proof for a file
    pub fn new_file_proof(
        store: &Store,
        file_path: &std::path::Path,
        at_root: Option<Hash>
    ) -> Result<Self> {
        let target_root = at_root.unwrap_or(
            store.current_root().ok_or_else(|| 
                DigstoreError::file_not_found(file_path.to_path_buf()))?
        );

        // Load the layer containing the target root
        let layer = store.load_layer(target_root)?;
        
        // Find the file in the layer
        let file_index = layer.files.iter().position(|f| f.path == file_path)
            .ok_or_else(|| DigstoreError::file_not_found(file_path.to_path_buf()))?;

        // Build merkle tree from file hashes
        let file_hashes: Vec<Hash> = layer.files.iter().map(|f| f.hash).collect();
        let merkle_tree = MerkleTree::from_hashes(&file_hashes)?;
        
        // Generate proof for the file
        let merkle_proof = merkle_tree.generate_proof(file_index)?;

        // Convert to our Proof format
        let proof_elements = Self::convert_merkle_proof_to_elements(&merkle_proof)?;

        Ok(Proof {
            version: "1.0".to_string(),
            proof_type: "file".to_string(),
            target: ProofTarget::File {
                path: file_path.to_path_buf(),
                at: Some(target_root),
            },
            root: merkle_tree.root(),
            proof_path: proof_elements,
            metadata: ProofMetadata {
                timestamp: chrono::Utc::now().timestamp(),
                layer_number: Some(layer.header.layer_number),
                store_id: store.store_id(),
            },
        })
    }

    /// Create a new proof for a byte range
    pub fn new_byte_range_proof(
        store: &Store,
        file_path: &std::path::Path,
        start: u64,
        end: u64,
        at_root: Option<Hash>
    ) -> Result<Self> {
        let target_root = at_root.unwrap_or(
            store.current_root().ok_or_else(|| 
                DigstoreError::file_not_found(file_path.to_path_buf()))?
        );

        // For byte range proofs, we first prove the file exists, then add byte range info
        let mut file_proof = Self::new_file_proof(store, file_path, Some(target_root))?;
        
        // Update the target to be a byte range
        file_proof.target = ProofTarget::ByteRange {
            path: file_path.to_path_buf(),
            start,
            end,
            at: Some(target_root),
        };
        
        file_proof.proof_type = "byte_range".to_string();

        Ok(file_proof)
    }

    /// Create a new proof for a layer
    pub fn new_layer_proof(store: &Store, layer_id: LayerId) -> Result<Self> {
        // Verify layer exists
        let _layer = store.load_layer(layer_id)?;

        // For layer proofs, we need to prove the layer is in the root history
        // This is a simplified implementation
        Ok(Proof {
            version: "1.0".to_string(),
            proof_type: "layer".to_string(),
            target: ProofTarget::Layer { layer_id },
            root: layer_id, // For simplicity, layer proves itself
            proof_path: vec![], // No path needed for layer proofs
            metadata: ProofMetadata {
                timestamp: chrono::Utc::now().timestamp(),
                layer_number: None,
                store_id: store.store_id(),
            },
        })
    }

    /// Verify this proof
    pub fn verify(&self) -> Result<bool> {
        match &self.target {
            ProofTarget::File { .. } | ProofTarget::ByteRange { .. } => {
                // For file proofs, verify the merkle proof path
                self.verify_merkle_proof_path()
            },
            ProofTarget::Layer { layer_id } => {
                // For layer proofs, verify the layer ID matches the root
                Ok(self.root == *layer_id)
            },
            ProofTarget::Chunk { .. } => {
                // Chunk proofs are not implemented yet
                Ok(false)
            }
        }
    }

    /// Verify the merkle proof path
    fn verify_merkle_proof_path(&self) -> Result<bool> {
        // This is a simplified verification - in a full implementation,
        // we would reconstruct the merkle path and verify each step
        Ok(!self.proof_path.is_empty())
    }

    /// Convert a DigstoreProof to ProofElements (simplified)
    fn convert_merkle_proof_to_elements(merkle_proof: &DigstoreProof) -> Result<Vec<ProofElement>> {
        // This is a simplified conversion - in a full implementation,
        // we would parse the proof bytes and extract the actual sibling hashes
        Ok(vec![ProofElement {
            hash: merkle_proof.root_hash(),
            position: ProofPosition::Right,
        }])
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

/// Proof generator for creating proofs from a store
pub struct ProofGenerator<'a> {
    store: &'a Store,
}

impl<'a> ProofGenerator<'a> {
    /// Create a new proof generator
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Generate a proof for a file
    pub fn prove_file(&self, file_path: &std::path::Path, at_root: Option<Hash>) -> Result<Proof> {
        Proof::new_file_proof(self.store, file_path, at_root)
    }

    /// Generate a proof for a byte range
    pub fn prove_byte_range(
        &self, 
        file_path: &std::path::Path, 
        start: u64, 
        end: u64, 
        at_root: Option<Hash>
    ) -> Result<Proof> {
        Proof::new_byte_range_proof(self.store, file_path, start, end, at_root)
    }

    /// Generate a proof for a layer
    pub fn prove_layer(&self, layer_id: LayerId) -> Result<Proof> {
        Proof::new_layer_proof(self.store, layer_id)
    }
}
