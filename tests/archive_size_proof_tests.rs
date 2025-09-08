//! Comprehensive tests for archive size proof system
//!
//! These tests validate the tamper-proof merkle proof generation and verification
//! for .dig archive file sizes without requiring file downloads.

use assert_cmd::Command;
use digstore_min::proofs::size_proof::{
    verify_archive_size_proof, verify_compressed_hex_proof, ArchiveSizeProof,
};
use digstore_min::storage::Store;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Helper to create a test repository with known content
struct TestArchive {
    temp_dir: TempDir,
    project_path: std::path::PathBuf,
    store_id: String,
    root_hash: String,
    archive_size: u64,
}

impl TestArchive {
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().to_path_buf();

        // Create test content
        fs::write(
            project_path.join("test1.txt"),
            "Test content for proof generation",
        )?;
        fs::write(
            project_path.join("test2.txt"),
            "More test content with different size",
        )?;
        fs::create_dir_all(project_path.join("subdir"))?;
        fs::write(
            project_path.join("subdir").join("nested.txt"),
            "Nested file content",
        )?;

        // Initialize digstore repository
        Command::cargo_bin("digstore")?
            .current_dir(&project_path)
            .args(&["init", "--name", "Archive Size Proof Test"])
            .assert()
            .success();

        // Add files and commit
        Command::cargo_bin("digstore")?
            .current_dir(&project_path)
            .args(&["--yes", "add", "."])
            .assert()
            .success();

        let commit_output = Command::cargo_bin("digstore")?
            .current_dir(&project_path)
            .args(&["commit", "-m", "Test commit for size proof"])
            .assert()
            .success()
            .get_output();

        // Extract store info
        let store_output = Command::cargo_bin("digstore")?
            .current_dir(&project_path)
            .args(&["store", "info", "--json"])
            .assert()
            .success()
            .get_output();

        let store_info: serde_json::Value = serde_json::from_slice(&store_output.stdout)?;
        let store_id = store_info["store_id"].as_str().unwrap().to_string();
        let root_hash = store_info["root_hash"].as_str().unwrap().to_string();

        // Get actual archive size
        let archive_path = format!(
            "{}/.dig/{}.dig",
            std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?,
            store_id
        );
        let archive_size = fs::metadata(&archive_path)?.len();

        Ok(Self {
            temp_dir,
            project_path,
            store_id,
            root_hash,
            archive_size,
        })
    }
}

#[test]
fn test_proof_generation_basic() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;

    // Generate proof using CLI
    let output = Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    // Verify we got a hex-encoded compressed proof
    let proof_hex = String::from_utf8(output.stdout)?.trim().to_string();
    assert!(!proof_hex.is_empty(), "Proof should not be empty");
    assert!(
        proof_hex.chars().all(|c| c.is_ascii_hexdigit()),
        "Proof should be hex-encoded"
    );
    assert!(proof_hex.len() > 100, "Proof should be substantial size");
    assert!(proof_hex.len() < 1000, "Proof should be compressed");

    Ok(())
}

#[test]
fn test_proof_generation_with_output_file() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;
    let proof_file = test_archive.project_path.join("test_proof.txt");

    // Generate proof with -o parameter
    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
            "-o",
            proof_file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("✓ Proof saved to"));

    // Verify file was created and contains proof
    assert!(proof_file.exists(), "Proof file should be created");
    let proof_content = fs::read_to_string(&proof_file)?;
    assert!(
        !proof_content.trim().is_empty(),
        "Proof file should not be empty"
    );
    assert!(
        proof_content
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c.is_whitespace()),
        "Proof file should contain hex data"
    );

    Ok(())
}

#[test]
fn test_proof_verification_success() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;

    // Generate proof
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Verify proof
    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "verify-archive-size",
            &proof_hex,
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "✓ Archive size proof verified successfully",
        ));

    Ok(())
}

#[test]
fn test_proof_verification_from_file() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;
    let proof_file = test_archive.project_path.join("verify_test_proof.txt");

    // Generate proof to file
    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
            "-o",
            proof_file.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify proof from file
    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "verify-archive-size",
            "--from-file",
            proof_file.to_str().unwrap(),
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "✓ Archive size proof verified successfully",
        ));

    Ok(())
}

#[test]
fn test_proof_generation_verbose_output() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;

    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
            "--verbose",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Archive found"))
        .stdout(predicate::str::contains("Layer count"))
        .stdout(predicate::str::contains("Total size"))
        .stdout(predicate::str::contains("Merkle tree"));

    Ok(())
}

#[test]
fn test_proof_generation_with_compression_info() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;

    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
            "--show-compression",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Compression"))
        .stdout(predicate::str::contains("bytes"))
        .stdout(predicate::str::contains("reduction"));

    Ok(())
}

