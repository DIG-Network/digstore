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

    // The parse error is SUMMARISED, never rendered.
    //
    // `toml`'s Display quotes the offending source line back — and this file is attacker-supplied,
    // so that is a channel for arbitrary bytes onto stdout. A carriage return survives the
    // renderer and erases its own `2 | ` framing, which lets the quoted line masquerade as digs'
    // own output. It also makes a SYMLINKED node.toml echo a line of whatever it points at.
    //
    // Same rule the refusal below follows: a value we are rejecting is never quoted verbatim.
    // A line number is all a real user needs to fix their file.
    let parsed: NodeConfigFile = toml::from_str(&text).map_err(|e| {
        let line = e
            .span()
            .map(|s| text[..s.start.min(text.len())].lines().count().max(1));
        CliError::InvalidArgument(match line {
            Some(n) => format!("{} is not valid TOML (line {n})", p.display()),
            None => format!("{} is not valid TOML", p.display()),
        })
    })?;

    // THE CONSENT PROMPT IS ONLY WORTH WHAT ITS DISPLAY IS WORTH.
    //
    // This value is attacker-controlled: it arrives in a repo-carried file, and a TOML basic
    // string can carry `\n`, `\t`, `\r` and other escapes. The WHATWG URL parser STRIPS ASCII
    // tab/LF/CR before parsing, so those bytes break the approval prompt across lines and then
    // vanish before the dial. `url = "https://rpc.dig.net\n\t\t\t\t.evil.example/"` prompts with a
    // first line reading `https://rpc.dig.net` — the very gateway our own NO_LOCAL_NODE text tells
    // people to configure — while the host actually dialled is `rpc.dig.net.evil.example`.
    //
    // Rejected HERE, at the single point every consumer reads through, rather than sanitised at
    // each display site. Redaction was already spread across eight sites and two were missed; a
    // ninth site added later would be unprotected by construction. No legitimate URL needs a
    // control character or whitespace, and these are exactly the bytes the parser discards.
    if let Some(url) = parsed.node.url.as_deref() {
        if let Some(bad) = url
            .chars()
            .find(|c| c.is_control() || c.is_whitespace() || !c.is_ascii())
        {
            return Err(CliError::InvalidArgument(format!(
                "{} declares a node.url containing {} — refusing it. A node URL is plain ASCII \
                 with no control characters or whitespace; those bytes can make the value \
                 displayed for your approval differ from the host actually contacted.",
                p.display(),
                char_name(bad)
            )));
        }
    }
    Ok(parsed.node.url)
}

/// A printable name for a byte we refuse, so the error says which one without
/// echoing a character that would itself scramble the message — a refused value
/// is attacker-chosen, so it must never be quoted back verbatim.
fn char_name(c: char) -> String {
    match c {
        '\n' => "a line feed (\\n)".into(),
        '\r' => "a carriage return (\\r)".into(),
        '\t' => "a tab (\\t)".into(),
        ' ' => "a space".into(),
        other => format!("the character U+{:04X}", other as u32),
    }
}

