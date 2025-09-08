//! Wallet management for Digstore Min
//!
//! This module provides wallet initialization and management functionality,
//! ensuring that a wallet is properly configured before running CLI commands.

pub mod wallet_manager;

// Re-export commonly used items
pub use wallet_manager::WalletManager;
