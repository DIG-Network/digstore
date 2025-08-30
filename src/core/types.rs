//! Core data types for Digstore Min

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// 32-byte SHA-256 hash
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Create a Hash from a 32-byte array
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }

    /// Get the underlying bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Create a Hash from a hex string
    pub fn from_hex(hex: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(hex)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Hash(array))
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Create a zero hash (for testing and special cases)
    pub fn zero() -> Self {
        Hash([0u8; 32])
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", &hex::encode(self.0)[..8])
    }
}

impl From<[u8; 32]> for Hash {
    fn from(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }
}

/// Store identifier (32-byte random value)
pub type StoreId = Hash;

/// Chunk hash
pub type ChunkHash = Hash;

/// Root hash of a layer or repository
pub type RootHash = Hash;

/// Layer ID
pub type LayerId = Hash;

/// Layer types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerType {
    /// Header layer (Layer 0) containing metadata
    Header,
    /// Full layer containing complete repository state
    Full,
    /// Delta layer containing only changes from parent
    Delta,
}

impl LayerType {
    /// Convert to byte representation for binary format
    pub fn to_byte(self) -> u8 {
        match self {
            LayerType::Header => 0x00,
            LayerType::Full => 0x01,
            LayerType::Delta => 0x02,
        }
    }

    /// Create from byte representation
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(LayerType::Header),
            0x01 => Some(LayerType::Full),
            0x02 => Some(LayerType::Delta),
            _ => None,
        }
    }
}

/// A data chunk with its metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Content hash of the chunk
    pub hash: ChunkHash,
    /// Offset within the original file
    pub offset: u64,
    /// Size of the chunk in bytes
    pub size: u32,
    /// The actual chunk data
    pub data: Vec<u8>,
}

/// File entry in a layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// File path relative to repository root
    pub path: PathBuf,
    /// Hash of the complete file content
    pub hash: Hash,
    /// Total file size in bytes
    pub size: u64,
    /// List of chunks that make up this file
    pub chunks: Vec<ChunkRef>,
    /// File metadata
    pub metadata: FileMetadata,
}

/// Reference to a chunk within a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// Hash of the chunk
    pub hash: ChunkHash,
    /// Offset within the file
    pub offset: u64,
    /// Size of the chunk
    pub size: u32,
}

/// File metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// File permissions (Unix-style)
    pub mode: u32,
    /// Last modified timestamp
    pub modified: i64,
    /// Whether this is a new file in this layer
    pub is_new: bool,
    /// Whether this file was modified in this layer
    pub is_modified: bool,
    /// Whether this file was deleted in this layer
    pub is_deleted: bool,
}

/// Layer header information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerHeader {
    /// Magic bytes for format identification
    pub magic: [u8; 4],
    /// Format version
    pub version: u16,
    /// Type of layer
    pub layer_type: LayerType,
    /// Feature flags
    pub flags: u8,
    /// Sequential layer number
    pub layer_number: u64,
    /// Creation timestamp
    pub timestamp: u64,
    /// Parent layer hash (zero for first layer)
    pub parent_hash: RootHash,
    /// Number of files in this layer
    pub files_count: u32,
    /// Number of chunks in this layer
    pub chunks_count: u32,
    /// Offset to index section
    pub index_offset: u64,
    /// Size of index section
    pub index_size: u64,
    /// Offset to data section
    pub data_offset: u64,
    /// Size of data section
    pub data_size: u64,
    /// Offset to merkle tree section
    pub merkle_offset: u64,
    /// Size of merkle tree section
    pub merkle_size: u64,
    /// Compression algorithm used
    pub compression: u8,
}

impl LayerHeader {
    /// Magic bytes for layer files
    pub const MAGIC: [u8; 4] = *b"DIGS";
    /// Current format version
    pub const VERSION: u16 = 1;
    /// Size of the header in bytes
    pub const SIZE: usize = 256;

    /// Create a new layer header
    pub fn new(layer_type: LayerType, layer_number: u64, parent_hash: RootHash) -> Self {
        Self {
            magic: Self::MAGIC,
            version: Self::VERSION,
            layer_type,
            flags: 0,
            layer_number,
            timestamp: chrono::Utc::now().timestamp() as u64,
            parent_hash,
            files_count: 0,
            chunks_count: 0,
            index_offset: 0,
            index_size: 0,
            data_offset: 0,
            data_size: 0,
            merkle_offset: 0,
            merkle_size: 0,
            compression: 0, // No compression
        }
    }

    /// Check if header is valid
    pub fn is_valid(&self) -> bool {
        self.magic == Self::MAGIC && self.version == Self::VERSION
    }
}

/// Layer metadata stored in JSON format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerMetadata {
    /// Layer identifier (hash of layer content)
    pub layer_id: LayerId,
    /// Parent layer identifier
    pub parent_id: Option<LayerId>,
    /// Creation timestamp
    pub timestamp: i64,
    /// Generation number
    pub generation: u64,
    /// Layer type
    pub layer_type: LayerType,
    /// Number of files
    pub file_count: usize,
    /// Total size of all files
    pub total_size: u64,
    /// Merkle root of all files in this layer
    pub merkle_root: Hash,
    /// Commit message (if any)
    pub message: Option<String>,
    /// Author information
    pub author: Option<String>,
}

/// Commit information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    /// Commit hash (same as layer ID)
    pub hash: LayerId,
    /// Parent commit hash
    pub parent: Option<LayerId>,
    /// Commit message
    pub message: String,
    /// Author name
    pub author: String,
    /// Commit timestamp
    pub timestamp: i64,
    /// Number of files changed
    pub files_changed: usize,
    /// Total size of changes
    pub size_delta: i64,
}

/// Tree node for merkle tree construction
#[derive(Debug, Clone)]
pub enum TreeNode {
    /// Leaf node containing a hash
    Leaf(Hash),
    /// Internal node with left and right children
    Internal {
        left: Box<TreeNode>,
        right: Box<TreeNode>,
        hash: Hash,
    },
}

impl TreeNode {
    /// Get the hash of this node
    pub fn hash(&self) -> Hash {
        match self {
            TreeNode::Leaf(hash) => *hash,
            TreeNode::Internal { hash, .. } => *hash,
        }
    }
}
