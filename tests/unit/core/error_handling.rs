//! Unit tests for error handling and error types
//!
//! Tests the error system and ensures proper error propagation and formatting.

use digstore_min::core::error::*;
use std::path::PathBuf;

#[test]
fn test_error_creation() {
    let path = PathBuf::from("/test/path");
    
    let store_not_found = DigstoreError::store_not_found(path.clone());
    match store_not_found {
        DigstoreError::StoreNotFound { path: error_path } => {
            assert_eq!(error_path, path);
        }
        _ => panic!("Expected StoreNotFound error"),
    }
}

#[test]
fn test_error_display() {
    let error = DigstoreError::invalid_store_id("invalid_id");
    let error_string = error.to_string();
    assert!(error_string.contains("Invalid store ID"));
    assert!(error_string.contains("invalid_id"));
}

#[test]
fn test_error_from_io() {
    let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
    let digstore_error: DigstoreError = io_error.into();
    
    match digstore_error {
        DigstoreError::Io(_) => {}, // Expected
        _ => panic!("Expected Io error"),
    }
}

#[test]
fn test_result_type_alias() {
    fn test_function() -> Result<i32> {
        Ok(42)
    }
    
    let result = test_function();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn test_error_helper_methods() {
    let internal_error = DigstoreError::internal("Test message");
    assert!(internal_error.to_string().contains("Test message"));
    
    let encryption_error = DigstoreError::encryption_error("Encryption failed");
    assert!(encryption_error.to_string().contains("Encryption failed"));
    
    let decryption_error = DigstoreError::decryption_error("Decryption failed");
    assert!(decryption_error.to_string().contains("Decryption failed"));
}
