//! CLI tests for output formatting and display
//!
//! Tests that CLI output is consistent, well-formatted, and user-friendly.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_success_indicators_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("success.txt"), "Success test").unwrap();

    // All successful operations should show ✓ indicators
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Success Test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "success.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Success commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));
}

#[test]
fn test_error_message_quality() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test error messages are helpful and actionable

    // 1. No repository error
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("repository"))
        .stderr(predicate::str::contains("init"));

    // 2. Initialize for other tests
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Error Test"])
        .assert()
        .success();

    // 3. Empty commit error
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Empty"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No files staged"))
        .stderr(predicate::str::contains("add"));

    // 4. File not found error
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["add", "nonexistent.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_json_output_consistency() {
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

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "JSON commit"])
        .assert()
        .success();

    // Test JSON output for various commands
    let json_commands = [
        vec!["status", "--json"],
        vec!["staged", "--json"],
        vec!["root", "--json"],
    ];

    for command in &json_commands {
        let output = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(command)
            .assert()
            .success()
            .get_output();

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should be valid JSON
        assert!(
            stdout.starts_with('{') || stdout.starts_with('['),
            "Command {:?} should output valid JSON",
            command
        );
        assert!(
            stdout.ends_with('}') || stdout.ends_with(']'),
            "Command {:?} should output complete JSON",
            command
        );

        // Should be parseable as JSON
        let _: serde_json::Value = serde_json::from_str(&stdout)
            .expect(&format!("Command {:?} should output valid JSON", command));
    }
}

#[test]
fn test_command_output_formatting() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(project_path.join("format.txt"), "Format test").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Format Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "format.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Format commit"])
        .assert()
        .success();

    // Test that commands use consistent formatting
    let analysis_commands = [
        ("status", "Repository Status"),
        ("log", "Commit History"),
        ("root", "Current Root Information"),
    ];

    for (command, expected_header) in &analysis_commands {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .arg(command)
            .assert()
            .success()
            .stdout(predicate::str::contains(expected_header))
            .stdout(predicate::str::contains("═")); // Consistent separator
    }
}

#[test]
fn test_help_system_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test main help
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Digstore Min"))
        .stdout(predicate::str::contains("Commands:"));

    // Test help for major commands
    let commands = ["init", "add", "commit", "status", "get", "staged", "config"];

    for cmd in &commands {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&[cmd, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Usage:").or(predicate::str::contains("USAGE:")));
    }
}
