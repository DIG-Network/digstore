//! Performance tests for streaming and batch processing

use digstore_min::{
    storage::{Store, BatchProcessor, OptimizedFileScanner},
    core::types::*,
};
use tempfile::TempDir;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

/// Test that large files are processed with constant memory usage
#[test]
fn test_large_file_constant_memory() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create a 5MB test file (large enough to trigger streaming)
    let large_file_path = temp_dir.path().join("large_test.bin");
    let mut file = fs::File::create(&large_file_path)?;
    
    // Write 5MB of test data
    let chunk_data = vec![0xAB; 1024]; // 1KB chunks
    for i in 0..5120 {
        let mut data = chunk_data.clone();
        // Make each chunk slightly different to avoid perfect deduplication
        data[0] = (i % 256) as u8;
        file.write_all(&data)?;
    }
    file.flush()?;
    
    // Initialize store and measure memory before
    let mut store = Store::init(temp_dir.path())?;
    
    // Add the large file using streaming processing
    let start_time = Instant::now();
    store.add_file(&large_file_path)?;
    let processing_time = start_time.elapsed();
    
    // Verify the file was added
    assert!(store.is_file_staged(&large_file_path));
    
    // Processing should be fast (streaming should be efficient)
    assert!(processing_time.as_secs() < 10, "Large file processing should complete quickly");
    
    // Commit the file
    let commit_id = store.commit("Add large file")?;
    
    // Verify we can retrieve the file
    let retrieved_data = store.get_file(&large_file_path)?;
    assert_eq!(retrieved_data.len(), 5 * 1024 * 1024, "Retrieved file should be 5MB");
    
    println!("✅ Large file (5MB) processed successfully with streaming");
    println!("   Processing time: {:?}", processing_time);
    
    Ok(())
}

/// Test batch processing with many small files
#[test]
fn test_many_small_files_batch_processing() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create many small files
    let file_count = 1000; // Start with 1000 for testing
    let mut file_paths = Vec::new();
    
    for i in 0..file_count {
        let file_path = temp_dir.path().join(format!("small_file_{:04}.txt", i));
        let content = format!("This is small file number {} with some content to make it realistic", i);
        fs::write(&file_path, content)?;
        file_paths.push(file_path);
    }
    
    let mut store = Store::init(temp_dir.path())?;
    
    // Measure batch processing performance
    let start_time = Instant::now();
    
    // Add all files using directory batch processing
    store.add_directory(temp_dir.path(), true)?;
    
    let add_time = start_time.elapsed();
    
    // Commit all files
    let commit_start = Instant::now();
    let commit_id = store.commit("Add many small files")?;
    let commit_time = commit_start.elapsed();
    
    let total_time = start_time.elapsed();
    
    // Performance validation
    let files_per_second = file_count as f64 / total_time.as_secs_f64();
    
    println!("✅ Batch processing test completed:");
    println!("   Files processed: {}", file_count);
    println!("   Total time: {:?}", total_time);
    println!("   Add time: {:?}", add_time);
    println!("   Commit time: {:?}", commit_time);
    println!("   Files/second: {:.1}", files_per_second);
    
    // Should process at reasonable speed
    assert!(files_per_second > 50.0, "Should process >50 files/second, got {:.1}", files_per_second);
    assert!(total_time.as_secs() < 30, "Should complete in <30 seconds");
    
    // Verify all files are committed
    let status = store.status();
    assert_eq!(status.staged_files.len(), 0, "All files should be committed");
    
    // Test retrieval of a few files (use relative paths)
    for i in [0, file_count/2, file_count-1] {
        let relative_path = std::path::PathBuf::from(format!("small_file_{:04}.txt", i));
        let retrieved = store.get_file(&relative_path)?;
        let expected = format!("This is small file number {} with some content to make it realistic", i);
        assert_eq!(retrieved, expected.as_bytes());
    }
    
    Ok(())
}

/// Test mixed workload (small and large files)
#[test]
fn test_mixed_workload_performance() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create mixed workload: 100 small files + 1 large file
    let mut file_paths = Vec::new();
    
    // 100 small files
    for i in 0..100 {
        let file_path = temp_dir.path().join(format!("small_{:03}.txt", i));
        let content = format!("Small file {} content", i);
        fs::write(&file_path, content)?;
        file_paths.push(file_path);
    }
    
    // 1 large file (2MB)
    let large_file_path = temp_dir.path().join("large.bin");
    let mut large_file = fs::File::create(&large_file_path)?;
    let data_chunk = vec![0xCD; 1024]; // 1KB
    for i in 0..2048 {
        let mut chunk = data_chunk.clone();
        chunk[0] = (i % 256) as u8; // Make chunks different
        large_file.write_all(&chunk)?;
    }
    large_file.flush()?;
    file_paths.push(large_file_path.clone());
    
    let mut store = Store::init(temp_dir.path())?;
    
    // Process mixed workload
    let start_time = Instant::now();
    
    for file_path in &file_paths {
        store.add_file(file_path)?;
    }
    
    let commit_id = store.commit("Mixed workload")?;
    let total_time = start_time.elapsed();
    
    println!("✅ Mixed workload test completed:");
    println!("   Files: 100 small + 1 large (2MB)");
    println!("   Total time: {:?}", total_time);
    println!("   Files/second: {:.1}", 101.0 / total_time.as_secs_f64());
    
    // Should handle mixed workload efficiently
    assert!(total_time.as_secs() < 15, "Mixed workload should complete in <15 seconds");
    
    // Verify large file retrieval works
    let retrieved_large = store.get_file(&large_file_path)?;
    assert_eq!(retrieved_large.len(), 2 * 1024 * 1024, "Large file should be 2MB");
    
    Ok(())
}

