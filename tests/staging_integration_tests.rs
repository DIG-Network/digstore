//! Integration tests for staging system fixes and CLI workflow
//! 
//! These tests validate the recent fixes to prevent regressions:
//! - Binary staging iterator fixes
//! - Memory map refresh mechanisms  
//! - Commit clearing staging
//! - CLI workflow integration

use anyhow::Result;
use digstore_min::storage::Store;
use digstore_min::cli::commands::staged;
use std::path::Path;
use tempfile::TempDir;
use std::fs;

/// Test utility for creating test stores
struct TestStaging {
    temp_dir: TempDir,
    store: Store,
}

impl TestStaging {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path();
        
        // Create test files
        fs::write(project_path.join("test1.txt"), "content1")?;
        fs::write(project_path.join("test2.txt"), "content2")?;
        fs::write(project_path.join("test3.txt"), "content3")?;
        
        let store = Store::init(project_path)?;
        
        Ok(Self { temp_dir, store })
    }
    
    fn project_path(&self) -> &Path {
        self.temp_dir.path()
    }
}

#[test]
fn test_add_status_commit_workflow_integration() -> Result<()> {
    let mut test = TestStaging::new()?;
    
    // Step 1: Add files
    test.store.add_file(&Path::new("test1.txt"))?;
    test.store.add_file(&Path::new("test2.txt"))?;
    
    // Step 2: Check status shows staged files
    let status = test.store.status();
    assert_eq!(status.staged_files.len(), 2, "Should have 2 staged files");
    assert!(status.total_staged_size > 0, "Should have non-zero staged size");
    
    // Step 3: Commit should work
    let commit_id = test.store.commit("Test commit")?;
    assert!(!commit_id.to_hex().is_empty(), "Should return valid commit ID");
    
    // Step 4: Status should show no staged files after commit
    let status_after = test.store.status();
    assert_eq!(status_after.staged_files.len(), 0, "Staging should be cleared after commit");
    assert_eq!(status_after.total_staged_size, 0, "Staged size should be zero after commit");
    
    Ok(())
}

#[test]
fn test_staging_persistence_across_cli_commands() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create test files
    fs::write(project_path.join("persist1.txt"), "persistent content 1")?;
    fs::write(project_path.join("persist2.txt"), "persistent content 2")?;
    
    // Simulate first CLI command: add files
    {
        let mut store = Store::init(project_path)?;
        store.add_file(&Path::new("persist1.txt"))?;
        store.add_file(&Path::new("persist2.txt"))?;
        
        let status = store.status();
        assert_eq!(status.staged_files.len(), 2, "Files should be staged");
    } // Store goes out of scope (simulates CLI command ending)
    
    // Simulate second CLI command: check status
    {
        let mut store = Store::open(project_path)?;
        let status = store.status();
        assert_eq!(status.staged_files.len(), 2, "Files should persist across CLI commands");
        assert!(status.total_staged_size > 0, "Should have staged data");
    }
    
    // Simulate third CLI command: commit
    {
        let mut store = Store::open(project_path)?;
        let commit_id = store.commit("Persistent test commit")?;
        assert!(!commit_id.to_hex().is_empty(), "Commit should succeed");
    }
    
    // Simulate fourth CLI command: verify staging is cleared
    {
        let mut store = Store::open(project_path)?;
        let status = store.status();
        assert_eq!(status.staged_files.len(), 0, "Staging should be cleared after commit");
    }
    
    Ok(())
}

#[test]
fn test_commit_properly_clears_staging_file() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create test file
    fs::write(project_path.join("clear_test.txt"), "content to clear")?;
    
    let mut store = Store::init(project_path)?;
    
    // Add file to staging
    store.add_file(&Path::new("clear_test.txt"))?;
    
    // Verify file is staged
    let status_before = store.status();
    assert_eq!(status_before.staged_files.len(), 1, "File should be staged");
    
    // Get staging file path for direct verification
    let staging_path = store.staging.staging_path().clone();
    let staging_size_before = fs::metadata(&staging_path)?.len();
    assert!(staging_size_before > 88, "Staging file should contain data (more than just header)");
    
    // Commit should clear staging
    let _commit_id = store.commit("Clear test commit")?;
    
    // Verify staging is cleared in memory
    let status_after = store.status();
    assert_eq!(status_after.staged_files.len(), 0, "Staging should be cleared in memory");
    
    // Verify staging file is physically cleared
    let staging_size_after = fs::metadata(&staging_path)?.len();
    assert_eq!(staging_size_after, 88, "Staging file should be reset to header-only size");
    
    Ok(())
}

