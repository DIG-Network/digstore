//! Encrypted archive wrapper that transforms storage addresses
//!
//! This wrapper handles URN transformation for zero-knowledge storage

use crate::config::GlobalConfig;
use crate::core::error::{DigstoreError, Result};
use crate::core::types::Hash;
use crate::crypto::{PublicKey, transform_urn};
use crate::storage::dig_archive::{self, DigArchive};
use crate::storage::layer::Layer;

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
            
            // Transform the URN
            let transformed_urn = transform_urn(&layer_urn, public_key)?;
            
            // Extract hash from transformed URN
            // Format is "urn:dig:transformed:<hex_hash>"
            let hash_str = transformed_urn
                .strip_prefix("urn:dig:transformed:")
                .ok_or_else(|| crate::core::error::DigstoreError::internal("Invalid transformed URN format"))?;
            
            Hash::from_hex(hash_str)
                .map_err(|_| DigstoreError::internal("Invalid hash in transformed URN"))
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
        
        self.archive.get_layer(&storage_hash)
    }
    
    /// Get raw layer data with optional transformation
    pub fn get_layer_data(&self, layer_hash: &Hash) -> Result<Vec<u8>> {
        let storage_hash = if self.is_encrypted() {
            self.transform_layer_hash(layer_hash)?
        } else {
            *layer_hash
        };
        
        self.archive.get_layer_data(&storage_hash)
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
}
