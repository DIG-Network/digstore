//! Merkle tree build + inclusion proof verify (paper 7.1, 7.2, 7.3).
//!
//! Domain separation (SECURITY.md residual #2): leaf hashing and internal-node
//! hashing use DISTINCT, named prefix tags so a 32-byte internal node can never
//! be reinterpreted as a leaf (and vice versa). This closes the classic merkle
//! second-preimage / type-confusion class.
//!
//! - `leaf = SHA-256(LEAF_TAG || chunk)`   (raw chunk -> leaf, in `build`)
//! - `node = SHA-256(NODE_TAG || left || right)` (internal node, in `hash_pair`)
//! - an odd node is carried up unchanged (no re-hash, so no extra tag)
//! - `root = generation root`
//!
//! The node tag is applied on BOTH the produce path (`build` / `from_leaves`
//! root computation) and the verify path (`MerkleProof::verify`), so a proof
//! still folds up to the (now domain-separated) root. `from_leaves` receives
//! already-computed leaf digests (the D5 per-resource leaves, which are
//! `SHA-256(ciphertext)`); those are the leaf layer itself and are NOT re-tagged
//! — the leaf/node separation there is provided entirely by the node tag.
//!
//! Proof size (§9.5): a carried-up leaf skips a level, so its inclusion path is
//! `<= ceil(log2 n)` siblings; the bound is attained by the full-spine leaf
//! (index 0). The `<=` bound is the binding contract (see
//! `00-DATASECTION-CONTRACT.md` D8 / design-doc deviation #5): forcing equality
//! would require duplicating odd nodes (changing the root) or an identity
//! `ProofStep` (breaking the §9.3 fold). Soundness (§9.4) is unaffected.

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::hash::sha256;
use alloc::vec::Vec;

/// Domain-separation prefix for a merkle LEAF (`leaf = SHA-256(LEAF_TAG || chunk)`).
/// Distinct from [`NODE_TAG`] so a leaf and an internal node can never collide.
pub const LEAF_TAG: &[u8] = b"digstore:leaf:v1";

/// Domain-separation prefix for an internal merkle NODE
/// (`node = SHA-256(NODE_TAG || left || right)`). Applied on both the build and
/// the verify paths so a proof folds up to the same domain-separated root.
pub const NODE_TAG: &[u8] = b"digstore:node:v1";

/// One step on a bottom-up inclusion path: the sibling hash and whether that
/// sibling sits on the LEFT of the current node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofStep {
    pub hash: Bytes32,
    pub is_left: bool,
}

impl Encode for ProofStep {
    fn encode(&self, enc: &mut Encoder) {
        self.hash.encode(enc);
        (self.is_left as u8).encode(enc);
    }
}

impl Decode for ProofStep {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let hash = Bytes32::decode(dec)?;
        let flag = u8::decode(dec)?;
        let is_left = match flag {
            0 => false,
            1 => true,
            other => return Err(DecodeError::InvalidTag(other)),
        };
        Ok(ProofStep { hash, is_left })
    }
}

/// A complete inclusion proof from a leaf up to the generation root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf: Bytes32,
    pub path: Vec<ProofStep>,
    pub root: Bytes32,
}

impl MerkleProof {
    /// Recompute the root from `leaf` + `path` and compare to `root`.
    pub fn verify(&self) -> bool {
        let mut acc = self.leaf;
        for step in &self.path {
            acc = if step.is_left {
                hash_pair(&step.hash, &acc)
            } else {
                hash_pair(&acc, &step.hash)
            };
        }
        acc == self.root
    }
}

impl Encode for MerkleProof {
    fn encode(&self, enc: &mut Encoder) {
        self.leaf.encode(enc);
        self.path.encode(enc);
        self.root.encode(enc);
    }
}

impl Decode for MerkleProof {
    fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(MerkleProof {
            leaf: Bytes32::decode(dec)?,
            path: Vec::<ProofStep>::decode(dec)?,
            root: Bytes32::decode(dec)?,
        })
    }
}

/// Internal-node hash with the NODE domain tag: `SHA-256(NODE_TAG || left || right)`.
/// Used identically on the produce side (`build` / `from_leaves`) and the verify
/// side (`MerkleProof::verify`).
fn hash_pair(left: &Bytes32, right: &Bytes32) -> Bytes32 {
    let mut buf = Vec::with_capacity(NODE_TAG.len() + 64);
    buf.extend_from_slice(NODE_TAG);
    buf.extend_from_slice(&left.0);
    buf.extend_from_slice(&right.0);
    sha256(&buf)
}

