//! Security-specific error types

use thiserror::Error;
use crate::core::types::{StoreId, Hash};
use std::path::PathBuf;

/// Security-specific error types
#[derive(Error, Debug)]
pub enum SecurityError {
    #[error("Invalid store ID for access: expected {expected}, got {actual}")]
    InvalidStoreId { expected: StoreId, actual: StoreId },
    
    #[error("Invalid root hash: {hash}")]
    InvalidRootHash { hash: Hash },
    
    #[error("Invalid resource path: {path}")]
    InvalidResourcePath { path: PathBuf },
    
    #[error("URN access denied: missing required component {component}")]
    MissingUrnComponent { component: String },
    
    #[error("Data scrambling failed: {reason}")]
    ScramblingFailed { reason: String },
    
    #[error("Data unscrambling failed: {reason}")]
    UnscramblingFailed { reason: String },
    
    #[error("Access denied: {reason}")]
    AccessDenied { reason: String },
    
    #[error("Legacy format not supported: {format}")]
    LegacyFormatNotSupported { format: String },
}

impl SecurityError {
    /// Create access denied error
    pub fn access_denied(reason: impl Into<String>) -> Self {
        Self::AccessDenied { reason: reason.into() }
    }
    
    /// Create scrambling failed error
    pub fn scrambling_failed(reason: impl Into<String>) -> Self {
        Self::ScramblingFailed { reason: reason.into() }
    }
    
    /// Create missing URN component error
    pub fn missing_urn_component(component: impl Into<String>) -> Self {
        Self::MissingUrnComponent { component: component.into() }
    }
    
    /// Create legacy format error
    pub fn legacy_format_not_supported(format: impl Into<String>) -> Self {
        Self::LegacyFormatNotSupported { format: format.into() }
    }
}

/// Result type for security operations
pub type SecurityResult<T> = std::result::Result<T, SecurityError>;
