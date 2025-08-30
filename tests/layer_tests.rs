//! Layer format tests

use digstore_min::{
    storage::layer::Layer,
    core::{types::*, hash::*}
};
use tempfile::NamedTempFile;
use std::path::PathBuf;
use anyhow::Result;

#[test]
fn test_layer_header_binary_format() -> Result<()> {
    let parent_hash = Hash::from_hex("a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2").unwrap();
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
        bytes[8], bytes[9], bytes[10], bytes[11],
        bytes[12], bytes[13], bytes[14], bytes[15]
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
fn test_layer_header_roundtrip() -> Result<()> {
    let parent_hash = sha256(b"test parent");
    let original = LayerHeader::new(LayerType::Delta, 123, parent_hash);
    
    let bytes = original.to_bytes();
    let parsed = LayerHeader::from_bytes(&bytes).unwrap();
    
    assert_eq!(parsed.magic, original.magic);
    assert_eq!(parsed.version, original.version);
    assert_eq!(parsed.layer_type, original.layer_type);
    assert_eq!(parsed.flags, original.flags);
    assert_eq!(parsed.layer_number, original.layer_number);
    assert_eq!(parsed.get_parent_hash(), original.get_parent_hash());
    
    Ok(())
}

#[test]
fn test_empty_layer_creation() -> Result<()> {
    let layer = Layer::new(LayerType::Full, 1, Hash::zero());
    
    assert_eq!(layer.header.layer_number, 1);
    assert_eq!(layer.header.get_layer_type().unwrap(), LayerType::Full);
    assert_eq!(layer.files.len(), 0);
    assert_eq!(layer.chunks.len(), 0);
    assert!(layer.verify()?);
    
    Ok(())
}

#[test]
fn test_layer_with_files() -> Result<()> {
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
    
    // Debug the verification
    println!("Header files_count: {}", layer.header.files_count);
    println!("Actual files.len(): {}", layer.files.len());
    println!("Header chunks_count: {}", layer.header.chunks_count);
    println!("Actual chunks.len(): {}", layer.chunks.len());
    
    let verify_result = layer.verify()?;
    assert!(verify_result, "Layer verification failed");
    
    Ok(())
}

#[test]
fn test_layer_json_roundtrip() -> Result<()> {
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
    assert_eq!(loaded_layer.header.get_layer_type().unwrap(), LayerType::Full);
    assert_eq!(loaded_layer.files.len(), 1);
    assert_eq!(loaded_layer.chunks.len(), 1);
    assert_eq!(loaded_layer.files[0].path, PathBuf::from("simple.txt"));
    assert_eq!(loaded_layer.chunks[0].data, b"Simple content");
    assert!(loaded_layer.verify()?);
    
    Ok(())
}

#[test]
fn test_layer_header_validation() -> Result<()> {
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
fn test_layer_header_size() {
    // Verify the header is exactly 256 bytes
    let header = LayerHeader::new(LayerType::Full, 0, Hash::zero());
    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), 256);
    
    // Verify all fields fit within 256 bytes
    assert_eq!(std::mem::size_of::<LayerHeader>(), 256);
}

#[test]
fn test_layer_types() -> Result<()> {
    let test_cases = vec![
        (LayerType::Header, 0u8),
        (LayerType::Full, 1u8),
        (LayerType::Delta, 2u8),
    ];
    
    for (layer_type, expected_byte) in test_cases {
        let header = LayerHeader::new(layer_type, 0, Hash::zero());
        assert_eq!(header.layer_type, expected_byte);
        assert_eq!(header.get_layer_type().unwrap(), layer_type);
    }
    
    Ok(())
}

#[test]
fn test_layer_compute_id() -> Result<()> {
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

#[test]
fn test_layer_parent_hash() -> Result<()> {
    let parent = sha256(b"parent layer");
    let mut header = LayerHeader::new(LayerType::Delta, 1, parent);
    
    assert_eq!(header.get_parent_hash(), parent);
    
    // Test setting parent hash
    let new_parent = sha256(b"new parent");
    header.set_parent_hash(&new_parent);
    assert_eq!(header.get_parent_hash(), new_parent);
    
    Ok(())
}

#[test]
fn test_invalid_header_size() {
    let short_bytes = vec![0u8; 100]; // Too short
    let result = LayerHeader::from_bytes(&short_bytes);
    assert!(result.is_err());
}
