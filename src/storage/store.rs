//! Store management for Digstore Min

use crate::core::{types::*, error::*, digstore_file::DigstoreFile};
use sha2::Digest;
use crate::storage::{chunk::ChunkingEngine, layer::Layer, streaming::StreamingChunkingEngine, batch::BatchProcessor, binary_staging::{BinaryStagingArea, BinaryStagedFile}, dig_archive::{DigArchive, get_archive_path}};
use colored::Colorize;
use crate::security::{AccessController, StoreAccessControl};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use directories::UserDirs;

/// Main store structure
pub struct Store {
    /// Store identifier
    pub store_id: StoreId,
    /// Archive file for storing layers
    pub archive: DigArchive,
    /// Path to the project directory (if in project context)
    pub project_path: Option<PathBuf>,
    /// Current root hash (latest generation)
    pub current_root: Option<RootHash>,
    /// Binary staging area for files to be committed
    pub staging: BinaryStagingArea,
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
        Self::init_with_options(project_path, false)
    }

    /// Initialize a new store with options
    pub fn init_with_options(project_path: &Path, auto_yes: bool) -> Result<Self> {
        // Check if already initialized
        let digstore_path = project_path.join(".digstore");
        if digstore_path.exists() {
            // Ask for confirmation to overwrite
            use crate::cli::interactive;
            
            if !interactive::ask_overwrite_digstore(&digstore_path, auto_yes)
                .map_err(|e| DigstoreError::internal(format!("Interactive prompt failed: {}", e)))? {
                println!();
                println!("{}", "Initialization cancelled".yellow());
                return Err(DigstoreError::store_already_exists(project_path.to_path_buf()));
            }
            
            // Remove existing .digstore file to proceed with new initialization
            std::fs::remove_file(&digstore_path)?;
            println!();
            println!("{}", "Existing repository file removed".yellow());
        }

        // Generate new store ID
        let store_id = generate_store_id();
        
        // Get archive path
        let archive_path = get_archive_path(&store_id)?;
        
        // Create parent directory for .dig files
        if let Some(parent) = archive_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create .digstore file
        let repository_name = project_path.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        
        let digstore_file = DigstoreFile::new(store_id, repository_name);
        digstore_file.save(&digstore_path)?;

        // Create archive
        let archive = DigArchive::create(archive_path.clone())?;
        
        // Initialize binary staging area (in same directory as archive)
        let staging_path = archive_path.with_extension("staging.bin");
        let mut staging = BinaryStagingArea::new(staging_path);
        staging.initialize()?;

        // Initialize Layer 0 (metadata layer)
        let mut store = Self {
            store_id,
            archive,
            project_path: Some(project_path.to_path_buf()),
            current_root: None,
            staging,
            chunking_engine: ChunkingEngine::new(),
            streaming_engine: StreamingChunkingEngine::new(),
            batch_processor: BatchProcessor::new(),
        };

        store.init_layer_zero()?;

        Ok(store)
    }

    /// Open an existing store from project directory
    pub fn open(project_path: &Path) -> Result<Self> {
        Self::open_with_options(project_path, false)
    }

    /// Open an existing store with options
    pub fn open_with_options(project_path: &Path, auto_yes: bool) -> Result<Self> {
        let digstore_path = project_path.join(".digstore");
        if !digstore_path.exists() {
            // No .digstore file found - provide helpful guidance
            eprintln!("{}", "No repository found!".red().bold());
            eprintln!("  Looking for: {}", digstore_path.display().to_string().yellow());
            eprintln!();
            eprintln!("{}", "This directory is not a Digstore repository.".blue());
            eprintln!();
            eprintln!("{}", "To create a new repository:".green().bold());
            eprintln!("  {}", "digstore init --name \"my-project\"".cyan());
            eprintln!();
            
            return Err(DigstoreError::store_not_found(project_path.to_path_buf()));
        }

        let mut digstore_file = DigstoreFile::load(&digstore_path)?;
        let store_id = digstore_file.get_store_id()?;
        let archive_path = get_archive_path(&store_id)?;

        // Check for migration from old directory format
        let old_global_path = get_global_store_path(&store_id)?;
        let archive = if archive_path.exists() {
            // Open existing archive
            DigArchive::open(archive_path.clone())?
        } else if old_global_path.exists() {
            // Migrate from old directory format
            println!("Migrating store from directory to archive format...");
            let archive = DigArchive::migrate_from_directory(archive_path.clone(), &old_global_path)?;
            
            // Clean up old directory after successful migration
            std::fs::remove_dir_all(&old_global_path)?;
            println!("✓ Migration completed successfully");
            
            archive
        } else {
            // Store archive not found - offer interactive recreation
            use crate::cli::interactive;
            
            match interactive::handle_missing_store(&archive_path, &store_id.to_hex(), project_path, auto_yes)
                .map_err(|e| DigstoreError::internal(format!("Interactive prompt failed: {}", e))) {
                Ok(new_store) => return Ok(new_store),
                Err(e) => return Err(e),
            }
        };

        // Update last accessed time
        digstore_file.update_last_accessed();
        digstore_file.save(&digstore_path)?;

        // Load current root hash from Layer 0 in archive
        let current_root = Self::load_current_root_from_archive(&archive)?;

        // Load persistent binary staging
        let staging_path = archive_path.with_extension("staging.bin");
        let mut staging = BinaryStagingArea::new(staging_path);
        staging.load()?;

        Ok(Self {
            store_id,
            archive,
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
        let archive_path = get_archive_path(store_id)?;
        
        // Check for migration from old directory format
        let old_global_path = get_global_store_path(store_id)?;
        let archive = if archive_path.exists() {
            DigArchive::open(archive_path.clone())?
        } else if old_global_path.exists() {
            // Migrate from old directory format
            let archive = DigArchive::migrate_from_directory(archive_path.clone(), &old_global_path)?;
            std::fs::remove_dir_all(&old_global_path)?;
            archive
        } else {
            // Store archive not found - for global access, just show error (no project context)
            eprintln!("{}", "Store not found!".red().bold());
            eprintln!("  Archive file: {}", archive_path.display().to_string().yellow());
            eprintln!("  Store ID: {}", store_id.to_hex().cyan());
            eprintln!();
            eprintln!("{}", "This store does not exist or has been deleted.".blue());
            eprintln!("{}", "Use 'digstore init' in a project directory to create a new store.".green());
            eprintln!();
            
            return Err(DigstoreError::store_not_found(archive_path.clone()));
        };

        // Load current root hash from Layer 0 in archive
        let current_root = Self::load_current_root_from_archive(&archive)?;

        // Load persistent binary staging
        let staging_path = archive_path.with_extension("staging.bin");
        let mut staging = BinaryStagingArea::new(staging_path);
        staging.load()?;

        Ok(Self {
            store_id: *store_id,
            archive,
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
        
        // Convert to binary staged file format
        let binary_staged_file = BinaryStagedFile {
            path: file_path.to_path_buf(),
            hash: file_hash,
            size: file_size,
            chunks,
            modified_time: std::fs::metadata(&full_path)?.modified().ok(),
        };
        
        // Add to binary staging area
        self.staging.stage_file_streaming(binary_staged_file)?;
        
        Ok(())
    }

    /// Add a single file to staging (public method with persistence)
    pub fn add_file(&mut self, file_path: &Path) -> Result<()> {
        self.add_file_internal(file_path)?;
        self.staging.flush()?;
        Ok(())
    }

    /// Add many files efficiently using batch processing
    pub fn add_files_batch(&mut self, files: Vec<PathBuf>, progress: Option<&indicatif::ProgressBar>) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        
        // Convert absolute paths to relative paths for storage, but keep absolute for processing
        let project_root = self.project_path.as_ref().cloned().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let files_with_relative: Vec<(PathBuf, PathBuf)> = files.into_iter().map(|abs_path| {
            // Try to make path relative to project root
            let relative_path = if let Ok(rel_path) = abs_path.strip_prefix(&project_root) {
                rel_path.to_path_buf()
            } else {
                // Fallback: use the absolute path as-is for now
                abs_path.clone()
            };
            (abs_path, relative_path)
        }).collect();
        
        // Use batch processing for efficiency (with absolute paths)
        let absolute_paths: Vec<PathBuf> = files_with_relative.iter().map(|(abs, _)| abs.clone()).collect();
        let batch_result = self.batch_processor.process_files_batch(absolute_paths, progress)?;
        
        // Store metrics before consuming batch_result
        let file_count = batch_result.file_entries.len();
        let metrics = batch_result.performance_metrics.clone();
        let dedup_stats = batch_result.deduplication_stats.clone();
        
        // Convert to binary staged files and add in batch
        let mut binary_staged_files = Vec::with_capacity(batch_result.file_entries.len());
        
        for (i, (mut file_entry, file_chunks)) in batch_result.file_entries.into_iter().zip(batch_result.chunks.into_iter()).enumerate() {
            // Update file entry to use relative path
            if let Some((abs_path, relative_path)) = files_with_relative.get(i) {
                file_entry.path = relative_path.clone();
                
                // Get file metadata
                let modified_time = std::fs::metadata(abs_path)?.modified().ok();
                
                binary_staged_files.push(BinaryStagedFile {
                    path: relative_path.clone(),
                    hash: file_entry.hash,
                    size: file_entry.size,
                    chunks: vec![file_chunks], // Store the actual chunks
                    modified_time,
                });
            }
        }
        
        // Add all files to binary staging in one batch operation
        self.staging.stage_files_batch(binary_staged_files)?;
        
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
            
            // Collect all files first (use absolute paths for batch processing)
            let mut files = Vec::new();
            for entry in WalkDir::new(dir_path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                // Store absolute path for batch processing
                files.push(entry.path().to_path_buf());
            }
            
            // Use batch processing if many files, otherwise process individually
            if files.len() > 10 {
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
        self.staging.flush()?;

        Ok(())
    }

    /// Create a commit from staged files (cumulative approach for MVP)
    pub fn commit(&mut self, message: &str) -> Result<LayerId> {
        if self.staging.staged_count() == 0 {
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
                    if !self.staging.is_staged(&file_entry.path) {
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
        let staged_files = self.staging.get_all_staged_files()?;
        for staged_file in &staged_files {
            // Convert BinaryStagedFile to FileEntry
            let file_entry = FileEntry {
                path: staged_file.path.clone(),
                hash: staged_file.hash,
                size: staged_file.size,
                chunks: staged_file.chunks.iter().map(|c| ChunkRef {
                    hash: c.hash,
                    offset: c.offset,
                    size: c.size,
                }).collect(),
                metadata: FileMetadata {
                    mode: 0o644,
                    modified: staged_file.modified_time
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0),
                    is_new: true,
                    is_modified: false,
                    is_deleted: false,
                },
            };
            
            layer.add_file(file_entry);
            
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

        // Write layer to disk with scrambling
        let layer_filename = format!("{}.layer", layer_id.to_hex());
        let layer_path = self.archive.path().parent()
            .ok_or_else(|| DigstoreError::internal("Invalid archive path"))?
            .join(layer_filename);
        
        // Create URN for this layer
        let layer_urn = crate::urn::Urn {
            store_id: self.store_id,
            root_hash: Some(layer_id),
            resource_path: None,
            byte_range: None,
        };
        
        // Serialize layer to bytes
        let layer_data = layer.serialize_to_bytes()?;
        
        // Add layer to archive
        self.archive.add_layer(layer_id, &layer_data)?;

        // Update root history in Layer 0
        self.update_root_history(layer_id)?;

        // Update current root
        self.current_root = Some(layer_id);

        // Close memory maps before clearing (Windows file locking fix)
        self.staging.mmap = None;
        self.staging.mmap_mut = None;
        
        // Clear binary staging
        self.staging.clear()?;

        Ok(layer_id)
    }

    /// Load a layer by its ID using secure operations
    pub fn load_layer(&self, layer_id: LayerId) -> Result<Layer> {
        // Load layer from archive
        self.archive.get_layer(&layer_id)
    }

    /// Get a file by path from the latest commit
    pub fn get_file(&self, file_path: &Path) -> Result<Vec<u8>> {
        // First check if file is in staging
        if let Some(staged_file) = self.staging.get_staged_file(file_path)? {
            // For binary staging, we need to reconstruct the file from chunks
            // This is a simplified version - in practice, chunks would be stored separately
            let mut file_data = Vec::with_capacity(staged_file.size as usize);
            for chunk in &staged_file.chunks {
                // In real implementation, we'd read chunk data from storage
                // For now, return empty data as placeholder
                file_data.extend_from_slice(&vec![0u8; chunk.size as usize]);
            }
            return Ok(file_data);
        }

        // Get from committed layers
        self.get_file_at(file_path, self.current_root)
    }

    /// Get a file at a specific root hash
    pub fn get_file_at(&self, file_path: &Path, root_hash: Option<RootHash>) -> Result<Vec<u8>> {
        let target_root = root_hash.unwrap_or(
            self.current_root.ok_or_else(|| DigstoreError::file_not_found(file_path.to_path_buf()))?
        );

        // Load layer from archive
        let layer = self.archive.get_layer(&target_root)?;

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
    pub fn status(&mut self) -> StoreStatus {
        // Ensure staging area is reloaded to see latest changes
        let _ = self.staging.reload();
        
        let staged_files = self.staging.get_all_staged_files()
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.path)
            .collect();
            
        let total_staged_size = self.staging.get_all_staged_files()
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.size)
            .sum();
            
        StoreStatus {
            store_id: self.store_id,
            current_root: self.current_root,
            staged_files,
            total_staged_size,
        }
    }

    /// Check if a file is staged
    pub fn is_file_staged(&self, path: &Path) -> bool {
        self.staging.is_staged(path)
    }

    /// Get the global path (archive directory) for backward compatibility
    pub fn global_path(&self) -> PathBuf {
        self.archive.path().parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }

    /// Remove a file from staging
    /// Note: Binary staging doesn't support individual file removal efficiently
    /// For now, this is not implemented - use clear_staging() to remove all files
    pub fn unstage_file(&mut self, _path: &Path) -> Result<()> {
        // TODO: Implement efficient individual file removal for binary staging
        Err(DigstoreError::internal("Individual file unstaging not yet implemented for binary staging. Use clear_staging() to remove all files."))
    }

    /// Clear all staged files
    pub fn clear_staging(&mut self) -> Result<()> {
        self.staging.clear()?;
        Ok(())
    }

    /// Initialize Layer 0 with metadata
    fn init_layer_zero(&mut self) -> Result<()> {
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
        
        // Add Layer 0 to archive
        let layer_zero_hash = Hash::zero(); // Use all zeros for Layer 0
        self.archive.add_layer(layer_zero_hash, &metadata_bytes)?;
        
        Ok(())
    }

    /// Load current root hash from Layer 0 in archive
    fn load_current_root_from_archive(archive: &DigArchive) -> Result<Option<RootHash>> {
        let layer_zero_hash = Hash::zero();
        
        if !archive.has_layer(&layer_zero_hash) {
            return Ok(None);
        }
        
        // Get Layer 0 metadata
        let metadata_bytes = archive.get_layer_data(&layer_zero_hash)?;
        let metadata: serde_json::Value = serde_json::from_slice(&metadata_bytes)?;
        
        if let Some(root_history) = metadata.get("root_history").and_then(|v| v.as_array()) {
            if let Some(latest_root) = root_history.last() {
                if let Some(root_str) = latest_root.as_str() {
                    if let Ok(root_hash) = Hash::from_hex(root_str) {
                        return Ok(Some(root_hash));
                    }
                }
            }
        }
        
        Ok(None)
    }

    /// Load current root hash from Layer 0 (legacy directory format)
    fn load_current_root(global_path: &Path) -> Result<Option<RootHash>> {
        let layer_zero_path = global_path.join("0000000000000000000000000000000000000000000000000000000000000000.layer");
        
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
    // Removed duplicate global_path method - using the one that returns PathBuf

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
    
    let dig_dir = user_dirs.home_dir().join(".layer");
    Ok(dig_dir.join(store_id.to_hex()))
}

/// Get the global .layer directory
pub fn get_global_dig_directory() -> Result<PathBuf> {
    let user_dirs = UserDirs::new()
        .ok_or(DigstoreError::HomeDirectoryNotFound)?;
    
    Ok(user_dirs.home_dir().join(".layer"))
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
        // Count layers in archive (excluding Layer 0)
        let layer_count = self.archive.list_layers()
            .into_iter()
            .filter(|(hash, _)| *hash != Hash::zero()) // Exclude Layer 0
            .count();
        
        Ok(layer_count as u64 + 1)
    }

    /// Update root history in Layer 0
    fn update_root_history(&mut self, new_root: RootHash) -> Result<()> {
        let layer_zero_hash = Hash::zero();
        
        // Get current Layer 0 metadata
        let mut metadata: serde_json::Value = if self.archive.has_layer(&layer_zero_hash) {
            let content = self.archive.get_layer_data(&layer_zero_hash)?;
            serde_json::from_slice(&content)?
        } else {
            // Create new metadata if Layer 0 doesn't exist
            serde_json::json!({
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
            })
        };
        
        // Add new root to history
        if let Some(root_history) = metadata.get_mut("root_history").and_then(|v| v.as_array_mut()) {
            root_history.push(serde_json::json!({
                "generation": root_history.len(),
                "root_hash": new_root.to_hex(),
                "timestamp": chrono::Utc::now().timestamp(),
                "layer_count": root_history.len() + 1
            }));
        }
        
        // Update Layer 0 in archive
        let updated_content = serde_json::to_vec_pretty(&metadata)?;
        self.archive.add_layer(layer_zero_hash, &updated_content)?;
        
        Ok(())
    }

    // Binary staging methods removed - now handled by BinaryStagingArea
    
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
