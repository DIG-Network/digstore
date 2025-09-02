# Implementation Guide

## Overview

This guide provides a roadmap for implementing Digstore Min, including architecture decisions, implementation order, and key algorithms.

## Implementation Phases

### Phase 1: Core Foundation
1. Basic types (StoreId, RootHash, Urn)
2. Error handling framework
3. Layer header format
4. Basic file I/O
5. Global directory management (~/.dig)
6. .digstore file handling

### Phase 2: Storage Layer
1. Chunk definition and hashing
2. Layer file format implementation
3. File chunking engine
4. Basic compression support

### Phase 3: Layer Management
1. Layer creation (full layers)
2. Layer reading and parsing
3. Repository initialization
4. Layer 0 special handling

### Phase 4: File Operations
1. File staging system
2. Commit creation
3. File reconstruction
4. Basic diff support

### Phase 5: Retrieval System
1. URN parsing with byte ranges
2. File retrieval by URN
3. Cross-layer file reconstruction
4. Streaming support

### Phase 6: Merkle Proofs
1. Merkle tree construction
2. Proof generation
3. Proof verification
4. Proof serialization

### Phase 7: Delta Optimization
1. Delta layer support
2. Chunk deduplication
3. Delta chain management
4. Storage optimization

### Phase 8: CLI Interface
1. Command parsing
2. Core commands (init, add, commit)
3. Retrieval commands (get, cat)
4. Utility commands

## Key Algorithms

### Content-Defined Chunking

```rust
fn find_chunk_boundaries(data: &[u8], min_size: usize, target_size: usize, max_size: usize) -> Vec<usize> {
    let mut boundaries = vec![0];
    let mut pos = min_size;
    
    while pos < data.len() {
        // Use rolling hash for boundary detection
        let window_start = pos.saturating_sub(WINDOW_SIZE);
        let window = &data[window_start..pos];
        let hash = rolling_hash(window);
        
        // Check if this is a boundary
        if (hash & BOUNDARY_MASK) == 0 || pos - boundaries.last().unwrap() >= max_size {
            boundaries.push(pos);
            pos += min_size;
        } else {
            pos += 1;
        }
    }
    
    if boundaries.last() != Some(&data.len()) {
        boundaries.push(data.len());
    }
    
    boundaries
}
```

### Merkle Tree Construction

```rust
fn build_merkle_tree(leaves: &[Hash]) -> MerkleTree {
    if leaves.is_empty() {
        return MerkleTree::empty();
    }
    
    let mut current_level = leaves.to_vec();
    let mut tree_levels = vec![current_level.clone()];
    
    while current_level.len() > 1 {
        let mut next_level = Vec::new();
        
        for chunk in current_level.chunks(2) {
            let hash = if chunk.len() == 2 {
                hash_pair(&chunk[0], &chunk[1])
            } else {
                chunk[0].clone()
            };
            next_level.push(hash);
        }
        
        tree_levels.push(next_level.clone());
        current_level = next_level;
    }
    
    MerkleTree {
        root: current_level[0].clone(),
        levels: tree_levels,
    }
}
```

### File Reconstruction from Layers

```rust
fn reconstruct_file(path: &Path, target_layer: &Layer, layer_chain: &[Layer]) -> Result<Vec<u8>> {
    // Get file entry from target layer
    let file_entry = target_layer.get_file(path)?;
    
    // Collect all chunks
    let mut chunks = Vec::new();
    
    for chunk_ref in &file_entry.chunks {
        // Try to find chunk in current layer
        if let Some(chunk_data) = target_layer.get_chunk(&chunk_ref.hash) {
            chunks.push((chunk_ref.offset, chunk_data));
        } else {
            // Search in parent layers
            for parent_layer in layer_chain {
                if let Some(chunk_data) = parent_layer.get_chunk(&chunk_ref.hash) {
                    chunks.push((chunk_ref.offset, chunk_data));
                    break;
                }
            }
        }
    }
    
    // Sort by offset and concatenate
    chunks.sort_by_key(|(offset, _)| *offset);
    let mut result = Vec::new();
    
    for (_, data) in chunks {
        result.extend_from_slice(&data);
    }
    
    // Verify reconstructed file hash
    let computed_hash = sha256(&result);
    if computed_hash != file_entry.hash {
        return Err(DigstoreError::IntegrityError);
    }
    
    Ok(result)
}
```

### Byte Range Extraction

