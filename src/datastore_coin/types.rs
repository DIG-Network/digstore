//! Type definitions for datastore coins

use crate::core::Hash;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a datastore coin
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoinId(String);

impl CoinId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CoinId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a datastore
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DatastoreId(String);

impl DatastoreId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    pub fn from_hash(hash: &Hash) -> Self {
        Self(hash.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DatastoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Metadata associated with a datastore coin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinMetadata {
    /// The datastore this coin is associated with
    pub datastore_id: DatastoreId,
    
    /// The root hash of the datastore at creation
    pub root_hash: Hash,
    
    /// The size of the datastore in bytes
    pub size_bytes: u64,
    
    /// The amount of DIG tokens locked as collateral (in DIG tokens, not mojos)
    pub collateral_amount: u64,
    
    /// The address that owns this coin
    pub owner_address: String,
    
    /// The address of the data host (if different from owner)
    pub host_address: Option<String>,
    
    /// Creation timestamp (Unix timestamp)
    pub created_at: u64,
    
    /// Expiration timestamp (Unix timestamp)
    pub expires_at: Option<u64>,
    
    /// Additional metadata
    pub extra: Option<serde_json::Value>,
}

/// Requirements for collateral
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralConfig {
    /// Minimum collateral per GB in DIG tokens
    pub min_collateral_per_gb_dig: f64,
    
    /// Maximum datastore size without additional requirements
    pub max_size_standard: u64,
    
    /// Multiplier for large datastores
    pub large_datastore_multiplier: f64,
    
    /// Grace period before collateral can be reclaimed (in seconds)
    pub grace_period_seconds: u64,
}

impl Default for CollateralConfig {
    fn default() -> Self {
        Self {
            min_collateral_per_gb_dig: 0.1, // 0.1 DIG tokens per GB
            max_size_standard: 1024 * 1024 * 1024, // 1 GB
            large_datastore_multiplier: 1.5,
            grace_period_seconds: 86400 * 30, // 30 days
        }
    }
}