# dig-node — normative specification

This is the authoritative statement of what the `dig-node` crate implements. It is normative: an
independent reimplementation MUST satisfy the MUST/SHALL clauses here to interoperate. Behaviour that
contradicts this document is a bug in the code OR in this document — they are kept in agreement in the
same unit of work as any change.

`dig-node` is the DIG Browser local node: a loopback JSON-RPC server implementing the `dig.getContent`
contract (the same contract as `rpc.dig.net`), serving `dig://` content from LOCAL `.dig` store
modules first (via `digstore_host::serve_blind`), falling back to `rpc.dig.net` on a miss, caching
synced stores with an LRU size cap. It also runs the L7 DIG Node **peer network** (node↔node
peer-to-peer content location + transfer) in the standalone binary.

---

## 1. Identity and transport (peer network)

- Every node↔node link is **mutual TLS**. A node presents ONE stable certificate; its identity is
  `peer_id = SHA-256(TLS SubjectPublicKeyInfo DER)`. The `peer_id` MUST be derived from the presented
  certificate on every link — it is NEVER taken from a wire body (identity is not self-asserted).
- The standalone binary derives a deterministic mTLS identity from its persistent 32-byte seed
  (`peer::identity_from_seed`), so the node's `peer_id` is stable across restarts.
- The in-process FFI path (the browser) is a pure consumer: it opens NO peer network and NO listener,
  so the byte-exact `dig.getContent` / §21 read contract is unaffected by anything in this section.

## 2. Address-family policy — IPv6-first, IPv4-fallback (HARD RULE)

All peer communication is **IPv6-first, with IPv4 as the fallback**. This applies at three points; the
mechanics live in `crate::net`.

### 2.1 Listener bind (dual-stack)

- The mTLS peer-RPC listener MUST bind the IPv6 unspecified address `[::]:{port}` as a **dual-stack**
  socket: `IPV6_V6ONLY` is explicitly cleared (`set_only_v6(false)`) before `listen`, so the ONE
  socket accepts both native IPv6 connections and IPv4 connections (via IPv4-mapped-IPv6) on the same
  port.
- The listener MUST NOT bind `0.0.0.0` (IPv4-only, drops IPv6), and MUST NOT leave `IPV6_V6ONLY` at its
  OS default (which is `1` on Windows and some Linux distributions, making the socket IPv6-only and
  silently dropping IPv4).
- `SO_REUSEADDR` is set (matching std/tokio bind behaviour) so a restarted node can rebind promptly.
- An explicit IPv4 bind address is left unchanged (dual-stack is meaningless for an IPv4 socket).

### 2.2 Advertised addresses

- A node advertises its **real, directly-dialable** candidate address(es) — in its DHT provider record
  (`crate::peer::bring_up_dht`) and in `dig.getNetworkInfo` — ordered **IPv6-first**: a global-unicast
  IPv6 address (when the host has one) precedes the IPv4 fallback.
- The wildcard bind address (`[::]` or `0.0.0.0`) is NOT dialable and MUST NEVER appear as an
  advertised candidate. `dig.getNetworkInfo.listen_addr` reports the primary (IPv6-preferred) dialable
  candidate, never the wildcard bind target.
- An address is advertisable only if routable: an IPv6 candidate MUST NOT be loopback, unspecified,
  link-local (`fe80::/10`), unique-local (`fc00::/7`), or IPv4-mapped; an IPv4 candidate MUST NOT be
  loopback, unspecified, link-local (`169.254.0.0/16`), or broadcast. (RFC-1918 private IPv4 ranges ARE
  advertisable — a LAN peer is reachable there.)
- A NAT'd node with NO routable local address advertises no direct candidate and relies on the
  relay-coordinated traversal tiers. It MUST NOT substitute a wildcard or a bogus candidate.
- Loopback candidates (`::1` first, then `127.0.0.1`) are advertised ONLY when
  `DIG_NODE_ADVERTISE_LOOPBACK` is truthy (`1`/`true`/`yes`/`on`) — for tests and single-host /
  in-process setups. Off by default.

### 2.3 Dialing (happy-eyeballs, IPv6-preferred)

