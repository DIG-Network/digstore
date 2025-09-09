//! CLI tests for core commands (init, add, commit, status)
//!
//! Tests the fundamental CLI commands that form the basic user workflow.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_init_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Test Repository"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository initialized"))
        .stdout(predicate::str::contains("Store ID:"))
        .stdout(predicate::str::contains("✓"));

    // Verify .digstore file was created
    assert!(project_path.join(".digstore").exists());
}

#[test]
fn test_add_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    fs::write(project_path.join("test1.txt"), "Test content 1").unwrap();
    fs::write(project_path.join("test2.txt"), "Test content 2").unwrap();

    // Initialize repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Add Test"])
        .assert()
        .success();

    // Test add single file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "test1.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"))
        .stdout(predicate::str::contains("✓"));

    // Test add multiple files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "test2.txt"])
        .assert()
        .success();
}

#[test]
fn test_status_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Initialize repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Status Test"])
        .assert()
        .success();

    // Test status with empty repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Status"))
        .stdout(predicate::str::contains("No changes staged"));

    // Add file and test status with staged files
    fs::write(project_path.join("status_test.txt"), "Status test").unwrap();
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "status_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("staged"));
}

#[test]
fn test_commit_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup
    fs::write(project_path.join("commit_test.txt"), "Commit test").unwrap();
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Commit Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "commit_test.txt"])
        .assert()
        .success();

    // Test commit
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Test commit message"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit created"))
        .stdout(predicate::str::contains("✓"));
}

#[test]
fn test_add_all_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create multiple files
    for i in 1..=5 {
        fs::write(
            project_path.join(format!("file{}.txt", i)),
            format!("Content {}", i),
        ).unwrap();
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Add All Test"])
        .assert()
        .success();

    // Test add -A (add all files)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files added to staging"));
}

#[test]
fn test_error_conditions() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test commands without repository
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"));

    // Initialize for other error tests
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Error Test"])
        .assert()
        .success();

    // Test commit without staged files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty commit"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"));

    // Test add non-existent file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "nonexistent.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}
