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
    let raw = list_remotes(ctx)?
        .get(name)
        .cloned()
        .ok_or_else(|| CliError::NotFound(format!("remote {name}")))?;
    Ok(normalize_remote_url(&raw))
}

/// The default network RPC host a bare `dig://` resolves to.
pub const DEFAULT_DIG_RPC_HOST: &str = "rpc.dig.net";

/// Resolve a `dig://` remote to the concrete HTTPS base the protocol client uses (it appends
/// `/stores/{id}/...`). `dig://` is the network scheme — it resolves to HTTPS under the hood, the
/// same way `git@github.com:` resolves to a transport. Forms:
///   * `dig://<host>[:port]`            -> `https://<host>[:port]`            (a specific RPC node)
///   * `dig://<host>/<storeId>`         -> `https://<host>`                   (host is the base; the
///                                          storeId is carried by the clone/push args)
///   * `dig://<storeId>` (bare 64-hex)  -> `https://rpc.dig.net`             (the default network RPC)
/// Any non-`dig://` URL passes through unchanged (an explicit `https://…` remote still works).
pub fn normalize_remote_url(url: &str) -> String {
    let Some(rest) = url.strip_prefix("dig://") else {
        return url.to_string();
    };
    let authority = rest.split('/').next().unwrap_or("");
    // A bare 64-hex authority is a store id, not a host → use the default network RPC.
    if authority.len() == 64 && authority.bytes().all(|b| b.is_ascii_hexdigit()) {
        return format!("https://{DEFAULT_DIG_RPC_HOST}");
    }
    // Otherwise the authority is the node host; the base is just the host (the client appends the
    // `/stores/{id}/...` protocol paths). Trailing path (e.g. a store id) is informational.
    if authority.is_empty() {
        return format!("https://{DEFAULT_DIG_RPC_HOST}");
    }
    format!("https://{authority}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx() -> (tempfile::TempDir, CliContext) {
        let td = tempdir().unwrap();
        let ctx = CliContext::workspace_only(td.path().to_path_buf(), false, false);
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

    #[test]
    fn dig_scheme_resolves_to_https_base() {
        // Specific node host.
        assert_eq!(normalize_remote_url("dig://rpc.dig.net"), "https://rpc.dig.net");
        // Host + store id → base is the host (client appends /stores/{id}).
        assert_eq!(
            normalize_remote_url("dig://rpc.dig.net/abcd1234"),
            "https://rpc.dig.net"
        );
        // Bare 64-hex store id → default network RPC.
        let id = "ccd5bb71183532bff220ba46c268991a3ff07eb358e8255a65c30a2dce0e5fbb";
        assert_eq!(
            normalize_remote_url(&format!("dig://{id}")),
            "https://rpc.dig.net"
        );
        // Non-dig URLs pass through.
        assert_eq!(normalize_remote_url("https://h/x"), "https://h/x");
    }

    #[test]
    fn resolve_remote_url_normalizes_dig_scheme() {
        let (_td, ctx) = ctx();
        add_remote(&ctx, "origin", "dig://rpc.dig.net/store1").unwrap();
        assert_eq!(resolve_remote_url(&ctx, "origin").unwrap(), "https://rpc.dig.net");
    }
}
