# digstore-remote — normative specification

`digstore-remote` implements the digstore HTTPS remote protocol (paper §21): the `DigClient`
(clone/fetch/pull/push/tombstone/delta/content-read) and the `RemoteServer` (`digstore serve`'s
axum router). This document is the authoritative, normative contract an independent
reimplementation of either side MUST be buildable against. Keywords **MUST**, **MUST NOT**,
**SHALL**, **SHOULD**, **MAY** are used as in RFC 2119. Behaviour that contradicts this document is
a bug in the code OR in this document — they are kept in agreement in the same unit of work as any
change (`CLAUDE.md` §4.2).

**Cross-references (this SPEC MUST NOT contradict them):**

- Ecosystem cross-repo contract map: superproject `CLAUDE.md` §5.3 (the client→node resolution
  order, normative across every DIG client, not just this crate) and `SYSTEM.md`.
- `dig-node/SPEC.md` §2.2 "Client → node resolution order" — the same ladder, specified from the
  node's perspective (dig-node is itself a valid resolution target at tiers 2–4, and its own
  upstream-on-miss fallback is a *server-side* use of tier 4).
- L7 peer-network protocol: `docs.dig.net/docs/protocol/peer-network.md` (mTLS peer identity, the
  dual-transport RPC tiers, the `peer_id = SHA-256(TLS SPKI DER)` identity model this crate's
  deferred mTLS transport will reuse).
- The DIG store format + read crypto (`digstore-core`/`-host`/`-crypto`/`-chain`/`-stage`), which
  this crate consumes and does not reimplement.

This document currently covers the **client→node connection-order + transport contract**
(`src/resolver.rs`) in full; the wire-level `§21` protocol (routes, status codes, auth headers) is
documented inline in `src/server.rs`/`src/client.rs`/`src/wire.rs` doc comments and is summarized
here only where the resolver depends on it (the `/health` route).

---

## 1. Client → node connection-order ladder (`CLAUDE.md` §5.3)

### 1.1 Requirement

Any digstore client that needs to reach **a** DIG node — as opposed to a specific, already-known
store remote configured via `digstore remote add` — MUST resolve the endpoint in this fixed order,
using the FIRST tier that answers a cheap health probe within a short timeout:

1. **An explicitly-configured node** — always wins, overriding the ladder entirely. Precedence
   among override sources, highest first:
   1. an explicit `--node <url>` CLI flag (or equivalent constructor argument for a non-CLI
      client);
   2. `$DIG_NODE_URL`;
   3. a persisted `digstore config node.url <url>` value.
