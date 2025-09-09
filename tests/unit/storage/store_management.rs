//! Unit tests for store management
//!
//! Tests store initialization, opening, and basic store operations.

use digstore_min::{
    core::{digstore_file::DigstoreFile, error::DigstoreError, types::*},
    storage::store::{generate_store_id, Store},
};
use tempfile::TempDir;

#[test]
fn test_store_initialization() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    let store = Store::init(project_path)?;

    // Check that .digstore file was created
    let digstore_path = project_path.join(".digstore");
    assert!(digstore_path.exists());

    // Check that global store directory was created
    assert!(store.global_path().exists());

    // Verify store properties
    assert_eq!(store.project_path().unwrap(), project_path);
    assert!(store.current_root().is_none()); // No commits yet

    Ok(())
}

#[test]
fn test_store_already_exists() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store first time
    Store::init(project_path)?;

    // Try to initialize again - should fail
    let result = Store::init(project_path);
    assert!(result.is_err());

    if let Err(DigstoreError::StoreAlreadyExists { path }) = result {
        assert_eq!(path, project_path);
    } else {
        panic!("Expected StoreAlreadyExists error");
    }

    Ok(())
}

#[test]
fn test_store_open() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    let original_store = Store::init(project_path)?;
    let original_store_id = original_store.store_id();

    // Open the store
    let opened_store = Store::open(project_path)?;

    // Should have same store ID and paths
    assert_eq!(opened_store.store_id(), original_store_id);
    assert_eq!(opened_store.global_path(), original_store.global_path());
    assert_eq!(opened_store.project_path().unwrap(), project_path);

    Ok(())
}

#[test]
fn test_store_open_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Try to open store that doesn't exist
    let result = Store::open(project_path);
    assert!(result.is_err());

    if let Err(DigstoreError::StoreNotFound { path }) = result {
        assert_eq!(path, project_path);
    } else {
        panic!("Expected StoreNotFound error");
    }
}

#[test]
fn test_store_open_global() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    let original_store = Store::init(project_path)?;
    let store_id = original_store.store_id();

    // Open by store ID
    let global_store = Store::open_global(&store_id)?;

    // Should have same store ID and global path
    assert_eq!(global_store.store_id(), store_id);
    assert_eq!(global_store.global_path(), original_store.global_path());
    assert!(global_store.project_path().is_none()); // No project context

    Ok(())
}

#[test]
fn test_digstore_file_creation() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    Store::init(project_path)?;

    // Load and verify .digstore file
    let digstore_path = project_path.join(".digstore");
    let digstore_file = DigstoreFile::load(&digstore_path)?;

    assert_eq!(digstore_file.version, "1.0.0");
    assert!(!digstore_file.encrypted);
    assert!(digstore_file.store_id.len() == 64);
    assert!(digstore_file.is_valid());

    // Should be able to parse the store ID
    let store_id = digstore_file.get_store_id()?;
    assert_eq!(store_id.to_hex(), digstore_file.store_id);

    Ok(())
}

#[test]
fn test_store_id_generation() {
    // Generate multiple store IDs
    let id1 = generate_store_id();
    let id2 = generate_store_id();
    let id3 = generate_store_id();

    // Should all be different
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);

    // Should all be non-zero
    assert_ne!(id1, Hash::zero());
    assert_ne!(id2, Hash::zero());
    assert_ne!(id3, Hash::zero());
}

#[test]
fn test_multiple_stores() -> anyhow::Result<()> {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    // Initialize two different stores
    let store1 = Store::init(temp_dir1.path())?;
    let store2 = Store::init(temp_dir2.path())?;

    // Should have different store IDs
    assert_ne!(store1.store_id(), store2.store_id());

    // Should have different global paths
    assert_ne!(store1.global_path(), store2.global_path());

    // Both should be openable
    let opened1 = Store::open(temp_dir1.path())?;
    let opened2 = Store::open(temp_dir2.path())?;

    assert_eq!(opened1.store_id(), store1.store_id());
    assert_eq!(opened2.store_id(), store2.store_id());

    Ok(())
}
