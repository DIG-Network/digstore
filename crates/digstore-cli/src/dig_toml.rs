//! The committable `dig.toml` project manifest.
//!
//! `dig.toml` lives at the project root, is safe to commit (it holds NO secrets),
//! and is the single source of project config shared across `new`, `dev`,
//! `deploy`, `link`, and the scaffolding templates. Every field is optional;
//! flags/env always override the file. The precedence is uniform across the CLI:
//!
//!   flags  >  env  >  dig.toml  >  built-in defaults
//!
//! The env layer is provided by [`DigToml::with_env`], which overlays the
//! `DIGSTORE_*` variables onto a file (env beats file, so a CI runner can pin a
//! value without editing the committed manifest). Resolvers then apply flags on
//! top of the result, completing `flags > env > file > default`.

use crate::error::CliError;

/// Project metadata embedded with a deployment (the dighub `Manifest` shape).
/// All fields optional; what is set is surfaced on the store's hub page.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct DigMetadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
}

impl DigMetadata {
    /// Whether any metadata field is set (an all-empty table is treated as absent).
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.license.is_none()
            && self.homepage.is_none()
            && self.repository.is_none()
            && self.keywords.as_ref().map(|k| k.is_empty()).unwrap_or(true)
    }
}

/// The parsed `dig.toml`. Accepts both kebab-case (`output-dir`) and snake_case
/// (`output_dir`) keys so a hand-edited file is forgiving.
#[derive(Debug, Default, serde::Deserialize)]
pub struct DigToml {
    #[serde(default, rename = "store-id", alias = "store_id")]
    pub store_id: Option<String>,
    #[serde(default, rename = "output-dir", alias = "output_dir")]
    pub output_dir: Option<String>,
    #[serde(default, rename = "build-command", alias = "build_command")]
    pub build_command: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, rename = "wait-timeout", alias = "wait_timeout")]
    pub wait_timeout: Option<u64>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub remote: Option<String>,
    /// Glob patterns to exclude from the deployment (in addition to `.digignore`/
    /// `.gitignore`). e.g. `["*.map", "node_modules/**"]`.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Whether the project is published as a PRIVATE (salted/encrypted) store. The
    /// secret recovery key itself NEVER lives in `dig.toml` (it is a credential —
    /// supplied via `--salt`/`DIGSTORE_STORE_SALT`); this is only the policy bit so
    /// `new`/`deploy` know which path the project intends.
    #[serde(default)]
    pub private: bool,
    /// Embedded project metadata (the dighub `Manifest` shape).
    #[serde(default)]
    pub metadata: DigMetadata,
}

impl DigToml {
    /// Read `dig.toml` from `dir`, if present. A missing file yields the default
    /// (all config can come from flags/env); a malformed file is a hard error so
    /// a typo never silently deploys the wrong thing.
    pub fn read(dir: &std::path::Path) -> Result<DigToml, CliError> {
        let path = dir.join("dig.toml");
        if !path.exists() {
            return Ok(DigToml::default());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CliError::Other(anyhow::anyhow!("read dig.toml: {e}")))?;
        toml::from_str(&text).map_err(|e| CliError::InvalidArgument(format!("parse dig.toml: {e}")))
    }

    /// Read `dig.toml` from `dir`, then overlay the `DIGSTORE_*` env layer on top
    /// (env beats the file). The single entry point for resolvers that want the
    /// uniform `flags > env > dig.toml > defaults` precedence: read with this, then
    /// apply flags last. Recognized env vars:
    ///   - `DIGSTORE_STORE_ID`     → `store-id`
    ///   - `DIGSTORE_OUTPUT_DIR`   → `output-dir`
    ///   - `DIGSTORE_BUILD_COMMAND`→ `build-command`
    ///   - `DIGSTORE_REMOTE`       → `remote`
    ///   - `DIGSTORE_NETWORK`      → `network`
    pub fn read_with_env(dir: &std::path::Path) -> Result<DigToml, CliError> {
        let mut file = Self::read(dir)?;
        file.with_env();
        Ok(file)
    }

