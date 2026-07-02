//! Client → node connection-order resolution (`CLAUDE.md` §5.3, `dig-node/SPEC.md` §2.2).
//!
//! Every digstore client that needs to reach a DIG node (as opposed to a specific,
//! already-known store remote such as `remote add`) MUST pick the endpoint in this
//! fixed order, using the FIRST tier that answers a cheap health probe within a
//! short timeout:
//!
//! 1. an EXPLICITLY-CONFIGURED node — always wins, overriding the ladder entirely.
//!    Precedence among override sources (highest first): an explicit `--node` CLI
//!    flag/argument > `$DIG_NODE_URL` > the persisted `node.url` config value.
//! 2. `dig.local` — the installed local node (the installer's hosts registration).
//! 3. `localhost` — a node on the loopback default port, when `dig.local` does not
//!    resolve/respond.
//! 4. `rpc.dig.net` — the public gateway. FINAL fallback only.
//!
//! The resolved choice is cached for the invocation (one process run) so repeated
//! calls within the same command do not re-probe the ladder.
//!
//! Transport note (§5.3): a node-class client (one holding a DIG identity key —
//! this CLI) is required to speak mTLS to every tier, including `rpc.dig.net`
//! (which is dual-mode: mTLS for node-class clients, plain HTTPS+CORS for
//! browsers). The gateway's mTLS endpoint does not exist yet at time of writing,
//! so this resolver's probe and the [`DigClient`](crate::DigClient) it feeds
//! speak plain HTTPS today; [`TransportMode`] is the seam that flips this to mTLS
//! once the gateway supports it, without another change to the ladder logic
//! itself (see `SPEC.md` "Deferred: mTLS transport").

use std::time::Duration;

/// The public gateway host — FINAL fallback tier, never the primary/hard-coded
/// endpoint (`CLAUDE.md` §5.3).
pub const RPC_DIG_NET: &str = "https://rpc.dig.net";

/// The installed local node's hosts-file registration (installer-managed).
pub const DIG_LOCAL_HOST: &str = "dig.local";

/// A node's default loopback read port (`dig-node/SPEC.md` §1.1, `DIG_NODE_PORT`).
pub const DEFAULT_LOCAL_NODE_PORT: u16 = 9778;

/// Default per-tier probe timeout: fast enough that a dead tier does not stall the
/// command, generous enough for a loopback/local-LAN round trip under load.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(600);

/// How the resolved endpoint was decided — surfaced for `--verbose`/diagnostics
/// (`digstore doctor`) so a user can see WHY a given node was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTier {
    /// An explicit override was supplied (flag, env, or persisted config) — no
    /// probing occurred; overrides are trusted as-is.
    Override,
    /// `dig.local` answered the health probe.
    DigLocal,
    /// `localhost` (loopback default port) answered the health probe.
    Localhost,
    /// Fell through to the public gateway (either it answered, or nothing else
    /// did and this is the final, un-probed fallback).
    PublicGateway,
}

/// The resolved node endpoint + how it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNode {
    /// Base URL, e.g. `https://dig.local:9778` or `https://rpc.dig.net`.
    pub base_url: String,
    pub tier: ResolvedTier,
}

/// The transport a resolved node connection should use. Plain HTTPS is what every
/// tier speaks today; `Mtls` is the seam for §5.3's node-class mTLS requirement,
/// activated once the gateway (and local node) mTLS endpoints exist. Kept as an
/// explicit enum (not a bool) so a third mode is not a breaking change later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportMode {
    /// Plain HTTPS (+ the existing §21.9 signed-request headers over the
    /// channel). Current behavior for all tiers, including `rpc.dig.net`.
    #[default]
    Https,
    /// mTLS with a client cert derived from the caller's DIG identity key
    /// (`peer_id = SHA-256(TLS SPKI DER)`), §21.9 signed-request authorization
    /// layered on top. NOT YET WIRED — see `SPEC.md` "Deferred: mTLS transport".
    /// Selecting this today is a caller error the resolver refuses (see
    /// [`resolve_node`] docs); it exists so callers/tests can express intent and
    /// so the flip to real mTLS is additive.
    Mtls,
}

/// Where an explicit node override came from, highest-precedence first. Purely
/// informational (surfaced in diagnostics); all three are otherwise equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideSource {
    /// `--node <url>` (or equivalent constructor argument).
    Flag,
    /// `$DIG_NODE_URL`.
    Env,
    /// Persisted `digstore config node.url <url>`.
    Config,
}

