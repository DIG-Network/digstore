//! Regression tests for JSON serialization errors
//!
//! These tests ensure that Layer 0 metadata corruption doesn't reoccur.
//! The fix involved removing truncate(true) from the add_layer method.

use digstore_min::storage::Store;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Test for JSON serialization error regression (Layer 0 corruption)
/// This test ensures that Layer 0 metadata is properly written and readable as valid JSON
#[test]
fn test_layer_zero_json_serialization() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Test 1: Initialize repository and check Layer 0 is valid JSON
    let mut store = Store::init(project_path)?;

    // Test 2: Verify Layer 0 exists and contains valid JSON metadata
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    assert!(
        store.archive.has_layer(&layer_zero_hash),
        "Layer 0 should exist after initialization"
    );

    let metadata_bytes_result = store.archive.get_layer_data(&layer_zero_hash);
    assert!(
        metadata_bytes_result.is_ok(),
        "Should be able to read Layer 0 metadata: {:?}",
        metadata_bytes_result.err()
    );

    let metadata_bytes = metadata_bytes_result.unwrap();
    assert!(
        !metadata_bytes.is_empty(),
        "Layer 0 metadata should not be empty"
    );

    // Test 3: Verify the metadata is valid JSON
    let json_result = serde_json::from_slice::<serde_json::Value>(&metadata_bytes);
    assert!(
        json_result.is_ok(),
        "Layer 0 metadata should be valid JSON. Data length: {}, Error: {:?}",
        metadata_bytes.len(),
        json_result.err()
    );

    let metadata = json_result.unwrap();

    // Test 4: Verify required fields are present
    assert!(
        metadata.get("store_id").is_some(),
        "Layer 0 should contain store_id"
    );
    assert!(
        metadata.get("created_at").is_some(),
        "Layer 0 should contain created_at"
    );
    assert!(
        metadata.get("format_version").is_some(),
        "Layer 0 should contain format_version"
    );
    assert!(
        metadata.get("root_history").is_some(),
        "Layer 0 should contain root_history"
    );

    // Test 5: Add file and commit, then verify Layer 0 is still valid
    let test_file = project_path.join("test.txt");
    fs::write(&test_file, "test content")?;
    store.add_file(Path::new("test.txt"))?;
    store.commit("Test commit for Layer 0 validation")?;

    // Verify Layer 0 is still valid JSON after commit
    let metadata_bytes = store.archive.get_layer_data(&layer_zero_hash)?;
    let json_result = serde_json::from_slice::<serde_json::Value>(&metadata_bytes);
    assert!(
        json_result.is_ok(),
        "Layer 0 metadata should still be valid JSON after commit: {:?}",
        json_result.err()
    );

    Ok(())
}

/// Test that add_layer doesn't truncate the archive file incorrectly
#[test]
fn test_archive_add_layer_no_truncation_corruption() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Test 1: Initialize repository
    let mut store = Store::init(project_path)?;

    // Test 2: Add multiple layers and verify archive integrity
    for i in 0..3 {
        let test_file = project_path.join(format!("test_{}.txt", i));
        fs::write(&test_file, format!("test content {}", i))?;
        
        store.add_file(Path::new(&format!("test_{}.txt", i)))?;
        let commit_result = store.commit(&format!("Test commit {}", i));
        
        assert!(
            commit_result.is_ok(),
            "Commit {} should succeed without truncation corruption: {:?}",
            i,
            commit_result.err()
        );

        // Test 3: Verify Layer 0 is still valid after each commit
        let layer_zero_hash = digstore_min::core::types::Hash::zero();
        let metadata_bytes_result = store.archive.get_layer_data(&layer_zero_hash);
        assert!(
            metadata_bytes_result.is_ok(),
            "Layer 0 should still be readable after commit {}: {:?}",
            i,
            metadata_bytes_result.err()
        );

        let metadata_bytes = metadata_bytes_result.unwrap();
        assert!(
            !metadata_bytes.is_empty(),
            "Layer 0 should not be empty after commit {}",
            i
        );

        // Verify it's still valid JSON
        let json_result = serde_json::from_slice::<serde_json::Value>(&metadata_bytes);
        assert!(
            json_result.is_ok(),
            "Layer 0 should still be valid JSON after commit {}: Data length: {}, Error: {:?}",
            i,
            metadata_bytes.len(),
            json_result.err()
        );
    }

    Ok(())
}