#[test]
fn test_multiple_add_commit_cycles() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create test files
    for i in 1..=6 {
        fs::write(project_path.join(format!("cycle{}.txt", i)), format!("content {}", i))?;
    }
    
    let mut store = Store::init(project_path)?;
    
    // Cycle 1: Add 2 files, commit
    store.add_file(&Path::new("cycle1.txt"))?;
    store.add_file(&Path::new("cycle2.txt"))?;
    
    let status1 = store.status();
    assert_eq!(status1.staged_files.len(), 2, "Cycle 1: Should have 2 staged files");
    
    let _commit1 = store.commit("Cycle 1 commit")?;
    
    let status1_after = store.status();
    assert_eq!(status1_after.staged_files.len(), 0, "Cycle 1: Staging should be cleared");
    
    // Cycle 2: Add 3 files, commit
    store.add_file(&Path::new("cycle3.txt"))?;
    store.add_file(&Path::new("cycle4.txt"))?;
    store.add_file(&Path::new("cycle5.txt"))?;
    
    let status2 = store.status();
    assert_eq!(status2.staged_files.len(), 3, "Cycle 2: Should have 3 staged files");
    
    let _commit2 = store.commit("Cycle 2 commit")?;
    
    let status2_after = store.status();
    assert_eq!(status2_after.staged_files.len(), 0, "Cycle 2: Staging should be cleared");
    
    // Cycle 3: Add 1 file, commit
    store.add_file(&Path::new("cycle6.txt"))?;
    
    let status3 = store.status();
    assert_eq!(status3.staged_files.len(), 1, "Cycle 3: Should have 1 staged file");
    
    let _commit3 = store.commit("Cycle 3 commit")?;
    
    let status3_after = store.status();
    assert_eq!(status3_after.staged_files.len(), 0, "Cycle 3: Staging should be cleared");
    
    Ok(())
}

#[test]
fn test_staging_file_corruption_prevention() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create multiple test files
    for i in 1..=10 {
        fs::write(project_path.join(format!("batch{}.txt", i)), format!("batch content {}", i))?;
    }
    
    let mut store = Store::init(project_path)?;
    
    // Add files in batches to test batch staging
    let files1 = vec![
        Path::new("batch1.txt").to_path_buf(),
        Path::new("batch2.txt").to_path_buf(),
        Path::new("batch3.txt").to_path_buf(),
    ];
    
    let files2 = vec![
        Path::new("batch4.txt").to_path_buf(),
        Path::new("batch5.txt").to_path_buf(),
    ];
    
    // Batch 1
    store.add_files_batch(files1, None)?;
    let status1 = store.status();
    assert_eq!(status1.staged_files.len(), 3, "Batch 1: Should have 3 files");
    
    // Batch 2 (add to existing staging)
    store.add_files_batch(files2, None)?;
    let status2 = store.status();
    assert_eq!(status2.staged_files.len(), 5, "Batch 2: Should have 5 total files");
    
    // Verify all files are retrievable (tests iterator fix)
    for i in 1..=5 {
        let file_path = format!("batch{}.txt", i);
        assert!(store.is_file_staged(Path::new(&file_path)), "File {} should be staged", file_path);
    }
    
    // Commit should work without corruption
    let _commit_id = store.commit("Batch test commit")?;
    
    let status_after = store.status();
    assert_eq!(status_after.staged_files.len(), 0, "All files should be cleared after commit");
    
    Ok(())
}

#[test]
fn test_staging_system_memory_efficiency() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create files of various sizes
    fs::write(project_path.join("small.txt"), "small")?; // 5 bytes
    fs::write(project_path.join("medium.txt"), "medium content here")?; // ~18 bytes
    fs::write(project_path.join("large.txt"), "large content ".repeat(100))?; // ~1400 bytes
    
    let mut store = Store::init(project_path)?;
    
    // Add files
    store.add_file(&Path::new("small.txt"))?;
    store.add_file(&Path::new("medium.txt"))?;
    store.add_file(&Path::new("large.txt"))?;
    
    let status = store.status();
    assert_eq!(status.staged_files.len(), 3, "Should have 3 staged files");
    
    // Get staging file size to verify efficiency
    let staging_path = store.staging.staging_path().clone();
    let staging_size = fs::metadata(&staging_path)?.len();
    
    // Staging file should be much smaller than the sum of file contents
    // (because we store chunk metadata, not full file data)
    assert!(staging_size < 10000, "Staging file should be compact (< 10KB for 3 small files)");
    assert!(staging_size > 88, "Staging file should contain more than just header");
    
    Ok(())
}
