//! Test for layers command with multiple layers
//!
//! This test ensures the layers command can list all layers correctly

use anyhow::Result;
use digstore_min::storage::Store;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_layers_command_with_multiple_commits() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create test files for multiple commits
    fs::write(project_path.join("file1.txt"), "First file")?;
    fs::write(project_path.join("file2.txt"), "Second file")?;
    fs::write(project_path.join("file3.txt"), "Third file")?;
    fs::write(project_path.join("file4.txt"), "Fourth file")?;

    let mut store = Store::init(project_path)?;

    // Create multiple commits (layers)

    // First commit
    store.add_file(Path::new("file1.txt"))?;
    let layer1 = store.commit("First commit")?;

    // Second commit
    store.add_file(Path::new("file2.txt"))?;
    let layer2 = store.commit("Second commit")?;

    // Third commit
    store.add_file(Path::new("file3.txt"))?;
    store.add_file(Path::new("file4.txt"))?;
    let layer3 = store.commit("Third commit with multiple files")?;

    // Verify all layers exist in archive
    let archive_layers = store.archive.list_layers();

    // Should have Layer 0 + 3 commit layers = 4 total
    assert!(
        archive_layers.len() >= 4,
        "Should have at least 4 layers (Layer 0 + 3 commits)"
    );

    // Verify specific layers exist
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    assert!(
        store.archive.has_layer(&layer_zero_hash),
        "Should have Layer 0"
    );
    assert!(store.archive.has_layer(&layer1), "Should have layer 1");
    assert!(store.archive.has_layer(&layer2), "Should have layer 2");
    assert!(store.archive.has_layer(&layer3), "Should have layer 3");

    // Test that each layer can be loaded
    let loaded_layer1 = store.load_layer(layer1)?;
    let loaded_layer2 = store.load_layer(layer2)?;
    let loaded_layer3 = store.load_layer(layer3)?;

    // Verify layer contents
    assert_eq!(loaded_layer1.files.len(), 1, "Layer 1 should have 1 file");
    assert_eq!(
        loaded_layer2.files.len(),
        2,
        "Layer 2 should have 2 files (cumulative)"
    );
    assert_eq!(
        loaded_layer3.files.len(),
        4,
        "Layer 3 should have 4 files (cumulative)"
    );

    // Test that layers command can list all layers
    let listed_layers: Vec<_> = archive_layers
        .into_iter()
        .filter(|(hash, _)| *hash != layer_zero_hash) // Exclude Layer 0
        .collect();

    assert_eq!(
        listed_layers.len(),
        3,
        "Should list exactly 3 commit layers"
    );

    // Verify layer hashes match
    let layer_hashes: Vec<_> = listed_layers.iter().map(|(hash, _)| *hash).collect();
    assert!(
        layer_hashes.contains(&layer1),
        "Should contain layer 1 hash"
    );
    assert!(
        layer_hashes.contains(&layer2),
        "Should contain layer 2 hash"
    );
    assert!(
        layer_hashes.contains(&layer3),
        "Should contain layer 3 hash"
    );

    Ok(())
}

#[test]
fn test_layers_command_ordering() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create files
    fs::write(project_path.join("a.txt"), "A")?;
    fs::write(project_path.join("b.txt"), "B")?;
    fs::write(project_path.join("c.txt"), "C")?;

    let mut store = Store::init(project_path)?;

    // Create commits in sequence
    store.add_file(Path::new("a.txt"))?;
    let layer1 = store.commit("Commit A")?;

    // Small delay to ensure different timestamps
    std::thread::sleep(std::time::Duration::from_millis(10));

    store.add_file(Path::new("b.txt"))?;
    let layer2 = store.commit("Commit B")?;

    std::thread::sleep(std::time::Duration::from_millis(10));

    store.add_file(Path::new("c.txt"))?;
    let layer3 = store.commit("Commit C")?;

    // Load layers and verify ordering
    let layer1_data = store.load_layer(layer1)?;
    let layer2_data = store.load_layer(layer2)?;
    let layer3_data = store.load_layer(layer3)?;

    // Verify generation numbers increase
    assert!(layer1_data.header.layer_number <= layer2_data.header.layer_number);
    assert!(layer2_data.header.layer_number <= layer3_data.header.layer_number);

    // Verify timestamps increase (should be sequential)
    assert!(layer1_data.header.timestamp <= layer2_data.header.timestamp);
    assert!(layer2_data.header.timestamp <= layer3_data.header.timestamp);

    // Test that current root is the latest layer
    assert_eq!(store.current_root(), Some(layer3));

    Ok(())
}

#[test]
fn test_layers_command_with_empty_repository() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    let store = Store::init(project_path)?;

    // Should have only Layer 0
    let archive_layers = store.archive.list_layers();
    assert_eq!(
        archive_layers.len(),
        1,
        "Empty repository should have only Layer 0"
    );

    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    assert!(
        store.archive.has_layer(&layer_zero_hash),
        "Should have Layer 0"
    );

    // Current root should be None for empty repository
    assert_eq!(
        store.current_root(),
        None,
        "Empty repository should have no current root"
    );

    Ok(())
}

#[test]
fn test_layers_command_archive_consistency() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    fs::write(project_path.join("test.txt"), "Test content")?;

    let mut store = Store::init(project_path)?;
    store.add_file(Path::new("test.txt"))?;
    let commit_hash = store.commit("Test commit")?;

    // Verify layer exists in archive
    assert!(
        store.archive.has_layer(&commit_hash),
        "Archive should contain the layer"
    );

    // Verify layer can be retrieved
    let layer = store.load_layer(commit_hash)?;
    assert!(!layer.files.is_empty(), "Layer should contain files");
    assert!(!layer.chunks.is_empty(), "Layer should contain chunks");

    // Verify layer metadata
    assert_eq!(layer.metadata.message, Some("Test commit".to_string()));
    assert!(
        layer.header.timestamp > 0,
        "Layer should have valid timestamp"
    );

    Ok(())
}
