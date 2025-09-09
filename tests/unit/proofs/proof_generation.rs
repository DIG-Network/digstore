//! Unit tests for proof generation
//!
//! Tests the proof generation system and verification logic.

use digstore_min::{
    core::{hash::*, types::*},
    proofs::{Proof, ProofGenerator, ProofTarget},
    storage::store::Store,
};
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_file_proof_generation() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    let test_content = b"This is a test file for proof generation.";
    std::fs::write(temp_dir.path().join("test.txt"), test_content)?;

    store.add_file(Path::new("test.txt"))?;
    let commit_id = store.commit("Add test file for proof")?;

    // Generate proof for the file
    let proof_generator = ProofGenerator::new(&store);
    let proof = proof_generator.prove_file(Path::new("test.txt"), None)?;

    // Verify proof properties
    assert_eq!(proof.version, "1.0");
    assert_eq!(proof.proof_type, "file");

    match &proof.target {
        ProofTarget::File { path, at } => {
            assert_eq!(path, &Path::new("test.txt"));
            assert_eq!(at, &Some(commit_id));
        }
        _ => panic!("Expected File target"),
    }

    assert_eq!(proof.metadata.store_id, store.store_id());
    assert!(proof.verify()?);

    Ok(())
}

#[test]
fn test_byte_range_proof_generation() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    let test_content = b"This is a longer test file for byte range proof generation. It has enough content to make byte ranges meaningful.";
    std::fs::write(temp_dir.path().join("long_file.txt"), test_content)?;

    store.add_file(Path::new("long_file.txt"))?;
    let commit_id = store.commit("Add long file for byte range proof")?;

    // Generate byte range proof
    let proof_generator = ProofGenerator::new(&store);
    let proof = proof_generator.prove_byte_range(Path::new("long_file.txt"), 0, 50, None)?;

    // Verify proof properties
    assert_eq!(proof.version, "1.0");
    assert_eq!(proof.proof_type, "byte_range");

    match &proof.target {
        ProofTarget::ByteRange { path, start, end, at } => {
            assert_eq!(path, &Path::new("long_file.txt"));
            assert_eq!(*start, 0);
            assert_eq!(*end, 50);
            assert_eq!(at, &Some(commit_id));
        }
        _ => panic!("Expected ByteRange target"),
    }

    assert!(proof.verify()?);

    Ok(())
}

#[test]
fn test_proof_json_serialization() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    std::fs::write(
        temp_dir.path().join("serialize_test.txt"),
        b"Serialization test",
    )?;
    store.add_file(Path::new("serialize_test.txt"))?;
    store.commit("Add file for serialization test")?;

    // Generate proof
    let proof_generator = ProofGenerator::new(&store);
    let original_proof = proof_generator.prove_file(Path::new("serialize_test.txt"), None)?;

    // Serialize to JSON
    let json = original_proof.to_json()?;
    assert!(json.contains("\"version\""));
    assert!(json.contains("\"proof_type\""));
    assert!(json.contains("\"file\""));

    // Deserialize from JSON
    let deserialized_proof = Proof::from_json(&json)?;

    assert_eq!(deserialized_proof.version, original_proof.version);
    assert_eq!(deserialized_proof.proof_type, original_proof.proof_type);
    assert_eq!(deserialized_proof.root, original_proof.root);

    Ok(())
}

#[test]
fn test_proof_verification() -> anyhow::Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create multiple files for a more complex proof
    std::fs::write(temp_dir.path().join("file1.txt"), b"Content 1")?;
    std::fs::write(temp_dir.path().join("file2.txt"), b"Content 2")?;
    std::fs::write(temp_dir.path().join("file3.txt"), b"Content 3")?;

    store.add_file(Path::new("file1.txt"))?;
    store.add_file(Path::new("file2.txt"))?;
    store.add_file(Path::new("file3.txt"))?;
    let commit_id = store.commit("Add multiple files")?;

    // Generate proofs for all files
    let proof_generator = ProofGenerator::new(&store);

    let proof1 = proof_generator.prove_file(Path::new("file1.txt"), None)?;
    let proof2 = proof_generator.prove_file(Path::new("file2.txt"), None)?;
    let proof3 = proof_generator.prove_file(Path::new("file3.txt"), None)?;

    // All proofs should have the same root (same commit)
    assert_eq!(proof1.root, proof2.root);
    assert_eq!(proof2.root, proof3.root);

    // All should verify
    assert!(proof1.verify()?);
    assert!(proof2.verify()?);
    assert!(proof3.verify()?);

    // Metadata should be consistent
    assert_eq!(proof1.metadata.store_id, store.store_id());
    assert_eq!(proof2.metadata.store_id, store.store_id());
    assert_eq!(proof3.metadata.store_id, store.store_id());

    Ok(())
}
