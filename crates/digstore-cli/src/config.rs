//! CLI-level configuration: the remotes table (`remotes.toml`) and the global
//! node-resolution config (`node.url`, `CLAUDE.md` §5.3).

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

/// The URL explicitly configured for remote `name` (via `digstore remote add`),
/// normalized; [`None`] when the user has configured no such remote.
///
/// This reports ONLY what the user configured. It deliberately supplies no
/// default for `origin`: an unconfigured `origin` resolves through the §5.3
/// node ladder to the user's own node, which needs a probe and so cannot be
/// answered here (see `ops::node::resolve_remote_or_origin`).
pub fn configured_remote_url(ctx: &CliContext, name: &str) -> Result<Option<String>, CliError> {
    Ok(list_remotes(ctx)?
        .get(name)
        .map(|raw| normalize_remote_url(raw)))
}

/// The URL for remote `name`, erroring when it is not configured.
///
/// Callers that want the §5.3 default for an unconfigured `origin` must use
/// `ops::node::resolve_remote_or_origin` instead — before #2099 this function
/// answered an unconfigured `origin` with a hard-coded `https://rpc.dig.net`,
/// which routed every un-configured user through the public gateway even while
/// their own node was running.
pub fn resolve_remote_url(ctx: &CliContext, name: &str) -> Result<String, CliError> {
    configured_remote_url(ctx, name)?.ok_or_else(|| CliError::NotFound(format!("remote {name}")))
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

// ===========================================================================
// Global node-resolution config (`CLAUDE.md` §5.3): `digstore config node.url`.
//
// This is deliberately NOT part of `remotes.toml` (which is per-workspace,
// under `.dig/`): the node override is a machine-wide default (like `git`'s
// `--global` config), so it lives beside the identity/session state in the OS
// config dir, keyed the same way `ops::identity`/`ops::dighub` key their state
// (`DIG_IDENTITY_DIR` override, else `<config_dir>/dig`) — one global "dig"
// config home, not a third ad-hoc location.
// ===========================================================================

/// The environment variable an explicit node override can be supplied through
/// (`CLAUDE.md` §5.3 tier-1 override, second-highest precedence after `--node`).
pub const DIG_NODE_URL_ENV: &str = "DIG_NODE_URL";

#[derive(Debug, Default, Serialize, Deserialize)]
struct NodeConfigFile {
    #[serde(default)]
    node: NodeSection,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NodeSection {
    #[serde(default)]
    url: Option<String>,
}

/// The global dig config directory: `DIG_IDENTITY_DIR` override, else
/// `<OS config_dir>/dig`. Mirrors `ops::identity`/`ops::dighub` exactly (the
/// same env var) so a test/deployment that redirects one redirects all of them
/// together — there is one global "dig home", not per-feature ones.
fn global_dig_dir() -> Result<std::path::PathBuf, CliError> {
    if let Some(d) = std::env::var_os("DIG_IDENTITY_DIR") {
        return Ok(std::path::PathBuf::from(d));
    }
    let base = dirs::config_dir().ok_or_else(|| {
        CliError::Other(anyhow::anyhow!(
            "no OS config directory available for the dig config"
        ))
    })?;
    Ok(base.join("dig"))
}

fn node_config_path_in(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("config.toml")
}

fn load_node_config_in(dir: &std::path::Path) -> Result<NodeConfigFile, CliError> {
    let p = node_config_path_in(dir);
    if !p.exists() {
        return Ok(NodeConfigFile::default());
    }
    let text = fs::read_to_string(&p).map_err(|e| CliError::Other(e.into()))?;
    toml::from_str(&text).map_err(|e| CliError::Other(e.into()))
}

fn save_node_config_in(dir: &std::path::Path, f: &NodeConfigFile) -> Result<(), CliError> {
    fs::create_dir_all(dir).map_err(|e| CliError::Other(e.into()))?;
    let text = toml::to_string_pretty(f).map_err(|e| CliError::Other(e.into()))?;
    fs::write(node_config_path_in(dir), text).map_err(|e| CliError::Other(e.into()))
}

/// Persist `digstore config node.url <url>` — the LOWEST-precedence override
/// source (a `--node` flag or `$DIG_NODE_URL` still win; see
/// `digstore_remote::resolver::OverrideInputs`), but the only one that
/// survives across invocations. Writes into the global dig config dir
/// (`DIG_IDENTITY_DIR` override, else `<OS config_dir>/dig`).
pub fn set_node_url(url: &str) -> Result<(), CliError> {
    set_node_url_in(&global_dig_dir()?, url)
}

/// Clear a persisted `node.url` (`digstore config node.url --unset`). A no-op
/// (not an error) when nothing was set — `--unset` is idempotent.
pub fn unset_node_url() -> Result<(), CliError> {
    unset_node_url_in(&global_dig_dir()?)
}

/// The persisted `node.url`, if any has been set via `digstore config node.url`.
pub fn get_node_url() -> Result<Option<String>, CliError> {
    get_node_url_in(&global_dig_dir()?)
}

/// Explicit-directory variant of [`set_node_url`] — free of the process-global
/// `DIG_IDENTITY_DIR` env var, so tests need no lock (mirrors the `*_in(dir)`
/// pattern already used by `ops::dighub`'s session storage).
pub fn set_node_url_in(dir: &std::path::Path, url: &str) -> Result<(), CliError> {
    let mut f = load_node_config_in(dir)?;
    f.node.url = Some(url.trim_end_matches('/').to_string());
    save_node_config_in(dir, &f)
}

/// Explicit-directory variant of [`unset_node_url`].
pub fn unset_node_url_in(dir: &std::path::Path) -> Result<(), CliError> {
    let mut f = load_node_config_in(dir)?;
    f.node.url = None;
    save_node_config_in(dir, &f)
}

/// Explicit-directory variant of [`get_node_url`].
pub fn get_node_url_in(dir: &std::path::Path) -> Result<Option<String>, CliError> {
    Ok(load_node_config_in(dir)?.node.url)
}

// ===========================================================================
// PROJECT-scoped node config (`digstore config node.url --local`, #2099).
//
// Lives at `<workspace>/node.toml` — that is, `.dig/node.toml` — as a sibling
// of the per-project `remotes.toml`, and is therefore found by the SAME
// git-style nearest-ancestor `.dig` walk the rest of the CLI already uses
// (`CliContext::discover_workspace`). Reusing that boundary means "this
// project" has exactly one definition throughout the tool.
//
// SECURITY: this file can travel inside a repository. A `git clone` of a
// hostile repo would otherwise repoint the victim's node, and digs sends
// §21.9 identity-SIGNED requests to whatever node it resolves — so the
// exposure is signature and content harvesting, not merely a wrong read.
// The value is therefore NOT trusted on sight: see [`is_project_node_trusted_in`].
// ===========================================================================

/// The per-project node-config file inside a `.dig/` workspace directory.
pub fn project_node_path(workspace_dir: &std::path::Path) -> std::path::PathBuf {
    workspace_dir.join("node.toml")
}

/// Read the project-scoped `node.url`, if the project declares one.
///
/// This is the RAW declared value — it has NOT been trust-checked. Callers MUST
/// pass it through [`is_project_node_trusted_in`] before using it to route a
/// request (`ops::node::override_inputs` is the one place that does).
pub fn get_project_node_url_in(
    workspace_dir: &std::path::Path,
) -> Result<Option<String>, CliError> {
    let p = project_node_path(workspace_dir);
    if !p.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p).map_err(|e| CliError::Other(e.into()))?;
    let parsed: NodeConfigFile = toml::from_str(&text).map_err(|e| CliError::Other(e.into()))?;
    Ok(parsed.node.url)
}

/// Persist a project-scoped `node.url` into `<workspace_dir>/node.toml`.
///
/// Refuses a URL carrying credentials: this file lives in the project and is
/// routinely committed, so writing `https://user:token@host` here would publish
/// the token to the repository.
pub fn set_project_node_url_in(workspace_dir: &std::path::Path, url: &str) -> Result<(), CliError> {
    if has_userinfo(url) {
        return Err(CliError::InvalidArgument(format!(
            "{} embeds credentials, and .dig/node.toml is a project file that gets committed. \
             Use a URL without a user/password.",
            redact_url_userinfo(url)
        )));
    }
    fs::create_dir_all(workspace_dir).map_err(|e| CliError::Other(e.into()))?;
    let f = NodeConfigFile {
        node: NodeSection {
            url: Some(url.trim_end_matches('/').to_string()),
        },
    };
    let text = toml::to_string_pretty(&f).map_err(|e| CliError::Other(e.into()))?;
    fs::write(project_node_path(workspace_dir), text).map_err(|e| CliError::Other(e.into()))
}

/// Clear a project-scoped `node.url`. Idempotent when none was set.
pub fn unset_project_node_url_in(workspace_dir: &std::path::Path) -> Result<(), CliError> {
    let p = project_node_path(workspace_dir);
    if !p.exists() {
        return Ok(());
    }
    fs::remove_file(&p).map_err(|e| CliError::Other(e.into()))
}

// --- The trust store -------------------------------------------------------

/// Records which (project directory, node URL) pairs the user has approved.
/// Keyed by the workspace path so two projects are independent, and holding the
/// exact URL so that EDITING the project file re-arms the check — approving a
/// directory once must not hand it a blank cheque to point anywhere later.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default)]
    trusted: BTreeMap<String, String>,
}

