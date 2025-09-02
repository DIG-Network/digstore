//! Test utilities for Digstore Min tests

use digstore_min::storage::Store;
use std::path::Path;
use tempfile::TempDir;

/// Create a test store with automatic cleanup
pub struct TestStore {
    pub store: Store,
    pub temp_dir: TempDir,
    store_id: digstore_min::core::types::StoreId,
}

impl TestStore {
    /// Create a new test store
    pub fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let store = Store::init(temp_dir.path())?;
        let store_id = store.store_id();

        Ok(Self {
            store,
            temp_dir,
            store_id,
        })
    }

    /// Get reference to the store
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Get mutable reference to the store
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Get the temporary directory path
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }
}

impl Drop for TestStore {
    fn drop(&mut self) {
        // Clean up the global store directory
        let global_store_path = get_global_store_path(&self.store_id);
        if let Ok(path) = global_store_path {
            let _ = std::fs::remove_dir_all(path);
        }

        // Clean up any staging files
        let staging_path = self.store.global_path().join("staging.json");
        let _ = std::fs::remove_file(staging_path);
    }
}

/// Get the global store path for cleanup
fn get_global_store_path(
    store_id: &digstore_min::core::types::StoreId,
) -> anyhow::Result<std::path::PathBuf> {
    use directories::UserDirs;

    let user_dirs =
        UserDirs::new().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;

    let dig_dir = user_dirs.home_dir().join(".layer");
    Ok(dig_dir.join(store_id.to_hex()))
}

/// Clean up all test stores in ~/.layer/ (for emergency cleanup)
pub fn cleanup_all_test_stores() -> anyhow::Result<()> {
    use directories::UserDirs;

    let user_dirs =
        UserDirs::new().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;

    let dig_dir = user_dirs.home_dir().join(".layer");

    if dig_dir.exists() {
        // Only remove directories that look like test stores (recent creation)
        for entry in std::fs::read_dir(&dig_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(created) = metadata.created() {
                        let age = created
                            .elapsed()
                            .unwrap_or(std::time::Duration::from_secs(0));
                        // Remove stores created in the last hour (likely test stores)
                        if age < std::time::Duration::from_secs(3600) {
                            let _ = std::fs::remove_dir_all(entry.path());
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
