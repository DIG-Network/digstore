//! CLI-side wiring for the client→node resolution ladder (`CLAUDE.md` §5.3):
//! assembles the override precedence (`--node` flag > `$DIG_NODE_URL` >
//! persisted `digstore config node.url`) and the `dig.local`/`localhost`
//! candidate URLs, then delegates the actual probing/fall-through logic to
//! [`digstore_remote::resolver`] (kept there so `digstore-remote`, the crate
//! that owns [`digstore_remote::DigClient`], is the single source of truth for
//! the ladder — this module only supplies the CLI-specific inputs).

use digstore_remote::{
    resolve_node as resolver_resolve_node, HttpHealthProbe, LadderCandidate, OverrideInputs,
    ResolvedNode, ResolvedTier, DEFAULT_LOCAL_NODE_PORT, DEFAULT_PROBE_TIMEOUT, DIG_LOCAL_HOST,
};

use crate::config;
use crate::context::CliContext;
use crate::error::CliError;

/// The environment variable an explicit node override can be supplied through
/// (§5.3 tier-1 override, second-highest precedence after `--node`).
pub const DIG_NODE_URL_ENV: &str = "DIG_NODE_URL";

/// Build the [`OverrideInputs`] for THIS invocation: `flag` is the `--node`
/// value already parsed by clap (`Cli.node`); `env_var` is read fresh from
/// `$DIG_NODE_URL`; `project_value` is the nearest-ancestor `.dig/node.toml`
/// value, admitted ONLY if the user has approved it (see
/// [`trusted_project_node`]); `config_value` is the machine-wide persisted
/// `node.url`.
///
/// The lower tiers are read lazily — a `--node`/env-only invocation never
/// touches disk — but note that laziness also means an UNTRUSTED project value
/// is never even consulted when a higher tier already decided the answer, so
/// there is no warning spam on invocations the project file cannot influence.
fn override_inputs(
    ctx: Option<&CliContext>,
    ui: Option<&crate::ui::Ui>,
    node_flag: Option<&str>,
) -> Result<OverrideInputs, CliError> {
    let flag = node_flag.map(|s| s.to_string());
    let env_var = std::env::var(DIG_NODE_URL_ENV)
        .ok()
        .filter(|s| !s.is_empty());
    if flag.is_some() || env_var.is_some() {
        // A higher-precedence source already decided this; skip both disk reads.
        return Ok(OverrideInputs {
            flag,
            env_var,
            ..Default::default()
        });
    }
    let project_value = match ctx {
        Some(ctx) => trusted_project_node(ctx, ui)?,
        None => None,
    };
    let config_value = if project_value.is_some() {
        None
    } else {
        config::get_node_url()?
    };
    Ok(OverrideInputs {
        flag,
        env_var,
        project_value,
        config_value,
    })
}

/// The project's declared node URL, but ONLY when the user has approved this
/// exact directory+URL pair; otherwise [`None`] plus a warning.
///
/// `.dig/node.toml` can arrive inside a cloned repository, and every request
/// digs sends to the resolved node carries the caller's §21.9 identity
/// SIGNATURE. Honouring a repo-supplied endpoint on sight would therefore let a
/// hostile repo harvest signatures and content simply by being checked out and
/// read — so the file is treated as an untrusted input that PROPOSES a value,
/// and the machine-local trust store is what authorizes it.
///
/// Approval is granted by `digstore config node.url --local <url>` (typing the
/// URL is the approval) or by confirming the prompt below. When digs cannot
/// prompt — a script, a pipeline, `--json` — the answer is always "no": a
/// non-interactive run must never silently adopt a repo's endpoint.
fn trusted_project_node(
    ctx: &CliContext,
    ui: Option<&crate::ui::Ui>,
) -> Result<Option<String>, CliError> {
    let Some(url) = config::get_project_node_url_in(&ctx.workspace_dir)? else {
        return Ok(None);
    };
    let global = config::global_config_dir()?;
    if config::is_project_node_trusted_in(&global, &ctx.workspace_dir, &url)? {
        return Ok(Some(url));
    }

    let Some(ui) = ui else {
        return Ok(None);
    };
    if !ui.can_prompt() {
        ui.hint(format!(
            "ignoring this project's node.url ({url}): it has not been approved on this machine. \
             Run `digstore config node.url --local {url}` to approve it."
        ));
        return Ok(None);
    }
    ui.line(format!(
        "This project asks digs to use the node {url}.\n\
         It comes from {}, which can travel inside a repository — and digs signs every \
         request it sends to that node with your identity key.",
        config::project_node_path(&ctx.workspace_dir).display()
    ));
    if !ui.confirm("Use this node for this project?", false) {
        ui.hint("using the standard node ladder instead");
        return Ok(None);
    }
    config::trust_project_node_in(&global, &ctx.workspace_dir, &url)?;
    Ok(Some(url))
}

