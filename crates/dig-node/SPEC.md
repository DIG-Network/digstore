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

## 5. Configuration (environment)

- `DIG_PEER_PORT` — peer-RPC listen port (default `9444`).
- `DIG_NETWORK_ID` — network id registered/discovered under (default `DIG_MAINNET`).
- `DIG_RELAY_URL` — relay endpoint (default `wss://relay.dig.net:9450`); `off`/`disabled` disables the
  reservation.
- `DIG_PEER_NETWORK` — `off`/`0`/`false` disables the peer network entirely (HTTP read path only).
- `DIG_NODE_ADVERTISE_LOOPBACK` — truthy to advertise loopback candidates when no routable address is
  discoverable (§2.2). Off by default.

## 6. Conformance

- The peer-RPC wire framing, the DHT request/response encoding, and the mTLS `peer_id` derivation MUST
  match the peer crates byte-for-byte (`dig-nat`, `dig-dht`, `dig-gossip`); see the ecosystem
  `SYSTEM.md` shared-contract map.
- The IPv6-first policy (§2) is the ecosystem-wide HARD RULE and MUST hold on every peer-comms surface
  the node exposes (bind, advertise, dial).
