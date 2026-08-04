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
    /// Base URL, e.g. `http://localhost:9778` or `https://rpc.dig.net`.
    pub base_url: String,
    pub tier: ResolvedTier,
}

impl ResolvedNode {
    /// True when a node on THIS machine answered — i.e. the ladder did not have
    /// to fall through to the public gateway.
    ///
    /// An [`Override`](ResolvedTier::Override) counts as local for this purpose:
    /// the user named that endpoint deliberately, so it is their chosen node
    /// whatever it points at. This is the predicate a caller uses to decide
    /// between "read remotely, but say so" and "refuse, because this operation
    /// needs a node the user actually controls" (see the `digstore` CLI's
    /// `NoLocalNode` error).
    pub fn is_local(&self) -> bool {
        !matches!(self.tier, ResolvedTier::PublicGateway)
    }
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
    /// The PROJECT-scoped `node.url` (`digstore config node.url --local`),
    /// persisted in the nearest ancestor `.dig/node.toml`. Beats the global
    /// value because it is the more specific scope, the same way a repository
    /// `.git/config` beats `~/.gitconfig`.
    Project,
    /// Persisted machine-wide `digstore config node.url <url>`.
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
/// (a CLI flag, `std::env::var`, or a persisted config file) so this module
/// stays free of I/O and is trivially unit-testable.
///
/// Precedence, highest first: `flag` > `env_var` > `project_value` >
/// `config_value` — narrowest scope wins, and anything the user typed on THIS
/// invocation beats anything persisted.
///
/// `project_value` is the caller's responsibility to gate: it originates from a
/// file that can travel inside a cloned repository, so the CLI only populates
/// it once that specific directory+URL pair has been trusted (see the CLI's
/// `ops::node`). This struct trusts whatever it is handed.
#[derive(Debug, Clone, Default)]
pub struct OverrideInputs {
    pub flag: Option<String>,
    pub env_var: Option<String>,
    pub project_value: Option<String>,
    pub config_value: Option<String>,
}

impl OverrideInputs {
    /// The highest-precedence override present, with its source tag.
    fn resolve(&self) -> Option<(&str, OverrideSource)> {
        let ordered = [
            (self.flag.as_deref(), OverrideSource::Flag),
            (self.env_var.as_deref(), OverrideSource::Env),
            (self.project_value.as_deref(), OverrideSource::Project),
            (self.config_value.as_deref(), OverrideSource::Config),
        ];
        ordered
            .into_iter()
            .find_map(|(value, source)| value.map(|v| (v, source)))
    }
}

/// One rung of the local ladder: a fully-formed base URL plus the tier it
/// represents. A single tier may need SEVERAL candidates — `dig.local` is
/// served on both `https://dig.local` (`127.0.0.2:443`, only once the installer
/// has provisioned a dig-cert leaf) and `http://dig.local` (`127.0.0.2:80`), so
/// the ladder must try both before concluding `dig.local` is absent
/// (`dig-node/SPEC.md` §4.1/§4.1a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LadderCandidate {
    /// Fully-formed base URL, scheme included, e.g. `http://localhost:9778`.
    pub url: String,
    /// The tier to report if THIS candidate is the one that answers.
    pub tier: ResolvedTier,
}

impl LadderCandidate {
    /// A candidate at `tier` reachable at `url` (trailing slash normalized away).
    pub fn new(url: impl Into<String>, tier: ResolvedTier) -> Self {
        Self {
            url: url.into().trim_end_matches('/').to_string(),
            tier,
        }
    }
}

