//! Unit tests for content-defined chunking
//!
//! Tests the chunking engine and chunk-related functionality.

use digstore_min::{
    core::{hash::*, types::*},
    storage::chunk::{ChunkConfig, ChunkingEngine},
};
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_chunk_empty_data() {
    let engine = ChunkingEngine::new();
    let chunks = engine.chunk_data(&[]).unwrap();
    assert!(chunks.is_empty());
}

#[test]
fn test_chunk_small_data() {
    let engine = ChunkingEngine::new();
    let data = b"Hello, World! This is a small test file.";
    let chunks = engine.chunk_data(data).unwrap();

    // Small data should result in one chunk
    assert_eq!(chunks.len(), 1);

    let chunk = &chunks[0];
    assert_eq!(chunk.offset, 0);
    assert_eq!(chunk.size, data.len() as u32);
    assert_eq!(chunk.data, data);

    // Verify chunk hash
    let expected_hash = sha256(data);
    assert_eq!(chunk.hash, expected_hash);
}

#[test]
fn test_chunk_deterministic() {
    let engine = ChunkingEngine::new();
    // Create 2MB of varied data for better chunking
    let mut data = Vec::new();
    for i in 0..(2 * 1024 * 1024) {
        data.push((i % 256) as u8);
    }

    let chunks1 = engine.chunk_data(&data).unwrap();
    let chunks2 = engine.chunk_data(&data).unwrap();

    // Should be deterministic
    assert_eq!(chunks1.len(), chunks2.len());
    for (c1, c2) in chunks1.iter().zip(chunks2.iter()) {
        assert_eq!(c1.hash, c2.hash);
        assert_eq!(c1.offset, c2.offset);
        assert_eq!(c1.size, c2.size);
    }
}

#[test]
fn test_chunk_reconstruction() {
    let engine = ChunkingEngine::new();
    let original_data = b"This is a test file that will be chunked and then reconstructed.";

    let chunks = engine.chunk_data(original_data).unwrap();

    // Reconstruct the data from chunks
    let reconstructed = engine.reconstruct_from_chunks(&chunks);

    assert_eq!(reconstructed, original_data);
}

#[test]
fn test_config_validation() {
    let config = ChunkConfig::new(1024, 2048, 4096);
    assert_eq!(config.min_size, 1024);
    assert_eq!(config.avg_size, 2048);
    assert_eq!(config.max_size, 4096);
    assert!(config.is_valid());

    let small_config = ChunkConfig::small_files();
    assert!(small_config.avg_size < ChunkConfig::default().avg_size);
    assert!(small_config.is_valid());

    let large_config = ChunkConfig::large_files();
    assert!(large_config.avg_size > ChunkConfig::default().avg_size);
    assert!(large_config.is_valid());

    // Invalid config
    let invalid_config = ChunkConfig::new(2048, 1024, 4096); // min > avg
    assert!(!invalid_config.is_valid());
}
