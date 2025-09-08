//! User Security Features Tests
//!
//! These tests validate the zero-knowledge and encryption features from a user perspective.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_zero_knowledge_user_experience() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test that invalid URNs return deterministic data (zero-knowledge property)

    // User gets data for invalid URN - should succeed with random data
    let invalid_urn1 =
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/fake1.txt";
    let output1 = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", invalid_urn1])
        .assert()
        .success()
        .get_output();

    // Same invalid URN should return same data (deterministic)
    let output2 = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", invalid_urn1])
        .assert()
        .success()
        .get_output();

    assert_eq!(
        output1.stdout, output2.stdout,
        "Same invalid URN should return identical data"
    );

    // Different invalid URN should return different data
    let invalid_urn2 =
        "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake2.txt";
    let output3 = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", invalid_urn2])
        .assert()
        .success()
        .get_output();

    assert_ne!(
        output1.stdout, output3.stdout,
        "Different invalid URNs should return different data"
    );

    // Test that all invalid URNs return data (no error responses)
    let invalid_urns = [
        "urn:dig:chia:2222222222222222222222222222222222222222222222222222222222222222/test.txt",
        "urn:dig:chia:3333333333333333333333333333333333333333333333333333333333333333/document.pdf",
        "urn:dig:chia:4444444444444444444444444444444444444444444444444444444444444444/image.jpg",
        "malformed-urn-format",
        "urn:dig:chia:invalid-store/file.txt",
    ];

    for urn in &invalid_urns {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn])
            .assert()
            .success(); // Should always succeed, never return errors
    }
}

#[test]
fn test_encryption_setup_user_workflow() {
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

    // User generates content keys
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:abc123/secret.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated Keys"))
        .stdout(predicate::str::contains("Storage Address"))
        .stdout(predicate::str::contains("Encryption Key"))
        .stdout(predicate::str::contains(
            "Zero-knowledge storage addressing",
        ));

    // User can get JSON format for keys
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:abc123/secret.txt", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"storage_address\""))
        .stdout(predicate::str::contains("\"encryption_key\""))
        .stdout(predicate::str::contains("\"urn\""));

    // User can get specific key types
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "keygen",
            "urn:dig:chia:abc123/secret.txt",
            "--storage-address",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Storage Address"));

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "keygen",
            "urn:dig:chia:abc123/secret.txt",
            "--encryption-key",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Encryption Key"));
}

#[test]
fn test_encryption_workflow_validation() {
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

    // Create sensitive content
    fs::write(
        project_path.join("sensitive.txt"),
        "Highly sensitive information",
    )
    .unwrap();
    fs::write(project_path.join("public.txt"), "Public information").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Encrypted Test"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "sensitive.txt", "public.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Encrypted data commit"])
        .assert()
        .success();

    // User retrieves encrypted data (returns encrypted, not plaintext)
    let encrypted_output = Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "sensitive.txt", "-o", "encrypted_sensitive.bin"])
        .assert()
        .success()
        .get_output();

    let stdout = String::from_utf8_lossy(&encrypted_output.stdout);
    assert!(stdout.contains("Content written to"));

    // Verify encrypted file exists
    assert!(project_path.join("encrypted_sensitive.bin").exists());

    // User can decrypt using URN
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "decrypt",
            "encrypted_sensitive.bin",
            "--urn",
            "urn:dig:chia:test123/sensitive.txt",
            "-o",
            "decrypted.txt",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Decrypted content written to"));

    // Verify decrypted content matches original
    let decrypted = fs::read_to_string(project_path.join("decrypted.txt")).unwrap();
    assert!(decrypted.contains("Highly sensitive"));

    // User can generate keys for analysis
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:test123/sensitive.txt", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"transformed_address\""))
        .stdout(predicate::str::contains("\"encryption_key\""));
}

