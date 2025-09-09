//! Performance benchmarks for Digstore operations
//!
//! These tests measure performance and ensure operations complete within acceptable timeframes.

use digstore_min::storage::{BatchProcessor, Store};
use std::fs;
use std::time::Instant;
use tempfile::TempDir;

#[test]
fn test_large_file_processing_performance() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create a 5MB test file
    let large_file_path = project_path.join("large_test.bin");
    let mut file = fs::File::create(&large_file_path)?;

    // Write 5MB of test data
    use std::io::Write;
    let chunk_data = vec![0xAB; 1024]; // 1KB chunks
    for i in 0..5120 {
        let mut data = chunk_data.clone();
        data[0] = (i % 256) as u8; // Make each chunk slightly different
        file.write_all(&data)?;
    }
    file.flush()?;

    let mut store = Store::init(temp_dir.path())?;

    // Measure processing time
    let start_time = Instant::now();
    store.add_file(&large_file_path)?;
    let processing_time = start_time.elapsed();

    // Verify the file was added
    assert!(store.is_file_staged(&large_file_path));

    // Processing should be reasonable (< 10 seconds for 5MB)
    assert!(
        processing_time.as_secs() < 10,
        "Large file processing should complete quickly: {:?}",
        processing_time
    );

    // Commit the file
    let commit_start = Instant::now();
    let commit_id = store.commit("Add large file")?;
    let commit_time = commit_start.elapsed();

    println!("✅ Large file (5MB) performance test:");
    println!("   Processing time: {:?}", processing_time);
    println!("   Commit time: {:?}", commit_time);

    Ok(())
}

#[test]
fn test_many_small_files_performance() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path();

    // Create many small files
    let file_count = 200;
    for i in 0..file_count {
        let file_path = project_path.join(format!("small_file_{:03}.txt", i));
        let content = format!("Small file content {}", i);
        fs::write(&file_path, content)?;
    }

    let mut store = Store::init(temp_dir.path())?;

    // Measure batch processing performance
    let start_time = Instant::now();
    store.add_directory(project_path, true)?;
    let add_time = start_time.elapsed();

    let commit_start = Instant::now();
    let commit_id = store.commit("Add many small files")?;
    let commit_time = commit_start.elapsed();

    let total_time = start_time.elapsed();
    let files_per_second = file_count as f64 / total_time.as_secs_f64();

    println!("✅ Many small files performance test:");
    println!("   Files processed: {}", file_count);
    println!("   Add time: {:?}", add_time);
    println!("   Commit time: {:?}", commit_time);
    println!("   Total time: {:?}", total_time);
    println!("   Files/second: {:.1}", files_per_second);

    // Should process at reasonable speed (>20 files/second)
    assert!(
        files_per_second > 20.0,
        "Should process >20 files/second, got {:.1}",
        files_per_second
    );
    assert!(
        total_time.as_secs() < 30,
        "Should complete in <30 seconds: {:?}",
        total_time
    );

    Ok(())
}

#[test]
fn test_batch_processor_performance() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;

    // Create test files
    let file_count = 100;
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

    // Performance requirements
    assert!(
        files_per_second > 50.0,
        "Batch processor should handle >50 files/second, got {:.1}",
        files_per_second
    );

    Ok(())
}