fn trust_path(global_dir: &std::path::Path) -> std::path::PathBuf {
    global_dir.join("trusted-project-nodes.toml")
}

fn load_trust(global_dir: &std::path::Path) -> Result<TrustFile, CliError> {
    let p = trust_path(global_dir);
    if !p.exists() {
        return Ok(TrustFile::default());
    }
    let text = fs::read_to_string(&p).map_err(|e| CliError::Other(e.into()))?;
    toml::from_str(&text).map_err(|e| CliError::Other(e.into()))
}

/// A stable key for a workspace directory. Canonicalized where the path exists
/// (so `.`/`..`/symlink spellings of the same project share one trust record)
/// and otherwise used verbatim.
/// A node URL with any embedded credentials replaced, safe to print.
///
/// `https://user:token@host` is a legal URL, and digs prints the node URL in
/// half a dozen places — the approval prompt, the ignored-value warning,
/// `--show`, `doctor`, and the "reading remotely" notice. Printing one
/// verbatim would put a credential on stdout and into any log or CI transcript
/// that captured it.
///
/// The whole userinfo section goes, not just the password: a bare
/// `https://alice@host` still names someone.
pub fn redact_url_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    // Userinfo is only userinfo before the first `/` — an `@` later in the URL
    // is part of the path or query and must survive.
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (rest, None),
    };
    let Some((_creds, host)) = authority.rsplit_once('@') else {
        return url.to_string();
    };
    match path {
        Some(p) => format!("{scheme}://***@{host}/{p}"),
        None => format!("{scheme}://***@{host}"),
    }
}

