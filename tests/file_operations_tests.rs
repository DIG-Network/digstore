//! File operations tests
#![allow(unused_imports, unused_variables, unused_mut, dead_code, clippy::all)]

use anyhow::Result;
use digstore_min::{
    core::{error::DigstoreError, hash::*, types::*},
    storage::store::Store,
};
use std::io::Write;
use std::path::Path;
use tempfile::{NamedTempFile, TempDir};

#[test]
fn test_add_single_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create a test file
    let test_file = temp_dir.path().join("test.txt");
    std::fs::write(&test_file, b"Hello, Digstore!")?;

    // Add file to staging
    store.add_file(Path::new("test.txt"))?;

    // Check that file is staged
    assert!(store.is_file_staged(Path::new("test.txt")));

    let status = store.status();
    assert_eq!(status.staged_files.len(), 1);
    assert_eq!(status.staged_files[0], Path::new("test.txt"));
    assert_eq!(status.total_staged_size, 16); // "Hello, Digstore!" length

    Ok(())
}

#[test]
fn test_add_multiple_files() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create test files
    std::fs::write(temp_dir.path().join("file1.txt"), b"Content 1")?;
    std::fs::write(temp_dir.path().join("file2.txt"), b"Content 2")?;
    std::fs::write(temp_dir.path().join("file3.txt"), b"Content 3")?;

    // Add files
    store.add_files(&["file1.txt", "file2.txt", "file3.txt"])?;

    let status = store.status();
    assert_eq!(status.staged_files.len(), 3);
    assert_eq!(status.total_staged_size, 27); // 9 * 3

    Ok(())
}

#[test]
fn test_add_nonexistent_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Try to add non-existent file
    let result = store.add_file(Path::new("nonexistent.txt"));
    assert!(result.is_err());

    // Should be a FileNotFound error
    match result.unwrap_err() {
        DigstoreError::FileNotFound { .. } => {}, // Expected
        e => panic!("Expected FileNotFound, got: {:?}", e),
    }

    Ok(())
}

#[test]
fn test_commit_staged_files() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and add test file
    std::fs::write(
        temp_dir.path().join("commit_test.txt"),
        b"Test commit content",
    )?;
    store.add_file(Path::new("commit_test.txt"))?;

    // Commit
    let commit_id = store.commit("Test commit message")?;

    // Verify commit was created
    assert_ne!(commit_id, Hash::zero());
    assert_eq!(store.current_root(), Some(commit_id));

    // Staging should be cleared
    assert!(store.staging.is_empty());

    // Layer file should exist (using .layer extension)
    let layer_path = store
        .global_path()
        .join(format!("{}.layer", commit_id.to_hex()));
    assert!(layer_path.exists());

    Ok(())
}

#[test]
fn test_commit_empty_staging() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Try to commit with no staged files
    let result = store.commit("Empty commit");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_get_staged_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    let test_content = b"Staged file content";
    std::fs::write(temp_dir.path().join("staged.txt"), test_content)?;
    store.add_file(Path::new("staged.txt"))?;

    // Get file from staging (before commit)
    let retrieved_content = store.get_file(Path::new("staged.txt"))?;
    assert_eq!(retrieved_content, test_content);

    Ok(())
}

#[test]
fn test_get_committed_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    let test_content = b"Committed file content";
    std::fs::write(temp_dir.path().join("committed.txt"), test_content)?;
    store.add_file(Path::new("committed.txt"))?;

    let commit_id = store.commit("Commit test file")?;

    // Get file from committed layer
    let retrieved_content = store.get_file(Path::new("committed.txt"))?;
    assert_eq!(retrieved_content, test_content);

    // Get file at specific commit
    let retrieved_at_commit = store.get_file_at(Path::new("committed.txt"), Some(commit_id))?;
    assert_eq!(retrieved_at_commit, test_content);

    Ok(())
}

#[test]
fn test_get_nonexistent_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let store = Store::init(temp_dir.path())?;

    let result = store.get_file(Path::new("nonexistent.txt"));
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_staging_operations() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create test files
    std::fs::write(temp_dir.path().join("file1.txt"), b"File 1")?;
    std::fs::write(temp_dir.path().join("file2.txt"), b"File 2")?;

    // Add files
    store.add_file(Path::new("file1.txt"))?;
    store.add_file(Path::new("file2.txt"))?;

    assert_eq!(store.staging.len(), 2);

    // Unstage one file
    store.unstage_file(Path::new("file1.txt"))?;
    assert_eq!(store.staging.len(), 1);
    assert!(!store.is_file_staged(Path::new("file1.txt")));
    assert!(store.is_file_staged(Path::new("file2.txt")));

    // Clear staging
    store.clear_staging();
    assert!(store.staging.is_empty());

    Ok(())
}