- When dialing a discovered peer, the node MUST pass that peer's FULL candidate list (every dialable
  candidate the contact advertises) to `dig_nat::PeerTarget::with_addrs`, which orders the list
  IPv6-first. It MUST NOT collapse the peer to a single address before dialing.
- `dig-nat`'s dialer then tries the peer's IPv6 candidate(s) first and falls back to IPv4 only on IPv6
  failure/timeout. A contact with no dialable candidate becomes a `relay_only` target (reached via the
  relay-coordinated tiers).

## 3. Peer-RPC method surface (over the mTLS mux)

Requests are length-prefixed JSON frames over dig-nat logical streams. The node serves:

- `dig.getNetworkInfo` — this node's own posture: `{ peer_id, network_id, listen_addr,
  reflexive_addr, candidate_addresses, reachability, relay }`. `candidate_addresses` is the
  IPv6-first advertised list (§2.2); `listen_addr` is its first (IPv6-preferred) entry.
- `dig.getPeers` — the live connected pool (peer discovery).
- `dig.announce` — a peer announces `{ peer_id (64-hex), addresses (array) }`.
- `dig.getAvailability` — batch answer for queried items against the local inventory (positionally
  aligned with the request `items`).
- `dig.listInventory` — the node's held capsules (store / capsule granularity).
- `dig.fetchRange` — one range frame of a served resource (the caller streams by requesting successive
  ranges); the first frame (offset 0) carries the per-range verification metadata (total length, chunk
  lengths, inclusion proof, root).
- The four Kademlia DHT methods (`find_node`, `find_providers`, `add_provider`, `ping`) are dispatched
  to the content-location DHT, folding in the mTLS-verified caller as a routing-table contact.

`control.peerStatus` (loopback control RPC) reports whether the peer network is running, the node's
`peer_id`, the connected-pool size, and the relay-reservation state; it reports "not running" when no
network is up (always safe to call, including on the FFI path).

## 4. Content-location DHT (Kademlia)

- The node LOCATES which peers hold content via `find_providers`, and keeps its OWN held-inventory
  provider records CURRENT: announce every held capsule on startup (store AND capsule granularity),
  announce/withdraw on inventory change, `republish` before TTL via the maintenance loop, and a
  best-effort `withdraw` sweep on graceful shutdown.
- The DHT rides the SAME dig-nat mTLS transport as the rest of the peer network (§1); there is no
  unauthenticated DHT channel. Each outbound DHT RPC is one dial + one logical stream, bounded by a
  per-RPC timeout; a dial/stream/parse failure or timeout is treated as "that peer is unreachable".

## 5. Content fetch — discovery → selection → download → learning

When the node needs content it does NOT hold (a miss on a `dig.getContent` / `dig.fetchRange` / peer
range-stream / `dig.getAvailability` request), it locates the holders via the DHT and either REDIRECTS
the caller to them (default) or FETCHES-THROUGH — pulling the resource over the peer network,
verifying it, and serving it directly (`DIG_NODE_ON_MISS=fetch`). A provider-held resource is never
silently 404'd; a fetch-through failure falls back to the redirect. Redirect hops are bounded (a
request already redirected `4` times is answered with the plain not-found) so nodes cannot bounce a
caller in a loop; the caller echoes the served `redirect_depth` on its re-request.

The multi-source fetch is a `locate → confirm → fan byte-ranges across multiple providers → verify
each range + the whole resource against the chain-anchored root → reassemble` pipeline, with
per-range resume so an interrupted transfer re-fetches only the missing ranges. Every served window
(fetch-through) carries the same per-range verification metadata (total length, chunk lengths,
inclusion proof, root) a locally-held serve does, so it is indistinguishable in shape and the caller
verifies it against the chain-anchored root — a peer mix can never forge content.

### 5.1 Self-optimizing peer selection

A **peer selector** is the decision + learning layer between DHT discovery and the download executor:
of the providers `find_providers` returns, it decides WHICH subset serves the content and in what
order, and it learns the answer from the REAL, measured outcome of every range it influenced. It has
NO user-facing configuration — every tradeoff (a per-connection-class saturation point, an adaptive
relayed penalty, the recency decay) is self-tuned from observed data.

- **The loop.** On a content want the node calls `find_providers`, hands the located providers to the
  selector's `select`, and the download executor fans byte-ranges only across the selector's ranked
  subset (with each source's recommended concurrency), instead of picking sources blindly. As the
  transfer runs, every completed range (measured throughput = bytes transferred / wall-clock) and
  every failed range streams back into the selector via `record_outcome` IN REAL TIME, so the next
  `select` — and a mid-transfer `rebalance` — is smarter.
- **Selection drives replacement too.** When the executor's live sources run low it re-queries
  providers; that re-query is a `rebalance`, which re-ranks the up-to-the-moment learned models and
  de-ranks the peers already active, so the selector DRIVES the replacement-source choice, not a blind
  retry.
- **Measured-only, non-gameable.** A peer's quality is refined ONLY from measured outcomes; there is
  no input path by which a peer raises its own score, and observed capacity always overrides any
  advertised capacity. A range that fails merkle/decryption verification is a HARD failure that drives
  the source toward the bottom of the ranking (below unmeasured peers) — a bad or hostile source is
  routed around. A `Banned` peer (from pool churn) is ineligible until re-added.
- **Registry feed.** The selector's candidate registry is fed by the connected-pool churn (a pool
  `PeerAdded` upserts a candidate, preserving any learned quality; a `PeerRemoved` marks it
  disconnected but retains its history for a reconnect) and, where available, the dig-nat connection
  class of a link (an observational prior only, subordinate to measured outcomes). The identity is the
  SAME transport-verified `peer_id = SHA-256(TLS SPKI DER)` used everywhere else (§1); the selector
  re-uses it verbatim.
- **Boundaries.** The selector opens no socket, runs no discovery/DHT, and fetches/verifies no bytes —
  those remain the DHT's, dig-nat's, and the download executor's jobs. It only reads their outputs and
  drives their choices. It is a pure, in-memory decision layer; its learned state is not persisted (a
  restart re-learns from the resumed transfer's outcomes).
- **Where it runs.** Like the rest of the peer network, the selector-driven fetch path runs only in
  the standalone binary. The in-process FFI path (the browser) is a pure consumer: no peer network, no
  selector, and the byte-exact `dig.getContent` / §21 read contract is unaffected.

## 6. Configuration (environment)

- `DIG_PEER_PORT` — peer-RPC listen port (default `9444`).
- `DIG_NETWORK_ID` — network id registered/discovered under (default `DIG_MAINNET`).
- `DIG_RELAY_URL` — relay endpoint (default `wss://relay.dig.net:9450`); `off`/`disabled` disables the
  reservation.
