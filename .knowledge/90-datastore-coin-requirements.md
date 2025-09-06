# DataStoreCoin Implementation Requirements and Design

## Overview

This document provides a comprehensive requirements specification and implementation plan for the `DataStoreCoin` class in Rust. The implementation is based on exhaustive analysis of the TypeScript `DataStore.ts` implementation and deep examination of the underlying Chia blockchain infrastructure including `dig-chia-sdk`, `chia-wallet-sdk`, and `DataLayer-Driver`.

### What We Are Building

The `DataStoreCoin` represents a revolutionary integration between digstore's local content-addressable storage system and Chia's blockchain-based DataLayer. This integration transforms digstore from a purely local tool into a blockchain-verified, globally accessible data storage system. Every digstore repository will have a corresponding DataLayer store on the Chia blockchain, creating a permanent, verifiable record of the repository's existence and current state.

### Why This Integration Matters

Currently, digstore repositories exist only locally with randomly generated store IDs. While this provides excellent local version control and content addressing, it lacks global verifiability and blockchain-backed authenticity. By integrating with Chia's DataLayer, we achieve:

1. **Global Verifiability**: Anyone can verify a repository's authenticity using its launcher ID on the Chia blockchain
2. **Decentralized Integrity**: Repository metadata is stored on a decentralized blockchain, preventing tampering
3. **Content Authenticity**: The blockchain maintains a permanent record of content hashes and repository evolution
4. **Distributed Discovery**: Repositories can be discovered and verified through blockchain queries
5. **Economic Incentives**: Repository owners stake value (collateral) in their repositories, creating economic incentives for data integrity

### Critical Synchronization Requirement

The most critical aspect of this implementation is maintaining perfect synchronization between the local digstore state and the on-chain DataStoreCoin state. This means:

- **Every digstore commit MUST update the blockchain metadata**
- **The blockchain root hash MUST always match the current digstore commit hash**
- **The blockchain size metadata MUST always reflect the actual repository size**
- **Any divergence between local and blockchain state MUST be detected and resolved**
- **Failed synchronization attempts MUST be queued for retry with exponential backoff**

This synchronization requirement transforms digstore from a local tool into a blockchain-integrated system where local operations have global, verifiable consequences.

## Feature Parity Analysis

### TypeScript DataStore.ts Implementation Status
- ✅ **mint()** - Via `DataStore.create()` static method (IMPLEMENTED)
- ✅ **updateMetadata()** - Direct method (IMPLEMENTED)
- ❌ **melt()** - NOT IMPLEMENTED in TypeScript (Rust will add this)

### Rust DataStoreCoin Implementation Plan
- ✅ **mint()** - Exact feature parity with TypeScript create() method
- ✅ **update_metadata()** - Exact feature parity with TypeScript updateMetadata()
- 🆕 **melt()** - NEW FEATURE using DataLayer-Driver melt_store functionality

## Critical Architecture Changes

### 1. Store ID Generation Paradigm Shift
- **BEFORE**: `digstore init` generates random 32-byte store ID locally
- **AFTER**: `digstore init` mints DataStoreCoin on Chia blockchain, uses launcher ID as store ID
- **Impact**: 
  - All digstore repositories become verifiable on Chia blockchain
  - Store IDs are globally unique and cryptographically secure
  - Repository authenticity can be verified by anyone with the launcher ID
  - Enables decentralized data verification and integrity checking

### 2. Synchronization Architecture (Critical Requirement)
- **Bidirectional Sync**: Offline digstore ↔ On-chain DataStoreCoin must always match
- **Immediate Consistency**: Any digstore modification triggers immediate DataLayer update
- **Conflict Resolution**: Handle cases where local and blockchain state diverge
- **Recovery Mechanisms**: Robust error handling and retry logic for sync failures
- **State Validation**: Continuous verification that local and blockchain state match

### 3. Command Integration Changes
- **`digstore init`**: 
  - Creates DataStoreCoin first, waits for confirmation, uses launcher ID
  - Adds blockchain verification step before repository creation
  - Implements comprehensive error handling for blockchain failures
- **`digstore commit`**: 
  - Updates DataLayer metadata with new root hash and size immediately
  - Implements atomic commit + blockchain update operations
  - Provides rollback mechanisms if blockchain update fails
- **`digstore add`**: 
  - Tracks size changes for accurate blockchain metadata
  - Prepares metadata for eventual commit synchronization
- **`digstore status`**: 
  - Shows sync status between local and blockchain state
  - Displays any sync conflicts or pending updates
- **No new CLI commands**: Integration happens within existing command flow

### 4. State Management Architecture
- **Store Reference**: Store maintains DataStoreCoin reference for ongoing operations
- **Persistence**: DataStoreCoin state saved/loaded with repository metadata
- **Caching**: Local cache of blockchain state for performance
- **Monitoring**: Continuous monitoring of blockchain state changes
- **Synchronization**: Automatic DataLayer updates on any digstore modification

## Architecture Analysis

### Key Components Analyzed

#### 1. **TypeScript DataStore Implementation** (`dig-chia-sdk/src/blockchain/DataStore.ts`)
- **Purpose**: High-level abstraction for DataLayer store management
- **Key Features**: 
  - Store creation with delegation layers
  - Metadata management and updates
  - Integration with DataIntegrityTree for content management
  - Peer connection and blockchain synchronization
  - File set management and integrity validation

#### 2. **DataLayer-Driver** (`DataLayer-Driver/src/`)
- **Core Functions**: `mint_store`, `update_store_metadata`, `melt_store`, `oracle_spend`
- **Key Structures**: `DataStore`, `DataStoreMetadata`, `DelegatedPuzzle`, `DataStoreInnerSpend`
- **Blockchain Integration**: Direct Chia blockchain interaction layer

#### 3. **Chia-Wallet-SDK** (`chia-wallet-sdk/crates/chia-sdk-driver/`)
- **Layer System**: `StandardLayer`, `SingletonLayer`, `NftStateLayer`, `DelegationLayer`
- **Primitives**: DataStore primitives with comprehensive puzzle handling
- **Spend Context**: Transaction building and signing infrastructure

#### 4. **dig-wallet Integration**
- **Wallet Management**: Mnemonic-based wallet with key derivation
- **Coin Selection**: `select_unspent_coins` for transaction funding
- **Key Operations**: Public/private key management and signing

## DataStoreCoin Requirements

### Core Functionality (Integration with existing commands)

#### 1. **Mint Operation** (Integrated into `digstore init`)
- **Purpose**: Create new DataLayer store on Chia blockchain and use launcher ID as digstore store ID
- **Integration Point**: `digstore init` command
- **Parameters**:
  - `label: Option<String>` - Human-readable store name  
  - `description: Option<String>` - Store description
  - `size_in_bytes: Option<u64>` - Initial store size (0 for new stores)
  - `authorized_writer_public_key: Option<PublicKey>` - Writer delegation
  - `admin_public_key: Option<PublicKey>` - Admin delegation
- **Returns**: `Bytes32` launcher ID (becomes the digstore store ID)
- **Critical Flow** (Matching TypeScript DataStore.create()):
  1. Create DataStoreCoin and mint on blockchain
  2. Wait for transaction confirmation (`waitForConfirmation`)
  3. Use `launcherId` from minted store as digstore `store_id`
  4. Initialize digstore repository with DataLayer-derived store ID
  5. Store DataStoreCoin reference for future operations

#### 2. **Update Metadata Operation** (Integrated into `digstore commit`)
- **Purpose**: Update DataLayer store metadata after digstore commits
- **Integration Point**: `digstore commit` command  
- **Parameters**: 
  - `metadata: DataStoreMetadata` - Complete metadata struct with all fields
    - `root_hash: Bytes32` - New commit hash from digstore
    - `label: Option<String>` - Keep existing label (None to preserve)
    - `description: Option<String>` - Use commit message as description
    - `bytes: Option<u64>` - Current digstore repository size
- **Returns**: Updated `DataStore` instance (new store state)
- **Critical Flow** (Matching TypeScript updateMetadata()):
  1. Complete digstore commit operation first
  2. Calculate total repository size
  3. Connect to Chia peer 
  4. Get wallet synthetic key
  5. Fetch latest DataLayer store state (`fetchCoinInfo` equivalent)
  6. Call `update_store_metadata_rust()` with owner authority
  7. Handle fee calculation and coin selection
  8. Combine store update and fee transactions
  9. Sign and broadcast to blockchain
  10. Update local DataStoreCoin state

#### 3. **Melt Operation** (NEW - Advanced Feature Beyond TypeScript)

The melt operation represents a critical blockchain feature that allows repository owners to permanently destroy their DataLayer store and recover the staked collateral. This operation is irreversible and effectively removes the repository's blockchain presence while preserving the local digstore data.

##### What Melting Achieves

**Economic Recovery**: When a DataLayer store is created, a small amount of XCH (typically 1 mojo) is locked as collateral in the singleton coin. Melting recovers this collateral, returning it to the owner's wallet.

**Permanent Destruction**: The melt operation destroys the singleton coin on the blockchain, making the DataLayer store permanently inaccessible. This is a one-way operation that cannot be undone.

**Clean Shutdown**: For repositories that are no longer needed on the blockchain, melting provides a clean way to remove the blockchain footprint while preserving local data.

**Security Implications**: Only the store owner can initiate a melt operation, as it requires the owner's private key to sign the transaction. This prevents unauthorized destruction of DataLayer stores.

##### Melt Implementation Details

**Status**: **NEW FEATURE** - Not implemented in TypeScript DataStore.ts
**Complexity**: HIGH - Requires deep understanding of Chia singleton mechanics
**Security Level**: CRITICAL - Irreversible operation requiring owner authority

**Parameters**:
- `owner_public_key: PublicKey` - Owner's public key for transaction signing
- `confirmation_required: bool` - Whether to require explicit user confirmation
- `recovery_address: Option<Bytes32>` - Optional address for collateral recovery

**Returns**: 
- `Vec<CoinSpend>` - Unsigned coin spends for melting transaction
- `u64` - Amount of collateral to be recovered

**Blockchain Operations Flow**:
1. **Authority Validation**: Verify caller is the store owner
2. **Melt Condition Creation**: Create `MeltSingleton` condition (opcode 51, magic amount -113)
3. **Fee Reservation**: Reserve 1 mojo fee for the transaction
4. **Owner Spend Generation**: Create StandardLayer spend with melt conditions
5. **Transaction Building**: Build complete coin spend for the DataStore singleton
6. **Return Unsigned**: Return unsigned coin spends for secure external signing

**Critical Implementation Notes**:
- **No Automatic Broadcasting**: For security, melt returns unsigned coin spends
- **External Signing Required**: Caller must sign with owner's private key
- **Irreversible Operation**: Once broadcast, the DataLayer store is permanently destroyed
- **Collateral Recovery**: The singleton coin's value is recovered to the owner
- **Local Data Preserved**: Melting only affects blockchain state, local digstore data remains intact

##### Melt vs. Local Repository Deletion

It's crucial to understand that melting a DataStoreCoin is different from deleting a local digstore repository:

**Melt DataStoreCoin**:
- Destroys blockchain presence
- Recovers economic collateral
- Removes global verifiability
- Local digstore data remains intact
- Irreversible blockchain operation

**Delete Local Repository**:
- Removes local files and data
- No blockchain interaction
- No economic implications
- Can be restored from backups
- Purely local operation

The melt operation is designed for cases where you want to remove the blockchain footprint while preserving local data, such as when transitioning from blockchain-integrated to local-only operation.

### Integration Requirements

#### 1. **Store ID Architecture Revolution** (Critical Paradigm Shift)

This represents a fundamental change in how digstore repositories are identified and managed. The transformation from locally-generated random IDs to blockchain-derived launcher IDs has profound implications for the entire system.

**What Changes**:
- **BEFORE**: `digstore init` generates a cryptographically random 32-byte store ID locally
- **AFTER**: `digstore init` mints a DataStoreCoin on Chia blockchain, uses the resulting launcher ID

**Why This Matters**:
The launcher ID is not just an identifier—it's a cryptographic proof of the repository's existence on the Chia blockchain. This ID is generated through Chia's singleton creation mechanism, which ensures global uniqueness and provides cryptographic guarantees about the repository's authenticity.

**Technical Implementation**:
- **DataLayer Minting**: The `mint_store_rust` function creates a singleton coin on Chia
- **Launcher ID Generation**: Chia's consensus mechanism generates a unique launcher ID
- **Global Uniqueness**: The launcher ID is mathematically guaranteed to be globally unique
- **Verifiability**: Anyone can query the Chia blockchain to verify the store's existence

**Implications for digstore**:
- **Global Namespace**: Repository IDs become part of a global, decentralized namespace
- **Authenticity Proofs**: Repository authenticity can be verified by blockchain queries
- **Distributed Discovery**: Repositories can be discovered through blockchain indexing
- **Economic Backing**: Each repository has economic value staked on the blockchain

#### 2. **Comprehensive Wallet Integration** (Blockchain Operations Foundation)

The wallet integration goes far beyond simple key management—it provides the cryptographic foundation for all blockchain operations and ensures secure, authenticated interactions with the Chia network.

**What the Wallet Provides**:
The dig-wallet integration provides several critical services that enable blockchain operations:

**Cryptographic Services**:
- **Key Derivation**: Generates synthetic keys from the wallet's master mnemonic
- **Transaction Signing**: Signs all blockchain transactions with appropriate private keys
- **Address Generation**: Creates puzzle hashes for receiving funds and identifying ownership
- **Signature Verification**: Validates signatures and transaction authenticity

**Economic Services**:
- **Coin Selection**: Intelligently selects unspent coins for transaction funding
- **Fee Management**: Calculates and manages transaction fees for blockchain operations
- **Collateral Management**: Handles the economic aspects of DataLayer store creation
- **Balance Tracking**: Monitors wallet balance and available funds for operations

**Security Services**:
- **Private Key Protection**: Securely manages private keys without exposing them
- **Authority Validation**: Verifies that operations are authorized by the appropriate keys
- **Permission Checking**: Ensures only authorized operations are performed
- **Secure Communication**: Handles all cryptographic aspects of blockchain communication

**Why This Integration is Critical**:
DataLayer operations involve real economic value and irreversible blockchain transactions. The wallet integration ensures that these operations are performed securely, with proper authorization, and with full protection of the user's cryptographic assets.

#### 3. **Mandatory Commit Integration** (Synchronization Enforcement)

The commit integration represents the most critical aspect of maintaining synchronization between local and blockchain state. This is not optional—every digstore commit that modifies repository state MUST trigger a corresponding blockchain update.

**What Happens During Commit**:

**Local Operations (Existing)**:
1. Process staged files and create chunks
2. Build merkle tree and calculate commit hash
3. Write new layer to local storage
4. Update current root pointer
5. Clear staging area