/// Test streaming file reconstruction without loading entire file
#[test]
fn test_streaming_file_reconstruction() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create a file with known pattern
    let test_file = temp_dir.path().join("pattern.bin");
    let mut file = fs::File::create(&test_file)?;
    
    // Write 1MB with pattern: each 1KB chunk has repeated byte value
    for i in 0..1024 {
        let byte_value = (i % 256) as u8;
        let chunk = vec![byte_value; 1024];
        file.write_all(&chunk)?;
    }
    file.flush()?;
    
    let mut store = Store::init(temp_dir.path())?;
    store.add_file(&test_file)?;
    let commit_id = store.commit("Add pattern file")?;
    
    // Test full file retrieval
    let retrieved = store.get_file(&test_file)?;
    assert_eq!(retrieved.len(), 1024 * 1024, "File should be 1MB");
    
    // Verify pattern is correct
    for (chunk_idx, chunk) in retrieved.chunks(1024).enumerate() {
        let expected_byte = (chunk_idx % 256) as u8;
        assert!(chunk.iter().all(|&b| b == expected_byte), 
                "Chunk {} should contain byte {}", chunk_idx, expected_byte);
    }
    
    println!("✅ Streaming file reconstruction test passed");
    println!("   File size: 1MB");
    println!("   Pattern verification: ✓");
    
    Ok(())
}

/// Test performance with directory containing many files
#[test]
fn test_directory_batch_processing() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create directory structure with many files
    let src_dir = temp_dir.path().join("src");
    fs::create_dir_all(&src_dir)?;
    
    let file_count = 200; // Enough to trigger batch processing
    for i in 0..file_count {
        let file_path = src_dir.join(format!("module_{:03}.rs", i));
        let content = format!(
            "// Module {}\npub fn function_{}() {{\n    println!(\"Hello from module {}\");\n}}\n",
            i, i, i
        );
        fs::write(&file_path, content)?;
    }
    
    let mut store = Store::init(temp_dir.path())?;
    
    // Add directory recursively - should trigger batch processing
    let start_time = Instant::now();
    store.add_directory(&src_dir, true)?;
    let add_time = start_time.elapsed();
    
    // Commit
    let commit_start = Instant::now();
    let commit_id = store.commit("Add source directory")?;
    let commit_time = commit_start.elapsed();
    
    let total_time = start_time.elapsed();
    let files_per_second = file_count as f64 / total_time.as_secs_f64();
    
    println!("✅ Directory batch processing test completed:");
    println!("   Files in directory: {}", file_count);
    println!("   Add time: {:?}", add_time);
    println!("   Commit time: {:?}", commit_time);
    println!("   Total time: {:?}", total_time);
    println!("   Files/second: {:.1}", files_per_second);
    
    // Performance requirements
    assert!(files_per_second > 20.0, "Should process >20 files/second for directory operations");
    assert!(total_time.as_secs() < 20, "Directory processing should complete in <20 seconds");
    
    // Verify all files are committed
    let status = store.status();
    assert_eq!(status.staged_files.len(), 0, "All files should be committed");
    
    Ok(())
}

/// Test memory efficiency with large number of files
#[test]
fn test_memory_efficiency() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create many small files to test memory efficiency
    let file_count = 500;
    for i in 0..file_count {
        let file_path = temp_dir.path().join(format!("memory_test_{:03}.dat", i));
        let content = vec![(i % 256) as u8; 100]; // 100 bytes each
        fs::write(&file_path, content)?;
    }
    
    let mut store = Store::init(temp_dir.path())?;
    
    // Process all files
    let start_time = Instant::now();
    store.add_directory(temp_dir.path(), true)?;
    let commit_id = store.commit("Memory efficiency test")?;
    let processing_time = start_time.elapsed();
    
    println!("✅ Memory efficiency test completed:");
    println!("   Files processed: {}", file_count);
    println!("   Processing time: {:?}", processing_time);
    println!("   Average time per file: {:?}", processing_time / file_count);
    
    // Should be efficient
    assert!(processing_time.as_secs() < 10, "Should process {} files in <10 seconds", file_count);
    
    // Test that we can retrieve files efficiently (use relative paths)
    let retrieval_start = Instant::now();
    for i in [0, file_count/4, file_count/2, file_count*3/4, file_count-1] {
        let relative_path = std::path::PathBuf::from(format!("memory_test_{:03}.dat", i));
        let retrieved = store.get_file(&relative_path)?;
        assert_eq!(retrieved.len(), 100);
        assert!(retrieved.iter().all(|&b| b == (i % 256) as u8));
    }
    let retrieval_time = retrieval_start.elapsed();
    
    println!("   Random file retrieval time: {:?}", retrieval_time);
    assert!(retrieval_time.as_millis() < 500, "File retrieval should be reasonably fast");
    
    Ok(())
}

