//! The `.dig/` workspace: a registry of named stores plus the active selection.
//! CLI-owned (the store/core crates know nothing about names or content roots).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspaceToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default)]
    stores: BTreeMap<String, StoreEntryToml>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreEntryToml {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_root: Option<String>,
}

/// Runtime view of `workspace.toml`.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub dir: PathBuf, // the .dig/ directory
    pub active: Option<String>,
    pub stores: BTreeMap<String, StoreEntry>,
}

#[derive(Debug, Clone)]
pub struct StoreEntry {
    pub id: String,
    pub content_root: Option<String>,
}

/// Store names: non-empty, only `[A-Za-z0-9._-]`, not `.`/`..`, no separators.
pub fn validate_store_name(name: &str) -> Result<(), CliError> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(CliError::InvalidArgument(format!(
            "invalid store name '{name}': use letters, digits, '.', '_', '-' (no path separators)"
        )))
    }
}

impl Workspace {
    fn toml_path(dir: &Path) -> PathBuf {
        dir.join("workspace.toml")
    }

    /// Load an existing workspace (workspace.toml must exist).
    pub fn load(dir: &Path) -> Result<Self, CliError> {
        let path = Self::toml_path(dir);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CliError::Other(anyhow::anyhow!("read workspace.toml: {e}")))?;
        let wt: WorkspaceToml = toml::from_str(&text)
            .map_err(|e| CliError::Other(anyhow::anyhow!("parse workspace.toml: {e}")))?;
        Ok(Workspace {
            dir: dir.to_path_buf(),
            active: wt.active,
            stores: wt
                .stores
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        StoreEntry {
                            id: v.id,
                            content_root: v.content_root,
                        },
                    )
                })
                .collect(),
        })
    }

    /// Load, migrating a legacy single-store `.dig/` (config.toml at the root,
    /// no `stores/`, no workspace.toml) into `stores/default/` first.
    pub fn load_or_migrate(dir: &Path) -> Result<Self, CliError> {
        if Self::toml_path(dir).exists() {
            return Self::load(dir);
        }
        if dir.join("config.toml").exists() && !dir.join("stores").exists() {
            return Self::migrate_legacy(dir);
        }
        // Fresh/empty workspace.
        Ok(Workspace {
            dir: dir.to_path_buf(),
            active: None,
            stores: BTreeMap::new(),
        })
    }

    fn migrate_legacy(dir: &Path) -> Result<Self, CliError> {
        let dest = dir.join("stores").join("default");
        std::fs::create_dir_all(&dest)
            .map_err(|e| CliError::Other(anyhow::anyhow!("migrate mkdir: {e}")))?;
        // Move every entry currently directly under .dig/ (except the new stores/ dir)
        // into stores/default/.
        for entry in
            std::fs::read_dir(dir).map_err(|e| CliError::Other(anyhow::anyhow!("migrate scan: {e}")))?
        {
            let entry =
                entry.map_err(|e| CliError::Other(anyhow::anyhow!("migrate entry: {e}")))?;
            let name = entry.file_name();
            if name == "stores" || name == "workspace.toml" {
                continue;
            }
            let to = dest.join(&name);
            std::fs::rename(entry.path(), &to)
                .map_err(|e| CliError::Other(anyhow::anyhow!("migrate move {name:?}: {e}")))?;
        }
        let cfg = digstore_store::load_config(dest.join("config.toml"))
            .map_err(|e| CliError::Other(anyhow::anyhow!("migrate read config: {e}")))?;
        let mut ws = Workspace {
            dir: dir.to_path_buf(),
            active: None,
            stores: BTreeMap::new(),
        };
        ws.register("default", &cfg.store_id.to_hex(), None)?;
        ws.set_active("default")?;
        ws.save()?;
        Ok(ws)
    }

    pub fn save(&self) -> Result<(), CliError> {
        let wt = WorkspaceToml {
            active: self.active.clone(),
            stores: self
                .stores
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        StoreEntryToml {
                            id: v.id.clone(),
                            content_root: v.content_root.clone(),
                        },
                    )
                })
                .collect(),
        };
        let text = toml::to_string_pretty(&wt)
            .map_err(|e| CliError::Other(anyhow::anyhow!("serialize workspace.toml: {e}")))?;
        std::fs::write(Self::toml_path(&self.dir), text)
            .map_err(|e| CliError::Other(anyhow::anyhow!("write workspace.toml: {e}")))
    }

    pub fn register(
        &mut self,
        name: &str,
        id_hex: &str,
        content_root: Option<String>,
    ) -> Result<(), CliError> {
        validate_store_name(name)?;
        if self.stores.contains_key(name) {
            return Err(CliError::InvalidArgument(format!(
                "store '{name}' already exists"
            )));
        }
        self.stores.insert(
            name.to_string(),
            StoreEntry {
                id: id_hex.to_string(),
                content_root,
            },
        );
        Ok(())
    }

    pub fn set_active(&mut self, name: &str) -> Result<(), CliError> {
        if !self.stores.contains_key(name) {
            return Err(CliError::InvalidArgument(format!("unknown store '{name}'")));
        }
        self.active = Some(name.to_string());
        Ok(())
    }

    pub fn set_content_root(&mut self, name: &str, root: Option<String>) -> Result<(), CliError> {
        let e = self
            .stores
            .get_mut(name)
            .ok_or_else(|| CliError::InvalidArgument(format!("unknown store '{name}'")))?;
        e.content_root = root;
        Ok(())
    }

    pub fn content_root(&self, name: &str) -> Option<String> {
        self.stores.get(name).and_then(|e| e.content_root.clone())
    }

    pub fn store_dir(&self, name: &str) -> PathBuf {
        self.dir.join("stores").join(name)
    }

    /// §2.3 precedence: explicit flag > active > single > error.
    pub fn resolve_store_name(&self, flag: Option<&str>) -> Result<String, CliError> {
        if let Some(name) = flag {
            if self.stores.contains_key(name) {
                return Ok(name.to_string());
            }
            return Err(CliError::InvalidArgument(format!("unknown store '{name}'")));
        }
        if let Some(active) = &self.active {
            if self.stores.contains_key(active) {
                return Ok(active.clone());
            }
        }
        if self.stores.len() == 1 {
            return Ok(self.stores.keys().next().unwrap().clone());
        }
        Err(CliError::InvalidArgument(
            "no store selected: use --store <name>, set one with `digstore use <name>`, or create one with `digstore init <name>`".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn id(n: u8) -> String {
        format!("{:064x}", n)
    }

    #[test]
    fn name_validation_accepts_and_rejects() {
        for ok in ["default", "site", "a.b_c-1"] {
            assert!(validate_store_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "a b"] {
            assert!(validate_store_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn toml_round_trip_preserves_active_and_content_root() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        std::fs::create_dir_all(&dig).unwrap();
        let mut ws = Workspace {
            dir: dig.clone(),
            active: None,
            stores: Default::default(),
        };
        ws.register("default", &id(1), None).unwrap();
        ws.register("site", &id(2), Some("dist".into())).unwrap();
        ws.set_active("site").unwrap();
        ws.save().unwrap();
        let re = Workspace::load(&dig).unwrap();
        assert_eq!(re.active.as_deref(), Some("site"));
        assert_eq!(re.content_root("site"), Some("dist".to_string()));
        assert_eq!(re.content_root("default"), None);
    }

    #[test]
    fn selection_precedence_flag_then_active_then_single_then_error() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        std::fs::create_dir_all(&dig).unwrap();
        let mut ws = Workspace {
            dir: dig,
            active: None,
            stores: Default::default(),
        };
        // none yet -> error
        assert!(ws.resolve_store_name(None).is_err());
        // single implicit
        ws.register("only", &id(1), None).unwrap();
        assert_eq!(ws.resolve_store_name(None).unwrap(), "only");
        // two -> needs active or flag
        ws.register("two", &id(2), None).unwrap();
        assert!(ws.resolve_store_name(None).is_err());
        ws.set_active("two").unwrap();
        assert_eq!(ws.resolve_store_name(None).unwrap(), "two");
        // explicit flag wins, unknown flag errors
        assert_eq!(ws.resolve_store_name(Some("only")).unwrap(), "only");
        assert!(ws.resolve_store_name(Some("nope")).is_err());
    }

    #[test]
    fn migrate_moves_legacy_single_store_into_default() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        std::fs::create_dir_all(&dig).unwrap();
        // legacy: config.toml directly under .dig, no stores/, no workspace.toml
        std::fs::write(
            dig.join("config.toml"),
            format!(
                "store_id = \"{}\"\ndata_dir = \".\"\nmax_size = 1000\nvisibility = \"public\"\n",
                id(7)
            ),
        )
        .unwrap();
        std::fs::write(dig.join("roots.log"), b"").unwrap();
        let ws = Workspace::load_or_migrate(&dig).unwrap();
        assert_eq!(ws.active.as_deref(), Some("default"));
        assert!(dig.join("stores/default/config.toml").exists());
        assert!(!dig.join("config.toml").exists());
        assert!(dig.join("workspace.toml").exists());
    }
}
