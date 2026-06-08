//! CLI-level configuration: the remotes table (`remotes.toml`).

use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::context::CliContext;
use crate::error::CliError;

#[derive(Debug, Default, Serialize, Deserialize)]
struct RemotesFile {
    #[serde(default)]
    remotes: BTreeMap<String, String>,
}

fn remotes_path(ctx: &CliContext) -> std::path::PathBuf {
    ctx.dig_dir.join("remotes.toml")
}

fn load(ctx: &CliContext) -> Result<RemotesFile, CliError> {
    let p = remotes_path(ctx);
    if !p.exists() {
        return Ok(RemotesFile::default());
    }
    let text = fs::read_to_string(&p).map_err(|e| CliError::Other(e.into()))?;
    toml::from_str(&text).map_err(|e| CliError::Other(e.into()))
}

fn save(ctx: &CliContext, f: &RemotesFile) -> Result<(), CliError> {
    let text = toml::to_string_pretty(f).map_err(|e| CliError::Other(e.into()))?;
    fs::write(remotes_path(ctx), text).map_err(|e| CliError::Other(e.into()))
}

pub fn add_remote(ctx: &CliContext, name: &str, url: &str) -> Result<(), CliError> {
    let mut f = load(ctx)?;
    f.remotes.insert(name.to_string(), url.to_string());
    save(ctx, &f)
}

pub fn remove_remote(ctx: &CliContext, name: &str) -> Result<(), CliError> {
    let mut f = load(ctx)?;
    if f.remotes.remove(name).is_none() {
        return Err(CliError::NotFound(format!("remote {name}")));
    }
    save(ctx, &f)
}

pub fn list_remotes(ctx: &CliContext) -> Result<BTreeMap<String, String>, CliError> {
    Ok(load(ctx)?.remotes)
}

pub fn resolve_remote_url(ctx: &CliContext, name: &str) -> Result<String, CliError> {
    list_remotes(ctx)?
        .get(name)
        .cloned()
        .ok_or_else(|| CliError::NotFound(format!("remote {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx() -> (tempfile::TempDir, CliContext) {
        let td = tempdir().unwrap();
        let ctx = CliContext::resolve(Some(td.path().to_path_buf()), false, false);
        std::fs::create_dir_all(&ctx.dig_dir).unwrap();
        (td, ctx)
    }

    #[test]
    fn add_then_list_remote_persists() {
        let (_td, ctx) = ctx();
        add_remote(&ctx, "origin", "https://h/stores/x").unwrap();
        assert_eq!(
            list_remotes(&ctx)
                .unwrap()
                .get("origin")
                .map(String::as_str),
            Some("https://h/stores/x")
        );
    }

    #[test]
    fn remove_remote_deletes_it() {
        let (_td, ctx) = ctx();
        add_remote(&ctx, "origin", "https://h").unwrap();
        remove_remote(&ctx, "origin").unwrap();
        assert!(list_remotes(&ctx).unwrap().is_empty());
    }

    #[test]
    fn resolve_remote_url_errors_for_unknown() {
        let (_td, ctx) = ctx();
        assert!(resolve_remote_url(&ctx, "nope").is_err());
    }
}
