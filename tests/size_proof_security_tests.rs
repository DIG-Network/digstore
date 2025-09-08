//! Security tests for archive size proof system
//!
//! These tests validate the tamper-proof properties and security guarantees
//! of the merkle proof system for .dig archive file sizes.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to create test repositories for security testing
struct SecurityTestSetup {
    temp_dir1: TempDir,
    temp_dir2: TempDir,
    project_path1: std::path::PathBuf,
    project_path2: std::path::PathBuf,
    store_id1: String,
    store_id2: String,
    root_hash1: String,
    root_hash2: String,
    archive_size1: u64,
    archive_size2: u64,
}

impl SecurityTestSetup {
    fn new() -> anyhow::Result<Self> {
        // Create two different test repositories
        let temp_dir1 = TempDir::new()?;
        let temp_dir2 = TempDir::new()?;
        let project_path1 = temp_dir1.path().to_path_buf();
        let project_path2 = temp_dir2.path().to_path_buf();

        // Repository 1: Small files
        fs::write(project_path1.join("small1.txt"), "Small content 1")?;
        fs::write(project_path1.join("small2.txt"), "Small content 2")?;

        // Repository 2: Different content and size
        fs::write(
            project_path2.join("large.txt"),
            "Large content ".repeat(1000),
        )?;
        fs::create_dir_all(project_path2.join("subdir"))?;
        fs::write(
            project_path2.join("subdir").join("nested.txt"),
            "Nested content ".repeat(500),
        )?;

        // Initialize both repositories
        for (path, name) in [
            (&project_path1, "Security Test 1"),
            (&project_path2, "Security Test 2"),
        ] {
            Command::cargo_bin("digstore")?
                .current_dir(path)
                .args(&["init", "--name", name])
                .assert()
                .success();

            Command::cargo_bin("digstore")?
                .current_dir(path)
                .args(&["--yes", "add", "."])
                .assert()
                .success();

            Command::cargo_bin("digstore")?
                .current_dir(path)
                .args(&["commit", "-m", "Security test commit"])
                .assert()
                .success();
        }

        // Get store information for both
        let get_store_info = |path: &std::path::Path| -> anyhow::Result<(String, String, u64)> {
            let store_output = Command::cargo_bin("digstore")?
                .current_dir(path)
                .args(&["store", "info", "--json"])
                .assert()
                .success()
                .get_output();

            let store_info: serde_json::Value = serde_json::from_slice(&store_output.stdout)?;
            let store_id = store_info["store_id"].as_str().unwrap().to_string();
            let root_hash = store_info["root_hash"].as_str().unwrap().to_string();

            let archive_path = format!(
                "{}/.dig/{}.dig",
                std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?,
                store_id
            );
            let archive_size = fs::metadata(&archive_path)?.len();

            Ok((store_id, root_hash, archive_size))
        };

        let (store_id1, root_hash1, archive_size1) = get_store_info(&project_path1)?;
        let (store_id2, root_hash2, archive_size2) = get_store_info(&project_path2)?;

        Ok(Self {
            temp_dir1,
            temp_dir2,
            project_path1,
            project_path2,
            store_id1,
            store_id2,
            root_hash1,
            root_hash2,
            archive_size1,
            archive_size2,
        })
    }
}

/// Test that proofs are tied to specific store IDs
#[test]
fn test_store_id_binding() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proof for repository 1
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Try to verify proof with different store ID (should fail)
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path2)
        .args(&[
            "verify-archive-size",
            &proof_hex,
            &setup.store_id2, // Wrong store ID
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Verification failed")
                .or(predicate::str::contains("mismatch")),
        );

    Ok(())
}

/// Test that proofs are tied to specific root hashes
#[test]
fn test_root_hash_binding() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proof for repository 1
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Try to verify proof with different root hash (should fail)
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "verify-archive-size",
            &proof_hex,
            &setup.store_id1,
            &setup.root_hash2, // Wrong root hash
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Verification failed")
                .or(predicate::str::contains("mismatch")),
        );

    Ok(())
}