#[test]
fn test_add_directory_non_recursive() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create directory structure
    let subdir = temp_dir.path().join("subdir");
    std::fs::create_dir(&subdir)?;
    std::fs::write(temp_dir.path().join("root_file.txt"), b"Root file")?;
    std::fs::write(subdir.join("sub_file.txt"), b"Sub file")?;

    // Add directory non-recursively
    store.add_directory(temp_dir.path(), false)?;

    // Should only include root_file.txt, not sub_file.txt
    assert!(store.is_file_staged(Path::new("root_file.txt")));
    assert!(!store.is_file_staged(Path::new("subdir/sub_file.txt")));

    Ok(())
}

#[test]
fn test_add_directory_recursive() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create directory structure
    let subdir = temp_dir.path().join("subdir");
    std::fs::create_dir(&subdir)?;
    std::fs::write(temp_dir.path().join("root_file.txt"), b"Root file")?;
    std::fs::write(subdir.join("sub_file.txt"), b"Sub file")?;

    // Add directory recursively
    store.add_directory(temp_dir.path(), true)?;

    // Debug: print all staged files
    let status = store.status();
    println!("Staged files: {:?}", status.staged_files);

    // Should include both files (check by filename since paths may be absolute)
    let has_root_file = status
        .staged_files
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("root_file.txt"));
    let has_sub_file = status
        .staged_files
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("sub_file.txt"));
    let has_digstore = status
        .staged_files
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some(".layerstore"));

    assert!(has_root_file, "root_file.txt should be staged");
    assert!(has_sub_file, "sub_file.txt should be staged");

    // Should have at least 2 files (excluding .layerstore)
    let non_digstore_files = status
        .staged_files
        .iter()
        .filter(|p| !p.to_string_lossy().ends_with(".layerstore"))
        .count();
    assert_eq!(non_digstore_files, 2, "Should have 2 non-.layerstore files");

    Ok(())
}

#[test]
fn test_full_workflow() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create test files
    let file1_content = b"First file content";
    let file2_content = b"Second file content";

    std::fs::write(temp_dir.path().join("file1.txt"), file1_content)?;
    std::fs::write(temp_dir.path().join("file2.txt"), file2_content)?;

    // Add files
    store.add_file(Path::new("file1.txt"))?;
    store.add_file(Path::new("file2.txt"))?;

    // Verify staging
    assert_eq!(store.staging.len(), 2);

    // Commit
    let commit_id = store.commit("Initial commit")?;
    assert_ne!(commit_id, Hash::zero());

    // Verify staging is cleared
    assert!(store.staging.is_empty());

    // Verify files can be retrieved
    let retrieved1 = store.get_file(Path::new("file1.txt"))?;
    let retrieved2 = store.get_file(Path::new("file2.txt"))?;

    assert_eq!(retrieved1, file1_content);
    assert_eq!(retrieved2, file2_content);

    // Create second commit
    let file3_content = b"Third file content";
    std::fs::write(temp_dir.path().join("file3.txt"), file3_content)?;
    store.add_file(Path::new("file3.txt"))?;

    let commit_id2 = store.commit("Second commit")?;
    assert_ne!(commit_id2, commit_id);

    // Verify all files are accessible
    let retrieved3 = store.get_file(Path::new("file3.txt"))?;
    assert_eq!(retrieved3, file3_content);

    // Verify old files still accessible
    let retrieved1_again = store.get_file(Path::new("file1.txt"))?;
    assert_eq!(retrieved1_again, file1_content);

    Ok(())
}

#[test]
fn test_large_file_operations() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create a large file (2MB)
    let mut large_content = Vec::new();
    for i in 0..(2 * 1024 * 1024) {
        large_content.push((i % 256) as u8);
    }

    let large_file_path = temp_dir.path().join("large_file.bin");
    std::fs::write(&large_file_path, &large_content)?;

    // Add and commit large file
    store.add_file(Path::new("large_file.bin"))?;
    let commit_id = store.commit("Add large file")?;

    // Retrieve and verify
    let retrieved_content = store.get_file(Path::new("large_file.bin"))?;
    assert_eq!(retrieved_content, large_content);

    Ok(())
}

#[test]
fn test_file_overwrite_in_staging() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create initial file
    std::fs::write(temp_dir.path().join("test.txt"), b"Initial content")?;
    store.add_file(Path::new("test.txt"))?;

    // Modify and re-add file
    std::fs::write(temp_dir.path().join("test.txt"), b"Modified content")?;
    store.add_file(Path::new("test.txt"))?;

    // Should still have only one staged file
    assert_eq!(store.staging.len(), 1);

    // Content should be the latest
    let retrieved = store.get_file(Path::new("test.txt"))?;
    assert_eq!(retrieved, b"Modified content");

    Ok(())
}
