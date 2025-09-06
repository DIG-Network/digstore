//! Configuration for datastore coin system

use crate::core::error::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Global configuration for datastore coins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatastoreCoinConfig {
    /// Network to use (mainnet, testnet)
    pub network: String,
    
    /// Whether to auto-create coins on commit
    pub auto_create_on_commit: bool,
    
    /// Whether to require coins for store operations
    pub require_collateral: bool,
    
    /// Minimum confirmations for coin transactions
    pub min_confirmations: u32,
    
    /// Custom collateral configuration
    pub collateral: Option<crate::datastore_coin::types::CollateralConfig>,
    
    /// CAT (Colored Coin) configuration
    pub cat_config: CatConfig,
}

/// CAT-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatConfig {
    /// DIG token asset ID on Chia blockchain
    pub dig_asset_id: String,
    
    /// Tail program hash for DIG CAT
    pub tail_program_hash: String,
    
    /// Whether to use simulator for testing
    pub use_simulator: bool,
}

impl Default for DatastoreCoinConfig {
    fn default() -> Self {
        Self {
            network: "mainnet".to_string(),
            auto_create_on_commit: false,
            require_collateral: false,
            min_confirmations: 32,
            collateral: None,
            cat_config: CatConfig::default(),
        }
    }
}

impl Default for CatConfig {
    fn default() -> Self {
        Self {
            // Official DIG CAT asset ID
            dig_asset_id: "6d95dae356e32a71db5ddcb42224754a02524c615c5fc35f568c2af04774e589".to_string(),
            tail_program_hash: "6d95dae356e32a71db5ddcb42224754a02524c615c5fc35f568c2af04774e589".to_string(),
            use_simulator: false,
        }
    }
}

impl DatastoreCoinConfig {
    /// Load configuration from file
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }
    
    /// Save configuration to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }
    
    /// Get configuration path
    pub fn default_path() -> Result<std::path::PathBuf> {
        let config_dir = directories::BaseDirs::new()
            .ok_or_else(|| crate::core::error::DigstoreError::internal("Could not determine config directory"))?
            .config_dir()
            .join("digstore");
        
        Ok(config_dir.join("datastore_coin.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_config_defaults() {
        let config = DatastoreCoinConfig::default();
        assert_eq!(config.network, "mainnet");
        assert!(!config.auto_create_on_commit);
        assert!(!config.require_collateral);
        assert_eq!(config.min_confirmations, 32);
    }
    
    #[test]
    fn test_config_save_load() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let config_path = temp_dir.path().join("test_config.toml");
        
        let mut config = DatastoreCoinConfig::default();
        config.network = "testnet".to_string();
        config.auto_create_on_commit = true;
        
        config.save(&config_path)?;
        
        let loaded = DatastoreCoinConfig::load(&config_path)?;
        assert_eq!(loaded.network, "testnet");
        assert!(loaded.auto_create_on_commit);
        
        Ok(())
    }
}