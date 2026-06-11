//! Global digstore config and `~/.dig` path resolution.

use crate::error::{ChainError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default coinset.org endpoint (used by the anchoring subsystem).
pub const DEFAULT_COINSET_URL: &str = "https://api.coinset.org";
/// Default cached-unlock TTL in seconds (1 hour).
pub const DEFAULT_UNLOCK_TTL: u64 = 3600;

/// Resolves the global `~/.dig` directory.
///
/// Honors the `DIGSTORE_HOME` environment variable (used by tests and for
/// relocating the home dir); otherwise `<home>/.dig`.
pub fn dig_home() -> Result<PathBuf> {
    if let Some(over) = std::env::var_os("DIGSTORE_HOME") {
        return Ok(PathBuf::from(over));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| ChainError::Config("could not resolve home directory".into()))?;
    Ok(home.join(".dig"))
}

pub fn seed_path(home: &Path) -> PathBuf {
    home.join("seed.enc")
}
pub fn session_path(home: &Path) -> PathBuf {
    home.join("session")
}
pub fn config_path(home: &Path) -> PathBuf {
    home.join("config.toml")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub coinset_url: String,
    pub unlock_ttl: u64,
    pub fee: u64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        GlobalConfig {
            coinset_url: DEFAULT_COINSET_URL.to_string(),
            unlock_ttl: DEFAULT_UNLOCK_TTL,
            fee: 0,
        }
    }
}

impl GlobalConfig {
    /// Loads config from `<home>/config.toml`, or returns defaults if absent.
    pub fn load(home: &Path) -> Result<Self> {
        let path = config_path(home);
        if !path.exists() {
            return Ok(GlobalConfig::default());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).map_err(|e| ChainError::Config(e.to_string()))
    }

    /// Writes config to `<home>/config.toml`, creating the dir if needed.
    pub fn save(&self, home: &Path) -> Result<()> {
        std::fs::create_dir_all(home)?;
        let text = toml::to_string_pretty(self).map_err(|e| ChainError::Config(e.to_string()))?;
        std::fs::write(config_path(home), text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let c = GlobalConfig::default();
        assert_eq!(c.coinset_url, "https://api.coinset.org");
        assert_eq!(c.unlock_ttl, 3600);
        assert_eq!(c.fee, 0);
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let c = GlobalConfig::load(dir.path()).unwrap();
        assert_eq!(c, GlobalConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let c = GlobalConfig { coinset_url: "https://example.org".into(), unlock_ttl: 60, fee: 5 };
        c.save(dir.path()).unwrap();
        let loaded = GlobalConfig::load(dir.path()).unwrap();
        assert_eq!(loaded, c);
    }

    #[test]
    fn dig_home_honors_env_override() {
        std::env::set_var("DIGSTORE_HOME", "/tmp/digstore-test-home");
        let h = dig_home().unwrap();
        assert_eq!(h, PathBuf::from("/tmp/digstore-test-home"));
        std::env::remove_var("DIGSTORE_HOME");
    }

    #[test]
    fn path_helpers_join_filenames() {
        let h = Path::new("/x/.dig");
        assert_eq!(seed_path(h), PathBuf::from("/x/.dig/seed.enc"));
        assert_eq!(session_path(h), PathBuf::from("/x/.dig/session"));
        assert_eq!(config_path(h), PathBuf::from("/x/.dig/config.toml"));
    }
}