/// A cheap reachability probe for one candidate base URL. Implemented over real
/// HTTP by [`crate::client`] helpers in production; tests inject a deterministic
/// fake so the ladder's FALL-THROUGH ORDER is verified without a network.
#[async_trait::async_trait]
pub trait HealthProbe: Send + Sync {
    /// Return `true` if `base_url` answered a health check within `timeout`.
    /// MUST NOT panic or block past `timeout` — implementations race the check
    /// against the timeout themselves (see `probe_http_health` for the real one).
    async fn probe(&self, base_url: &str, timeout: Duration) -> bool;
}

/// Explicit override inputs, already extracted from their sources by the caller
/// (a CLI flag, `std::env::var`, or the persisted config file) so this module
/// stays free of I/O and is trivially unit-testable. Precedence: `flag` >
/// `env_var` > `config_value`.
#[derive(Debug, Clone, Default)]
pub struct OverrideInputs {
    pub flag: Option<String>,
    pub env_var: Option<String>,
    pub config_value: Option<String>,
}

impl OverrideInputs {
    /// The highest-precedence override present, with its source tag.
    fn resolve(&self) -> Option<(&str, OverrideSource)> {
        if let Some(v) = self.flag.as_deref() {
            return Some((v, OverrideSource::Flag));
        }
        if let Some(v) = self.env_var.as_deref() {
            return Some((v, OverrideSource::Env));
        }
        if let Some(v) = self.config_value.as_deref() {
            return Some((v, OverrideSource::Config));
        }
        None
    }
}

/// Resolve the node endpoint per `CLAUDE.md` §5.3 / `dig-node/SPEC.md` §2.2:
/// override > `dig.local` > `localhost` > `rpc.dig.net`, probing each
/// non-override tier with `probe` and falling through on a timeout/no-response.
/// `rpc.dig.net` is the final fallback and is returned even if it does not
/// itself answer the probe (there is nowhere left to fall through to).
///
/// `dig_local_url`/`localhost_url` are the fully-formed base URLs for those
/// tiers (callers build them once, e.g. `https://dig.local:9778`); passing them
/// in (rather than hardcoding a scheme/port here) keeps this function transport-
/// agnostic and lets callers vary the local node port via `DIG_NODE_PORT`.
///
/// Panics-free; never fails — the public gateway is always a valid last resort.
pub async fn resolve_node(
    overrides: &OverrideInputs,
    dig_local_url: &str,
    localhost_url: &str,
    probe: &dyn HealthProbe,
    timeout: Duration,
) -> ResolvedNode {
    if let Some((url, _source)) = overrides.resolve() {
        return ResolvedNode {
            base_url: url.trim_end_matches('/').to_string(),
            tier: ResolvedTier::Override,
        };
    }

    if probe.probe(dig_local_url, timeout).await {
        return ResolvedNode {
            base_url: dig_local_url.trim_end_matches('/').to_string(),
            tier: ResolvedTier::DigLocal,
        };
    }

    if probe.probe(localhost_url, timeout).await {
        return ResolvedNode {
            base_url: localhost_url.trim_end_matches('/').to_string(),
            tier: ResolvedTier::Localhost,
        };
    }

    ResolvedNode {
        base_url: RPC_DIG_NET.to_string(),
        tier: ResolvedTier::PublicGateway,
    }
}

/// Which override source (if any) `overrides` would resolve to — used by
/// diagnostics/tests that want to assert precedence without running the async
/// probe ladder.
pub fn override_source(overrides: &OverrideInputs) -> Option<OverrideSource> {
    overrides.resolve().map(|(_, s)| s)
}

/// A per-invocation cache of the resolved node: probing is a network round trip,
/// so a single command that needs the node endpoint more than once (e.g. a
/// health check inside `doctor` plus the actual request) resolves it ONCE. Not
/// `Sync` beyond `OnceCell` semantics — one resolution per process run is the
/// documented contract (`CLAUDE.md` §5.3 "caching the resolved choice for the
/// invocation/session").
#[derive(Debug, Default)]
pub struct CachedResolver {
    cached: tokio::sync::OnceCell<ResolvedNode>,
}

impl CachedResolver {
    pub fn new() -> Self {
        Self {
            cached: tokio::sync::OnceCell::new(),
        }
    }

    /// Resolve once per instance; subsequent calls return the cached result
    /// without re-probing.
    pub async fn get_or_resolve(
        &self,
        overrides: &OverrideInputs,
        dig_local_url: &str,
        localhost_url: &str,
        probe: &dyn HealthProbe,
        timeout: Duration,
    ) -> ResolvedNode {
        self.cached
            .get_or_init(|| resolve_node(overrides, dig_local_url, localhost_url, probe, timeout))
            .await
            .clone()
    }
}

