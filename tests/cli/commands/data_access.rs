//! CLI tests for data access commands (get, cat)
//!
//! Tests commands used to retrieve and display file content.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_get_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup repository with data
    fs::write(project_path.join("get_test.txt"), "Content to get").unwrap();
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Get Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "get_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Add test file"])
        .assert()
        .success();

    // Test get command (streams to stdout)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "get_test.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content to get"));

    // Test get command with output file
    let output_file = project_path.join("output.txt");
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "get_test.txt", "-o", output_file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    // Verify output file was created
    assert!(output_file.exists());
    let content = fs::read_to_string(&output_file).unwrap();
    assert_eq!(content, "Content to get");
}

#[test]
fn test_cat_command() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup repository with data
    fs::write(project_path.join("cat_test.txt"), "Content to display").unwrap();
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Cat Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "cat_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Add test file"])
        .assert()
        .success();

    // Test cat command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["cat", "cat_test.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content to display"));
}

#[test]
fn test_get_nonexistent_file() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Error Test"])
        .assert()
        .success();

    // Test get non-existent file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "nonexistent.txt"])
        .assert()
        .failure();
}

#[test]
fn test_zero_knowledge_urn_access() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test that invalid URNs return data (zero-knowledge property)
    let invalid_urns = [
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/fake1.txt",
        "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake2.txt",
        "malformed-urn-format",
    ];

    for urn in &invalid_urns {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn])
            .assert()
            .success(); // Should always return data, never errors
    }

    // Test deterministic behavior
    let test_urn = "urn:dig:chia:abcd1234567890abcdef1234567890abcdef1234567890abcdef1234567890/test.txt";

    let output1 = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", test_urn])
        .assert()
        .success()
        .get_output();

    let output2 = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", test_urn])
        .assert()
        .success()
        .get_output();

    assert_eq!(output1.stdout, output2.stdout, "Should be deterministic");
}

#[test]
fn test_get_with_byte_ranges() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test byte range access with invalid URNs (zero-knowledge property)
    let range_urns = [
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/fake.txt#bytes=0-9",
        "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake.txt#bytes=10-19",
        "urn:dig:chia:2222222222222222222222222222222222222222222222222222222222222222/fake.txt#bytes=20-",
    ];

    for urn in &range_urns {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn])
            .assert()
            .success(); // Should return appropriate amount of random data
    }
}