/// Test that proofs are tied to specific archive sizes
#[test]
fn test_size_binding() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proof for repository 1
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Try to verify proof with different size (should fail)
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "verify-archive-size",
            &proof_hex,
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size2.to_string(), // Wrong size
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Verification failed")
                .or(predicate::str::contains("mismatch")),
        );

    Ok(())
}

/// Test that you cannot substitute one archive's proof for another
#[test]
fn test_archive_substitution_attack() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proof for repository 1
    let proof1_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof1_hex = String::from_utf8(proof1_output.stdout)?.trim().to_string();

    // Try to use proof1 to verify repository 2's parameters (should fail)
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path2)
        .args(&[
            "verify-archive-size",
            &proof1_hex,
            &setup.store_id2,                 // Different archive
            &setup.root_hash2,                // Different root hash
            &setup.archive_size2.to_string(), // Different size
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Verification failed")
                .or(predicate::str::contains("mismatch")),
        );

    Ok(())
}

/// Test that proof generation fails for non-matching parameters
#[test]
fn test_proof_generation_parameter_validation() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Try to generate proof with wrong size for repository 1 (should fail)
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size2.to_string(), // Wrong size
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Size mismatch")
                .or(predicate::str::contains("does not match")),
        );

    // Try to generate proof with non-existent store ID (should fail)
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            "0000000000000000000000000000000000000000000000000000000000000000", // Non-existent
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Archive not found")
                .or(predicate::str::contains("No such file")),
        );

    Ok(())
}

/// Test resistance to proof tampering
#[test]
fn test_proof_tampering_resistance() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate valid proof
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let original_proof = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Test various tampering attempts
    let tampering_tests = vec![
        ("flip_first_bit", {
            let mut tampered = original_proof.clone();
            if let Some(first_char) = tampered.chars().next() {
                let flipped = if first_char == '0' { '1' } else { '0' };
                tampered.replace_range(0..1, &flipped.to_string());
            }
            tampered
        }),
        ("flip_middle_bits", {
            let mut tampered = original_proof.clone();
            let mid = tampered.len() / 2;
            if mid < tampered.len() {
                tampered.replace_range(mid..mid + 1, "F");
            }
            tampered
        }),
        ("truncate", {
            let mut tampered = original_proof.clone();
            tampered.truncate(tampered.len() - 10);
            tampered
        }),
        ("extend", { format!("{}DEADBEEF", original_proof) }),
        ("replace_section", {
            let mut tampered = original_proof.clone();
            let start = tampered.len() / 4;
            let end = start + 8;
            if end <= tampered.len() {
                tampered.replace_range(start..end, "FFFFFFFF");
            }
            tampered
        }),
    ];

    for (test_name, tampered_proof) in tampering_tests {
        // All tampering attempts should fail verification
        let result = Command::cargo_bin("digstore")?
            .current_dir(&setup.project_path1)
            .args(&[
                "verify-archive-size",
                &tampered_proof,
                &setup.store_id1,
                &setup.root_hash1,
                &setup.archive_size1.to_string(),
            ])
            .assert()
            .failure();

        // Should fail with appropriate error message
        result.stderr(
            predicate::str::contains("Verification failed")
                .or(predicate::str::contains("Invalid proof"))
                .or(predicate::str::contains("Invalid hex"))
                .or(predicate::str::contains("Failed to decode")),
        );

        println!("✓ Tampering test '{}' correctly failed", test_name);
    }

    Ok(())
}

/// Test deterministic proof generation
#[test]
fn test_deterministic_proof_generation() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate the same proof multiple times
    let mut proofs = vec![];
    for _ in 0..5 {
        let proof_output = Command::cargo_bin("digstore")?
            .current_dir(&setup.project_path1)
            .args(&[
                "prove-archive-size",
                &setup.store_id1,
                &setup.root_hash1,
                &setup.archive_size1.to_string(),
            ])
            .assert()
            .success()
            .get_output();

        proofs.push(String::from_utf8(proof_output.stdout)?.trim().to_string());
    }

    // All proofs should be identical (deterministic)
    let first_proof = &proofs[0];
    for (i, proof) in proofs.iter().enumerate().skip(1) {
        assert_eq!(
            proof, first_proof,
            "Proof {} differs from first proof - not deterministic",
            i
        );
    }

    println!(
        "✓ All {} proof generations produced identical results",
        proofs.len()
    );

    Ok(())
}

