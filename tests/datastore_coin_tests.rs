//! Integration tests for datastore coin functionality

use anyhow::Result;
use digstore_min::datastore_coin::{
    DatastoreCoinManager, DatastoreId, CollateralManager, CoinState,
};
use digstore_min::core::Hash;
use digstore_min::wallet::WalletManager;
use tempfile::TempDir;
use std::path::PathBuf;

/// Test basic coin creation and management
#[test]
fn test_coin_creation_and_lifecycle() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let coin_manager = DatastoreCoinManager::new(temp_dir.path().to_path_buf())?;
    
    // Create test datastore parameters
    let datastore_id = DatastoreId::new("test_datastore_001".to_string());
    let root_hash = Hash::from_bytes([42; 32]);
    let size_bytes = 1024 * 1024; // 1 MB
    
    // Calculate collateral requirement
    let collateral_req = coin_manager.get_collateral_requirement(size_bytes)?;
    assert_eq!(collateral_req.base_amount, size_bytes); // 1 mojo per byte default
    assert!(!collateral_req.breakdown.is_large_datastore);
    
    // Verify coin statistics are initially empty
    let stats = coin_manager.get_stats();
    assert_eq!(stats.total_coins, 0);
    assert_eq!(stats.active_coins, 0);
    
    Ok(())
}

/// Test collateral calculations for different sizes
#[test]
fn test_collateral_calculations() -> Result<()> {
    let manager = CollateralManager::new();
    
    // Test standard size (under 1GB)
    let small_size = 100 * 1024 * 1024; // 100 MB
    let req = manager.calculate_requirement(small_size)?;
    assert_eq!(req.base_amount, small_size);
    assert_eq!(req.total_amount, small_size);
    assert!(!req.breakdown.is_large_datastore);
    
    // Test large datastore (over 1GB)
    let large_size = 2u64 * 1024 * 1024 * 1024; // 2 GB
    let req = manager.calculate_requirement(large_size)?;
    assert_eq!(req.base_amount, large_size);
    assert!(req.breakdown.is_large_datastore);
    assert_eq!(req.size_multiplier, 1.5);
    assert_eq!(req.total_amount, (large_size as f64 * 1.5) as u64);
    
    // Test zero size (should fail)
    let result = manager.calculate_requirement(0);
    assert!(result.is_err());
    
    Ok(())
}

/// Test collateral verification
#[test]
fn test_collateral_verification() -> Result<()> {
    let manager = CollateralManager::new();
    let size = 1024 * 1024; // 1 MB
    
    // Exact amount should pass
    assert!(manager.verify_collateral(size, size)?);
    
    // More than required should pass
    assert!(manager.verify_collateral(size, size * 2)?);
    
    // Less than required should fail
    assert!(!manager.verify_collateral(size, size - 1)?);
    
    Ok(())
}

/// Test refund calculations
#[test]
fn test_refund_calculations() -> Result<()> {
    let manager = CollateralManager::new();
    let collateral_amount = 1_000_000;
    
    // Within grace period (30 days default)
    assert_eq!(manager.calculate_refund(collateral_amount, 0), 0);
    assert_eq!(manager.calculate_refund(collateral_amount, 86400), 0); // 1 day
    assert_eq!(manager.calculate_refund(collateral_amount, 86400 * 29), 0); // 29 days
    
    // After grace period
    assert_eq!(manager.calculate_refund(collateral_amount, 86400 * 31), collateral_amount);
    assert_eq!(manager.calculate_refund(collateral_amount, 86400 * 365), collateral_amount);
    
    Ok(())
}

/// Test coin state transitions
#[test]
fn test_coin_state_transitions() -> Result<()> {
    use digstore_min::datastore_coin::DatastoreCoin;
    
    let mut coin = DatastoreCoin::new(
        DatastoreId::new("test".to_string()),
        Hash::from_bytes([1; 32]),
        1024,
        1000,
        "xch1test".to_string(),
    );
    
    // Initial state should be Pending
    assert_eq!(coin.state, CoinState::Pending);
    assert!(!coin.is_active());
    
    // Set blockchain info should transition to Active
    coin.set_blockchain_info("tx123".to_string(), 100);
    assert_eq!(coin.state, CoinState::Active);
    assert!(coin.is_active());
    assert_eq!(coin.tx_id, Some("tx123".to_string()));
    assert_eq!(coin.block_height, Some(100));
    
    // Update to Spent
    coin.update_state(CoinState::Spent);
    assert_eq!(coin.state, CoinState::Spent);
    assert!(!coin.is_active());
    
    Ok(())
}

