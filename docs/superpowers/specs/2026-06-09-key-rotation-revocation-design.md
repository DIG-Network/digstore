# Design: Key Rotation & Root Revocation (Residual Risk #1)

- **Date:** 2026-06-09
- **Status:** Design (brainstorm) — for review, then phased implementation
- **Closes:** SECURITY.md residual risk #1 — "a leaked store key cannot be rotated
  without minting a new store id, and there is no signed tombstone to retract a
  previously published root. The store key is effectively long-lived."

## 1. The hard constraint

Store identity is the key hash: `store_id = SHA-256(store BLS public key)` (§20.1),
and a URN embeds the `store_id`. Therefore **you cannot change the signing key
while keeping the same `store_id`** — the identity *is* the key. Any "rotation"
that preserves URLs is impossible by construction; rotation necessarily means a
new `store_id` (new URNs), cryptographically linked to the old one.

A second reality: if the hot signing key **leaks**, the attacker can produce every
signature the legitimate holder can — including a "rotation" or "revocation"
signed by that same key. So a single-key scheme cannot defend against its own
leak. Real leak-recovery requires a *separate* authority committed up front.

These two facts shape the design into three independently-shippable layers.

## 2. Three layers (ship in order)

### Layer 1 — Signed root revocation (tombstones) — IMPLEMENTABLE NOW

The tractable, high-value piece: let a publisher **retract a previously published
root** with a signed tombstone that remotes persist and clients honor fail-closed.

- **Tombstone record** (new, signed by the store key):
  ```
  Tombstone {
    store_id:  Bytes32,
    scope:     Root(Bytes32) | Store,     // retract one generation, or the whole store
    not_after: u64,                        // unix seconds; the revocation's own timestamp
    reason:    u8,                         // 0=unspecified, 1=compromise, 2=superseded, 3=takedown
  }
  signature: Bytes96 = BLS_sign(store_key, domain_tomb || canonical(Tombstone))
  ```
  `domain_tomb` is a per-role domain-separation tag (coordinate with residual #2).
- **Wire / remote:** the remote stores tombstones per store and returns the active
  set in the store descriptor (next to the authenticated head, §21.6). A
  `Root`-scoped tombstone retracts that generation; a `Store`-scoped tombstone
  retracts everything up to `not_after`.
- **Client behavior (fail-closed):** `clone`/`pull`/`cat` verify the tombstone
  signature against the store-id-bound module key and **refuse** to serve or trust
  a revoked root (and refuse the whole store if `Store`-scoped). A revoked head
  cannot be "un-revoked" by an older unsigned response (monotonic, like the
  authenticated head).
- **What it does NOT do:** it cannot stop a *malicious origin that withholds* the
  tombstone (the origin is untrusted for availability). It binds honest infra +
  any client that has seen the tombstone. That is the standard, useful guarantee.

This closes "no way to retract a published root" without touching the identity
model. **Recommended to implement in the residual-risk code phase.**

### Layer 2 — Key succession (planned, voluntary rotation) — DESIGNED, FUTURE

For *planned* rotation (not leak): the old key endorses a new identity.

- **Succession record** (signed by the OLD store key):
  ```
  Succession { old_store_id, new_store_id, new_public_key: Bytes48, at: u64 }
  signature = BLS_sign(old_key, domain_succ || canonical(Succession))
  ```
- Old URNs keep resolving the old (immutable) content; new content publishes under
  `new_store_id`. A client that trusts the old key can **follow** the chain to the
  new store. Remotes serve the succession record alongside the descriptor.
- **Limitation:** signed by the hot key, so it does not help a *leak* (the attacker
  can also sign a succession to an identity *they* control). Useful only for
  voluntary rotation by an uncompromised holder.

### Layer 3 — Offline recovery authority (leak defense) — DESIGNED, FUTURE, BIGGER

The only real defense against a hot-key leak: commit a **separate, offline
recovery key** at `init`, used solely to authorize revocation/succession.

- At `init`, generate two keys: the **hot** signing key (used per-commit/push) and
  a **cold recovery** key kept offline. Commit *both* to the identity, e.g.
  `store_id = SHA-256(hot_pub || recovery_pub)` (or include `SHA-256(recovery_pub)`
  in a committed, signed genesis record).
- A **recovery action** (revoke-and-rotate) is signed by the **recovery** key: it
  tombstones the compromised hot key and endorses a new hot key (new `store_id`).
  Because the recovery key never touches the hot path, a hot-key leak cannot forge
  it.
- **Cost:** changes the identity derivation + genesis + every place `store_id` is
  computed/verified, and adds offline-key UX. This is a deliberate, separate
  project; do NOT bundle it with Layers 1–2.

## 3. Recommendation

1. **Now (residual-risk code phase):** implement **Layer 1 tombstones** — signed
   root/store revocation, persisted by the remote, verified fail-closed by clients,
   with a regression test proving a revoked root is refused. Highest value, no
   identity-model change. Use the domain-separation tags from residual #2.
2. **Next minor:** **Layer 2 succession** for voluntary rotation.
3. **Deliberate future project:** **Layer 3 offline recovery key** — the genuine
   leak-recovery story; needs its own spec + migration because it reshapes the
   `store_id` derivation that the entire URN system rests on.

## 4. Out of scope here
- Changing `store_id` to support same-identity rotation (impossible by design).
- Defending against an origin that simply withholds tombstones (availability, not
  authenticity).
- Multi-party / threshold control of the recovery key (possible Layer-3 extension).

## 5. Testing (Layer 1)
- Unit: tombstone canonical encoding + BLS sign/verify with the role domain tag;
  monotonicity (a revoked root stays revoked).
- Integration: a remote seeded with a `Root` tombstone → `clone`/`pull` of that
  root fails closed; a `Store` tombstone → the whole store is refused; an unsigned
  or wrong-key tombstone is ignored (does not revoke).
- Negative: a non-revoked root still serves; tombstone tampering fails verification.
