//! Datastore Coin Module
//!
//! This module implements the datastore coin functionality for the DIG Network.
//! Datastore coins are CAT (Colored Coins) on the Chia blockchain that represent
//! storage commitments and require DIG token collateral.

pub mod coin;
pub mod collateral;
pub mod config;
pub mod integration;
pub mod manager;
pub mod types;
pub mod utils;

// Re-export commonly used items
pub use coin::{DatastoreCoin, CoinState};
pub use collateral::{CollateralManager, CollateralRequirement};
pub use manager::DatastoreCoinManager;
pub use types::{CoinId, CoinMetadata, DatastoreId};