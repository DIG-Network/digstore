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
    match list_remotes(ctx)?.get(name).cloned() {
        Some(raw) => Ok(normalize_remote_url(&raw)),
        // `origin` defaults to the public RPC even when never `remote add`-ed: identity is the
        // owner puzzle hash (keys authenticate the push), so the canonical origin is fixed and
        // needs no per-store configuration. Other names must be explicitly added.
        None if name == "origin" => Ok("https://rpc.dig.net".to_string()),
        None => Err(CliError::NotFound(format!("remote {name}"))),
    }
}

/// The default network RPC host a bare `dig://` resolves to.
pub const DEFAULT_DIG_RPC_HOST: &str = "rpc.dig.net";

/// True for a 64-hex (32-byte) store id.
fn is_store_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Resolve a `dig://` remote to the concrete HTTPS **store URL** (`https://<host>/stores/<id>`)
/// the protocol client and `parse_store_url` expect. `dig://` is the network scheme — it resolves
/// to HTTPS under the hood, the same way `git@github.com:` resolves to a transport.
///
/// A `dig://` URL names BOTH the host (which node serves it — there can be many) AND, optionally,
/// the `<user>` (the owner identity, like GitHub's `user/` namespace). The `<user>@` part is
/// INFORMATIONAL for routing/display — the 64-hex store id alone identifies the store on the wire —
/// so it is stripped from the resolved HTTPS URL. Caller AUTHENTICATION is separate: every request
/// carries the requester's own signed identity headers (paper §21.9), not the URL's `<user>`.
/// Forms (`[<user>@]` optional everywhere):
///   * `dig://<storeId>` (bare 64-hex)        -> `https://rpc.dig.net/stores/<storeId>`  (default RPC)
///   * `dig://<user>@<storeId>`               -> `https://rpc.dig.net/stores/<storeId>`
///   * `dig://[<user>@]<host>[:port]/<storeId>` -> `https://<host>[:port]/stores/<storeId>` (a node)
///   * `dig://[<user>@]<host>/stores/<storeId>` -> `https://<host>/stores/<storeId>`        (pathed)
///   * `dig://[<user>@]<host>[:port]`          -> `https://<host>[:port]`                   (base only)
///
/// Any non-`dig://` URL passes through unchanged (an explicit `https://…` remote still works).
pub fn normalize_remote_url(url: &str) -> String {
    let Some(rest) = url.strip_prefix("dig://") else {
        return url.to_string();
    };
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, p.trim_start_matches('/')),
        None => (rest, ""),
    };
    // Strip the optional `<user>@` owner namespace from the authority — it is informational
    // (display/routing), not part of the wire address (the store id is). Caller auth is a
    // separate signed-header mechanism (§21.9).
    let host_part = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);

    // `dig://[<user>@]<64-hex>` — the host part IS the store id (not a host): default network RPC.
    if path.is_empty() && is_store_id(host_part) {
        return format!("https://{DEFAULT_DIG_RPC_HOST}/stores/{host_part}");
    }

    // Otherwise the host part is the node host (empty → default RPC host).
    let host = if host_part.is_empty() {
        DEFAULT_DIG_RPC_HOST
    } else {
        host_part
    };
    if path.is_empty() {
        // Node base only (no store id): used by `remote add` of a node; clone/push/pull
        // that need a store id should use the `/stores/<id>` form.
        return format!("https://{host}");
    }
    // Already canonical `stores/<id>[/...]`.
    if path.starts_with("stores/") {
        return format!("https://{host}/{path}");
    }
    // `dig://<host>/<storeId>` — insert the `/stores/` segment the protocol expects.
    let first = path.split('/').next().unwrap_or("");
    if is_store_id(first) {
        return format!("https://{host}/stores/{first}");
    }
    // Fallback: preserve host + path verbatim.
    format!("https://{host}/{path}")
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
    fn dig_scheme_resolves_to_https_store_url() {
        let id = "ccd5bb71183532bff220ba46c268991a3ff07eb358e8255a65c30a2dce0e5fbb";
        // Bare 64-hex store id → default network RPC + /stores/<id>.
        assert_eq!(
            normalize_remote_url(&format!("dig://{id}")),
            format!("https://rpc.dig.net/stores/{id}")
        );
        // Specific node host + store id → /stores/<id> on that host.
        assert_eq!(
            normalize_remote_url(&format!("dig://node.example:8443/{id}")),
            format!("https://node.example:8443/stores/{id}")
        );
        // Already-pathed `stores/<id>` is preserved.
        assert_eq!(
            normalize_remote_url(&format!("dig://rpc.dig.net/stores/{id}")),
            format!("https://rpc.dig.net/stores/{id}")
        );
        // Node base only (no store id) → just the host.
        assert_eq!(
            normalize_remote_url("dig://rpc.dig.net"),
            "https://rpc.dig.net"
        );
        // `<user>@` owner namespace is informational and stripped from the wire URL.
        assert_eq!(
            normalize_remote_url(&format!("dig://alice@node.example:8443/{id}")),
            format!("https://node.example:8443/stores/{id}")
        );
        assert_eq!(
            normalize_remote_url(&format!("dig://alice@{id}")),
            format!("https://rpc.dig.net/stores/{id}")
        );
        // Non-dig URLs pass through.
        assert_eq!(normalize_remote_url("https://h/x"), "https://h/x");
    }

    #[test]
    fn resolve_remote_url_normalizes_dig_scheme() {
        let (_td, ctx) = ctx();
        let id = "ccd5bb71183532bff220ba46c268991a3ff07eb358e8255a65c30a2dce0e5fbb";
        add_remote(&ctx, "origin", &format!("dig://{id}")).unwrap();
        assert_eq!(
            resolve_remote_url(&ctx, "origin").unwrap(),
            format!("https://rpc.dig.net/stores/{id}")
        );
    }
}
