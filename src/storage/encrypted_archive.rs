//! Encrypted archive wrapper that transforms storage addresses
//!
//! This wrapper handles URN transformation for zero-knowledge storage

use crate::config::GlobalConfig;
use crate::core::error::{DigstoreError, Result};
use crate::core::types::Hash;
use crate::crypto::{PublicKey, transform_urn};
use crate::storage::dig_archive::{self, DigArchive};
use crate::storage::layer::Layer;
use crate::core::types::{FileEntry, FileMetadata};
use std::path::PathBuf;

/// Wrapper around DigArchive that handles address transformation
pub struct EncryptedArchive {
    archive: DigArchive,
    public_key: Option<PublicKey>,
    encrypted_storage: bool,
}

impl EncryptedArchive {
    /// Create new encrypted archive wrapper
    pub fn new(archive: DigArchive) -> Result<Self> {
        let config = GlobalConfig::load()?;
        
        let public_key = if let Some(hex_key) = config.crypto.public_key {
            Some(PublicKey::from_hex(&hex_key)?)
        } else {
            None
        };
        
        let encrypted_storage = config.crypto.encrypted_storage.unwrap_or(false);
        
        Ok(Self {
            archive,
            public_key,
            encrypted_storage,
        })
    }
    
    /// Check if transformation is enabled
    pub fn is_encrypted(&self) -> bool {
        self.encrypted_storage && self.public_key.is_some()
    }
    
    /// Transform a layer hash for storage
    fn transform_layer_hash(&self, layer_hash: &Hash) -> Result<Hash> {
        if let Some(public_key) = &self.public_key {
            // Create URN from layer hash
            let layer_urn = format!("urn:dig:layer:{}", layer_hash.to_hex());
            
            // Transform the URN (returns raw hex string, not URN format)
            let transformed_hex = transform_urn(&layer_urn, public_key)?;
            
            // Parse the hex string directly as the new hash
            Hash::from_hex(&transformed_hex)
                .map_err(|_| DigstoreError::internal("Invalid hash from URN transformation"))
        } else {
            Ok(*layer_hash)
        }
    }
    
    /// Add a layer with optional transformation
    pub fn add_layer(&mut self, layer_hash: Hash, layer_data: &[u8]) -> Result<()> {
        let storage_hash = if self.is_encrypted() {
            self.transform_layer_hash(&layer_hash)?
        } else {
            layer_hash
        };
        
        self.archive.add_layer(storage_hash, layer_data)
    }
    
    /// Get a layer with optional transformation
    pub fn get_layer(&self, layer_hash: &Hash) -> Result<Layer> {
        let storage_hash = if self.is_encrypted() {
            self.transform_layer_hash(layer_hash)?
        } else {
            *layer_hash
        };
        
        // Try to get the layer, but if it fails and encryption is enabled,
        // return deterministic random data instead of error
        match self.archive.get_layer(&storage_hash) {
            Ok(layer) => Ok(layer),
            Err(_) if self.is_encrypted() => {
                // Generate deterministic random layer data based on the original layer hash
                self.generate_random_layer_data(layer_hash)
            }
            Err(e) => Err(e),
        }
    }
    
    /// Get raw layer data with optional transformation
    pub fn get_layer_data(&self, layer_hash: &Hash) -> Result<Vec<u8>> {
        let storage_hash = if self.is_encrypted() {
            self.transform_layer_hash(layer_hash)?
        } else {
            *layer_hash
        };
        
        // Try to get the layer data, but if it fails and encryption is enabled,
        // return deterministic random data instead of error
        match self.archive.get_layer_data(&storage_hash) {
            Ok(data) => Ok(data),
            Err(_) if self.is_encrypted() => {
                // Generate deterministic random data based on the original layer hash
                Ok(self.generate_random_data_for_hash(layer_hash))
            }
            Err(e) => Err(e),
        }
    }
    