**NEW: Mandatory Blockchain Operations**:
6. Calculate total repository size across all layers
7. Prepare DataStoreMetadata with new root hash and size
8. Connect to Chia blockchain peer
9. Fetch latest DataStoreCoin state from blockchain
10. Build metadata update transaction
11. Handle fee calculation and coin selection
12. Sign and broadcast update transaction
13. Wait for blockchain confirmation
14. Update local DataStoreCoin state cache
15. Verify synchronization success

**Why Synchronization is Mandatory**:

**Data Integrity**: The blockchain serves as a tamper-evident record of repository state. If local and blockchain state diverge, the integrity guarantees are compromised.

**Global Consistency**: Other users querying the blockchain must see accurate, up-to-date information about the repository's current state.

**Economic Incentives**: The economic model of DataLayer stores depends on accurate metadata. Incorrect size or root hash information undermines the economic incentive structure.

**Verifiability**: Third parties must be able to verify repository state through blockchain queries. Stale blockchain metadata breaks this verifiability.

**What Happens When Synchronization Fails**:

Synchronization failure creates a critical state inconsistency that must be resolved:

1. **Immediate Detection**: The commit operation detects synchronization failure immediately
2. **Local Success**: The local digstore commit succeeds regardless of blockchain sync failure
3. **Warning Display**: Clear warnings inform the user about the synchronization failure
4. **Retry Queuing**: Failed updates are queued for automatic retry with exponential backoff
5. **Status Tracking**: The repository status shows synchronization conflicts
6. **Manual Resolution**: Users can manually trigger synchronization through status commands

**Error Recovery Mechanisms**:
- **Automatic Retry**: Failed synchronization attempts are automatically retried up to 3 times
- **Exponential Backoff**: Retry delays increase exponentially (2s, 4s, 8s) to handle network issues
- **Persistent Queuing**: Failed updates are persisted to disk and retried across CLI sessions
- **Manual Override**: Users can force synchronization or disable blockchain integration
- **Conflict Resolution**: Tools to resolve conflicts between local and blockchain state

## Detailed Implementation Plan

### Phase 1: Core Structure and Dependencies

#### 1.1 DataStoreCoin Struct (Comprehensive Implementation)
```rust
use datalayer_driver::{
    DataStore, DataStoreMetadata, DelegatedPuzzle, DataStoreInnerSpend,
    Peer, PublicKey, SecretKey, Bytes32, Coin, CoinSpend, Signature,
    mint_store_rust, update_store_metadata_rust, melt_store_rust,
    connect_random, NetworkType, SuccessResponse, async_api,
    sync_store_from_launcher_id_rust, get_all_unspent_coins_rust
};
use crate::wallet::WalletManager;
use crate::core::error::{DigstoreError, Result};
use crate::core::types::Hash;
use crate::storage::Store;
use serde::{Serialize, Deserialize};
use tokio::time::{timeout, Duration, Instant};
use std::collections::HashMap;
use indicatif::{ProgressBar, ProgressStyle};
use colored::Colorize;

/// Comprehensive DataStoreCoin implementation with full state management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataStoreCoin {
    // Core identification
    launcher_id: Bytes32,                    // DataLayer launcher ID (also digstore store_id)
    
    // Blockchain state
    datastore: Option<DataStore>,            // Current DataLayer store state from blockchain
    last_known_coin: Option<Coin>,           // Last known coin state for tracking
    last_sync_height: Option<u32>,           // Last blockchain height we synced at
    last_sync_timestamp: Option<i64>,        // Last successful sync timestamp
    
    // Network configuration
    network: NetworkType,                    // Mainnet or Testnet11
    
    // Creation tracking (matching TypeScript patterns)
    creation_height: Option<u32>,            // Blockchain height when created
    creation_hash: Option<Bytes32>,          // Header hash when created
    
    // Delegation and permissions
    delegated_puzzles: Vec<DelegatedPuzzle>, // Admin, Writer, Oracle delegation layers
    owner_puzzle_hash: Bytes32,              // Owner puzzle hash for permission validation
    
    // Local state tracking
    pending_updates: Vec<PendingUpdate>,     // Queue of updates waiting for blockchain sync
    sync_status: SyncStatus,                 // Current synchronization status
    last_error: Option<String>,              // Last error for debugging
    
    // Performance optimization
    cached_metadata: Option<DataStoreMetadata>, // Cached metadata to avoid redundant fetches
    cache_expiry: Option<Instant>,           // Cache expiration time
}

/// Pending update tracking for robust synchronization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdate {
    pub update_type: UpdateType,
    pub metadata: DataStoreMetadata,
    pub timestamp: i64,
    pub retry_count: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateType {
    MetadataUpdate,
    RootHashUpdate,
    SizeUpdate,
    FullSync,
}

/// Synchronization status tracking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SyncStatus {
    InSync,                    // Local and blockchain state match
    PendingUpdate,             // Updates waiting to be sent to blockchain
    SyncInProgress,            // Currently syncing with blockchain
    SyncFailed(String),        // Sync failed with error message
    Conflicted,                // Local and blockchain state conflict
    Unknown,                   // Status unknown, needs verification
}
```

#### 1.2 Comprehensive Constructor and Factory Methods
```rust
impl DataStoreCoin {
    /// Create new DataStoreCoin for minting (used in digstore init)
    pub fn new(network: NetworkType) -> Result<Self> {
        Ok(Self {
            launcher_id: Bytes32::default(), // Will be set during mint
            datastore: None,
            last_known_coin: None,
            last_sync_height: None,
            last_sync_timestamp: None,
            network,
            creation_height: None,
            creation_hash: None,
            delegated_puzzles: Vec::new(),
            owner_puzzle_hash: Bytes32::default(),
            pending_updates: Vec::new(),
            sync_status: SyncStatus::Unknown,
            last_error: None,
            cached_metadata: None,
            cache_expiry: None,
        })
    }
    
    /// Load existing DataStoreCoin from launcher ID (used when opening existing stores)
    /// Implements comprehensive blockchain state fetching matching TypeScript patterns
    pub async fn from_launcher_id(launcher_id: Bytes32, network: NetworkType) -> Result<Self> {
        let mut datastore_coin = Self {
            launcher_id,
            network,
            sync_status: SyncStatus::Unknown,
            ..Self::new(network)?
        };
        
        // Fetch current state from blockchain (matching TypeScript fetchCoinInfo pattern)
        datastore_coin.sync_from_blockchain_internal().await?;
        
        Ok(datastore_coin)
    }
    
    /// Create from existing DataStore state (for advanced use cases)
    pub fn from_datastore(datastore: DataStore, network: NetworkType) -> Result<Self> {
        Ok(Self {
            launcher_id: datastore.info.launcher_id,
            datastore: Some(datastore.clone()),
            last_known_coin: Some(datastore.coin),
            network,
            creation_height: None,
            creation_hash: None,
            delegated_puzzles: datastore.info.delegated_puzzles,
            owner_puzzle_hash: datastore.info.owner_puzzle_hash,
            pending_updates: Vec::new(),
            sync_status: SyncStatus::InSync,
            last_error: None,
            cached_metadata: Some(datastore.info.metadata),
            cache_expiry: Some(Instant::now() + Duration::from_secs(300)), // 5 minute cache
            last_sync_height: None,
            last_sync_timestamp: Some(chrono::Utc::now().timestamp()),
        })
    }
    
    /// Get launcher ID (used as digstore store_id)
    pub fn launcher_id(&self) -> Bytes32 {
        self.launcher_id
    }
    
    /// Get current sync status
    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync_status
    }
    
    /// Check if local and blockchain state are in sync
    pub fn is_in_sync(&self) -> bool {
        matches!(self.sync_status, SyncStatus::InSync)
    }
    
    /// Get current coin ID for tracking (matching TypeScript getCoinId pattern)
    pub fn get_coin_id(&self) -> Result<Bytes32> {
        match &self.datastore {
            Some(ds) => Ok(ds.coin.coin_id()),
            None => Err(DigstoreError::DataStoreCoinError("No datastore state available".into())),
        }
    }
    
    /// Wait for transaction confirmation with progress feedback and timeout
    /// Matches TypeScript FullNodePeer.waitForConfirmation pattern
    pub async fn wait_for_confirmation(&self) -> Result<()> {
        let coin_id = self.get_coin_id()?;
        let peer = self.connect_peer().await?;
        
        // Use DataLayer-Driver's async API for confirmation waiting
        // This matches the TypeScript FullNodePeer.waitForConfirmation pattern
        match timeout(
            Duration::from_secs(120), // 2 minute timeout
            async_api::wait_for_coin_confirmation(&peer, coin_id)
        ).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(DigstoreError::DataStoreCoinError(format!("Confirmation failed: {}", e))),
            Err(_) => Err(DigstoreError::DataStoreCoinError("Confirmation timeout after 2 minutes".into())),
        }
    }
    
    /// Connect to Chia peer (centralized connection logic)
    async fn connect_peer(&self) -> Result<Peer> {
        // Use appropriate SSL paths for network
        let (cert_path, key_path) = match self.network {
            NetworkType::Mainnet => (
                "~/.chia/mainnet/config/ssl/wallet/wallet_node.crt",
                "~/.chia/mainnet/config/ssl/wallet/wallet_node.key"
            ),
            NetworkType::Testnet11 => (
                "~/.chia/testnet11/config/ssl/wallet/wallet_node.crt", 
                "~/.chia/testnet11/config/ssl/wallet/wallet_node.key"
            ),
        };
        
        connect_random(self.network, cert_path, key_path)
            .await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to connect to peer: {}", e)))
    }
    
    /// Comprehensive synchronization methods for state consistency
    
    /// Sync local state from blockchain (matching TypeScript fetchCoinInfo pattern)
    pub async fn sync_from_blockchain_internal(&mut self) -> Result<()> {
        self.sync_status = SyncStatus::SyncInProgress;
        
        let peer = self.connect_peer().await?;
        
        // Get creation info if not cached (matching TypeScript getCreationHeight pattern)
        if self.creation_height.is_none() || self.creation_hash.is_none() {
            let creation_info = self.cache_store_creation_height(&peer).await?;
            self.creation_height = Some(creation_info.0);
            self.creation_hash = Some(creation_info.1);
        }
        
        // Sync store state from blockchain (matching TypeScript sync pattern)
        let sync_response = sync_store_from_launcher_id_rust(
            &peer,
            self.launcher_id,
            self.last_sync_height,
            self.creation_hash.unwrap(),
            true, // with history
        ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Sync failed: {}", e)))?;
        
        // Update internal state
        self.datastore = Some(sync_response.latest_store);
        self.last_known_coin = self.datastore.as_ref().map(|ds| ds.coin);
        self.last_sync_height = Some(sync_response.latest_height);
        self.last_sync_timestamp = Some(chrono::Utc::now().timestamp());
        self.sync_status = SyncStatus::InSync;
        self.cached_metadata = self.datastore.as_ref().map(|ds| ds.info.metadata.clone());
        self.cache_expiry = Some(Instant::now() + Duration::from_secs(300));
        
        Ok(())
    }
    
    /// Cache store creation height (matching TypeScript cacheStoreCreationHeight)
    async fn cache_store_creation_height(&self, peer: &Peer) -> Result<(u32, Bytes32)> {
        use datalayer_driver::async_api::get_store_creation_height_rust;
        
        let creation_height = get_store_creation_height_rust(
            peer,
            self.launcher_id,
            None,
            match self.network {
                NetworkType::Mainnet => datalayer_driver::constants::get_mainnet_genesis_challenge(),
                NetworkType::Testnet11 => datalayer_driver::constants::get_testnet11_genesis_challenge(),
            }
        ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get creation height: {}", e)))?;
        
        // Get header hash at creation height - 1 (matching TypeScript pattern)
        let creation_hash = datalayer_driver::async_api::get_header_hash_rust(
            peer, 
            creation_height.saturating_sub(1)
        ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get creation hash: {}", e)))?;
        
        Ok((creation_height.saturating_sub(1), creation_hash))
    }
    
    /// Validate that local digstore state matches blockchain state
    pub async fn validate_state_consistency(&mut self, local_root_hash: Bytes32, local_size: u64) -> Result<bool> {
        // Fetch latest blockchain state
        if self.is_cache_expired() {
            self.sync_from_blockchain_internal().await?;
        }
        
        let blockchain_metadata = self.get_current_metadata().await?;
        
        // Check for consistency
        let root_hash_matches = blockchain_metadata.root_hash == local_root_hash;
        let size_matches = blockchain_metadata.bytes.unwrap_or(0) == local_size;
        
        if !root_hash_matches || !size_matches {
            self.sync_status = SyncStatus::Conflicted;
            return Ok(false);
        }
        
        self.sync_status = SyncStatus::InSync;
        Ok(true)
    }
    
    /// Force synchronization of local state to blockchain
    pub async fn force_sync_to_blockchain(&mut self, digstore: &Store) -> Result<()> {
        let current_root = digstore.current_root()
            .ok_or_else(|| DigstoreError::DataStoreCoinError("No current root in digstore".into()))?;
        let total_size = digstore.calculate_total_size()?;
        
        let metadata = DataStoreMetadata {
            root_hash: current_root,
            label: self.cached_metadata.as_ref().and_then(|m| m.label.clone()),
            description: Some("Synchronized from digstore".to_string()),
            bytes: Some(total_size),
        };
        
        self.update_metadata(metadata).await?;
        Ok(())
    }
    
    /// Check if metadata cache has expired
    fn is_cache_expired(&self) -> bool {
        match self.cache_expiry {
            Some(expiry) => Instant::now() > expiry,
            None => true,
        }
    }
    
    /// Add pending update to queue for retry logic
    fn queue_pending_update(&mut self, update_type: UpdateType, metadata: DataStoreMetadata) {
        self.pending_updates.push(PendingUpdate {
            update_type,
            metadata,
            timestamp: chrono::Utc::now().timestamp(),
            retry_count: 0,
            last_error: None,
        });
        self.sync_status = SyncStatus::PendingUpdate;
    }
    
    /// Process pending updates with retry logic
    pub async fn process_pending_updates(&mut self) -> Result<usize> {
        let mut processed = 0;
        let mut failed_updates = Vec::new();
        
        for mut update in self.pending_updates.drain(..) {
            match self.update_metadata_internal(update.metadata.clone()).await {
                Ok(_) => {
                    processed += 1;
                }
                Err(e) => {
                    update.retry_count += 1;
                    update.last_error = Some(e.to_string());
                    
                    // Retry up to 3 times
                    if update.retry_count < 3 {
                        failed_updates.push(update);
                    }
                }
            }
        }
        
        self.pending_updates = failed_updates;
        if self.pending_updates.is_empty() {
            self.sync_status = SyncStatus::InSync;
        }
        
        Ok(processed)
    }
}
```

### Phase 2: Comprehensive Mint Implementation

