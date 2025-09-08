//! Configuration management for Digstore Min
//!
//! This module provides both global and store-specific configuration functionality

pub mod global_config;
pub mod store_config;

// Re-export commonly used items
pub use global_config::{ConfigKey, ConfigValue, GlobalConfig};
pub use store_config::StoreConfig;
