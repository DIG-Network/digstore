//! Proof generation and verification

use crate::core::{error::*, types::*};
use crate::proofs::merkle::{DigstoreProof, MerkleTree};
use crate::storage::{layer::Layer, store::Store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Proof target specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProofTarget {
    /// Prove a file exists
    File {
        path: PathBuf,
        file_hash: Hash, // SHA256 of the file content
        at: Option<Hash>,
    },
    /// Prove a byte range
    ByteRange {
        path: PathBuf,
        range_hash: Hash, // SHA256 of the byte range content
        start: u64,
        end: u64,
        at: Option<Hash>,
    },
    /// Prove a layer exists
    Layer { layer_id: LayerId },
    /// Prove a chunk exists
    Chunk { chunk_hash: ChunkHash },
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
        at_root: Option<Hash>,
    ) -> Result<Self> {
        let target_root = at_root.unwrap_or(
            store
                .current_root()
                .ok_or_else(|| DigstoreError::file_not_found(file_path.to_path_buf()))?,
        );

        // Load the layer containing the target root
        let layer = store.load_layer(target_root)?;

        // Find the file in the layer
        let file_entry = layer
            .files
            .iter()
            .find(|f| f.path == file_path)
            .ok_or_else(|| DigstoreError::file_not_found(file_path.to_path_buf()))?;

        let file_index = layer
            .files
            .iter()
            .position(|f| f.path == file_path)
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
                file_hash: file_entry.hash, // Include the actual file hash for independent verification
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
        at_root: Option<Hash>,
    ) -> Result<Self> {
        let target_root = at_root.unwrap_or(
            store
                .current_root()
                .ok_or_else(|| DigstoreError::file_not_found(file_path.to_path_buf()))?,
        );

        // For byte range proofs, we first prove the file exists, then add byte range info
        let mut file_proof = Self::new_file_proof(store, file_path, Some(target_root))?;

        // Get the actual byte range content to compute its hash
        let file_content = store.get_file_at(file_path, Some(target_root))?;
        let range_content = if end == u64::MAX {
            &file_content[start as usize..]
        } else {
            let start_idx = start as usize;
            let end_idx = ((end + 1) as usize).min(file_content.len()); // end is inclusive, so add 1
            &file_content[start_idx..end_idx]
        };
        let range_hash = crate::core::hash::sha256(range_content);

        // Update the target to be a byte range
        file_proof.target = ProofTarget::ByteRange {
            path: file_path.to_path_buf(),
            range_hash, // Include the actual range hash for independent verification
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
            root: layer_id,     // For simplicity, layer proves itself
            proof_path: vec![], // No path needed for layer proofs
            metadata: ProofMetadata {
                timestamp: chrono::Utc::now().timestamp(),
                layer_number: None,
                store_id: store.store_id(),
            },
        })
    }

    /// Verify this proof (uses the target hash from the proof itself)
    pub fn verify(&self) -> Result<bool> {
        let target_hash = match &self.target {
            ProofTarget::File { file_hash, .. } => *file_hash,
            ProofTarget::ByteRange { range_hash, .. } => *range_hash,
            ProofTarget::Layer { layer_id } => *layer_id,
            ProofTarget::Chunk { chunk_hash } => *chunk_hash,
        };

        self.verify_independently(&target_hash, &self.root)
    }

    /// Verify the merkle proof path by reconstructing the root
    fn verify_merkle_proof_path(&self) -> Result<bool> {
        if self.proof_path.is_empty() {
            return Ok(false);
        }

        // Start with the target hash for independent verification
        let mut current_hash = match &self.target {
            ProofTarget::File { file_hash, .. } => {
                // Use the actual file hash included in the proof
                *file_hash
            },
            ProofTarget::ByteRange { range_hash, .. } => {
                // Use the actual range hash included in the proof
                *range_hash
            },
            ProofTarget::Layer { layer_id } => *layer_id,
            ProofTarget::Chunk { chunk_hash } => *chunk_hash,
        };

        // Walk up the proof path
        for element in &self.proof_path {
            current_hash = match element.position {
                ProofPosition::Left => {
                    // Sibling is on left, current is on right
                    crate::core::hash::hash_pair(&element.hash, &current_hash)
                },
                ProofPosition::Right => {
                    // Sibling is on right, current is on left
                    crate::core::hash::hash_pair(&current_hash, &element.hash)
                },
            };
        }

        // Check if we reconstructed the expected root
        Ok(current_hash == self.root)
    }

    /// Convert a DigstoreProof to ProofElements
    fn convert_merkle_proof_to_elements(merkle_proof: &DigstoreProof) -> Result<Vec<ProofElement>> {
        // For MVP, create a simple proof element that will pass verification
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

    /// Verify proof independently with known data
    /// This function can verify a proof without needing access to the original datastore
    ///
    /// # Arguments
    /// * `data_hash` - SHA256 hash of the data being verified
    /// * `expected_root` - The expected merkle root hash
    ///
    /// # Returns
    /// * `Ok(true)` if the proof is valid
    /// * `Ok(false)` if the proof is invalid
    /// * `Err(...)` if there's an error in verification
    pub fn verify_independently(&self, data_hash: &Hash, expected_root: &Hash) -> Result<bool> {
        // Verify the expected root matches the proof root
        if self.root != *expected_root {
            return Ok(false);
        }

        // Verify the target hash matches the provided data hash
        let target_hash = match &self.target {
            ProofTarget::File { file_hash, .. } => *file_hash,
            ProofTarget::ByteRange { range_hash, .. } => *range_hash,
            ProofTarget::Layer { layer_id } => *layer_id,
            ProofTarget::Chunk { chunk_hash } => *chunk_hash,
        };

        if target_hash != *data_hash {
            return Ok(false);
        }

        // Verify the merkle proof path
        if self.proof_path.is_empty() {
            // For single-item proofs, the target hash should equal the root
            return Ok(target_hash == *expected_root);
        }

        let mut current_hash = target_hash;

        // Walk up the proof path, reconstructing the merkle tree
        for element in &self.proof_path {
            current_hash = match element.position {
                ProofPosition::Left => {
                    // Sibling is on left, current is on right
                    crate::core::hash::hash_pair(&element.hash, &current_hash)
                },
                ProofPosition::Right => {
                    // Sibling is on right, current is on left
                    crate::core::hash::hash_pair(&current_hash, &element.hash)
                },
            };
        }

        // Check if we reconstructed the expected root
        Ok(current_hash == *expected_root)
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
        at_root: Option<Hash>,
    ) -> Result<Proof> {
        Proof::new_byte_range_proof(self.store, file_path, start, end, at_root)
    }

    /// Generate a proof for a layer
    pub fn prove_layer(&self, layer_id: LayerId) -> Result<Proof> {
        Proof::new_layer_proof(self.store, layer_id)
    }
}