- `DIG_PEER_NETWORK` — `off`/`0`/`false` disables the peer network entirely (HTTP read path only).
- `DIG_NODE_ON_MISS` — `fetch`/`fetch-through` makes a content miss FETCH-THROUGH (pull + verify +
  serve) instead of the default REDIRECT.
- `DIG_NODE_ADVERTISE_LOOPBACK` — truthy to advertise loopback candidates when no routable address is
  discoverable (§2.2). Off by default.

## 7. Conformance

- The peer-RPC wire framing, the DHT request/response encoding, and the mTLS `peer_id` derivation MUST
  match the peer crates byte-for-byte (`dig-nat`, `dig-dht`, `dig-gossip`); see the ecosystem
  `SYSTEM.md` shared-contract map.
- The IPv6-first policy (§2) is the ecosystem-wide HARD RULE and MUST hold on every peer-comms surface
  the node exposes (bind, advertise, dial).
- The peer selector is the authoritative source-selection layer for the multi-source fetch: source
  choice MUST flow through it (`select`/`rebalance`), and every range outcome MUST be fed back via
  `record_outcome`. Its API shapes + identity/candidate types MUST match `dig-peer-selector` (which
  re-uses the `dig-nat`/`dig-dht` identity/content types); the node maps the `dig-gossip` pool-churn
  event into the selector's local churn-event shape 1:1 (field-identical).
