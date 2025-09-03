# API Design Documentation

## Overview

The Digstore Min API is designed as a layered architecture with clear separation of concerns. The API provides both a Rust library interface and a CLI wrapper, enabling integration into other applications.

## Architecture Layers

```
┌─────────────────────────────┐
│      CLI Interface          │
├─────────────────────────────┤
│    High-Level API           │
├─────────────────────────────┤
│     Core Services           │
├─────────────────────────────┤
│    Storage Engine           │
├─────────────────────────────┤
│   Platform Abstractions     │
└─────────────────────────────┘
```

## Core Types

### DigstoreFile

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigstoreFile {
    pub version: String,
    pub store_id: String,
    pub encrypted: bool,
    pub created_at: String,
    pub last_accessed: String,
    pub repository_name: Option<String>,
}

impl DigstoreFile {
    pub fn load(path: &Path) -> Result<Self>;
    pub fn save(&self, path: &Path) -> Result<()>;
    pub fn create(store_id: StoreId, name: Option<String>) -> Self;
}
```

### Store

```rust
pub struct Store {
    store_id: StoreId,
    root_hash: RootHash,
    config: StoreConfig,
    layers: LayerManager,
    index: StoreIndex,
    global_path: PathBuf,  // Path to ~/.dig/{store_id}
}

impl Store {
    // Initialize new store in global directory, create .digstore in current dir
    pub fn init(project_path: &Path) -> Result<Self>;
    
    // Open existing store via .digstore file
    pub fn open(project_path: &Path) -> Result<Self>;
    
    // Direct access to global store (no .digstore needed)
    pub fn open_global(store_id: &StoreId) -> Result<Self>;
    
    pub fn add(&mut self, paths: &[PathBuf]) -> Result<()>;
    pub fn commit(&mut self, message: &str) -> Result<LayerId>;
    pub fn get_by_urn(&self, urn: &Urn) -> Result<Vec<u8>>;
    pub fn get_by_path(&self, path: &Path) -> Result<Vec<u8>>;
    pub fn generate_proof(&self, target: &ProofTarget) -> Result<Proof>;
}
```

### StoreId

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreId([u8; 31]);

impl StoreId {
    pub fn generate() -> Self;
    pub fn from_bytes(bytes: [u8; 31]) -> Self;
    pub fn as_hex(&self) -> String;
    pub fn from_hex(hex: &str) -> Result<Self>;
}
```

### Urn

```rust
pub struct Urn {
    store_id: StoreId,
    root_hash: Option<RootHash>,
    resource_path: Option<PathBuf>,
    byte_range: Option<ByteRange>,
}

impl Urn {
    pub fn parse(s: &str) -> Result<Self>;
    pub fn to_string(&self) -> String;
    pub fn with_byte_range(self, range: ByteRange) -> Self;
}

pub struct ByteRange {
    pub start: Option<u64>,
    pub end: Option<u64>,
}
```

### Layer

```rust
pub struct Layer {
    header: LayerHeader,
    files: HashMap<PathBuf, FileEntry>,
    chunks: ChunkStore,
    merkle_tree: MerkleTree,
}

pub struct LayerHeader {
    pub layer_type: LayerType,
    pub layer_number: u64,
    pub parent_hash: Option<RootHash>,
    pub timestamp: u64,
    pub metadata: HashMap<String, String>,
}

pub enum LayerType {
    Header,
    Full,
    Delta,
}
```

## Core Services

### LayerManager

Manages the collection of layers in a store.

```rust
pub struct LayerManager {
    layers: Vec<Layer>,
    layer_index: HashMap<RootHash, usize>,
}

impl LayerManager {
    pub fn add_layer(&mut self, layer: Layer) -> Result<()>;
    pub fn get_layer(&self, hash: &RootHash) -> Option<&Layer>;
    pub fn get_latest(&self) -> &Layer;
    pub fn iter_history(&self) -> impl Iterator<Item = &Layer>;
}
```

### FileReconstructor

Reconstructs files from potentially multiple layers.

```rust
pub struct FileReconstructor<'a> {
    layers: &'a LayerManager,
}

impl<'a> FileReconstructor<'a> {
    pub fn reconstruct_at(
        &self,
        path: &Path,
        root_hash: &RootHash,
    ) -> Result<Vec<u8>>;
    
    pub fn reconstruct_range(
        &self,
        path: &Path,
        root_hash: &RootHash,
        range: &ByteRange,
    ) -> Result<Vec<u8>>;
}
```

### ProofGenerator

Generates merkle proofs for various targets.

```rust
pub struct ProofGenerator<'a> {
    store: &'a Store,
}

impl<'a> ProofGenerator<'a> {
    pub fn prove_file(&self, path: &Path, at: &RootHash) -> Result<Proof>;
    pub fn prove_bytes(&self, target: &ByteProofTarget) -> Result<Proof>;
    pub fn prove_layer(&self, layer_id: &LayerId) -> Result<Proof>;
}

pub struct Proof {
    pub version: String,
    pub proof_type: ProofType,
    pub target: ProofTarget,
    pub root: RootHash,
    pub proof_path: Vec<ProofElement>,
}
```

### ChunkingEngine

Handles file chunking for deduplication and deltas.

```rust
pub struct ChunkingEngine {
    config: ChunkConfig,
}

impl ChunkingEngine {
    pub fn chunk_file(&self, data: &[u8]) -> Vec<Chunk>;
    pub fn find_boundaries(&self, data: &[u8]) -> Vec<usize>;
}

pub struct Chunk {
    pub offset: u64,
    pub size: u32,
    pub hash: ChunkHash,
    pub data: Vec<u8>,
}
```

