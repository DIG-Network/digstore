//! Proof generation and verification tests

use digstore_min::{
    storage::store::Store,
    proofs::{ProofGenerator, Proof, ProofTarget},
    core::{types::*, hash::*}
};
use tempfile::TempDir;
use std::path::Path;
use anyhow::Result;

#[test]
fn test_file_proof_generation() -> Result<()> {
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
        },
        _ => panic!("Expected File target"),
    }

    assert_eq!(proof.metadata.store_id, store.store_id());
    assert!(proof.verify()?);

    Ok(())
}

#[test]
fn test_byte_range_proof_generation() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    let test_content = b"This is a longer test file for byte range proof generation. It has enough content to make byte ranges meaningful.";
    std::fs::write(temp_dir.path().join("long_file.txt"), test_content)?;
    
    store.add_file(Path::new("long_file.txt"))?;
    let commit_id = store.commit("Add long file for byte range proof")?;

    // Generate byte range proof
    let proof_generator = ProofGenerator::new(&store);
    let proof = proof_generator.prove_byte_range(
        Path::new("long_file.txt"), 
        0, 
        50, 
        None
    )?;

    // Verify proof properties
    assert_eq!(proof.version, "1.0");
    assert_eq!(proof.proof_type, "byte_range");
    
    match &proof.target {
        ProofTarget::ByteRange { path, start, end, at } => {
            assert_eq!(path, &Path::new("long_file.txt"));
            assert_eq!(*start, 0);
            assert_eq!(*end, 50);
            assert_eq!(at, &Some(commit_id));
        },
        _ => panic!("Expected ByteRange target"),
    }

    assert!(proof.verify()?);

    Ok(())
}

#[test]
fn test_layer_proof_generation() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit files to create a layer
    std::fs::write(temp_dir.path().join("file1.txt"), b"File 1")?;
    std::fs::write(temp_dir.path().join("file2.txt"), b"File 2")?;
    
    store.add_file(Path::new("file1.txt"))?;
    store.add_file(Path::new("file2.txt"))?;
    let layer_id = store.commit("Create layer for proof")?;

    // Generate layer proof
    let proof_generator = ProofGenerator::new(&store);
    let proof = proof_generator.prove_layer(layer_id)?;

    // Verify proof properties
    assert_eq!(proof.version, "1.0");
    assert_eq!(proof.proof_type, "layer");
    
    match &proof.target {
        ProofTarget::Layer { layer_id: target_layer } => {
            assert_eq!(*target_layer, layer_id);
        },
        _ => panic!("Expected Layer target"),
    }

    assert_eq!(proof.root, layer_id);
    assert!(proof.verify()?);

    Ok(())
}

#[test]
fn test_proof_for_nonexistent_file() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let store = Store::init(temp_dir.path())?;

    let proof_generator = ProofGenerator::new(&store);
    let result = proof_generator.prove_file(Path::new("nonexistent.txt"), None);
    
    assert!(result.is_err());
    
    Ok(())
}

#[test]
fn test_proof_json_serialization() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    std::fs::write(temp_dir.path().join("serialize_test.txt"), b"Serialization test")?;
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
fn test_proof_at_specific_commit() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create first commit
    std::fs::write(temp_dir.path().join("versioned.txt"), b"Version 1")?;
    store.add_file(Path::new("versioned.txt"))?;
    let commit1 = store.commit("First version")?;

    // Create second commit
    std::fs::write(temp_dir.path().join("versioned.txt"), b"Version 2 - updated content")?;
    store.add_file(Path::new("versioned.txt"))?;
    let commit2 = store.commit("Second version")?;

    // Generate proofs for both versions
    let proof_generator = ProofGenerator::new(&store);
    
    let proof1 = proof_generator.prove_file(Path::new("versioned.txt"), Some(commit1))?;
    let proof2 = proof_generator.prove_file(Path::new("versioned.txt"), Some(commit2))?;

    // Proofs should be different (different roots)
    assert_ne!(proof1.root, proof2.root);
    
    // Both should verify
    assert!(proof1.verify()?);
    assert!(proof2.verify()?);

    Ok(())
}

#[test]
fn test_multiple_file_proofs() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create multiple files
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

#[test]
fn test_proof_metadata() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create and commit a test file
    std::fs::write(temp_dir.path().join("metadata_test.txt"), b"Metadata test")?;
    store.add_file(Path::new("metadata_test.txt"))?;
    store.commit("Add file for metadata test")?;

    // Generate proof
    let proof_generator = ProofGenerator::new(&store);
    let proof = proof_generator.prove_file(Path::new("metadata_test.txt"), None)?;

    // Check metadata
    assert_eq!(proof.metadata.store_id, store.store_id());
    assert!(proof.metadata.layer_number.is_some());
    assert!(proof.metadata.timestamp > 0);

    Ok(())
}

#[test]
fn test_proof_generator_lifecycle() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path())?;

    // Create files and commits
    std::fs::write(temp_dir.path().join("lifecycle.txt"), b"Lifecycle test")?;
    store.add_file(Path::new("lifecycle.txt"))?;
    let commit_id = store.commit("Lifecycle test commit")?;

    // Create proof generator
    let proof_generator = ProofGenerator::new(&store);

    // Test all proof types
    let file_proof = proof_generator.prove_file(Path::new("lifecycle.txt"), None)?;
    let byte_range_proof = proof_generator.prove_byte_range(
        Path::new("lifecycle.txt"), 0, 10, None
    )?;
    let layer_proof = proof_generator.prove_layer(commit_id)?;

    // All should verify
    assert!(file_proof.verify()?);
    assert!(byte_range_proof.verify()?);
    assert!(layer_proof.verify()?);

    Ok(())
}