2. **`dig.local`** — the installed local node (the DIG installer's hosts-file registration).
3. **`localhost`** — a node on the loopback default port (`DIG_NODE_PORT`, default **9778**,
   matching `dig-node/SPEC.md` §1.1), tried when `dig.local` does not resolve/respond.
4. **`rpc.dig.net`** — the public gateway. FINAL fallback only.

A conforming client MUST NOT hard-code `rpc.dig.net` as the primary or only endpoint for a
generic node connection. `rpc.dig.net` MUST always be reachable as tier 4 (the "never fails"
guarantee below) but MUST NOT be consulted before tiers 1–3 have been tried.

This ladder governs connections where the client has NOT been told a specific host (e.g. a
`urn:dig:…` network content-read with no `remote add`-ed `origin`). A remote the user has
EXPLICITLY configured for a specific store (`digstore remote add origin https://my-host`) is
itself a form of explicit configuration and takes precedence over the ladder for operations
scoped to that remote — the ladder exists for the "any node will do" case, not to override a
user's per-store remote choice.

### 1.2 Probing

- Each non-override tier is probed with a cheap, short-timeout health check:
  `GET {base_url}/health`, expecting any 2xx response.
- The probe MUST NOT block past the caller-supplied timeout (`resolve_node`'s `timeout` parameter;
  the CLI's default is `DEFAULT_PROBE_TIMEOUT` = 600ms). A timeout is treated identically to a
  transport error or a non-2xx response: "this tier did not respond," and the ladder falls through
  to the next tier.
- The FIRST tier that responds wins — the ladder does not probe every tier and rank them; it stops
  at the first success (`ResolvedTier::DigLocal`/`Localhost`), or falls all the way through to
  `PublicGateway` if nothing else answered.
- `rpc.dig.net` (tier 4) is returned as the resolved endpoint even when it is not itself probed
  successfully — there is nowhere left to fall through to. `resolve_node` therefore **never
  fails**; it always returns a `ResolvedNode`.

### 1.3 Caching

- The resolved choice MUST be cached for the invocation/session — a single command run that needs
  the node endpoint more than once resolves it ONCE. `CachedResolver` provides this via
  `tokio::sync::OnceCell`; a CLI/binary MAY instead resolve once and thread the `ResolvedNode`
  through, which is equivalent.
- The cache is scoped to one process invocation. A long-lived client (a daemon, a browser session)
  MAY re-resolve periodically or on a detected failure of the cached endpoint; this crate does not
  mandate a specific re-resolution policy beyond "do not re-probe on every single request."

### 1.4 The `/health` route

`RemoteServer::router()` serves `GET /health` unauthenticated (outside `/stores/:id`, so it is not
subject to the §21.9 per-request auth middleware even when the server otherwise requires auth on
every route) returning `200 {"status":"ok"}` with no backend work — no store lookup, no chain call.
This mirrors `dig-node`'s own `GET /health` (`dig-node/SPEC.md` §1.1), so a resolver probe speaks
one contract regardless of which of the two server implementations answers it.

A conforming node-class server (`digstore serve`, `dig-node`, and eventually the `rpc.dig.net`
gateway) MUST serve `GET /health` unauthenticated and MUST NOT gate it behind §21.9 signed-request
auth or mTLS — a probe cannot know in advance whether the target is even the right kind of server,
let alone hold a valid signed request for it.

### 1.5 Public API (`src/resolver.rs`)

| Item | Role |
|---|---|
| `OverrideInputs { flag, env_var, config_value }` | The three override sources, pre-extracted by the caller (keeps this module I/O-free and unit-testable). `override_source()` reports which one (if any) would win, without running the async ladder. |
| `HealthProbe` (trait, `async fn probe(&self, base_url: &str, timeout: Duration) -> bool`) | Pluggable reachability check. `HttpHealthProbe` is the production `GET /health` implementation; tests inject a scripted fake to verify fall-through order deterministically, with no network. |
| `resolve_node(overrides, dig_local_url, localhost_url, probe, timeout) -> ResolvedNode` | The ladder itself. Async, panics-free, never fails. |
| `ResolvedNode { base_url, tier }` / `ResolvedTier` | The resolved endpoint + which tier answered (`Override`/`DigLocal`/`Localhost`/`PublicGateway`) — surfaced for diagnostics (e.g. `digstore doctor`, `--verbose`). |
| `CachedResolver` | Per-invocation memoization wrapper around `resolve_node`. |
| `RPC_DIG_NET`, `DIG_LOCAL_HOST`, `DEFAULT_LOCAL_NODE_PORT`, `DEFAULT_PROBE_TIMEOUT` | The ladder's constants. |

`dig_local_url`/`localhost_url` are supplied by the caller (already-formed base URLs, e.g.
`https://dig.local:9778`) rather than hardcoded inside `resolve_node`, so the function stays
transport-agnostic and callers can honor a non-default `DIG_NODE_PORT`.

---

## 2. Transport

### 2.1 Current state: plain HTTPS

Every tier speaks plain HTTPS today (`http://` permitted only to a loopback host — `is_loopback_http`
in the CLI's `remote_ops` module — for local dev/test). `DigClient` layers the paper §21.9
signed-request headers (`X-Dig-Identity`, `X-Dig-Timestamp`, `X-Dig-Nonce`, `X-Dig-Auth`) over this
plain-HTTPS channel when constructed `.with_identity(...)`.

### 2.2 Deferred: mTLS transport

`CLAUDE.md` §5.3 requires a node-class client (one holding a DIG identity key — this crate's
consumers: the digstore CLI, an SDK, any filesystem client) to connect over **mTLS** at every tier,
presenting a client certificate derived from its DIG identity key using the same
`peer_id = SHA-256(TLS SPKI DER)` model as the peer-network layer (`dig-nat`/`dig-gossip`), with
§21.9 signed-request authorization layered on top of the mTLS channel. `rpc.dig.net` is specified as
**dual-mode**: an mTLS endpoint for node-class clients, plus the plain-HTTPS+CORS public read tier
browsers require (a browser cannot present a client certificate).

**This is NOT YET WIRED.** As of this document, no tier — including `rpc.dig.net` — serves an mTLS
endpoint, so there is nothing for a node-class client to dial over mTLS yet. Implementing the full
mTLS client path now, with no real server to validate against, would be speculative and untestable.
Per `CLAUDE.md` §5.3's own gating instruction ("implementations MUST NOT hard-break before the
gateway's mTLS endpoint exists — gate on it"), this crate:

- implements the ladder + plain-HTTPS transport fully (§1 above), which is testable today and does
  not regress when mTLS lands;
- reserves `TransportMode` (`Https` | `Mtls`) as the seam: an explicit enum (not a bool) so adding
  the real mTLS wiring is an additive change to `DigClient`'s constructors (e.g. a future
  `DigClient::with_mtls_identity(...)`), not a breaking one. `TransportMode::Https` is `Default`.
  `TransportMode::Mtls` exists today only so callers/tests can express intent; nothing in this
  crate currently activates real mTLS behavior for it.

**When the gateway's mTLS endpoint exists**, this section MUST be updated (in the same unit of work
as the implementation) to specify: how the client derives/loads its identity-key-backed certificate,
how `DigClient` is constructed with an mTLS transport, and how the resolver's probe adapts (an mTLS
health probe cannot be a bare unauthenticated `GET`, since the TLS handshake itself requires a
client cert — the probe contract in §1.4 will need a matching mTLS variant). Until then, treating
every tier as plain HTTPS is the conformant, documented behavior of this crate — not a shortcut to
be silently carried forward once the gateway exists.

### 2.3 Test coverage of the deferred seam

`resolver.rs`'s `default_transport_is_https` test pins `TransportMode::default() ==
TransportMode::Https`, so a future change that flips the default without updating this document (and
the CLI's transport-selection code) fails loudly instead of silently changing behavior.

---

## 3. Conformance notes

- A reimplementation of the resolver MUST reproduce the exact precedence order in §1.1 and the
  exact fall-through behavior in §1.2 (including: an override is trusted outright with NO probing
  of any kind, and the public gateway is returned even when unprobed/unreachable).
- A reimplementation MUST NOT probe tiers in parallel and pick the "best" one by some other metric
  (latency, etc.) — the contract is strictly first-tier-that-responds among 2→3→4, checked in
  order, not a race.
- `dig-node/SPEC.md` §2.2 describes the SAME ladder from the perspective of a node that is itself a
  valid resolution target; the two documents MUST stay in agreement (a change to one that shifts
  the ladder's behavior updates the other in the same unit of work, per `CLAUDE.md` §4.2 "Layering").
