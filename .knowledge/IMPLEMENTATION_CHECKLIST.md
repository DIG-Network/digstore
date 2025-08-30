# Digstore Min Implementation Checklist

## Phase 1: Project Setup & Foundation

### 1.1 Initialize Rust Project
- [ ] Create new Rust project with `cargo new digstore_min --bin`
- [ ] Configure `Cargo.toml` with project metadata
- [ ] Add initial dependencies:
  - [ ] `clap` (v4) for CLI parsing
  - [ ] `serde` and `serde_json` for serialization
  - [ ] `sha2` for SHA-256 hashing
  - [ ] `hex` for hex encoding/decoding
  - [ ] `thiserror` for error handling
  - [ ] `anyhow` for error propagation
  - [ ] `tokio` (optional) for async operations
  - [ ] `zstd` for compression
  - [ ] `directories` for finding home directory
  - [ ] `uuid` for generating store IDs
  - [ ] `chrono` for timestamps

### 1.2 Project Structure
- [ ] Create directory structure:
  ```
  src/
  ├── main.rs          # CLI entry point
  ├── lib.rs           # Library interface
  ├── core/
  │   ├── mod.rs
  │   ├── types.rs     # Core data types
  │   ├── hash.rs      # Hashing utilities
  │   └── error.rs     # Error types
  ├── storage/
  │   ├── mod.rs
  │   ├── store.rs     # Store management
  │   ├── layer.rs     # Layer operations
  │   └── chunk.rs     # Chunking logic
  ├── proofs/
  │   ├── mod.rs
  │   ├── merkle.rs    # Merkle tree implementation
  │   └── proof.rs     # Proof generation/verification
  ├── urn/
  │   ├── mod.rs
  │   └── parser.rs    # URN parsing
  └── cli/
      ├── mod.rs
      └── commands/      # Individual CLI commands
  ```

## Phase 2: Core Types and Data Structures

### 2.1 Define Core Types (`src/core/types.rs`)
- [ ] `Hash` type (32-byte SHA-256)
- [ ] `StoreId` type (32-byte identifier)
- [ ] `ChunkHash` type
- [ ] `TreeNode` enum (for merkle tree)
- [ ] `LayerType` enum (Full, Delta)
- [ ] `LayerHeader` struct
- [ ] `LayerMetadata` struct
- [ ] `Chunk` struct
- [ ] `FileEntry` struct
- [ ] `CommitInfo` struct

### 2.2 Error Handling (`src/core/error.rs`)
- [ ] Define custom error types using `thiserror`
- [ ] `StoreError` for storage operations
- [ ] `ProofError` for merkle proof errors
- [ ] `UrnError` for URN parsing errors
- [ ] `IoError` wrapper

### 2.3 Hash Utilities (`src/core/hash.rs`)
- [ ] SHA-256 hashing function
- [ ] Hash formatting (hex encoding)
- [ ] Hash parsing from hex
- [ ] Hash comparison operators

## Phase 3: Storage Engine Implementation

### 3.1 Store Management (`src/storage/store.rs`)
- [ ] `Store` struct with store ID and path
- [ ] `init_store()` - Create new store in ~/.dig/
- [ ] `open_store()` - Open existing store
- [ ] `get_store_path()` - Get path for store ID
- [ ] Store metadata management (Layer 0)

### 3.2 Layer Format (`src/storage/layer.rs`)
- [ ] Binary layer format implementation:
  ```
  Header (fixed size):
  - Magic bytes (4): "DLAY"
  - Version (2): 0x0001
  - Layer type (1): 0x00 (Full) or 0x01 (Delta)
  - Parent hash (32): Previous layer hash
  - Timestamp (8): Unix timestamp
  - Metadata length (4): Length of metadata section
  - Tree data length (4): Length of merkle tree
  - Chunk data offset (8): Offset to chunk data
  ```
- [ ] `write_layer()` - Write layer to disk
- [ ] `read_layer()` - Read layer from disk
- [ ] `verify_layer()` - Verify layer integrity

### 3.3 Content-Defined Chunking (`src/storage/chunk.rs`)
- [ ] Rolling hash implementation (Rabin fingerprint)
- [ ] `chunk_file()` - Split file into chunks
- [ ] Chunk size parameters (min: 512KB, avg: 1MB, max: 4MB)
- [ ] Chunk deduplication logic
- [ ] Optional compression per chunk

