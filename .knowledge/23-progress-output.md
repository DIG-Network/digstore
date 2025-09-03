# Progress Output and Compression Specification

## Overview

Digstore Min should provide detailed progress output during operations, similar to Git, showing compression statistics and operation progress to give users visibility into what's happening.

## Progress Output Requirements

### 1. Commit Operations

When creating a new layer (`digstore commit`), show:

```
Enumerating objects: 1234, done.
Counting objects: 100% (1234/1234), done.
Delta compression using up to 8 threads
Compressing objects: 100% (987/987), done.
Writing layer: 100% (1234/1234), 45.67 MiB | 23.45 MiB/s, done.
Total 1234 (delta 234), reused 567 (delta 123), pack-reused 0
Layer created: a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2
```

### 2. Add Operations

When staging files (`digstore add`):

```
Scanning files: 100% (456/456), done.
Computing hashes: 100% (456/456), done.
Analyzing chunks: 100% (2341/2341), done.
Deduplication: 234 chunks saved (12.3 MiB)
Staged 456 files (123.4 MiB)
```

### 3. Retrieval Operations

When retrieving data (`digstore get`):

```
Reading layer index: done.
Decompressing chunks: 100% (234/234), done.
Reconstructing file: 100% (45.6 MiB/45.6 MiB), done.
Verification: OK (SHA-256: a3f5c8d9...)
```

## Compression Implementation

### Per-Chunk Compression

Each chunk in the data section should be compressed individually:

1. **Default Algorithm**: Zstd level 3 (good balance of speed and ratio)
2. **Adaptive Compression**: Skip compression if chunk doesn't compress well
3. **Parallel Compression**: Use multiple threads for large operations

### Compression Flow

```rust
pub struct ChunkCompressor {
    algorithm: CompressionType,
    level: i32,
    min_ratio: f32, // Minimum compression ratio to keep compressed
}

impl ChunkCompressor {
    pub fn compress_chunk(&self, data: &[u8]) -> CompressedChunk {
        match self.algorithm {
            CompressionType::None => CompressedChunk::Uncompressed(data.to_vec()),
            CompressionType::Zstd => {
                let compressed = zstd::encode_all(data, self.level).unwrap();
                
                // Only use compression if it saves space
                let ratio = compressed.len() as f32 / data.len() as f32;
                if ratio < self.min_ratio {
                    CompressedChunk::Compressed {
                        algorithm: CompressionType::Zstd,
                        data: compressed,
                        original_size: data.len(),
                    }
                } else {
                    CompressedChunk::Uncompressed(data.to_vec())
                }
            }
            CompressionType::Lz4 => {
                // Similar logic for LZ4
            }
        }
    }
}
```

### Progress Reporting

Use the `indicatif` crate for progress bars:

```rust
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};

pub struct ProgressReporter {
    multi: MultiProgress,
    compression_bar: ProgressBar,
    writing_bar: ProgressBar,
}

impl ProgressReporter {
    pub fn new() -> Self {
        let multi = MultiProgress::new();
        
        let compression_bar = multi.add(ProgressBar::new(0));
        compression_bar.set_style(
            ProgressStyle::default_bar()
                .template("Compressing objects: {percent}% ({pos}/{len}), done.")
                .progress_chars("=>-")
        );
        
        let writing_bar = multi.add(ProgressBar::new(0));
        writing_bar.set_style(
            ProgressStyle::default_bar()
                .template("Writing layer: {percent}% ({pos}/{len}), {bytes}/{total_bytes} | {bytes_per_sec}, done.")
                .progress_chars("=>-")
        );
        
        Self { multi, compression_bar, writing_bar }
    }
    
    pub fn start_compression(&self, total_chunks: u64) {
        self.compression_bar.set_length(total_chunks);
        println!("Delta compression using up to {} threads", num_cpus::get());
    }
    
    pub fn update_compression(&self, completed: u64) {
        self.compression_bar.set_position(completed);
    }
    
    pub fn finish_compression(&self) {
        self.compression_bar.finish();
    }
}
```

## Compression Statistics

Track and report compression statistics:

