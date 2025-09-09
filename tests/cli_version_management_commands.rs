//! CLI tests for version management commands
//!
//! Tests all the version management CLI commands and their output.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_version_subcommands_help() {
    let temp_dir = TempDir::new().unwrap();

    // Test main version command
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("Version Information"))
        .stdout(predicate::str::contains("Current Version:"))
        .stdout(predicate::str::contains("Available Commands:"));
}

#[test]
fn test_version_list_command_output() {
    let temp_dir = TempDir::new().unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "list"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Installed Versions")
                .or(predicate::str::contains("No versions installed")),
        );
}

#[test]
fn test_version_list_system_command() {
    let temp_dir = TempDir::new().unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "list-system"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("System-Installed Versions")
                .or(predicate::str::contains("No system versions installed")),
        );
}

#[test]
fn test_version_current_command_output() {
    let temp_dir = TempDir::new().unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "current"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Current version")
                .or(predicate::str::contains("No versions managed")),
        );
}

#[test]
fn test_version_error_messages() {
    let temp_dir = TempDir::new().unwrap();

    // Test error messages are helpful and specific

    // Missing version parameter
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

    // Missing MSI path
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "install-msi"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("MSI path required"));

    // Non-existent version
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "set", "999.999.999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not installed"));

    // Non-existent MSI file
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "install-msi", "/fake/path/nonexistent.msi"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_version_command_consistency() {
    let temp_dir = TempDir::new().unwrap();

    // Test that version commands have consistent output format
    let commands = [
        vec!["version"],
        vec!["version", "list"],
        vec!["version", "list-system"],
        vec!["version", "current"],
        vec!["version", "fix-path"],
    ];

    for command in &commands {
        let command_assert = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(temp_dir.path())
            .args(command)
            .assert()
            .success();
        let output = command_assert.get_output();

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should not be empty
        assert!(
            !stdout.trim().is_empty(),
            "Command {:?} should produce output",
            command
        );

        // Should not contain error indicators in success output
        assert!(
            !stdout.contains("Error:") && !stdout.contains("Failed:"),
            "Success output should not contain error indicators for {:?}",
            command
        );
    }
}

#[test]
fn test_version_install_current_error_handling() {
    let temp_dir = TempDir::new().unwrap();

    // Test install-current from non-project directory
    let command_assert = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "install-current"])
        .assert();
    let output = command_assert.get_output();

    // This might succeed or fail depending on the test environment
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Installing current digstore binary")
                || stdout.contains("Installation completed"),
            "Should show installation progress when successful"
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("project directory")
                || stderr.contains("Cargo.toml")
                || stderr.contains("digstore project"),
            "Should provide helpful error about project directory requirement"
        );
    }
}

#[test]
fn test_path_fix_command_output_quality() {
    let temp_dir = TempDir::new().unwrap();

    // Test that PATH analysis provides useful information
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["version", "fix-path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Analyzing PATH"))
        .stdout(
            predicate::str::contains("Found digstore installations")
                .or(predicate::str::contains("No digstore installations")),
        )
        .stdout(predicate::str::contains("Position").or(predicate::str::contains("No digstore")));
}

#[test]
fn test_version_management_integration_with_main_commands() {
    let temp_dir = TempDir::new().unwrap();

    // Test that version management doesn't interfere with main digstore commands

    // These commands should work regardless of version management state
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("digstore"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("digstore"));

    // Update check should work
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(temp_dir.path())
        .args(&["update", "--check-only"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Checking for updates"));
}
