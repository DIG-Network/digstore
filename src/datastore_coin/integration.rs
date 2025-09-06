//! Integration between datastore coins and store operations

use crate::core::error::Result;
use crate::datastore_coin::{DatastoreCoinManager, DatastoreId, CoinState};
use crate::storage::Store;
use crate::wallet::WalletManager;
use std::path::Path;

/// Extension trait for Store to add coin functionality
pub trait StoreCoinExt {
    /// Create a datastore coin for this store
    fn create_coin(&self, wallet_profile: Option<String>) -> Result<String>;
    
    /// Get all coins associated with this store
    fn list_coins(&self) -> Result<Vec<crate::datastore_coin::DatastoreCoin>>;
    
    /// Check if store has an active coin
    fn has_active_coin(&self) -> Result<bool>;
}

impl StoreCoinExt for Store {
    fn create_coin(&self, wallet_profile: Option<String>) -> Result<String> {
        // Get coin manager
        let config_dir = directories::BaseDirs::new()
            .ok_or_else(|| crate::core::error::DigstoreError::internal("Could not determine config directory"))?
            .config_dir()
            .join("digstore")
            .join("coins");
        
        let coin_manager = DatastoreCoinManager::new(config_dir)?;
        
        // Get wallet
        let wallet_mgr = WalletManager::new_with_profile(wallet_profile)?;
        wallet_mgr.ensure_wallet_initialized()?;
        let wallet = wallet_mgr.get_wallet()?;
        
        // Get store info
        let root_hash = self.get_root_hash()?;
        let datastore_id = DatastoreId::from_hash(&root_hash);
        let size_bytes = self.calculate_total_size();
        
        // Create coin
        let coin = coin_manager.create_coin(
            datastore_id,
            root_hash,
            size_bytes,
            &wallet,
        )?;
        
        Ok(coin.id.to_string())
    }
    
    fn list_coins(&self) -> Result<Vec<crate::datastore_coin::DatastoreCoin>> {
        let config_dir = directories::BaseDirs::new()
            .ok_or_else(|| crate::core::error::DigstoreError::internal("Could not determine config directory"))?
            .config_dir()
            .join("digstore")
            .join("coins");
        
        let coin_manager = DatastoreCoinManager::new(config_dir)?;
        
        let root_hash = self.get_root_hash()?;
        let datastore_id = DatastoreId::from_hash(&root_hash);
        
        coin_manager.get_coins_by_datastore(&datastore_id)
            .or_else(|_| Ok(Vec::new()))
    }
    
    fn has_active_coin(&self) -> Result<bool> {
        let coins = self.list_coins()?;
        Ok(coins.iter().any(|c| c.state == CoinState::Active))
    }
}

/// Hook to automatically create coin on commit if configured
pub fn maybe_create_coin_on_commit(
    store: &Store,
    auto_create_coin: bool,
    wallet_profile: Option<String>,
) -> Result<Option<String>> {
    if !auto_create_coin {
        return Ok(None);
    }
    
    // Check if store already has active coin
    if store.has_active_coin()? {
        return Ok(None);
    }
    
    // Create coin
    match store.create_coin(wallet_profile) {
        Ok(coin_id) => {
            println!("Created datastore coin: {}", coin_id);
            Ok(Some(coin_id))
        }
        Err(e) => {
            eprintln!("Warning: Failed to create datastore coin: {}", e);
            Ok(None)
        }
    }
}

/// Verify store has required collateral before allowing operations
pub fn verify_store_collateral(store_path: &Path) -> Result<bool> {
    let store = Store::open(store_path)?;
    
    // For now, just check if store has any coins
    // In production, would verify active coin with sufficient collateral
    let coins = store.list_coins()?;
    Ok(!coins.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_store_coin_extension() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let store = Store::init(temp_dir.path())?;
        
        // Test listing coins on empty store
        let coins = store.list_coins()?;
        assert_eq!(coins.len(), 0);
        
        // Test has_active_coin
        assert!(!store.has_active_coin()?);
        
        Ok(())
    }
}