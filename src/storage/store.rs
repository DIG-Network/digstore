//! Store management for Digstore Min

use crate::core::{types::*, error::*, digstore_file::DigstoreFile};
use sha2::Digest;
use crate::storage::{chunk::ChunkingEngine, layer::Layer, streaming::StreamingChunkingEngine, batch::BatchProcessor};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use directories::UserDirs;

/// Main store structure
pub struct Store {
    /// Store identifier
    pub store_id: StoreId,
    /// Path to the global store directory
    pub global_path: PathBuf,
    /// Path to the project directory (if in project context)
    pub project_path: Option<PathBuf>,
    /// Current root hash (latest generation)
    pub current_root: Option<RootHash>,
    /// Staging area for files to be committed
    pub staging: HashMap<PathBuf, StagedFile>,
    /// Chunking engine for processing files
    pub chunking_engine: ChunkingEngine,
    /// Streaming chunking engine for large files
    pub streaming_engine: StreamingChunkingEngine,
    /// Batch processor for many small files
    pub batch_processor: BatchProcessor,
}

/// A file in the staging area
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StagedFile {
    /// File entry with chunks
    pub file_entry: FileEntry,
    /// The actual chunks
    pub chunks: Vec<Chunk>,
    /// Whether this file was added in this session
    pub is_staged: bool,
}

impl Store {
    /// Initialize a new store in the current directory
    pub fn init(project_path: &Path) -> Result<Self> {
        // Check if already initialized
        let digstore_path = project_path.join(".digstore");
        if digstore_path.exists() {
            return Err(DigstoreError::store_already_exists(project_path.to_path_buf()));
        }

        // Generate new store ID
        let store_id = generate_store_id();
        
        // Get global store directory
        let global_path = get_global_store_path(&store_id)?;
        
        // Create global store directory
        std::fs::create_dir_all(&global_path)
            .map_err(|e| DigstoreError::Io(e))?;

        // Create .digstore file
        let repository_name = project_path.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        
        let digstore_file = DigstoreFile::new(store_id, repository_name);
        digstore_file.save(&digstore_path)?;

        // Initialize Layer 0 (metadata layer)
        let store = Self {
            store_id,
            global_path,
            project_path: Some(project_path.to_path_buf()),
            current_root: None,
            staging: HashMap::new(),
            chunking_engine: ChunkingEngine::new(),
            streaming_engine: StreamingChunkingEngine::new(),
            batch_processor: BatchProcessor::new(),
        };

        store.init_layer_zero()?;

        Ok(store)
    }

    /// Open an existing store from project directory
    pub fn open(project_path: &Path) -> Result<Self> {
        let digstore_path = project_path.join(".digstore");
        if !digstore_path.exists() {
            return Err(DigstoreError::store_not_found(project_path.to_path_buf()));
        }

        let mut digstore_file = DigstoreFile::load(&digstore_path)?;
        let store_id = digstore_file.get_store_id()?;
        let global_path = get_global_store_path(&store_id)?;

        if !global_path.exists() {
            return Err(DigstoreError::store_not_found(global_path));
        }

        // Update last accessed time
        digstore_file.update_last_accessed();
        digstore_file.save(&digstore_path)?;

        // Load current root hash from Layer 0
        let current_root = Self::load_current_root(&global_path)?;

        // Load persistent staging
        let staging = Self::load_staging(&global_path)?;

        Ok(Self {
            store_id,
            global_path,
            project_path: Some(project_path.to_path_buf()),
            current_root,
            staging,
            chunking_engine: ChunkingEngine::new(),
            streaming_engine: StreamingChunkingEngine::new(),
            batch_processor: BatchProcessor::new(),
        })
    }

    /// Open a store by ID directly (without project context)
    pub fn open_global(store_id: &StoreId) -> Result<Self> {
        let global_path = get_global_store_path(store_id)?;
        
        if !global_path.exists() {
            return Err(DigstoreError::store_not_found(global_path));
        }

        // Load current root hash from Layer 0
        let current_root = Self::load_current_root(&global_path)?;

        // Load persistent staging
        let staging = Self::load_staging(&global_path)?;

        Ok(Self {
            store_id: *store_id,
            global_path,
            project_path: None,
            current_root,
            staging,
            chunking_engine: ChunkingEngine::new(),
            streaming_engine: StreamingChunkingEngine::new(),
            batch_processor: BatchProcessor::new(),
        })
    }

