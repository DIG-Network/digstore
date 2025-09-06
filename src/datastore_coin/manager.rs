//! Datastore Coin Manager - Handles coin lifecycle and blockchain interactions

use crate::core::error::{DigstoreError, Result};
use crate::core::Hash;
use crate::datastore_coin::{
    coin::{CoinState, DatastoreCoin},
    collateral::{CollateralManager, CollateralRequirement},
    types::{CoinId, CoinMetadata, DatastoreId, CollateralConfig},
};
use crate::wallet::WalletManager;
use datalayer_driver::{ChiaDriver, OfferStore, OfferFile};
use dig_wallet::{Wallet, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;

/// Storage for datastore coins
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoinStorage {
    coins: HashMap<CoinId, DatastoreCoin>,
    by_datastore: HashMap<DatastoreId, Vec<CoinId>>,
    by_owner: HashMap<String, Vec<CoinId>>,
}

impl Default for CoinStorage {
    fn default() -> Self {
        Self {
            coins: HashMap::new(),
            by_datastore: HashMap::new(),
            by_owner: HashMap::new(),
        }
    }
}

/// Manages datastore coins and their lifecycle
pub struct DatastoreCoinManager {
    storage: Arc<Mutex<CoinStorage>>,
    collateral_manager: CollateralManager,
    storage_path: PathBuf,
    chia_driver: Option<ChiaDriver>,
    runtime: Runtime,
}

impl DatastoreCoinManager {
    /// Create a new DatastoreCoinManager
    pub fn new(storage_path: PathBuf) -> Result<Self> {
        // Ensure storage directory exists
        fs::create_dir_all(&storage_path)?;
        
        let coins_file = storage_path.join("coins.json");
        let storage = if coins_file.exists() {
            let data = fs::read_to_string(&coins_file)?;
            serde_json::from_str(&data)?
        } else {
            CoinStorage::default()
        };
        
        let runtime = Runtime::new().map_err(|e| {
            DigstoreError::internal(format!("Failed to create tokio runtime: {}", e))
        })?;
        
        Ok(Self {
            storage: Arc::new(Mutex::new(storage)),
            collateral_manager: CollateralManager::new(),
            storage_path,
            chia_driver: None,
            runtime,
        })
    }
    
    /// Initialize with Chia blockchain connection
    pub fn init_blockchain(&mut self, network: &str) -> Result<()> {
        let driver = self.runtime.block_on(async {
            ChiaDriver::new(network.to_string()).await
        }).map_err(|e| {
            DigstoreError::internal(format!("Failed to connect to Chia network: {}", e))
        })?;
        
        self.chia_driver = Some(driver);
        Ok(())
    }
    
    /// Create a new datastore coin
    pub fn create_coin(
        &self,
        datastore_id: DatastoreId,
        root_hash: Hash,
        size_bytes: u64,
        owner_wallet: &Wallet,
    ) -> Result<DatastoreCoin> {
        // Calculate collateral requirement
        let collateral_req = self.collateral_manager.calculate_requirement(size_bytes)?;
        
        // Get owner address from wallet
        let owner_address = self.runtime.block_on(async {
            owner_wallet.get_address().await
        }).map_err(|e| {
            DigstoreError::internal(format!("Failed to get wallet address: {}", e))
        })?;
        
        // Check wallet balance for DIG tokens
        let balance = self.check_dig_balance(owner_wallet)?;
        if balance < collateral_req.total_amount {
            return Err(DigstoreError::ValidationError {
                field: "balance".to_string(),
                reason: format!(
                    "Insufficient DIG token balance. Required: {}, Available: {}",
                    collateral_req.total_amount, balance
                ),
            });
        }
        
        // Create the coin
        let coin = DatastoreCoin::new(
            datastore_id.clone(),
            root_hash,
            size_bytes,
            collateral_req.total_amount,
            owner_address.clone(),
        );
        
        // Store the coin
        let mut storage = self.storage.lock().unwrap();
        storage.coins.insert(coin.id.clone(), coin.clone());
        storage.by_datastore
            .entry(datastore_id)
            .or_insert_with(Vec::new)
            .push(coin.id.clone());
        storage.by_owner
            .entry(owner_address)
            .or_insert_with(Vec::new)
            .push(coin.id.clone());
        drop(storage);
        
        // Persist storage
        self.save_storage()?;
        
        Ok(coin)
    }
    
    /// Mint a coin on the blockchain
    pub fn mint_coin(
        &self,
        coin_id: &CoinId,
        owner_wallet: &Wallet,
    ) -> Result<String> {
        let mut storage = self.storage.lock().unwrap();
        let coin = storage.coins.get_mut(coin_id)
            .ok_or_else(|| DigstoreError::NotFound {
                resource: "coin".to_string(),
                identifier: coin_id.to_string(),
            })?;
        
        if coin.state != CoinState::Pending {
            return Err(DigstoreError::ValidationError {
                field: "state".to_string(),
                reason: "Coin must be in pending state to mint".to_string(),
            });
        }
        
        let chia_driver = self.chia_driver.as_ref()
            .ok_or_else(|| DigstoreError::internal("Blockchain not initialized"))?;
        
        // Create offer for DIG token collateral
        let offer_result = self.runtime.block_on(async {
            // Lock DIG tokens as collateral
            let offer = chia_driver.create_offer(
                &coin.metadata.owner_address,
                coin.metadata.collateral_amount,
                "DIG",
            ).await?;
            
            // Submit to blockchain
            chia_driver.submit_offer(offer).await
        }).map_err(|e| {
            DigstoreError::internal(format!("Failed to create blockchain offer: {}", e))
        })?;
        
        // Update coin with blockchain info
        coin.set_blockchain_info(offer_result.tx_id.clone(), offer_result.block_height);
        drop(storage);
        
        // Persist storage
        self.save_storage()?;
        
        Ok(offer_result.tx_id)
    }
    
    /// Get a coin by ID
    pub fn get_coin(&self, coin_id: &CoinId) -> Result<DatastoreCoin> {
        let storage = self.storage.lock().unwrap();
        storage.coins.get(coin_id)
            .cloned()
            .ok_or_else(|| DigstoreError::NotFound {
                resource: "coin".to_string(),
                identifier: coin_id.to_string(),
            })
    }
    
    /// Get all coins for a datastore
    pub fn get_coins_by_datastore(&self, datastore_id: &DatastoreId) -> Result<Vec<DatastoreCoin>> {
        let storage = self.storage.lock().unwrap();
        let coin_ids = storage.by_datastore.get(datastore_id)
            .ok_or_else(|| DigstoreError::NotFound {
                resource: "datastore coins".to_string(),
                identifier: datastore_id.to_string(),
            })?;
        
        let coins: Vec<_> = coin_ids.iter()
            .filter_map(|id| storage.coins.get(id).cloned())
            .collect();
        
        Ok(coins)
    }
    
    /// Get all coins owned by an address
    pub fn get_coins_by_owner(&self, owner_address: &str) -> Result<Vec<DatastoreCoin>> {
        let storage = self.storage.lock().unwrap();
        let coin_ids = storage.by_owner.get(owner_address)
            .ok_or_else(|| DigstoreError::NotFound {
                resource: "owner coins".to_string(),
                identifier: owner_address.to_string(),
            })?;
        
        let coins: Vec<_> = coin_ids.iter()
            .filter_map(|id| storage.coins.get(id).cloned())
            .collect();
        
        Ok(coins)
    }
    
    /// Transfer coin ownership
    pub fn transfer_coin(
        &self,
        coin_id: &CoinId,
        from_wallet: &Wallet,
        to_address: &str,
    ) -> Result<()> {
        let mut storage = self.storage.lock().unwrap();
        let coin = storage.coins.get_mut(coin_id)
            .ok_or_else(|| DigstoreError::NotFound {
                resource: "coin".to_string(),
                identifier: coin_id.to_string(),
            })?;
        
        // Verify ownership
        let from_address = self.runtime.block_on(async {
            from_wallet.get_address().await
        }).map_err(|e| {
            DigstoreError::internal(format!("Failed to get wallet address: {}", e))
        })?;
        
        if coin.metadata.owner_address != from_address {
            return Err(DigstoreError::ValidationError {
                field: "owner".to_string(),
                reason: "Only the owner can transfer a coin".to_string(),
            });
        }
        
        if coin.state != CoinState::Active {
            return Err(DigstoreError::ValidationError {
                field: "state".to_string(),
                reason: "Only active coins can be transferred".to_string(),
            });
        }
        
        // Update ownership in blockchain
        if let Some(chia_driver) = &self.chia_driver {
            self.runtime.block_on(async {
                chia_driver.transfer_coin(
                    &coin.tx_id.as_ref().unwrap(),
                    &from_address,
                    to_address,
                ).await
            }).map_err(|e| {
                DigstoreError::internal(format!("Failed to transfer coin on blockchain: {}", e))
            })?;
        }
        
        // Update local storage
        let old_owner = coin.metadata.owner_address.clone();
        coin.metadata.owner_address = to_address.to_string();
        
        // Update indexes
        if let Some(owner_coins) = storage.by_owner.get_mut(&old_owner) {
            owner_coins.retain(|id| id != coin_id);
        }
        storage.by_owner
            .entry(to_address.to_string())
            .or_insert_with(Vec::new)
            .push(coin_id.clone());
        
        drop(storage);
        
        // Persist storage
        self.save_storage()?;
        
        Ok(())
    }
    
    /// Spend a coin to release collateral
    pub fn spend_coin(
        &self,
        coin_id: &CoinId,
        owner_wallet: &Wallet,
    ) -> Result<u64> {
        let mut storage = self.storage.lock().unwrap();
        let coin = storage.coins.get_mut(coin_id)
            .ok_or_else(|| DigstoreError::NotFound {
                resource: "coin".to_string(),
                identifier: coin_id.to_string(),
            })?;
        
        // Verify ownership
        let owner_address = self.runtime.block_on(async {
            owner_wallet.get_address().await
        }).map_err(|e| {
            DigstoreError::internal(format!("Failed to get wallet address: {}", e))
        })?;
        
        if coin.metadata.owner_address != owner_address {
            return Err(DigstoreError::ValidationError {
                field: "owner".to_string(),
                reason: "Only the owner can spend a coin".to_string(),
            });
        }
        
        if coin.state != CoinState::Active && coin.state != CoinState::Expired {
            return Err(DigstoreError::ValidationError {
                field: "state".to_string(),
                reason: "Coin must be active or expired to spend".to_string(),
            });
        }
        
        // Calculate refundable amount
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() - coin.metadata.created_at;
        
        let refund_amount = self.collateral_manager.calculate_refund(
            coin.metadata.collateral_amount,
            elapsed,
        );
        
        if refund_amount == 0 {
            return Err(DigstoreError::ValidationError {
                field: "grace_period".to_string(),
                reason: "Coin is still in grace period, no refund available".to_string(),
            });
        }
        
        // Release collateral on blockchain
        if let Some(chia_driver) = &self.chia_driver {
            self.runtime.block_on(async {
                chia_driver.release_collateral(
                    &coin.tx_id.as_ref().unwrap(),
                    &owner_address,
                    refund_amount,
                ).await
            }).map_err(|e| {
                DigstoreError::internal(format!("Failed to release collateral: {}", e))
            })?;
        }
        
        // Update coin state
        coin.update_state(CoinState::Spent);
        drop(storage);
        
        // Persist storage
        self.save_storage()?;
        
        Ok(refund_amount)
    }
    
    /// Check DIG token balance for a wallet
    fn check_dig_balance(&self, wallet: &Wallet) -> Result<u64> {
        self.runtime.block_on(async {
            wallet.get_cat_balance("DIG").await
        }).map_err(|e| {
            DigstoreError::internal(format!("Failed to check DIG balance: {}", e))
        })
    }
    
    /// Save storage to disk
    fn save_storage(&self) -> Result<()> {
        let storage = self.storage.lock().unwrap();
        let data = serde_json::to_string_pretty(&*storage)?;
        let coins_file = self.storage_path.join("coins.json");
        fs::write(coins_file, data)?;
        Ok(())
    }
    
    /// Get collateral requirement for a datastore size
    pub fn get_collateral_requirement(&self, size_bytes: u64) -> Result<CollateralRequirement> {
        self.collateral_manager.calculate_requirement(size_bytes)
    }
    
    /// Update collateral configuration
    pub fn update_collateral_config(&mut self, config: CollateralConfig) {
        self.collateral_manager.update_config(config);
    }
    
    /// List all coins with optional filtering
    pub fn list_coins(&self, active_only: bool) -> Vec<DatastoreCoin> {
        let storage = self.storage.lock().unwrap();
        storage.coins.values()
            .filter(|coin| !active_only || coin.is_active())
            .cloned()
            .collect()
    }
    
    /// Get statistics about coins
    pub fn get_stats(&self) -> CoinStats {
        let storage = self.storage.lock().unwrap();
        let coins: Vec<&DatastoreCoin> = storage.coins.values().collect();
        
        let total_coins = coins.len();
        let active_coins = coins.iter().filter(|c| c.is_active()).count();
        let total_collateral: u64 = coins.iter()
            .filter(|c| c.is_active())
            .map(|c| c.metadata.collateral_amount)
            .sum();
        let total_storage: u64 = coins.iter()
            .filter(|c| c.is_active())
            .map(|c| c.metadata.size_bytes)
            .sum();
        
        CoinStats {
            total_coins,
            active_coins,
            pending_coins: coins.iter().filter(|c| c.state == CoinState::Pending).count(),
            expired_coins: coins.iter().filter(|c| c.state == CoinState::Expired).count(),
            spent_coins: coins.iter().filter(|c| c.state == CoinState::Spent).count(),
            total_collateral_locked: total_collateral,
            total_storage_bytes: total_storage,
        }
    }
}

/// Statistics about datastore coins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinStats {
    pub total_coins: usize,
    pub active_coins: usize,
    pub pending_coins: usize,
    pub expired_coins: usize,
    pub spent_coins: usize,
    pub total_collateral_locked: u64,
    pub total_storage_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = DatastoreCoinManager::new(temp_dir.path().to_path_buf()).unwrap();
        assert_eq!(manager.list_coins(false).len(), 0);
    }
    
    #[test]
    fn test_coin_lifecycle() {
        let temp_dir = TempDir::new().unwrap();
        let manager = DatastoreCoinManager::new(temp_dir.path().to_path_buf()).unwrap();
        
        // Create mock wallet
        let runtime = Runtime::new().unwrap();
        let wallet = runtime.block_on(async {
            Wallet::create_new_wallet("test").await.unwrap();
            Wallet::load(Some("test".to_string()), false).await.unwrap()
        });
        
        // Test coin creation (would need mock DIG balance in production)
        let datastore_id = DatastoreId::new("test_datastore".to_string());
        let root_hash = Hash::from_bytes([1; 32]);
        let size = 1024 * 1024; // 1 MB
        
        // Note: In production, this would check actual DIG balance
        // For testing, we'd need to mock the balance check
    }
}