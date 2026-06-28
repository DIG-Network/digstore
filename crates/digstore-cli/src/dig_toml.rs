//! The committable `dig.toml` project manifest.
//!
//! `dig.toml` lives at the project root, is safe to commit (it holds NO secrets),
//! and is the single source of project config shared across `dev`, `deploy`, and
//! the scaffolding templates. Every field is optional; flags/env always override
//! the file (precedence: flags > env > dig.toml > built-in defaults).

use crate::error::CliError;

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
}
