//! Integration tests for version management workflows
//!
//! Tests the complete version management workflow including updates,
//! PATH management, and version switching.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_version_command_help() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test version command shows help
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("Version Information"))
        .stdout(predicate::str::contains("Available Commands"))
        .stdout(predicate::str::contains("install-current"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("fix-path"));
}

#[test]
fn test_version_list_command() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test version list command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "list"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Installed Versions")
                .or(predicate::str::contains("No versions installed"))
        );
}

#[test]
fn test_version_fix_path_analysis() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test PATH analysis command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "fix-path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyzing PATH"))
        .stdout(
            predicate::str::contains("Found digstore installations")
                .or(predicate::str::contains("No digstore installations"))
        );
}

#[test]
fn test_version_current_command() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test current version command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "current"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Current version")
                .or(predicate::str::contains("No versions managed"))
        );
}

#[test]
fn test_version_install_current_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();
    
    // Change to a project directory (version install-current expects to be in project)
    std::env::set_current_dir(std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().parent().unwrap()).unwrap();
    
    // Test install-current command (might fail due to permissions)
    let command_assert = Command::cargo_bin("digstore")
        .unwrap()
        .args(&["version", "install-current"])
        .assert();
    let output = command_assert.get_output();
    
    // Should either succeed or fail gracefully
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Installing current digstore binary") ||
            stdout.contains("Installation completed successfully"),
            "Should show installation progress"
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Common expected failure reasons in test environments
        assert!(
            stderr.contains("project directory") ||
            stderr.contains("permission") ||
            stderr.contains("access"),
            "Should fail gracefully with helpful error"
        );
    }
}

#[test]
fn test_version_command_error_handling() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test commands that require parameters
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "set"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Version required"));
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "remove"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Version required"));
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "update-path"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Version required"));
    
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "install-msi"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("MSI path required"));
}

#[test]
fn test_version_set_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test setting non-existent version
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "set", "999.999.999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not installed").or(predicate::str::contains("not found")));
}

#[test]
fn test_version_install_msi_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test installing from non-existent MSI
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "install-msi", "/nonexistent/file.msi"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_update_command_with_version_management() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test update check-only (should not fail)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["update", "--check-only"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Checking for updates")
                .or(predicate::str::contains("latest version"))
                .or(predicate::str::contains("Update available"))
        );
}

#[test]
fn test_version_management_json_output() {
    let temp_dir = TempDir::new().unwrap();
    
    // Test that version commands can produce JSON when needed
    let command_assert = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["update", "--check-only", "--json"])
        .assert()
        .success();
    let output = command_assert.get_output();
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Should be valid JSON
    if !stdout.trim().is_empty() {
        let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
        assert!(
            json_result.is_ok(),
            "Update JSON output should be valid JSON: {}",
            stdout
        );
        
        if let Ok(json) = json_result {
            // Should contain version information
            assert!(
                json.get("current_version").is_some() ||
                json.get("action").is_some(),
                "JSON should contain version or action information"
            );
        }
    }
}