/// Test proof uniqueness across different archives
#[test]
fn test_proof_uniqueness() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proofs for both repositories
    let proof1_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof2_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path2)
        .args(&[
            "prove-archive-size",
            &setup.store_id2,
            &setup.root_hash2,
            &setup.archive_size2.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof1 = String::from_utf8(proof1_output.stdout)?.trim().to_string();
    let proof2 = String::from_utf8(proof2_output.stdout)?.trim().to_string();

    // Proofs for different archives should be different
    assert_ne!(
        proof1, proof2,
        "Proofs for different archives should be unique"
    );

    // Should have different lengths or content
    if proof1.len() == proof2.len() {
        let different_chars = proof1
            .chars()
            .zip(proof2.chars())
            .filter(|(a, b)| a != b)
            .count();

        assert!(
            different_chars > 10,
            "Proofs are too similar - only {} characters differ",
            different_chars
        );
    }

    println!("✓ Proofs for different archives are unique");

    Ok(())
}

/// Test resistance to replay attacks
#[test]
fn test_replay_attack_resistance() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proof for repository 1
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success()
        .get_output();

    let proof_hex = String::from_utf8(proof_output.stdout)?.trim().to_string();

    // Proof should verify correctly for the original parameters
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "verify-archive-size",
            &proof_hex,
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
        ])
        .assert()
        .success();

    // But should fail when "replayed" with different parameters
    let replay_attempts = vec![
        // Different store ID
        (&setup.store_id2, &setup.root_hash1, setup.archive_size1),
        // Different root hash
        (&setup.store_id1, &setup.root_hash2, setup.archive_size1),
        // Different size
        (&setup.store_id1, &setup.root_hash1, setup.archive_size2),
        // All different
        (&setup.store_id2, &setup.root_hash2, setup.archive_size2),
    ];

    for (i, (store_id, root_hash, size)) in replay_attempts.iter().enumerate() {
        Command::cargo_bin("digstore")?
            .current_dir(&setup.project_path1)
            .args(&[
                "verify-archive-size",
                &proof_hex,
                store_id,
                root_hash,
                &size.to_string(),
            ])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("Verification failed")
                    .or(predicate::str::contains("mismatch")),
            );

        println!("✓ Replay attack attempt {} correctly failed", i + 1);
    }

    Ok(())
}

/// Test cryptographic integrity of proof components
#[test]
fn test_cryptographic_integrity() -> anyhow::Result<()> {
    let setup = SecurityTestSetup::new()?;

    // Generate proof with verbose output to see internal hashes
    let proof_output = Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "prove-archive-size",
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
            "--verbose",
        ])
        .assert()
        .success()
        .get_output();

    let output_str = String::from_utf8(proof_output.stdout)?;

    // Verify that cryptographic components are present
    assert!(
        output_str.contains("Merkle tree"),
        "Should mention merkle tree construction"
    );
    assert!(
        output_str.contains("Layer count"),
        "Should show layer count"
    );
    assert!(
        output_str.contains("Total size"),
        "Should show total size calculation"
    );

    // Extract the actual proof (last line should be hex)
    let proof_hex = output_str
        .lines()
        .last()
        .ok_or_else(|| anyhow::anyhow!("No proof hex found"))?
        .trim();

    // Verify proof is properly formatted hex
    assert!(proof_hex.len() > 100, "Proof should be substantial");
    assert!(
        proof_hex.chars().all(|c| c.is_ascii_hexdigit()),
        "Proof should be hex"
    );
    assert!(proof_hex.len() % 2 == 0, "Proof should have even length");

    // Verify the proof can be verified
    Command::cargo_bin("digstore")?
        .current_dir(&setup.project_path1)
        .args(&[
            "verify-archive-size",
            proof_hex,
            &setup.store_id1,
            &setup.root_hash1,
            &setup.archive_size1.to_string(),
            "--verbose",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "✓ Archive size proof verified successfully",
        ));

    Ok(())
}