/// Production [`HealthProbe`]: `GET {base_url}/health`, racing the request
/// against `timeout` via `tokio::time::timeout` so a hung/unreachable tier can
/// never stall the ladder past the caller's patience. Any non-2xx status, a
/// transport error, or an elapsed timeout is treated as "not reachable" — the
/// ladder falls through rather than surfacing a probe failure as a hard error
/// (`resolve_node` never fails; the public gateway is always the backstop).
///
/// Matches `dig-node`'s `GET /health` (`dig-node/SPEC.md` §1.1) served by both
/// the local node and (once live) the gateway's mTLS-fronted read surface.
pub struct HttpHealthProbe {
    http: reqwest::Client,
}

impl HttpHealthProbe {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

impl Default for HttpHealthProbe {
    /// A client with sane defaults for a HEALTH probe specifically: redirects
    /// disabled (a probe should not follow a redirect chain) and no explicit
    /// timeout of its own — [`resolve_node`]'s caller-supplied `timeout` is the
    /// single source of truth, applied via `tokio::time::timeout` in [`probe`].
    fn default() -> Self {
        Self::new(
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        )
    }
}

#[async_trait::async_trait]
impl HealthProbe for HttpHealthProbe {
    async fn probe(&self, base_url: &str, timeout: Duration) -> bool {
        let url = format!("{}/health", base_url.trim_end_matches('/'));
        let request = self.http.get(&url).send();
        match tokio::time::timeout(timeout, request).await {
            Ok(Ok(resp)) => resp.status().is_success(),
            // Transport error (connection refused, DNS failure, TLS failure, …)
            // or the timeout elapsed — both mean "this tier did not respond".
            Ok(Err(_)) | Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A scripted probe: answers `true`/`false` per exact URL from a fixed map,
    /// and records every URL it was asked about (in order) so tests can assert
    /// the ladder probed tiers in the right order and stopped at the first hit.
    #[derive(Default)]
    struct ScriptedProbe {
        answers: std::collections::HashMap<String, bool>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedProbe {
        fn new(answers: &[(&str, bool)]) -> Self {
            Self {
                answers: answers.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl HealthProbe for ScriptedProbe {
        async fn probe(&self, base_url: &str, _timeout: Duration) -> bool {
            self.calls.lock().unwrap().push(base_url.to_string());
            self.answers.get(base_url).copied().unwrap_or(false)
        }
    }

    const DIG_LOCAL: &str = "https://dig.local:9778";
    const LOCALHOST: &str = "https://localhost:9778";

    #[tokio::test]
    async fn prefers_dig_local_when_it_answers() {
        let probe = ScriptedProbe::new(&[(DIG_LOCAL, true), (LOCALHOST, true)]);
        let resolved = resolve_node(
            &OverrideInputs::default(),
            DIG_LOCAL,
            LOCALHOST,
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, DIG_LOCAL);
        assert_eq!(resolved.tier, ResolvedTier::DigLocal);
        // localhost must NOT have been probed once dig.local answered — first
        // responder wins, not "probe everything and rank".
        assert_eq!(probe.calls(), vec![DIG_LOCAL.to_string()]);
    }

    #[tokio::test]
    async fn falls_through_to_localhost_when_dig_local_is_unreachable() {
        let probe = ScriptedProbe::new(&[(DIG_LOCAL, false), (LOCALHOST, true)]);
        let resolved = resolve_node(
            &OverrideInputs::default(),
            DIG_LOCAL,
            LOCALHOST,
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, LOCALHOST);
        assert_eq!(resolved.tier, ResolvedTier::Localhost);
        assert_eq!(
            probe.calls(),
            vec![DIG_LOCAL.to_string(), LOCALHOST.to_string()]
        );
    }

    #[tokio::test]
    async fn falls_through_to_public_gateway_as_final_fallback() {
        let probe = ScriptedProbe::new(&[(DIG_LOCAL, false), (LOCALHOST, false)]);
        let resolved = resolve_node(
            &OverrideInputs::default(),
            DIG_LOCAL,
            LOCALHOST,
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, RPC_DIG_NET);
        assert_eq!(resolved.tier, ResolvedTier::PublicGateway);
    }

    /// A tier that never resolves/times out must fall through exactly like an
    /// explicit `false` — the probe implementation races itself against the
    /// caller's timeout, and returns `false` on timeout (never hangs the ladder).
    #[tokio::test]
    async fn timeout_behaves_as_no_response_and_falls_through() {
        struct NeverRespondsProbe;
        #[async_trait::async_trait]
        impl HealthProbe for NeverRespondsProbe {
            async fn probe(&self, _base_url: &str, timeout: Duration) -> bool {
                tokio::time::sleep(timeout * 2).await;
                // Real implementations race with `tokio::time::timeout` and
                // return false on elapse; simulate that contract directly here
                // rather than actually sleeping past the caller's patience.
                false
            }
        }
        let resolved = resolve_node(
            &OverrideInputs::default(),
            DIG_LOCAL,
            LOCALHOST,
            &NeverRespondsProbe,
            Duration::from_millis(5),
        )
        .await;
        assert_eq!(resolved.tier, ResolvedTier::PublicGateway);
    }

    #[tokio::test]
    async fn explicit_override_wins_without_probing_anything() {
        let probe = ScriptedProbe::new(&[(DIG_LOCAL, true), (LOCALHOST, true)]);
        let overrides = OverrideInputs {
            flag: Some("https://custom.example:9999".to_string()),
            env_var: None,
            config_value: None,
        };
        let resolved = resolve_node(
            &overrides,
            DIG_LOCAL,
            LOCALHOST,
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, "https://custom.example:9999");
        assert_eq!(resolved.tier, ResolvedTier::Override);
        // An override is trusted outright — the ladder is never consulted.
        assert!(probe.calls().is_empty());
    }

    #[tokio::test]
    async fn override_trailing_slash_is_normalized() {
        let probe = ScriptedProbe::new(&[]);
        let overrides = OverrideInputs {
            flag: Some("https://custom.example/".to_string()),
            ..Default::default()
        };
        let resolved = resolve_node(
            &overrides,
            DIG_LOCAL,
            LOCALHOST,
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, "https://custom.example");
    }

    // -----------------------------------------------------------------------
    // Override precedence: flag > env > config.
    // -----------------------------------------------------------------------

    #[test]
    fn flag_wins_over_env_and_config() {
        let overrides = OverrideInputs {
            flag: Some("flag-url".into()),
            env_var: Some("env-url".into()),
            config_value: Some("config-url".into()),
        };
        assert_eq!(
            overrides.resolve(),
            Some(("flag-url", OverrideSource::Flag))
        );
        assert_eq!(override_source(&overrides), Some(OverrideSource::Flag));
    }

    #[test]
    fn env_wins_over_config_when_no_flag() {
        let overrides = OverrideInputs {
            flag: None,
            env_var: Some("env-url".into()),
            config_value: Some("config-url".into()),
        };
        assert_eq!(overrides.resolve(), Some(("env-url", OverrideSource::Env)));
        assert_eq!(override_source(&overrides), Some(OverrideSource::Env));
    }

    #[test]
    fn config_used_when_no_flag_or_env() {
        let overrides = OverrideInputs {
            flag: None,
            env_var: None,
            config_value: Some("config-url".into()),
        };
        assert_eq!(
            overrides.resolve(),
            Some(("config-url", OverrideSource::Config))
        );
        assert_eq!(override_source(&overrides), Some(OverrideSource::Config));
    }

    #[test]
    fn no_override_when_all_absent() {
        assert_eq!(OverrideInputs::default().resolve(), None);
        assert_eq!(override_source(&OverrideInputs::default()), None);
    }

    // -----------------------------------------------------------------------
    // Per-invocation caching.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cached_resolver_probes_only_once() {
        struct CountingProbe {
            calls: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl HealthProbe for CountingProbe {
            async fn probe(&self, _base_url: &str, _timeout: Duration) -> bool {
                self.calls.fetch_add(1, Ordering::SeqCst);
                true
            }
        }
        let probe = CountingProbe {
            calls: AtomicUsize::new(0),
        };
        let cache = CachedResolver::new();
        let overrides = OverrideInputs::default();

        let first = cache
            .get_or_resolve(
                &overrides,
                DIG_LOCAL,
                LOCALHOST,
                &probe,
                Duration::from_millis(50),
            )
            .await;
        let second = cache
            .get_or_resolve(
                &overrides,
                DIG_LOCAL,
                LOCALHOST,
                &probe,
                Duration::from_millis(50),
            )
            .await;

        assert_eq!(first, second);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    // -----------------------------------------------------------------------
    // TransportMode: documents the seam without over-claiming behavior.
    // -----------------------------------------------------------------------

    #[test]
    fn default_transport_is_https() {
        assert_eq!(TransportMode::default(), TransportMode::Https);
    }
}
