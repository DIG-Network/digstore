//! Unit tests for archive format and operations
//!
//! Tests the .dig archive format that stores multiple layers in a single file.

use digstore_min::storage::dig_archive::{DigArchive, get_archive_path};
use digstore_min::core::types::*;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_archive_creation() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("test.dig");
    
    let archive = DigArchive::create(archive_path.clone())?;
    
    // Archive file should exist
    assert!(archive_path.exists());
    
    // Should have empty index initially
    let layers = archive.list_layers();
    assert!(layers.is_empty());
    
    Ok(())
}

#[test]
fn test_archive_add_and_get_layer() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("test.dig");
    
    let mut archive = DigArchive::create(archive_path)?;
    
    // Add test layer data
    let layer_hash = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
    let layer_data = b"Test layer content";
    
    archive.add_layer(layer_hash, layer_data)?;
    
    // Should be able to retrieve the layer
    assert!(archive.has_layer(&layer_hash));
    let retrieved_data = archive.get_layer_data(&layer_hash)?;
    assert_eq!(retrieved_data, layer_data);
    
    // Should appear in layer list
    let layers = archive.list_layers();
    assert_eq!(layers.len(), 1);
    assert!(layers.iter().any(|(hash, _)| *hash == layer_hash));
    
    Ok(())
}

#[test]
fn test_archive_multiple_layers() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("test.dig");
    
    let mut archive = DigArchive::create(archive_path)?;
    
    // Add multiple layers
    let layers = vec![
        (Hash::from_hex("1111111111111111111111111111111111111111111111111111111111111111").unwrap(), b"Layer 1".to_vec()),
        (Hash::from_hex("2222222222222222222222222222222222222222222222222222222222222222").unwrap(), b"Layer 2".to_vec()),
        (Hash::from_hex("3333333333333333333333333333333333333333333333333333333333333333").unwrap(), b"Layer 3".to_vec()),
    ];
    
    for (hash, data) in &layers {
        archive.add_layer(*hash, data)?;
    }
    
    // All layers should be retrievable
    for (hash, expected_data) in &layers {
        assert!(archive.has_layer(hash));
        let retrieved_data = archive.get_layer_data(hash)?;
        assert_eq!(retrieved_data, *expected_data);
    }
    
    // Layer count should be correct
    let layer_list = archive.list_layers();
    assert_eq!(layer_list.len(), 3);
    
    Ok(())
}

#[test]
fn test_archive_reopen() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let archive_path = temp_dir.path().join("persistent.dig");
    
    let layer_hash = Hash::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let layer_data = b"Persistent layer data";
    
    // Create archive and add data
    {
        let mut archive = DigArchive::create(archive_path.clone())?;
        archive.add_layer(layer_hash, layer_data)?;
    } // Archive goes out of scope
    
    // Reopen archive
    let archive = DigArchive::open(archive_path)?;
    
    // Data should still be accessible
    assert!(archive.has_layer(&layer_hash));
    let retrieved_data = archive.get_layer_data(&layer_hash)?;
    assert_eq!(retrieved_data, layer_data);
    
    Ok(())
}

#[test]
fn test_get_archive_path() -> anyhow::Result<()> {
    let store_id = Hash::from_hex("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890").unwrap();
    
    let archive_path = get_archive_path(&store_id)?;
    
    // Should be in .dig directory
    assert!(archive_path.to_string_lossy().contains(".dig"));
    
    // Should have .dig extension
    assert_eq!(archive_path.extension().unwrap(), "dig");
    
    // Should contain store ID
    assert!(archive_path.file_name().unwrap().to_string_lossy().contains("abcdef1234567890"));
    
    Ok(())
}
