//! Store-specific configuration management
//!
//! Provides per-store configuration stored in ~/.dig/{store_id}.config.toml

use crate::core::error::{DigstoreError, Result};
use crate::core::types::StoreId;
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Store-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoreConfig {
    /// Custom encryption key for this store (hex encoded)
    pub custom_encryption_key: Option<String>,
    /// Store creation timestamp
    pub created_at: Option<String>,
    /// Store name/description
    pub name: Option<String>,
}

impl StoreConfig {
    /// Load store configuration from disk
    pub fn load(store_id: &StoreId) -> Result<Self> {
        let config_path = Self::get_config_path(store_id)?;

        if !config_path.exists() {
            // Return default configuration if file doesn't exist
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: StoreConfig =
            toml::from_str(&content).map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to parse store config: {}", e),
            })?;

        Ok(config)
    }

    /// Save store configuration to disk
    pub fn save(&self, store_id: &StoreId) -> Result<()> {
        let config_path = Self::get_config_path(store_id)?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content =
            toml::to_string_pretty(self).map_err(|e| DigstoreError::ConfigurationError {
                reason: format!("Failed to serialize store config: {}", e),
            })?;

        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get the path to the store configuration file
    pub fn get_config_path(store_id: &StoreId) -> Result<PathBuf> {
        let user_dirs = UserDirs::new().ok_or(DigstoreError::HomeDirectoryNotFound)?;

        let dig_dir = user_dirs.home_dir().join(".dig");
        Ok(dig_dir.join(format!("{}.config.toml", store_id.to_hex())))
    }

    /// Check if store uses custom encryption
    pub fn has_custom_encryption(&self) -> bool {
        self.custom_encryption_key.is_some()
    }

    /// Get the custom encryption key
    pub fn get_custom_encryption_key(&self) -> Option<String> {
        self.custom_encryption_key.clone()
    }

    /// Set custom encryption key
    pub fn set_custom_encryption_key(&mut self, key: Option<String>) {
        self.custom_encryption_key = key;
    }
}