```rust
fn extract_byte_range(file_data: Vec<u8>, range: &ByteRange) -> Result<Vec<u8>> {
    let file_len = file_data.len() as u64;
    
    let (start, end) = match (range.start, range.end) {
        (Some(start), Some(end)) => {
            if start > end || start >= file_len {
                return Err(DigstoreError::InvalidRange);
            }
            (start, end.min(file_len))
        }
        (Some(start), None) => {
            if start >= file_len {
                return Err(DigstoreError::InvalidRange);
            }
            (start, file_len)
        }
        (None, Some(end)) => {
            // Last 'end' bytes
            let start = file_len.saturating_sub(end);
            (start, file_len)
        }
        (None, None) => (0, file_len),
    };
    
    Ok(file_data[start as usize..end as usize].to_vec())
}
```

## Directory Management

### Global Store Directory

```rust
fn get_global_store_path() -> Result<PathBuf> {
    // Try standard locations in order
    if let Some(home) = dirs::home_dir() {
        Ok(home.join(".dig"))
    } else {
        Err(DigstoreError::NoHomeDirectory)
    }
}

fn ensure_global_directory() -> Result<PathBuf> {
    let global_path = get_global_store_path()?;
    if !global_path.exists() {
        std::fs::create_dir_all(&global_path)?;
    }
    Ok(global_path)
}
```

### Local Project Management

```rust
struct ProjectLink {
    project_path: PathBuf,
    digstore_file: DigstoreFile,
}

impl ProjectLink {
    fn init(project_path: &Path, store_id: StoreId) -> Result<Self> {
        let digstore_path = project_path.join(".digstore");
        
        if digstore_path.exists() {
            return Err(DigstoreError::AlreadyInitialized);
        }
        
        let digstore_file = DigstoreFile::create_device_agnostic(
            store_id,
            project_path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string()),
        );
        
        digstore_file.save(&digstore_path)?;
        Ok(Self { project_path, digstore_file })
    }
    
    fn load(project_path: &Path) -> Result<Self> {
        let digstore_path = project_path.join(".digstore");
        let digstore_file = DigstoreFile::load(&digstore_path)?;
        Ok(Self { project_path, digstore_file })
    }
}
```

## Data Structures

### Layer Header Layout

```rust
#[repr(C, packed)]
struct LayerHeaderRaw {
    magic: [u8; 4],           // "DIGS"
    version: u16,             // 1
    layer_type: u8,           // 0=Header, 1=Full, 2=Delta
    flags: u8,                // Bitfield
    layer_number: u64,        // Sequential
    timestamp: u64,           // Unix timestamp
    parent_hash: [u8; 32],    // Parent layer hash
    // ... rest of fields
}
```

### In-Memory Index

```rust
struct MemoryIndex {
    // File path -> (layer_hash, file_metadata)
    files: HashMap<PathBuf, (Hash, FileMetadata)>,
    
    // Chunk hash -> (layer_hash, offset, size)
    chunks: HashMap<Hash, Vec<ChunkLocation>>,
    
    // Layer hash -> layer metadata
    layers: HashMap<Hash, LayerMetadata>,
}
```

## Performance Optimizations

### 1. Memory-Mapped Files

Use memory mapping for large layer files:

```rust
use memmap2::Mmap;

struct MappedLayer {
    mmap: Mmap,
    header: LayerHeader,
    index_offset: usize,
}

impl MappedLayer {
    fn read_chunk(&self, offset: usize, size: usize) -> &[u8] {
        &self.mmap[offset..offset + size]
    }
}
```

### 2. Parallel Chunk Processing

```rust
use rayon::prelude::*;

fn parallel_chunk_files(files: Vec<PathBuf>) -> Vec<ChunkedFile> {
    files.par_iter()
        .map(|path| {
            let data = std::fs::read(path)?;
            let chunks = chunk_file(&data);
            Ok(ChunkedFile { path, chunks })
        })
        .collect()
}
```

### 3. Index Caching

```rust
struct LayerCache {
    cache: LruCache<Hash, Arc<Layer>>,
    max_size: usize,
}

impl LayerCache {
    fn get_or_load(&mut self, hash: &Hash) -> Result<Arc<Layer>> {
        if let Some(layer) = self.cache.get(hash) {
            return Ok(layer.clone());
        }
        
        let layer = Arc::new(load_layer(hash)?);
        self.cache.put(*hash, layer.clone());
        Ok(layer)
    }
}
```

## Testing Strategy

### Unit Tests
- Test each algorithm in isolation
- Property-based testing for chunking
- Roundtrip tests for serialization

