//! Unit tests for layer format and operations
//!
//! Tests layer creation, serialization, and validation.

use digstore_min::{
    core::{hash::*, types::*},
    storage::layer::Layer,
};
use std::path::PathBuf;
use tempfile::NamedTempFile;

#[test]
fn test_layer_header_binary_format() -> anyhow::Result<()> {
    let parent_hash =
        Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
    let header = LayerHeader::new(LayerType::Full, 42, parent_hash);

    // Test serialization
    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), LayerHeader::SIZE);

    // Check magic bytes
    assert_eq!(&bytes[0..4], b"DIGS");

    // Check version (little-endian)
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1);

    // Check layer type
    assert_eq!(bytes[6], LayerType::Full.to_byte());

    // Check layer number (little-endian)
    let layer_num_bytes = [
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ];
    assert_eq!(u64::from_le_bytes(layer_num_bytes), 42);

    // Test deserialization
    let parsed_header = LayerHeader::from_bytes(&bytes).unwrap();
    assert_eq!(parsed_header.magic, LayerHeader::MAGIC);
    assert_eq!(parsed_header.version, LayerHeader::VERSION);
    assert_eq!(parsed_header.layer_type, LayerType::Full.to_byte());
    assert_eq!(parsed_header.layer_number, 42);
    assert_eq!(parsed_header.get_parent_hash(), parent_hash);
    assert!(parsed_header.is_valid());

    Ok(())
}

#[test]
fn test_empty_layer_creation() -> anyhow::Result<()> {
    let layer = Layer::new(LayerType::Full, 1, Hash::zero());

    assert_eq!(layer.header.layer_number, 1);
    assert_eq!(layer.header.get_layer_type().unwrap(), LayerType::Full);
    assert_eq!(layer.files.len(), 0);
    assert_eq!(layer.chunks.len(), 0);
    assert!(layer.verify()?);

    Ok(())
}

#[test]
fn test_layer_with_files() -> anyhow::Result<()> {
    let mut layer = Layer::new(LayerType::Full, 1, Hash::zero());

    // Create a test file entry
    let file_entry = FileEntry {
        path: PathBuf::from("test.txt"),
        hash: sha256(b"Hello, World!"),
        size: 13,
        chunks: vec![ChunkRef {
            hash: sha256(b"Hello, World!"),
            offset: 0,
            size: 13,
        }],
        metadata: FileMetadata {
            mode: 0o644,
            modified: chrono::Utc::now().timestamp(),
            is_new: true,
            is_modified: false,
            is_deleted: false,
        },
    };

    // Create corresponding chunk
    let chunk = Chunk {
        hash: sha256(b"Hello, World!"),
        offset: 0,
        size: 13,
        data: b"Hello, World!".to_vec(),
    };

    layer.add_file(file_entry);
    layer.add_chunk(chunk);

    assert_eq!(layer.files.len(), 1);
    assert_eq!(layer.chunks.len(), 1);
    assert_eq!(layer.metadata.file_count, 1);
    assert_eq!(layer.metadata.total_size, 13);

    let verify_result = layer.verify()?;
    assert!(verify_result, "Layer verification failed");

    Ok(())
}

#[test]
fn test_layer_serialization_roundtrip() -> anyhow::Result<()> {
    let mut layer = Layer::new(LayerType::Full, 1, Hash::zero());

    // Add a simple file
    let file_entry = FileEntry {
        path: PathBuf::from("simple.txt"),
        hash: sha256(b"Simple content"),
        size: 14,
        chunks: vec![ChunkRef {
            hash: sha256(b"Simple content"),
            offset: 0,
            size: 14,
        }],
        metadata: FileMetadata {
            mode: 0o644,
            modified: chrono::Utc::now().timestamp(),
            is_new: true,
            is_modified: false,
            is_deleted: false,
        },
    };

    let chunk = Chunk {
        hash: sha256(b"Simple content"),
        offset: 0,
        size: 14,
        data: b"Simple content".to_vec(),
    };

    layer.add_file(file_entry);
    layer.add_chunk(chunk);

    // Write to temporary file
    let temp_file = NamedTempFile::new().unwrap();
    layer.write_to_file(temp_file.path())?;

    // Read back
    let loaded_layer = Layer::read_from_file(temp_file.path())?;

    // Verify basic properties
    assert_eq!(loaded_layer.header.layer_number, layer.header.layer_number);
    assert_eq!(
        loaded_layer.header.get_layer_type().unwrap(),
        LayerType::Full
    );
    assert_eq!(loaded_layer.files.len(), 1);
    assert_eq!(loaded_layer.chunks.len(), 1);
    assert_eq!(loaded_layer.files[0].path, PathBuf::from("simple.txt"));
    assert_eq!(loaded_layer.chunks[0].data, b"Simple content");
    assert!(loaded_layer.verify()?);

    Ok(())
}

#[test]
fn test_layer_header_validation() -> anyhow::Result<()> {
    // Test invalid magic
    let mut bytes = vec![0u8; LayerHeader::SIZE];
    bytes[0..4].copy_from_slice(b"XXXX"); // Wrong magic

    let result = LayerHeader::from_bytes(&bytes);
    assert!(result.is_ok()); // Parsing succeeds

    let header = result.unwrap();
    assert!(!header.is_valid()); // But validation fails

    // Test invalid version
    let mut bytes = vec![0u8; LayerHeader::SIZE];
    bytes[0..4].copy_from_slice(b"DIGS"); // Correct magic
    bytes[4..6].copy_from_slice(&999u16.to_le_bytes()); // Wrong version

    let header = LayerHeader::from_bytes(&bytes).unwrap();
    assert!(!header.is_valid());

    Ok(())
}

#[test]
fn test_layer_compute_id() -> anyhow::Result<()> {
    let layer1 = Layer::new(LayerType::Full, 1, Hash::zero());
    let layer2 = Layer::new(LayerType::Full, 2, Hash::zero());

    let id1 = layer1.compute_layer_id()?;
    let id2 = layer2.compute_layer_id()?;

    // Different layers should have different IDs
    assert_ne!(id1, id2);

    // Same layer should have same ID
    let id1_again = layer1.compute_layer_id()?;
    assert_eq!(id1, id1_again);

    Ok(())
}
