//! Per-store on-chain anchor state, persisted to `<dig_dir>/anchor.toml`.
//!
//! This file is CLI-owned (a sibling of the core-owned `config.toml`): the
//! `digstore-store` `save_config` truncates/rewrites `config.toml` and only
//! knows `StoreConfig` fields, so anchor metadata lives here where the CLI owns
//! it end-to-end.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// Confirmation status of the store's current singleton coin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorStatus {
    Pending,
    Confirmed,
}

/// On-chain anchor state for a single store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorState {
    /// Chain network; always `"mainnet"` for now.
    pub network: String,
    /// Store id == singleton launcher id, hex (no `0x`).
    pub store_id: String,
    /// Current singleton coin id, hex.
    pub coin_id: String,
    /// Confirmation status of `coin_id`.
    pub status: AnchorStatus,
    /// Last root anchored on-chain, hex (empty for a freshly-minted empty store).
    pub last_root: String,
    /// Transaction / spend coin id, hex; may be empty.
    pub last_tx_id: String,
    /// Block height at which `coin_id` confirmed; 0 until confirmed.
    pub confirmed_height: u32,
}

impl AnchorState {
    /// Path of the anchor state file under `dig_dir`.
    pub fn path(dig_dir: &Path) -> PathBuf {
        dig_dir.join("anchor.toml")
    }

    /// Load anchor state from `<dig_dir>/anchor.toml`, or `None` if the file is absent.
    pub fn load(dig_dir: &Path) -> Result<Option<AnchorState>, CliError> {
        let path = Self::path(dig_dir);
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CliError::Other(anyhow::anyhow!("read anchor.toml: {e}")))?;
        let state = toml::from_str(&text)
            .map_err(|e| CliError::Other(anyhow::anyhow!("parse anchor.toml: {e}")))?;
        Ok(Some(state))
    }

    /// Persist this anchor state to `<dig_dir>/anchor.toml` (pretty TOML).
    pub fn save(&self, dig_dir: &Path) -> Result<(), CliError> {
        let path = Self::path(dig_dir);
        let text = toml::to_string_pretty(self)
            .map_err(|e| CliError::Other(anyhow::anyhow!("serialize anchor.toml: {e}")))?;
        std::fs::write(&path, text)
            .map_err(|e| CliError::Other(anyhow::anyhow!("write anchor.toml: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> AnchorState {
        AnchorState {
            network: "mainnet".into(),
            store_id: "aa".repeat(32),
            coin_id: "bb".repeat(32),
            status: AnchorStatus::Pending,
            last_root: String::new(),
            last_tx_id: "cc".repeat(32),
            confirmed_height: 0,
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let td = TempDir::new().unwrap();
        let s = sample();
        s.save(td.path()).unwrap();
        let loaded = AnchorState::load(td.path()).unwrap().unwrap();
        assert_eq!(loaded, s);
    }

    #[test]
    fn load_missing_returns_none() {
        let td = TempDir::new().unwrap();
        assert!(AnchorState::load(td.path()).unwrap().is_none());
    }

    #[test]
    fn status_serializes_as_lowercase() {
        let pending = toml::to_string(&sample()).unwrap();
        assert!(pending.contains("status = \"pending\""));
        let confirmed = AnchorState {
            status: AnchorStatus::Confirmed,
            confirmed_height: 42,
            ..sample()
        };
        let text = toml::to_string(&confirmed).unwrap();
        assert!(text.contains("status = \"confirmed\""));
    }

    #[test]
    fn status_deserializes_both_ways() {
        let td = TempDir::new().unwrap();
        for (status, height) in [(AnchorStatus::Pending, 0), (AnchorStatus::Confirmed, 7)] {
            let s = AnchorState {
                status,
                confirmed_height: height,
                ..sample()
            };
            s.save(td.path()).unwrap();
            let loaded = AnchorState::load(td.path()).unwrap().unwrap();
            assert_eq!(loaded.status, status);
        }
    }

    #[test]
    fn path_is_anchor_toml_under_dig_dir() {
        let td = TempDir::new().unwrap();
        assert_eq!(
            AnchorState::path(td.path()),
            td.path().join("anchor.toml")
        );
    }
}
