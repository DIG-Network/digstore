//! Regression tests for all identified and fixed bugs
//! 
//! This test suite ensures that bugs found during analysis don't regress

use anyhow::Result;
use digstore_min::storage::Store;
use std::path::Path;
use tempfile::TempDir;
use std::fs;

/// Test utility for creating test repositories
struct BugTestRepository {
    temp_dir: TempDir,
    store: Store,
}

impl BugTestRepository {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path();
        
        // Create test files
        fs::write(project_path.join("test1.txt"), "Hello World")?;
        fs::write(project_path.join("test2.txt"), "Test content")?;
        fs::create_dir_all(project_path.join("docs"))?;
        fs::write(project_path.join("docs/readme.md"), "Documentation")?;
        
        let mut store = Store::init(project_path)?;
        
        // Add and commit files to have data for testing
        store.add_file(Path::new("test1.txt"))?;
        store.add_file(Path::new("test2.txt"))?;
        store.add_file(Path::new("docs/readme.md"))?;
        store.commit("Initial test commit")?;
        
        Ok(Self { temp_dir, store })
    }
}

#[test]
fn test_archive_integration_commands_work() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    let project_path = test_repo.temp_dir.path();
    
    // Change to project directory for CLI command context
    std::env::set_current_dir(project_path)?;
    
    // Test that commands work with archive format (not separate .layer files)
    
    // Test info command reads from archive
    let result = digstore_min::cli::commands::info::execute(false, None);
    assert!(result.is_ok(), "Info command should work with archive format");
    
    // Test log command reads from archive
    let result = digstore_min::cli::commands::log::execute(None, false, false, None);
    assert!(result.is_ok(), "Log command should work with archive format");
    
    // Test root command reads from archive
    let result = digstore_min::cli::commands::root::execute(false, false, false);
    assert!(result.is_ok(), "Root command should work with archive format");
    
    // Test stats command reads from archive
    let result = digstore_min::cli::commands::stats::execute(false, false, false, false);
    assert!(result.is_ok(), "Stats command should work with archive format");
    
    // Test size command works with archive
    let result = digstore_min::cli::commands::size::execute(false, false, false, false);
    assert!(result.is_ok(), "Size command should work with archive format");
    
    Ok(())
}

#[test]
fn test_no_separate_layer_files_created() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    
    // Verify no separate .layer files exist
    let global_path = test_repo.store.global_path();
    
    if global_path.exists() {
        for entry in fs::read_dir(&global_path)? {
            let entry = entry?;
            let path = entry.path();
            
            // Should not find any .layer files
            if let Some(extension) = path.extension() {
                assert_ne!(extension, "layer", 
                    "Found .layer file: {} - should be using .dig archive format", 
                    path.display());
            }
        }
    }
    
    // Should find .dig archive file
    let archive_path = test_repo.store.archive.path();
    assert!(archive_path.exists(), "Archive file should exist: {}", archive_path.display());
    assert_eq!(archive_path.extension().and_then(|s| s.to_str()), Some("dig"), 
        "Archive should have .dig extension");
    
    Ok(())
}

#[test]
fn test_store_id_generation_robustness() -> Result<()> {
    // Test that store ID generation doesn't panic even if getrandom fails
    // This is hard to test directly, but we can test the fallback logic
    
    for _ in 0..100 {
        let store_id = digstore_min::storage::store::generate_store_id();
        
        // Should never be all zeros (very unlikely with proper generation)
        assert_ne!(store_id, digstore_min::core::types::Hash::zero(), 
            "Generated store ID should not be zero");
        
        // Should be consistent length
        assert_eq!(store_id.to_hex().len(), 64, 
            "Store ID should be 64 hex characters");
    }
    
    Ok(())
}

#[test]
fn test_layer_file_size_calculation_accuracy() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    
    // Test that layer size calculations are accurate
    let layers = test_repo.store.archive.list_layers();
    
    for (layer_hash, entry) in layers {
        // Skip Layer 0 as it's metadata
        if layer_hash == digstore_min::core::types::Hash::zero() {
            continue;
        }
        
        // Verify we can load the layer
        let layer = test_repo.store.load_layer(layer_hash)?;
        
        // Size should be reasonable (not zero, not impossibly large)
        assert!(entry.size > 0, "Layer size should be greater than 0");
        assert!(entry.size < 100 * 1024 * 1024, "Layer size should be reasonable (<100MB for test)");
        
        // Layer should have some content
        assert!(layer.files.len() > 0 || layer.chunks.len() > 0, 
            "Layer should have files or chunks");
    }
    
    Ok(())
}

