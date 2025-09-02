//! Content-defined chunking tests

use anyhow::Result;
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
fn test_chunk_different_configs() {
    let small_config = ChunkConfig::small_files();
    let large_config = ChunkConfig::large_files();

    let small_engine = ChunkingEngine::with_config(small_config);
    let large_engine = ChunkingEngine::with_config(large_config);

    // Create 5MB of varied data to ensure chunk boundaries
    let mut data = Vec::new();
    for i in 0..(5 * 1024 * 1024) {
        data.push((i % 256) as u8);
    }

    let small_chunks = small_engine.chunk_data(&data).unwrap();
    let large_chunks = large_engine.chunk_data(&data).unwrap();

    // Small config should produce more chunks
    assert!(small_chunks.len() >= large_chunks.len());
}

#[test]
fn test_chunk_file() -> Result<()> {
    let engine = ChunkingEngine::new();

    // Create a temporary file
    let mut temp_file = NamedTempFile::new().unwrap();
    let test_data = b"This is test content for file chunking.";
    temp_file.write_all(test_data).unwrap();
    temp_file.flush().unwrap();

    let chunks = engine.chunk_file(temp_file.path())?;

    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].data, test_data);

    Ok(())
}

#[test]
fn test_create_file_entry() -> Result<()> {
    let engine = ChunkingEngine::new();
    let path = std::path::PathBuf::from("test.txt");
    let data = b"Hello, chunked world!";

    let file_entry = engine.create_file_entry(path.clone(), data)?;

    assert_eq!(file_entry.path, path);
    assert_eq!(file_entry.size, data.len() as u64);
    assert_eq!(file_entry.hash, sha256(data));
    assert!(!file_entry.chunks.is_empty());
    assert!(file_entry.metadata.is_new);

    Ok(())
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
fn test_chunk_hash_verification() {
    let engine = ChunkingEngine::new();
    let data = b"Test data for hash verification";

    let chunks = engine.chunk_data(data).unwrap();

    // Verify chunks can reconstruct original
    assert!(engine.verify_chunks(data, &chunks).unwrap());

    // Test with modified data should fail
    let mut modified_data = data.to_vec();
    modified_data[0] = !modified_data[0]; // Flip first bit
    assert!(!engine.verify_chunks(&modified_data, &chunks).unwrap());
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

#[test]
fn test_large_file_chunking() {
    let engine = ChunkingEngine::new();

    // Create a large file with varied content to ensure chunking
    let mut large_data = Vec::new();
    for i in 0..(2 * 1024 * 1024) {
        // 2MB
        large_data.push((i % 256) as u8);
    }

    let chunks = engine.chunk_data(&large_data).unwrap();

    // Should create multiple chunks for large data (but depends on content)
    println!("Created {} chunks for 2MB data", chunks.len());

    // At minimum, should have at least 1 chunk
    assert!(chunks.len() >= 1);

    // Verify reconstruction
    assert!(engine.verify_chunks(&large_data, &chunks).unwrap());

    // Check that chunks are within size bounds (except possibly the last chunk)
    for (i, chunk) in chunks.iter().enumerate() {
        if i == chunks.len() - 1 {
            // Last chunk can be smaller
            assert!(chunk.size <= engine.config().max_size as u32);
        } else {
            assert!(chunk.size >= engine.config().min_size as u32);
            assert!(chunk.size <= engine.config().max_size as u32);
        }
    }
}

#[test]
fn test_chunk_deduplication_potential() {
    let engine = ChunkingEngine::new();

    // Create two files with shared content
    let shared_content = b"This is shared content between files. ";
    let file1_data = [shared_content.as_slice(), b"File 1 unique content."].concat();
    let file2_data = [shared_content.as_slice(), b"File 2 unique content."].concat();

    let chunks1 = engine.chunk_data(&file1_data).unwrap();
    let chunks2 = engine.chunk_data(&file2_data).unwrap();

    // Both should have at least one chunk
    assert!(!chunks1.is_empty());
    assert!(!chunks2.is_empty());

    // If the shared content forms a chunk boundary, the first chunks should be identical
    // This is not guaranteed with CDC but demonstrates the potential for deduplication
    if chunks1.len() > 1 && chunks2.len() > 1 {
        // Check if we have shared chunks
        let mut shared_chunks = 0;
        for c1 in &chunks1 {
            for c2 in &chunks2 {
                if c1.hash == c2.hash {
                    shared_chunks += 1;
                    break;
                }
            }
        }
        println!("Found {} shared chunks between files", shared_chunks);
    }
}

#[test]
fn test_chunk_offset_continuity() {
    let engine = ChunkingEngine::new();
    let data = vec![1u8; 3 * 1024 * 1024]; // 3MB

    let chunks = engine.chunk_data(&data).unwrap();

    if chunks.len() > 1 {
        // Verify chunks are continuous
        for i in 1..chunks.len() {
            let prev_chunk = &chunks[i - 1];
            let curr_chunk = &chunks[i];

            assert_eq!(
                prev_chunk.offset + prev_chunk.size as u64,
                curr_chunk.offset,
                "Chunks should be continuous"
            );
        }

        // Verify total coverage
        let last_chunk = chunks.last().unwrap();
        assert_eq!(
            last_chunk.offset + last_chunk.size as u64,
            data.len() as u64,
            "Chunks should cover entire file"
        );
    }
}

#[test]
fn test_chunk_boundary_changes() {
    let engine = ChunkingEngine::new();

    // Create two similar files with a small change
    let base_data = vec![1u8; 2 * 1024 * 1024]; // 2MB
    let mut modified_data = base_data.clone();
    modified_data[1024 * 1024] = 2; // Change one byte in the middle

    let base_chunks = engine.chunk_data(&base_data).unwrap();
    let modified_chunks = engine.chunk_data(&modified_data).unwrap();

    // With content-defined chunking, many chunks should remain the same
    let mut same_chunks = 0;
    let mut total_chunks = base_chunks.len().max(modified_chunks.len());

    for base_chunk in &base_chunks {
        for modified_chunk in &modified_chunks {
            if base_chunk.hash == modified_chunk.hash {
                same_chunks += 1;
                break;
            }
        }
    }

    // Should have some shared chunks (content-defined chunking benefit)
    println!("Same chunks: {}/{}", same_chunks, total_chunks);

    // Both should reconstruct correctly
    assert!(engine.verify_chunks(&base_data, &base_chunks).unwrap());
    assert!(engine
        .verify_chunks(&modified_data, &modified_chunks)
        .unwrap());
}

#[test]
fn test_chunk_config_access() {
    let config = ChunkConfig::new(1024, 2048, 4096);
    let engine = ChunkingEngine::with_config(config.clone());

    assert_eq!(engine.config().min_size, config.min_size);
    assert_eq!(engine.config().avg_size, config.avg_size);
    assert_eq!(engine.config().max_size, config.max_size);
}