### 3.4 File Operations
- [ ] `add_file()` - Add file to staging
- [ ] `add_directory()` - Recursively add directory
- [ ] `get_file()` - Retrieve file by path
- [ ] `cat_file()` - Output file contents
- [ ] Staging area management

## Phase 4: Merkle Tree & Proofs

### 4.1 Merkle Tree (`src/proofs/merkle.rs`)
- [ ] Binary merkle tree implementation
- [ ] `MerkleTree` struct
- [ ] `build_tree()` - Build from file entries
- [ ] `get_root()` - Get root hash
- [ ] Tree serialization/deserialization

### 4.2 Proof Generation (`src/proofs/proof.rs`)
- [ ] `MerkleProof` struct
- [ ] `generate_proof()` - Create proof for path
- [ ] `generate_chunk_proof()` - Proof for chunk
- [ ] `generate_range_proof()` - Proof for byte range
- [ ] Proof JSON serialization

### 4.3 Proof Verification
- [ ] `verify_proof()` - Verify merkle proof
- [ ] Root hash validation
- [ ] Path reconstruction
- [ ] Sibling hash verification

## Phase 5: URN System

### 5.1 URN Parser (`src/urn/parser.rs`)
- [ ] URN format: `urn:dig:chia:{storeID}[:{rootHash}][/{path}][#{byteRange}]`
- [ ] `Urn` struct with components
- [ ] `parse_urn()` - Parse URN string
- [ ] `format_urn()` - Generate URN string
- [ ] Byte range parsing (e.g., "bytes=0-1048576")

### 5.2 URN Resolution
- [ ] `resolve_urn()` - Resolve URN to content
- [ ] Store ID lookup
- [ ] Root hash resolution (latest if not specified)
- [ ] Path traversal
- [ ] Byte range extraction

## Phase 6: CLI Interface

### 6.1 CLI Structure (`src/cli/mod.rs`)
- [ ] Set up clap CLI with subcommands
- [ ] Global options (verbose, quiet, no-progress, color)
- [ ] Help text and examples
- [ ] Auto-completion generation
- [ ] Pipe detection (`atty` crate)
- [ ] Error handling with suggestions

### 6.2 Progress Infrastructure
- [ ] Progress manager system
  - [ ] Multi-progress support (`indicatif`)
  - [ ] Automatic terminal detection
  - [ ] Progress bar styles and templates
- [ ] Streaming I/O wrapper
  - [ ] Progress tracking for reads/writes
  - [ ] Buffer management
  - [ ] Backpressure handling

### 6.3 Core Commands with Polish
- [ ] `init` - Initialize new repository
  - [ ] Visual feedback for each step
  - [ ] Success summary with formatting
  - [ ] Store ID display
- [ ] `add` - Add files to staging
  - [ ] Support `-r` for recursive
  - [ ] Path validation
  - [ ] Real-time progress with current file
  - [ ] Deduplication statistics
  - [ ] `--from-stdin` support
- [ ] `commit` - Create new layer
  - [ ] Multi-stage progress display
  - [ ] File processing progress
  - [ ] Chunk computation progress
  - [ ] Merkle tree building progress
  - [ ] Layer writing progress
  - [ ] Rich commit summary
- [ ] `status` - Show repository status
  - [ ] Rich formatted output
  - [ ] Short mode (`-s`)
  - [ ] Porcelain mode for scripts
  - [ ] Table formatting (`tabled`)

### 6.4 Retrieval Commands with Streaming
- [ ] `get` - Retrieve files
  - [ ] Full streaming support
  - [ ] Progress bars for file output
  - [ ] Automatic pipe detection
  - [ ] `-o` output option
  - [ ] `--progress` force flag
  - [ ] Byte range support
- [ ] `cat` - Output file contents
  - [ ] Automatic pager detection
  - [ ] Byte range support
  - [ ] Line numbering option
  - [ ] No buffering for pipes
- [ ] `extract` - Extract files
  - [ ] Progress for multiple files
  - [ ] Current file indication
  - [ ] Summary statistics

### 6.5 Proof Commands
- [ ] `prove` - Generate merkle proof
  - [ ] Progress for proof generation
  - [ ] Multiple output formats
  - [ ] Streaming output support
