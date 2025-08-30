# Store Structure Specification

## Repository Layout

Digstore Min uses a two-part structure:

### 1. Global Store Directory (~/.dig)
The actual repository data lives in a global directory in the user's home:

```
~/.dig/
└── {store_id}/                  # Named by store ID
    ├── 0000000000000000.layer   # Layer 0 (header/metadata)
    ├── {root_hash_1}.layer      # Layer 1
    ├── {root_hash_2}.layer      # Layer 2
    └── {root_hash_n}.layer      # Layer N
```

### 2. Local Project Directory
Each project contains only a `.digstore` file that links to the global store:

```
my-project/
├── .digstore                    # Links to global store
├── src/
├── docs/
└── README.md
```

## .digstore File Format

The `.digstore` file is a TOML configuration file that links a local project directory to a global store:

```toml
version = "1.0.0"
store_id = "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2"
encrypted = false
created_at = "2023-11-10T12:00:00Z"
last_accessed = "2023-11-10T14:30:00Z"
repository_name = "my-project"
```

### Fields

- **version**: Format version of the .digstore file
- **store_id**: The 64-character hex store ID this project is linked to
- **encrypted**: Whether the repository uses encryption (always false for digstore_min)
- **created_at**: ISO 8601 timestamp when the link was created
- **last_accessed**: ISO 8601 timestamp of last access
- **repository_name**: Optional human-readable name for the repository

### Portable Design

Digstore Min uses a portable design:
- No absolute paths are stored
- Global `.dig` directory is discovered automatically (typically `~/.dig`)
- Working directory is wherever the `.digstore` file is found
- Repository can be moved/shared without path issues

## Store ID

- **Format**: 32-byte random value, hex-encoded (64 characters)
- **Generation**: Cryptographically secure random bytes
- **Purpose**: Unique identifier for the repository
- **Usage**: Directory name and URN component

Example: `a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2`

## Root Hash and Generations

### Root Hash
- **Definition**: Merkle root of all layers in the repository
- **Calculation**: Hash of concatenated layer hashes in order
- **Size**: 32 bytes (SHA-256)
- **Updates**: Changes with each new layer addition

### Generation
- **Definition**: A specific state of the repository identified by its root hash
- **Creation**: Each layer addition creates a new generation
- **Tracking**: Root history maintained in Layer 0

### Root History
- **Storage**: Array of root hashes in Layer 0
- **Format**: Sequential list of all historical root hashes
- **Purpose**: Enable retrieval of any historical state
- **Growth**: Appended with each new generation

## Layer System

### Layer Types

1. **Layer 0 - Header Layer**
   - Special metadata layer
   - Contains store configuration
   - Maintains root history
   - Updated with each generation

2. **Full Layers**
   - Complete snapshot of repository state
   - Self-contained data
   - Created periodically for efficiency

3. **Delta Layers**
   - Only contains changes from parent
   - References parent layer
   - Optimizes storage space

### Layer Naming

- **Layer 0**: Always `0000000000000000.layer`
- **Data Layers**: `{layer_root_hash}.layer`
  - Example: `a5c3f8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6.layer`

## Data Organization

### Within Layers

1. **File Entries**
   - Path (relative to repository root)
   - Content hash
   - Size
   - Permissions/metadata
   - Data chunks (for delta optimization)

2. **Chunk System**
   - Files split into content-defined chunks
   - Enables efficient delta storage
   - Chunks identified by hash
   - Shared chunks deduplicated

3. **Merkle Tree**
   - Built from all file hashes
   - Enables proof generation
   - Stored within each layer

### Cross-Layer References

For delta layers:
- Parent layer hash reference
- List of changed files
- New/modified chunks only
- Deleted file markers

## Metadata Structure

### Store Metadata (in Layer 0)
```json
{
  "store_id": "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2",
  "created_at": 1699564800,
  "format_version": "1.0",
  "root_history": [
    "initial_root_hash",
    "second_generation_root_hash",
    "current_root_hash"
  ],
  "layer_count": 42,
  "total_size": 1048576000
}
```

### Layer Metadata
```json
{
  "layer_id": "layer_hash",
  "parent_id": "parent_layer_hash",
  "timestamp": 1699564800,
  "generation": 42,
  "type": "delta",
  "file_count": 150,
  "total_size": 5242880,
  "merkle_root": "files_merkle_root"
}
```

## File Storage

### File Representation
```json
{
  "path": "src/main.rs",
  "hash": "file_content_hash",
  "size": 4096,
  "chunks": [
    {"offset": 0, "size": 1024, "hash": "chunk1_hash"},
    {"offset": 1024, "size": 1024, "hash": "chunk2_hash"},
    {"offset": 2048, "size": 2048, "hash": "chunk3_hash"}
  ],
  "metadata": {
    "mode": "0644",
    "modified": 1699564800
  }
}
```

### Chunk Storage
- Chunks stored separately in layer
- Referenced by hash
- Enables deduplication
- Supports partial file retrieval

## Optimization Strategies

1. **Delta Chain Limits**
   - Maximum delta chain depth: 10
   - Force full layer after limit
   - Prevents deep recursion

2. **Chunk Size**
   - Target chunk size: 64KB
   - Minimum: 16KB
   - Maximum: 1MB
   - Content-defined boundaries

3. **Deduplication**
   - Chunk-level deduplication
   - Cross-file sharing
   - Significant space savings

4. **Compression**
   - Optional zstd compression
   - Per-chunk compression
   - Configurable levels
