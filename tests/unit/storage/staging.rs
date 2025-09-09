//! Unit tests for binary staging area
//!
//! Tests the staging system used for preparing files before commits.

use digstore_min::storage::{
    binary_staging::{BinaryStagingArea, BinaryStagedFile},
    Store,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_staging_initialization() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let staging_path = temp_dir.path().join("test.staging.bin");
    
    let mut staging = BinaryStagingArea::new(staging_path.clone());
    staging.initialize()?;
    
    // Staging file should exist
    assert!(staging_path.exists());
    
    // Should be empty initially
    assert!(staging.is_empty());
    assert_eq!(staging.len(), 0);
    
    Ok(())
}

#[test]
fn test_staging_add_file() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();
    let staging_path = temp_dir.path().join("test.staging.bin");
    
    // Create test file
    let test_content = b"Test file content";
    fs::write(project_path.join("test.txt"), test_content)?;
    
    let mut staging = BinaryStagingArea::new(staging_path);
    staging.initialize()?;
    
    // Create staged file entry
    let staged_file = BinaryStagedFile {
        path: Path::new("test.txt").to_path_buf(),
        hash: digstore_min::core::hash::sha256(test_content),
        size: test_content.len() as u64,
        chunks: vec![],
        modified_time: None,
    };
    
    staging.stage_file_streaming(staged_file)?;
    
    // Should no longer be empty
    assert!(!staging.is_empty());
    assert_eq!(staging.len(), 1);
    
    // Should be able to check if file is staged
    assert!(staging.is_staged(Path::new("test.txt")));
    
    Ok(())
}

#[test]
fn test_staging_get_all_files() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();
    let staging_path = temp_dir.path().join("test.staging.bin");
    
    // Create test files
    let files = [
        ("file1.txt", b"Content 1"),
        ("file2.txt", b"Content 2"),
        ("file3.txt", b"Content 3"),
    ];
    
    for (name, content) in &files {
        fs::write(project_path.join(name), content)?;
    }
    
    let mut staging = BinaryStagingArea::new(staging_path);
    staging.initialize()?;
    
    // Add files to staging
    for (name, content) in &files {
        let staged_file = BinaryStagedFile {
            path: Path::new(name).to_path_buf(),
            hash: digstore_min::core::hash::sha256(content),
            size: content.len() as u64,
            chunks: vec![],
            modified_time: None,
        };
        staging.stage_file_streaming(staged_file)?;
    }
    
    // Should be able to get all staged files
    let all_staged = staging.get_all_staged_files()?;
    assert_eq!(all_staged.len(), 3);
    
    // Check that all files are present
    let staged_names: Vec<_> = all_staged
        .iter()
        .map(|f| f.path.file_name().unwrap().to_str().unwrap())
        .collect();
    
    for (name, _) in &files {
        assert!(staged_names.contains(name));
    }
    
    Ok(())
}

#[test]
fn test_staging_clear() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();
    let staging_path = temp_dir.path().join("test.staging.bin");
    
    // Create and stage a file
    fs::write(project_path.join("clear_test.txt"), b"Test content")?;
    
    let mut staging = BinaryStagingArea::new(staging_path.clone());
    staging.initialize()?;
    
    let staged_file = BinaryStagedFile {
        path: Path::new("clear_test.txt").to_path_buf(),
        hash: digstore_min::core::hash::sha256(b"Test content"),
        size: 12,
        chunks: vec![],
        modified_time: None,
    };
    staging.stage_file_streaming(staged_file)?;
    
    // Verify file is staged
    assert!(!staging.is_empty());
    
    // Clear staging
    staging.clear()?;
    
    // Should be empty
    assert!(staging.is_empty());
    assert_eq!(staging.len(), 0);
    
    // File should be reset to header-only size
    let file_size = fs::metadata(&staging_path)?.len();
    assert_eq!(file_size, 88); // Header size
    
    Ok(())
}

#[test]
fn test_staging_persistence() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();
    let staging_path = temp_dir.path().join("persistent.staging.bin");
    
    // Create test file
    fs::write(project_path.join("persist.txt"), b"Persistent content")?;
    
    // Add file to staging and close
    {
        let mut staging = BinaryStagingArea::new(staging_path.clone());
        staging.initialize()?;
        
        let staged_file = BinaryStagedFile {
            path: Path::new("persist.txt").to_path_buf(),
            hash: digstore_min::core::hash::sha256(b"Persistent content"),
            size: 18,
            chunks: vec![],
            modified_time: None,
        };
        staging.stage_file_streaming(staged_file)?;
        
        assert!(!staging.is_empty());
    } // staging goes out of scope
    
    // Reopen staging
    let mut new_staging = BinaryStagingArea::new(staging_path);
    new_staging.load()?;
    
    // Should still have the staged file
    assert!(!new_staging.is_empty());
    assert_eq!(new_staging.len(), 1);
    assert!(new_staging.is_staged(Path::new("persist.txt")));
    
    Ok(())
}
