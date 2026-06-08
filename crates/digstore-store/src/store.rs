use crate::clock::Clock;
use crate::config::{load_config, save_config};
use crate::error::{Result, StoreError};
use crate::history::RootHistory;
use crate::paths::StorePaths;
use crate::staging::StagingArea;
use digstore_core::{Bytes32, GenerationState, StoreConfig};
use std::path::Path;

/// The host-side Store entity (§4). Owns the on-disk layout, staging, and
/// generations. Generic over a `Clock` so commit timestamps are injectable.
pub struct Store<C: Clock> {
    config: StoreConfig,
    paths: StorePaths,
    clock: C,
}

impl<C: Clock> std::fmt::Debug for Store<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("config", &self.config)
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl<C: Clock> Store<C> {
    /// Create a new store: write config + the §4.4 directory tree. Refuses to
    /// overwrite an existing store (presence of `config.toml`).
    pub fn init(config: StoreConfig, clock: C) -> Result<Self> {
        let paths = StorePaths::new(&config.data_dir, config.store_id);
        if paths.config_file().exists() {
            return Err(StoreError::AlreadyExists(paths.root().display().to_string()));
        }
        std::fs::create_dir_all(paths.root())?;
        std::fs::create_dir_all(paths.generations_dir())?;
        std::fs::create_dir_all(paths.modules_dir())?;
        save_config(paths.config_file(), &config)?;
        StagingArea::open(paths.staging_file())?;
        RootHistory::open(paths.history_file())?;
        Ok(Self { config, paths, clock })
    }

    /// Open an existing store rooted at `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>, clock: C) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let config_file = data_dir.join("config.toml");
        if !config_file.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        let config = load_config(&config_file)?;
        let paths = StorePaths::new(data_dir, config.store_id);
        Ok(Self { config, paths, clock })
    }

    pub fn store_id(&self) -> Bytes32 {
        self.config.store_id
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// All generation states, oldest first (§4.3 root history).
    pub fn root_history(&self) -> Result<Vec<GenerationState>> {
        RootHistory::open(self.paths.history_file())?.entries()
    }

    /// Stage raw bytes under an explicit resource key (§20.2).
    pub fn stage_file(&mut self, resource_key: &str, bytes: &[u8]) -> Result<()> {
        let mut staging = StagingArea::open(self.paths.staging_file())?;
        staging.append(resource_key, bytes)?;
        Ok(())
    }

    /// Stage a file from disk. The path relative to `base` becomes the resource
    /// key (forward-slash normalized); the file bytes are staged verbatim.
    pub fn add(&mut self, file: impl AsRef<Path>, base: impl AsRef<Path>) -> Result<()> {
        let file = file.as_ref();
        let base = base.as_ref();
        let rel = file
            .strip_prefix(base)
            .map_err(|_| StoreError::PathEscape(file.to_path_buf()))?;
        let resource_key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(file)?;
        self.stage_file(&resource_key, &bytes)
    }
}
