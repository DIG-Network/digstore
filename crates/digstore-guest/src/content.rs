//! Content path (§7,8,14). Gate (attestation/session/JWT/temporal) -> key-table
//! lookup -> oblivious gather -> ContentResponse, else a decoy. The guest never
//! decrypts; it returns ciphertext + a merkle proof to the generation root.

use digstore_core::merkle::MerkleTree;
use digstore_core::MerkleProof;

/// Emit an inclusion proof for `leaf_index` using the same rules as core:
/// leaf = SHA-256(chunk), node = SHA-256(left||right), odd node carried up,
/// root = generation root. Delegates to the core tree's proof builder so guest
/// and client agree byte-for-byte. Out-of-range indices yield an empty proof
/// rooted at the tree root (which fails `verify`, as expected).
pub fn emit_merkle_proof(tree: &MerkleTree, leaf_index: usize) -> MerkleProof {
    tree.prove(leaf_index).unwrap_or_else(|| MerkleProof {
        leaf: tree.root(),
        path: alloc::vec::Vec::new(),
        root: tree.root(),
    })
}