    /// Check if layer exists with optional transformation
    pub fn has_layer(&self, layer_hash: &Hash) -> bool {
        let storage_hash = if self.is_encrypted() {
            match self.transform_layer_hash(layer_hash) {
                Ok(hash) => hash,
                Err(_) => return false,
            }
        } else {
            *layer_hash
        };
        
        self.archive.has_layer(&storage_hash)
    }
    
    /// Get layer count
    pub fn layer_count(&self) -> usize {
        self.archive.layer_count()
    }
    
    /// List layers (returns original hashes, not transformed)
    pub fn list_layers(&self) -> Vec<(Hash, &dig_archive::LayerIndexEntry)> {
        // This is complex because we need to reverse the transformation
        // For now, return the raw list
        self.archive.list_layers()
    }
    
    /// Flush changes
    pub fn flush(&mut self) -> Result<()> {
        self.archive.flush()
    }
    
    /// Get the path of the archive file
    pub fn path(&self) -> &std::path::Path {
        self.archive.path()
    }
    
    /// Generate deterministic random data for invalid content addresses
    fn generate_random_data_for_hash(&self, layer_hash: &Hash) -> Vec<u8> {
        use sha2::{Sha256, Digest};
        
        // Use the layer hash as seed for deterministic random generation
        let seed = format!("invalid_content_address:{}", layer_hash.to_hex());
        let default_size = 1024 * 1024; // 1MB default
        
        let mut result = Vec::with_capacity(default_size);
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let mut counter = 0u64;
        
        while result.len() < default_size {
            let mut current_hasher = hasher.clone();
            current_hasher.update(&counter.to_le_bytes());
            let hash = current_hasher.finalize();
            
            let bytes_needed = default_size - result.len();
            let bytes_to_copy = bytes_needed.min(hash.len());
            result.extend_from_slice(&hash[..bytes_to_copy]);
            
            counter += 1;
        }
        
        result
    }
    
    /// Generate deterministic random layer for invalid content addresses
    fn generate_random_layer_data(&self, layer_hash: &Hash) -> Result<Layer> {
        use crate::core::types::{LayerType, LayerMetadata, FileEntry, FileMetadata};
        use std::path::PathBuf;
        
        // Create a fake layer with random data that appears legitimate
        let mut layer = Layer::new(crate::core::types::LayerType::Full, 1, Hash::zero());
        
        // Add fake file entries with deterministic random properties
        let random_data = self.generate_random_data_for_hash(layer_hash);
        let chunk_size = 64 * 1024; // 64KB chunks
        
        for i in 0..(random_data.len() / chunk_size).min(10) {
            let start = i * chunk_size;
            let end = (start + chunk_size).min(random_data.len());
            let chunk_data = &random_data[start..end];
            
            let chunk_hash = crate::core::hash::sha256(chunk_data);
            let fake_file_path = PathBuf::from(format!("fake_file_{}.dat", i));
            
            let file_entry = FileEntry {
                path: fake_file_path,
                hash: chunk_hash,
                size: chunk_data.len() as u64,
                chunks: vec![crate::core::types::ChunkRef {
                    hash: chunk_hash,
                    offset: 0,
                    size: chunk_data.len() as u32,
                }],
                metadata: FileMetadata {
                    mode: 0o644,
                    modified: chrono::Utc::now().timestamp(),
                    is_new: true,
                    is_modified: false,
                    is_deleted: false,
                },
            };
            
            layer.add_file(file_entry);
            
            let chunk = crate::core::types::Chunk {
                hash: chunk_hash,
                offset: 0,
                size: chunk_data.len() as u32,
                data: chunk_data.to_vec(),
            };
            
            layer.add_chunk(chunk);
        }
        
        // Set fake metadata
        layer.metadata.message = Some("Generated random layer".to_string());
        layer.metadata.author = Some("system".to_string());
        
        Ok(layer)
    }
}