/// The local-node candidates the ladder probes when no override wins, in order.
///
/// These MUST mirror the addresses `dig-node` actually binds
/// (`dig-node/SPEC.md` §4.1, §4.1a) — a candidate pointing anywhere else can
/// never match a real install, and the ladder then silently reports "no local
/// node" on a machine that is running one:
///
/// 1. `https://dig.local` — the `127.0.0.2:443` TLS listener. **Portless**: the
///    installer's hosts entry maps `dig.local` to `127.0.0.2`, and the listener
///    is on the default TLS port. Present only once dig-cert has provisioned a
///    leaf, so it is tried first but must fall through when absent.
/// 2. `http://dig.local` — the `127.0.0.2:80` plaintext listener. Also
///    portless, and the fail-soft surface when no leaf exists.
/// 3. `http://localhost:{port}` — the always-on loopback listener. It is
///    **plaintext**, never TLS, and dual-binds `127.0.0.1`/`[::1]` so
///    `localhost` reaches it whichever address family the resolver prefers.
///
/// Only rung 3 is port-configurable: `DIG_NODE_PORT` (default
/// [`DEFAULT_LOCAL_NODE_PORT`] = 9778) moves the loopback listener, while the
/// `dig.local` binds are fixed at 443/80 by the `127.0.0.2` alias.
fn local_candidates() -> Vec<LadderCandidate> {
    let port: u16 = std::env::var("DIG_NODE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_LOCAL_NODE_PORT);
    vec![
        LadderCandidate::new(format!("https://{DIG_LOCAL_HOST}"), ResolvedTier::DigLocal),
        LadderCandidate::new(format!("http://{DIG_LOCAL_HOST}"), ResolvedTier::DigLocal),
        LadderCandidate::new(format!("http://localhost:{port}"), ResolvedTier::Localhost),
    ]
}

/// Resolve the node endpoint for THIS invocation per `CLAUDE.md` §5.3:
/// override (`--node` > `$DIG_NODE_URL` > `node.url`) > `dig.local` >
/// `localhost` > `rpc.dig.net`. `node_flag` is `Cli.node` (already parsed).
///
/// Callers that need the node endpoint more than once within one command
/// should resolve it ONCE and reuse the result (the "cache the resolved
/// choice for the invocation" requirement) rather than calling this
/// repeatedly — [`digstore_remote::CachedResolver`] is available for a
/// long-lived context that wants that automatically.
pub async fn resolve_node(node_flag: Option<&str>) -> Result<ResolvedNode, CliError> {
    resolve_node_in(None, None, node_flag).await
}

/// [`resolve_node`] with the project context available, so the per-directory
/// `.dig/node.toml` tier participates. `ui` is optional: without it the project
/// value can only be used if it was already trusted (there is nobody to ask).
pub async fn resolve_node_in(
    ctx: Option<&CliContext>,
    ui: Option<&crate::ui::Ui>,
    node_flag: Option<&str>,
) -> Result<ResolvedNode, CliError> {
    let overrides = override_inputs(ctx, ui, node_flag)?;
    let probe = HttpHealthProbe::default();
    Ok(resolver_resolve_node(
        &overrides,
        &local_candidates(),
        &probe,
        DEFAULT_PROBE_TIMEOUT,
    )
    .await)
}

/// What an operation needs from the node it is about to talk to.
///
/// The split exists because §5.3's third rung and the user's demand for a hard
/// error pull in opposite directions, and both are right for different work
/// (#2099):
///
/// - [`Read`](NodeRequirement::Read) — consuming content. Falling through to
///   the public gateway is CORRECT here: someone with no node installed should
///   still be able to read (`CLAUDE.md` §6.0). It is not silent, though — the
///   caller is told which remote host answered.
/// - [`LocalNode`](NodeRequirement::LocalNode) — publishing and other
///   identity-signed writes. Falling through would ship the user's content and
///   their §21.9 request signatures to a public server they never chose, so
///   this refuses with [`CliError::NoLocalNode`] instead.
///
/// An explicit override (`--node`, `$DIG_NODE_URL`, either config scope, or a
/// configured `origin` remote) satisfies BOTH: a deliberately-named endpoint is
/// the user's own choice, even when it happens to be `rpc.dig.net`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRequirement {
    Read,
    LocalNode,
}