/// Persist a project-scoped `node.url` into `<workspace_dir>/node.toml`.
///
/// Refuses a URL carrying credentials.
///
/// `digstore init` gitignores `.dig/`, so this file is not committed by default — but it is a
/// PROJECT file whose whole purpose is to travel with the project, and a repo that un-ignores or
/// force-adds it publishes whatever is inside. A token written here would then be in the
/// repository's history, where redacting the echo afterwards does not reach.
pub fn set_project_node_url_in(workspace_dir: &std::path::Path, url: &str) -> Result<(), CliError> {
    // The same shape the READ path enforces. Without this, `config node.url --local` happily
    // writes a value its own next invocation refuses — the user is left with a file digs told
    // them to create and then ignores.
    if let Some(bad) = url
        .chars()
        .find(|c| c.is_control() || c.is_whitespace() || !c.is_ascii())
    {
        return Err(CliError::InvalidArgument(format!(
            "that node.url contains {} — a node URL is plain ASCII with no control \
             characters or whitespace.",
            char_name(bad)
        )));
    }
    if has_userinfo(url) {
        return Err(CliError::InvalidArgument(format!(
            "{} embeds credentials, and .dig/node.toml is a project file meant to travel with \
             the repository. Use a URL without a user/password.",
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

/// A node URL that is safe to hand to a sink — it renders and serializes redacted.
///
/// This is a NEWTYPE rather than a `redact(…)` call at each print site on purpose. Redaction was
/// spread across eight display sites and the last two — the `--json` paths in `commands/config.rs`
/// — were missed, which is exactly the shape of bug that recurs: correctness by remembering.
/// Wrapping the value moves it to correctness by construction, because reaching a sink now
/// requires the wrapper and the wrapper has no un-redacted rendering.
///
/// `Display` covers `format!`/`ui.line`; `Serialize` covers `--json`, which is the mode whose
/// transcripts CI and agents capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedUrl(String);

impl RedactedUrl {
    /// Wrap a URL for display.
    pub fn new(url: &str) -> Self {
        RedactedUrl(redact_url_userinfo(url))
    }
}

impl std::fmt::Display for RedactedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for RedactedUrl {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

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
    // PARSE, do not split. This function previously terminated the authority at `/` only, while
    // the WHATWG parser — `url`, the same crate reqwest dials with — also terminates it at `\`,
    // `?` and `#` for special schemes. That divergence was directly exploitable:
    //
    //     https://evil.example\@rpc.dig.net/
    //
    // displayed as `https://***@rpc.dig.net/` (host `rpc.dig.net`, credentials tidily hidden)
    // while the host actually dialled was `evil.example`. Worse than showing the raw string,
    // because the redaction lent it the official gateway's name.
    //
    // Round-tripping through the real parser makes display and dial agree BY CONSTRUCTION: the
    // host shown is the host `Url` resolved, and the `\`/`?`/`#` payload re-serializes into the
    // path or query where it plainly belongs.
    let Ok(mut parsed) = url::Url::parse(url) else {
        // Unparseable: there is no authority to reason about, so echo NOTHING attacker-chosen.
        // Falling back to string surgery here is exactly the divergence this function exists to
        // remove.
        return "<unparseable URL>".to_string();
    };

    // A node URL is an http(s) origin. `mailto:`, `data:`, `foo:` and friends parse cleanly with
    // an empty username and no host, so without this they would sail through and be printed
    // verbatim — the redactor would be echoing an arbitrary attacker-chosen string again, by a
    // different door than the one just closed.
    if parsed.host().is_none() || !matches!(parsed.scheme(), "http" | "https") {
        return "<unusable URL>".to_string();
    }

    if parsed.username().is_empty() && parsed.password().is_none() {
        return parsed.to_string();
    }
    // `***` rather than dropping the section, so the reader can see credentials WERE present.
    // Both setters only fail on a cannot-be-a-base URL, which cannot carry userinfo anyway.
    let _ = parsed.set_username("***");
    let _ = parsed.set_password(None);
    parsed.to_string()
}

/// Whether a node URL carries embedded credentials.
///
/// Refused at the point a value is STORED rather than merely redacted on
/// display, because `.dig/node.toml` is a project file people commit — a
/// credential written there would be published to the repository, and
/// redacting the echo afterwards would not take it back.
pub fn has_userinfo(url: &str) -> bool {
    // Same rule as the redactor: ask the parser, never the string. A `\@` payload puts the `@`
    // in the PATH, so a split-based check would report credentials that are not there — and,
    // reversed, would miss ones that are.
    url::Url::parse(url)
        .map(|u| !u.username().is_empty() || u.password().is_some())
        .unwrap_or(false)
}

/// A stable key for a workspace directory. Canonicalized where the path exists
/// (so `.`/`..`/symlink spellings of the same project share one trust record)
/// and otherwise used verbatim.
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

    /// A malformed `node.toml` must not echo its own bytes back.
    ///
    /// `toml`'s Display quotes the offending source line, and this file is attacker-supplied. A
    /// carriage return survives the renderer and erases its `2 | ` framing, so the quoted line can
    /// masquerade as digs' own output — and a SYMLINKED node.toml would echo a line of whatever it
    /// points at. Contradicts the rule the refusal path already follows: never quote a rejected
    /// value verbatim.
    #[test]
    fn a_malformed_node_toml_is_summarised_not_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join(".dig");
        fs::create_dir_all(&ws).unwrap();
        // Invalid TOML (an unterminated basic string) whose content is a decoy line.
        fs::write(
            ws.join("node.toml"),
            "[node]\nurl = \"https://rpc.dig.net\r  ok  everything is fine\n",
        )
        .unwrap();

        let err = get_project_node_url_in(&ws).expect_err("malformed TOML must be refused");
        let msg = err.to_string();

        assert!(
            !msg.contains("everything is fine"),
            "the error quoted the file's contents back: {msg:?}"
        );
        assert!(
            !msg.contains('\r') && !msg.contains('\n'),
            "the error must be a single unscrambled line: {msg:?}"
        );
        assert!(
            msg.contains("not valid TOML"),
            "the error must still say what is wrong: {msg:?}"
        );
    }

    /// Non-http schemes parse cleanly with an empty username and no host, so without a scheme
    /// check the redactor would print them verbatim — echoing an arbitrary attacker-chosen string
    /// through a different door than the one just closed.
    #[test]
    fn a_non_http_scheme_is_not_echoed_by_the_redactor() {
        for payload in [
            "mailto:someone@example.com",
            "data:text/html,<script>alert(1)</script>",
            "foo:whatever-i-like",
            "ftp://files.example/x",
            "ws://node.example/socket",
        ] {
            let shown = redact_url_userinfo(payload);
            assert_eq!(
                shown, "<unusable URL>",
                "{payload} is not an http(s) origin and must not be echoed"
            );
        }
        // …while real node URLs still render.
        assert!(redact_url_userinfo("https://rpc.dig.net").starts_with("https://rpc.dig.net"));
        assert!(redact_url_userinfo("http://localhost:9778").starts_with("http://localhost:9778"));
    }

    /// The WRITE path must refuse what the READ path refuses.
    ///
    /// Shipped untested in the previous round: deleting the guard from `set_project_node_url_in`
    /// entirely left the whole suite green, which makes it a guard that can be removed silently.
    /// Without it, `digstore config node.url --local` happily writes a value its own next
    /// invocation refuses — digs tells the user to create a file and then ignores it.
    #[test]
    fn the_write_path_refuses_what_the_read_path_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join(".dig");

        for (payload, what) in [
            ("https://a\nb.example", "a line feed"),
            ("https://a\tb.example", "a tab"),
            ("https://a b.example", "a space"),
            ("https://rpc.dig.n\u{0435}t", "a Cyrillic homograph"),
        ] {
            let err = set_project_node_url_in(&ws, payload)
                .expect_err("the write path must refuse {what}");
            let msg = err.to_string();
            assert!(
                !msg.contains('\n') && !msg.contains('\t'),
                "the refusal must not be scrambled by the payload it describes ({what}): {msg:?}"
            );
            assert!(
                get_project_node_url_in(&ws).unwrap().is_none(),
                "nothing may be persisted when the write is refused ({what})"
            );
        }

        // …and a legitimate value still writes, so this is a guard rather than a wall.
        set_project_node_url_in(&ws, "https://node.example:9778").unwrap();
        assert_eq!(
            get_project_node_url_in(&ws).unwrap().as_deref(),
            Some("https://node.example:9778")
        );
    }

    /// A node URL may legally carry `user:token@`, and digs prints the node URL
    /// in the approval prompt, the ignored-value warning, `--show`, `doctor`,
    /// and the reading-remotely notice. Any of those would otherwise put a
    /// credential on stdout and into a CI transcript.
    #[test]
    fn redaction_removes_the_whole_userinfo_section() {
        // The trailing `/` is the parser's canonical form, not a rewrite: redaction
        // round-trips through `url::Url` so the string shown is the one it resolved.
        assert_eq!(
            redact_url_userinfo("https://alice:s3cret@node.example"),
            "https://***@node.example/"
        );
        // A bare username still identifies someone.
        assert_eq!(
            redact_url_userinfo("https://alice@node.example"),
            "https://***@node.example/"
        );
        assert_eq!(
            redact_url_userinfo("https://alice:s3cret@node.example:9778/base"),
            "https://***@node.example:9778/base"
        );
        // The password must be gone, not merely masked alongside a surviving copy.
        assert!(!redact_url_userinfo("https://alice:s3cret@node.example").contains("s3cret"));
    }

    /// It must not mangle the ordinary case, and an `@` after the authority is
    /// path or query data, not credentials.
    ///
    /// Compared against the PARSER's canonical form rather than byte-identity: redaction now
    /// round-trips through `url::Url`, which appends a root path (`https://rpc.dig.net` ->
    /// `https://rpc.dig.net/`). That normalization is the point — the string shown is the one
    /// the parser resolved — so the assertion checks the host and userinfo survive, not that the
    /// bytes are untouched.
    #[test]
    fn redaction_leaves_a_credential_free_url_alone() {
        for url in [
            "https://rpc.dig.net",
            "http://dig.local",
            "http://localhost:9778",
            "https://node.example/path/with@sign",
        ] {
            let shown = redact_url_userinfo(url);
            let expected = ::url::Url::parse(url).unwrap().to_string();
            assert_eq!(
                shown, expected,
                "must not rewrite {url} beyond canonicalizing"
            );
            assert!(!shown.contains("***"), "{url} has no credentials to redact");
        }
    }

    /// A value the parser cannot read has no authority to reason about, so nothing
    /// attacker-chosen may be echoed. Falling back to string surgery here is exactly
    /// the divergence the parser-based redactor exists to remove.
    #[test]
    fn an_unparseable_url_is_not_echoed() {
        for bad in ["not-a-url", "://", "https://", ""] {
            assert_eq!(
                redact_url_userinfo(bad),
                "<unparseable URL>",
                "{bad:?} must not be echoed back"
            );
        }
    }

    /// THE ROUND-TWO ATTACK. Pure ASCII, no control characters, no whitespace — so the
    /// control-character guard does not fire and redaction is the only thing standing between
    /// the user and a spoofed prompt.
    ///
    /// The old redactor terminated the authority at `/` only. The WHATWG parser — the same crate
    /// reqwest dials with — also terminates it at `\`, `?` and `#` for special schemes, so
    /// `https://evil.example\@rpc.dig.net/` displayed as `https://***@rpc.dig.net/`: the official
    /// gateway's name, with credentials tidily hidden, while `evil.example` was dialled. The
    /// redaction made it MORE convincing than the raw string.
    #[test]
    fn a_backslash_or_query_or_fragment_cannot_disguise_the_real_host() {
        for payload in [
            "https://evil.example\\@rpc.dig.net/",
            "https://evil.example?@rpc.dig.net/",
            "https://evil.example#@rpc.dig.net/",
        ] {
            let shown = redact_url_userinfo(payload);
            let dialled = ::url::Url::parse(payload).unwrap();

            // The host the user is shown must be the host that will be contacted.
            assert_eq!(
                dialled.host_str(),
                Some("evil.example"),
                "fixture: {payload} must really resolve to evil.example, or this proves nothing"
            );
            assert!(
                shown.starts_with("https://evil.example"),
                "the display must lead with the host actually dialled; got {shown} for {payload}"
            );
            // And it must not read as though rpc.dig.net were the authority.
            assert!(
                !shown.starts_with("https://***@rpc.dig.net")
                    && !shown.starts_with("https://rpc.dig.net"),
                "the display disguised the host as rpc.dig.net: {shown}"
            );
        }
    }

    /// The same divergence, on the credential CHECK. A split-based test reported credentials in
    /// `evil.example\@host` (there are none — the `@` is in the path) and would refuse a
    /// perfectly legal URL while missing ones that do carry userinfo.
    #[test]
    fn userinfo_is_decided_by_the_parser_not_by_splitting() {
        assert!(has_userinfo("https://alice:s3cret@node.example"));
        assert!(has_userinfo("https://alice@node.example"));

        assert!(!has_userinfo("https://rpc.dig.net"));
        assert!(!has_userinfo("https://node.example/path/with@sign"));
        for payload in [
            "https://evil.example\\@rpc.dig.net/",
            "https://evil.example?@rpc.dig.net/",
            "https://evil.example#@rpc.dig.net/",
        ] {
            assert!(
                !has_userinfo(payload),
                "{payload} carries no userinfo — the @ is past the authority"
            );
        }
    }

    /// `.dig/node.toml` is a project file meant to travel with the repository.
    /// `digstore init` gitignores `.dig/`, so it is not committed by default, but a
    /// repo that un-ignores or force-adds it publishes whatever is inside — and
    /// redacting the echo afterwards does not reach git history. Refuse to store it.
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

#[cfg(test)]
mod display_spoofing_tests {
    use super::*;

    /// Writes `node.toml` VERBATIM, so the TOML the test declares is the TOML the
    /// parser sees. The escapes must stay as two characters (`\` then `n`) in the
    /// file: TOML decodes them into real control characters, and it is that
    /// decoded value the guard has to reject. A test that embedded a raw newline
    /// instead would fail at TOML parse and prove nothing about the guard.
    fn project_declaring(raw_toml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join(".dig");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("node.toml"), raw_toml).unwrap();
        (dir, ws)
    }

    /// The attack this closes, exactly as it was demonstrated end to end.
    ///
    /// The WHATWG URL parser strips ASCII tab/LF/CR before parsing, so these bytes break the
    /// approval prompt across lines and then vanish before the dial. The prompt's first line
    /// reads `https://rpc.dig.net` — the gateway our own NO_LOCAL_NODE text tells users to
    /// configure — while the host actually contacted is `rpc.dig.net.evil.example`.
    #[test]
    fn a_node_url_that_can_spoof_its_own_display_is_refused_at_read_time() {
        let (_d, ws) = project_declaring(
            "[node]\nurl = \"https://rpc.dig.net\\n\\t\\t\\t\\t.evil.example/\"\n",
        );

        // Fixture check: TOML must have decoded the escapes into real control
        // characters. If it did not, the assertion below would pass without the
        // guard ever being exercised.
        let raw = fs::read_to_string(ws.join("node.toml")).unwrap();
        assert!(
            raw.contains("\\n"),
            "the file must hold the two-character escape, not a raw newline"
        );

        let err = get_project_node_url_in(&ws).expect_err("a multi-line URL must be refused");

        let msg = err.to_string();
        assert!(
            msg.contains("line feed"),
            "the refusal must name the offending byte: {msg}"
        );
        assert!(
            !msg.contains('\n'),
            "the refusal must not itself be scrambled by the payload: {msg}"
        );
    }

    /// Every byte the URL parser silently discards, plus a plain space.
    #[test]
    fn every_control_character_and_whitespace_is_refused() {
        for (escape, label) in [
            ("\\n", "line feed"),
            ("\\r", "carriage return"),
            ("\\t", "tab"),
            ("\\u0000", "NUL"),
            ("\\u000B", "vertical tab"),
        ] {
            let (_d, ws) = project_declaring(&format!(
                "[node]\nurl = \"https://good.example{escape}.evil.example\"\n"
            ));
            assert!(
                get_project_node_url_in(&ws).is_err(),
                "a URL containing a {label} must be refused"
            );
        }

        let (_d, ws) = project_declaring("[node]\nurl = \"https://good.example /x\"\n");
        assert!(
            get_project_node_url_in(&ws).is_err(),
            "a URL containing a space must be refused"
        );
    }

    /// Non-ASCII is refused too, and it needs its own assertion: homographs and invisible
    /// separators are a spoofing family distinct from the ASCII control characters above, and
    /// no DIG endpoint — `dig.local`, `localhost`, `rpc.dig.net` — needs a byte above 0x7F.
    ///
    /// Written with real characters rather than `\u{...}` escapes so the payload in the source
    /// is the payload TOML receives. Dropping `!c.is_ascii()` from the predicate makes this fail.
    #[test]
    fn a_non_ascii_node_url_is_refused() {
        let cyrillic_e = '\u{0435}'; // looks identical to ASCII 'e' in most fonts
        let zero_width = '\u{200B}';
        let line_sep = '\u{2028}';
        let bom = '\u{FEFF}';

        for (payload, what) in [
            (
                format!("https://rpc.dig.n{cyrillic_e}t"),
                "Cyrillic homograph",
            ),
            (
                format!("https://rpc.dig.net{zero_width}.evil.example"),
                "zero-width space",
            ),
            (
                format!("https://rpc.dig.net{line_sep}.evil.example"),
                "line separator",
            ),
            (format!("https://rpc.dig.net{bom}"), "byte-order mark"),
        ] {
            let (_d, ws) = project_declaring(&format!("[node]\nurl = \"{payload}\"\n"));
            assert!(
                get_project_node_url_in(&ws).is_err(),
                "a URL containing a {what} must be refused"
            );
        }
    }

    /// …and an ordinary URL still reads back untouched, so this is a guard rather
    /// than a blanket refusal that would break every legitimate project.
    #[test]
    fn an_ordinary_project_node_url_still_reads_back() {
        for good in [
            "https://rpc.dig.net",
            "http://dig.local",
            "http://localhost:9778",
            "https://node.example:9778/base-path",
        ] {
            let (_d, ws) = project_declaring(&format!("[node]\nurl = \"{good}\"\n"));
            assert_eq!(
                get_project_node_url_in(&ws).unwrap().as_deref(),
                Some(good),
                "{good} must be accepted unchanged"
            );
        }
    }

    /// The `--json` path is the one that leaked. The newtype makes the redacted
    /// rendering the ONLY rendering, so a sink cannot obtain a raw value by omission.
    #[test]
    fn a_redacted_url_serializes_redacted() {
        let r = RedactedUrl::new("https://alice:s3cr3tT0K3N@node.example");

        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("s3cr3tT0K3N"),
            "serialization leaked the credential: {json}"
        );
        // Trailing `/` is the parser's canonical form — redaction round-trips through
        // `url::Url` so display and dial cannot disagree.
        assert_eq!(json, "\"https://***@node.example/\"");
        assert_eq!(r.to_string(), "https://***@node.example/");

        // The exact shape `digstore config node.url --show --json` emits.
        let body =
            serde_json::json!({ "node_url": RedactedUrl::new("https://bob:hunter2@n.example") });
        assert!(
            !body.to_string().contains("hunter2"),
            "the json body leaked the credential: {body}"
        );
    }
}
