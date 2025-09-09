//! Regression tests for Windows file mapping error 1224
//!
//! These tests ensure that the Windows-specific file mapping issues don't reoccur.
//! The fixes involved adding retry logic and proper memory map cleanup.

use digstore_min::storage::Store;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Test for Windows file mapping error 1224 regression
/// This test ensures that memory-mapped files don't conflict with file operations
#[test]
fn test_windows_file_mapping_no_error_1224() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Test 1: Initialize repository (this used to fail with error 1224)
    let store = Store::init(project_path);
    assert!(
        store.is_ok(),
        "Store initialization should not fail with Windows file mapping error 1224: {:?}",
        store.err()
    );

    let mut store = store.unwrap();

    // Test 2: Add a file and commit (this used to fail with error 1224)
    let test_file = project_path.join("test_file.txt");
    fs::write(&test_file, "test content for Windows file mapping test")?;

    let add_result = store.add_file(Path::new("test_file.txt"));
    assert!(
        add_result.is_ok(),
        "Adding file should not fail with Windows file mapping error: {:?}",
        add_result.err()
    );

    let commit_result = store.commit("Test commit for Windows file mapping");
    assert!(
        commit_result.is_ok(),
        "Commit should not fail with Windows file mapping error: {:?}",
        commit_result.err()
    );

    // Test 3: Multiple commits (stress test the file mapping fixes)
    for i in 0..5 {
        let file_name = format!("test_file_{}.txt", i);
        let file_path = project_path.join(&file_name);
        fs::write(&file_path, format!("test content {}", i))?;

        let add_result = store.add_file(Path::new(&file_name));
        assert!(
            add_result.is_ok(),
            "Adding file {} should not fail with Windows file mapping error: {:?}",
            i,
            add_result.err()
        );

        let commit_result = store.commit(&format!("Test commit {}", i));
        assert!(
            commit_result.is_ok(),
            "Commit {} should not fail with Windows file mapping error: {:?}",
            i,
            commit_result.err()
        );
    }

    Ok(())
}

/// Test that the complete workflow works without any corruption
#[test]
fn test_complete_workflow_no_corruption() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Test 1: Initialize (Windows file mapping fix)
    let mut store = Store::init(project_path)?;

    // Test 2: Create multiple files and commits (JSON serialization fix)
    let files = ["file1.txt", "file2.txt", "file3.txt"];
    for (i, filename) in files.iter().enumerate() {
        let file_path = project_path.join(filename);
        fs::write(&file_path, format!("Content for file {}", i + 1))?;

        store.add_file(Path::new(filename))?;
        let commit_result = store.commit(&format!("Add {}", filename));

        assert!(
            commit_result.is_ok(),
            "Complete workflow commit should succeed: {:?}",
            commit_result.err()
        );
    }

    // Test 3: Verify repository state is consistent
    let layers = store.archive.list_layers();
    assert!(
        layers.len() >= 4, // Layer 0 + 3 commits
        "Should have correct number of layers"
    );

    // Test 4: Verify Layer 0 metadata integrity
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    let metadata_bytes = store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata_bytes)?;

    assert!(
        metadata.get("store_id").is_some(),
        "Layer 0 metadata should be intact"
    );

    // Test 5: Verify all files are accessible
    for filename in &files {
        let file_content = store.get_file(Path::new(filename));
        assert!(
            file_content.is_ok(),
            "File {} should be accessible in the store: {:?}",
            filename,
            file_content.err()
        );
    }

    Ok(())
}