    /// Add files to staging
    pub fn add_files(&mut self, paths: &[&str]) -> Result<()> {
        for path_str in paths {
            self.add_file(Path::new(path_str))?;
        }
        Ok(())
    }

    /// Add a single file to staging (internal method)
    fn add_file_internal(&mut self, file_path: &Path) -> Result<()> {
        // Resolve relative to project directory if available
        let full_path = if let Some(project_path) = &self.project_path {
            if file_path.is_relative() {
                project_path.join(file_path)
            } else {
                file_path.to_path_buf()
            }
        } else {
            file_path.to_path_buf()
        };

        // Check if file exists
        if !full_path.exists() {
            return Err(DigstoreError::file_not_found(full_path));
        }

        if !full_path.is_file() {
            return Err(DigstoreError::invalid_file_path(full_path));
        }

        // Get file size to determine processing strategy  
        let file_size = std::fs::metadata(&full_path)?.len();
        
        // Use streaming processing - NEVER load entire file into memory
        let chunks = if file_size > 10 * 1024 * 1024 {
            // Large files: use streaming chunking engine
            self.streaming_engine.chunk_file_streaming(&full_path)?
        } else {
            // Smaller files: use regular chunking but still streaming
            self.chunking_engine.chunk_file_streaming(&full_path)?
        };
        
        // Create file entry from chunk metadata (no file data stored)
        let file_hash = Self::compute_file_hash_from_chunks(&chunks);
        let file_entry = crate::core::types::FileEntry {
            path: file_path.to_path_buf(),
            hash: file_hash,
            size: file_size,
            chunks: chunks.iter().map(|c| crate::core::types::ChunkRef {
                hash: c.hash,
                offset: c.offset,
                size: c.size,
            }).collect(),
            metadata: FileMetadata {
                mode: 0o644,
                modified: std::fs::metadata(&full_path)?.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                is_new: true,
                is_modified: false,
                is_deleted: false,
            },
        };
        
        // Add to staging
        let staged_file = StagedFile {
            file_entry,
            chunks,
            is_staged: true,
        };
        
        self.staging.insert(file_path.to_path_buf(), staged_file);
        
        Ok(())
    }

    /// Add a single file to staging (public method with persistence)
    pub fn add_file(&mut self, file_path: &Path) -> Result<()> {
        self.add_file_internal(file_path)?;
        self.save_staging()?;
        Ok(())
    }

    /// Add many files efficiently using batch processing
    pub fn add_files_batch(&mut self, files: Vec<PathBuf>, progress: Option<&indicatif::ProgressBar>) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        
        // Use batch processing for efficiency
        let batch_result = self.batch_processor.process_files_batch(files, progress)?;
        
        // Store metrics before consuming batch_result
        let file_count = batch_result.file_entries.len();
        let metrics = batch_result.performance_metrics.clone();
        let dedup_stats = batch_result.deduplication_stats.clone();
        
        // Add all processed files to staging
        for (file_entry, chunks) in batch_result.file_entries.into_iter().zip(batch_result.chunks.into_iter()) {
            let staged_file = StagedFile {
                file_entry: file_entry.clone(),
                chunks: vec![chunks], // Single chunk for now
                is_staged: true,
            };
            self.staging.insert(file_entry.path.clone(), staged_file);
        }
        
        // Save staging to disk
        self.save_staging()?;
        
        // Print performance summary
        println!("  • Processed {} files at {:.1} files/s ({:.1} MB/s)", 
                 file_count,
                 metrics.files_per_second,
                 metrics.mb_per_second);
        
        if dedup_stats.deduplication_ratio > 0.01 {
            println!("  • Deduplication: {:.1}% ({} bytes saved)",
                     dedup_stats.deduplication_ratio * 100.0,
                     dedup_stats.bytes_saved);
        }
        
