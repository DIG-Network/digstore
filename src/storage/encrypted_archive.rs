//! Encrypted archive wrapper that transforms storage addresses
//!
//! This wrapper handles URN transformation for zero-knowledge storage

use crate::cli::context::CliContext;
use crate::config::GlobalConfig;
use crate::core::error::{DigstoreError, Result};
use crate::core::types::Hash;
use crate::crypto::{transform_urn, PublicKey};
use crate::storage::dig_archive::{self, DigArchive};
use crate::storage::layer::Layer;
use crate::wallet::WalletManager;

/// Wrapper around DigArchive that handles address transformation
pub struct EncryptedArchive {
    archive: DigArchive,
    public_key: Option<PublicKey>,
    custom_encryption_key: Option<String>,
    encrypted_storage: bool,
}

impl EncryptedArchive {
    /// Create new encrypted archive wrapper
    pub fn new(archive: DigArchive) -> Result<Self> {
        Self::new_with_store_id(archive, None)
    }

    /// Create new encrypted archive wrapper with store ID for config loading
    pub fn new_with_store_id(
        archive: DigArchive,
        store_id: Option<&crate::core::types::StoreId>,
    ) -> Result<Self> {
        let config = GlobalConfig::load()?;

        // Try to get public key from wallet (using CLI context profile), fallback to config
        let wallet_profile = CliContext::get_wallet_profile();
        let public_key = match WalletManager::get_wallet_public_key(wallet_profile) {
            Ok(wallet_key) => Some(wallet_key),
            Err(_) => {
                // Fallback to config for backward compatibility
                if let Some(hex_key) = config.crypto.public_key {
                    Some(PublicKey::from_hex(&hex_key)?)
                } else {
                    None
                }
            },
        };

        let encrypted_storage = config.crypto.encrypted_storage.unwrap_or(true);

        // Check for custom encryption/decryption key from multiple sources:
        // 1. CLI context (for current command)
        // 2. Store config (for persistent store-wide encryption)
        let custom_encryption_key = CliContext::get_custom_encryption_key()
            .or_else(CliContext::get_custom_decryption_key)
            .or_else(|| {
                // Load from store config if available
                if let Some(store_id) = store_id {
                    crate::config::StoreConfig::load(store_id)
                        .ok()
                        .and_then(|config| config.get_custom_encryption_key())
                } else {
                    None
                }
            });

        Ok(Self {
            archive,
            public_key,
            custom_encryption_key,
            encrypted_storage,
        })
    }

    /// Check if transformation is enabled
    pub fn is_encrypted(&self) -> bool {
        self.encrypted_storage
            && (self.public_key.is_some() || self.custom_encryption_key.is_some())
    }

    /// Get the encryption key to use (custom key takes priority over wallet public key)
    pub fn get_encryption_key(&self) -> Option<String> {
        self.custom_encryption_key.clone()
    }

    /// Check if using custom encryption (not wallet-based)
    pub fn is_using_custom_encryption(&self) -> bool {
        self.custom_encryption_key.is_some()
    }

    /// Check if the archive was created with custom encryption (from header flag)
    pub fn archive_has_custom_encryption(&self) -> bool {
        self.archive.has_custom_encryption()
    }

    /// Check if we need a custom decryption key to read this archive
    pub fn requires_custom_decryption_key(&self) -> bool {
        self.archive_has_custom_encryption() && self.custom_encryption_key.is_none()
    }

    /// Transform a layer hash for storage
    fn transform_layer_hash(&self, layer_hash: &Hash) -> Result<Hash> {
        if let Some(custom_key) = &self.custom_encryption_key {
            // For custom encryption, use the key directly as a transformation seed
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(layer_hash.as_bytes());
            hasher.update(custom_key.as_bytes());
            hasher.update(b"layer_transform");
            let transformed_bytes = hasher.finalize();
            Ok(Hash::from_bytes(transformed_bytes.into()))
        } else if let Some(public_key) = &self.public_key {
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
        // Set custom encryption flag in archive header if using custom key
        if self.custom_encryption_key.is_some() {
            self.archive.set_custom_encryption(true)?;
        }

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
            },
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
            },
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

    /// Generate a deterministic random file size that looks realistic
    /// This produces sizes that follow common file size patterns to make decoys indistinguishable
    /// from real content based on size alone.
    fn generate_deterministic_random_size(&self, seed: &str) -> usize {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        hasher.update(b"size_generation");
        let hash = hasher.finalize();

        // Use first 8 bytes as a u64 for randomness
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&hash[0..8]);
        let random_value = u64::from_le_bytes(bytes);

        // Create realistic file size distribution:
        // 40% small files (1KB - 100KB)
        // 35% medium files (100KB - 1MB)
        // 20% large files (1MB - 10MB)
        // 5% very large files (10MB - 20MB)

        let size_category = random_value % 100;
        let size_random = (random_value >> 8) % 1000000; // Use remaining bits for size within category

        match size_category {
            0..=39 => {
                // Small files: 1KB - 100KB
                let base = 1024; // 1KB
                let range = 99 * 1024; // up to 100KB
                base + (size_random % range) as usize
            },
            40..=74 => {
                // Medium files: 100KB - 1MB
                let base = 100 * 1024; // 100KB
                let range = 924 * 1024; // up to 1MB
                base + (size_random % range as u64) as usize
            },
            75..=94 => {
                // Large files: 1MB - 10MB
                let base = 1024 * 1024; // 1MB
                let range = 9 * 1024 * 1024; // up to 10MB
                base + (size_random % range as u64) as usize
            },
            _ => {
                // Very large files: 10MB - 20MB
                let base = 10 * 1024 * 1024; // 10MB
                let range = 10 * 1024 * 1024; // up to 20MB
                base + (size_random % range as u64) as usize
            },
        }
    }

    /// Generate deterministic random data for invalid content addresses
    fn generate_random_data_for_hash(&self, layer_hash: &Hash) -> Vec<u8> {
        use sha2::{Digest, Sha256};

        // Use the layer hash as seed for deterministic random generation
        let seed = format!("invalid_content_address:{}", layer_hash.to_hex());
        let random_size = self.generate_deterministic_random_size(&seed);

        let mut result = Vec::with_capacity(random_size);
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let mut counter = 0u64;

        while result.len() < random_size {
            let mut current_hasher = hasher.clone();
            current_hasher.update(counter.to_le_bytes());
            let hash = current_hasher.finalize();

            let bytes_needed = random_size - result.len();
            let bytes_to_copy = bytes_needed.min(hash.len());
            result.extend_from_slice(&hash[..bytes_to_copy]);

            counter += 1;
        }

        result
    }

    /// Generate deterministic random layer for invalid content addresses
    fn generate_random_layer_data(&self, layer_hash: &Hash) -> Result<Layer> {
        use crate::core::types::{FileEntry, FileMetadata};
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
