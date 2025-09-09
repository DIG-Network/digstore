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

/// Test that archive operations don't cause Windows file mapping conflicts
#[test]
fn test_archive_operations_no_file_mapping_conflict() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    let mut store = Store::init(project_path)?;

    // Test multiple archive operations that could trigger file mapping issues
    for i in 0..3 {
        let test_file = project_path.join(format!("archive_test_{}.txt", i));
        fs::write(&test_file, format!("archive test content {}", i))?;
        
        store.add_file(Path::new(&format!("archive_test_{}.txt", i)))?;
        let commit_result = store.commit(&format!("Archive test commit {}", i));
        
        assert!(
            commit_result.is_ok(),
            "Archive commit {} should succeed without file mapping corruption: {:?}",
            i,
            commit_result.err()
        );

        // Verify Layer 0 is still valid after each commit
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

/// Test that staging operations don't cause Windows file mapping conflicts
#[test]
fn test_staging_operations_no_file_mapping_conflict() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    let mut store = Store::init(project_path)?;

    // Test rapid staging and clearing operations
    for cycle in 0..3 {
        // Add multiple files
        for i in 0..5 {
            let file_name = format!("staging_test_{}_{}.txt", cycle, i);
            let file_path = project_path.join(&file_name);
            fs::write(&file_path, format!("staging test content {} {}", cycle, i))?;
            
            let add_result = store.add_file(Path::new(&file_name));
            assert!(
                add_result.is_ok(),
                "Staging add should not fail with file mapping error: {:?}",
                add_result.err()
            );
        }

        // Verify staging works
        let status = store.status();
        assert_eq!(status.staged_files.len(), 5);

        // Commit (which clears staging)
        let commit_result = store.commit(&format!("Staging cycle {}", cycle));
        assert!(
            commit_result.is_ok(),
            "Staging commit should not fail with file mapping error: {:?}",
            commit_result.err()
        );

        // Verify staging is cleared
        let status_after = store.status();
        assert_eq!(status_after.staged_files.len(), 0);
    }

    Ok(())
}
