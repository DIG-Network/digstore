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
    resolve_with_probe(ctx, ui, node_flag, &HttpHealthProbe::default()).await
}

/// [`resolve_node_in`] with the reachability probe injected.
///
/// The probe is a seam so tests can assert THIS layer's job — assembling the
/// override tiers and the candidate list and handing both to the shared
/// resolver — without depending on whether the machine running the test
/// happens to have a dig-node listening. (It matters: the previous test here
/// asserted "falls through to the public gateway", which only held because the
/// candidate URLs were wrong. Once they were corrected it failed on any
/// developer machine with a node installed — a green that had been measuring
/// the bug.)
async fn resolve_with_probe(
    ctx: Option<&CliContext>,
    ui: Option<&crate::ui::Ui>,
    node_flag: Option<&str>,
    probe: &dyn digstore_remote::HealthProbe,
) -> Result<ResolvedNode, CliError> {
    let overrides = override_inputs(ctx, ui, node_flag)?;
    Ok(resolver_resolve_node(
        &overrides,
        &local_candidates(),
        probe,
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

    match decide_origin(&resolved, requirement) {
        OriginDecision::Use(url) => Ok(url),
        OriginDecision::UseWithNotice(url) => {
            ui.hint(format!(
                "no local DIG node answered — reading from {url} instead. \
                 Run `dig-node status` to check yours, or see https://docs.dig.net/docs/run-a-node"
            ));
            Ok(url)
        }
        OriginDecision::Refuse => Err(CliError::NoLocalNode {
            operation: operation.to_string(),
        }),
    }
}

/// What to do with a resolved node, given what the operation needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginDecision {
    /// Go ahead silently — this is a node the user chose or runs.
    Use(String),
    /// Go ahead, but tell the user their read is leaving this machine.
    UseWithNotice(String),
    /// Refuse: the operation needs a local node and there is none.
    Refuse,
}

/// The read/write split, as a pure function of the resolved node (#2099).
///
/// Extracted from [`resolve_remote_or_origin`] so the rule can be tested
/// without a network: the surrounding function's other half is I/O (probing and
/// printing), and a rule this load-bearing should not be reachable only through
/// it.
pub fn decide_origin(resolved: &ResolvedNode, requirement: NodeRequirement) -> OriginDecision {
    if resolved.is_local() {
        return OriginDecision::Use(resolved.base_url.clone());
    }
    match requirement {
        NodeRequirement::Read => OriginDecision::UseWithNotice(resolved.base_url.clone()),
        NodeRequirement::LocalNode => OriginDecision::Refuse,
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
        let overrides = override_inputs(None, None, Some("https://flag-node.example")).unwrap();
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
        let overrides = override_inputs(None, None, None).unwrap();
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
        let overrides = override_inputs(None, None, None).unwrap();
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
        let overrides = override_inputs(None, None, None).unwrap();
        assert_eq!(overrides.env_var, None);
        clear_env();
    }

    /// #2099 regression. These URLs are the whole defect: the shipped ladder
    /// probed `https://dig.local:9778` and `https://localhost:9778`, neither of
    /// which any dig-node has ever bound, so the ladder could not find a local
    /// node on ANY machine. Pinned literally against `dig-node/SPEC.md`
    /// §4.1/§4.1a — a candidate list is only as good as its agreement with what
    /// the server actually listens on, and nothing else in this repo checks it.
    #[test]
    fn local_candidates_match_the_addresses_dig_node_binds() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let urls: Vec<String> = local_candidates().into_iter().map(|c| c.url).collect();
        assert_eq!(
            urls,
            vec![
                // 127.0.0.2:443 — portless, TLS (SPEC 4.1a listener 1).
                "https://dig.local".to_string(),
                // 127.0.0.2:80 — portless, plaintext (SPEC 4.1 listener 3).
                "http://dig.local".to_string(),
                // 127.0.0.1/[::1]:9778 — PLAINTEXT, never TLS (SPEC 4.1 listener 1/2).
                "http://localhost:9778".to_string(),
            ]
        );
        // The tiers must be labelled correctly too, or a dig.local hit would be
        // reported (and cached/diagnosed) as a localhost hit.
        let tiers: Vec<ResolvedTier> = local_candidates().into_iter().map(|c| c.tier).collect();
        assert_eq!(
            tiers,
            vec![
                ResolvedTier::DigLocal,
                ResolvedTier::DigLocal,
                ResolvedTier::Localhost
            ]
        );
    }

    /// `DIG_NODE_PORT` moves ONLY the loopback listener: the `dig.local` binds
    /// are pinned to 443/80 by the `127.0.0.2` hosts alias. Asserting the
    /// dig.local rungs are UNCHANGED is the load-bearing half — an
    /// implementation that applied the port everywhere would still pass a test
    /// that only checked the localhost rung.
    #[test]
    fn dig_node_port_moves_only_the_loopback_rung() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("DIG_NODE_PORT", "12345");
        let urls: Vec<String> = local_candidates().into_iter().map(|c| c.url).collect();
        assert_eq!(
            urls,
            vec![
                "https://dig.local".to_string(),
                "http://dig.local".to_string(),
                "http://localhost:12345".to_string(),
            ]
        );
        clear_env();
    }

    /// A `DIG_NODE_PORT` of 0 is not a port; dig-node itself treats it as unset
    /// (`SPEC.md` §3.2), so the ladder must not probe `localhost:0`.
    #[test]
    fn a_zero_dig_node_port_falls_back_to_the_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("DIG_NODE_PORT", "0");
        let urls: Vec<String> = local_candidates().into_iter().map(|c| c.url).collect();
        assert_eq!(urls[2], "http://localhost:9778");
        clear_env();
    }

    // -----------------------------------------------------------------------
    // The read/write split (#2099 deliverable 3).
    //
    // Each case varies exactly ONE input against a common baseline, so a
    // collapsed implementation — one that always refuses, always allows, or
    // ignores the tier — is caught by at least one of them.
    // -----------------------------------------------------------------------

    fn node(tier: digstore_remote::ResolvedTier) -> ResolvedNode {
        ResolvedNode {
            base_url: "https://rpc.dig.net".into(),
            tier,
        }
    }

    /// A read with no local node proceeds — §6.0 keeps consuming frictionless —
    /// but is NOT silent. Asserting `UseWithNotice` (not merely `Use`) is the
    /// load-bearing half: the user explicitly asked to be told.
    #[test]
    fn a_read_with_no_local_node_proceeds_but_is_announced() {
        assert_eq!(
            decide_origin(
                &node(digstore_remote::ResolvedTier::PublicGateway),
                NodeRequirement::Read
            ),
            OriginDecision::UseWithNotice("https://rpc.dig.net".into())
        );
    }

    /// The same resolved node, the same absent local node — only the
    /// REQUIREMENT differs — must refuse. This is the pair that proves the
    /// split exists rather than one branch being dead.
    #[test]
    fn an_identity_signed_write_with_no_local_node_refuses() {
        assert_eq!(
            decide_origin(
                &node(digstore_remote::ResolvedTier::PublicGateway),
                NodeRequirement::LocalNode
            ),
            OriginDecision::Refuse
        );
    }

    /// A write against a node the user actually runs proceeds silently — the
    /// refusal must be about "no local node", not about writes in general.
    #[test]
    fn a_write_against_a_local_node_proceeds_silently() {
        for tier in [
            digstore_remote::ResolvedTier::DigLocal,
            digstore_remote::ResolvedTier::Localhost,
        ] {
            assert_eq!(
                decide_origin(&node(tier), NodeRequirement::LocalNode),
                OriginDecision::Use("https://rpc.dig.net".into()),
                "a {tier:?} node must satisfy a write"
            );
        }
    }

    /// `--node https://rpc.dig.net` is the documented escape hatch, and the
    /// error message tells users to reach for it — so an override must satisfy
    /// a write EVEN THOUGH the URL is the public gateway. The base_url here is
    /// identical to the refusing case above; only the tier differs, so an
    /// implementation that refused on the URL rather than on how it was chosen
    /// fails here.
    #[test]
    fn an_explicitly_chosen_gateway_satisfies_a_write() {
        assert_eq!(
            decide_origin(
                &node(digstore_remote::ResolvedTier::Override),
                NodeRequirement::LocalNode
            ),
            OriginDecision::Use("https://rpc.dig.net".into())
        );
    }

    /// The error names the failing operation and carries BOTH things the user
    /// asked for: how to check the node, and where to get it.
    #[test]
    fn the_no_local_node_error_tells_the_user_how_to_check_and_where_to_download() {
        let msg = CliError::NoLocalNode {
            operation: "push".into(),
        }
        .to_string();
        assert!(msg.contains("push"), "must name the operation: {msg}");
        assert!(msg.contains("dig-node status"), "must say how to check: {msg}");
        assert!(
            msg.contains("https://dig.net/install.sh")
                && msg.contains("https://dig.net/install.ps1"),
            "must give the installer for both platform families: {msg}"
        );
        assert!(
            msg.contains("https://docs.dig.net/docs/run-a-node"),
            "must link the published docs page: {msg}"
        );
        assert_eq!(
            CliError::NoLocalNode {
                operation: "push".into()
            }
            .code(),
            "NO_LOCAL_NODE"
        );
    }

    /// A probe scripted per URL, so this layer's wiring is tested against a
    /// KNOWN world rather than the developer's machine.
    struct ScriptedProbe(std::collections::HashMap<String, bool>);

    #[async_trait::async_trait]
    impl digstore_remote::HealthProbe for ScriptedProbe {
        async fn probe(&self, base_url: &str, _timeout: std::time::Duration) -> bool {
            self.0.get(base_url).copied().unwrap_or(false)
        }
    }

    fn scripted(live: &[&str]) -> ScriptedProbe {
        ScriptedProbe(live.iter().map(|u| (u.to_string(), true)).collect())
    }

    /// The #2099 acceptance case: a node listening where dig-node ACTUALLY
    /// listens is found, and the ladder does not reach the public gateway.
    /// Only the plaintext `dig.local` rung is live — the shape on a machine
    /// whose installer has not provisioned a TLS leaf, which is the common one.
    #[tokio::test]
    async fn a_node_on_a_real_dig_node_address_is_found_before_the_gateway() {
        {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            clear_env();
        }
        let probe = scripted(&["http://dig.local"]);
        let resolved = resolve_with_probe(None, None, None, &probe).await.unwrap();
        assert_eq!(resolved.base_url, "http://dig.local");
        assert_eq!(resolved.tier, digstore_remote::ResolvedTier::DigLocal);
        assert!(resolved.is_local());
    }

    /// Same, for the always-on loopback listener — the rung that exists on
    /// every install, TLS leaf or not, privileged ports or not.
    #[tokio::test]
    async fn a_plaintext_loopback_node_is_found_before_the_gateway() {
        {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            clear_env();
        }
        let probe = scripted(&["http://localhost:9778"]);
        let resolved = resolve_with_probe(None, None, None, &probe).await.unwrap();
        assert_eq!(resolved.base_url, "http://localhost:9778");
        assert_eq!(resolved.tier, digstore_remote::ResolvedTier::Localhost);
        assert!(resolved.is_local());
    }

    /// With genuinely nothing listening anywhere the ladder still must not
    /// fail — the public gateway is the documented backstop.
    #[tokio::test]
    async fn resolve_node_returns_public_gateway_when_nothing_local_answers() {
        {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            clear_env();
        }
        let probe = scripted(&[]);
        let resolved = resolve_with_probe(None, None, None, &probe).await.unwrap();
        assert_eq!(resolved.base_url, digstore_remote::RPC_DIG_NET);
        assert_eq!(resolved.tier, digstore_remote::ResolvedTier::PublicGateway);
        assert!(!resolved.is_local());
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
