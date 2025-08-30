//! Store management for Digstore Min

use crate::core::{types::*, error::*, digstore_file::DigstoreFile};
use std::path::{Path, PathBuf};
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

        Ok(Self {
            store_id,
            global_path,
            project_path: Some(project_path.to_path_buf()),
            current_root,
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

        Ok(Self {
            store_id: *store_id,
            global_path,
            project_path: None,
            current_root,
        })
    }

    /// Add files to staging
    pub fn add_files(&self, _paths: &[&str]) -> Result<()> {
        // TODO: Implement file adding
        todo!("Store::add_files not yet implemented")
    }

    /// Create a commit
    pub fn commit(&self, _message: &str) -> Result<LayerId> {
        // TODO: Implement commit creation
        todo!("Store::commit not yet implemented")
    }

    /// Get a file by path
    pub fn get_file(&self, _path: &Path) -> Result<Vec<u8>> {
        // TODO: Implement file retrieval
        todo!("Store::get_file not yet implemented")
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
            if let Some(latest_root) = root_history.last().and_then(|v| v.as_str()) {
                return Ok(Some(Hash::from_hex(latest_root)
                    .map_err(|_| DigstoreError::store_corrupted("Invalid root hash in Layer 0"))?));
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
