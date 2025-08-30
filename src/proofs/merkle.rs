//! Merkle tree implementation using rs_merkle

use crate::core::{types::*, error::*};
use rs_merkle::{MerkleTree as RsMerkleTree, MerkleProof as RsMerkleProof, Hasher};
use sha2::{Sha256, Digest};

/// Custom hasher implementation for rs_merkle
#[derive(Clone)]
pub struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    type Hash = [u8; 32];

    fn hash(data: &[u8]) -> Self::Hash {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

/// Merkle tree wrapper for Digstore Min
pub struct MerkleTree {
    /// The underlying rs_merkle tree
    inner: RsMerkleTree<Sha256Hasher>,
    /// Root hash of the tree
    root: Hash,
    /// Original leaf hashes
    leaves: Vec<Hash>,
}

impl MerkleTree {
    /// Build a merkle tree from a list of hashes
    pub fn from_hashes(hashes: &[Hash]) -> Result<Self> {
        if hashes.is_empty() {
            return Err(DigstoreError::MerkleTreeFailed { 
                reason: "Cannot create merkle tree from empty hash list".to_string() 
            });
        }

        // Convert our Hash types to [u8; 32] for rs_merkle
        let leaves: Vec<[u8; 32]> = hashes
            .iter()
            .map(|h| *h.as_bytes())
            .collect();

        // Build the tree
        let tree = RsMerkleTree::<Sha256Hasher>::from_leaves(&leaves);
        
        // Get the root hash
        let root_bytes = tree.root().ok_or_else(|| DigstoreError::MerkleTreeFailed {
            reason: "Failed to get merkle tree root".to_string()
        })?;
        
        let root = Hash::from_bytes(root_bytes);

        Ok(Self {
            inner: tree,
            root,
            leaves: hashes.to_vec(),
        })
    }

    /// Get the root hash
    pub fn root(&self) -> Hash {
        self.root
    }

    /// Get the number of leaves
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Generate a proof for a specific leaf index
    pub fn generate_proof(&self, leaf_index: usize) -> Result<DigstoreProof> {
        if leaf_index >= self.leaves.len() {
            return Err(DigstoreError::MerkleTreeFailed {
                reason: format!("Leaf index {} out of bounds (tree has {} leaves)", 
                    leaf_index, self.leaves.len())
            });
        }

        let proof = self.inner.proof(&[leaf_index]);
        let proof_bytes = proof.to_bytes();

        Ok(DigstoreProof {
            leaf_index,
            leaf_hash: self.leaves[leaf_index],
            proof_bytes,
            root_hash: self.root,
        })
    }

    /// Verify a proof against this tree's root
    pub fn verify_proof(&self, merkle_proof: &DigstoreProof) -> bool {
        // Parse the proof from bytes
        if let Ok(proof) = RsMerkleProof::<Sha256Hasher>::try_from(merkle_proof.proof_bytes.as_slice()) {
            // Verify the proof
            proof.verify(
                *self.root.as_bytes(),
                &[merkle_proof.leaf_index],
                &[*merkle_proof.leaf_hash.as_bytes()],
                self.leaves.len()
            )
        } else {
            false
        }
    }

    /// Get all leaf hashes
    pub fn leaves(&self) -> &[Hash] {
        &self.leaves
    }
}

/// A merkle proof for a specific leaf
#[derive(Debug, Clone)]
pub struct DigstoreProof {
    /// Index of the leaf being proved
    pub leaf_index: usize,
    /// Hash of the leaf being proved
    pub leaf_hash: Hash,
    /// Serialized proof bytes
    pub proof_bytes: Vec<u8>,
    /// Root hash this proof is against
    pub root_hash: Hash,
}

impl DigstoreProof {
    /// Verify this proof against a given root hash
    pub fn verify(&self, expected_root: &Hash) -> bool {
        if self.root_hash != *expected_root {
            return false;
        }

        if let Ok(proof) = RsMerkleProof::<Sha256Hasher>::try_from(self.proof_bytes.as_slice()) {
            proof.verify(
                *expected_root.as_bytes(),
                &[self.leaf_index],
                &[*self.leaf_hash.as_bytes()],
                // Note: We need the leaf count, but it's not stored in the proof
                // For now, we'll assume verification happens against the original tree
                usize::MAX // This is a limitation we'll address
            )
        } else {
            false
        }
    }

    /// Get the leaf index
    pub fn leaf_index(&self) -> usize {
        self.leaf_index
    }

    /// Get the leaf hash
    pub fn leaf_hash(&self) -> Hash {
        self.leaf_hash
    }

    /// Get the root hash
    pub fn root_hash(&self) -> Hash {
        self.root_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hash::sha256;

    #[test]
    fn test_merkle_tree_creation() -> Result<()> {
        let hashes = vec![
            sha256(b"file1"),
            sha256(b"file2"),
            sha256(b"file3"),
            sha256(b"file4"),
        ];

        let tree = MerkleTree::from_hashes(&hashes)?;
        
        assert_eq!(tree.leaf_count(), 4);
        assert_ne!(tree.root(), Hash::zero());
        assert_eq!(tree.leaves(), &hashes);

        Ok(())
    }

    #[test]
    fn test_merkle_tree_empty() {
        let result = MerkleTree::from_hashes(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_merkle_tree_single_leaf() -> Result<()> {
        let hash = sha256(b"single file");
        let tree = MerkleTree::from_hashes(&[hash])?;
        
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.leaves()[0], hash);

        Ok(())
    }

    #[test]
    fn test_merkle_proof_generation() -> Result<()> {
        let hashes = vec![
            sha256(b"file1"),
            sha256(b"file2"),
            sha256(b"file3"),
            sha256(b"file4"),
        ];

        let tree = MerkleTree::from_hashes(&hashes)?;
        
        // Generate proof for each leaf
        for i in 0..hashes.len() {
            let proof = tree.generate_proof(i)?;
            assert_eq!(proof.leaf_index(), i);
            assert_eq!(proof.leaf_hash(), hashes[i]);
            assert_eq!(proof.root_hash(), tree.root());
        }

        Ok(())
    }

    #[test]
    fn test_merkle_proof_out_of_bounds() -> Result<()> {
        let hashes = vec![sha256(b"file1"), sha256(b"file2")];
        let tree = MerkleTree::from_hashes(&hashes)?;
        
        let result = tree.generate_proof(5);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_merkle_tree_deterministic() -> Result<()> {
        let hashes = vec![
            sha256(b"file1"),
            sha256(b"file2"),
            sha256(b"file3"),
        ];

        let tree1 = MerkleTree::from_hashes(&hashes)?;
        let tree2 = MerkleTree::from_hashes(&hashes)?;
        
        // Same input should produce same root
        assert_eq!(tree1.root(), tree2.root());

        Ok(())
    }

    #[test]
    fn test_merkle_tree_different_inputs() -> Result<()> {
        let hashes1 = vec![sha256(b"file1"), sha256(b"file2")];
        let hashes2 = vec![sha256(b"file1"), sha256(b"file3")];

        let tree1 = MerkleTree::from_hashes(&hashes1)?;
        let tree2 = MerkleTree::from_hashes(&hashes2)?;
        
        // Different inputs should produce different roots
        assert_ne!(tree1.root(), tree2.root());

        Ok(())
    }

    #[test]
    fn test_merkle_proof_verification_with_tree() -> Result<()> {
        let hashes = vec![
            sha256(b"file1"),
            sha256(b"file2"),
            sha256(b"file3"),
        ];

        let tree = MerkleTree::from_hashes(&hashes)?;
        
        // Generate and verify proof for middle leaf
        let proof = tree.generate_proof(1)?;
        assert!(tree.verify_proof(&proof));
        
        // Verify all proofs
        for i in 0..hashes.len() {
            let proof = tree.generate_proof(i)?;
            assert!(tree.verify_proof(&proof), "Proof verification failed for index {}", i);
        }

        Ok(())
    }
}
