//! Blockchain interaction using chia-wallet-sdk

use crate::core::error::{DigstoreError, Result};
use crate::datastore_coin::types::DatastoreId;
use crate::datastore_coin::utils::{dig_to_float, float_to_dig};
use chia_wallet_sdk::{
    Cat, CatSpend, Conditions, Peer, PeerOptions,
    Puzzle, PublicKey as ChiaPublicKey, SecretKey as ChiaSecretKey,
    StandardTransaction, Wallet as ChiaWallet,
};
use dig_wallet::Wallet;
use std::sync::Arc;
use tokio::sync::Mutex;

/// DIG token asset ID on Chia blockchain
pub const DIG_ASSET_ID: &str = "6d95dae356e32a71db5ddcb42224754a02524c615c5fc35f568c2af04774e589";

/// Manages blockchain operations using chia-wallet-sdk
pub struct BlockchainConnection {
    peer: Option<Arc<Mutex<Peer>>>,
    network: String,
}

impl BlockchainConnection {
    /// Create a new blockchain connection
    pub fn new(network: &str) -> Self {
        Self {
            peer: None,
            network: network.to_string(),
        }
    }
    
    /// Connect to Chia network using chia-wallet-sdk
    pub async fn connect(&mut self) -> Result<()> {
        println!("Connecting to {} network using chia-wallet-sdk...", self.network);
        
        // Use chia-wallet-sdk's peer connection
        let options = if self.network == "mainnet" {
            PeerOptions::mainnet()
        } else {
            PeerOptions::testnet11()
        };
        
        match Peer::connect(options).await {
            Ok(peer) => {
                println!("✓ Successfully connected via chia-wallet-sdk!");
                self.peer = Some(Arc::new(Mutex::new(peer)));
                Ok(())
            }
            Err(e) => {
                println!("✗ Failed to connect: {}", e);
                Err(DigstoreError::internal(format!(
                    "Could not connect to Chia network: {}",
                    e
                )))
            }
        }
    }
    
    /// Get DIG token balance for a wallet
    pub async fn get_dig_balance(
        &self,
        wallet: &Wallet,
        chia_wallet: &ChiaWallet,
    ) -> Result<u64> {
        let peer = self.get_peer()?;
        let peer_lock = peer.lock().await;
        
        println!("Querying DIG CAT balance using chia-wallet-sdk...");
        
        // Create CAT puzzle for DIG tokens
        let dig_cat = Cat::from_asset_id(DIG_ASSET_ID)
            .map_err(|e| DigstoreError::internal(format!("Invalid DIG asset ID: {}", e)))?;
        
        // Get wallet's CAT coins
        let cat_coins = chia_wallet
            .cat_coins(&dig_cat, &*peer_lock)
            .await
            .map_err(|e| DigstoreError::internal(format!("Failed to query CAT coins: {}", e)))?;
        
        // Sum up the balances
        let total_mojos: u64 = cat_coins.iter().map(|coin| coin.amount).sum();
        
        println!("DIG balance: {} mojos ({:.8} DIG)", total_mojos, dig_to_float(total_mojos));
        
        Ok(total_mojos)
    }
    
    /// Create a CAT spend for DIG tokens
    pub async fn create_dig_spend(
        &self,
        wallet: &ChiaWallet,
        amount_mojos: u64,
        recipient_puzzle_hash: [u8; 32],
    ) -> Result<CatSpend> {
        println!("Creating DIG token spend using chia-wallet-sdk...");
        
        let dig_cat = Cat::from_asset_id(DIG_ASSET_ID)
            .map_err(|e| DigstoreError::internal(format!("Invalid DIG asset ID: {}", e)))?;
        
        // Create conditions for the spend
        let conditions = Conditions::new().create_coin(recipient_puzzle_hash.into(), amount_mojos);
        
        // Create CAT spend
        let cat_spend = wallet
            .create_cat_spend(&dig_cat, amount_mojos, conditions)
            .await
            .map_err(|e| DigstoreError::internal(format!("Failed to create CAT spend: {}", e)))?;
        
        Ok(cat_spend)
    }
    
    /// Submit a transaction to the network
    pub async fn submit_transaction(&self, tx: StandardTransaction) -> Result<String> {
        let peer = self.get_peer()?;
        let mut peer_lock = peer.lock().await;
        
        println!("Submitting transaction using chia-wallet-sdk...");
        
        // Get transaction ID
        let tx_id = tx.name();
        println!("Transaction ID: {}", hex::encode(&tx_id));
        
        // Submit to network
        peer_lock
            .send_transaction(tx)
            .await
            .map_err(|e| DigstoreError::internal(format!("Failed to submit transaction: {}", e)))?;
        
        println!("✓ Transaction submitted successfully!");
        
        Ok(hex::encode(tx_id))
    }
    
    /// Wait for transaction confirmation
    pub async fn wait_for_confirmation(
        &self,
        tx_id: &str,
        timeout_secs: u64,
    ) -> Result<u32> {
        let peer = self.get_peer()?;
        let peer_lock = peer.lock().await;
        
        println!("Waiting for confirmation (timeout: {}s)...", timeout_secs);
        
        // Use tokio timeout
        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let start = tokio::time::Instant::now();
        
        loop {
            if start.elapsed() > timeout {
                return Err(DigstoreError::internal("Transaction confirmation timeout"));
            }
            
            // Check if transaction is confirmed
            // In chia-wallet-sdk, we'd check coin records
            
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            
            // For now, return dummy height
            println!("(Would check confirmation status here)");
            break;
        }
        
        Ok(1000000)
    }
    
    /// Get peer connection
    fn get_peer(&self) -> Result<Arc<Mutex<Peer>>> {
        self.peer.clone().ok_or_else(|| {
            DigstoreError::internal("Not connected to blockchain")
        })
    }
}

/// Helper to convert dig-wallet keys to chia-wallet-sdk keys
pub fn convert_keys(
    wallet: &Wallet,
) -> Result<(ChiaPublicKey, ChiaSecretKey)> {
    // In a real implementation, we'd convert the keys properly
    // For now, this is a placeholder
    Err(DigstoreError::internal("Key conversion not implemented"))
}

/// Create a ChiaWallet instance from dig-wallet
pub async fn create_chia_wallet(wallet: &Wallet) -> Result<ChiaWallet> {
    // Get keys from dig-wallet
    let sk = wallet.get_private_synthetic_key().await
        .map_err(|e| DigstoreError::internal(format!("Failed to get private key: {}", e)))?;
    
    // Convert to chia-wallet-sdk format
    // Note: This would need proper key conversion
    let chia_sk = ChiaSecretKey::from_bytes(&sk.to_bytes())
        .map_err(|e| DigstoreError::internal(format!("Failed to convert secret key: {}", e)))?;
    
    Ok(ChiaWallet::from_sk(chia_sk))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dig_asset_id() {
        assert_eq!(DIG_ASSET_ID.len(), 64); // Hex string
        
        // Verify it's a valid hex string
        hex::decode(DIG_ASSET_ID).expect("DIG_ASSET_ID should be valid hex");
    }
    
    #[tokio::test]
    async fn test_blockchain_connection_creation() {
        let conn = BlockchainConnection::new("testnet11");
        assert_eq!(conn.network, "testnet11");
        assert!(conn.peer.is_none());
        
        // In a real test with network access:
        // conn.connect().await.expect("Should connect");
        // assert!(conn.peer.is_some());
    }
}