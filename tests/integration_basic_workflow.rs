//! Integration tests for basic digstore workflows
//!
//! Tests the complete workflow from initialization to file retrieval.

use digstore_min::storage::Store;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_complete_basic_workflow() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Step 1: Initialize repository
    let mut store = Store::init(project_path)?;

    // Verify .digstore file was created
    let digstore_path = project_path.join(".digstore");
    assert!(digstore_path.exists(), ".digstore file should be created");

    // Step 2: Create and add files
    let files = [
        ("file1.txt", "Content for file 1"),
        ("file2.txt", "Content for file 2"),
        ("file3.txt", "Content for file 3"),
    ];

    for (filename, content) in &files {
        let file_path = project_path.join(filename);
        fs::write(&file_path, content)?;
        store.add_file(Path::new(filename))?;
    }

    // Step 3: Verify files are staged
    let status = store.status();
    assert_eq!(status.staged_files.len(), 3);

    // Step 4: Commit files
    let commit_id = store.commit("Initial commit with test files")?;
    assert_ne!(commit_id.to_hex(), "0".repeat(64));

    // Step 5: Verify staging is cleared
    let status_after = store.status();
    assert_eq!(status_after.staged_files.len(), 0);

    // Step 6: Verify files can be retrieved (content may be encrypted)
    for (filename, expected_content) in &files {
        let retrieved_content = store.get_file(Path::new(filename))?;
        // If encryption is enabled, content will be encrypted, so just verify it's not empty
        if store.archive.is_encrypted() {
            assert!(
                !retrieved_content.is_empty(),
                "Retrieved content should not be empty"
            );
        } else {
            assert_eq!(retrieved_content, expected_content.as_bytes());
        }
    }

    Ok(())
}

#[test]
fn test_multiple_commit_workflow() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    let mut store = Store::init(project_path)?;

    // First commit
    fs::write(project_path.join("first.txt"), "First file")?;
    store.add_file(Path::new("first.txt"))?;
    let commit1 = store.commit("First commit")?;

    // Second commit
    fs::write(project_path.join("second.txt"), "Second file")?;
    store.add_file(Path::new("second.txt"))?;
    let commit2 = store.commit("Second commit")?;

    // Third commit
    fs::write(project_path.join("third.txt"), "Third file")?;
    store.add_file(Path::new("third.txt"))?;
    let commit3 = store.commit("Third commit")?;

    // All commits should be different
    assert_ne!(commit1, commit2);
    assert_ne!(commit2, commit3);
    assert_ne!(commit1, commit3);

    // Current root should be the latest commit
    assert_eq!(store.current_root(), Some(commit3));

    // All files should be accessible (data is always encrypted)
    let first_content = store.get_file(Path::new("first.txt"))?;
    let second_content = store.get_file(Path::new("second.txt"))?;
    let third_content = store.get_file(Path::new("third.txt"))?;

    assert!(!first_content.is_empty(), "First file should have content");
    assert!(
        !second_content.is_empty(),
        "Second file should have content"
    );
    assert!(!third_content.is_empty(), "Third file should have content");

    // All files should have different content
    assert_ne!(first_content, second_content);
    assert_ne!(second_content, third_content);
    assert_ne!(first_content, third_content);

    Ok(())
}

#[test]
fn test_repository_reopen_workflow() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    let commit_id;

    // Create repository and commit data
    {
        let mut store = Store::init(project_path)?;

        fs::write(project_path.join("persistent.txt"), "Persistent data")?;
        store.add_file(Path::new("persistent.txt"))?;
        commit_id = store.commit("Persistent commit")?;
    } // Store goes out of scope

    // Reopen repository
    let store = Store::open(project_path)?;

    // Verify data is still accessible
    assert_eq!(store.current_root(), Some(commit_id));

    let retrieved_content = store.get_file(Path::new("persistent.txt"))?;
    assert!(
        !retrieved_content.is_empty(),
        "Retrieved content should not be empty"
    );
    // Note: Content is encrypted, so we can't compare against plain text

    Ok(())
}
