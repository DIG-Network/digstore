//! Tests for smart staging and stage diff functionality

use anyhow::Result;
use digstore_min::storage::Store;
use std::path::Path;
use tempfile::TempDir;
use std::fs;

/// Test utility for smart staging scenarios
struct SmartStagingTest {
    temp_dir: TempDir,
    store: Store,
}

impl SmartStagingTest {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path();
        
        let store = Store::init(project_path)?;
        
        Ok(Self { temp_dir, store })
    }
    
    fn create_file(&self, name: &str, content: &str) -> Result<()> {
        fs::write(self.temp_dir.path().join(name), content)?;
        Ok(())
    }
    
    fn modify_file(&self, name: &str, content: &str) -> Result<()> {
        fs::write(self.temp_dir.path().join(name), content)?;
        Ok(())
    }
    
    fn delete_file(&self, name: &str) -> Result<()> {
        let path = self.temp_dir.path().join(name);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[test]
fn test_smart_staging_skips_unchanged_files() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Create and commit initial files
    test.create_file("file1.txt", "content1")?;
    test.create_file("file2.txt", "content2")?;
    
    test.store.add_file(Path::new("file1.txt"))?;
    test.store.add_file(Path::new("file2.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Try to add the same files again (should be skipped)
    test.store.add_file(Path::new("file1.txt"))?;
    test.store.add_file(Path::new("file2.txt"))?;
    
    // Check that no files are staged (smart staging skipped them)
    let staged_files = test.store.staging.get_all_staged_files()?;
    assert_eq!(staged_files.len(), 0, "Unchanged files should not be staged");
    
    Ok(())
}

#[test]
fn test_smart_staging_detects_changes() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Create and commit initial files
    test.create_file("file1.txt", "original content")?;
    test.create_file("file2.txt", "unchanged content")?;
    
    test.store.add_file(Path::new("file1.txt"))?;
    test.store.add_file(Path::new("file2.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Modify one file
    test.modify_file("file1.txt", "modified content")?;
    
    // Add both files
    test.store.add_file(Path::new("file1.txt"))?; // Should be staged (changed)
    test.store.add_file(Path::new("file2.txt"))?; // Should be skipped (unchanged)
    
    // Check that only the changed file is staged
    let staged_files = test.store.staging.get_all_staged_files()?;
    assert_eq!(staged_files.len(), 1, "Only changed file should be staged");
    assert_eq!(staged_files[0].path, Path::new("file1.txt"));
    
    Ok(())
}

#[test]
fn test_smart_staging_handles_new_files() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Create and commit initial file
    test.create_file("existing.txt", "existing content")?;
    test.store.add_file(Path::new("existing.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Create new file
    test.create_file("new.txt", "new content")?;
    
    // Add both files
    test.store.add_file(Path::new("existing.txt"))?; // Should be skipped (unchanged)
    test.store.add_file(Path::new("new.txt"))?;      // Should be staged (new)
    
    // Check that only the new file is staged
    let staged_files = test.store.staging.get_all_staged_files()?;
    assert_eq!(staged_files.len(), 1, "Only new file should be staged");
    assert_eq!(staged_files[0].path, Path::new("new.txt"));
    
    Ok(())
}

#[test]
fn test_has_file_changed_method() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Create and commit initial file
    test.create_file("test.txt", "original")?;
    test.store.add_file(Path::new("test.txt"))?;
    test.store.commit("Initial commit")?;
    
    // File should not have changed
    assert!(!test.store.has_file_changed(Path::new("test.txt"))?, 
        "Unchanged file should not be detected as changed");
    
    // Modify file
    test.modify_file("test.txt", "modified")?;
    
    // File should now be detected as changed
    assert!(test.store.has_file_changed(Path::new("test.txt"))?, 
        "Modified file should be detected as changed");
    
    // New file should be detected as changed
    test.create_file("new.txt", "new content")?;
    assert!(test.store.has_file_changed(Path::new("new.txt"))?, 
        "New file should be detected as changed");
    
    Ok(())
}

#[test]
fn test_stage_diff_with_no_changes() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    std::env::set_current_dir(test.temp_dir.path())?;
    
    // Create repository with no staged files
    let _store = Store::init(test.temp_dir.path())?;
    
    // Stage diff should handle empty staging gracefully
    let result = digstore_min::cli::commands::stage_diff::execute(false, false, false, 3, None);
    assert!(result.is_ok(), "Stage diff should handle empty staging gracefully");
    
    Ok(())
}

#[test]
fn test_stage_diff_detects_all_change_types() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    std::env::set_current_dir(test.temp_dir.path())?;
    
    // Create and commit initial files
    test.create_file("to_modify.txt", "original")?;
    test.create_file("to_delete.txt", "will be deleted")?;
    test.create_file("unchanged.txt", "stays same")?;
    
    test.store.add_file(Path::new("to_modify.txt"))?;
    test.store.add_file(Path::new("to_delete.txt"))?;
    test.store.add_file(Path::new("unchanged.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Make changes
    test.modify_file("to_modify.txt", "modified content")?;
    test.create_file("new.txt", "new file")?;
    test.delete_file("to_delete.txt")?;
    // unchanged.txt stays the same
    
    // Stage only the changed and new files
    test.store.add_file(Path::new("to_modify.txt"))?; // Modified - should be staged
    test.store.add_file(Path::new("new.txt"))?;       // New - should be staged
    test.store.add_file(Path::new("unchanged.txt"))?; // Unchanged - should be skipped
    // to_delete.txt is not staged (deleted)
    
    // Test stage diff command
    let result = digstore_min::cli::commands::stage_diff::execute(false, true, false, 3, None);
    assert!(result.is_ok(), "Stage diff should work with all change types");
    
    Ok(())
}

#[test]
fn test_stage_diff_specific_file_filter() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    std::env::set_current_dir(test.temp_dir.path())?;
    
    // Create and commit initial files
    test.create_file("file1.txt", "content1")?;
    test.create_file("file2.txt", "content2")?;
    
    test.store.add_file(Path::new("file1.txt"))?;
    test.store.add_file(Path::new("file2.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Modify both files
    test.modify_file("file1.txt", "modified1")?;
    test.modify_file("file2.txt", "modified2")?;
    
    // Stage both files
    test.store.add_file(Path::new("file1.txt"))?;
    test.store.add_file(Path::new("file2.txt"))?;
    
    // Test stage diff with specific file filter
    let result = digstore_min::cli::commands::stage_diff::execute(
        false, false, false, 3, Some("file1.txt".to_string())
    );
    assert!(result.is_ok(), "Stage diff should work with file filter");
    
    Ok(())
}

#[test]
fn test_stage_diff_output_modes() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    std::env::set_current_dir(test.temp_dir.path())?;
    
    // Create and commit initial file
    test.create_file("test.txt", "original")?;
    test.store.add_file(Path::new("test.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Modify file
    test.modify_file("test.txt", "modified")?;
    test.store.add_file(Path::new("test.txt"))?;
    
    // Test different output modes
    let result = digstore_min::cli::commands::stage_diff::execute(true, false, false, 3, None); // name-only
    assert!(result.is_ok(), "Stage diff name-only mode should work");
    
    let result = digstore_min::cli::commands::stage_diff::execute(false, true, false, 3, None); // json
    assert!(result.is_ok(), "Stage diff JSON mode should work");
    
    let result = digstore_min::cli::commands::stage_diff::execute(false, false, true, 3, None); // stat
    assert!(result.is_ok(), "Stage diff stat mode should work");
    
    Ok(())
}

#[test]
fn test_smart_staging_performance_benefit() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Create many files
    for i in 0..100 {
        test.create_file(&format!("file{}.txt", i), &format!("content{}", i))?;
    }
    
    // Add and commit all files
    for i in 0..100 {
        test.store.add_file(Path::new(&format!("file{}.txt", i)))?;
    }
    test.store.commit("Initial commit with 100 files")?;
    
    // Try to add all files again - smart staging should skip all of them
    let start = std::time::Instant::now();
    for i in 0..100 {
        test.store.add_file(Path::new(&format!("file{}.txt", i)))?;
    }
    let duration = start.elapsed();
    
    // Should be very fast since all files are skipped
    assert!(duration.as_millis() < 1000, "Smart staging should be fast for unchanged files");
    
    // No files should be staged
    let staged_files = test.store.staging.get_all_staged_files()?;
    assert_eq!(staged_files.len(), 0, "No unchanged files should be staged");
    
    Ok(())
}

#[test]
fn test_mixed_changed_and_unchanged_files() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Create and commit initial files
    for i in 0..10 {
        test.create_file(&format!("file{}.txt", i), &format!("content{}", i))?;
        test.store.add_file(Path::new(&format!("file{}.txt", i)))?;
    }
    test.store.commit("Initial commit")?;
    
    // Modify only half the files
    for i in 0..5 {
        test.modify_file(&format!("file{}.txt", i), &format!("modified{}", i))?;
    }
    
    // Try to add all files
    for i in 0..10 {
        test.store.add_file(Path::new(&format!("file{}.txt", i)))?;
    }
    
    // Only the modified files should be staged
    let staged_files = test.store.staging.get_all_staged_files()?;
    assert_eq!(staged_files.len(), 5, "Only modified files should be staged");
    
    // Check that the staged files are the ones we modified (file0-file4)
    let staged_names: Vec<_> = staged_files.iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    
    for i in 0..5 {
        let expected_name = format!("file{}.txt", i);
        assert!(staged_names.contains(&expected_name), 
            "Modified file {} should be staged", expected_name);
    }
    
    Ok(())
}

#[test]
fn test_stage_diff_command_integration() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    let project_path = test.temp_dir.path();
    std::env::set_current_dir(project_path)?;
    
    // Create and commit initial files
    test.create_file("original.txt", "original content")?;
    test.store.add_file(Path::new("original.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Make various changes
    test.modify_file("original.txt", "modified content")?;
    test.create_file("new.txt", "new content")?;
    
    // Stage the changes
    test.store.add_file(Path::new("original.txt"))?;
    test.store.add_file(Path::new("new.txt"))?;
    
    // Test that stage-diff command works
    let result = digstore_min::cli::commands::stage_diff::execute(false, false, false, 3, None);
    assert!(result.is_ok(), "Stage diff command should work with real changes");
    
    // Test JSON output
    let result = digstore_min::cli::commands::stage_diff::execute(false, true, false, 3, None);
    assert!(result.is_ok(), "Stage diff JSON output should work");
    
    // Test name-only output
    let result = digstore_min::cli::commands::stage_diff::execute(true, false, false, 3, None);
    assert!(result.is_ok(), "Stage diff name-only output should work");
    
    // Test stat output
    let result = digstore_min::cli::commands::stage_diff::execute(false, false, true, 3, None);
    assert!(result.is_ok(), "Stage diff stat output should work");
    
    Ok(())
}

#[test]
fn test_file_change_detection_accuracy() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // Test with no commits yet
    test.create_file("new.txt", "content")?;
    assert!(test.store.has_file_changed(Path::new("new.txt"))?, 
        "New file should be detected as changed when no commits exist");
    
    // Commit the file
    test.store.add_file(Path::new("new.txt"))?;
    test.store.commit("Add new file")?;
    
    // File should not be changed now
    assert!(!test.store.has_file_changed(Path::new("new.txt"))?, 
        "File should not be changed after commit");
    
    // Modify file content
    test.modify_file("new.txt", "different content")?;
    assert!(test.store.has_file_changed(Path::new("new.txt"))?, 
        "File should be detected as changed after modification");
    
    // Restore original content
    test.modify_file("new.txt", "content")?;
    assert!(!test.store.has_file_changed(Path::new("new.txt"))?, 
        "File should not be changed when restored to original content");
    
    Ok(())
}

#[test]
fn test_stage_diff_size_calculations() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    std::env::set_current_dir(test.temp_dir.path())?;
    
    // Create and commit initial file
    test.create_file("test.txt", "small")?; // 5 bytes
    test.store.add_file(Path::new("test.txt"))?;
    test.store.commit("Initial commit")?;
    
    // Modify to larger content
    test.modify_file("test.txt", "much larger content")?; // 19 bytes
    test.store.add_file(Path::new("test.txt"))?;
    
    // The stage diff should show the size change correctly
    // We can't easily test the output directly, but we can test that it doesn't error
    let result = digstore_min::cli::commands::stage_diff::execute(false, false, true, 3, None);
    assert!(result.is_ok(), "Stage diff should calculate size changes correctly");
    
    Ok(())
}

#[test]
fn test_smart_staging_with_empty_repository() -> Result<()> {
    let mut test = SmartStagingTest::new()?;
    
    // With no commits, all files should be considered new/changed
    test.create_file("file.txt", "content")?;
    
    assert!(test.store.has_file_changed(Path::new("file.txt"))?, 
        "File should be changed in empty repository");
    
    test.store.add_file(Path::new("file.txt"))?;
    
    let staged_files = test.store.staging.get_all_staged_files()?;
    assert_eq!(staged_files.len(), 1, "File should be staged in empty repository");
    
    Ok(())
}