#[test]
fn test_deterministic_decoy_sizes() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test that decoy sizes are deterministic and realistic
    let test_urns = [
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/test1.txt",
        "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/test2.txt",
        "urn:dig:chia:2222222222222222222222222222222222222222222222222222222222222222/test3.txt",
    ];

    let mut sizes = Vec::new();

    for urn in &test_urns {
        // Get content and measure size
        let output = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn, "-o", "temp_output.bin"])
            .assert()
            .success()
            .get_output();

        // Verify file was created
        let temp_file = project_path.join("temp_output.bin");
        assert!(temp_file.exists());

        let size = fs::metadata(&temp_file).unwrap().len();
        sizes.push(size);

        // Clean up
        fs::remove_file(&temp_file).unwrap();

        // Test that same URN returns same size (deterministic)
        let output2 = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn, "-o", "temp_output2.bin"])
            .assert()
            .success()
            .get_output();

        let size2 = fs::metadata(project_path.join("temp_output2.bin"))
            .unwrap()
            .len();
        assert_eq!(size, size2, "Same URN should return same size");

        fs::remove_file(project_path.join("temp_output2.bin")).unwrap();
    }

    // Sizes should be varied (not all the same)
    let first_size = sizes[0];
    let all_same = sizes.iter().all(|&size| size == first_size);
    assert!(!all_same, "Decoy sizes should be varied, not all the same");

    // Sizes should be realistic (1KB to 20MB range)
    for &size in &sizes {
        assert!(
            size >= 1024,
            "Decoy size should be at least 1KB, got {}",
            size
        );
        assert!(
            size <= 20 * 1024 * 1024,
            "Decoy size should be at most 20MB, got {}",
            size
        );
    }
}

#[test]
fn test_zero_knowledge_properties() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test various invalid URN formats - all should return data, never errors
    let invalid_formats = [
        "urn:dig:chia:invalid-store-id/file.txt",
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/nonexistent.txt",
        "malformed-urn",
        "urn:dig:chia:short/file.txt",
        "urn:dig:chia:toolong000000000000000000000000000000000000000000000000000000000000000000/file.txt",
    ];

    for invalid_urn in &invalid_formats {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", invalid_urn])
            .assert()
            .success(); // Should never return error, always return data
    }

    // Test byte range with invalid URNs
    let invalid_with_ranges = [
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/fake.txt#bytes=0-1023",
        "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/fake.txt#bytes=1024-",
        "urn:dig:chia:2222222222222222222222222222222222222222222222222222222222222222/fake.txt#bytes=-2048",
    ];

    for invalid_urn in &invalid_with_ranges {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", invalid_urn])
            .assert()
            .success(); // Should respect byte ranges in random data
    }

    // Test that no timing differences reveal validity
    use std::time::Instant;

    let valid_looking_urn =
        "urn:dig:chia:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/file.txt";
    let invalid_urn =
        "urn:dig:chia:0000000000000000000000000000000000000000000000000000000000000000/file.txt";

    // Measure timing for both (should be similar)
    let start1 = Instant::now();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", valid_looking_urn])
        .assert()
        .success();
    let time1 = start1.elapsed();

    let start2 = Instant::now();
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", invalid_urn])
        .assert()
        .success();
    let time2 = start2.elapsed();

    // Times should be reasonably similar (within an order of magnitude)
    let ratio = time1.as_millis() as f64 / time2.as_millis().max(1) as f64;
    assert!(
        ratio > 0.1 && ratio < 10.0,
        "Timing should not reveal URN validity"
    );
}