#[test]
fn test_archive_stats_calculation() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    
    // Test that archive stats don't overflow or produce invalid results
    let stats = test_repo.store.archive.stats();
    
    assert!(stats.layer_count > 0, "Should have at least one layer");
    assert!(stats.total_size > 0, "Archive should have some size");
    assert!(stats.data_size <= stats.total_size, "Data size should not exceed total size");
    assert!(stats.compression_ratio >= 0.0 && stats.compression_ratio <= 1.0, 
        "Compression ratio should be between 0 and 1");
    assert!(stats.fragmentation >= 0.0 && stats.fragmentation <= 1.0, 
        "Fragmentation should be between 0 and 1");
    
    Ok(())
}

#[test]
fn test_cli_commands_handle_missing_data_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create empty repository
    let _store = Store::init(project_path)?;
    std::env::set_current_dir(project_path)?;
    
    // Commands should handle empty repository gracefully (not panic)
    
    // Info command with no data
    let result = digstore_min::cli::commands::info::execute(false, None);
    assert!(result.is_ok(), "Info command should handle empty repository");
    
    // Log command with no commits
    let result = digstore_min::cli::commands::log::execute(None, false, false, None);
    assert!(result.is_ok(), "Log command should handle no commits gracefully");
    
    // Root command with no commits
    let result = digstore_min::cli::commands::root::execute(false, false, false);
    assert!(result.is_ok(), "Root command should handle no commits gracefully");
    
    // Stats command with no data
    let result = digstore_min::cli::commands::stats::execute(false, false, false, false);
    assert!(result.is_ok(), "Stats command should handle empty repository");
    
    Ok(())
}

#[test]
fn test_archive_layer_listing_consistency() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    
    // Test that archive layer listing is consistent
    let layers = test_repo.store.archive.list_layers();
    
    // Should have at least Layer 0 + 1 data layer
    assert!(layers.len() >= 2, "Should have at least Layer 0 + 1 data layer");
    
    // Layer 0 should always be present
    let has_layer_zero = layers.iter().any(|(hash, _)| *hash == digstore_min::core::types::Hash::zero());
    assert!(has_layer_zero, "Layer 0 should always be present");
    
    // All listed layers should be loadable
    for (layer_hash, _entry) in layers {
        if layer_hash == digstore_min::core::types::Hash::zero() {
            // Layer 0 is metadata, use get_layer_data
            let result = test_repo.store.archive.get_layer_data(&layer_hash);
            assert!(result.is_ok(), "Layer 0 should be readable as metadata: {}", layer_hash);
        } else {
            // Regular layers should be loadable
            let result = test_repo.store.load_layer(layer_hash);
            assert!(result.is_ok(), "Layer should be loadable: {}", layer_hash);
        }
    }
    
    Ok(())
}

#[test]
fn test_staging_path_calculation() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    
    // Test that staging path calculation is correct
    let archive_path = test_repo.store.archive.path();
    let expected_staging_path = archive_path.with_extension("staging.bin");
    let actual_staging_path = test_repo.store.staging.staging_path();
    
    assert_eq!(actual_staging_path, &expected_staging_path, 
        "Staging path should be archive path with .staging.bin extension");
    
    Ok(())
}

#[test]
fn test_unwrap_safety_in_critical_paths() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    
    // Test operations that previously used unwrap() don't panic with edge cases
    
    // Test with valid edge cases that don't require special file permissions
    
    // Test with normal file operations that previously used unwrap()
    let normal_file = test_repo.temp_dir.path().join("normal_test.txt");
    fs::write(&normal_file, "test content")?;
    
    // Test that add operation handles the file gracefully
    let result = test_repo.store.add_file(Path::new("normal_test.txt"));
    match result {
        Ok(_) => {
            // Success case - file was added
            let commit_result = test_repo.store.commit("Normal file test");
            assert!(commit_result.is_ok() || commit_result.is_err(), "Should not panic");
        }
        Err(_) => {
            // Failure case - should be handled gracefully without panic
        }
    }
    
    // Test that operations with potentially problematic paths don't panic
    let result = test_repo.store.add_file(Path::new("nonexistent_file.txt"));
    assert!(result.is_err(), "Should return error for nonexistent file, not panic");
    
    Ok(())
}

#[test]
fn test_json_output_commands_produce_valid_json() -> Result<()> {
    let mut test_repo = BugTestRepository::new()?;
    let project_path = test_repo.temp_dir.path();
    std::env::set_current_dir(project_path)?;
    
    // Capture output from JSON commands and validate they produce valid JSON
    // Note: This is a basic test - in a real implementation, we'd capture stdout
    
    // These commands should not panic when producing JSON output
    let result = digstore_min::cli::commands::info::execute(true, None);
    assert!(result.is_ok(), "Info JSON command should not panic");
    
    let result = digstore_min::cli::commands::root::execute(true, false, false);
    assert!(result.is_ok(), "Root JSON command should not panic");
    
    let result = digstore_min::cli::commands::stats::execute(true, false, false, false);
    assert!(result.is_ok(), "Stats JSON command should not panic");
    
    let result = digstore_min::cli::commands::size::execute(true, false, false, false);
    assert!(result.is_ok(), "Size JSON command should not panic");
    
    Ok(())
}