/// Whether a node URL carries embedded credentials.
///
/// Refused at the point a value is STORED rather than merely redacted on
/// display, because `.dig/node.toml` is a project file people commit — a
/// credential written there would be published to the repository, and
/// redacting the echo afterwards would not take it back.
pub fn has_userinfo(url: &str) -> bool {
    url.split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .is_some_and(|authority| authority.contains('@'))
}

fn trust_key(workspace_dir: &std::path::Path) -> String {
    std::fs::canonicalize(workspace_dir)
        .unwrap_or_else(|_| workspace_dir.to_path_buf())
        .display()
        .to_string()
}

/// Has the user approved THIS project directory pointing at THIS exact URL?
///
/// Both halves matter. Keying on the directory alone would let a repo change
/// its `node.toml` after being approved once; keying on the URL alone would let
/// approval in one project silently authorize another.
pub fn is_project_node_trusted_in(
    global_dir: &std::path::Path,
    workspace_dir: &std::path::Path,
    url: &str,
) -> Result<bool, CliError> {
    Ok(load_trust(global_dir)?
        .trusted
        .get(&trust_key(workspace_dir))
        == Some(&url.trim_end_matches('/').to_string()))
}

/// Record the user's approval of `url` for `workspace_dir`, replacing any
/// previous approval for that directory.
pub fn trust_project_node_in(
    global_dir: &std::path::Path,
    workspace_dir: &std::path::Path,
    url: &str,
) -> Result<(), CliError> {
    let mut f = load_trust(global_dir)?;
    f.trusted.insert(
        trust_key(workspace_dir),
        url.trim_end_matches('/').to_string(),
    );
    fs::create_dir_all(global_dir).map_err(|e| CliError::Other(e.into()))?;
    let text = toml::to_string_pretty(&f).map_err(|e| CliError::Other(e.into()))?;
    fs::write(trust_path(global_dir), text).map_err(|e| CliError::Other(e.into()))
}

