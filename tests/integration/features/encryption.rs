//! Integration tests for encryption features
//!
//! Tests the complete encryption workflow including key generation and decryption.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_encryption_setup_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // User sets up encryption
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓"));

    // User verifies encryption configuration
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "crypto.public_key"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1234567890abcdef"));

    // User creates repository (encryption enabled by default)
    fs::write(project_path.join("secret.txt"), "Secret document content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Encrypted Repository"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "secret.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Encrypted commit"])
        .assert()
        .success();
}

#[test]
fn test_keygen_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup encryption
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        ])
        .assert()
        .success();

    // User generates content keys
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:abc123/secret.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated Keys"))
        .stdout(predicate::str::contains("Storage Address"))
        .stdout(predicate::str::contains("Encryption Key"));

    // User can get JSON format
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:abc123/secret.txt", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"storage_address\""))
        .stdout(predicate::str::contains("\"encryption_key\""));
}

#[test]
fn test_decrypt_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup encryption
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
        ])
        .assert()
        .success();

    // Create test data
    fs::write(project_path.join("decrypt_test.txt"), "Decrypt test content").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Decrypt Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "decrypt_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Decrypt test"])
        .assert()
        .success();

    // User gets encrypted data
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "decrypt_test.txt", "-o", "encrypted.bin"])
        .assert()
        .success();

    // User decrypts using URN
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "decrypt",
            "encrypted.bin",
            "--urn",
            "urn:dig:chia:test123/decrypt_test.txt",
            "-o",
            "decrypted.txt",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Decrypted content written to"));
}

#[test]
fn test_encryption_error_handling() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test keygen without public key configured
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:test123/file.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No public key configured"));

    // Test invalid public key formats
    let invalid_keys = [
        "short",
        "toolong1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        "invalid-hex-gggg1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    ];

    for invalid_key in &invalid_keys {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["config", "crypto.public_key", invalid_key])
            .assert()
            .failure()
            .stderr(predicate::str::contains("64-character hex string"));
    }
}
