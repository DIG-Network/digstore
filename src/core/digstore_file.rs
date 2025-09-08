//! .digstore file management

use crate::core::{error::*, types::*};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Contents of a .digstore file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigstoreFile {
    /// Format version
    pub version: String,
    /// Store ID this project links to
    pub store_id: String,
    /// Whether encryption is enabled (always false for digstore_min)
    pub encrypted: bool,
    /// When the link was created
    pub created_at: String,
    /// Last access time
    pub last_accessed: String,
    /// Optional repository name
    pub repository_name: Option<String>,
}

impl DigstoreFile {
    /// Create a new .digstore file configuration
    pub fn new(store_id: StoreId, repository_name: Option<String>) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            version: "1.0.0".to_string(),
            store_id: store_id.to_hex(),
            encrypted: false,
            created_at: now.clone(),
            last_accessed: now,
            repository_name,
        }
    }

    /// Load .digstore file from disk
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(DigstoreError::Io)?;

        let digstore_file: DigstoreFile =
            toml::from_str(&content).map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to parse .digstore file: {}", e),
            })?;

        // Validate the format
        if digstore_file.version != "1.0.0" {
            return Err(DigstoreError::ConfigurationError {
                reason: format!("Unsupported .digstore version: {}", digstore_file.version),
            });
        }

        // Validate store ID format
        if digstore_file.store_id.len() != 64 {
            return Err(DigstoreError::invalid_store_id(format!(
                "Store ID must be 64 hex characters, got {}",
                digstore_file.store_id.len()
            )));
        }

        Ok(digstore_file)
    }

    /// Save .digstore file to disk
    pub fn save(&self, path: &Path) -> Result<()> {
        let content =
            toml::to_string_pretty(self).map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to serialize .digstore file: {}", e),
            })?;

        std::fs::write(path, content).map_err(DigstoreError::Io)?;

        Ok(())
    }

    /// Update last accessed time
    pub fn update_last_accessed(&mut self) {
        self.last_accessed = Utc::now().to_rfc3339();
    }

    /// Get the store ID as a Hash
    pub fn get_store_id(&self) -> Result<StoreId> {
        Hash::from_hex(&self.store_id).map_err(|_| {
            DigstoreError::invalid_store_id(format!(
                "Invalid store ID in .digstore file: {}",
                self.store_id
            ))
        })
    }

    /// Check if this .digstore file is valid
    pub fn is_valid(&self) -> bool {
        self.version == "1.0.0" 
            && self.store_id.len() == 64
            && !self.encrypted // digstore_min doesn't support encryption
            && Hash::from_hex(&self.store_id).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_digstore_file_creation() {
        let store_id =
            Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")
                .unwrap();
        let digstore_file = DigstoreFile::new(store_id, Some("test-repo".to_string()));

        assert_eq!(digstore_file.version, "1.0.0");
        assert_eq!(
            digstore_file.store_id,
            "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2"
        );
        assert_eq!(digstore_file.encrypted, false);
        assert_eq!(digstore_file.repository_name, Some("test-repo".to_string()));
        assert!(digstore_file.is_valid());
    }

    #[test]
    fn test_digstore_file_roundtrip() -> Result<()> {
        let store_id =
            Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")
                .unwrap();
        let original = DigstoreFile::new(store_id, Some("test-repo".to_string()));

        let mut temp_file = NamedTempFile::new().unwrap();
        original.save(temp_file.path())?;

        let loaded = DigstoreFile::load(temp_file.path())?;

        assert_eq!(original.version, loaded.version);
        assert_eq!(original.store_id, loaded.store_id);
        assert_eq!(original.encrypted, loaded.encrypted);
        assert_eq!(original.repository_name, loaded.repository_name);

        Ok(())
    }

    #[test]
    fn test_get_store_id() -> Result<()> {
        let store_id =
            Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")
                .unwrap();
        let digstore_file = DigstoreFile::new(store_id, None);

        let retrieved_id = digstore_file.get_store_id()?;
        assert_eq!(store_id, retrieved_id);

        Ok(())
    }

    #[test]
    fn test_update_last_accessed() {
        let store_id =
            Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2")
                .unwrap();
        let mut digstore_file = DigstoreFile::new(store_id, None);

        let original_time = digstore_file.last_accessed.clone();

        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));

        digstore_file.update_last_accessed();
        assert_ne!(original_time, digstore_file.last_accessed);
    }

    #[test]
    fn test_invalid_digstore_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "invalid toml content").unwrap();
        temp_file.flush().unwrap();

        let result = DigstoreFile::load(temp_file.path());
        assert!(result.is_err());
    }
}