#[test]
fn test_proof_verification_verbose_output() -> anyhow::Result<()> {
    let test_archive = TestArchive::new()?;

    // Generate proof
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "prove-archive-size",
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Verify with verbose output
    Command::cargo_bin("digstore")?
        .current_dir(&test_archive.project_path)
        .args(&[
            "verify-archive-size",
            &proof_hex,
            &test_archive.store_id,
            &test_archive.root_hash,
            &test_archive.archive_size.to_string(),
            "--verbose",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Store ID"))
        .stdout(predicate::str::contains("Root Hash"))
        .stdout(predicate::str::contains("Expected Size"))
        .stdout(predicate::str::contains("Decompressed"))
        .stdout(predicate::str::contains("Merkle verification"));

    Ok(())
}

/// Test error cases and edge conditions
mod error_cases {
    use super::*;

    #[test]
    fn test_invalid_store_id() {
        Command::cargo_bin("digstore")
            .unwrap()
            .args(&[
                "prove-archive-size",
                "invalid_store_id",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "1000",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid store ID format"));
    }

    #[test]
    fn test_invalid_root_hash() {
        Command::cargo_bin("digstore")
            .unwrap()
            .args(&[
                "prove-archive-size",
                "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2",
                "invalid_root_hash",
                "1000",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid root hash format"));
    }

    #[test]
    fn test_nonexistent_archive() {
        Command::cargo_bin("digstore")
            .unwrap()
            .args(&[
                "prove-archive-size",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "1000",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Archive not found")
                    .or(predicate::str::contains("No such file")),
            );
    }

    #[test]
    fn test_size_mismatch() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        // Try to prove with wrong size (should fail)
        Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &(test_archive.archive_size + 1000).to_string(), // Wrong size
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Size mismatch"));

        Ok(())
    }

    #[test]
    fn test_invalid_proof_verification() {
        Command::cargo_bin("digstore")
            .unwrap()
            .args(&[
                "verify-archive-size",
                "invalid_hex_proof",
                "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "1000",
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Invalid hex")
                    .or(predicate::str::contains("Failed to decode")),
            );
    }

    #[test]
    fn test_corrupted_proof_verification() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        // Generate valid proof
        let proof_output = Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
            ])
            .assert()
            .success()
            .get_output();

        let mut proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

        // Corrupt the proof by changing some characters
        if proof_hex.len() > 10 {
            proof_hex.replace_range(5..10, "FFFFF");
        }

        // Try to verify corrupted proof (should fail)
        Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "verify-archive-size",
                &proof_hex,
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Verification failed")
                    .or(predicate::str::contains("Invalid proof")),
            );

        Ok(())
    }
}

/// Test JSON output formats
mod json_output {
    use super::*;

    #[test]
    fn test_proof_generation_json_output() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        let output = Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
                "--json",
            ])
            .assert()
            .success()
            .get_output();

        // Verify JSON format
        let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert!(json_output["success"].as_bool().unwrap_or(false));
        assert!(json_output["proof"].as_str().is_some());
        assert!(json_output["proof"].as_str().unwrap().len() > 100);

        Ok(())
    }

    #[test]
    fn test_proof_verification_json_output() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        // Generate proof
        let proof_output = Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
            ])
            .assert()
            .success()
            .get_output();

        let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

        // Verify with JSON output
        let verify_output = Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "verify-archive-size",
                &proof_hex,
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
                "--json",
            ])
            .assert()
            .success()
            .get_output();

        // Verify JSON format
        let json_output: serde_json::Value = serde_json::from_slice(&verify_output.stdout)?;
        assert!(json_output["success"].as_bool().unwrap_or(false));
        assert!(json_output["verified"].as_bool().unwrap_or(false));
        assert_eq!(
            json_output["archive_size"].as_u64().unwrap(),
            test_archive.archive_size
        );

        Ok(())
    }
}

/// Performance and compression tests
mod performance {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_proof_generation_speed() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        let start = Instant::now();

        Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
            ])
            .assert()
            .success();

        let duration = start.elapsed();

        // Proof generation should be fast (< 5 seconds)
        assert!(
            duration.as_secs() < 5,
            "Proof generation took too long: {:?}",
            duration
        );

        Ok(())
    }

    #[test]
    fn test_proof_verification_speed() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        // Generate proof first
        let proof_output = Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
            ])
            .assert()
            .success()
            .get_output();

        let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

        let start = Instant::now();

        Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "verify-archive-size",
                &proof_hex,
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
            ])
            .assert()
            .success();

        let duration = start.elapsed();

        // Verification should be very fast (< 1 second)
        assert!(
            duration.as_secs() < 1,
            "Proof verification took too long: {:?}",
            duration
        );

        Ok(())
    }

    #[test]
    fn test_proof_compression_ratio() -> anyhow::Result<()> {
        let test_archive = TestArchive::new()?;

        let output = Command::cargo_bin("digstore")?
            .current_dir(&test_archive.project_path)
            .args(&[
                "prove-archive-size",
                &test_archive.store_id,
                &test_archive.root_hash,
                &test_archive.archive_size.to_string(),
                "--show-compression",
            ])
            .assert()
            .success()
            .get_output();

        let proof_hex = String::from_utf8(output.stdout)?.trim().to_string();

        // Extract just the hex proof (last line)
        let proof_only = proof_hex.lines().last().unwrap_or(&proof_hex);

        // Proof should be compressed (< 1000 characters for small archives)
        assert!(
            proof_only.len() < 1000,
            "Proof not sufficiently compressed: {} chars",
            proof_only.len()
        );

        // But should still contain substantial data (> 100 characters)
        assert!(
            proof_only.len() > 100,
            "Proof too small, might be missing data: {} chars",
            proof_only.len()
        );

        Ok(())
    }
}