```rust
pub struct CompressionStats {
    pub total_chunks: usize,
    pub compressed_chunks: usize,
    pub original_size: u64,
    pub compressed_size: u64,
    pub compression_time: Duration,
}

impl CompressionStats {
    pub fn report(&self) {
        let ratio = 1.0 - (self.compressed_size as f64 / self.original_size as f64);
        let saved = self.original_size - self.compressed_size;
        
        println!("Compression statistics:");
        println!("  Total chunks: {}", self.total_chunks);
        println!("  Compressed: {} ({:.1}%)", 
            self.compressed_chunks, 
            self.compressed_chunks as f64 / self.total_chunks as f64 * 100.0
        );
        println!("  Original size: {}", format_bytes(self.original_size));
        println!("  Compressed size: {}", format_bytes(self.compressed_size));
        println!("  Space saved: {} ({:.1}%)", format_bytes(saved), ratio * 100.0);
        println!("  Compression time: {:.2}s", self.compression_time.as_secs_f64());
    }
}
```

## Configuration Options

Allow users to configure compression:

```toml
# ~/.dig/config.toml
[core]
compression = "zstd"           # none, zstd, lz4
compression_level = 3          # 1-22 for zstd, 1-16 for lz4
compression_threads = 0        # 0 = auto-detect
min_compression_ratio = 0.9    # Only compress if saves >10%

[ui]
progress = true               # Show progress bars
verbose = false              # Extra detailed output
color = "auto"               # auto, always, never
```

## CLI Flags

Add compression-related CLI flags:

```bash
# Override compression for single operation
digstore commit -m "message" --compression=lz4 --compression-level=9

# Disable compression
digstore commit -m "message" --no-compression

# Verbose progress
digstore add -r . --verbose
```

## Performance Considerations

1. **Parallel Compression**: Use rayon for parallel chunk compression
2. **Streaming**: Compress chunks as they're written, not all at once
3. **Memory Usage**: Limit concurrent compressions based on available RAM
4. **Adaptive Strategy**: 
   - Small files: Fast compression (LZ4)
   - Large files: Better compression (Zstd)
   - Already compressed: Skip compression

## Example Implementation

```rust
use indicatif::{ProgressBar, MultiProgress};
use rayon::prelude::*;

pub fn commit_with_progress(files: Vec<FileEntry>) -> Result<LayerId> {
    let mp = MultiProgress::new();
    
    // Enumeration phase
    let enum_pb = mp.add(ProgressBar::new(files.len() as u64));
    enum_pb.set_message("Enumerating objects");
    
    for (i, file) in files.iter().enumerate() {
        // Process file
        enum_pb.set_position(i as u64);
    }
    enum_pb.finish_with_message("done");
    
    // Counting phase
    println!("Counting objects: 100% ({}/{}), done.", files.len(), files.len());
    
    // Compression phase
    println!("Delta compression using up to {} threads", rayon::current_num_threads());
    let compress_pb = mp.add(ProgressBar::new(total_chunks as u64));
    compress_pb.set_message("Compressing objects");
    
    let compressed_chunks: Vec<_> = chunks
        .par_iter()
        .map(|chunk| {
            let result = compress_chunk(chunk);
            compress_pb.inc(1);
            result
        })
        .collect();
    
    compress_pb.finish_with_message("done");
    
    // Writing phase
    let write_pb = mp.add(ProgressBar::new(total_size));
    write_pb.set_message("Writing layer");
    
    // Write compressed data with progress updates
    
    // Final summary
    println!("Total {} (delta {}), reused {} (delta {}), pack-reused 0",
        total_objects, delta_objects, reused_objects, reused_deltas);
    println!("Layer created: {}", layer_id);
    
    Ok(layer_id)
}
```

## Testing Progress Output

Create tests that capture and verify progress output:

```rust
#[test]
fn test_compression_progress() {
    let output = capture_stdout(|| {
        create_layer_with_progress(test_files);
    });
    
    assert!(output.contains("Compressing objects: 100%"));
    assert!(output.contains("Delta compression using"));
    assert!(output.contains("Layer created:"));
}
```

## Summary

By implementing Git-like progress output and proper compression:

1. Users get clear visibility into operations
2. Large operations don't appear frozen
3. Compression reduces storage and transfer sizes
4. Statistics help users understand space savings
5. Configurable compression allows optimization for different use cases