## Storage Engine

### LayerStorage

Handles reading and writing layer files.

```rust
pub trait LayerStorage {
    fn write_layer(&mut self, layer: &Layer) -> Result<RootHash>;
    fn read_layer(&self, hash: &RootHash) -> Result<Layer>;
    fn list_layers(&self) -> Result<Vec<RootHash>>;
    fn delete_layer(&mut self, hash: &RootHash) -> Result<()>;
}

pub struct FileSystemStorage {
    base_path: PathBuf,
}

impl LayerStorage for FileSystemStorage {
    // Implementation
}
```

### IndexManager

Maintains indices for fast lookups.

```rust
pub struct IndexManager {
    file_index: HashMap<PathBuf, FileLocation>,
    chunk_index: HashMap<ChunkHash, ChunkLocation>,
}

pub struct FileLocation {
    pub layer_hash: RootHash,
    pub offset: u64,
    pub size: u64,
}
```

## High-Level API

### Repository Operations

```rust
pub struct Repository {
    store: Store,
}

impl Repository {
    // Initialization
    pub fn init(path: &Path) -> Result<Self>;
    pub fn open(path: &Path) -> Result<Self>;
    
    // Working with files
    pub fn add_files(&mut self, paths: &[PathBuf]) -> Result<()>;
    pub fn remove_files(&mut self, paths: &[PathBuf]) -> Result<()>;
    pub fn commit(&mut self, message: &str) -> Result<CommitId>;
    
    // Retrieval
    pub fn get_file(&self, path: &Path) -> Result<Vec<u8>>;
    pub fn get_file_at(&self, path: &Path, commit: &CommitId) -> Result<Vec<u8>>;
    pub fn cat(&self, urn: &Urn) -> Result<Vec<u8>>;
    
    // History
    pub fn log(&self) -> impl Iterator<Item = CommitInfo>;
    pub fn diff(&self, from: &CommitId, to: &CommitId) -> Result<Diff>;
    
    // Proofs
    pub fn prove(&self, target: ProofTarget) -> Result<Proof>;
    pub fn verify(&self, proof: &Proof) -> Result<bool>;
}
```

### Streaming API

```rust
pub struct StreamingReader {
    store: Arc<Store>,
}

impl StreamingReader {
    pub fn stream_file(&self, urn: &Urn) -> impl Stream<Item = Result<Bytes>>;
    pub fn stream_range(&self, urn: &Urn, range: ByteRange) -> impl Stream<Item = Result<Bytes>>;
}

impl AsyncRead for UrnReader {
    // Implementation for async reading
}
```

## Error Handling

```rust
#[derive(Debug, Error)]
pub enum DigstoreError {
    #[error("Store not found: {0}")]
    StoreNotFound(PathBuf),
    
    #[error("Invalid URN: {0}")]
    InvalidUrn(String),
    
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),
    
    #[error("Proof verification failed")]
    ProofVerificationFailed,
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    // ... other variants
}

pub type Result<T> = std::result::Result<T, DigstoreError>;
```

## Configuration

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreConfig {
    pub chunk_size: usize,
    pub compression: CompressionType,
    pub delta_chain_limit: usize,
    pub index_cache_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompressionType {
    None,
    Zstd { level: i32 },
    Lz4,
}
```

## Usage Examples

### Basic Operations

```rust
use digstore_min::{Repository, Urn};

// Initialize repository
let mut repo = Repository::init("./my-store")?;

// Add files
repo.add_files(&["src/main.rs", "Cargo.toml"])?;
let commit = repo.commit("Initial commit")?;

// Retrieve file
let content = repo.get_file("src/main.rs")?;

// Get by URN
let urn = Urn::parse("urn:dig:chia:abc123/src/main.rs")?;
let data = repo.cat(&urn)?;
```

### Proof Generation

```rust
// Generate proof for file
let proof = repo.prove(ProofTarget::File {
    path: "important.dat".into(),
    at: Some(commit),
})?;

// Verify proof
assert!(repo.verify(&proof)?);

// Export proof
let json = serde_json::to_string_pretty(&proof)?;
std::fs::write("proof.json", json)?;
```

### Streaming Large Files

```rust
use futures::StreamExt;

let reader = StreamingReader::new(repo.store());
let urn = Urn::parse("urn:dig:chia:abc123/video.mp4#bytes=0-1048576")?;

let mut stream = reader.stream_range(&urn);
while let Some(chunk) = stream.next().await {
    let chunk = chunk?;
    // Process chunk
}
```

## Thread Safety

- `Store`: Thread-safe for reads, requires external synchronization for writes
- `Repository`: Not thread-safe, use `Arc<Mutex<Repository>>` for sharing
- `StreamingReader`: Thread-safe, can be cloned and shared

## Performance Considerations

1. **Index Caching**: File and chunk indices kept in memory
2. **Lazy Loading**: Layers loaded on demand
3. **Parallel I/O**: Chunk reads can be parallelized
4. **Zero-Copy**: Where possible, avoid unnecessary copies

## Extension Points

1. **Custom Storage Backends**: Implement `LayerStorage` trait
2. **Alternative Hash Functions**: Configurable hasher
3. **Compression Algorithms**: Pluggable compression
4. **Chunk Strategies**: Different chunking algorithms

## Future API Additions

1. **Watch Mode**: Monitor filesystem changes
2. **Remote Backends**: S3, HTTP storage
3. **Replication**: Multi-store synchronization
4. **Transactions**: Atomic multi-file operations
5. **Hooks**: Pre/post commit hooks
