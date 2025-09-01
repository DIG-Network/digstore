//! Global configuration management for Digstore Min
//!
//! This module provides Git-like global configuration functionality,
//! storing user settings in ~/.dig/config.toml

pub mod global_config;

// Re-export commonly used items
pub use global_config::{GlobalConfig, ConfigKey, ConfigValue};
