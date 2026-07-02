//! CLI-side wiring for the client→node resolution ladder (`CLAUDE.md` §5.3):
//! assembles the override precedence (`--node` flag > `$DIG_NODE_URL` >
//! persisted `digstore config node.url`) and the `dig.local`/`localhost`
//! candidate URLs, then delegates the actual probing/fall-through logic to
//! [`digstore_remote::resolver`] (kept there so `digstore-remote`, the crate
//! that owns [`digstore_remote::DigClient`], is the single source of truth for
//! the ladder — this module only supplies the CLI-specific inputs).

use digstore_remote::{
    resolve_node as resolver_resolve_node, HttpHealthProbe, OverrideInputs, ResolvedNode,
    DEFAULT_LOCAL_NODE_PORT, DEFAULT_PROBE_TIMEOUT, DIG_LOCAL_HOST,
};

use crate::config;
use crate::error::CliError;

/// The environment variable an explicit node override can be supplied through
/// (§5.3 tier-1 override, second-highest precedence after `--node`).
pub const DIG_NODE_URL_ENV: &str = "DIG_NODE_URL";

/// Build the [`OverrideInputs`] for THIS invocation: `flag` is the `--node`
/// value already parsed by clap (`Cli.node`); `env_var` is read fresh from
/// `$DIG_NODE_URL`; `config_value` is the persisted `node.url`, read lazily
/// (only when neither of the higher-precedence sources is set, so a
/// `--node`/env-only invocation never touches disk).
fn override_inputs(node_flag: Option<&str>) -> Result<OverrideInputs, CliError> {
    let flag = node_flag.map(|s| s.to_string());
    let env_var = std::env::var(DIG_NODE_URL_ENV)
        .ok()
        .filter(|s| !s.is_empty());
    let config_value = if flag.is_some() || env_var.is_some() {
        None // higher-precedence source already present; skip the disk read
    } else {
        config::get_node_url()?
    };
    Ok(OverrideInputs {
        flag,
        env_var,
        config_value,
    })
}

/// The default local-node candidate URLs the ladder probes when no override
/// wins: `https://dig.local:{port}` and `https://localhost:{port}`. The port
/// honors `DIG_NODE_PORT` (mirroring `dig-node`'s own env var,
/// `dig-node/SPEC.md` §1.1 "Configuration") so a node running on a
/// non-default port is still found.
fn local_candidate_urls() -> (String, String) {
    let port: u16 = std::env::var("DIG_NODE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_LOCAL_NODE_PORT);
    (
        format!("https://{DIG_LOCAL_HOST}:{port}"),
        format!("https://localhost:{port}"),
    )
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
    let overrides = override_inputs(node_flag)?;
    let (dig_local, localhost) = local_candidate_urls();
    let probe = HttpHealthProbe::default();
    Ok(resolver_resolve_node(
        &overrides,
        &dig_local,
        &localhost,
        &probe,
        DEFAULT_PROBE_TIMEOUT,
    )
    .await)
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