/// Leaf hash with the LEAF domain tag: `SHA-256(LEAF_TAG || chunk)`. Applied only
/// where a raw chunk becomes a leaf (`MerkleTree::build`); `from_leaves` receives
/// already-hashed leaf digests and does not re-tag them.
fn hash_leaf(chunk: &[u8]) -> Bytes32 {
    let mut buf = Vec::with_capacity(LEAF_TAG.len() + chunk.len());
    buf.extend_from_slice(LEAF_TAG);
    buf.extend_from_slice(chunk);
    sha256(&buf)
}

/// A built Merkle tree retaining every level so proofs can be generated.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    /// `levels[0]` are leaves; the last level is the single-element root.
    levels: Vec<Vec<Bytes32>>,
}

impl MerkleTree {
    /// Build a tree from raw chunk byte-slices (`leaf = SHA-256(LEAF_TAG || chunk)`).
    pub fn build(chunks: &[Vec<u8>]) -> MerkleTree {
        let leaves: Vec<Bytes32> = chunks.iter().map(|c| hash_leaf(c)).collect();
        Self::from_leaves(leaves)
    }

    /// Build a tree directly from precomputed leaf hashes.
    pub fn from_leaves(leaves: Vec<Bytes32>) -> MerkleTree {
        let mut levels: Vec<Vec<Bytes32>> = Vec::new();
        let first = if leaves.is_empty() {
            // Empty tree: root is SHA-256 of nothing, kept as a single level.
            alloc::vec![sha256(&[])]
        } else {
            leaves
        };
        levels.push(first);

        while levels.last().map(|l| l.len()).unwrap_or(0) > 1 {
            let prev = levels.last().unwrap();
            let mut next = Vec::with_capacity(prev.len().div_ceil(2));
            let mut i = 0;
            while i < prev.len() {
                if i + 1 < prev.len() {
                    next.push(hash_pair(&prev[i], &prev[i + 1]));
                    i += 2;
                } else {
                    // Odd node carried up unchanged.
                    next.push(prev[i]);
                    i += 1;
                }
            }
            levels.push(next);
        }
        MerkleTree { levels }
    }

    /// The generation root (last level, single element).
    pub fn root(&self) -> Bytes32 {
        *self.levels.last().unwrap().last().unwrap()
    }

    /// Number of leaves.
    pub fn leaf_count(&self) -> usize {
        self.levels[0].len()
    }

    /// Generate an inclusion proof for leaf `index`, or `None` if out of range.
    pub fn prove(&self, index: usize) -> Option<MerkleProof> {
        if index >= self.leaf_count() {
            return None;
        }
        let leaf = self.levels[0][index];
        let mut path = Vec::new();
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            if idx.is_multiple_of(2) {
                // Right sibling exists unless this is a carried-up odd node.
                if idx + 1 < level.len() {
                    path.push(ProofStep {
                        hash: level[idx + 1],
                        is_left: false,
                    });
                }
                // else: node carried up unchanged, no step added.
            } else {
                // Left sibling always exists.
                path.push(ProofStep {
                    hash: level[idx - 1],
                    is_left: true,
                });
            }
            idx /= 2;
        }
        Some(MerkleProof {
            leaf,
            path,
            root: self.root(),
        })
    }
}

#[cfg(test)]
mod domain_separation_tests {
    use super::*;
    use alloc::vec;

    /// Leaf and node hashing are domain-separated: a single-chunk `build` root
    /// (a tagged leaf) must differ from `hash_pair` over the same bytes (a node),
    /// and a leaf must never equal a bare untagged SHA-256 of the chunk.
    #[test]
    fn leaf_and_node_tags_are_distinct() {
        let chunk = vec![0xABu8; 13];
        let leaf = hash_leaf(&chunk);

        // A leaf is tagged, so it differs from the untagged SHA-256 of the chunk.
        assert_ne!(leaf, sha256(&chunk), "leaf must carry the LEAF domain tag");

        // A node over (leaf, leaf) must differ from a leaf computed over the
        // 64-byte concatenation — i.e. an internal node can't be reread as a leaf.
        let node = hash_pair(&leaf, &leaf);
        let mut cat = Vec::new();
        cat.extend_from_slice(&leaf.0);
        cat.extend_from_slice(&leaf.0);
        assert_ne!(node, hash_leaf(&cat), "node tag must differ from leaf tag");
        assert_ne!(node, sha256(&cat), "node must carry the NODE domain tag");

        // The tags themselves are distinct constants.
        assert_ne!(LEAF_TAG, NODE_TAG);
    }
}