#[test]
fn test_encrypted_storage_user_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // User configures encryption
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

    // User verifies encryption is configured
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["config", "crypto.public_key"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fedcba0987654321"));

    // User creates repository with sensitive data
    fs::write(
        project_path.join("confidential.txt"),
        "Confidential business data",
    )
    .unwrap();
    fs::write(project_path.join("public.txt"), "Public information").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Encrypted Repository"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "confidential.txt", "public.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Encrypted data storage"])
        .assert()
        .success();

    // User retrieves encrypted data (should get encrypted bytes, not plaintext)
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["get", "confidential.txt", "-o", "encrypted_output.bin"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Content written to"));

    // User generates keys for their content
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:test123/confidential.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated Keys"))
        .stdout(predicate::str::contains("Security Properties"));

    // User can export key information
    let keys_file = project_path.join("keys.json");
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "keygen",
            "urn:dig:chia:test123/confidential.txt",
            "--json",
            "-o",
            keys_file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Key information written to"));

    // Verify keys file is valid JSON
    assert!(keys_file.exists());
    let keys_content = fs::read_to_string(&keys_file).unwrap();
    let _: serde_json::Value =
        serde_json::from_str(&keys_content).expect("Keys should be valid JSON");
}

#[test]
fn test_public_key_validation() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test invalid public key formats
    let invalid_keys = [
        "short",
        "toolong1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        "invalid-hex-characters-gggg1234567890abcdef1234567890abcdef1234567890",
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcde", // 63 chars
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

    // Test valid public key
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
}

#[test]
fn test_keygen_without_public_key() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // User tries keygen without configuring public key
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "urn:dig:chia:test123/file.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No public key configured"))
        .stderr(predicate::str::contains(
            "digstore config crypto.public_key",
        ));
}

#[test]
fn test_decrypt_command_user_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup encryption
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "1111222233334444555566667777888899990000aaaabbbbccccddddeeeeffff",
        ])
        .assert()
        .success();

    // Create test data
    fs::write(
        project_path.join("decrypt_test.txt"),
        "Decrypt test content",
    )
    .unwrap();

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

    // User can stream decrypted content to stdout
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "decrypt",
            "encrypted.bin",
            "--urn",
            "urn:dig:chia:test123/decrypt_test.txt",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Decrypt test content"));

    // User gets JSON metadata
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "decrypt",
            "encrypted.bin",
            "--urn",
            "urn:dig:chia:test123/decrypt_test.txt",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"action\""))
        .stdout(predicate::str::contains("\"decrypted_size\""));
}

#[test]
fn test_security_features_integration() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup complete security configuration
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .assert()
        .success();

    // Create multi-file project
    fs::create_dir_all(project_path.join("secure")).unwrap();
    fs::write(
        project_path.join("secure/document1.txt"),
        "Secure document 1",
    )
    .unwrap();
    fs::write(
        project_path.join("secure/document2.txt"),
        "Secure document 2",
    )
    .unwrap();
    fs::write(project_path.join("public.txt"), "Public document").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Security Integration"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "-A"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Secure multi-file commit"])
        .assert()
        .success();

    // User can generate keys for different files
    for filename in &["secure/document1.txt", "secure/document2.txt", "public.txt"] {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["keygen", &format!("urn:dig:chia:test123/{}", filename)])
            .assert()
            .success()
            .stdout(predicate::str::contains("Generated Keys"));
    }

    // User can retrieve all files (as encrypted data)
    for filename in &["secure/document1.txt", "secure/document2.txt", "public.txt"] {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", filename])
            .assert()
            .success();
    }

    // User can analyze security metrics
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--security"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Security Metrics"));
}

#[test]
fn test_advanced_urn_features() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create file for byte range testing
    let content = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    fs::write(project_path.join("range_test.txt"), content).unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "URN Features"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "range_test.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Range test"])
        .assert()
        .success();

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

    // Test cat command with URNs
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["cat", "range_test.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains(content));
}

