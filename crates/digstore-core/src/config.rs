//! Store/generation/compiler configuration types (paper 5.2 structs).

use crate::bytes::Bytes32;
use crate::merkle::MerkleTree;
use alloc::string::String;

/// Per-store hard cap on staged plaintext content (§3). 128 MB, decimal.
///
/// Single source of truth: the CLI enforces it at stage/commit, the compiler
/// uses it as the uniform-filler budget (`FIXED_BLOB_LEN` must cover a max-cap
/// store's ciphertext + key table + merkle + header), and the host derives its
/// memory bound from the resulting module size.
pub const MAX_STORE_BYTES: u64 = 128_000_000;

#[cfg(feature = "std")]
use std::path::PathBuf;

/// 32-byte secret salt mixed into private-store key derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretSalt(pub [u8; 32]);

/// Store visibility: public, or private with a secret salt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private(SecretSalt),
}

/// Static configuration for a store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConfig {
    pub store_id: Bytes32,
    pub data_dir: String,
    pub max_size: u64,
    pub visibility: Visibility,
}

/// Logical generation identifier.
pub type GenerationId = u64;

/// The committed state of a generation (no tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationState {
    pub id: u64,
    pub root: Bytes32,
    pub timestamp: u64,
}

/// A full generation: its state plus the built Merkle tree.
#[derive(Debug, Clone)]
pub struct Generation {
    pub state: GenerationState,
    pub tree: MerkleTree,
}

/// Content-defined chunker configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkerConfig {
    pub min_size: usize,
    pub target_size: usize,
    pub max_size: usize,
    pub mask: u64,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        ChunkerConfig {
            min_size: 16 * 1024,
            target_size: 64 * 1024,
            max_size: 256 * 1024,
            // Mask with ~16 bits set to target ~64KiB average chunks.
            mask: 0xFFFF,
        }
    }
}

/// Host imports configuration / limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostImportsConfig {
    pub return_buffer_capacity: usize,
    pub max_return_buffer_size: usize,
    pub max_random_bytes: u32,
    pub host_version: String,
}

impl Default for HostImportsConfig {
    fn default() -> Self {
        HostImportsConfig {
            return_buffer_capacity: 64 * 1024,
            max_return_buffer_size: 16 * 1024 * 1024,
            max_random_bytes: 1024,
            host_version: String::new(),
        }
    }
}

/// A trusted host BLS public key with its label (`dig-host-key-v1:<hex>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedHostKey {
    pub public_key: [u8; 48],
    pub label: String,
}

/// Statistics produced by a compilation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompilationStats {
    pub chunk_count: u64,
    pub total_bytes: u64,
    pub generation_count: u64,
}

/// Result of compiling a store into a serving module.
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilationResult {
    pub store_id: Bytes32,
    pub roothash: Bytes32,
    pub output_path: PathBuf,
    pub output_size: u64,
    pub stats: CompilationStats,
}

/// Errors raised by the compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerError {
    NoTrustedKeys,
    Io(String),
    Validation(String),
}
