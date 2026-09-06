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

This document covers the **client→node connection-order + transport contract** (`src/resolver.rs`,
§1–§2) and the **whole-module read's root pinning** (`GET|HEAD /stores/{id}/module`, §4) in full;
the rest of the wire-level `§21` protocol (routes, status codes, auth headers) is documented inline
in `src/server.rs`/`src/client.rs`/`src/wire.rs` doc comments and is summarized here only where §1
or §4 depends on it (the `/health` route, the `module` method tag).

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
- A whole-module read (`GET|HEAD /stores/{id}/module`) MUST follow §4: a client that holds the head
  pins it with `?root=`, refuses a served root other than the one it asked for BEFORE invoking its
  verifier, and never follows a redirect; a `digstore serve` remote honours `?root=` (`422`
  malformed → `404` not-served → `200` with `ETag` = the served root, in that precedence) and serves
  its head on a rootless read with no redirect.

---

## 4. Whole-module read — root pinning (§21.3 clone, §21.4 pull)

`GET|HEAD /stores/{id}/module` is the whole-module read: it returns the compiled store module (the
`.dig` WASM bytes) for ONE generation of store `{id}`, identified by that generation's 32-byte merkle
root. This section fixes how a client names the generation it wants (the **root pin**), how the two
server implementations answer, and how the client fails when the answer is not the generation it
asked for. Every failure in this section fails CLOSED: the client never returns bytes labelled with a
root it did not ask for, and a server never substitutes its head for a root a client named.