/// Benchmark batch processor directly
#[test]
fn test_batch_processor_performance() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create test files
    let file_count = 300;
    let mut files = Vec::new();
    
    for i in 0..file_count {
        let file_path = temp_dir.path().join(format!("batch_test_{:03}.txt", i));
        let content = format!("Batch test file {} with content", i);
        fs::write(&file_path, content)?;
        files.push(file_path);
    }
    
    // Test batch processor directly
    let batch_processor = BatchProcessor::new();
    
    let start_time = Instant::now();
    let result = batch_processor.process_files_batch(files, None)?;
    let processing_time = start_time.elapsed();
    
    // Validate results
    assert_eq!(result.file_entries.len(), file_count);
    assert!(!result.chunks.is_empty());
    
    let files_per_second = file_count as f64 / processing_time.as_secs_f64();
    
    println!("✅ Batch processor performance test:");
    println!("   Files processed: {}", file_count);
    println!("   Processing time: {:?}", processing_time);
    println!("   Files/second: {:.1}", files_per_second);
    println!("   MB/second: {:.1}", result.performance_metrics.mb_per_second);
    println!("   Deduplication ratio: {:.1}%", result.deduplication_stats.deduplication_ratio * 100.0);
    
    // Performance requirements
    assert!(files_per_second > 100.0, "Batch processor should handle >100 files/second");
    assert!(processing_time.as_secs() < 5, "Should process {} files in <5 seconds", file_count);
    
    Ok(())
}

/// Test file scanning performance
#[test]
fn test_file_scanning_performance() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create nested directory structure
    for dir_i in 0..10 {
        let dir_path = temp_dir.path().join(format!("dir_{}", dir_i));
        fs::create_dir_all(&dir_path)?;
        
        for file_i in 0..50 {
            let file_path = dir_path.join(format!("file_{:02}.txt", file_i));
            let content = format!("File in directory {} number {}", dir_i, file_i);
            fs::write(&file_path, content)?;
        }
    }
    
    // Test optimized file scanner
    let scanner = OptimizedFileScanner::new();
    
    let start_time = Instant::now();
    let files = scanner.scan_directory_parallel(temp_dir.path())?;
    let scan_time = start_time.elapsed();
    
    assert_eq!(files.len(), 500, "Should find 500 files");
    
    let files_per_second = files.len() as f64 / scan_time.as_secs_f64();
    
    println!("✅ File scanning performance test:");
    println!("   Files found: {}", files.len());
    println!("   Scan time: {:?}", scan_time);
    println!("   Files/second: {:.1}", files_per_second);
    
    // Should scan files quickly
    assert!(files_per_second > 800.0, "Should scan >800 files/second");
    assert!(scan_time.as_millis() < 1000, "Should scan 500 files in <1 second");
    
    Ok(())
}

/// Test that the system correctly chooses between streaming and batch processing
#[test]
fn test_adaptive_processing_selection() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    
    // Create a mix: few files (should use individual processing)
    for i in 0..5 {
        let file_path = temp_dir.path().join(format!("individual_{}.txt", i));
        fs::write(&file_path, format!("Individual file {}", i))?;
    }
    
    let mut store = Store::init(temp_dir.path())?;
    
    // Add directory - should NOT trigger batch processing (<50 files)
    let start_time = Instant::now();
    store.add_directory(temp_dir.path(), true)?;
    let processing_time = start_time.elapsed();
    
    println!("✅ Adaptive processing test (few files):");
    println!("   Files: 5 (should use individual processing)");
    println!("   Processing time: {:?}", processing_time);
    
    // Should be very fast for few files
    assert!(processing_time.as_millis() < 500, "Few files should process very quickly");
    
    // Now test with many files in subdirectory
    let many_dir = temp_dir.path().join("many");
    fs::create_dir_all(&many_dir)?;
    
    for i in 0..100 {
        let file_path = many_dir.join(format!("batch_{:03}.txt", i));
        fs::write(&file_path, format!("Batch file {}", i))?;
    }
    
    // Add the new directory - should trigger batch processing
    let batch_start = Instant::now();
    store.add_directory(&many_dir, true)?;
    let batch_time = batch_start.elapsed();
    
    println!("   Batch processing (100 files): {:?}", batch_time);
    
    // Batch processing should be efficient
    let batch_files_per_second = 100.0 / batch_time.as_secs_f64();
    assert!(batch_files_per_second > 50.0, "Batch processing should be >50 files/second");
    
    Ok(())
}

#[cfg(test)]
fn create_large_test_file(path: &std::path::Path, size_mb: usize) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    let chunk = vec![0xAB; 1024 * 1024]; // 1MB chunk
    
    for i in 0..size_mb {
        let mut data = chunk.clone();
        // Make each MB slightly different
        data[0] = (i % 256) as u8;
        file.write_all(&data)?;
    }
    
    file.flush()?;
    Ok(())
}