        Ok(())
    }

    /// Add a directory recursively
    pub fn add_directory(&mut self, dir_path: &Path, recursive: bool) -> Result<()> {
        if !dir_path.exists() {
            return Err(DigstoreError::file_not_found(dir_path.to_path_buf()));
        }

        if !dir_path.is_dir() {
            return Err(DigstoreError::invalid_file_path(dir_path.to_path_buf()));
        }

        if recursive {
            use walkdir::WalkDir;
            
            // Collect all files first
            let mut files = Vec::new();
            for entry in WalkDir::new(dir_path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let relative_path = entry.path().strip_prefix(
                    self.project_path.as_ref().unwrap_or(&std::env::current_dir()?)
                ).unwrap_or(entry.path());
                
                files.push(relative_path.to_path_buf());
            }
            
            // Use batch processing if many files, otherwise process individually
            if files.len() > 50 {
                println!("  • Using batch processing for {} files", files.len());
                self.add_files_batch(files, None)?;
                return Ok(());
            } else {
                // Process individually for small numbers of files
                for file_path in files {
                    self.add_file_internal(&file_path)?;
                }
            }
        } else {
            // Add only direct files in directory
            for entry in std::fs::read_dir(dir_path)? {
                let entry = entry?;
                let path = entry.path();
                
                if path.is_file() {
                    let relative_path = path.strip_prefix(
                        self.project_path.as_ref().unwrap_or(&std::env::current_dir()?)
                    ).unwrap_or(&path);
                    
                    self.add_file_internal(relative_path)?;
                }
            }
        }

        // Save staging to disk
        self.save_staging()?;

        Ok(())
    }

    /// Create a commit from staged files (cumulative approach for MVP)
    pub fn commit(&mut self, message: &str) -> Result<LayerId> {
        if self.staging.is_empty() {
            return Err(DigstoreError::internal("No files staged for commit"));
        }

        // Create new layer
        let layer_number = self.get_next_layer_number()?;
        let parent_hash = self.current_root.unwrap_or(Hash::zero());
        let mut layer = Layer::new(LayerType::Full, layer_number, parent_hash);
        
        // For MVP: Create cumulative layers that include all files from previous commits
        // First, add all files from the previous layer (if any)
        if let Some(current_root) = self.current_root {
            if let Ok(previous_layer) = self.load_layer(current_root) {
                for file_entry in previous_layer.files {
                    // Only add if not being replaced by staged version
                    if !self.staging.contains_key(&file_entry.path) {
                        layer.add_file(file_entry.clone());
                        
                        // Add chunks for this file
                        for chunk in &previous_layer.chunks {
                            // Check if this chunk belongs to this file
                            for chunk_ref in &file_entry.chunks {
                                if chunk_ref.hash == chunk.hash {
                                    layer.add_chunk(chunk.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Add all staged files and chunks to layer
        for staged_file in self.staging.values() {
            layer.add_file(staged_file.file_entry.clone());
            
            for chunk in &staged_file.chunks {
                layer.add_chunk(chunk.clone());
            }
        }

        // Set commit message in metadata
        layer.metadata.message = Some(message.to_string());
        layer.metadata.author = Some(get_default_author());

        // Compute layer ID
        let layer_id = layer.compute_layer_id()?;
        layer.metadata.layer_id = layer_id;

        // Write layer to disk
        let layer_filename = format!("{}.layer", layer_id.to_hex());
        let layer_path = self.global_path.join(layer_filename);
        layer.write_to_file(&layer_path)?;

        // Update root history in Layer 0
        self.update_root_history(layer_id)?;

        // Update current root
        self.current_root = Some(layer_id);

        // Clear staging
        self.staging.clear();
        
        // Save empty staging to disk
        self.save_staging()?;

        Ok(layer_id)
    }

    /// Load a layer by its ID
    pub fn load_layer(&self, layer_id: LayerId) -> Result<Layer> {
        let layer_filename = format!("{}.layer", layer_id.to_hex());
        let layer_path = self.global_path.join(layer_filename);
        Layer::read_from_file(&layer_path)
    }

    /// Get a file by path from the latest commit
    pub fn get_file(&self, file_path: &Path) -> Result<Vec<u8>> {
        // First check if file is in staging
        if let Some(staged_file) = self.staging.get(file_path) {
            return Ok(self.chunking_engine.reconstruct_from_chunks(&staged_file.chunks));
        }

        // Get from committed layers
        self.get_file_at(file_path, self.current_root)
    }

    /// Get a file at a specific root hash
    pub fn get_file_at(&self, file_path: &Path, root_hash: Option<RootHash>) -> Result<Vec<u8>> {
        let target_root = root_hash.unwrap_or(
            self.current_root.ok_or_else(|| DigstoreError::file_not_found(file_path.to_path_buf()))?
        );

        // Find the layer containing this root hash
        let layer_filename = format!("{}.layer", target_root.to_hex());
        let layer_path = self.global_path.join(layer_filename);

        if !layer_path.exists() {
            return Err(DigstoreError::layer_not_found(target_root));
        }

        // Read layer
        let layer = Layer::read_from_file(&layer_path)?;

        // Find file in layer
        for file_entry in &layer.files {
            if file_entry.path == file_path {
                // Reconstruct file from chunks
                let mut file_chunks = Vec::new();
                
                for chunk_ref in &file_entry.chunks {
                    // Find chunk in layer
                    for chunk in &layer.chunks {
                        if chunk.hash == chunk_ref.hash {
                            file_chunks.push(chunk.clone());
                            break;
                        }
                    }
                }

                return Ok(self.chunking_engine.reconstruct_from_chunks(&file_chunks));
            }
        }

        Err(DigstoreError::file_not_found(file_path.to_path_buf()))
    }

    /// Get repository status
    pub fn status(&self) -> StoreStatus {
        StoreStatus {
            store_id: self.store_id,
            current_root: self.current_root,
            staged_files: self.staging.keys().cloned().collect(),
            total_staged_size: self.staging.values()
                .map(|f| f.file_entry.size)
                .sum(),
        }
    }

    /// Check if a file is staged
    pub fn is_file_staged(&self, path: &Path) -> bool {
        self.staging.contains_key(path)
    }

    /// Remove a file from staging
    pub fn unstage_file(&mut self, path: &Path) -> Result<()> {
        if self.staging.remove(path).is_none() {
            return Err(DigstoreError::file_not_found(path.to_path_buf()));
        }
        // Save staging to disk
        self.save_staging()?;
        Ok(())
    }

    /// Clear all staged files
    pub fn clear_staging(&mut self) {
        self.staging.clear();
        // Save empty staging to disk
        let _ = self.save_staging();
    }

    /// Initialize Layer 0 with metadata
    fn init_layer_zero(&self) -> Result<()> {
        let layer_zero_path = self.global_path.join("0000000000000000.layer");
        
        // Create initial metadata
        let metadata = serde_json::json!({
            "store_id": self.store_id.to_hex(),
            "created_at": chrono::Utc::now().timestamp(),
            "format_version": "1.0",
            "protocol_version": "1.0", 
            "digstore_version": env!("CARGO_PKG_VERSION"),
            "root_history": [],
            "config": {
                "chunk_size": 65536,
                "compression": "zstd",
                "delta_chain_limit": 10
            }
        });

        let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
        std::fs::write(layer_zero_path, metadata_bytes)
            .map_err(|e| DigstoreError::Io(e))?;

        Ok(())
    }

    /// Load current root hash from Layer 0
    fn load_current_root(global_path: &Path) -> Result<Option<RootHash>> {
        let layer_zero_path = global_path.join("0000000000000000.layer");
        
        if !layer_zero_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read(layer_zero_path)
            .map_err(|e| DigstoreError::Io(e))?;
        
        let metadata: serde_json::Value = serde_json::from_slice(&content)?;
        
        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            if let Some(latest_entry) = root_history.last() {
                if let Some(root_hash_str) = latest_entry.get("root_hash").and_then(|v| v.as_str()) {
                    return Ok(Some(Hash::from_hex(root_hash_str)
                        .map_err(|_| DigstoreError::store_corrupted("Invalid root hash in Layer 0"))?));
                }
            }
        }

        Ok(None)
    }

    /// Get the store ID
    pub fn store_id(&self) -> StoreId {
        self.store_id
    }

    /// Get the global store path
    pub fn global_path(&self) -> &Path {
        &self.global_path
    }

    /// Get the project path (if in project context)
    pub fn project_path(&self) -> Option<&Path> {
        self.project_path.as_deref()
    }

    /// Get the current root hash
    pub fn current_root(&self) -> Option<RootHash> {
        self.current_root
    }
}

/// Generate a new random store ID
pub fn generate_store_id() -> StoreId {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("Failed to generate random bytes");
    Hash::from_bytes(bytes)
}

/// Get the path to the global store directory for a store ID
fn get_global_store_path(store_id: &StoreId) -> Result<PathBuf> {
    let user_dirs = UserDirs::new()
        .ok_or(DigstoreError::HomeDirectoryNotFound)?;
    
    let dig_dir = user_dirs.home_dir().join(".dig");
    Ok(dig_dir.join(store_id.to_hex()))
}

/// Get the global .dig directory
pub fn get_global_dig_directory() -> Result<PathBuf> {
    let user_dirs = UserDirs::new()
        .ok_or(DigstoreError::HomeDirectoryNotFound)?;
    
    Ok(user_dirs.home_dir().join(".dig"))
}

/// Repository status information
#[derive(Debug, Clone)]
pub struct StoreStatus {
    /// Store identifier
    pub store_id: StoreId,
    /// Current root hash
    pub current_root: Option<RootHash>,
    /// List of staged files
    pub staged_files: Vec<PathBuf>,
    /// Total size of staged files
    pub total_staged_size: u64,
}

impl Store {
    /// Get the next layer number
    fn get_next_layer_number(&self) -> Result<u64> {
        // For now, just count existing layer files
        let mut max_layer = 0u64;
        
        for entry in std::fs::read_dir(&self.global_path)? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();
            
            if filename_str.ends_with(".layer") && filename_str != "0000000000000000.layer" {
                // Try to parse layer number from filename (this is simplified)
                max_layer += 1;
            }
        }
        
        Ok(max_layer + 1)
    }

    /// Update root history in Layer 0
    fn update_root_history(&self, new_root: RootHash) -> Result<()> {
        let layer_zero_path = self.global_path.join("0000000000000000.layer");
        
        // Read current Layer 0
        let content = std::fs::read(&layer_zero_path)?;
        let mut metadata: serde_json::Value = serde_json::from_slice(&content)?;
        
        // Add new root to history
        if let Some(root_history) = metadata.get_mut("root_history").and_then(|v| v.as_array_mut()) {
            root_history.push(serde_json::json!({
                "generation": root_history.len(),
                "root_hash": new_root.to_hex(),
                "timestamp": chrono::Utc::now().timestamp(),
                "layer_count": root_history.len() + 1
            }));
        }
        
        // Write back to Layer 0
        let updated_content = serde_json::to_vec_pretty(&metadata)?;
        std::fs::write(layer_zero_path, updated_content)?;
        
        Ok(())
    }

    /// Load staging from disk
    fn load_staging(global_path: &Path) -> Result<HashMap<PathBuf, StagedFile>> {
        let staging_path = global_path.join("staging.json");
        
        if !staging_path.exists() {
            return Ok(HashMap::new());
        }

        let content = std::fs::read(staging_path)?;
        let staging: HashMap<PathBuf, StagedFile> = serde_json::from_slice(&content)
            .unwrap_or_else(|_| HashMap::new()); // If corrupted, start fresh
        
        Ok(staging)
    }

    /// Save staging to disk
    fn save_staging(&self) -> Result<()> {
        let staging_path = self.global_path.join("staging.json");
        let content = serde_json::to_vec_pretty(&self.staging)?;
        std::fs::write(staging_path, content)?;
        Ok(())
    }
    
    /// Compute file hash from chunks without loading file data
    fn compute_file_hash_from_chunks(chunks: &[Chunk]) -> Hash {
        let mut hasher = sha2::Sha256::new();
        for chunk in chunks {
            hasher.update(&chunk.data);
        }
        Hash::from_bytes(hasher.finalize().into())
    }
}

/// Get default author name
fn get_default_author() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "Unknown".to_string())
}
