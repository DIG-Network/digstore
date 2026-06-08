//! CLI execution context: where the store lives, output mode.

use std::path::PathBuf;

use digstore_core::{Bytes32, StoreConfig};

use crate::error::CliError;

#[derive(Debug, Clone)]
pub struct CliContext {
    pub dig_dir: PathBuf,
    pub json: bool,
    pub verbose: bool,
}

impl CliContext {
    pub fn resolve(explicit: Option<PathBuf>, json: bool, verbose: bool) -> Self {
        let dig_dir = explicit.unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".dig")
        });
        CliContext {
            dig_dir,
            json,
            verbose,
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.dig_dir.join("config.toml")
    }

    pub fn load_config(&self) -> Result<StoreConfig, CliError> {
        let path = self.config_path();
        if !path.exists() {
            return Err(CliError::NoStore(self.dig_dir.display().to_string()));
        }
        digstore_store::load_config(&path)
            .map_err(|e| CliError::Other(anyhow::anyhow!("load config: {e}")))
    }

    pub fn find_store_id(&self) -> Result<Bytes32, CliError> {
        Ok(self.load_config()?.store_id)
    }

    pub fn modules_dir(&self) -> PathBuf {
        self.dig_dir.join("modules")
    }

    pub fn generations_dir(&self) -> PathBuf {
        self.dig_dir.join("generations")
    }

    pub fn staging_path(&self, store_id: &Bytes32) -> PathBuf {
        self.dig_dir
            .join(format!("{}.staging.bin", store_id.to_hex()))
    }

    /// Path of the append-only root history (`roots.log`), matching the store.
    pub fn history_path(&self) -> PathBuf {
        self.dig_dir.join("roots.log")
    }

    pub fn salt_path(&self) -> PathBuf {
        self.dig_dir.join("secret_salt.hex")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn explicit_dig_dir_is_used_verbatim() {
        let td = tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
        assert_eq!(ctx.dig_dir, td.path());
    }

    #[test]
    fn config_toml_path_is_under_dig_dir() {
        let td = tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
        assert_eq!(ctx.config_path(), td.path().join("config.toml"));
    }

    #[test]
    fn find_store_id_errors_when_no_config() {
        let td = tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
        assert!(ctx.find_store_id().is_err());
    }
}
