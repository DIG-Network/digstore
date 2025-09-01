//! Regression tests for all recently fixed commands
//! 
//! These tests ensure that the fixes for info, log, verify, and inspect commands
//! don't regress in the future.

use anyhow::Result;
use digstore_min::storage::Store;
use std::path::Path;
use tempfile::TempDir;
use std::fs;

/// Test utility for creating test repositories with committed data
struct FixedCommandTester {
    temp_dir: TempDir,
    store: Store,
    commit_hash: String,
}

impl FixedCommandTester {
    fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path();
        
        // Create test files
        fs::write(project_path.join("test1.txt"), "Hello World")?;
        fs::write(project_path.join("test2.txt"), "Test content")?;
        fs::create_dir_all(project_path.join("docs"))?;
        fs::write(project_path.join("docs/readme.md"), "Documentation")?;
        
        // Initialize and commit data
        let mut store = Store::init(project_path)?;
        store.add_file(Path::new("test1.txt"))?;
        store.add_file(Path::new("test2.txt"))?;
        store.add_file(Path::new("docs/readme.md"))?;
        
        let commit_id = store.commit("Test commit for fixed commands")?;
        let commit_hash = commit_id.to_hex();
        
        Ok(Self {
            temp_dir,
            store,
            commit_hash,
        })
    }
}

#[test]
fn test_info_command_archive_integration() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Test that info command can read from archive format
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    assert!(tester.store.archive.has_layer(&layer_zero_hash));
    
    // Test that archive contains the expected layer data
    let layer_data = tester.store.archive.get_layer_data(&layer_zero_hash)?;
    assert!(!layer_data.is_empty());
    
    // Parse metadata to ensure it's valid JSON
    let metadata: serde_json::Value = serde_json::from_slice(&layer_data)?;
    assert!(metadata.get("store_id").is_some());
    assert!(metadata.get("root_history").is_some());
    
    Ok(())
}

#[test]
fn test_log_command_root_history_parsing() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Test that log command can find commits in archive
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    let layer_data = tester.store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&layer_data)?;
    
    // Verify root history exists and is properly formatted
    let root_history = metadata.get("root_history")
        .and_then(|v| v.as_array())
        .expect("Root history should exist");
    
    assert!(!root_history.is_empty(), "Should have at least one commit");
    
    // Test that the latest root entry has the expected structure
    let latest_entry = root_history.last().expect("Should have latest entry");
    assert!(latest_entry.get("root_hash").is_some());
    assert!(latest_entry.get("generation").is_some());
    assert!(latest_entry.get("timestamp").is_some());
    
    Ok(())
}

#[test]
fn test_verify_command_proof_validation() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Generate a proof for a committed file
    let proof = digstore_min::proofs::proof::Proof::new_file_proof(
        &tester.store,
        Path::new("test1.txt"),
        None
    )?;
    
    // Test that proof has expected structure
    assert!(matches!(proof.target, digstore_min::proofs::proof::ProofTarget::File { .. }));
    assert!(!proof.proof_path.is_empty());
    assert!(proof.version == "1.0");
    
    // Test that verification works
    let is_valid = proof.verify()?;
    assert!(is_valid, "Generated proof should be valid");
    
    Ok(())
}

#[test]
fn test_inspect_command_layer_access() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Parse commit hash
    let commit_hash = digstore_min::core::types::Hash::from_hex(&tester.commit_hash)?;
    
    // Test that inspect can load the layer from archive
    let layer = tester.store.load_layer(commit_hash)?;
    
    // Verify layer structure
    assert_eq!(layer.header.layer_type, 1); // Full layer
    assert!(!layer.files.is_empty());
    assert!(!layer.chunks.is_empty());
    
    // Test that layer contains expected files
    let file_paths: Vec<_> = layer.files.iter().map(|f| &f.path).collect();
    assert!(file_paths.iter().any(|p| p.to_string_lossy().contains("test1.txt")));
    assert!(file_paths.iter().any(|p| p.to_string_lossy().contains("test2.txt")));
    
    Ok(())
}

#[test]
fn test_archive_layer_persistence() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Test that layers are properly stored in archive
    let layers = tester.store.archive.list_layers();
    assert!(layers.len() >= 2); // At least Layer 0 and one commit layer
    
    // Test that Layer 0 exists
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    assert!(layers.iter().any(|(hash, _)| *hash == layer_zero_hash));
    
    // Test that commit layer exists
    let commit_hash = digstore_min::core::types::Hash::from_hex(&tester.commit_hash)?;
    assert!(layers.iter().any(|(hash, _)| *hash == commit_hash));
    
    Ok(())
}

#[test]
fn test_root_tracking_consistency() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Test that current root matches what's in Layer 0
    let current_root = tester.store.current_root.expect("Should have current root");
    
    // Load Layer 0 and check root history
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    let layer_data = tester.store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&layer_data)?;
    
    let root_history = metadata.get("root_history")
        .and_then(|v| v.as_array())
        .expect("Root history should exist");
    
    let latest_root_str = root_history.last()
        .and_then(|entry| entry.get("root_hash"))
        .and_then(|v| v.as_str())
        .expect("Latest root should exist");
    
    assert_eq!(current_root.to_hex(), latest_root_str);
    
    Ok(())
}

#[test]
fn test_multi_commit_workflow() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();
    
    // Create initial files
    fs::write(project_path.join("file1.txt"), "Initial content")?;
    fs::write(project_path.join("file2.txt"), "More content")?;
    
    let mut store = Store::init(project_path)?;
    
    // First commit
    store.add_file(Path::new("file1.txt"))?;
    let commit1 = store.commit("First commit")?;
    
    // Second commit
    store.add_file(Path::new("file2.txt"))?;
    let commit2 = store.commit("Second commit")?;
    
    // Test that both commits are tracked
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    let layer_data = store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&layer_data)?;
    
    let root_history = metadata.get("root_history")
        .and_then(|v| v.as_array())
        .expect("Root history should exist");
    
    assert_eq!(root_history.len(), 2, "Should have 2 commits");
    
    // Test that current root is the latest commit
    assert_eq!(store.current_root.unwrap(), commit2);
    
    // Test that both layers exist in archive
    assert!(store.archive.has_layer(&commit1));
    assert!(store.archive.has_layer(&commit2));
    
    Ok(())
}

#[test]
fn test_command_integration_after_fixes() -> Result<()> {
    let tester = FixedCommandTester::new()?;
    
    // Test that all fixed commands can access the same data consistently
    
    // 1. info command should show correct layer count
    let layers = tester.store.archive.list_layers();
    assert!(layers.len() >= 2);
    
    // 2. log command should find the commit
    let layer_zero_hash = digstore_min::core::types::Hash::zero();
    let layer_data = tester.store.archive.get_layer_data(&layer_zero_hash)?;
    let metadata: serde_json::Value = serde_json::from_slice(&layer_data)?;
    let root_history = metadata.get("root_history").and_then(|v| v.as_array());
    assert!(root_history.is_some() && !root_history.unwrap().is_empty());
    
    // 3. inspect command should be able to load the layer
    let commit_hash = digstore_min::core::types::Hash::from_hex(&tester.commit_hash)?;
    let layer = tester.store.load_layer(commit_hash)?;
    assert!(!layer.files.is_empty());
    
    // 4. Data access should work
    let file_data = tester.store.get_file(Path::new("test1.txt"))?;
    assert_eq!(String::from_utf8_lossy(&file_data), "Hello World");
    
    Ok(())
}