/// Resolve the node endpoint per `CLAUDE.md` §5.3 / `dig-node/SPEC.md` §2.2:
/// an explicit override wins outright; otherwise each candidate in
/// `local_candidates` is probed IN ORDER and the first to answer wins;
/// `rpc.dig.net` is the terminal fallback when none does.
///
/// Candidates are supplied by the caller rather than built here so this
/// function stays transport-agnostic: the caller owns the scheme/host/port
/// knowledge (which differs per tier — see [`LadderCandidate`]) and can vary it
/// via `DIG_NODE_PORT`.
///
/// Panics-free; never fails — the public gateway is always a valid last resort.
/// Callers that must NOT silently use the public gateway branch on
/// [`ResolvedNode::is_local`].
pub async fn resolve_node(
    overrides: &OverrideInputs,
    local_candidates: &[LadderCandidate],
    probe: &dyn HealthProbe,
    timeout: Duration,
) -> ResolvedNode {
    if let Some((url, _source)) = overrides.resolve() {
        return ResolvedNode {
            base_url: url.trim_end_matches('/').to_string(),
            tier: ResolvedTier::Override,
        };
    }

    for candidate in local_candidates {
        if probe.probe(&candidate.url, timeout).await {
            return ResolvedNode {
                base_url: candidate.url.clone(),
                tier: candidate.tier,
            };
        }
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
        local_candidates: &[LadderCandidate],
        probe: &dyn HealthProbe,
        timeout: Duration,
    ) -> ResolvedNode {
        self.cached
            .get_or_init(|| resolve_node(overrides, local_candidates, probe, timeout))
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

    // The REAL candidate set a `dig-node` install presents (`dig-node/SPEC.md`
    // §4.1/§4.1a): `dig.local` is portless on :443/:80, and the localhost
    // listener is PLAINTEXT on 9778. Using the real shapes here (rather than
    // two abstract URLs) is deliberate — a fixture built from invented
    // scheme/port pairs is exactly what let the shipped ladder probe
    // `https://dig.local:9778`, where nothing has ever listened, for two
    // releases without a test noticing.
    const DIG_LOCAL_HTTPS: &str = "https://dig.local";
    const DIG_LOCAL_HTTP: &str = "http://dig.local";
    const LOCALHOST: &str = "http://localhost:9778";

    /// The three-rung candidate list under test, in ladder order.
    fn candidates() -> Vec<LadderCandidate> {
        vec![
            LadderCandidate::new(DIG_LOCAL_HTTPS, ResolvedTier::DigLocal),
            LadderCandidate::new(DIG_LOCAL_HTTP, ResolvedTier::DigLocal),
            LadderCandidate::new(LOCALHOST, ResolvedTier::Localhost),
        ]
    }

    /// Only `dig.local`-over-TLS answers. Every LOWER rung is scripted to answer
    /// too, so an implementation that probed in the wrong order — or probed
    /// everything and picked by rank — would return a DIFFERENT url here and
    /// fail. A fixture where only the winner answers cannot see that.
    #[tokio::test]
    async fn prefers_dig_local_when_it_answers() {
        let probe = ScriptedProbe::new(&[
            (DIG_LOCAL_HTTPS, true),
            (DIG_LOCAL_HTTP, true),
            (LOCALHOST, true),
        ]);
        let resolved = resolve_node(
            &OverrideInputs::default(),
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, DIG_LOCAL_HTTPS);
        assert_eq!(resolved.tier, ResolvedTier::DigLocal);
        // Nothing below the first responder may be probed — first responder
        // wins, not "probe everything and rank".
        assert_eq!(probe.calls(), vec![DIG_LOCAL_HTTPS.to_string()]);
    }

    /// The machine has no dig-cert leaf yet, so `:443` is dead but `:80` serves
    /// (`SPEC.md` §4.1a fail-soft). The ladder must still report `DigLocal` —
    /// and must NOT skip to localhost. `localhost` is scripted live so a ladder
    /// that dropped the http rung would visibly resolve to the wrong tier.
    #[tokio::test]
    async fn falls_through_to_plaintext_dig_local_when_tls_is_absent() {
        let probe = ScriptedProbe::new(&[
            (DIG_LOCAL_HTTPS, false),
            (DIG_LOCAL_HTTP, true),
            (LOCALHOST, true),
        ]);
        let resolved = resolve_node(
            &OverrideInputs::default(),
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, DIG_LOCAL_HTTP);
        assert_eq!(resolved.tier, ResolvedTier::DigLocal);
        assert_eq!(
            probe.calls(),
            vec![DIG_LOCAL_HTTPS.to_string(), DIG_LOCAL_HTTP.to_string()]
        );
    }

    #[tokio::test]
    async fn falls_through_to_localhost_when_dig_local_is_unreachable() {
        let probe = ScriptedProbe::new(&[
            (DIG_LOCAL_HTTPS, false),
            (DIG_LOCAL_HTTP, false),
            (LOCALHOST, true),
        ]);
        let resolved = resolve_node(
            &OverrideInputs::default(),
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, LOCALHOST);
        assert_eq!(resolved.tier, ResolvedTier::Localhost);
        assert!(resolved.is_local());
        assert_eq!(
            probe.calls(),
            vec![
                DIG_LOCAL_HTTPS.to_string(),
                DIG_LOCAL_HTTP.to_string(),
                LOCALHOST.to_string()
            ]
        );
    }

    #[tokio::test]
    async fn falls_through_to_public_gateway_as_final_fallback() {
        let probe = ScriptedProbe::new(&[]); // nothing answers
        let resolved = resolve_node(
            &OverrideInputs::default(),
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, RPC_DIG_NET);
        assert_eq!(resolved.tier, ResolvedTier::PublicGateway);
        // The distinguishing assertion: the gateway is NOT a local node, which
        // is what gates the local-node-required operations.
        assert!(!resolved.is_local());
        // Every rung must have been tried before giving up.
        assert_eq!(probe.calls().len(), 3);
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
            &candidates(),
            &NeverRespondsProbe,
            Duration::from_millis(5),
        )
        .await;
        assert_eq!(resolved.tier, ResolvedTier::PublicGateway);
    }

    /// A timing-out rung must not ABORT the ladder: a live rung BELOW it still
    /// wins. The earlier fixture (nothing answers) passes whether the loop
    /// continues or returns early on the first timeout, so it cannot see this.
    #[tokio::test]
    async fn a_timing_out_rung_does_not_abort_the_rungs_below_it() {
        struct SlowFirstRungProbe;
        #[async_trait::async_trait]
        impl HealthProbe for SlowFirstRungProbe {
            async fn probe(&self, base_url: &str, timeout: Duration) -> bool {
                if base_url.contains("dig.local") {
                    tokio::time::sleep(timeout * 2).await;
                    return false;
                }
                true
            }
        }
        let resolved = resolve_node(
            &OverrideInputs::default(),
            &candidates(),
            &SlowFirstRungProbe,
            Duration::from_millis(5),
        )
        .await;
        assert_eq!(resolved.base_url, LOCALHOST);
        assert_eq!(resolved.tier, ResolvedTier::Localhost);
    }

    #[tokio::test]
    async fn explicit_override_wins_without_probing_anything() {
        let probe = ScriptedProbe::new(&[
            (DIG_LOCAL_HTTPS, true),
            (DIG_LOCAL_HTTP, true),
            (LOCALHOST, true),
        ]);
        let overrides = OverrideInputs {
            flag: Some("https://custom.example:9999".to_string()),
            ..Default::default()
        };
        let resolved = resolve_node(
            &overrides,
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, "https://custom.example:9999");
        assert_eq!(resolved.tier, ResolvedTier::Override);
        // An override is trusted outright — the ladder is never consulted.
        assert!(probe.calls().is_empty());
        // A deliberately-named endpoint counts as the user's own node, so it
        // never trips the "no local node" refusal.
        assert!(resolved.is_local());
    }

    /// An override naming the public gateway is the user's explicit choice and
    /// must be honoured as such — reported as `Override`, not demoted to
    /// `PublicGateway`, so `digstore push --node https://rpc.dig.net` works
    /// while a silent fall-through to the same host still refuses.
    #[tokio::test]
    async fn override_naming_the_public_gateway_is_still_an_override() {
        let probe = ScriptedProbe::new(&[]);
        let overrides = OverrideInputs {
            flag: Some(RPC_DIG_NET.to_string()),
            ..Default::default()
        };
        let resolved = resolve_node(
            &overrides,
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, RPC_DIG_NET);
        assert_eq!(resolved.tier, ResolvedTier::Override);
        assert!(resolved.is_local());
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
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, "https://custom.example");
    }

    #[test]
    fn candidate_normalizes_a_trailing_slash() {
        let c = LadderCandidate::new("http://localhost:9778/", ResolvedTier::Localhost);
        assert_eq!(c.url, "http://localhost:9778");
    }

    // -----------------------------------------------------------------------
    // Override precedence: flag > env > config.
    // -----------------------------------------------------------------------

    /// Every tier populated with a DISTINCT value, so the winner identifies
    /// itself unambiguously. A fixture that left the lower tiers empty could
    /// not tell "flag wins" from "only flag was read".
    fn all_tiers() -> OverrideInputs {
        OverrideInputs {
            flag: Some("flag-url".into()),
            env_var: Some("env-url".into()),
            project_value: Some("project-url".into()),
            config_value: Some("config-url".into()),
        }
    }

    #[test]
    fn flag_wins_over_every_other_source() {
        let overrides = all_tiers();
        assert_eq!(
            overrides.resolve(),
            Some(("flag-url", OverrideSource::Flag))
        );
        assert_eq!(override_source(&overrides), Some(OverrideSource::Flag));
    }

    #[test]
    fn env_wins_over_project_and_config_when_no_flag() {
        let overrides = OverrideInputs {
            flag: None,
            ..all_tiers()
        };
        assert_eq!(overrides.resolve(), Some(("env-url", OverrideSource::Env)));
        assert_eq!(override_source(&overrides), Some(OverrideSource::Env));
    }

    /// The per-directory value beats the machine-wide one: a project that pins
    /// its own node must not be overridden by a global default.
    #[test]
    fn project_wins_over_global_config_when_no_flag_or_env() {
        let overrides = OverrideInputs {
            flag: None,
            env_var: None,
            ..all_tiers()
        };
        assert_eq!(
            overrides.resolve(),
            Some(("project-url", OverrideSource::Project))
        );
        assert_eq!(override_source(&overrides), Some(OverrideSource::Project));
    }

    #[test]
    fn global_config_used_when_it_is_the_only_source() {
        let overrides = OverrideInputs {
            config_value: Some("config-url".into()),
            ..Default::default()
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

    /// A project value alone must beat the LADDER, not merely the global config
    /// — §5.3's "a configured value overrides the ladder entirely". The ladder
    /// is scripted fully live so a bug that consulted it anyway would resolve
    /// somewhere else and fail here.
    #[tokio::test]
    async fn project_value_alone_beats_a_fully_live_ladder() {
        let probe = ScriptedProbe::new(&[
            (DIG_LOCAL_HTTPS, true),
            (DIG_LOCAL_HTTP, true),
            (LOCALHOST, true),
        ]);
        let overrides = OverrideInputs {
            project_value: Some("https://project-node.example".into()),
            ..Default::default()
        };
        let resolved = resolve_node(
            &overrides,
            &candidates(),
            &probe,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(resolved.base_url, "https://project-node.example");
        assert_eq!(resolved.tier, ResolvedTier::Override);
        assert!(probe.calls().is_empty());
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

        let cands = candidates();
        let first = cache
            .get_or_resolve(&overrides, &cands, &probe, Duration::from_millis(50))
            .await;
        let second = cache
            .get_or_resolve(&overrides, &cands, &probe, Duration::from_millis(50))
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