- [ ] `verify` - Verify merkle proof
  - [ ] Step-by-step verification display
  - [ ] Clear pass/fail indication
  - [ ] `--from-stdin` support

### 6.6 Output Formatting
- [ ] Color support (`colored`/`console`)
  - [ ] Success indicators (✓)
  - [ ] Error indicators (✗)
  - [ ] Smart color detection
- [ ] Table formatting
  - [ ] Status summaries
  - [ ] File listings
  - [ ] Statistics display
- [ ] Error formatting
  - [ ] Clear error messages
  - [ ] Helpful suggestions
  - [ ] Recovery instructions

### 6.7 Utility Commands
- [ ] `log` - Show commit history
- [ ] `info` - Display store information
- [ ] `gc` - Garbage collection (remove unreferenced chunks)

## Phase 7: Testing

### 7.1 Unit Tests
- [ ] Core type tests
- [ ] Hash function tests
- [ ] Chunking algorithm tests
- [ ] Merkle tree tests
- [ ] URN parser tests
- [ ] Layer format tests

### 7.2 Integration Tests (`tests/`)
- [ ] End-to-end workflow tests
- [ ] Store initialization and operations
- [ ] File add/commit/retrieve cycle
- [ ] Proof generation and verification
- [ ] URN resolution tests
- [ ] Large file handling
- [ ] Directory operations

### 7.3 Property-Based Tests
- [ ] Chunking determinism
- [ ] Merkle proof correctness
- [ ] Round-trip serialization

### 7.4 Performance Benchmarks (`benches/`)
- [ ] Chunking speed benchmark
- [ ] Hashing performance
- [ ] Merkle tree construction
- [ ] Large file operations

## Phase 8: Documentation & Polish

### 8.1 Knowledge Base (`digstore_min/.knowledge/`)
- [ ] Create `overview.md`
- [ ] Create `store-structure.md`
- [ ] Create `urn-specification.md`
- [ ] Create `layer-format.md`
- [ ] Create `merkle-proof.md`
- [ ] Create `cli-commands.md`
- [ ] Create `api-design.md`
- [ ] Create `implementation-guide.md`

### 8.2 Code Documentation
- [ ] Add rustdoc comments to all public APIs
- [ ] Add module-level documentation
- [ ] Create examples in doc comments
- [ ] Generate API documentation

### 8.3 Examples & Tutorials
- [ ] Basic usage example
- [ ] Advanced URN usage
- [ ] Proof verification example
- [ ] Integration examples

### 8.4 Final Polish
- [ ] Error message improvements
- [ ] Progress indicators for long operations
- [ ] Colored output support
- [ ] Cross-platform path handling
- [ ] Performance optimizations
- [ ] Code formatting (`cargo fmt`)
- [ ] Linting (`cargo clippy`)

## Phase 9: Release Preparation

### 9.1 Build & Packaging
- [ ] Configure release profile in Cargo.toml
- [ ] Set up GitHub Actions CI/CD
- [ ] Cross-platform builds (Linux, macOS, Windows)
- [ ] Create release binaries

### 9.2 Distribution
- [ ] Create installation script
- [ ] Package for cargo install
- [ ] Create Homebrew formula (macOS)
- [ ] Create snap package (Linux)
- [ ] Windows installer

### 9.3 Testing & Validation
- [ ] Full test suite passes
- [ ] Manual testing on all platforms
- [ ] Performance validation
- [ ] Security audit

## Implementation Tips

1. **Start Simple**: Begin with basic file operations before adding complexity
2. **Test Early**: Write tests as you implement each component
3. **Iterative Development**: Get basic functionality working before optimizing
4. **Error Handling**: Use Result<T, E> throughout, avoid unwrap() in production
5. **Documentation**: Document as you code, not after
6. **Performance**: Profile before optimizing, focus on correctness first

## Dependencies Summary

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
thiserror = "1"
anyhow = "1"
directories = "5"
uuid = { version = "1", features = ["v4"] }
chrono = "0.4"
zstd = "0.13"

[dev-dependencies]
tempfile = "3"
proptest = "1"
criterion = "0.5"
```

This checklist provides a complete roadmap for implementing Digstore Min from scratch. Each item should be completed in order, with testing and documentation happening continuously throughout the process.