#### 2.1 Mint Method Signatures (Multiple Variants for Different Use Cases)
```rust
impl DataStoreCoin {
    /// Primary mint method - exact TypeScript DataStore.create() parity
    pub async fn mint(
        &mut self,
        label: Option<String>,
        description: Option<String>,
        size_in_bytes: Option<u64>,
        authorized_writer_public_key: Option<PublicKey>,
        admin_public_key: Option<PublicKey>,
    ) -> Result<Bytes32> // Returns launcher ID
    
    /// Mint with comprehensive progress feedback and error handling
    pub async fn mint_with_progress(
        &mut self,
        label: Option<String>,
        description: Option<String>, 
        size_in_bytes: Option<u64>,
        authorized_writer: Option<PublicKey>,
        admin: Option<PublicKey>,
        progress_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
    ) -> Result<Bytes32>
    
    /// Mint with custom delegation layers (advanced)
    pub async fn mint_with_custom_delegation(
        &mut self,
        label: Option<String>,
        description: Option<String>,
        size_in_bytes: Option<u64>,
        custom_delegated_puzzles: Vec<DelegatedPuzzle>,
        oracle_fee: Option<u64>,
    ) -> Result<Bytes32>
}
```

#### 2.2 Comprehensive Mint Implementation Steps (Exact TypeScript Pattern)

##### Step 1: Wallet and Network Setup (Matching TypeScript mint() method)
```rust
async fn mint_internal(&mut self, params: MintParams) -> Result<Bytes32> {
    // 1.1 Connect to peer (matching FullNodePeer.connect())
    let peer = self.connect_peer().await?;
    
    // 1.2 Get blockchain height and hash (matching TypeScript pattern)
    let peak_height = peer.get_peak_height().await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get peak height: {}", e)))?;
    let header_hash = datalayer_driver::async_api::get_header_hash_rust(&peer, peak_height).await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get header hash: {}", e)))?;
    
    // 1.3 Get wallet keys (matching Wallet.load("default") pattern)
    let wallet = WalletManager::get_active_wallet()?;
    let public_synthetic_key = wallet.get_public_synthetic_key().await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get synthetic key: {}", e)))?;
    let owner_puzzle_hash = wallet.get_owner_puzzle_hash().await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get puzzle hash: {}", e)))?;
    
    self.owner_puzzle_hash = owner_puzzle_hash;
```

##### Step 2: Delegation Layer Setup (Exact TypeScript Pattern)
```rust
    // 2.1 Build delegation layers (matching TypeScript delegationLayers pattern)
    let mut delegated_puzzles = Vec::new();
    
    // Add admin delegation if provided
    if let Some(admin_key) = params.admin_public_key {
        delegated_puzzles.push(datalayer_driver::admin_delegated_puzzle_from_key(&admin_key));
    }
    
    // Add writer delegation if provided  
    if let Some(writer_key) = params.authorized_writer_public_key {
        delegated_puzzles.push(datalayer_driver::writer_delegated_puzzle_from_key(&writer_key));
    }
    
    // Add oracle delegation (matching TypeScript oracleDelegatedPuzzle pattern)
    let oracle_fee = params.oracle_fee.unwrap_or(100_000); // Default 100k mojos
    delegated_puzzles.push(datalayer_driver::oracle_delegated_puzzle(
        owner_puzzle_hash, 
        oracle_fee
    ));
    
    self.delegated_puzzles = delegated_puzzles.clone();
```

##### Step 3: Coin Selection and Fee Management (Exact TypeScript Pattern)
```rust
    // 3.1 Select coins for transaction (matching TypeScript selectUnspentCoins pattern)
    let store_creation_coins = wallet.select_unspent_coins(
        &peer,
        1, // 1 mojo for store creation
        0  // Fee calculated separately
    ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Coin selection failed: {}", e)))?;
    
    // 3.2 Prepare initial root hash (matching TypeScript pattern)
    let root_hash = Bytes32::new([0u8; 32]); // All zeros for initial state
```

##### Step 4: Store Minting (Exact TypeScript Pattern)
```rust
    // 4.1 Prepare mint parameters (matching TypeScript mintStoreParams pattern)
    let mint_params = (
        public_synthetic_key,
        store_creation_coins.clone(),
        root_hash,
        params.label.clone(),
        params.description.clone(),
        params.size_in_bytes.unwrap_or(0),
        owner_puzzle_hash,
        delegated_puzzles.clone(),
    );
    
    // 4.2 Preflight transaction for fee calculation (matching TypeScript pattern)
    let preflight_response = mint_store_rust(
        mint_params.0, mint_params.1.clone(), mint_params.2, mint_params.3.clone(),
        mint_params.4.clone(), Some(mint_params.5), mint_params.6, mint_params.7.clone(),
        0 // Zero fee for preflight
    )?;
    
    // 4.3 Calculate actual fee (matching TypeScript calculateFeeForCoinSpends pattern)
    let fee = datalayer_driver::async_api::calculate_fee_for_coin_spends_rust(
        &peer, 
        Some(&preflight_response.coin_spends)
    ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Fee calculation failed: {}", e)))?;
    
    // 4.4 Execute actual mint with calculated fee
    let store_creation_response = mint_store_rust(
        mint_params.0, mint_params.1, mint_params.2, mint_params.3,
        mint_params.4, Some(mint_params.5), mint_params.6, mint_params.7,
        fee
    )?;
```

##### Step 5: Transaction Signing and Broadcasting (Exact TypeScript Pattern)
```rust
    // 5.1 Sign coin spends (matching TypeScript signCoinSpends pattern)
    let private_synthetic_key = wallet.get_private_synthetic_key().await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get private key: {}", e)))?;
    
    let signature = datalayer_driver::sign_coin_spends_rust(
        &store_creation_response.coin_spends,
        &[private_synthetic_key],
        matches!(self.network, NetworkType::Testnet11)
    )?;
    
    // 5.2 Broadcast transaction (matching TypeScript broadcastSpend pattern)
    let spend_bundle = datalayer_driver::SpendBundle::new(
        store_creation_response.coin_spends.clone(),
        signature
    );
    
    let broadcast_result = datalayer_driver::async_api::broadcast_spend_bundle_rust(
        &peer, 
        spend_bundle
    ).await;
    
    if let Err(e) = broadcast_result {
        return Err(DigstoreError::DataStoreCoinError(format!("Broadcast failed: {}", e)));
    }
    
    // 5.3 Update internal state
    self.launcher_id = store_creation_response.new_datastore.info.launcher_id;
    self.datastore = Some(store_creation_response.new_datastore);
    self.creation_height = Some(peak_height);
    self.creation_hash = Some(header_hash);
    self.sync_status = SyncStatus::InSync;
    self.last_sync_timestamp = Some(chrono::Utc::now().timestamp());
    
    // 5.4 Cache creation info for future operations
    self.cache_creation_info(peak_height, header_hash).await?;
    
    Ok(self.launcher_id)
}

/// Cache creation height and hash (matching TypeScript setCreationHeight pattern)
async fn cache_creation_info(&self, height: u32, hash: Bytes32) -> Result<()> {
    use crate::storage::FileCache;
    
    let file_cache = FileCache::new(&format!("stores/{}", self.launcher_id.to_hex()))?;
    file_cache.set("height", serde_json::json!({
        "height": height,
        "hash": hash.to_hex()
    }))?;
    
    Ok(())
}
```

### Phase 3: Comprehensive Update Metadata Implementation

#### 3.1 Update Method Signatures (Multiple Variants for Robust Integration)
```rust
impl DataStoreCoin {
    /// Primary update method - exact match to TypeScript DataStore.updateMetadata()
    pub async fn update_metadata(
        &mut self,
        metadata: DataStoreMetadata,
    ) -> Result<DataStore>
    
    /// Update with progress feedback and error recovery
    pub async fn update_metadata_with_progress(
        &mut self,
        metadata: DataStoreMetadata,
        progress_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
    ) -> Result<DataStore>
    
    /// Update with retry logic for network failures
    pub async fn update_metadata_with_retry(
        &mut self,
        metadata: DataStoreMetadata,
        max_retries: u32,
        retry_delay: Duration,
    ) -> Result<DataStore>
    
    /// Internal update method with comprehensive error handling
    async fn update_metadata_internal(&mut self, metadata: DataStoreMetadata) -> Result<DataStore>
    
    /// Atomic update ensuring local/blockchain consistency
    pub async fn atomic_update_from_digstore_commit(
        &mut self,
        commit_hash: Bytes32,
        commit_message: String,
        repository_size: u64,
    ) -> Result<DataStore>
}

/// DataStoreMetadata struct matching the TypeScript interface exactly
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataStoreMetadata {
    pub root_hash: Bytes32,
    pub label: Option<String>,
    pub description: Option<String>,
    pub bytes: Option<u64>,
}

/// Parameters for mint operations
#[derive(Debug, Clone)]
pub struct MintParams {
    pub label: Option<String>,
    pub description: Option<String>,
    pub size_in_bytes: Option<u64>,
    pub authorized_writer_public_key: Option<PublicKey>,
    pub admin_public_key: Option<PublicKey>,
    pub oracle_fee: Option<u64>,
}
```

#### 3.2 Comprehensive Update Implementation (Exact TypeScript Pattern)

##### Step 1: State Validation and Preparation
```rust
async fn update_metadata_internal(&mut self, metadata: DataStoreMetadata) -> Result<DataStore> {
    // 1.1 Validate current state
    if self.sync_status == SyncStatus::SyncInProgress {
        return Err(DigstoreError::DataStoreCoinError("Sync already in progress".into()));
    }
    
    self.sync_status = SyncStatus::SyncInProgress;
    
    // 1.2 Connect to peer (matching TypeScript FullNodePeer.connect())
    let peer = self.connect_peer().await?;
    
    // 1.3 Get wallet keys (matching TypeScript Wallet.load("default") pattern)
    let wallet = WalletManager::get_active_wallet()?;
    let public_synthetic_key = wallet.get_public_synthetic_key().await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get synthetic key: {}", e)))?;
```

##### Step 2: Fetch Latest Store State (Exact TypeScript fetchCoinInfo Pattern)
```rust
    // 2.1 Fetch latest store state from blockchain (matching TypeScript fetchCoinInfo)
    let latest_store = if self.is_cache_expired() {
        // Sync from blockchain if cache expired
        self.sync_from_blockchain_internal().await?;
        self.datastore.as_ref().unwrap().clone()
    } else {
        // Use cached state if still valid
        match &self.datastore {
            Some(ds) => ds.clone(),
            None => {
                // No cached state, fetch from blockchain
                self.sync_from_blockchain_internal().await?;
                self.datastore.as_ref().unwrap().clone()
            }
        }
    };
```

##### Step 3: Transaction Building (Exact TypeScript Pattern)
```rust
    // 3.1 Build update transaction (matching TypeScript updateStoreMetadata call)
    let update_store_response = update_store_metadata_rust(
        latest_store,
        metadata.root_hash,
        metadata.label.clone(),
        metadata.description.clone(),
        metadata.bytes,
        DataStoreInnerSpend::Owner(public_synthetic_key), // Owner authority matching TS
    )?;
    
    // 3.2 Calculate fee (matching TypeScript calculateFeeForCoinSpends pattern)
    let fee = datalayer_driver::async_api::calculate_fee_for_coin_spends_rust(
        &peer,
        None // Pass None like TypeScript does
    ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Fee calculation failed: {}", e)))?;
    
    // 3.3 Select unspent coins for fee payment (matching TypeScript pattern)
    let unspent_coins = wallet.select_unspent_coins(
        &peer,
        0,   // No amount needed, just fee
        fee
    ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Fee coin selection failed: {}", e)))?;
    
    // 3.4 Add fee coins (matching TypeScript addFee pattern)
    let fee_coin_spends = datalayer_driver::add_fee_rust(
        &public_synthetic_key,
        &unspent_coins,
        &update_store_response.coin_spends.iter().map(|cs| cs.coin.coin_id()).collect::<Vec<_>>(),
        fee
    )?;
```

##### Step 4: Transaction Combination and Broadcasting (Exact TypeScript Pattern)
```rust
    // 4.1 Combine coin spends (matching TypeScript combinedCoinSpends pattern)
    let mut combined_coin_spends = update_store_response.coin_spends;
    combined_coin_spends.extend(fee_coin_spends);
    
    // 4.2 Sign combined transaction (matching TypeScript signCoinSpends pattern)
    let private_synthetic_key = wallet.get_private_synthetic_key().await
        .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get private key: {}", e)))?;
    
    let signature = datalayer_driver::sign_coin_spends_rust(
        &combined_coin_spends,
        &[private_synthetic_key],
        matches!(self.network, NetworkType::Testnet11)
    )?;
    
    // 4.3 Broadcast transaction (matching TypeScript broadcastSpend pattern)
    let spend_bundle = datalayer_driver::SpendBundle::new(combined_coin_spends, signature);
    let broadcast_result = datalayer_driver::async_api::broadcast_spend_bundle_rust(
        &peer,
        spend_bundle
    ).await;
    
    if let Err(e) = broadcast_result {
        self.sync_status = SyncStatus::SyncFailed(e.to_string());
        self.last_error = Some(e.to_string());
        return Err(DigstoreError::DataStoreCoinError(format!("Broadcast failed: {}", e)));
    }
```

##### Step 5: State Update and Consistency (Critical for Sync)
```rust
    // 5.1 Update internal state (matching TypeScript return pattern)
    let new_datastore = update_store_response.new_datastore;
    self.datastore = Some(new_datastore.clone());
    self.last_known_coin = Some(new_datastore.coin);
    self.sync_status = SyncStatus::InSync;
    self.last_sync_timestamp = Some(chrono::Utc::now().timestamp());
    self.cached_metadata = Some(metadata.clone());
    self.cache_expiry = Some(Instant::now() + Duration::from_secs(300));
    self.last_error = None;
    
    // 5.2 Clear any pending updates for this metadata
    self.pending_updates.retain(|update| update.metadata != metadata);
    
    // 5.3 Return new store state (matching TypeScript return updateStoreResponse.newStore)
    Ok(new_datastore)
}
```

#### 3.2 Update Implementation Steps (Exact TypeScript pattern)
1. **Peer and Wallet Setup**:
   - Connect to peer via `connect_random()` (equivalent to FullNodePeer.connect())
   - Get active wallet via WalletManager
   - Get public synthetic key from wallet

2. **Store State Fetching**:
   - Fetch latest store state from blockchain (equivalent to fetchCoinInfo())
   - This gets the current DataStore coin state

3. **Transaction Building**:
   - Call `update_store_metadata_rust()` with:
     - Latest store state
     - New metadata (root_hash, label, description, bytes)
     - Owner authority (publicSyntheticKey, null, null - matching TS pattern)
   - Calculate fee via DataLayer-Driver fee calculation
   - Select unspent coins for fee payment via wallet

