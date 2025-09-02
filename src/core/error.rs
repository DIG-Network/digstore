//! Error types for Digstore Min

use crate::core::types::Hash;
use std::path::PathBuf;
use thiserror::Error;

/// Main error type for Digstore operations
#[derive(Error, Debug)]
pub enum DigstoreError {
    /// Store-related errors
    #[error("Store not found: {path}")]
    StoreNotFound { path: PathBuf },

    #[error("Store already exists: {path}")]
    StoreAlreadyExists { path: PathBuf },

    #[error("Invalid store ID: {store_id}")]
    InvalidStoreId { store_id: String },

    #[error("Store is corrupted: {reason}")]
    StoreCorrupted { reason: String },

    /// Layer-related errors
    #[error("Layer not found: {hash}")]
    LayerNotFound { hash: Hash },

    #[error("Invalid layer format: {reason}")]
    InvalidLayerFormat { reason: String },

    #[error("Layer verification failed: {hash}")]
    LayerVerificationFailed { hash: Hash },

    /// File-related errors
    #[error("File not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("File already exists: {path}")]
    FileAlreadyExists { path: PathBuf },

    #[error("Invalid file path: {path}")]
    InvalidFilePath { path: PathBuf },

    /// Chunk-related errors
    #[error("Chunk not found: {hash}")]
    ChunkNotFound { hash: Hash },

    #[error("Chunk verification failed: {hash}")]
    ChunkVerificationFailed { hash: Hash },

    #[error("Invalid chunk size: {size}")]
    InvalidChunkSize { size: usize },

    /// URN-related errors
    #[error("Invalid URN format: {urn}")]
    InvalidUrn { urn: String },

    #[error("URN parsing failed: {reason}")]
    UrnParsingFailed { reason: String },

    #[error("Invalid byte range: {range}")]
    InvalidByteRange { range: String },

    /// Proof-related errors
    #[error("Proof generation failed: {reason}")]
    ProofGenerationFailed { reason: String },

    #[error("Proof verification failed")]
    ProofVerificationFailed,

    #[error("Invalid proof format: {reason}")]
    InvalidProofFormat { reason: String },

    /// Merkle tree errors
    #[error("Merkle tree construction failed: {reason}")]
    MerkleTreeFailed { reason: String },

    #[error("Invalid merkle proof")]
    InvalidMerkleProof,

    /// Compression errors
    #[error("Compression failed: {reason}")]
    CompressionFailed { reason: String },

    #[error("Decompression failed: {reason}")]
    DecompressionFailed { reason: String },

    /// Configuration errors
    #[error("Configuration error: {reason}")]
    ConfigurationError { reason: String },

    #[error("Home directory not found")]
    HomeDirectoryNotFound,

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization errors
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),

    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    /// Hex encoding/decoding errors
    #[error("Hex encoding error: {0}")]
    HexError(#[from] hex::FromHexError),

    /// Compression library errors  
    #[error("Zstd error: {message}")]
    ZstdError { message: String },

    /// UUID errors
    #[error("UUID error: {0}")]
    UuidError(#[from] uuid::Error),

    /// Time parsing errors
    #[error("Time parsing error: {0}")]
    TimeParsingError(#[from] chrono::ParseError),

    /// Generic error for unexpected conditions
    #[error("Internal error: {message}")]
    Internal { message: String },

    #[error("Invalid format for {format}: {reason}")]
    InvalidFormat { format: String, reason: String },

    #[error("Unsupported version {version}, supported: {supported}")]
    UnsupportedVersion { version: u32, supported: u32 },

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
}

impl DigstoreError {
    /// Create a new store not found error
    pub fn store_not_found(path: PathBuf) -> Self {
        Self::StoreNotFound { path }
    }

    /// Create a new store already exists error
    pub fn store_already_exists(path: PathBuf) -> Self {
        Self::StoreAlreadyExists { path }
    }

    /// Create a new invalid store ID error
    pub fn invalid_store_id(store_id: impl Into<String>) -> Self {
        Self::InvalidStoreId {
            store_id: store_id.into(),
        }
    }

    /// Create a new store corrupted error
    pub fn store_corrupted(reason: impl Into<String>) -> Self {
        Self::StoreCorrupted {
            reason: reason.into(),
        }
    }

    /// Create a new layer not found error
    pub fn layer_not_found(hash: Hash) -> Self {
        Self::LayerNotFound { hash }
    }

    /// Create a new invalid layer format error
    pub fn invalid_layer_format(reason: impl Into<String>) -> Self {
        Self::InvalidLayerFormat {
            reason: reason.into(),
        }
    }

    /// Create a new file not found error
    pub fn file_not_found(path: PathBuf) -> Self {
        Self::FileNotFound { path }
    }

    /// Create a new invalid file path error
    pub fn invalid_file_path(path: PathBuf) -> Self {
        Self::InvalidFilePath { path }
    }

    /// Create a new chunk not found error
    pub fn chunk_not_found(hash: Hash) -> Self {
        Self::ChunkNotFound { hash }
    }

    /// Create a new invalid URN error
    pub fn invalid_urn(urn: impl Into<String>) -> Self {
        Self::InvalidUrn { urn: urn.into() }
    }

    /// Create a new URN parsing failed error
    pub fn urn_parsing_failed(reason: impl Into<String>) -> Self {
        Self::UrnParsingFailed {
            reason: reason.into(),
        }
    }

    /// Create a new proof generation failed error
    pub fn proof_generation_failed(reason: impl Into<String>) -> Self {
        Self::ProofGenerationFailed {
            reason: reason.into(),
        }
    }

    /// Create a new internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

/// Result type alias for Digstore operations
pub type Result<T> = std::result::Result<T, DigstoreError>;