### Integration Tests
- Full repository operations
- Cross-layer file reconstruction
- Proof generation and verification

### Performance Tests
- Benchmark chunking algorithms
- Measure layer creation time
- Test with large files

### Example Test

```rust
#[test]
fn test_file_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::init(temp_dir.path()).unwrap();
    
    // Add file
    let content = b"Hello, Digstore!";
    std::fs::write("test.txt", content).unwrap();
    store.add(&["test.txt"]).unwrap();
    let commit = store.commit("Test commit").unwrap();
    
    // Retrieve file
    let urn = format!("urn:dig:chia:{}/test.txt", store.store_id().as_hex());
    let retrieved = store.get_by_urn(&Urn::parse(&urn).unwrap()).unwrap();
    
    assert_eq!(retrieved, content);
}
```

## Progress Output Implementation

### Using indicatif for Progress Bars

```rust
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use std::time::Duration;

fn create_commit_with_progress(staged_files: Vec<StagedFile>) -> Result<LayerId> {
    let mp = MultiProgress::new();
    
    // Phase 1: Enumeration
    println!("Enumerating objects: {}, done.", staged_files.len());
    
    // Phase 2: Counting
    let count_pb = ProgressBar::new(staged_files.len() as u64);
    count_pb.set_style(ProgressStyle::default_bar()
        .template("Counting objects: {percent}% ({pos}/{len}), done.")
        .unwrap());
    
    for (i, file) in staged_files.iter().enumerate() {
        // Process file
        count_pb.set_position(i as u64 + 1);
    }
    count_pb.finish();
    
    // Phase 3: Compression
    println!("Delta compression using up to {} threads", num_cpus::get());
    
    let compress_pb = mp.add(ProgressBar::new(total_chunks));
    compress_pb.set_style(ProgressStyle::default_bar()
        .template("Compressing objects: {percent}% ({pos}/{len}), done.")
        .unwrap());
    
    // Compress chunks in parallel
    let compressed = chunks.par_iter()
        .map(|chunk| {
            let result = compress_chunk(chunk);
            compress_pb.inc(1);
            result
        })
        .collect::<Vec<_>>();
    
    compress_pb.finish();
    
    // Phase 4: Writing
    let write_pb = mp.add(ProgressBar::new(total_bytes));
    write_pb.set_style(ProgressStyle::default_bar()
        .template("Writing layer: {percent}% ({pos}/{len}), {bytes}/{total_bytes} | {bytes_per_sec}, done.")
        .unwrap());
    
    // Write with progress
    write_layer_with_progress(compressed, &write_pb)?;
    
    // Summary
    println!("Total {} (delta {}), reused {} (delta {}), pack-reused 0",
        total_objects, delta_objects, reused_objects, reused_deltas);
    
    Ok(layer_id)
}
```

## Common Pitfalls

1. **Endianness**: Always use little-endian for serialization
2. **Path Handling**: Normalize paths to forward slashes
3. **Hash Consistency**: Use binary format internally, hex for display
4. **Memory Usage**: Stream large files instead of loading entirely
5. **Error Handling**: Preserve error context through the stack
6. **Progress Updates**: Don't update progress too frequently (throttle to ~60Hz)
7. **Terminal Detection**: Disable progress bars when output is piped

## Debugging Tools

### Layer Inspector

```rust
fn inspect_layer(path: &Path) -> Result<()> {
    let data = std::fs::read(path)?;
    let header = LayerHeader::from_bytes(&data[..256])?;
    
    println!("Layer Type: {:?}", header.layer_type);
    println!("Layer Number: {}", header.layer_number);
    println!("Parent Hash: {}", hex::encode(header.parent_hash));
    println!("Files Count: {}", header.files_count);
    // ... more fields
    
    Ok(())
}
```

### Merkle Tree Visualizer

```rust
fn print_merkle_tree(tree: &MerkleTree) {
    for (level, nodes) in tree.levels.iter().enumerate() {
        println!("Level {}: {} nodes", level, nodes.len());
        for (i, node) in nodes.iter().enumerate() {
            println!("  [{}] {}", i, hex::encode(&node[..8]));
        }
    }
}
```

## Future Enhancements

1. **Parallel Layer Creation**: Process files concurrently
2. **Smart Chunking**: Content-aware boundaries
3. **Compression Dictionary**: Shared dictionary for better ratios
4. **Incremental Hashing**: Update hashes without full recomputation
5. **Network Protocol**: For distributed repositories
