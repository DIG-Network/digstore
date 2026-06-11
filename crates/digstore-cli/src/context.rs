//! CLI execution context: where the store lives, output mode.

use std::path::PathBuf;

use digstore_core::{Bytes32, StoreConfig};

use crate::error::CliError;

#[derive(Debug, Clone)]
pub struct CliContext {
    /// Per-store directory: `.dig/stores/<name>/` (workspace dir for workspace-only cmds).
    pub dig_dir: PathBuf,
    /// The `.dig/` workspace directory (skip target for walks; home of workspace.toml).
    pub workspace_dir: PathBuf,
    /// Resolved operating directory for `add`/`urn`/`status` scans (§2.8).
    pub op_dir: PathBuf,
    /// Selected store name, when a store is resolved (None for workspace-only cmds).
    pub store_name: Option<String>,
    pub json: bool,
    pub verbose: bool,
}

impl CliContext {
    /// Discover the `.dig/` workspace by walking up from CWD (or `explicit`).
    pub fn discover_workspace(explicit: Option<PathBuf>) -> PathBuf {
        explicit
            .or_else(Self::discover_dig_dir)
            .unwrap_or_else(Self::cwd_dig_dir)
    }

    /// Workspace for `init`: anchored to CWD/.dig (or `explicit`), no walk-up.
    pub fn init_workspace(explicit: Option<PathBuf>) -> PathBuf {
        explicit.unwrap_or_else(Self::cwd_dig_dir)
    }

    /// Per-store context with op_dir defaulting to CWD (used before content_root is known).
    pub fn for_store(
        workspace_dir: PathBuf,
        name: &str,
        cwd_flag: Option<PathBuf>,
        cwd: PathBuf,
        json: bool,
        verbose: bool,
    ) -> Self {
        Self::for_store_with_op(workspace_dir, name, None, cwd_flag, cwd, json, verbose)
    }

    /// Per-store context resolving op_dir per §2.8: cwd_flag > content_root(joined to project root) > cwd.
    #[allow(clippy::too_many_arguments)]
    pub fn for_store_with_op(
        workspace_dir: PathBuf,
        name: &str,
        content_root: Option<String>,
        cwd_flag: Option<PathBuf>,
        cwd: PathBuf,
        json: bool,
        verbose: bool,
    ) -> Self {
        let dig_dir = workspace_dir.join("stores").join(name);
        let project_root = workspace_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| cwd.clone());
        let op_dir = match cwd_flag {
            Some(p) if p.is_absolute() => p,
            Some(p) => cwd.join(p),
            None => match content_root {
                Some(cr) => project_root.join(cr),
                None => cwd,
            },
        };
        CliContext {
            dig_dir,
            workspace_dir,
            op_dir,
            store_name: Some(name.to_string()),
            json,
            verbose,
        }
    }

    /// Workspace-only context (stores/use): no store resolved.
    pub fn workspace_only(workspace_dir: PathBuf, json: bool, verbose: bool) -> Self {
        CliContext {
            dig_dir: workspace_dir.clone(),
            workspace_dir,
            op_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            store_name: None,
            json,
            verbose,
        }
    }

    /// Backward-compatible single-store resolver (pre-multistore API). Treats the
    /// explicit path (or the discovered/CWD `.dig`) as BOTH the workspace dir and
    /// the store `dig_dir`, with an implicit `"default"` store. Retained so the
    /// standalone D6 integration tests (`dighost_serve`, `adv_self_serve`) and any
    /// other single-store driver keep working while the multi-store dispatch
    /// (which uses `discover_workspace`/`for_store`) is wired up.
    pub fn resolve(explicit: Option<PathBuf>, json: bool, verbose: bool) -> Self {
        let dir = explicit
            .or_else(Self::discover_dig_dir)
            .unwrap_or_else(Self::cwd_dig_dir);
        Self::single_store(dir, json, verbose)
    }

    /// Backward-compatible single-store resolver for `init`: anchored to the
    /// explicit path (or `<cwd>/.dig`), no walk-up. Companion to [`resolve`].
    pub fn resolve_init(explicit: Option<PathBuf>, json: bool, verbose: bool) -> Self {
        let dir = explicit.unwrap_or_else(Self::cwd_dig_dir);
        Self::single_store(dir, json, verbose)
    }

    /// Build a self-contained single-store context where the store lives directly
    /// at `dir` (`dig_dir == workspace_dir == op_dir`), with an implicit
    /// `"default"` store name. Matches the pre-multistore on-disk layout.
    fn single_store(dir: PathBuf, json: bool, verbose: bool) -> Self {
        CliContext {
            dig_dir: dir.clone(),
            workspace_dir: dir.clone(),
            op_dir: dir,
            store_name: Some("default".to_string()),
            json,
            verbose,
        }
    }

    /// `<current working directory>/.dig` (absolute, since `current_dir` is).
    fn cwd_dig_dir() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".dig")
    }

    /// Walk up from the current working directory; return the nearest ancestor's
    /// `.dig` directory if one exists (Git-style repository discovery).
    fn discover_dig_dir() -> Option<PathBuf> {
        let mut dir = std::env::current_dir().ok()?;
        loop {
            let candidate = dir.join(".dig");
            if candidate.is_dir() {
                return Some(candidate);
            }
            if !dir.pop() {
                return None;
            }
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

    /// Path of the CLI-owned on-chain anchor state (`anchor.toml`). Sibling of
    /// the core-owned `config.toml`; the CLI owns this file end-to-end.
    pub fn anchor_path(&self) -> PathBuf {
        self.dig_dir.join("anchor.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn for_store_points_dig_dir_at_store_subdir() {
        let dir = TempDir::new().unwrap();
        let dig = dir.path().join(".dig");
        let ctx = CliContext::for_store(
            dig.clone(),
            "site",
            None,
            dir.path().to_path_buf(),
            false,
            false,
        );
        assert_eq!(ctx.dig_dir, dig.join("stores").join("site"));
        assert_eq!(ctx.workspace_dir, dig);
        assert_eq!(ctx.store_name.as_deref(), Some("site"));
    }

    #[test]
    fn op_dir_precedence_cwd_flag_over_content_root() {
        let dir = TempDir::new().unwrap();
        let project = dir.path();
        let dig = project.join(".dig");
        // content_root = "dist" -> op_dir = project/dist
        let a = CliContext::for_store_with_op(
            dig.clone(),
            "s",
            Some("dist".into()),
            None,
            project.to_path_buf(),
            false,
            false,
        );
        assert_eq!(a.op_dir, project.join("dist"));
        // -C override wins (absolute)
        let abs = project.join("other");
        let b = CliContext::for_store_with_op(
            dig.clone(),
            "s",
            Some("dist".into()),
            Some(abs.clone()),
            project.to_path_buf(),
            false,
            false,
        );
        assert_eq!(b.op_dir, abs);
    }

    #[test]
    fn config_toml_path_is_under_dig_dir() {
        let td = TempDir::new().unwrap();
        let ctx = CliContext::workspace_only(td.path().to_path_buf(), false, false);
        assert_eq!(ctx.config_path(), td.path().join("config.toml"));
    }

    #[test]
    fn find_store_id_errors_when_no_config() {
        let td = TempDir::new().unwrap();
        let ctx = CliContext::workspace_only(td.path().to_path_buf(), false, false);
        assert!(ctx.find_store_id().is_err());
    }
}
