use crate::error::{Result, StoreError};
use digstore_core::{Bytes32, SecretSalt, StoreConfig, Visibility};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// TOML-friendly mirror of `StoreConfig`. Visibility is flattened to a string
/// tag plus an optional hex salt so the file stays human-readable.
#[derive(Debug, Serialize, Deserialize)]
struct ConfigToml {
    store_id: String,
    data_dir: String,
    max_size: u64,
    visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_salt: Option<String>,
}

impl ConfigToml {
    fn from_config(cfg: &StoreConfig) -> Self {
        let (visibility, secret_salt) = match &cfg.visibility {
            Visibility::Public => ("public".to_string(), None),
            Visibility::Private(salt) => ("private".to_string(), Some(hex::encode(salt.0))),
        };
        Self {
            store_id: cfg.store_id.to_hex(),
            data_dir: cfg.data_dir.clone(),
            max_size: cfg.max_size,
            visibility,
            secret_salt,
        }
    }

    fn into_config(self) -> Result<StoreConfig> {
        let store_id = Bytes32::from_hex(&self.store_id).map_err(|_| {
            StoreError::InvalidConfig(format!("bad store_id hex: {}", self.store_id))
        })?;
        let visibility = match self.visibility.as_str() {
            "public" => Visibility::Public,
            "private" => {
                let salt_hex = self.secret_salt.ok_or_else(|| {
                    StoreError::InvalidConfig("private store missing secret_salt".into())
                })?;
                let bytes = hex::decode(&salt_hex)
                    .map_err(|_| StoreError::InvalidConfig("bad secret_salt hex".into()))?;
                let arr: [u8; 32] = bytes.try_into().map_err(|_| {
                    StoreError::InvalidConfig("secret_salt must be 32 bytes".into())
                })?;
                Visibility::Private(SecretSalt(arr))
            }
            other => {
                return Err(StoreError::InvalidConfig(format!(
                    "unknown visibility: {other}"
                )))
            }
        };
        Ok(StoreConfig {
            store_id,
            data_dir: self.data_dir,
            max_size: self.max_size,
            visibility,
        })
    }
}

/// Serialize a `StoreConfig` to `config.toml` at `path`.
pub fn save_config(path: impl AsRef<Path>, cfg: &StoreConfig) -> Result<()> {
    let toml_repr = ConfigToml::from_config(cfg);
    let text = toml::to_string_pretty(&toml_repr).map_err(|e| StoreError::Config(e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Load a `StoreConfig` from a `config.toml` at `path`.
pub fn load_config(path: impl AsRef<Path>) -> Result<StoreConfig> {
    let text = std::fs::read_to_string(path)?;
    let toml_repr: ConfigToml =
        toml::from_str(&text).map_err(|e| StoreError::Config(e.to_string()))?;
    toml_repr.into_config()
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::{Bytes32, SecretSalt, StoreConfig, Visibility};
    use tempfile::tempdir;

    fn public_cfg() -> StoreConfig {
        StoreConfig {
            store_id: Bytes32([0x22u8; 32]),
            data_dir: "/data".to_string(),
            max_size: 1_000_000,
            visibility: Visibility::Public,
        }
    }

    #[test]
    fn public_config_roundtrips_through_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = public_cfg();
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.store_id, cfg.store_id);
        assert_eq!(loaded.data_dir, "/data");
        assert_eq!(loaded.max_size, 1_000_000);
        assert!(matches!(loaded.visibility, Visibility::Public));
    }

    #[test]
    fn private_config_preserves_salt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = public_cfg();
        cfg.visibility = Visibility::Private(SecretSalt([0x07u8; 32]));
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        match loaded.visibility {
            Visibility::Private(salt) => assert_eq!(salt.0, [0x07u8; 32]),
            Visibility::Public => panic!("expected private visibility"),
        }
    }

    #[test]
    fn config_toml_is_human_readable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save_config(&path, &public_cfg()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("store_id = \""));
        assert!(text.contains("visibility = \"public\""));
        assert!(text.contains(&"22".repeat(32)));
    }

    #[test]
    fn private_without_salt_is_invalid_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "store_id = \"22\"\ndata_dir = \"/data\"\nmax_size = 1\nvisibility = \"private\"\n",
        )
        .unwrap();
        let err = load_config(&path).unwrap_err();
        assert!(matches!(err, StoreError::InvalidConfig(_)));
    }
}
