# .dig Archive File Format Specification

## Overview

The `.dig` archive format is a single-file container that stores all layer files for a Digstore repository. This replaces the previous directory-based approach where each store had its own folder in `~/.dig/`.

## File Structure

### Location
- **Old Format**: `~/.dig/{store_id}/` (directory containing multiple `.layer` files)
- **New Format**: `~/.dig/{store_id}.dig` (single archive file)

### Archive Format

The `.dig` file uses a simple archive format optimized for layer storage:

```
┌─────────────────────────────────────────────────────────────┐
│                    Archive Header (64 bytes)                │
├─────────────────────────────────────────────────────────────┤
│                    Layer Index Section                      │
│  (Variable size: num_layers * LayerIndexEntry.SIZE)        │
├─────────────────────────────────────────────────────────────┤
│                    Layer Data Section                       │
│  (Variable size: concatenated layer files)                 │
└─────────────────────────────────────────────────────────────┘
```

### Archive Header (64 bytes)
```rust
struct ArchiveHeader {
    magic: [u8; 8],           // "DIGARCH\0" 
    version: u32,             // Format version (1)
    layer_count: u32,         // Number of layers in archive
    index_offset: u64,        // Offset to layer index section
    index_size: u64,          // Size of index section in bytes
    data_offset: u64,         // Offset to layer data section
    data_size: u64,           // Size of data section in bytes
    reserved: [u8; 24],       // Reserved for future use
}
```

### Layer Index Entry (80 bytes)
```rust
struct LayerIndexEntry {
    layer_hash: [u8; 32],     // SHA-256 hash of layer (filename)
    offset: u64,              // Offset to layer data in archive
    size: u64,                // Size of layer data in bytes
    compression: u32,         // Compression type (0=none, 1=zstd)
    checksum: u32,            // CRC32 checksum of layer data
    reserved: [u8; 8],        // Reserved for future use
}
```

## Implementation Requirements

### Core Components

#### 1. DigArchive Struct
```rust
pub struct DigArchive {
    archive_path: PathBuf,
    header: ArchiveHeader,
    index: Vec<LayerIndexEntry>,
    file: Option<File>,
    mmap: Option<Mmap>,
}
```

#### 2. Key Operations
- **`create()`**: Create new empty archive
- **`open()`**: Open existing archive with memory mapping
- **`add_layer()`**: Add layer to archive (append mode)
- **`get_layer()`**: Retrieve layer by hash
- **`list_layers()`**: List all layers in archive
- **`remove_layer()`**: Remove layer (compact archive)
- **`compact()`**: Defragment and optimize archive

#### 3. Store Integration
- Update `Store::init()` to create `.dig` archive instead of directory
- Update `Store::open()` to open `.dig` archive
- Update all layer read/write operations to use archive
- Maintain backward compatibility during transition

### Performance Requirements

#### Memory Efficiency
- **Memory-mapped access** for large archives
- **Lazy loading** of layer index
- **Streaming reads** for large layers
- **Constant memory usage** regardless of archive size

#### I/O Optimization
- **Append-only writes** for new layers
- **Batch operations** for multiple layer access
- **Compression support** for layer data
- **Checksums** for data integrity

#### Scalability
- **Handle 1000+ layers** efficiently
- **Multi-GB archives** with good performance
- **Concurrent read access** (multiple processes)
- **Atomic writes** for consistency

### CLI Integration

#### New Commands
```bash
# List layers in archive
digstore layers --list                    # List all layers
digstore layers --archive-info            # Show archive statistics

# Archive management
digstore archive --compact                # Compact archive (remove gaps)
digstore archive --verify                 # Verify archive integrity
digstore archive --stats                  # Show detailed archive stats

# Layer inspection within archive
digstore inspect --layer <hash>           # Inspect specific layer
digstore inspect --archive               # Inspect entire archive
```

#### Enhanced Existing Commands
- **`digstore info`**: Show archive file size and layer count
- **`digstore size`**: Include archive overhead and efficiency metrics
- **`digstore stats`**: Add archive-specific statistics

### Migration Strategy

#### Automatic Migration
- Detect old directory-based stores during `Store::open()`
- Automatically migrate to `.dig` archive format
- Preserve all existing layer data
- Clean up old directory after successful migration

#### Migration Process
1. **Detection**: Check if `~/.dig/{store_id}/` directory exists
2. **Archive Creation**: Create new `{store_id}.dig` archive
3. **Layer Migration**: Copy all `.layer` files to archive
4. **Verification**: Verify all layers are accessible
5. **Cleanup**: Remove old directory structure
6. **Update References**: Update `.digstore` file if needed

### Error Handling

#### Archive Corruption
- **Header validation**: Magic bytes and version checks
- **Index validation**: Verify layer offsets and sizes
- **Checksum verification**: Validate layer data integrity
- **Recovery**: Attempt to recover readable layers

#### Concurrent Access
- **File locking**: Prevent corruption from concurrent writes
- **Read-only access**: Allow multiple readers
- **Atomic operations**: Ensure consistency during updates
- **Backup strategy**: Create backup before major operations

## Testing Requirements

### Unit Tests
- Archive header serialization/deserialization
- Layer index operations (add, remove, lookup)
- Memory mapping and file I/O
- Compression and checksum validation

### Integration Tests
- Store migration from directory to archive format
- Full repository workflow with archive storage
- Concurrent access scenarios
- Large archive performance testing

### Performance Tests
- Archive with 1000+ layers
- Multi-GB archive handling
- Memory usage validation
- I/O performance benchmarks

## Success Criteria

### Functional Requirements
- ✅ Single `.dig` file replaces directory structure
- ✅ All existing layer data preserved during migration
- ✅ CLI commands can list and inspect layers within archive
- ✅ Memory-mapped access for performance
- ✅ Compression support for space efficiency

### Performance Requirements
- ✅ Handle 1000+ layers without degradation
- ✅ <100MB memory usage for large archives
- ✅ Fast layer lookup (O(log n) with binary search)
- ✅ Efficient append operations for new layers

### Compatibility Requirements
- ✅ Automatic migration from old format
- ✅ All existing CLI commands work unchanged
- ✅ All tests pass with new format
- ✅ Cross-platform compatibility maintained
