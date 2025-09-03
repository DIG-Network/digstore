# Layer Format Specification

## Overview

Digstore Min uses a binary layer format optimized for streaming, partial retrieval, and efficient diff storage. Each layer is a self-contained file that can be transported and verified independently.

## Layer File Structure

```
[Header Section]     256 bytes    - Fixed-size metadata
[Index Section]      Variable     - File and chunk index
[Data Section]       Variable     - Actual file/chunk data
[Merkle Section]     Variable     - Merkle tree for proofs
[Footer Section]     32 bytes     - Integrity checksum
```

## Header Section (256 bytes)

| Field | Offset | Size | Type | Description |
|-------|--------|------|------|-------------|
| Magic | 0 | 4 | `[u8; 4]` | "DIGS" identifier |
| Version | 4 | 2 | `u16` | Format version (1) |
| Layer Type | 6 | 1 | `u8` | 0=Header, 1=Full, 2=Delta |
| Flags | 7 | 1 | `u8` | Feature flags |
| Layer Number | 8 | 8 | `u64` | Sequential layer number |
| Timestamp | 16 | 8 | `u64` | Unix timestamp |
| Parent Hash | 24 | 32 | `[u8; 32]` | Parent layer root hash |
| Files Count | 56 | 4 | `u32` | Number of files |
| Chunks Count | 60 | 4 | `u32` | Number of chunks |
| Index Offset | 64 | 8 | `u64` | Offset to index section |
| Index Size | 72 | 8 | `u64` | Size of index section |
| Data Offset | 80 | 8 | `u64` | Offset to data section |
| Data Size | 88 | 8 | `u64` | Size of data section |
| Merkle Offset | 96 | 8 | `u64` | Offset to merkle section |
| Merkle Size | 104 | 8 | `u64` | Size of merkle section |
| Compression | 112 | 1 | `u8` | Compression algorithm |
| Reserved | 113 | 143 | `[u8; 143]` | Future use |

### Layer Types

- **0x00**: Header Layer (Layer 0 containing metadata)
- **0x01**: Full Layer (complete snapshot)
- **0x02**: Delta Layer (changes only)

### Flags Byte

| Bit | Purpose |
|-----|---------|
| 0 | Compressed |
| 1 | Has deleted files |
| 2-7 | Reserved |

### Compression Types

- **0x00**: None
- **0x01**: Zstd
- **0x02**: LZ4
- **0x03-0xFF**: Reserved

## Index Section

The index section contains metadata for all files and chunks in the layer.

### Index Header

| Field | Size | Type | Description |
|-------|------|------|-------------|
| Version | 2 | `u16` | Index format version |
| Entries Count | 4 | `u32` | Number of index entries |

### File Index Entry

| Field | Size | Type | Description |
|-------|------|------|-------------|
| Path Length | 2 | `u16` | Length of file path |
| Path | Variable | `String` | UTF-8 file path |
| File Size | 8 | `u64` | Total file size |
| File Hash | 32 | `[u8; 32]` | SHA-256 of complete file |
| Chunk Count | 2 | `u16` | Number of chunks |
| First Chunk | 4 | `u32` | Index of first chunk |
| Metadata Length | 2 | `u16` | Length of metadata |
| Metadata | Variable | `bytes` | File metadata (JSON) |

### Chunk Index Entry

| Field | Size | Type | Description |
|-------|------|------|-------------|
| Chunk Hash | 32 | `[u8; 32]` | SHA-256 of chunk |
| Offset | 8 | `u64` | Offset in file |
| Size | 4 | `u32` | Chunk size |
| Data Offset | 8 | `u64` | Offset in data section |
| Compressed Size | 4 | `u32` | Size after compression |
| Flags | 1 | `u8` | Chunk flags |

## Data Section

Contains the actual chunk data, potentially compressed.

### Data Layout
- Chunks stored sequentially
- Each chunk preceded by 4-byte size field
- Compressed chunks use specified algorithm
- Shared chunks appear only once

### Delta Layer Data
For delta layers, only new or modified chunks are stored. Unchanged chunks reference the parent layer.

## Merkle Tree Section

### Structure
```
[Tree Header]
  - Depth: u8
  - Leaf Count: u32
[Tree Nodes]
  - Level 0: Leaf hashes (file hashes)
  - Level 1: Parent nodes
  - ...
  - Root: Single root hash
```

### Node Format
- Each node: 32 bytes (SHA-256 hash)
- Nodes stored level by level
- Empty nodes filled with zero hash

## Footer Section (32 bytes)

| Field | Size | Type | Description |
|-------|------|------|-------------|
| Layer Hash | 32 | `[u8; 32]` | SHA-256 of entire layer |

## Layer 0 Special Format

Layer 0 contains repository metadata and root history.

### Layer 0 Data Section
```json
{
  "store_id": "hex_encoded_32_bytes",
  "created_at": 1699564800,
  "format_version": "1.0",
  "protocol_version": "1.0",
  "digstore_version": "0.1.0",
  "root_history": [
    {
      "generation": 0,
      "root_hash": "hex_encoded_hash",
      "timestamp": 1699564800,
      "layer_count": 1
    },
    {
      "generation": 1,
      "root_hash": "hex_encoded_hash", 
      "timestamp": 1699564801,
      "layer_count": 2
    }
  ],
  "config": {
    "chunk_size": 65536,
    "compression": "zstd",
    "delta_chain_limit": 10
  }
}
```

## Streaming Support

### Sequential Access
1. Read header (256 bytes)
2. Seek to index section
3. Load index into memory
4. Stream chunks as needed

### Random Access
1. Read header
2. Load index
3. Binary search for file
4. Seek directly to chunk data

### Byte Range Support
1. Locate file in index
2. Calculate chunk coverage
3. Read only required chunks
4. Extract byte range

## Optimization Techniques

### Chunk Deduplication
- Chunks identified by content hash
- Shared chunks stored once
- Significant space savings

### Delta Encoding
- Only modified chunks in delta layers
- Reference unchanged chunks in parent
- Efficient for small changes

### Compression
- Per-chunk compression
- Choose algorithm based on data type
- Skip compression for incompressible data

### Alignment
- Align sections to 4KB boundaries
- Optimize for filesystem performance
- Enable efficient memory mapping

## File Reconstruction

### From Full Layer
1. Find file in index
2. Read all chunks
3. Concatenate in order
4. Verify against file hash

### From Delta Chain
1. Start at target layer
2. Collect local chunks
3. For missing chunks:
   - Check parent layer
   - Recurse if needed
4. Assemble complete file
5. Verify hash

## Integrity Verification

### Layer Verification
1. Read entire layer
2. Compute SHA-256
3. Compare with footer hash

### File Verification
1. Reconstruct file
2. Compute SHA-256
3. Compare with index hash

### Chunk Verification
1. Read chunk data
2. Compute SHA-256
3. Compare with chunk hash

## Performance Considerations

1. **Index Caching**: Keep index in memory
2. **Chunk Alignment**: Align to page boundaries
3. **Compression**: Balance ratio vs speed
4. **Delta Chains**: Limit depth to prevent recursion
5. **Parallel I/O**: Read multiple chunks concurrently

## Implementation Notes

1. **Endianness**: All multi-byte values are little-endian
2. **Strings**: UTF-8 encoded, no null termination
3. **Hashes**: Binary format (not hex strings)
4. **Padding**: Sections padded to 4-byte alignment
5. **Versioning**: Check version before parsing