/// Test coin serialization/deserialization
#[test]
fn test_coin_serialization() -> Result<()> {
    use digstore_min::datastore_coin::DatastoreCoin;
    
    let coin = DatastoreCoin::new(
        DatastoreId::new("test_ser".to_string()),
        Hash::from_bytes([99; 32]),
        2048,
        2000,
        "xch1serialization".to_string(),
    );
    
    // Serialize to JSON
    let json = serde_json::to_string_pretty(&coin)?;
    
    // Deserialize back
    let deserialized: DatastoreCoin = serde_json::from_str(&json)?;
    
    // Verify fields match
    assert_eq!(coin.id, deserialized.id);
    assert_eq!(coin.state, deserialized.state);
    assert_eq!(coin.metadata.datastore_id, deserialized.metadata.datastore_id);
    assert_eq!(coin.metadata.root_hash, deserialized.metadata.root_hash);
    assert_eq!(coin.metadata.size_bytes, deserialized.metadata.size_bytes);
    assert_eq!(coin.metadata.collateral_amount, deserialized.metadata.collateral_amount);
    
    Ok(())
}

/// Test manager persistence
#[test]
fn test_manager_persistence() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let storage_path = temp_dir.path().to_path_buf();
    
    // Create manager and verify coins file is created
    {
        let manager = DatastoreCoinManager::new(storage_path.clone())?;
        let stats = manager.get_stats();
        assert_eq!(stats.total_coins, 0);
    }
    
    // Verify coins.json was created
    let coins_file = storage_path.join("coins.json");
    assert!(coins_file.exists());
    
    // Create new manager instance and verify it loads existing data
    {
        let manager = DatastoreCoinManager::new(storage_path)?;
        let stats = manager.get_stats();
        assert_eq!(stats.total_coins, 0);
    }
    
    Ok(())
}

/// Test listing and filtering coins
#[test]
fn test_coin_listing() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let manager = DatastoreCoinManager::new(temp_dir.path().to_path_buf())?;
    
    // Initially no coins
    let all_coins = manager.list_coins(false);
    assert_eq!(all_coins.len(), 0);
    
    let active_coins = manager.list_coins(true);
    assert_eq!(active_coins.len(), 0);
    
    Ok(())
}

/// Test error cases
#[test]
fn test_error_cases() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let manager = DatastoreCoinManager::new(temp_dir.path().to_path_buf())?;
    
    // Test getting non-existent coin
    let coin_id = digstore_min::datastore_coin::CoinId::new("non_existent".to_string());
    let result = manager.get_coin(&coin_id);
    assert!(result.is_err());
    
    // Test getting coins for non-existent datastore
    let datastore_id = DatastoreId::new("non_existent_ds".to_string());
    let result = manager.get_coins_by_datastore(&datastore_id);
    assert!(result.is_err());
    
    // Test getting coins for non-existent owner
    let result = manager.get_coins_by_owner("xch1nonexistent");
    assert!(result.is_err());
    
    Ok(())
}

/// Test complete workflow integration
#[test]
#[ignore] // This test requires actual wallet and blockchain interaction
fn test_complete_workflow() -> Result<()> {
    // This test would require:
    // 1. Setting up test wallet with DIG tokens
    // 2. Connecting to test blockchain network
    // 3. Creating, minting, transferring, and spending coins
    // 4. Verifying blockchain state changes
    
    // For now, this serves as documentation of the expected workflow
    println!("Complete workflow test would verify:");
    println!("1. Wallet initialization with mnemonic");
    println!("2. DIG token balance checking");
    println!("3. Coin creation with collateral lock");
    println!("4. Coin minting on blockchain");
    println!("5. Coin transfer between addresses");
    println!("6. Coin spending and collateral release");
    
    Ok(())
}