4. **Fee Management** (Exact TypeScript pattern):
   - Call `add_fee_rust()` to create fee coin spends
   - Combine store update coin spends with fee coin spends
   - This matches the TypeScript pattern exactly

5. **Transaction Execution**:
   - Sign combined coin spends with wallet's private synthetic key
   - Broadcast via peer.broadcast_spend()
   - Return new store state from update response

### Phase 4: Comprehensive Melt Implementation (Advanced Blockchain Feature)

The melt implementation represents one of the most complex and security-critical features of the DataStoreCoin system. This operation permanently destroys the blockchain presence of a repository while recovering economic value, requiring careful implementation with multiple safety mechanisms.

#### 4.1 Understanding the Melt Mechanism

**What Melt Does at the Blockchain Level**:

The melt operation is fundamentally a singleton destruction mechanism in Chia's UTXO model. When a DataLayer store is created, it exists as a singleton coin—a special type of coin that maintains uniqueness and lineage. Melting destroys this singleton, recovering its value.

**Chia Singleton Mechanics**:
1. **Singleton Coins**: DataLayer stores exist as singleton coins with unique launcher IDs
2. **Lineage Preservation**: Singletons maintain lineage through parent-child relationships  
3. **Melt Condition**: The `MeltSingleton` condition (opcode 51, magic amount -113) destroys the singleton
4. **Value Recovery**: The singleton's value (typically 1 mojo) is recovered to the owner
5. **Irreversible Destruction**: Once melted, the singleton cannot be recreated with the same launcher ID

**Economic Implications**:
- **Collateral Recovery**: The owner recovers the value locked in the singleton (usually 1 mojo)
- **Fee Cost**: Melting requires a transaction fee (typically 1 million mojos)
- **Net Economic Impact**: The fee cost usually exceeds the collateral recovery
- **Economic Motivation**: Melting is primarily for cleanup, not profit

### Critical DIG Token Collateral Requirements

#### DIG Token Collateral System
The DataLayer integration introduces a critical economic requirement: repositories must maintain sufficient DIG token collateral based on their size. This collateral system ensures economic incentives for data integrity and prevents spam repositories.

**Collateral Requirements**:
- **Rate**: 0.1 DIG tokens per MB of repository data
- **Token**: DIG CAT token (Asset ID: `a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81`)
- **Calculation**: `required_dig = ceil(repository_size_mb * 0.1)`
- **Check Timing**: Before every commit operation
- **Enforcement**: Commits blocked if insufficient DIG collateral

**Why Collateral is Required**:
1. **Economic Incentives**: Repository owners have skin in the game
2. **Spam Prevention**: Prevents creation of numerous low-value repositories
3. **Data Integrity**: Economic incentive to maintain accurate metadata
4. **Network Sustainability**: Funds network operations and storage incentives
5. **Quality Assurance**: Encourages meaningful, valuable repositories

**Implementation Impact**:
- **Commit Validation**: Every commit checks DIG balance before proceeding
- **User Feedback**: Clear messaging when collateral requirements aren't met
- **Graceful Handling**: Commits fail gracefully with helpful error messages
- **Balance Monitoring**: Continuous monitoring of DIG token balance

#### How DIG Collateral Checking Works

The DIG collateral system represents a fundamental shift in how repository commits are validated. Instead of purely local validation, commits now require economic validation through DIG token ownership.

**The Collateral Calculation Process**:

1. **Repository Size Calculation**: After creating the local commit, calculate the total repository size across all layers
2. **Megabyte Conversion**: Convert byte size to megabytes, rounding up to ensure sufficient collateral
3. **DIG Requirement Calculation**: Multiply MB by 0.1 to get required DIG tokens
4. **Token Balance Query**: Query the Chia blockchain for DIG CAT tokens in the user's wallet
5. **Balance Validation**: Compare required DIG against available DIG balance
6. **Decision Making**: Allow or block commit based on collateral availability

**Integration with Wallet Manager**:

The DIG collateral checking deeply integrates with the existing wallet management system to provide seamless user experience:

**Wallet Address Retrieval**: The system uses `WalletManager::get_active_wallet()` to get the current wallet instance, then calls `wallet.get_owner_public_key().await` to get the XCH address where DIG tokens should be held.

**Blockchain Connectivity**: The system leverages the same peer connection infrastructure used for DataStoreCoin operations to query DIG token balances.

**CAT Token Integration**: DIG tokens are CAT (Chia Asset Tokens) with a specific asset ID. The system queries for CAT coins with the DIG asset ID to determine the user's DIG balance.

**User Experience**: When collateral is insufficient, the system provides the user's XCH address and clear instructions on how to acquire and send DIG tokens to meet the requirements.

**Why This Approach**:

**Economic Sustainability**: The collateral system creates economic incentives for repository quality and prevents spam repositories from flooding the DataLayer network.

**Scalable Economics**: The per-MB cost scales with repository size, ensuring that larger repositories contribute proportionally to network sustainability.

**User Control**: Users maintain full control over their DIG tokens and can choose when to meet collateral requirements.

**Graceful Degradation**: If DIG requirements aren't met, the local digstore commit still succeeds, but blockchain synchronization is blocked until collateral is available.

#### Detailed Collateral Validation Flow

**Step 1: Post-Commit Size Calculation**
```rust
// After successful digstore commit
let total_size = self.calculate_total_size()?; // Calculate across all layers and files
let size_mb = (total_size as f64 / (1024.0 * 1024.0)).ceil() as u64; // Round up to next MB
let required_dig_mojos = (size_mb as f64 * 0.1 * 1000.0).ceil() as u64; // 0.1 DIG per MB in mojos
```

**Step 2: Wallet Address and Balance Retrieval**
```rust
// Get wallet address from active wallet (using existing wallet manager)
let wallet = WalletManager::get_active_wallet()?;
let wallet_address = wallet.get_owner_public_key().await?; // XCH address for DIG tokens
let owner_puzzle_hash = wallet.get_owner_puzzle_hash().await?; // For CAT coin queries
```

**Step 3: DIG Token Balance Query (Comprehensive CAT Implementation)**
```rust
// Query blockchain for DIG CAT tokens using chia-wallet-sdk patterns
const DIG_ASSET_ID: &str = "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81";
let dig_asset_id = Bytes32::from_hex(DIG_ASSET_ID)?;

// Calculate CAT puzzle hash for DIG tokens (based on chia-wallet-sdk CatInfo implementation)
// CAT coins have specific puzzle hashes that combine the asset ID with the owner's puzzle hash
let cat_puzzle_hash = calculate_cat_puzzle_hash(dig_asset_id, owner_puzzle_hash)?;

// Query for unspent CAT coins using DataLayer-Driver's get_all_unspent_coins_rust
let unspent_response = datalayer_driver::async_api::get_all_unspent_coins_rust(
    &peer,
    cat_puzzle_hash,  // Query specifically for CAT coins with DIG asset ID
    None,             // Start from genesis
    genesis_challenge // Network-appropriate genesis challenge
).await?;

// Extract DIG CAT coins and calculate total balance
let dig_cat_coins: Vec<Coin> = unspent_response.coin_states
    .into_iter()
    .filter(|cs| cs.spent_height.is_none()) // Only unspent coins
    .map(|cs| cs.coin)
    .collect();

let total_dig_balance = dig_cat_coins.iter().map(|coin| coin.amount).sum::<u64>();
```

**Understanding CAT Coin Querying**:

CAT (Chia Asset Token) coins have a unique structure that requires specific querying methods. Unlike standard XCH coins, CAT coins embed the asset ID in their puzzle hash, making them queryable by asset type.

**CAT Puzzle Hash Calculation**:
The puzzle hash for a CAT coin is calculated using the formula:
`CAT_puzzle_hash = curry(CAT_puzzle, asset_id, inner_puzzle_hash)`

Where:
- `CAT_puzzle` is the standard CAT puzzle program
- `asset_id` is the unique identifier for the token type (DIG in our case)
- `inner_puzzle_hash` is the owner's standard puzzle hash