    /// Overlay the `DIGSTORE_*` environment variables onto this manifest in place
    /// (a set, non-empty env var REPLACES the file value — env beats file). Pure
    /// w.r.t. flags; the caller layers flags on the result.
    pub fn with_env(&mut self) {
        fn env(name: &str) -> Option<String> {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        }
        if let Some(v) = env("DIGSTORE_STORE_ID") {
            self.store_id = Some(v);
        }
        if let Some(v) = env("DIGSTORE_OUTPUT_DIR") {
            self.output_dir = Some(v);
        }
        if let Some(v) = env("DIGSTORE_BUILD_COMMAND") {
            self.build_command = Some(v);
        }
        if let Some(v) = env("DIGSTORE_REMOTE") {
            self.remote = Some(v);
        }
        if let Some(v) = env("DIGSTORE_NETWORK") {
            self.network = Some(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_file_is_default() {
        let td = TempDir::new().unwrap();
        let cfg = DigToml::read(td.path()).unwrap();
        assert!(cfg.output_dir.is_none());
        assert!(cfg.store_id.is_none());
    }

    #[test]
    fn reads_kebab_and_snake_keys() {
        let td = TempDir::new().unwrap();
        std::fs::write(
            td.path().join("dig.toml"),
            "output-dir = \"dist\"\nbuild_command = \"npm run build\"\n",
        )
        .unwrap();
        let cfg = DigToml::read(td.path()).unwrap();
        assert_eq!(cfg.output_dir.as_deref(), Some("dist"));
        assert_eq!(cfg.build_command.as_deref(), Some("npm run build"));
    }

    #[test]
    fn malformed_file_errors() {
        let td = TempDir::new().unwrap();
        std::fs::write(td.path().join("dig.toml"), "this = = not toml").unwrap();
        assert!(DigToml::read(td.path()).is_err());
    }

    #[test]
    fn reads_ignore_globs_private_and_metadata() {
        let td = TempDir::new().unwrap();
        std::fs::write(
            td.path().join("dig.toml"),
            r#"
output-dir = "dist"
private = true
ignore = ["*.map", "node_modules/**"]

[metadata]
name = "My Site"
description = "a demo"
keywords = ["chia", "dapp"]
"#,
        )
        .unwrap();
        let cfg = DigToml::read(td.path()).unwrap();
        assert_eq!(cfg.output_dir.as_deref(), Some("dist"));
        assert!(cfg.private);
        assert_eq!(cfg.ignore, vec!["*.map", "node_modules/**"]);
        assert_eq!(cfg.metadata.name.as_deref(), Some("My Site"));
        assert_eq!(cfg.metadata.description.as_deref(), Some("a demo"));
        assert_eq!(
            cfg.metadata.keywords.as_deref(),
            Some(["chia".to_string(), "dapp".to_string()].as_slice())
        );
    }

    #[test]
    fn defaults_have_no_ignore_not_private_empty_metadata() {
        let td = TempDir::new().unwrap();
        let cfg = DigToml::read(td.path()).unwrap();
        assert!(cfg.ignore.is_empty());
        assert!(!cfg.private);
        assert!(cfg.metadata.is_empty());
    }

    /// The env layer beats the file (precedence: env > dig.toml). Serialized
    /// because it mutates process-global env vars.
    #[test]
    fn env_layer_overrides_file() {
        // Guard the process-global env mutation against other tests in this binary.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap();

        let td = TempDir::new().unwrap();
        std::fs::write(
            td.path().join("dig.toml"),
            "store-id = \"file-id\"\noutput-dir = \"file-dist\"\n",
        )
        .unwrap();

        std::env::set_var("DIGSTORE_STORE_ID", "env-id");
        std::env::set_var("DIGSTORE_OUTPUT_DIR", "env-dist");
        let cfg = DigToml::read_with_env(td.path()).unwrap();
        std::env::remove_var("DIGSTORE_STORE_ID");
        std::env::remove_var("DIGSTORE_OUTPUT_DIR");

        assert_eq!(cfg.store_id.as_deref(), Some("env-id"));
        assert_eq!(cfg.output_dir.as_deref(), Some("env-dist"));
    }

    /// An UNSET env var leaves the file value intact (env layer is additive).
    #[test]
    fn env_layer_keeps_file_value_when_env_unset() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap();
        std::env::remove_var("DIGSTORE_REMOTE");
        let td = TempDir::new().unwrap();
        std::fs::write(td.path().join("dig.toml"), "remote = \"file-remote\"\n").unwrap();
        let cfg = DigToml::read_with_env(td.path()).unwrap();
        assert_eq!(cfg.remote.as_deref(), Some("file-remote"));
    }
}
