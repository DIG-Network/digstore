//! Core datastore coin implementation

use crate::core::{error::Result, Hash};
use crate::datastore_coin::types::{CoinId, CoinMetadata, DatastoreId};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// State of a datastore coin
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoinState {
    /// Coin is active and datastore is available
    Active,
    /// Coin is pending creation on blockchain
    Pending,
    /// Coin has expired but is in grace period
    Expired,
    /// Coin has been spent/destroyed
    Spent,
    /// Coin is invalid or corrupted
    Invalid,
}

/// Represents a datastore coin on the blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatastoreCoin {
    /// Unique identifier for this coin
    pub id: CoinId,
    
    /// Current state of the coin
    pub state: CoinState,
    
    /// Metadata associated with this coin
    pub metadata: CoinMetadata,
    
    /// Blockchain transaction ID (if minted)
    pub tx_id: Option<String>,
    
    /// Block height when minted
    pub block_height: Option<u64>,
}

impl DatastoreCoin {
    /// Create a new datastore coin
    pub fn new(
        datastore_id: DatastoreId,
        root_hash: Hash,
        size_bytes: u64,
        collateral_amount: u64,
        owner_address: String,
    ) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let metadata = CoinMetadata {
            datastore_id: datastore_id.clone(),
            root_hash,
            size_bytes,
            collateral_amount,
            owner_address,
            host_address: None,
            created_at,
            expires_at: None,
            extra: None,
        };
        
        Self {
            id: CoinId::new(format!("dsc_{}", uuid::Uuid::new_v4())),
            state: CoinState::Pending,
            metadata,
            tx_id: None,
            block_height: None,
        }
    }
    
    /// Check if the coin is valid and active
    pub fn is_active(&self) -> bool {
        self.state == CoinState::Active
    }
    
    /// Check if the coin has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.metadata.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            now > expires_at
        } else {
            false
        }
    }
    
    /// Update the coin state
    pub fn update_state(&mut self, new_state: CoinState) {
        self.state = new_state;
    }
    
    /// Set blockchain transaction details
    pub fn set_blockchain_info(&mut self, tx_id: String, block_height: u64) {
        self.tx_id = Some(tx_id);
        self.block_height = Some(block_height);
        self.state = CoinState::Active;
    }
    
    /// Get the collateral amount in DIG tokens
    pub fn get_collateral_amount(&self) -> u64 {
        self.metadata.collateral_amount
    }
    
    /// Get the datastore size in bytes
    pub fn get_size_bytes(&self) -> u64 {
        self.metadata.size_bytes
    }
    
    /// Get the owner address
    pub fn get_owner_address(&self) -> &str {
        &self.metadata.owner_address
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_coin_creation() {
        let coin = DatastoreCoin::new(
            DatastoreId::new("test_datastore".to_string()),
            Hash::from_bytes([1; 32]),
            1024 * 1024, // 1 MB
            1000000, // 1M mojos
            "xch1test...".to_string(),
        );
        
        assert_eq!(coin.state, CoinState::Pending);
        assert_eq!(coin.get_size_bytes(), 1024 * 1024);
        assert_eq!(coin.get_collateral_amount(), 1000000);
    }
    
    #[test]
    fn test_coin_state_transitions() {
        let mut coin = DatastoreCoin::new(
            DatastoreId::new("test".to_string()),
            Hash::from_bytes([1; 32]),
            1024,
            1000,
            "xch1test".to_string(),
        );
        
        assert_eq!(coin.state, CoinState::Pending);
        assert!(!coin.is_active());
        
        coin.set_blockchain_info("tx123".to_string(), 100);
        assert_eq!(coin.state, CoinState::Active);
        assert!(coin.is_active());
        
        coin.update_state(CoinState::Spent);
        assert_eq!(coin.state, CoinState::Spent);
        assert!(!coin.is_active());
    }
}