#[test]
fn test_security_error_handling() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test keygen with invalid URN formats
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        ])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["keygen", "invalid-urn-format"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid URN format"));

    // Test decrypt with invalid files
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["decrypt", "nonexistent.bin"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No such file"));

    // Test decrypt with malformed data
    fs::write(project_path.join("malformed.bin"), "not encrypted data").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "decrypt",
            "malformed.bin",
            "--urn",
            "urn:dig:chia:test123/file.txt",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Decryption failed"));
}

#[test]
fn test_deterministic_random_data_quality() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test that deterministic random data has good properties
    let test_urn =
        "urn:dig:chia:abcd1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab/test.txt";

    // Get data multiple times
    let mut outputs = Vec::new();
    for _ in 0..5 {
        let output = Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", test_urn, "-o", "random_test.bin"])
            .assert()
            .success()
            .get_output();

        let data = fs::read(project_path.join("random_test.bin")).unwrap();
        outputs.push(data);
        fs::remove_file(project_path.join("random_test.bin")).unwrap();
    }

    // All outputs should be identical (deterministic)
    for i in 1..outputs.len() {
        assert_eq!(
            outputs[0], outputs[i],
            "Deterministic random data should be identical across calls"
        );
    }

    // Data should not be obviously patterned
    let data = &outputs[0];
    assert!(
        data.len() > 100,
        "Should generate reasonable amount of data"
    );

    // Simple entropy check - data should not be all the same byte
    let first_byte = data[0];
    let all_same = data.iter().all(|&b| b == first_byte);
    assert!(!all_same, "Random data should not be all the same byte");

    // Should have some variation in bytes
    let unique_bytes: std::collections::HashSet<_> = data.iter().collect();
    assert!(
        unique_bytes.len() > 10,
        "Should have reasonable byte variety"
    );
}

#[test]
fn test_urn_construction_and_validation() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Test various URN formats that users might try
    let valid_urns = [
        "urn:dig:chia:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef/file.txt",
        "urn:dig:chia:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/path/to/file.txt",
        "urn:dig:chia:fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321/file.txt#bytes=0-1023",
        "urn:dig:chia:1111111111111111111111111111111111111111111111111111111111111111/file.txt#bytes=1024-",
        "urn:dig:chia:2222222222222222222222222222222222222222222222222222222222222222/file.txt#bytes=-512",
    ];

    for urn in &valid_urns {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn])
            .assert()
            .success(); // Should handle all valid URN formats
    }

    // Test malformed URNs (should still return data, not errors)
    let malformed_urns = [
        "malformed-urn",
        "urn:dig:chia:short/file.txt",
        "urn:dig:invalid:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef/file.txt",
        "not-a-urn-at-all",
    ];

    for urn in &malformed_urns {
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", urn])
            .assert()
            .success(); // Zero-knowledge: should return data, not errors
    }
}

#[test]
fn test_comprehensive_security_validation() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Setup comprehensive security test
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&[
            "config",
            "crypto.public_key",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ])
        .assert()
        .success();

    // Create test files
    fs::write(project_path.join("secure1.txt"), "Secure content 1").unwrap();
    fs::write(project_path.join("secure2.txt"), "Secure content 2").unwrap();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["init", "--name", "Comprehensive Security"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["--yes", "add", "secure1.txt", "secure2.txt"])
        .assert()
        .success();

    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["commit", "-m", "Secure commit"])
        .assert()
        .success();

    // Test complete encryption workflow
    for filename in &["secure1.txt", "secure2.txt"] {
        // Generate keys
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["keygen", &format!("urn:dig:chia:test123/{}", filename)])
            .assert()
            .success()
            .stdout(predicate::str::contains("Generated Keys"));

        // Get encrypted data
        let encrypted_file = format!("encrypted_{}.bin", filename.replace('/', "_"));
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&["get", filename, "-o", &encrypted_file])
            .assert()
            .success();

        // Decrypt data
        let decrypted_file = format!("decrypted_{}.txt", filename.replace('/', "_"));
        Command::cargo_bin("digstore")
            .unwrap()
            .current_dir(project_path)
            .args(&[
                "decrypt",
                &encrypted_file,
                "--urn",
                &format!("urn:dig:chia:test123/{}", filename),
                "-o",
                &decrypted_file,
            ])
            .assert()
            .success();

        // Verify content matches
        let original = fs::read_to_string(project_path.join(filename)).unwrap();
        let decrypted = fs::read_to_string(project_path.join(&decrypted_file)).unwrap();
        assert!(decrypted.contains(&original.trim()));
    }

    // Test security metrics
    Command::cargo_bin("digstore")
        .unwrap()
        .current_dir(project_path)
        .args(&["stats", "--security"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Security Metrics"))
        .stdout(predicate::str::contains("Scrambling:"))
        .stdout(predicate::str::contains("URN Access Control:"));
}