/// Resolve the base URL for the §21 remote `name`.
///
/// An explicitly-configured remote of that name always wins. Otherwise —
/// and this is the #2099 behaviour change — `origin` defaults to the node
/// resolved by the §5.3 ladder (the user's OWN node) rather than to a
/// hard-coded `https://rpc.dig.net`. Any other unknown name is still an error.
///
/// `operation` names the command for the error message ("`push` needs one").
pub fn resolve_remote_or_origin(
    ctx: &CliContext,
    ui: &crate::ui::Ui,
    name: &str,
    node_flag: Option<&str>,
    operation: &str,
    requirement: NodeRequirement,
) -> Result<String, CliError> {
    if let Some(configured) = config::configured_remote_url(ctx, name)? {
        return Ok(configured);
    }
    if name != "origin" {
        return Err(CliError::NotFound(format!("remote {name}")));
    }

    let resolved = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?
        .block_on(resolve_node_in(Some(ctx), Some(ui), node_flag))?;

    if resolved.is_local() {
        return Ok(resolved.base_url);
    }
    match requirement {
        NodeRequirement::LocalNode => Err(CliError::NoLocalNode {
            operation: operation.to_string(),
        }),
        NodeRequirement::Read => {
            ui.hint(format!(
                "no local DIG node answered — reading from {} instead. \
                 Run `dig-node status` to check yours, or see https://docs.dig.net/docs/run-a-node",
                resolved.base_url
            ));
            Ok(resolved.base_url)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::DIG_IDENTITY_DIR_ENV_LOCK as ENV_LOCK;

    // `DIG_NODE_URL`/`DIG_NODE_PORT` are process-global; some tests here ALSO
    // touch `DIG_IDENTITY_DIR` (to isolate `config::set_node_url`'s default
    // location), which `ops::identity`'s tests mutate too — the SHARED
    // `testutil` lock is what actually serializes across both modules (a
    // private per-module lock does not).
    fn clear_env() {
        std::env::remove_var(DIG_NODE_URL_ENV);
        std::env::remove_var("DIG_NODE_PORT");
        std::env::remove_var("DIG_IDENTITY_DIR");
    }

    #[test]
    fn override_inputs_flag_beats_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(DIG_NODE_URL_ENV, "https://env-node.example");
        let overrides = override_inputs(Some("https://flag-node.example")).unwrap();
        assert_eq!(overrides.flag.as_deref(), Some("https://flag-node.example"));
        assert_eq!(
            digstore_remote::override_source(&overrides),
            Some(digstore_remote::OverrideSource::Flag)
        );
        clear_env();
    }

    #[test]
    fn override_inputs_env_used_when_no_flag() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(DIG_NODE_URL_ENV, "https://env-node.example");
        let overrides = override_inputs(None).unwrap();
        assert_eq!(
            digstore_remote::override_source(&overrides),
            Some(digstore_remote::OverrideSource::Env)
        );
        clear_env();
    }

    #[test]
    fn override_inputs_falls_back_to_persisted_config() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("DIG_IDENTITY_DIR", td.path());
        config::set_node_url("https://persisted-node.example").unwrap();
        let overrides = override_inputs(None).unwrap();
        assert_eq!(
            overrides.config_value.as_deref(),
            Some("https://persisted-node.example")
        );
        assert_eq!(
            digstore_remote::override_source(&overrides),
            Some(digstore_remote::OverrideSource::Config)
        );
        clear_env();
    }

    #[test]
    fn override_inputs_empty_env_var_is_ignored() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(DIG_NODE_URL_ENV, "");
        let overrides = override_inputs(None).unwrap();
        assert_eq!(overrides.env_var, None);
        clear_env();
    }

    #[test]
    fn local_candidate_urls_use_default_port() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let (dig_local, localhost) = local_candidate_urls();
        assert_eq!(dig_local, "https://dig.local:9778");
        assert_eq!(localhost, "https://localhost:9778");
    }

    #[test]
    fn local_candidate_urls_honor_dig_node_port_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("DIG_NODE_PORT", "12345");
        let (dig_local, localhost) = local_candidate_urls();
        assert_eq!(dig_local, "https://dig.local:12345");
        assert_eq!(localhost, "https://localhost:12345");
        clear_env();
    }

    #[tokio::test]
    async fn resolve_node_returns_public_gateway_when_nothing_local_answers() {
        // Scope the lock to the synchronous env setup/teardown only — holding a
        // `std::sync::Mutex` guard across an `.await` is a clippy
        // `await_holding_lock` violation (and a real deadlock risk under a
        // multi-threaded runtime). `resolve_node` itself reads no env vars once
        // called (its inputs are captured synchronously inside), so it is safe
        // to run the async ladder walk AFTER releasing the guard.
        {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            clear_env();
        }
        // No local node is running in CI/dev sandboxes, and no override is set,
        // so the ladder MUST fall all the way through to the public gateway
        // rather than erroring — this is the "never fails" contract.
        let resolved = resolve_node(None).await.unwrap();
        assert_eq!(resolved.base_url, digstore_remote::RPC_DIG_NET);
        assert_eq!(resolved.tier, digstore_remote::ResolvedTier::PublicGateway);
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
    }

    #[tokio::test]
    async fn resolve_node_honors_explicit_flag_override() {
        {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            clear_env();
        }
        let resolved = resolve_node(Some("https://my-node.example:9999"))
            .await
            .unwrap();
        assert_eq!(resolved.base_url, "https://my-node.example:9999");
        assert_eq!(resolved.tier, digstore_remote::ResolvedTier::Override);
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
    }
}
