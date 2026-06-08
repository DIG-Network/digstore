//! Merkle tree build + inclusion proof verify (paper 7.1, 7.2, 7.3).
//!
//! - `leaf = SHA-256(chunk)`
//! - `node = SHA-256(left || right)`
//! - an odd node is carried up unchanged
//! - `root = generation root`

use crate::bytes::Bytes32;
use crate::codec::{Decode, DecodeError, Decoder, Encode, Encoder};
use crate::hash::sha256;
use alloc::vec::Vec;

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

fn hash_pair(left: &Bytes32, right: &Bytes32) -> Bytes32 {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&left.0);
    buf[32..].copy_from_slice(&right.0);
    sha256(&buf)
}

/// A built Merkle tree retaining every level so proofs can be generated.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    /// `levels[0]` are leaves; the last level is the single-element root.
    levels: Vec<Vec<Bytes32>>,
}

impl MerkleTree {
    /// Build a tree from raw chunk byte-slices (`leaf = SHA-256(chunk)`).
    pub fn build(chunks: &[Vec<u8>]) -> MerkleTree {
        let leaves: Vec<Bytes32> = chunks.iter().map(|c| sha256(c)).collect();
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