**DIG Token Specifics**:
- **Asset ID**: `a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81`
- **Token Type**: CAT (Chia Asset Token)
- **Decimal Places**: 3 (1 DIG = 1000 mojos)
- **Query Method**: Standard coin state queries using calculated CAT puzzle hash
- **Acquisition**: Available on TibetSwap DEX at [https://v2.tibetswap.io/](https://v2.tibetswap.io/)

#### DIG Token Acquisition via TibetSwap Integration

When users have insufficient DIG collateral, they need a clear, simple path to acquire DIG tokens. TibetSwap provides the primary decentralized exchange for DIG tokens on the Chia blockchain.

**TibetSwap Integration Benefits**:

1. **Decentralized Exchange**: TibetSwap is a leading DEX on Chia, providing trustless DIG token trading
2. **XCH to DIG Trading**: Users can directly exchange their existing XCH for DIG tokens
3. **Immediate Availability**: Tokens are available immediately after transaction confirmation
4. **Fair Market Pricing**: Decentralized price discovery ensures fair market rates
5. **No KYC Required**: Decentralized trading without identity requirements

**User Experience Flow for Token Acquisition**:

When digstore detects insufficient DIG collateral, it provides a streamlined user experience:

```rust
/// Enhanced error messaging with TibetSwap integration
fn display_insufficient_collateral_guidance(balance_info: &DigTokenBalanceInfo, config: &DataLayerConfig) {
    eprintln!();
    eprintln!("{}", "❌ COMMIT BLOCKED: Insufficient DIG Token Collateral".red().bold());
    eprintln!();
    
    // Detailed balance breakdown
    eprintln!("Repository requires: {} DIG tokens", balance_info.required_balance as f64 / 1000.0);
    eprintln!("Current balance: {} DIG tokens", balance_info.current_balance as f64 / 1000.0);
    eprintln!("Additional needed: {} DIG tokens", balance_info.shortfall as f64 / 1000.0);
    eprintln!();
    
    // TibetSwap integration guidance
    if config.show_exchange_link {
        eprintln!("{}", "🔗 Acquire DIG Tokens:".bright_blue().bold());
        eprintln!("   1. Visit TibetSwap: {}", config.dig_token_exchange_url.bright_blue().underline());
        eprintln!("   2. Connect your Chia wallet");
        eprintln!("   3. Exchange XCH for {} DIG tokens", balance_info.shortfall as f64 / 1000.0);
        eprintln!("   4. Send DIG tokens to: {}", balance_info.wallet_address.bright_cyan());
        eprintln!();
    }
    
    // Alternative methods
    eprintln!("{}", "📋 Alternative Methods:".bright_green().bold());
    eprintln!("   • Use existing DIG tokens if you have them in another wallet");
    eprintln!("   • Purchase DIG tokens from other exchanges");
    eprintln!("   • Receive DIG tokens from other users");
    eprintln!();
    
    // Economic context
    eprintln!("{}", "💡 Why DIG Collateral is Required:".bright_yellow().bold());
    eprintln!("   • Prevents spam repositories on the DataLayer network");
    eprintln!("   • Creates economic incentives for data quality");
    eprintln!("   • Funds network operations and storage incentives");
    eprintln!("   • Ensures repository owners have stake in data integrity");
    eprintln!();
    
    // Next steps
    eprintln!("{}", "🚀 After Acquiring DIG Tokens:".bright_cyan().bold());
    eprintln!("   1. Wait for transaction confirmation");
    eprintln!("   2. Run 'digstore status' to verify balance");
    eprintln!("   3. Retry your commit operation");
}
```

**TibetSwap Technical Integration**:

The integration with TibetSwap is primarily informational—digstore provides users with the correct URL and guidance, but doesn't directly integrate with TibetSwap's trading infrastructure. This approach maintains simplicity while providing users with a clear path to token acquisition.

**Configuration Flexibility**:
The TibetSwap URL is configurable in the DataLayer configuration, allowing for:
- **Alternative Exchanges**: Users can configure different exchange URLs
- **Network-Specific Exchanges**: Different exchanges for mainnet vs testnet
- **Custom Trading Solutions**: Integration with custom or private trading systems
- **Disable Exchange Links**: Option to disable exchange link display

**User Guidance Principles**:
- **Clear Instructions**: Step-by-step guidance for token acquisition
- **Economic Context**: Explanation of why collateral is required
- **Multiple Options**: Present various ways to acquire DIG tokens
- **Next Steps**: Clear instructions for post-acquisition workflow

**Step 4: Validation and User Feedback**
```rust
if total_dig_balance >= required_dig_mojos {
    println!("✓ Sufficient DIG collateral: {} DIG available", total_dig_balance as f64 / 1000.0);
    // Proceed with DataLayer synchronization
} else {
    let shortfall = required_dig_mojos - total_dig_balance;
    eprintln!("❌ Insufficient DIG collateral:");
    eprintln!("  Required: {} DIG tokens", required_dig_mojos as f64 / 1000.0);
    eprintln!("  Available: {} DIG tokens", total_dig_balance as f64 / 1000.0);
    eprintln!("  Shortfall: {} DIG tokens", shortfall as f64 / 1000.0);
    eprintln!("  Send DIG tokens to: {}", wallet_address);
    
    return Err(DigstoreError::InsufficientCollateral { /* details */ });
}
```

#### 4.2 Comprehensive Melt Method Signatures
```rust
impl DataStoreCoin {
    /// Primary melt method - secure, returns unsigned coin spends
    pub async fn melt(&self) -> Result<MeltResult>
    
    /// Broadcast signed melt transaction (separate for security)
    pub async fn broadcast_melt(&self, signed_coin_spends: Vec<CoinSpend>) -> Result<()>
}
```

#### 4.2 Melt Implementation Steps (NEW FEATURE)
1. **Owner Key Validation**:
   - Ensure current wallet is store owner
   - Get owner public key from wallet

2. **Melt Transaction Creation**:
   - Call `melt_store_rust` with store and owner key
   - Generate unsigned coin spends
   - Return for external signing (security best practice)

3. **Broadcasting** (separate method):
   - Provide `broadcast_melt` method for signed coin spends
   - Handle transaction broadcasting
   - Cleanup internal state

**Note**: This functionality is available in DataLayer-Driver but not implemented in the TypeScript DataStore. The Rust implementation will provide this additional capability.

### Phase 5: Error Handling and Integration

#### 5.1 Comprehensive Error Types with DIG Collateral Support
```rust
#[derive(Debug, thiserror::Error)]
pub enum DataStoreCoinError {
    #[error("Wallet error: {0}")]
    Wallet(#[from] crate::wallet::WalletError),
    
    #[error("DataLayer error: {0}")]
    DataLayer(String),
    
    #[error("Network error: {0}")]
    Network(String),
    
    #[error("Permission error: {0}")]
    Permission(String),
    
    #[error("Store not found: {0}")]
    StoreNotFound(String),
    
    #[error("Invalid authority: expected {expected}, got {actual}")]
    InvalidAuthority { expected: String, actual: String },
    
    #[error("Insufficient DIG collateral: need {required} DIG, have {available} DIG (shortfall: {shortfall} DIG)")]
    InsufficientCollateral {
        required: u64,              // Required DIG tokens in mojos
        available: u64,             // Available DIG tokens in mojos
        shortfall: u64,             // Shortfall in DIG tokens in mojos
        wallet_address: String,     // Wallet address for receiving DIG tokens
    },
    
    #[error("DIG token query failed: {0}")]
    DigTokenQueryFailed(String),
    
    #[error("CAT coin query failed: {0}")]
    CatCoinQueryFailed(String),
    
    #[error("Collateral calculation error: {0}")]
    CollateralCalculationError(String),
    
    #[error("State synchronization failed: local and blockchain state are inconsistent")]
    StateSynchronizationFailed {
        local_root: String,         // Local commit hash
        blockchain_root: String,    // Blockchain root hash
        local_size: u64,            // Local repository size
        blockchain_size: u64,       // Blockchain reported size
    },
}

/// Enhanced DigstoreError with DIG collateral support
#[derive(Debug, thiserror::Error)]
pub enum DigstoreError {
    // ... existing error variants ...
    
    #[error("Insufficient DIG collateral for repository size")]
    InsufficientCollateral {
        required: u64,              // Required DIG tokens in mojos
        available: u64,             // Available DIG tokens in mojos  
        shortfall: u64,             // Shortfall in DIG tokens in mojos
        wallet_address: String,     // Wallet address for receiving DIG tokens
    },
}
```

#### 5.2 Integration Points
1. **Digstore Store Integration**:
   - Sync DataLayer root hash with digstore commits
   - Update store metadata when repository changes
   - Maintain consistency between local and blockchain state

2. **CLI Integration**:
   - Add `digstore datastore` command group
   - Subcommands: `mint`, `update`, `melt`, `info`, `sync`
   - Progress indicators for blockchain operations

## Advanced Features

### 1. Store Monitoring and Synchronization
```rust
impl DataStoreCoin {
    /// Sync store state from blockchain
    pub async fn sync_from_blockchain(&mut self) -> Result<DataStore>;
    
    /// Get store creation height and hash
    pub async fn get_creation_info(&self) -> Result<(u32, Bytes32)>;
    
    /// Check if store has specific root hash
    pub async fn has_root_hash(&self, root_hash: Bytes32) -> Result<bool>;
}
```

### 2. Permission Management
```rust
impl DataStoreCoin {
    /// Check if current wallet has write permissions
    pub async fn has_write_permissions(&self) -> Result<bool>;
    
    /// Check if current wallet has admin permissions  
    pub async fn has_admin_permissions(&self) -> Result<bool>;
    
    /// Check if current wallet is store owner
    pub async fn is_owner(&self) -> Result<bool>;
}
```

### 3. Metadata Synchronization
```rust
impl DataStoreCoin {
    /// Update DataLayer when digstore commits
    pub async fn sync_with_digstore_commit(
        &mut self, 
        new_root_hash: Bytes32,
        commit_message: Option<String>
    ) -> Result<()>;
    
    /// Get current metadata from blockchain
    pub async fn get_current_metadata(&self) -> Result<DataStoreMetadata>;
}
```

## Implementation Dependencies

### Cargo.toml Additions
```toml
[dependencies]
# Already added
datalayer-driver = "0.1.50"

# Additional for async operations
tokio = { version = "1.35", features = ["rt-multi-thread", "macros"] }
futures-util = "0.3"

# For error handling
thiserror = "1.0"
anyhow = "1.0"

# For serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# For hex encoding
hex = "0.4"

# For logging
tracing = "0.1"
```

### Module Structure
```
src/
├── datastore/
│   ├── mod.rs              # Module exports
│   ├── coin.rs             # DataStoreCoin implementation
│   ├── metadata.rs         # Metadata management
│   ├── permissions.rs      # Permission checking
│   └── sync.rs             # Blockchain synchronization
├── cli/
│   └── commands/
│       └── datastore.rs    # CLI commands for DataStore operations
```

## Security Considerations

### 1. Key Management
- **Wallet Integration**: Always use active wallet for key operations
- **Permission Validation**: Verify authority before operations
- **Signature Security**: Use appropriate private keys for different operations

### 2. Transaction Safety
- **Fee Management**: Proper fee calculation and coin selection
- **Atomic Operations**: Ensure transaction consistency
- **Error Recovery**: Handle partial transaction failures

### 3. Network Security
- **Peer Validation**: Use trusted peer connections
- **Transaction Verification**: Validate transaction responses
- **State Consistency**: Maintain consistent local/blockchain state

## Testing Strategy

### 1. Unit Tests
- **Mint Operation**: Test store creation with various parameters
- **Update Operations**: Test metadata updates with different authorities
- **Melt Operation**: Test store destruction and coin recovery
- **Permission Checks**: Validate authority checking logic

### 2. Integration Tests
- **Wallet Integration**: Test with different wallet profiles
- **Blockchain Integration**: Test with testnet blockchain
- **CLI Integration**: Test command-line interface
- **Error Scenarios**: Test network failures and permission errors

### 3. End-to-End Tests
- **Complete Workflow**: Mint → Update → Melt lifecycle
- **Multi-User Scenarios**: Test delegation and permissions
- **Content Synchronization**: Test digstore ↔ DataLayer sync
- **Network Resilience**: Test with network interruptions

## UI/UX Requirements for Blockchain Operations

### 1. Progress Indicators for Long Operations

#### Spinner Specifications
```rust
// Required spinner patterns for blockchain operations
use indicatif::{ProgressBar, ProgressStyle};
use colored::Colorize;

// 1. Blockchain confirmation spinner (init command)
let confirmation_spinner = ProgressBar::new_spinner();
confirmation_spinner.set_style(
    ProgressStyle::default_spinner()
        .template("{spinner:.green} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
);
confirmation_spinner.set_message("Waiting for blockchain confirmation...");

// 2. Metadata update spinner (commit command)  
let update_spinner = ProgressBar::new_spinner();
update_spinner.set_style(
    ProgressStyle::default_spinner()
        .template("{spinner:.blue} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
);
update_spinner.set_message("Broadcasting metadata update to blockchain...");

// 3. Store minting spinner (init command)
let mint_spinner = ProgressBar::new_spinner(); 
mint_spinner.set_style(
    ProgressStyle::default_spinner()
        .template("{spinner:.cyan} {msg}")
        .unwrap()
);
mint_spinner.set_message("Creating DataLayer store on blockchain...");
```

#### Success/Failure Messages
```rust
// Success patterns
spinner.finish_with_message("✓ Blockchain confirmation received".green().to_string());
spinner.finish_with_message("✓ DataLayer store created".green().to_string());
spinner.finish_with_message("✓ Metadata update successful".green().to_string());

// Failure patterns  
spinner.finish_with_message("✗ Confirmation timeout".red().to_string());
spinner.finish_with_message("✗ Transaction failed".red().to_string());
spinner.finish_with_message("✗ Network error".red().to_string());
```

### 2. Timeout Handling
- **Confirmation Timeout**: 120 seconds maximum wait
- **Network Timeout**: 30 seconds for peer connections
- **Update Timeout**: 60 seconds for metadata updates
- **Graceful Degradation**: Continue digstore operations if DataLayer fails

## Performance Requirements

### 1. Blockchain Operations
- **Mint Time**: <30 seconds for store creation
- **Update Time**: <15 seconds for metadata updates
- **Sync Time**: <10 seconds for state synchronization
- **Memory Usage**: <100MB for blockchain operations

### 2. Integration Performance
- **CLI Responsiveness**: <5 seconds for status operations
- **Background Sync**: Non-blocking blockchain synchronization
- **Error Recovery**: <3 seconds for retry operations
- **Cache Efficiency**: Minimize redundant blockchain queries

## Integration with Existing Commands

### 1. `digstore init` Command Integration

#### Current Behavior
```rust
// Current: Generate random store ID
let store_id = StoreId::generate(); // Random 32-byte ID
```

#### New Behavior (DataStoreCoin Integration with Progress Feedback)
```rust
// New: Create DataStoreCoin first, use its launcher ID as store ID
pub async fn execute_init(
    label: Option<String>,
    description: Option<String>, 
    authorized_writer: Option<String>,
    admin: Option<String>
) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};
    
    println!("{}", "Creating DataLayer store on Chia blockchain...".bright_cyan());
    
    // 1. Create DataStoreCoin and mint on blockchain
    let mut datastore_coin = DataStoreCoin::new(NetworkType::Mainnet)?;
    let launcher_id = datastore_coin.mint(
        label.clone(),
        description.clone(), 
        Some(0), // Initial size
        authorized_writer.map(|key| PublicKey::from_hex(&key)).transpose()?,
        admin.map(|key| PublicKey::from_hex(&key)).transpose()?
    ).await?;
    
    println!("✓ DataLayer store created");
    println!("  Launcher ID: {}", launcher_id.to_hex().bright_cyan());
    println!("  Coin ID: {}", datastore_coin.get_coin_id()?.to_hex().dim());
    
    // 2. Wait for confirmation with spinner (matching TypeScript pattern)
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap()
    );
    spinner.set_message("Waiting for blockchain confirmation...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));
    
    match datastore_coin.wait_for_confirmation().await {
        Ok(_) => {
            spinner.finish_with_message("✓ Blockchain confirmation received".green().to_string());
        }
        Err(e) => {
            spinner.finish_with_message("✗ Confirmation failed".red().to_string());
            return Err(e);
        }
    }
    
    // 3. Use launcher ID as digstore store ID
    let store_id = StoreId::from_bytes(*launcher_id.as_bytes());
    
    // 4. Create digstore repository with DataLayer store ID
    let store = Store::init_with_datastore_id(store_id, &datastore_coin)?;
    
    println!("✓ Repository initialized with DataLayer integration");
    if let Some(label) = label {
        println!("  Store Label: {}", label.green());
    }
    Ok(())
}
```

### 2. `digstore commit` Command Integration

#### Current Behavior
```rust
// Current: Just create digstore commit
let commit_hash = store.commit(message)?;
```

#### New Behavior (DataStoreCoin Integration with Progress Feedback)
```rust
// New: Update DataLayer store after digstore commit
pub async fn execute_commit(message: String) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};
    
    // 1. Create digstore commit (existing logic)
    let commit_hash = store.commit_internal(message)?;
    let total_size = store.calculate_total_size()?;
    
    // 2. Update DataLayer store metadata if available
    if let Some(mut datastore_coin) = store.get_datastore_coin()? {
        println!("{}", "Updating DataLayer store metadata...".bright_blue());
        
        // Create spinner for blockchain operation
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.blue} {msg}")
                .unwrap()
        );
        spinner.set_message("Broadcasting metadata update to blockchain...");
        spinner.enable_steady_tick(std::time::Duration::from_millis(100));
        
        let metadata = DataStoreMetadata {
            root_hash: commit_hash,
            label: None, // Preserve existing label
            description: Some(message.clone()),
            bytes: Some(total_size),
        };
        
        match datastore_coin.update_metadata(metadata).await {
            Ok(updated_store) => {
                spinner.finish_with_message("✓ DataLayer store updated".green().to_string());
                println!("  New root hash: {}", commit_hash.to_hex().bright_cyan());
                println!("  Repository size: {}", format_bytes(total_size).bright_white());
                println!("  Description: {}", message.dim());
            }
            Err(e) => {
                spinner.finish_with_message("✗ DataLayer update failed".red().to_string());
                eprintln!("  Warning: Commit succeeded but DataLayer update failed: {}", e);
                eprintln!("  You can retry the update later with the sync command.");
            }
        }
    }
    
    Ok(())
}
```

## Integration Patterns

### 1. Comprehensive Store Architecture Integration

#### Enhanced Store Struct with Full DataStoreCoin Integration
```rust
// Enhanced Store struct with comprehensive DataStoreCoin integration
pub struct Store {
    // Core digstore fields
    store_id: StoreId,                    // Now derived from DataLayer launcher ID
    current_root: Option<Hash>,           // Current commit hash
    staging_area: StagingArea,            // File staging
    global_path: PathBuf,                 // Path to ~/.dig/{store_id}/
    
    // DataLayer integration fields
    datastore_coin: Option<DataStoreCoin>, // Reference to DataLayer store
    sync_enabled: bool,                   // Whether DataLayer sync is enabled
    last_blockchain_sync: Option<i64>,    // Last successful blockchain sync timestamp
    pending_blockchain_updates: Vec<PendingBlockchainUpdate>, // Updates waiting for blockchain
    
    // State consistency tracking
    local_state_hash: Option<Hash>,       // Hash of current local state
    blockchain_state_hash: Option<Hash>,  // Hash of last known blockchain state
    consistency_check_interval: Duration, // How often to check consistency
    last_consistency_check: Option<Instant>, // Last consistency validation
}

/// Pending blockchain updates for robust synchronization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingBlockchainUpdate {
    pub commit_hash: Hash,
    pub commit_message: String,
    pub repository_size: u64,
    pub timestamp: i64,
    pub retry_count: u32,
    pub last_error: Option<String>,
}

impl Store {
    /// New init method with comprehensive DataStoreCoin integration
    pub async fn init_with_datastore(
        project_path: &Path,
        label: Option<String>,
        description: Option<String>,
        authorized_writer: Option<PublicKey>,
        admin: Option<PublicKey>,
        network: NetworkType,
    ) -> Result<Self> {
        // 1. Create and mint DataStoreCoin first (matching TypeScript DataStore.create)
        let mut datastore_coin = DataStoreCoin::new(network)?;
        
        println!("{}", "🚀 Creating DataLayer store on Chia blockchain...".bright_cyan());
        
        let launcher_id = datastore_coin.mint(
            label.clone(),
            description.clone(),
            Some(0), // Initial size
            authorized_writer,
            admin,
        ).await?;
        
        println!("✓ DataLayer store minted");
        println!("  Launcher ID: {}", launcher_id.to_hex().bright_cyan());
        
        // 2. Wait for blockchain confirmation with progress feedback
        println!("{}", "⏳ Waiting for blockchain confirmation...".bright_blue());
        datastore_coin.wait_for_confirmation().await?;
        println!("✓ Blockchain confirmation received");
        
        // 3. Use launcher ID as store ID (critical architecture change)
        let store_id = StoreId::from_bytes(*launcher_id.as_bytes());
        
        // 4. Initialize digstore with DataLayer-derived ID
        let mut store = Self::init_internal(project_path, store_id)?;
        store.datastore_coin = Some(datastore_coin);
        store.sync_enabled = true;
        store.last_blockchain_sync = Some(chrono::Utc::now().timestamp());
        store.consistency_check_interval = Duration::from_secs(300); // 5 minutes
        
        // 5. Create .digstore file with DataLayer integration info
        store.create_digstore_file_with_datalayer(project_path, label)?;
        
        println!("✓ Repository initialized with DataLayer integration");
        Ok(store)
    }
    
    /// Enhanced commit with mandatory DataLayer synchronization and DIG collateral validation
    pub async fn commit(&mut self, message: &str) -> Result<Hash> {
        // 1. Create digstore commit (existing logic)
        let commit_hash = self.create_commit_internal(message)?;
        let total_size = self.calculate_total_size()?;
        
        println!("✓ Digstore commit created: {}", commit_hash.to_hex().bright_cyan());
        
        // 2. CRITICAL: Check DIG token collateral requirements
        if let Some(ref datastore_coin) = self.datastore_coin {
            if self.sync_enabled {
                // Calculate required DIG collateral
                let size_mb = (total_size as f64 / (1024.0 * 1024.0)).ceil() as u64;
                let required_dig = (size_mb as f64 * 0.1).ceil() as u64; // 0.1 DIG per MB
                
                println!("🔍 Checking DIG token collateral requirements...");
                println!("  Repository size: {} MB", size_mb);
                println!("  Required DIG collateral: {} DIG tokens", required_dig as f64 / 1000.0);
                
                // Check DIG token balance
                match self.check_dig_token_balance(required_dig).await {
                    Ok(balance_info) => {
                        if balance_info.sufficient {
                            println!("✓ Sufficient DIG collateral available");
                            println!("  Current balance: {} DIG", balance_info.current_balance as f64 / 1000.0);
                            println!("  Required: {} DIG", required_dig as f64 / 1000.0);
                            println!("  Available: {} DIG", balance_info.available_balance as f64 / 1000.0);
                        } else {
                            // BLOCK COMMIT - Insufficient DIG collateral
                            eprintln!();
                            eprintln!("{}", "❌ COMMIT BLOCKED: Insufficient DIG Token Collateral".red().bold());
                            eprintln!();
                            eprintln!("Repository size: {} MB", size_mb);
                            eprintln!("Required DIG collateral: {} DIG tokens", required_dig as f64 / 1000.0);
                            eprintln!("Current DIG balance: {} DIG tokens", balance_info.current_balance as f64 / 1000.0);
                            eprintln!("Shortfall: {} DIG tokens", balance_info.shortfall as f64 / 1000.0);
                            eprintln!();
                            eprintln!("Your wallet address: {}", balance_info.wallet_address.bright_cyan());
                            eprintln!();
                            eprintln!("To proceed with this commit, you need to:");
                            eprintln!("1. Acquire {} additional DIG tokens at {}", 
                                     balance_info.shortfall as f64 / 1000.0,
                                     "https://v2.tibetswap.io/".bright_blue().underline());
                            eprintln!("2. Send DIG tokens to your wallet address: {}", balance_info.wallet_address);
                            eprintln!("3. Wait for the transaction to confirm");
                            eprintln!("4. Retry the commit operation");
                            eprintln!();
                            eprintln!("💡 {} Visit TibetSwap to exchange XCH for DIG tokens:", "Tip:".bright_yellow().bold());
                            eprintln!("   {}", "https://v2.tibetswap.io/".bright_blue().underline());
                            eprintln!();
                            eprintln!("Note: DIG tokens are required as collateral for DataLayer storage.");
                            eprintln!("This ensures economic incentives for data integrity and prevents spam.");
                            
                            return Err(DigstoreError::InsufficientCollateral {
                                required: required_dig,
                                available: balance_info.current_balance,
                                shortfall: balance_info.shortfall,
                                wallet_address: balance_info.wallet_address,
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠ Warning: Could not check DIG token balance: {}", e);
                        eprintln!("  Proceeding with commit, but DataLayer sync may fail");
                        eprintln!("  Ensure you have sufficient DIG tokens for collateral");
                    }
                }
            }
        }
        
        // 2. MANDATORY: Update DataLayer metadata (critical for sync)
        if let Some(ref mut datastore_coin) = self.datastore_coin {
            if self.sync_enabled {
                println!("{}", "🔗 Synchronizing with DataLayer...".bright_blue());
                
                let metadata = DataStoreMetadata {
                    root_hash: commit_hash,
                    label: datastore_coin.cached_metadata.as_ref().and_then(|m| m.label.clone()),
                    description: Some(message.to_string()),
                    bytes: Some(total_size),
                };
                
                match datastore_coin.update_metadata(metadata.clone()).await {
                    Ok(_) => {
                        println!("✓ DataLayer store synchronized");
                        println!("  Root hash: {}", commit_hash.to_hex().bright_cyan());
                        println!("  Size: {} bytes", format_bytes(total_size).bright_white());
                        
                        // Update local state tracking
                        self.local_state_hash = Some(commit_hash);
                        self.blockchain_state_hash = Some(commit_hash);
                        self.last_blockchain_sync = Some(chrono::Utc::now().timestamp());
                    }
                    Err(e) => {
                        eprintln!("⚠ DataLayer sync failed: {}", e.to_string().yellow());
                        eprintln!("  Digstore commit succeeded, but blockchain sync failed");
                        eprintln!("  This creates a state inconsistency that needs resolution");
                        
                        // Queue for retry
                        self.queue_pending_blockchain_update(commit_hash, message.to_string(), total_size);
                        
                        // Still return success since digstore commit succeeded
                        eprintln!("  Use 'digstore status' to check sync status");
                    }
                }
            }
        } else {
            // No DataLayer integration - warn user
            eprintln!("⚠ No DataLayer integration configured");
            eprintln!("  Repository is local-only, not synchronized with blockchain");
        }
        
        Ok(commit_hash)
    }
    
    /// Queue pending blockchain update for retry
    fn queue_pending_blockchain_update(&mut self, commit_hash: Hash, message: String, size: u64) {
        self.pending_blockchain_updates.push(PendingBlockchainUpdate {
            commit_hash,
            commit_message: message,
            repository_size: size,
            timestamp: chrono::Utc::now().timestamp(),
            retry_count: 0,
            last_error: None,
        });
    }
    
    /// Process pending blockchain updates with retry logic
    pub async fn process_pending_blockchain_updates(&mut self) -> Result<usize> {
        if let Some(ref mut datastore_coin) = self.datastore_coin {
            let mut processed = 0;
            let mut failed_updates = Vec::new();
            
            for mut update in self.pending_blockchain_updates.drain(..) {
                let metadata = DataStoreMetadata {
                    root_hash: update.commit_hash,
                    label: datastore_coin.cached_metadata.as_ref().and_then(|m| m.label.clone()),
                    description: Some(update.commit_message.clone()),
                    bytes: Some(update.repository_size),
                };
                
                match datastore_coin.update_metadata(metadata).await {
                    Ok(_) => {
                        processed += 1;
                        println!("✓ Synced commit {} to blockchain", update.commit_hash.to_hex()[..8].bright_cyan());
                    }
                    Err(e) => {
                        update.retry_count += 1;
                        update.last_error = Some(e.to_string());
                        
                        if update.retry_count < 3 {
                            failed_updates.push(update);
                        } else {
                            eprintln!("✗ Failed to sync commit {} after 3 retries", 
                                     update.commit_hash.to_hex()[..8].red());
                        }
                    }
                }
            }
            
            self.pending_blockchain_updates = failed_updates;
            Ok(processed)
        } else {
            Ok(0)
        }
    }
    
    /// Validate state consistency between local and blockchain
    pub async fn validate_blockchain_consistency(&mut self) -> Result<ConsistencyStatus> {
        if let Some(ref mut datastore_coin) = self.datastore_coin {
            let current_root = self.current_root.ok_or_else(|| {
                DigstoreError::DataStoreCoinError("No current root in digstore".into())
            })?;
            let total_size = self.calculate_total_size()?;
            
            let is_consistent = datastore_coin.validate_state_consistency(current_root, total_size).await?;
            
            if is_consistent {
                Ok(ConsistencyStatus::InSync)
            } else {
                Ok(ConsistencyStatus::OutOfSync {
                    local_root: current_root,
                    local_size: total_size,
                    blockchain_root: datastore_coin.cached_metadata.as_ref().map(|m| m.root_hash),
                    blockchain_size: datastore_coin.cached_metadata.as_ref().and_then(|m| m.bytes),
                })
            }
        } else {
            Ok(ConsistencyStatus::NoDataLayer)
        }
    }
    
    /// Force synchronization of local state to blockchain
    pub async fn force_sync_to_blockchain(&mut self) -> Result<()> {
        if let Some(ref mut datastore_coin) = self.datastore_coin {
            datastore_coin.force_sync_to_blockchain(self).await?;
            self.last_blockchain_sync = Some(chrono::Utc::now().timestamp());
            Ok(())
        } else {
            Err(DigstoreError::DataStoreCoinError("No DataLayer integration available".into()))
        }
    }
    
    /// Check DIG token balance for collateral requirements
    pub async fn check_dig_token_balance(&self, required_dig_mojos: u64) -> Result<DigTokenBalanceInfo> {
        // Get wallet address for DIG token checking
        let wallet = WalletManager::get_active_wallet()?;
        let wallet_address = wallet.get_owner_public_key().await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get wallet address: {}", e)))?;
        
        // Connect to peer for balance checking
        let peer = if let Some(ref datastore_coin) = self.datastore_coin {
            datastore_coin.connect_peer().await?
        } else {
            // Fallback peer connection
            let (cert_path, key_path) = self.get_ssl_paths()?;
            datalayer_driver::connect_random(
                datalayer_driver::NetworkType::Mainnet,
                &cert_path,
                &key_path
            ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to connect to peer: {}", e)))?
        };
        
        // Get DIG token balance using CAT functionality
        let dig_asset_id = "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81";
        let dig_asset_bytes = hex::decode(dig_asset_id)
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Invalid DIG asset ID: {}", e)))?;
        let dig_asset_id_bytes32 = Bytes32::new(
            dig_asset_bytes.try_into()
                .map_err(|_| DigstoreError::DataStoreCoinError("DIG asset ID must be 32 bytes".into()))?
        );
        
        // Get CAT coins for DIG token
        let cat_coins = self.get_cat_coins_for_asset(&peer, dig_asset_id_bytes32).await?;
        let total_dig_balance = cat_coins.iter().map(|coin| coin.amount).sum::<u64>();
        
        // Calculate balance information
        let sufficient = total_dig_balance >= required_dig_mojos;
        let shortfall = if sufficient { 0 } else { required_dig_mojos - total_dig_balance };
        
        Ok(DigTokenBalanceInfo {
            sufficient,
            current_balance: total_dig_balance,
            required_balance: required_dig_mojos,
            available_balance: total_dig_balance,
            shortfall,
            wallet_address,
        })
    }
    
    /// Get CAT coins for specific asset ID (DIG tokens)
    async fn get_cat_coins_for_asset(&self, peer: &Peer, asset_id: Bytes32) -> Result<Vec<Coin>> {
        // Get wallet puzzle hash for CAT coin queries
        let wallet = WalletManager::get_active_wallet()?;
        let owner_puzzle_hash = wallet.get_owner_puzzle_hash().await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get puzzle hash: {}", e)))?;
        
        // Calculate CAT puzzle hash for the specific asset (DIG tokens)
        // CAT coins have puzzle hashes that include both the asset ID and the inner puzzle hash
        let cat_puzzle_hash = self.calculate_cat_puzzle_hash(asset_id, owner_puzzle_hash)?;
        
        // Query for unspent coins with the CAT puzzle hash
        let unspent_response = datalayer_driver::async_api::get_all_unspent_coins_rust(
            peer,
            cat_puzzle_hash,
            None, // Start from genesis
            match self.network {
                NetworkType::Mainnet => datalayer_driver::constants::get_mainnet_genesis_challenge(),
                NetworkType::Testnet11 => datalayer_driver::constants::get_testnet11_genesis_challenge(),
            }
        ).await.map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to query CAT coins: {}", e)))?;
        
        // Extract coins from coin states
        let cat_coins: Vec<Coin> = unspent_response.coin_states
            .into_iter()
            .filter(|cs| cs.spent_height.is_none()) // Only unspent coins
            .map(|cs| cs.coin)
            .collect();
        
        Ok(cat_coins)
    }
    
    /// Calculate CAT puzzle hash for specific asset ID and inner puzzle hash
    /// Based on chia-wallet-sdk CAT implementation patterns
    fn calculate_cat_puzzle_hash(&self, asset_id: Bytes32, inner_puzzle_hash: Bytes32) -> Result<Bytes32> {
        use chia_puzzle_types::cat::CatArgs;
        use clvm_utils::ToTreeHash;
        
        // Create CAT puzzle hash using asset ID and inner puzzle hash
        // This matches the pattern from chia-wallet-sdk CatInfo::puzzle_hash()
        let cat_args = CatArgs::new(asset_id, inner_puzzle_hash.into());
        let cat_puzzle_hash = cat_args.curry_tree_hash();
        
        Ok(cat_puzzle_hash.into())
    }
}

/// DIG token balance information
#[derive(Debug, Clone)]
pub struct DigTokenBalanceInfo {
    pub sufficient: bool,              // Whether balance meets requirements
    pub current_balance: u64,          // Current DIG balance in mojos
    pub required_balance: u64,         // Required DIG balance in mojos  
    pub available_balance: u64,        // Available DIG balance (excluding locked)
    pub shortfall: u64,                // Amount of DIG tokens short (if insufficient)
    pub wallet_address: String,        // Wallet address for receiving DIG tokens
}
    
    /// Load existing store with DataStoreCoin integration
    pub async fn open_with_datastore(project_path: &Path) -> Result<Self> {
        // 1. Load .digstore file
        let digstore_file = DigstoreFile::load(&project_path.join(".digstore"))?;
        let store_id = StoreId::from_hex(&digstore_file.store_id)?;
        
        // 2. Load digstore repository
        let mut store = Self::open_internal(project_path, store_id)?;
        
        // 3. Try to load DataStoreCoin if DataLayer integration is enabled
        if digstore_file.datalayer_integration.unwrap_or(false) {
            match DataStoreCoin::from_launcher_id(
                Bytes32::new(*store_id.as_bytes()), 
                digstore_file.network.unwrap_or(NetworkType::Mainnet)
            ).await {
                Ok(datastore_coin) => {
                    store.datastore_coin = Some(datastore_coin);
                    store.sync_enabled = true;
                    
                    // Validate consistency on load
                    match store.validate_blockchain_consistency().await? {
                        ConsistencyStatus::InSync => {
                            println!("✓ DataLayer integration loaded and in sync");
                        }
                        ConsistencyStatus::OutOfSync { .. } => {
                            eprintln!("⚠ DataLayer state out of sync with local repository");
                            eprintln!("  Use 'digstore status' to check details");
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    eprintln!("⚠ Failed to load DataLayer integration: {}", e);
                    eprintln!("  Repository will operate in local-only mode");
                    store.sync_enabled = false;
                }
            }
        }
        
        Ok(store)
    }
}

/// Consistency status tracking
#[derive(Debug, Clone)]
pub enum ConsistencyStatus {
    InSync,
    OutOfSync {
        local_root: Hash,
        local_size: u64,
        blockchain_root: Option<Bytes32>,
        blockchain_size: Option<u64>,
    },
    NoDataLayer,
    SyncInProgress,
    SyncFailed(String),
}
```

### 2. Comprehensive Configuration Integration with .dig Folder Management

The configuration system must manage all aspects of DataLayer integration, including SSL certificates, network settings, and operational parameters. All configuration and certificates are stored in the `.dig` global folder to maintain consistency with digstore's architecture.

#### Configuration Structure
```rust
/// Comprehensive DataLayer configuration with SSL and network management
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataLayerConfig {
    // Core functionality
    pub enabled: bool,                      // Whether DataLayer integration is enabled
    pub network: NetworkType,               // Mainnet or Testnet11
    pub auto_sync: bool,                    // Automatic sync on commits
    pub oracle_fee: u64,                    // Oracle fee for store creation (mojos)
    
    // SSL configuration in .dig folder (NOT ~/.chia)
    pub ssl_cert_path: Option<PathBuf>,     // Custom SSL cert path in ~/.dig/ssl/
    pub ssl_key_path: Option<PathBuf>,      // Custom SSL key path in ~/.dig/ssl/
    pub use_dig_ssl: bool,                  // Whether to use .dig folder SSL certs
    pub auto_generate_ssl: bool,            // Auto-generate SSL certs if missing
    
    // Network and peer configuration
    pub trusted_peer: Option<String>,       // Trusted peer address (IP:PORT)
    pub connection_timeout: u64,            // Peer connection timeout in seconds
    pub confirmation_timeout: u64,          // Transaction confirmation timeout in seconds
    pub retry_attempts: u32,                // Number of retry attempts for failed operations
    pub retry_delay_base: u64,              // Base delay for exponential backoff (seconds)
    pub max_retry_delay: u64,               // Maximum retry delay (seconds)
    
    // Synchronization configuration
    pub sync_interval: u64,                 // Background sync interval (seconds)
    pub cache_duration: u64,                // Metadata cache duration (seconds)
    pub consistency_check_interval: u64,    // How often to check local/blockchain consistency
    pub force_sync_on_startup: bool,        // Force sync when opening repositories
    
    // Melt operation configuration (security-critical)
    pub melt_confirmation_required: bool,   // Require explicit "CONFIRM MELT" phrase
    pub melt_backup_check: bool,            // Check for backups before allowing melt
    pub melt_cost_warning_threshold: u64,   // Warn if melt costs more than this (mojos)
    pub melt_require_final_confirmation: bool, // Require final confirmation before broadcast
    
    // DIG token acquisition guidance
    pub dig_token_exchange_url: String,     // URL for DIG token acquisition (TibetSwap)
    pub show_exchange_link: bool,           // Whether to show exchange link in error messages
    
    // Advanced features
    pub enable_background_monitoring: bool, // Monitor blockchain for external changes
    pub delegation_support: bool,           // Support admin/writer delegation
    pub multi_network_support: bool,        // Support multiple networks simultaneously
}

impl Default for DataLayerConfig {
    fn default() -> Self {
        Self {
            // Conservative defaults for production use
            enabled: false,
            network: NetworkType::Mainnet,
            auto_sync: true,
            oracle_fee: 100_000, // 100k mojos (standard oracle fee)
            
            // SSL managed in .dig folder
            ssl_cert_path: None, // Auto-generated in ~/.dig/ssl/datalayer_client.crt
            ssl_key_path: None,  // Auto-generated in ~/.dig/ssl/datalayer_client.key
            use_dig_ssl: true,
            auto_generate_ssl: true,
            
            // Conservative network defaults
            trusted_peer: None,
            connection_timeout: 30,          // 30 second connection timeout
            confirmation_timeout: 120,      // 2 minute confirmation timeout
            retry_attempts: 3,
            retry_delay_base: 2,            // 2, 4, 8 second delays
            max_retry_delay: 60,            // Maximum 1 minute delay
            
            // Synchronization defaults
            sync_interval: 300,             // Sync every 5 minutes
            cache_duration: 300,            // 5 minute metadata cache
            consistency_check_interval: 600, // Check consistency every 10 minutes
            force_sync_on_startup: true,
            
            // Secure melt defaults
            melt_confirmation_required: true,
            melt_backup_check: true,
            melt_cost_warning_threshold: 1_000_000, // Warn if costs > 1M mojos
            melt_require_final_confirmation: true,
            
            // DIG token acquisition defaults
            dig_token_exchange_url: "https://v2.tibetswap.io/".to_string(),
            show_exchange_link: true,
            
            // Advanced features disabled by default
            enable_background_monitoring: false,
            delegation_support: false,
            multi_network_support: false,
        }
    }
}
```

#### SSL Certificate Management in .dig Folder

All SSL certificates and keys are managed within the `.dig` global folder structure, maintaining consistency with digstore's architecture and avoiding conflicts with existing Chia installations.

**SSL Folder Structure**:
```
~/.dig/
├── ssl/
│   ├── datalayer_client.crt    # Client certificate for DataLayer communication
│   ├── datalayer_client.key    # Client private key
│   ├── ca.crt                  # Certificate authority (if needed)
│   └── ssl_config.toml         # SSL configuration metadata
├── config.toml                 # Global digstore configuration
└── {store_id}.dig              # Repository archives
```

**Why .dig Folder for SSL**:
1. **Consistency**: Maintains digstore's architecture of keeping all data in .dig
2. **Isolation**: Avoids conflicts with existing Chia node SSL certificates
3. **Portability**: SSL configuration travels with digstore installation
4. **Security**: Dedicated certificates for DataLayer operations only
5. **Management**: Centralized certificate lifecycle management

#### SSL Certificate Generation and Management in .dig Folder

All SSL certificates for DataLayer communication are generated and managed within the `.dig` global folder, ensuring complete isolation from existing Chia installations and maintaining digstore's architectural consistency.

**SSL Management Implementation**:
```rust
impl DataStoreCoin {
    /// Get or create SSL certificates in ~/.dig/ssl/ folder
    fn get_ssl_paths(&self) -> Result<(String, String)> {
        let dig_dir = dirs::home_dir()
            .ok_or_else(|| DigstoreError::DataStoreCoinError("Cannot find home directory".into()))?
            .join(".dig");
        
        let ssl_dir = dig_dir.join("ssl");
        std::fs::create_dir_all(&ssl_dir)
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to create SSL directory: {}", e)))?;
        
        let cert_path = ssl_dir.join("datalayer_client.crt");
        let key_path = ssl_dir.join("datalayer_client.key");
        
        // Generate SSL certificates if they don't exist
        if !cert_path.exists() || !key_path.exists() {
            self.generate_ssl_certificates(&cert_path, &key_path)?;
            println!("✓ Generated SSL certificates for DataLayer communication");
            println!("  Certificate: {}", cert_path.display().to_string().dim());
            println!("  Private Key: {}", key_path.display().to_string().dim());
        }
        
        Ok((
            cert_path.to_string_lossy().to_string(),
            key_path.to_string_lossy().to_string()
        ))
    }
    
    /// Generate SSL certificates for DataLayer communication using DataLayer-Driver Tls
    /// Matches the exact pattern used in dig-chia-sdk getOrCreateSSLCerts()
    fn generate_ssl_certificates(&self, cert_path: &Path, key_path: &Path) -> Result<()> {
        use datalayer_driver::Tls;
        
        // Create Tls object - this automatically generates certificates if they don't exist
        // This matches the exact pattern from dig-chia-sdk: new Tls(certPath, keyPath)
        let _tls = Tls::new(
            cert_path.to_string_lossy().to_string(),
            key_path.to_string_lossy().to_string()
        ).map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to create TLS certificates: {}", e)))?;
        
        // The Tls constructor automatically generates certificates if they don't exist at the provided paths
        // This is the same pattern used throughout dig-chia-sdk for SSL certificate management
        
        // Set appropriate file permissions (readable only by owner)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            
            // Set secure permissions on both certificate and key files
            if cert_path.exists() {
                let mut perms = std::fs::metadata(cert_path)?.permissions();
                perms.set_mode(0o644); // Owner read/write, group/other read
                std::fs::set_permissions(cert_path, perms)?;
            }
            
            if key_path.exists() {
                let mut perms = std::fs::metadata(key_path)?.permissions();
                perms.set_mode(0o600); // Owner read/write only
                std::fs::set_permissions(key_path, perms)?;
            }
        }
        
        Ok(())
    }
    
    /// Connect to Chia peer using .dig folder SSL certificates
    async fn connect_peer(&self) -> Result<Peer> {
        let (cert_path, key_path) = self.get_ssl_paths()?;
        
        // Use datalayer-driver's connect_random with .dig SSL certificates
        connect_random(self.network, &cert_path, &key_path)
            .await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to connect to peer: {}", e)))
    }
}
```

**SSL Certificate Lifecycle Management**:

**Automatic Generation**: SSL certificates are automatically generated using DataLayer-Driver's `Tls::new()` constructor, which creates certificates if they don't exist at the specified paths. This matches the exact pattern used in dig-chia-sdk's `getOrCreateSSLCerts()` function.

**DataLayer-Driver Integration**: The `Tls` constructor from DataLayer-Driver handles all certificate generation automatically. When you call `Tls::new(cert_path, key_path)`, it checks if certificates exist at those paths and generates them if they don't, just like the TypeScript implementation.

**Secure Storage**: Certificates are stored in the `.dig/ssl/` directory with appropriate file permissions:
- Certificate files: 0o644 (owner read/write, group/other read)
- Private key files: 0o600 (owner read/write only)

**Reuse Across Operations**: Once generated, the same certificates are used for all DataLayer operations, providing consistent authentication across all blockchain interactions.

**Isolation from Chia**: The certificates are completely separate from any existing Chia node installations (which typically use `~/.chia/*/config/ssl/`), preventing conflicts and ensuring independence.

**Portability**: The entire `.dig` folder, including SSL certificates, can be copied between systems for portable digstore installations.

#### Comprehensive CAT Token Integration for DIG Balance Checking

The DIG token balance checking requires deep integration with Chia's CAT (Chia Asset Token) system. This integration involves understanding CAT puzzle structures, coin querying mechanisms, and balance calculation methods.

**CAT Token Architecture Understanding**:

CAT tokens in Chia are implemented as wrapped coins where the outer puzzle enforces token-specific rules while the inner puzzle handles ownership. For DIG tokens:

1. **Outer CAT Puzzle**: Enforces DIG token rules and supply constraints
2. **Asset ID**: Unique identifier (`a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81`) 
3. **Inner Puzzle**: Standard ownership puzzle (user's puzzle hash)
4. **Combined Puzzle Hash**: Cryptographic combination of CAT puzzle + asset ID + inner puzzle

**DIG Token Query Implementation Details**:

The process of querying DIG token balances involves several complex steps that integrate multiple Chia blockchain concepts:

```rust
/// Comprehensive DIG token balance checking implementation
impl Store {
    /// Check DIG token balance with detailed CAT coin analysis
    pub async fn check_dig_token_balance_detailed(&self, required_dig_mojos: u64) -> Result<DetailedDigTokenBalanceInfo> {
        // Step 1: Get wallet information using existing wallet manager
        let wallet = WalletManager::get_active_wallet()?;
        let wallet_address = wallet.get_owner_public_key().await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get wallet address: {}", e)))?;
        let owner_puzzle_hash = wallet.get_owner_puzzle_hash().await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to get puzzle hash: {}", e)))?;
        
        // Step 2: Calculate DIG CAT puzzle hash using chia-wallet-sdk patterns
        const DIG_ASSET_ID: &str = "a406d3a9de984d03c9591c10d917593b434d5263cabe2b42f6b367df16832f81";
        let dig_asset_id = Bytes32::from_hex(DIG_ASSET_ID)
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Invalid DIG asset ID: {}", e)))?;
        
        // Use chia-wallet-sdk CatArgs to calculate proper CAT puzzle hash
        let cat_puzzle_hash = {
            use chia_puzzle_types::cat::CatArgs;
            use clvm_utils::ToTreeHash;
            
            let cat_args = CatArgs::new(dig_asset_id, owner_puzzle_hash.into());
            cat_args.curry_tree_hash().into()
        };
        
        // Step 3: Connect to Chia peer using .dig folder SSL certificates
        let peer = self.connect_peer_for_cat_query().await?;
        
        // Step 4: Query for unspent CAT coins using DataLayer-Driver
        let unspent_response = datalayer_driver::async_api::get_all_unspent_coins_rust(
            &peer,
            cat_puzzle_hash,
            None, // Start from genesis for complete balance
            match self.get_network() {
                NetworkType::Mainnet => datalayer_driver::constants::get_mainnet_genesis_challenge(),
                NetworkType::Testnet11 => datalayer_driver::constants::get_testnet11_genesis_challenge(),
            }
        ).await.map_err(|e| DigstoreError::CatCoinQueryFailed(format!("CAT coin query failed: {}", e)))?;
        
        // Step 5: Filter and analyze CAT coins
        let mut dig_cat_coins = Vec::new();
        let mut total_balance = 0u64;
        let mut coin_count = 0;
        
        for coin_state in unspent_response.coin_states {
            if coin_state.spent_height.is_none() {
                dig_cat_coins.push(coin_state.coin);
                total_balance += coin_state.coin.amount;
                coin_count += 1;
            }
        }
        
        // Step 6: Calculate detailed balance information
        let sufficient = total_balance >= required_dig_mojos;
        let shortfall = if sufficient { 0 } else { required_dig_mojos - total_balance };
        
        Ok(DetailedDigTokenBalanceInfo {
            sufficient,
            current_balance: total_balance,
            required_balance: required_dig_mojos,
            available_balance: total_balance, // All unspent coins are available
            shortfall,
            wallet_address,
            dig_cat_coins,
            coin_count,
            asset_id: dig_asset_id,
            cat_puzzle_hash,
            query_timestamp: chrono::Utc::now().timestamp(),
        })
    }
    
    /// Connect to peer specifically for CAT coin queries with enhanced error handling
    async fn connect_peer_for_cat_query(&self) -> Result<Peer> {
        // Get SSL paths from .dig folder
        let (cert_path, key_path) = self.get_ssl_paths()?;
        
        // Connect using DataLayer-Driver with appropriate network
        let network = self.get_network();
        
        datalayer_driver::async_api::connect_random(network, &cert_path, &key_path)
            .await
            .map_err(|e| DigstoreError::DataStoreCoinError(format!("Failed to connect to peer for CAT query: {}", e)))
    }
}

/// Enhanced DIG token balance information with comprehensive CAT details
#[derive(Debug, Clone)]
pub struct DetailedDigTokenBalanceInfo {
    pub sufficient: bool,               // Whether balance meets requirements
    pub current_balance: u64,           // Current DIG balance in mojos
    pub required_balance: u64,          // Required DIG balance in mojos
    pub available_balance: u64,         // Available DIG balance (excluding locked)
    pub shortfall: u64,                 // Amount of DIG tokens short (if insufficient)
    pub wallet_address: String,         // Wallet address for receiving DIG tokens
    pub dig_cat_coins: Vec<Coin>,       // All DIG CAT coins in wallet
    pub coin_count: usize,              // Number of DIG CAT coins
    pub asset_id: Bytes32,              // DIG asset ID for verification
    pub cat_puzzle_hash: Bytes32,       // Calculated CAT puzzle hash
    pub query_timestamp: i64,           // When this balance was queried
}
```

**Why CAT Querying is Complex**:

CAT tokens require specialized querying because they exist as wrapped coins with embedded asset information. The querying process must:

1. **Calculate Correct Puzzle Hash**: Combine CAT puzzle, asset ID, and owner puzzle hash
2. **Query Specific Coins**: Use the calculated puzzle hash to find only DIG tokens
3. **Validate Asset ID**: Ensure coins actually represent DIG tokens
4. **Handle Spent Status**: Filter out spent coins to get accurate balance
5. **Sum Amounts**: Calculate total balance across all unspent DIG CAT coins

This implementation leverages the chia-wallet-sdk's CAT puzzle calculation methods and DataLayer-Driver's coin querying infrastructure to provide accurate DIG token balance information.

### 3. Enhanced Status Integration with DIG Collateral Monitoring

The status command becomes a critical tool for monitoring both repository state and DIG collateral requirements. This integration provides users with comprehensive information about their repository's blockchain status and economic requirements.

#### Comprehensive Status Command Enhancement
```rust
/// Enhanced status command with complete DataLayer and DIG collateral information
pub async fn execute_status() -> Result<()> {
    // ... existing digstore status logic ...
    
    // Add comprehensive DataLayer status if enabled
    if let Some(datastore_coin) = store.datastore_coin() {
        println!();
        println!("{}", "DataLayer Integration Status".bright_cyan().bold());
        println!("{}", "═══════════════════════════".cyan());
        
        // Basic DataLayer information
        println!("Launcher ID: {}", datastore_coin.launcher_id().to_hex().bright_cyan());
        println!("Network: {}", match datastore_coin.network {
            NetworkType::Mainnet => "Mainnet".green(),
            NetworkType::Testnet11 => "Testnet11".yellow(),
        });
        
        // Synchronization status
        match datastore_coin.sync_status() {
            SyncStatus::InSync => {
                println!("Sync Status: {} In Sync", "✓".green());
            }
            SyncStatus::PendingUpdate => {
                println!("Sync Status: {} Pending Updates", "⏳".yellow());
                println!("  {} pending blockchain updates", datastore_coin.pending_updates.len());
            }
            SyncStatus::SyncInProgress => {
                println!("Sync Status: {} Synchronizing...", "🔄".blue());
            }
            SyncStatus::SyncFailed(error) => {
                println!("Sync Status: {} Failed", "✗".red());
                println!("  Error: {}", error.red());
            }
            SyncStatus::Conflicted => {
                println!("Sync Status: {} Conflicted", "⚠".yellow());
                println!("  Local and blockchain state differ");
            }
            SyncStatus::Unknown => {
                println!("Sync Status: {} Unknown", "?".yellow());
            }
        }
        
        // Current metadata from blockchain
        if let Ok(metadata) = datastore_coin.get_current_metadata().await {
            println!("Blockchain Metadata:");
            println!("  Root Hash: {}", metadata.root_hash.to_hex().bright_cyan());
            if let Some(label) = metadata.label {
                println!("  Label: {}", label.bright_white());
            }
            if let Some(description) = metadata.description {
                println!("  Description: {}", description.dim());
            }
            if let Some(size) = metadata.bytes {
                println!("  Reported Size: {} bytes ({} MB)", size, (size as f64 / (1024.0 * 1024.0)).ceil());
            }
        }
        
        // CRITICAL: DIG Collateral Status
        println!();
        println!("{}", "DIG Token Collateral Status".bright_yellow().bold());
        println!("{}", "═══════════════════════════".yellow());
        
        let total_size = store.calculate_total_size()?;
        let size_mb = (total_size as f64 / (1024.0 * 1024.0)).ceil() as u64;
        let required_dig_mojos = (size_mb as f64 * 0.1 * 1000.0).ceil() as u64;
        
        println!("Repository Size: {} MB", size_mb);
        println!("Required Collateral: {} DIG tokens", required_dig_mojos as f64 / 1000.0);
        
        // Check current DIG balance
        match store.check_dig_token_balance(required_dig_mojos).await {
            Ok(balance_info) => {
                if balance_info.sufficient {
                    println!("Collateral Status: {} Sufficient", "✓".green());
                    println!("  Current Balance: {} DIG", balance_info.current_balance as f64 / 1000.0);
                    println!("  Available Balance: {} DIG", balance_info.available_balance as f64 / 1000.0);
                    println!("  Surplus: {} DIG", (balance_info.current_balance - required_dig_mojos) as f64 / 1000.0);
                } else {
                    println!("Collateral Status: {} Insufficient", "❌".red());
                    println!("  Current Balance: {} DIG", balance_info.current_balance as f64 / 1000.0);
                    println!("  Required Balance: {} DIG", required_dig_mojos as f64 / 1000.0);
                    println!("  Shortfall: {} DIG", balance_info.shortfall as f64 / 1000.0);
                    println!();
                    println!("  {} Commits will be blocked until collateral is met", "⚠".yellow());
                    println!("  {} Get DIG tokens at: {}", "💰".bright_yellow(), "https://v2.tibetswap.io/".bright_blue().underline());
                    println!("  Send DIG tokens to: {}", balance_info.wallet_address.bright_cyan());
                }
                
                println!("Wallet Address: {}", balance_info.wallet_address.bright_cyan());
            }
            Err(e) => {
                println!("Collateral Status: {} Error checking balance", "⚠".yellow());
                println!("  Error: {}", e.to_string().red());
                println!("  Cannot verify DIG token collateral requirements");
            }
        }
        
        // Show last sync information
        if let Some(last_sync) = store.last_blockchain_sync {
            let sync_time = chrono::DateTime::from_timestamp(last_sync, 0)
                .unwrap_or_else(|| chrono::Utc::now());
            println!("Last Sync: {}", sync_time.format("%Y-%m-%d %H:%M:%S UTC").to_string().dim());
        }
        
        // Show pending updates if any
        if !store.pending_blockchain_updates.is_empty() {
            println!();
            println!("{}", "Pending Blockchain Updates".bright_yellow());
            for (i, update) in store.pending_blockchain_updates.iter().enumerate() {
                println!("  {}. Commit {}", i + 1, update.commit_hash.to_hex()[..8].bright_cyan());
                println!("     Size: {} bytes", update.repository_size);
                println!("     Retries: {}", update.retry_count);
                if let Some(error) = &update.last_error {
                    println!("     Last Error: {}", error.red());
                }
            }
        }
    } else {
        println!();
        println!("{}", "DataLayer Integration: Disabled".dim());
        println!("  Repository operates in local-only mode");
        println!("  Use 'digstore init --with-datalayer' for blockchain integration");
    }
    
    Ok(())
}
```

This enhanced status command provides users with complete visibility into:
1. **DataLayer Integration Status**: Whether blockchain integration is active
2. **Synchronization State**: Current sync status and any conflicts
3. **DIG Collateral Requirements**: Real-time collateral status and requirements
4. **Economic Information**: Current balance, requirements, and shortfalls
5. **Pending Operations**: Any blockchain updates waiting for retry
6. **Troubleshooting Information**: Error messages and recovery guidance

## Error Handling Strategy

### 1. Network Resilience with User Feedback
- **Connection Failures**: Show clear spinner failure messages when blockchain unavailable
- **Timeout Handling**: Spinner timeout with helpful error messages
- **Retry Logic**: Spinner updates during retry attempts with exponential backoff
- **Offline Mode**: Graceful spinner termination, continue digstore operations without DataLayer

#### Spinner Error Handling Patterns
```rust
// Timeout handling with spinner
use tokio::time::{timeout, Duration};

let spinner = ProgressBar::new_spinner();
spinner.set_message("Waiting for blockchain confirmation...");

match timeout(Duration::from_secs(120), datastore_coin.wait_for_confirmation()).await {
    Ok(Ok(_)) => {
        spinner.finish_with_message("✓ Blockchain confirmation received".green().to_string());
    }
    Ok(Err(e)) => {
        spinner.finish_with_message("✗ Confirmation failed".red().to_string());
        eprintln!("  Error: {}", e);
    }
    Err(_) => {
        spinner.finish_with_message("⚠ Confirmation timeout (2 minutes)".yellow().to_string());
        eprintln!("  Transaction may still be pending. Check blockchain explorer.");
        eprintln!("  Repository created locally, DataLayer sync can be retried later.");
    }
}

// Retry logic with spinner updates
let mut retry_count = 0;
const MAX_RETRIES: u32 = 3;

while retry_count < MAX_RETRIES {
    spinner.set_message(format!("Attempt {} of {} - Broadcasting to blockchain...", retry_count + 1, MAX_RETRIES));
    
    match datastore_coin.update_metadata(metadata.clone()).await {
        Ok(result) => {
            spinner.finish_with_message("✓ DataLayer update successful".green().to_string());
            return Ok(result);
        }
        Err(e) if retry_count < MAX_RETRIES - 1 => {
            retry_count += 1;
            spinner.set_message(format!("Retrying in {} seconds...", retry_count * 2));
            tokio::time::sleep(Duration::from_secs(retry_count as u64 * 2)).await;
        }
        Err(e) => {
            spinner.finish_with_message("✗ All retry attempts failed".red().to_string());
            return Err(e);
        }
    }
}
```

### 2. Permission Management
- **Authority Validation**: Clear error messages for permission issues
- **Delegation Handling**: Proper support for admin/writer delegation
- **Owner Operations**: Secure handling of owner-only operations
- **Key Mismatches**: Clear guidance when wallet doesn't match store owner

### 3. State Consistency
- **Sync Conflicts**: Handle cases where local/blockchain state diverge
- **Partial Updates**: Recovery from incomplete transactions
- **Version Conflicts**: Handle concurrent modifications
- **Data Validation**: Ensure metadata consistency

## Security Architecture

### 1. Key Security
- **Wallet Isolation**: Use wallet manager for all key operations
- **Private Key Protection**: Never expose private keys directly
- **Signature Verification**: Validate all signatures before broadcasting
- **Authority Separation**: Enforce proper delegation layer permissions

### 2. Transaction Security
- **Fee Validation**: Ensure reasonable fees to prevent overpayment
- **Coin Selection**: Use secure coin selection algorithms
- **Double-Spend Prevention**: Proper coin state tracking
- **Atomic Operations**: Ensure transaction atomicity

### 3. Data Integrity
- **Root Hash Validation**: Verify root hashes match digstore state
- **Metadata Consistency**: Ensure metadata accuracy
- **Blockchain Verification**: Validate all blockchain responses
- **State Synchronization**: Maintain consistent state across systems

## Implementation Priority

### Phase 1: Foundation (Week 1)
1. **Core Structure**: DataStoreCoin struct with launcher ID architecture
2. **Wallet Integration**: Connect with existing wallet system for blockchain ops
3. **Basic Error Handling**: Core error types and async error handling
4. **Configuration**: Add DataLayer config to global settings

### Phase 2: Init Command Integration (Week 2) 
1. **Mint Method**: Complete DataStoreCoin mint functionality
2. **Init Command Update**: Modify `digstore init` to use DataStoreCoin
3. **Store ID Architecture**: Use launcher ID as store ID throughout
4. **Confirmation Handling**: Implement blockchain confirmation waiting

### Phase 3: Commit Command Integration (Week 3)
1. **Update Method**: Complete metadata update functionality  
2. **Commit Command Update**: Modify `digstore commit` to sync with DataLayer
3. **Size Calculation**: Implement repository size calculation
4. **Automatic Sync**: Seamless commit → DataLayer update flow

### Phase 4: Advanced Features (Week 4)
1. **Store Loading**: Load existing DataStoreCoin from launcher ID
2. **State Persistence**: Save/load DataStoreCoin references
3. **Error Recovery**: Handle blockchain failures gracefully
4. **Melt Method**: Optional store destruction functionality (NEW)

### Phase 5: Testing and Polish (Week 5)
1. **Integration Testing**: Test complete init → commit → update cycle
2. **Blockchain Testing**: Test with testnet blockchain
3. **Error Scenarios**: Test network failures and recovery
4. **Documentation**: Update command documentation

## Success Criteria

### Functional Requirements
- ✅ Mint new DataLayer stores from digstore (TypeScript parity)
- ✅ Update store metadata when commits occur (TypeScript parity)
- 🆕 Melt stores to recover collateral (NEW - beyond TypeScript)
- 🆕 Sync state between digstore and blockchain (NEW - enhanced)
- ✅ Proper permission and authority handling (TypeScript parity)

### Performance Requirements
- ✅ Mint operations complete in <30 seconds
- ✅ Update operations complete in <15 seconds
- ✅ Sync operations complete in <10 seconds
- ✅ CLI operations remain responsive during blockchain ops

### Security Requirements
- ✅ All operations use wallet manager for keys
- ✅ Proper authority validation for all operations
- ✅ Secure transaction signing and broadcasting
- ✅ Protection against unauthorized operations

### Integration Requirements
- ✅ Seamless integration with existing digstore CLI
- ✅ Automatic sync with commit operations (optional)
- ✅ Enhanced status reporting with DataLayer info
- ✅ Backward compatibility with non-DataLayer workflows

## Implementation Notes

### 1. Async Architecture
- All blockchain operations are async
- Use tokio runtime for async coordination
- Proper error propagation in async context
- Background operations for non-blocking sync

### 2. State Management
- Cache DataStore state locally for performance
- Invalidate cache on blockchain operations
- Lazy loading of blockchain state
- Efficient state serialization

### 3. Configuration Management
- Add DataLayer settings to global config
- Support network selection (mainnet/testnet)
- Configurable fees and timeouts
- Enable/disable DataLayer integration

### 4. Testing Infrastructure
- Mock blockchain for unit tests
- Testnet integration for integration tests
- Property-based testing for edge cases
- Performance benchmarking for operations

This comprehensive implementation plan provides a robust foundation for integrating Chia DataLayer functionality into digstore while maintaining the existing architecture and user experience.