Clauses marked **[implemented]** are true of the code at the cited `file:line`; clauses marked
**[pending #1903]** are the contract the #1903 implementation MUST satisfy, and the marker is replaced
by a citation in the same PR that lands the code (§4.9).

### 4.1 Terms and wire rendering

- **Generation root** — the 32-byte merkle root of one generation of a store. The store's **served
  head** is the root the remote currently serves as its confirmed head (`HeadState::served_root`).
- **`root = None` means the remote's current served head** — for `clone_store`, for
  `clone_store_at(.., None, ..)`, and for a rootless server-side `GET|HEAD /module`. It is a request
  for "whatever you serve now"; the client learns WHICH root that was only from the response `ETag`.
- **Root on the wire** — in a query string a root is `?root=<64 hex characters>` (32 bytes,
  hex-encoded, no quotes). A client MUST emit lowercase hex (`Bytes32::to_hex`). A server MUST accept
  lowercase; whether it accepts uppercase is implementation-defined and a client MUST NOT rely on it.
- **`ETag`** — the module's ETag is its generation root rendered as a strong quoted tag `"<64-hex>"`
  (`etag_for_root`, `src/etag.rs:5-7`) and is parsed by `parse_if_none_match` (`src/etag.rs:12-16`),
  which yields `None` for `*`, weak (`W/`) tags, and any value that is not a quoted 64-hex string.
  **[implemented]**

### 4.2 Client — `DigClient::clone_store_at` and `clone_store`

```rust
pub async fn clone_store_at<V>(
    &self,
    store_id: &Bytes32,
    root: Option<&Bytes32>,
    verify: V,
    on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> Result<(Bytes32, Vec<u8>), ClientError>
where
    V: FnOnce(&[u8], &Bytes32) -> Result<(), String>;
```

1. **Additive surface.** `clone_store_at` is a new public method. `clone_store(store_id, verify,
   on_progress)` keeps its exact signature (`src/client.rs:316-323`) and IS
   `clone_store_at(store_id, None, verify, on_progress)`. This is a SemVer MINOR change to
   `digstore-remote`: no existing caller changes. **[pending #1903]**
2. **Request formation.** `root = Some(r)` → `GET {base}/stores/{id}/module?root=<r as lowercase
   64-hex>`. `root = None` → `GET {base}/stores/{id}/module` with no query — byte-identical to today's
   request (`src/client.rs:325-331`). Both carry the §21.9 auth headers with method tag `module` when
   the client has an identity (§4.6). **[pending #1903]**
3. **Redirects are never followed.** The HTTP client is built with `redirect::Policy::none()`
   (`src/client.rs:196`) and this MUST NOT change: a redirect is a protocol error, not a hop (the
   push-bounce / SSRF guard). Consequently ANY non-2xx status — 3xx included — is returned as
   `ClientError::Status(code)` (`src/client.rs:335-337`), and a rootless read against a server that
   resolves the head by redirect (the `rpc.dig.net` gateway, §4.5) fails with
   `ClientError::Status(307)`. A caller that holds the head — every caller that has run `fetch` —
   MUST pass it as `Some(&head)`. **[implemented]** (redirect policy + status mapping; the 307
   consequence is a direct reading of them).
4. **The ETag is the served root.** A 2xx response MUST carry an `ETag` that parses (§4.1) to a root;
   missing or unparsable → `ClientError::Verification` (`src/client.rs:338-346`). **[implemented]**
5. **The pin.** When `root = Some(r)`, the parsed ETag root MUST equal `r`; otherwise the client
   returns `ClientError::Verification`. This check is made on the response HEADERS, before the body is
   read: on a mismatch the body is not downloaded, `on_progress` is not invoked, and `verify` is NOT
   invoked. The caller's verifier is the LAST check, not the only one — a server that answers a request
   for `r` with a different generation is refused before that generation's bytes reach caller code.
   **[pending #1903]**
6. **The verifier.** After the pin check the body is downloaded and `verify(&bytes, &root)` is
   invoked exactly once, with the root that will be returned — the served root, which equals `r` when
   pinned. `Err(msg)` → `ClientError::Verification(msg)` (`src/client.rs:348-349`). **[implemented]**
7. **What success proves — and does not.** `Ok((root, bytes))` proves only that the remote served
   `bytes` labelled `root`, that `root` is the one the caller asked for (when pinned), and that the
   caller's `verify` accepted `bytes` against `root`. It does NOT prove that `root` is the store's
   current head, that `root` is a confirmed on-chain generation, or that the publisher authorised it —
   a caller that needs those facts establishes them from `fetch` (descriptor `current_root`,
   `push_sig`, §21.6), as the CLI does (§4.7).

Failure directions, pinned: a wrong-generation answer is `Verification` (never returned as data); an
absent generation is `Status(404)`; a redirecting server is `Status(3xx)`; a transport failure is
`Transport`. No path in `clone_store_at` returns bytes for a root the caller did not name.

### 4.3 Client — `pull`'s full-module download (§21.4)

`pull(store_id, local_root, prefer_delta, on_progress)` (`src/client.rs:359-430`) first runs `fetch`
and derives the remote head `remote_root` from the descriptor's `current_root`
(`src/client.rs:368-370`); the delta path is unchanged by this section.

1. The full-module request MUST be `GET /stores/{id}/module?root=<remote_root>` — the head `pull`
   already holds — instead of today's rootless GET (`src/client.rs:403-407`). **[pending #1903]**
2. `If-None-Match: "<local_root>"` is sent when `local_root` is `Some`, and its semantics are unchanged
   (`src/client.rs:408-413`): a `304` → `PullResult::UpToDate` (`src/client.rs:418-420`). Against a
   conforming root-pinning server (§4.4) this branch is VACUOUS: `pull` only reaches the download when
   `local_root != remote_root`, and a server honouring `?root=<remote_root>` serves `remote_root` or
   404s, so its served root can never equal `local_root`. The branch is retained because the client
   MUST still handle a `304` correctly if a server sends one. **[implemented]**
3. On a 2xx the `ETag` root MUST equal `remote_root`; missing, unparsable or mismatching →
   `ClientError::Verification`, and the body is not returned. `PullResult::Module { root, bytes }`
   therefore carries a `root` the served `ETag` agreed with, not merely the descriptor's claim.
   **[pending #1903]**
4. A head advance between `fetch` and the module GET surfaces on a head-only server (§4.4) as
   `ClientError::Status(404)`; the caller re-runs `pull`. This is the intended fail-closed direction:
   `pull` never returns bytes for a generation other than the one it reports.

### 4.4 Server — `RemoteServer` (`digstore serve`) `GET|HEAD /stores/{id}/module`

`RemoteServer::router()` mounts `get_module` and `head_module` at `/stores/:id/module`
(`src/server.rs:85-88`). Today both ignore any query string and serve the head via
`backend.module_bytes(&store_id, None)` (`src/handlers/module.rs:74`). The contract:

1. **Optional `root` query.** GET and HEAD both accept an optional `root` query parameter — the same
   `?root=` the PUT handler already parses for pushes (`src/handlers/module.rs:192, 214-224`; note the
   PUT's `root` is the NEW root being pushed, a different meaning of the same name). **[pending #1903]**
2. **Precedence** — the first matching row wins; a later row is never evaluated:

   | # | condition | response |
   |---|---|---|
   | 1 | `{id}` is not 64-hex (`parse_store_id`, `src/server.rs:255`) | `400` |
   | 2 | `root` present and not a 64-hex string (empty, `latest`, wrong length, non-hex) | `422` (`RemoteError::Validation`, `src/error.rs:42`) — evaluated BEFORE any backend lookup; a malformed root never reaches `head_state` |
   | 3 | store unknown | `404` (`RemoteError::UnknownStore`, `src/error.rs:36`) |
   | 4 | `root` present, well-formed, and not the root the remote serves for this store | `404` (`RemoteError::UnknownRoot`, `src/error.rs:36`), no `ETag` |
   | 5 | GET only: `If-None-Match` parses to the served root (`matches_current`, `src/etag.rs:19-21`) | `304` + `ETag` of the served root |
   | 6 | otherwise | `200`, `Content-Type: application/wasm`, `ETag` = the served root; GET carries the module bytes, HEAD carries `Content-Length` and no body |

   Rows 1, 3, 5, 6 are the existing behaviour (`src/handlers/module.rs:15-38, 40-87`); rows 2 and 4
   are **[pending #1903]**. HEAD does not evaluate `If-None-Match` (row 5 is GET-only) — unchanged.
3. **Rootless serves the head, with no redirect.** `root` absent → the served head with `200`
   (row 6), exactly as today. A `digstore serve` remote MUST NOT answer a rootless read with a redirect:
   the local dev/test flow and every existing rootless client depend on the `200`. **[implemented]**
   (`src/handlers/module.rs:74-86`)
4. **Head-only.** A `digstore serve` remote serves exactly ONE module per store over this route — its
   served head. `RemoteBackend::module_bytes(id, Some(r))` returns `UnknownRoot` for any
   `r != served_root` (trait contract `src/backend.rs:83-89`; `InMemoryBackend`
   `src/backend_inmem.rs:239-250`; `StoreBackend` `src/backend_store.rs:270-277`), INCLUDING a
   historical generation whose bytes the backend still holds (`InMemoryBackend` keeps every generation
   in `generations` and refuses the non-head ones deliberately, `src/backend_inmem.rs:247-250`). A
   rooted read therefore succeeds against `digstore serve` only when `r` is the current head. This is
   the point where this server and the gateway differ (§4.5). **[implemented]**
5. **No pruned/never-existed oracle.** The `404` of row 4 is the same `RemoteError::UnknownRoot`
   response whether `r` is a generation the remote holds but does not serve, a generation it once
   held, or a root that never existed — the variant carries no payload (`src/error.rs:9-10`), so the
   body cannot distinguish them, and an implementation MUST NOT add a distinguishing body. A reader
   MUST NOT infer from a `404` that a root never existed; from `digstore serve` it means only "not the
   served head". **[implemented]** for the shared variant; reached from GET/HEAD once row 4 lands.
6. **A rooted request is never downgraded to the head.** With `?root=r` present, the served root MUST
   be `r` (row 6 is reachable only after row 4 passed) and the `ETag` MUST be the root actually
   served. The pre-#1903 behaviour — ignore `?root=`, serve the head under the head's `ETag` — is the
   fail-OPEN direction this section forbids: a client without §4.2.5's pin check would install a
   generation it did not ask for. **[pending #1903]**
7. **Auth.** The §21.9 method tag for GET and HEAD `/module` is `module`, with or without a query
   (`src/server.rs:161`); the query is not part of the signed message (§4.6). **[implemented]**

### 4.5 The two servers side by side

The `rpc.dig.net` gateway (hub.dig.net `SPEC.md` §16) serves EVERY generation it holds, immutably per
root, and resolves a rootless read by redirect. `digstore serve` serves only its head and never
redirects. One client contract (§4.2/§4.3) works against both because the client always pins when it
can and treats everything but a matching `200` as failure:

| request | `digstore serve` | `rpc.dig.net` gateway | client outcome (`clone_store_at`) |
|---|---|---|---|
| `GET /module` (rootless) | `200`, head, `ETag="<head>"` | `307` → `?root=<confirmed head>`, `no-store`, empty body | serve: `Ok((head, bytes))` · gateway: `Err(Status(307))` — pass the head |
| `GET /module?root=<served head>` | `200`, `ETag="<head>"` | `200`, `ETag="<root>"`, immutable | `Ok((root, bytes))` after pin + `verify` |
| `GET /module?root=<held, non-head generation>` | `404` (`UnknownRoot`) | `200`, `ETag="<root>"` | serve: `Err(Status(404))` · gateway: `Ok((root, bytes))` |
| `GET /module?root=<never existed>` | `404` (`UnknownRoot`) — same body as the row above | `404` | `Err(Status(404))` |
| `GET /module?root=` (empty) or `?root=latest` | `422` | `307` → `?root=<confirmed head>` | `Err(Status(422))` / `Err(Status(307))` — a conforming client never emits these |
| `GET /module?root=<not 64-hex>` | `422` | `400` | `Err(Status(422))` / `Err(Status(400))` |
| unknown store | `404` (`UnknownStore`) | `404` | `Err(Status(404))` |
| `200` whose `ETag` root ≠ the pinned root | does not occur on a conforming server | does not occur on a conforming server | `Err(Verification)`, `verify` and `on_progress` not invoked |
| GET with `If-None-Match` equal to the served root | `304` + `ETag` | per hub `SPEC.md` §16 | `pull`: `UpToDate` |

The two servers answer the same request differently in rows 1, 3, 5 and 6. A conforming client MUST
NOT special-case the server: it pins the root whenever it holds one and maps every status other than
`200` (and, in `pull`, `304`) to `Status(code)`.

### 4.6 What §21.9 does and does not sign

The signed-request headers cover `request_signing_message(method_tag, store_id, timestamp, nonce)`
(`src/client.rs:261-272`) — the QUERY STRING IS NOT SIGNED. Adding `?root=` therefore changes no
signature and no method tag (`module`). A reader MUST NOT conclude that the root pin is
integrity-protected by the request's authentication: the pin is enforced by the CLIENT — the `ETag`
comparison (§4.2.5, §4.3.3) and the caller's `verify` (§4.2.6) — so a `root` altered in flight yields
`Verification`, never a silently different generation. **[implemented]** for the signing scope; the
enforcement clauses carry their own markers.

### 4.7 CLI — `digstore clone`

`clone_from` (`crates/digstore-cli/src/ops/remote_ops.rs:360`) runs `client.fetch` and derives
`remote_root` from `current_root` (`:376-378`) BEFORE downloading. It MUST download with
`clone_store_at(&store_id, Some(&remote_root), ..)`; today it calls the rootless `clone_store`
(`:389-410`). Its verifier (embedded `StoreId == store_id`, recomputed content root == served root,
`:391-402`) and its post-check `etag_root == remote_root` (`:415-419`) are unchanged; with the pin in
place the post-check can no longer fire (the pin already guarantees the equality) and is retained as
defence in depth, not as the mechanism. **[pending #1903]**

`digstore pull` needs no CLI change: the pin lives inside `DigClient::pull` (§4.3).

### 4.8 What this section does not specify

- It does not make `digstore serve` retain or serve historical generations over `/module`; the
  head-only rule (§4.4.4) IS the served contract, and changing it is a `RemoteBackend` trait-contract
  change across both backends, decided separately.
- It does not change the gateway; §4.5 records the gateway's shipped behaviour (hub.dig.net `SPEC.md`
  §16) so the client contract can be checked against it, and MUST be updated in the same unit of work
  as any gateway change.
- It does not specify the delta path (`/delta`), content reads (`/content`), or pushes
  (`PUT /module?root=`, `src/handlers/module.rs:184-189`).

### 4.9 Implementation status

Each pending row is replaced by its `file:line` citation in the PR that lands it; a row left pending
after that PR is a defect in this document.

| clause | status |
|---|---|
| 4.2.1 `clone_store_at` exists; `clone_store` delegates to it | pending #1903 |
| 4.2.2 rooted request carries `?root=<lowercase hex>`; rootless request unchanged | pending #1903 |
| 4.2.3 redirects never followed; non-2xx → `Status(code)` | `src/client.rs:196, 335-337` |
| 4.2.4 `ETag` missing/unparsable → `Verification` | `src/client.rs:338-346` |
| 4.2.5 pin: `ETag` root ≠ requested → `Verification`, before body/`on_progress`/`verify` | pending #1903 |
| 4.2.6 `verify` invoked once with the served root | `src/client.rs:348-349` |
| 4.3.1 `pull` full GET carries `?root=<remote_root>` | pending #1903 |
| 4.3.3 `pull` checks `ETag` root == `remote_root` | pending #1903 |
| 4.4.2 rows 2 and 4 (422 before lookup; 404 for a non-served root) on GET and HEAD | pending #1903 |
| 4.4.3 rootless → `200` head, no redirect | `src/handlers/module.rs:74-86` |
| 4.4.4 head-only `module_bytes` | `src/backend_inmem.rs:239-250`, `src/backend_store.rs:270-277` |
| 4.4.6 rooted request never downgraded to the head | pending #1903 |
| 4.7 CLI passes `Some(&remote_root)` | pending #1903 |
