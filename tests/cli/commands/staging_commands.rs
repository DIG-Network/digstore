//! CLI tests for staging commands (staged list, diff, clear)
//!
//! Tests commands related to managing the staging area.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_staged_list_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test files
    for i in 1..=25 {
        fs::write(
            project_path.join(format!("staged{:02}.txt", i)),
            format!("Staged content {}", i),
        ).unwrap();
    }

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Staged Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    // Test staged list with default pagination
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("Staged Files"))
        .stdout(predicate::str::contains("Page 1"));

    // Test staged list with custom limit
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--limit", "10"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Showing 10 staged files"));

    // Test staged list with pagination
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--page", "2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Page 2"));

    // Test staged list all files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Showing 25 staged files"));
}

#[test]
fn test_staged_detailed_output() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create test file
    fs::write(project_path.join("detailed.txt"), "Detailed test content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Detailed Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "detailed.txt"])
        .assert()
        .success();

    // Test detailed output
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--detailed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hash"))
        .stdout(predicate::str::contains("Size"))
        .stdout(predicate::str::contains("Chunks"));
}

#[test]
fn test_staged_json_output() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("json_test.txt"), "JSON test").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "JSON Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "json_test.txt"])
        .assert()
        .success();

    // Test JSON output
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"staged_files\""));
}

#[test]
fn test_staged_diff_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create and commit initial file
    fs::write(project_path.join("diff_test.txt"), "Original content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Diff Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "diff_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Initial commit"])
        .assert()
        .success();

    // Modify file and stage
    fs::write(project_path.join("diff_test.txt"), "Modified content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "diff_test.txt"])
        .assert()
        .success();

    // Test staged diff
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stage Diff"));
}

#[test]
fn test_staged_clear_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup with staged files
    fs::write(project_path.join("clear_test.txt"), "Clear test").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Clear Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "clear_test.txt"])
        .assert()
        .success();

    // Verify file is staged
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("clear_test.txt"));

    // Test clear command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["staged", "clear", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleared"));

    // Staging should be empty
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("staged")
        .assert()
        .success()
        .stdout(predicate::str::contains("No files staged"));
}
