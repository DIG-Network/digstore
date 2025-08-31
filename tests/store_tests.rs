//! Store management tests

use digstore_min::{
    storage::store::{Store, get_global_dig_directory, generate_store_id},
    core::{types::*, digstore_file::DigstoreFile, error::DigstoreError}
};
use tempfile::TempDir;
use anyhow::Result;

#[test]
fn test_store_initialization() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    let store = Store::init(project_path)?;

    // Check that .layerstore file was created
    let digstore_path = project_path.join(".layerstore");
    assert!(digstore_path.exists());

    // Check that global store directory was created
    assert!(store.global_path().exists());

    // Check Layer 0 was created
    let layer_zero_path = store.global_path().join("0000000000000000000000000000000000000000000000000000000000000000.layer");
    assert!(layer_zero_path.exists());

    // Verify store properties
    assert_eq!(store.project_path().unwrap(), project_path);
    assert!(store.current_root().is_none()); // No commits yet

    Ok(())
}

#[test]
fn test_store_already_exists() -> Result<()> {
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
fn test_store_open() -> Result<()> {
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
fn test_store_open_global() -> Result<()> {
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
fn test_digstore_file_creation() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    Store::init(project_path)?;

    // Load and verify .layerstore file
    let digstore_path = project_path.join(".layerstore");
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
fn test_layer_zero_initialization() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    let store = Store::init(project_path)?;

    // Check Layer 0 content
    let layer_zero_path = store.global_path().join("0000000000000000000000000000000000000000000000000000000000000000.layer");
    let content = std::fs::read(layer_zero_path)?;
    let metadata: serde_json::Value = serde_json::from_slice(&content)?;

    // Verify metadata structure
    assert_eq!(metadata["store_id"], store.store_id().to_hex());
    assert!(metadata["created_at"].is_number());
    assert_eq!(metadata["format_version"], "1.0");
    assert_eq!(metadata["protocol_version"], "1.0");
    assert!(metadata["root_history"].is_array());
    assert!(metadata["config"].is_object());

    Ok(())
}

#[test]
fn test_global_dig_directory() -> Result<()> {
    let dig_dir = get_global_dig_directory()?;
    
    // Should be in user's home directory
    assert!(dig_dir.ends_with(".layer"));
    assert!(dig_dir.is_absolute());

    Ok(())
}

#[test]
fn test_store_with_custom_name() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize store
    Store::init(project_path)?;

    // Load .layerstore file and check name
    let digstore_path = project_path.join(".layerstore");
    let digstore_file = DigstoreFile::load(&digstore_path)?;

    // Should use directory name as repository name
    let expected_name = project_path.file_name().unwrap().to_str().unwrap();
    assert_eq!(digstore_file.repository_name, Some(expected_name.to_string()));

    Ok(())
}

#[test]
fn test_multiple_stores() -> Result<()> {
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
