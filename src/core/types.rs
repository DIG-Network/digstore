//! Core data types for Digstore Min

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// 32-byte SHA-256 hash
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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

// Custom serialization to use hex strings instead of byte arrays
impl Serialize for Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_string = String::deserialize(deserializer)?;
        Hash::from_hex(&hex_string).map_err(serde::de::Error::custom)
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

/// Layer header information (256 bytes fixed size)
#[derive(Debug, Clone)]
pub struct LayerHeader {
    /// Magic bytes for format identification: "DIGS"
    pub magic: [u8; 4],
    /// Format version (1)
    pub version: u16,
    /// Type of layer (0=Header, 1=Full, 2=Delta)
    pub layer_type: u8,
    /// Feature flags
    pub flags: u8,
    /// Sequential layer number
    pub layer_number: u64,
    /// Creation timestamp (Unix timestamp)
    pub timestamp: u64,
    /// Parent layer root hash (zero for first layer)
    pub parent_hash: [u8; 32],
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
    /// Compression algorithm used (0=None, 1=Zstd, 2=LZ4)
    pub compression: u8,
    /// Reserved bytes for future use (143 bytes to make total 256)
    pub reserved: [u8; 143],
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
            layer_type: layer_type.to_byte(),
            flags: 0,
            layer_number,
            timestamp: chrono::Utc::now().timestamp() as u64,
            parent_hash: *parent_hash.as_bytes(),
            files_count: 0,
            chunks_count: 0,
            index_offset: 0,
            index_size: 0,
            data_offset: 0,
            data_size: 0,
            merkle_offset: 0,
            merkle_size: 0,
            compression: 0, // No compression
            reserved: [0u8; 143],
        }
    }

    /// Check if header is valid
    pub fn is_valid(&self) -> bool {
        self.magic == Self::MAGIC && self.version == Self::VERSION
    }

    /// Get the layer type
    pub fn get_layer_type(&self) -> Option<LayerType> {
        LayerType::from_byte(self.layer_type)
    }

    /// Get the parent hash
    pub fn get_parent_hash(&self) -> Hash {
        Hash::from_bytes(self.parent_hash)
    }

    /// Set the parent hash
    pub fn set_parent_hash(&mut self, hash: &Hash) {
        self.parent_hash = *hash.as_bytes();
    }

    /// Convert to bytes for writing to disk
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::SIZE);
        
        // Magic (4 bytes)
        bytes.extend_from_slice(&self.magic);
        
        // Version (2 bytes, little-endian)
        bytes.extend_from_slice(&self.version.to_le_bytes());
        
        // Layer type (1 byte)
        bytes.push(self.layer_type);
        
        // Flags (1 byte)
        bytes.push(self.flags);
        
        // Layer number (8 bytes, little-endian)
        bytes.extend_from_slice(&self.layer_number.to_le_bytes());
        
        // Timestamp (8 bytes, little-endian)
        bytes.extend_from_slice(&self.timestamp.to_le_bytes());
        
        // Parent hash (32 bytes)
        bytes.extend_from_slice(&self.parent_hash);
        
        // Files count (4 bytes, little-endian)
        bytes.extend_from_slice(&self.files_count.to_le_bytes());
        
        // Chunks count (4 bytes, little-endian)
        bytes.extend_from_slice(&self.chunks_count.to_le_bytes());
        
        // Index offset (8 bytes, little-endian)
        bytes.extend_from_slice(&self.index_offset.to_le_bytes());
        
        // Index size (8 bytes, little-endian)
        bytes.extend_from_slice(&self.index_size.to_le_bytes());
        
        // Data offset (8 bytes, little-endian)
        bytes.extend_from_slice(&self.data_offset.to_le_bytes());
        
        // Data size (8 bytes, little-endian)
        bytes.extend_from_slice(&self.data_size.to_le_bytes());
        
        // Merkle offset (8 bytes, little-endian)
        bytes.extend_from_slice(&self.merkle_offset.to_le_bytes());
        
        // Merkle size (8 bytes, little-endian)
        bytes.extend_from_slice(&self.merkle_size.to_le_bytes());
        
        // Compression (1 byte)
        bytes.push(self.compression);
        
        // Reserved (143 bytes)
        bytes.extend_from_slice(&self.reserved);
        
        assert_eq!(bytes.len(), Self::SIZE);
        bytes
    }

    /// Create from bytes read from disk
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < Self::SIZE {
            return Err(format!("Header too short: {} bytes, expected {}", bytes.len(), Self::SIZE));
        }

        let mut offset = 0;

        // Magic (4 bytes)
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[offset..offset + 4]);
        offset += 4;

        // Version (2 bytes, little-endian)
        let version = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        offset += 2;

        // Layer type (1 byte)
        let layer_type = bytes[offset];
        offset += 1;

        // Flags (1 byte)
        let flags = bytes[offset];
        offset += 1;

        // Layer number (8 bytes, little-endian)
        let mut layer_number_bytes = [0u8; 8];
        layer_number_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let layer_number = u64::from_le_bytes(layer_number_bytes);
        offset += 8;

        // Timestamp (8 bytes, little-endian)
        let mut timestamp_bytes = [0u8; 8];
        timestamp_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let timestamp = u64::from_le_bytes(timestamp_bytes);
        offset += 8;

        // Parent hash (32 bytes)
        let mut parent_hash = [0u8; 32];
        parent_hash.copy_from_slice(&bytes[offset..offset + 32]);
        offset += 32;

        // Files count (4 bytes, little-endian)
        let mut files_count_bytes = [0u8; 4];
        files_count_bytes.copy_from_slice(&bytes[offset..offset + 4]);
        let files_count = u32::from_le_bytes(files_count_bytes);
        offset += 4;

        // Chunks count (4 bytes, little-endian)
        let mut chunks_count_bytes = [0u8; 4];
        chunks_count_bytes.copy_from_slice(&bytes[offset..offset + 4]);
        let chunks_count = u32::from_le_bytes(chunks_count_bytes);
        offset += 4;

        // Index offset (8 bytes, little-endian)
        let mut index_offset_bytes = [0u8; 8];
        index_offset_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let index_offset = u64::from_le_bytes(index_offset_bytes);
        offset += 8;

        // Index size (8 bytes, little-endian)
        let mut index_size_bytes = [0u8; 8];
        index_size_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let index_size = u64::from_le_bytes(index_size_bytes);
        offset += 8;

        // Data offset (8 bytes, little-endian)
        let mut data_offset_bytes = [0u8; 8];
        data_offset_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let data_offset = u64::from_le_bytes(data_offset_bytes);
        offset += 8;

        // Data size (8 bytes, little-endian)
        let mut data_size_bytes = [0u8; 8];
        data_size_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let data_size = u64::from_le_bytes(data_size_bytes);
        offset += 8;

        // Merkle offset (8 bytes, little-endian)
        let mut merkle_offset_bytes = [0u8; 8];
        merkle_offset_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let merkle_offset = u64::from_le_bytes(merkle_offset_bytes);
        offset += 8;

        // Merkle size (8 bytes, little-endian)
        let mut merkle_size_bytes = [0u8; 8];
        merkle_size_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let merkle_size = u64::from_le_bytes(merkle_size_bytes);
        offset += 8;

        // Compression (1 byte)
        let compression = bytes[offset];
        offset += 1;

        // Reserved (143 bytes)
        let mut reserved = [0u8; 143];
        reserved.copy_from_slice(&bytes[offset..offset + 143]);

        Ok(Self {
            magic,
            version,
            layer_type,
            flags,
            layer_number,
            timestamp,
            parent_hash,
            files_count,
            chunks_count,
            index_offset,
            index_size,
            data_offset,
            data_size,
            merkle_offset,
            merkle_size,
            compression,
            reserved,
        })
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