/// The global dig config directory (`DIG_IDENTITY_DIR`, else
/// `<OS config_dir>/dig`) — where the trust store lives. Exposed so
/// `ops::node` can pair a project value with its approval record.
pub fn global_config_dir() -> Result<std::path::PathBuf, CliError> {
    global_dig_dir()
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

    /// #2099 regression: an unconfigured `origin` must NOT resolve to the
    /// public gateway here. Naming the exact old value in the assertion pins
    /// the specific defect, not merely "some error happened".
    #[test]
    fn an_unconfigured_origin_no_longer_defaults_to_the_public_gateway() {
        let (_td, ctx) = ctx();
        assert!(list_remotes(&ctx).unwrap().is_empty());
        assert_eq!(configured_remote_url(&ctx, "origin").unwrap(), None);
        match resolve_remote_url(&ctx, "origin") {
            Err(CliError::NotFound(_)) => {}
            Ok(url) => panic!(
                "an unconfigured origin must not resolve here; got {url} \
                 (a hard-coded https://rpc.dig.net default is the #2099 defect)"
            ),
            other => panic!("expected NotFound, got {other:?}"),
        }
        // A non-origin unknown name behaves identically.
        assert!(matches!(
            resolve_remote_url(&ctx, "upstream"),
            Err(CliError::NotFound(_))
        ));
    }

    /// A CONFIGURED origin is still returned verbatim — the change removes the
    /// default, not the ability to point origin wherever you like.
    #[test]
    fn a_configured_origin_is_returned_unchanged() {
        let (_td, ctx) = ctx();
        add_remote(&ctx, "origin", "https://rpc.dig.net").unwrap();
        assert_eq!(
            resolve_remote_url(&ctx, "origin").unwrap(),
            "https://rpc.dig.net"
        );
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

    // -----------------------------------------------------------------------
    // Global node config (`digstore config node.url`, `CLAUDE.md` §5.3).
    //
    // Uses the `*_in(dir)` explicit-directory variants, so these tests are free
    // of the process-global `DIG_IDENTITY_DIR` env var and need no lock (the
    // same pattern `ops::dighub`'s `session_round_trip_save_load_clear` uses).
    // -----------------------------------------------------------------------

    #[test]
    fn node_url_unset_by_default() {
        let td = tempdir().unwrap();
        assert_eq!(get_node_url_in(td.path()).unwrap(), None);
    }

    #[test]
    fn node_url_set_then_get_round_trips() {
        let td = tempdir().unwrap();
        set_node_url_in(td.path(), "https://my-node.example:9778").unwrap();
        assert_eq!(
            get_node_url_in(td.path()).unwrap().as_deref(),
            Some("https://my-node.example:9778")
        );
    }

    #[test]
    fn node_url_set_strips_trailing_slash() {
        let td = tempdir().unwrap();
        set_node_url_in(td.path(), "https://my-node.example/").unwrap();
        assert_eq!(
            get_node_url_in(td.path()).unwrap().as_deref(),
            Some("https://my-node.example")
        );
    }

    #[test]
    fn node_url_unset_clears_it() {
        let td = tempdir().unwrap();
        set_node_url_in(td.path(), "https://my-node.example").unwrap();
        unset_node_url_in(td.path()).unwrap();
        assert_eq!(get_node_url_in(td.path()).unwrap(), None);
    }

    #[test]
    fn node_url_unset_is_idempotent_when_never_set() {
        let td = tempdir().unwrap();
        // Unsetting with no config file present at all must not error.
        unset_node_url_in(td.path()).unwrap();
        assert_eq!(get_node_url_in(td.path()).unwrap(), None);
    }

    // -----------------------------------------------------------------------
    // Project-scoped node config + its trust store (#2099).
    // -----------------------------------------------------------------------

    #[test]
    fn project_node_url_round_trips_and_clears() {
        let td = tempdir().unwrap();
        let ws = td.path().join(".dig");
        assert_eq!(get_project_node_url_in(&ws).unwrap(), None);
        set_project_node_url_in(&ws, "https://project.example/").unwrap();
        // Trailing slash normalized, matching the global setter.
        assert_eq!(
            get_project_node_url_in(&ws).unwrap().as_deref(),
            Some("https://project.example")
        );
        unset_project_node_url_in(&ws).unwrap();
        assert_eq!(get_project_node_url_in(&ws).unwrap(), None);
        // Idempotent when already absent.
        unset_project_node_url_in(&ws).unwrap();
    }

    /// The project file lives beside `remotes.toml` inside `.dig/`, so the
    /// existing nearest-ancestor workspace walk finds it. Pinned because the
    /// LOCATION is the contract a repo-carried file is judged against.
    #[test]
    fn project_node_file_sits_inside_the_dig_workspace_dir() {
        let ws = std::path::Path::new("/proj/.dig");
        assert_eq!(project_node_path(ws), ws.join("node.toml"));
    }

    /// A freshly-cloned repository's `node.toml` is NOT trusted: nobody has
    /// approved it. This is the whole point of the trust store.
    #[test]
    fn a_project_node_url_is_untrusted_until_approved() {
        let global = tempdir().unwrap();
        let proj = tempdir().unwrap();
        assert!(
            !is_project_node_trusted_in(global.path(), proj.path(), "https://evil.example")
                .unwrap()
        );
        trust_project_node_in(global.path(), proj.path(), "https://evil.example").unwrap();
        assert!(
            is_project_node_trusted_in(global.path(), proj.path(), "https://evil.example").unwrap()
        );
    }

    /// Approving a directory once must NOT authorize whatever that directory
    /// says NEXT — a repo that is trusted today and then edits its `node.toml`
    /// (a later commit, a malicious PR merged upstream) has to be re-approved.
    /// The fixture keeps the SAME directory and varies ONLY the URL, so a
    /// directory-keyed-only implementation returns true here and fails.
    #[test]
    fn trust_does_not_carry_over_when_the_project_changes_the_url() {
        let global = tempdir().unwrap();
        let proj = tempdir().unwrap();
        trust_project_node_in(global.path(), proj.path(), "https://good.example").unwrap();
        assert!(
            !is_project_node_trusted_in(global.path(), proj.path(), "https://evil.example")
                .unwrap(),
            "an edited node.toml must re-arm the approval check"
        );
        // The originally-approved URL still is trusted — the record was not
        // merely wiped, it is value-specific.
        assert!(
            is_project_node_trusted_in(global.path(), proj.path(), "https://good.example").unwrap()
        );
    }

    /// Approving one project must not authorize the same URL in a DIFFERENT
    /// project. The fixture varies ONLY the directory, so a URL-keyed-only
    /// implementation returns true here and fails.
    #[test]
    fn trust_is_scoped_to_the_project_that_was_approved() {
        let global = tempdir().unwrap();
        let approved = tempdir().unwrap();
        let other = tempdir().unwrap();
        trust_project_node_in(global.path(), approved.path(), "https://node.example").unwrap();
        assert!(
            !is_project_node_trusted_in(global.path(), other.path(), "https://node.example")
                .unwrap(),
            "approval in one project must not leak into another"
        );
    }

    /// Two spellings of one directory share a trust record, so approving via
    /// `.` does not leave the absolute path unapproved (and vice versa).
    #[test]
    fn trust_key_is_stable_across_path_spellings() {
        let global = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let nested = proj.path().join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        let dotted = nested.join("..");
        trust_project_node_in(global.path(), proj.path(), "https://node.example").unwrap();
        assert!(
            is_project_node_trusted_in(global.path(), &dotted, "https://node.example").unwrap()
        );
    }

    #[test]
    fn node_url_set_overwrites_previous_value() {
        let td = tempdir().unwrap();
        set_node_url_in(td.path(), "https://first.example").unwrap();
        set_node_url_in(td.path(), "https://second.example").unwrap();
        assert_eq!(
            get_node_url_in(td.path()).unwrap().as_deref(),
            Some("https://second.example")
        );
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    /// A node URL may legally carry `user:token@`, and digs prints the node URL
    /// in the approval prompt, the ignored-value warning, `--show`, `doctor`,
    /// and the reading-remotely notice. Any of those would otherwise put a
    /// credential on stdout and into a CI transcript.
    #[test]
    fn redaction_removes_the_whole_userinfo_section() {
        assert_eq!(
            redact_url_userinfo("https://alice:s3cret@node.example"),
            "https://***@node.example"
        );
        // A bare username still identifies someone.
        assert_eq!(
            redact_url_userinfo("https://alice@node.example"),
            "https://***@node.example"
        );
        assert_eq!(
            redact_url_userinfo("https://alice:s3cret@node.example:9778/base"),
            "https://***@node.example:9778/base"
        );
    }

    /// It must not mangle the ordinary case, and an `@` after the authority is
    /// path or query data, not credentials.
    #[test]
    fn redaction_leaves_a_credential_free_url_alone() {
        for url in [
            "https://rpc.dig.net",
            "http://dig.local",
            "http://localhost:9778",
            "https://node.example/path/with@sign",
            "not-a-url",
        ] {
            assert_eq!(redact_url_userinfo(url), url, "must not rewrite {url}");
        }
    }

    #[test]
    fn userinfo_is_detected_only_in_the_authority() {
        assert!(has_userinfo("https://alice:s3cret@node.example"));
        assert!(has_userinfo("https://alice@node.example"));
        assert!(!has_userinfo("https://rpc.dig.net"));
        assert!(!has_userinfo("https://node.example/path/with@sign"));
    }

    /// `.dig/node.toml` is a project file that gets committed, so a credential
    /// written there would be published to the repository. Redacting the echo
    /// afterwards would not take it back — refuse to store it at all.
    #[test]
    fn a_project_node_url_with_credentials_is_refused_and_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join(".dig");

        let err = set_project_node_url_in(&ws, "https://alice:s3cret@node.example")
            .expect_err("a credential-bearing URL must be refused");

        let msg = err.to_string();
        assert!(
            !msg.contains("s3cret"),
            "the refusal must not echo the credential: {msg}"
        );
        assert!(
            get_project_node_url_in(&ws).unwrap().is_none(),
            "nothing may be persisted when the value is refused"
        );

        // …and the ordinary case still works.
        set_project_node_url_in(&ws, "https://node.example").unwrap();
        assert_eq!(
            get_project_node_url_in(&ws).unwrap().as_deref(),
            Some("https://node.example")
        );
    }
}
