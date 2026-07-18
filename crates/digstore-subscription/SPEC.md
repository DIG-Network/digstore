# digstore-subscription — SPEC

Normative contract for the canonical DIG Network **Subscription** primitive. An independent
reimplementation MUST conform to this document. Cross-references: superproject `SYSTEM.md`
("Subscription — the canonical chain-watch primitive"), CLAUDE.md §5.1 (`.dig` permanence),
§5.3 (client→node read ladder), dig-store `SPEC.md` (`Capsule`, the §21 remote).

## 1. Concept

A **Subscription** follows exactly ONE CHIP-0035 store singleton and keeps a local set of
`.dig` files in sync with that singleton's on-chain history. A **capsule** is one immutable
store generation `(store_id, root_hash)`, written `storeId:rootHash`; a store is the ordered
sequence of capsules its singleton has committed, eve → current tip.

A Subscription MUST:

1. **Watch** the chain for the store's singleton tip progressing (a new generation / new root).
2. **On tip progression**, find the `.dig` matching the new tip's capsule on the network and
   sync it down.
3. **Track the ENTIRE singleton history** and backfill every historical tip's `.dig` where
   possible (best-effort; a tip whose `.dig` cannot be found is recorded missing and retried).
4. Be a **distinct, isolated behaviour** — one managed object per store, owning its watch
   state, full-history tracking, and per-tip sync status.

## 2. Architecture — a networkless core over four seams

The primitive owns POLICY (which capsules to fetch, ordering, reorg handling) and NONE of the
mechanism. Mechanism is four injected traits so the decision core and state machine are pure
and unit-testable with no chain and no network:

- **`ChainWatch`** — resolve a store's confirmed capsule lineage (eve → tip). Return type
  `LineageResult = Result<Option<Lineage>, String>`. This carries the fail-closed contract (§4).
- **`CapsuleFetcher`** — find + sync + verify + land the `.dig` for one capsule. MUST verify the
  fetched module against the chain-anchored root before landing it, and be idempotent.
- **`HeldCheck`** — whether the `.dig` for a capsule is already held locally.
- **`Persistence`** — load/save the persisted `SubscriptionSet`. A missing/corrupt store loads
  as an empty set, never an error.

A `Lineage` is an ordered, non-empty capsule list all sharing one `store_id`; it is exactly
the `history` a chain walk (dig-store's `sync_datastore_with_history` → `StoreHistory`)
produces. It is modelled chain-free so the crate does not depend on the spend-heavy
`digstore-chain` crate; a consumer adapts `StoreHistory.history` (a `Vec<Capsule>`) straight
into `Lineage::try_new`.

## 3. State machine

A `Subscription` tracks, per store:

- the last-observed confirmed `history` (ordered `Capsule` lineage, eve → tip);
- a per-capsule `TipSync` status keyed by root: `Held`, `Missing`, `Pending`,
  `Failed { attempts, last_error }`;
- the `orphaned` capsules — those dropped from the canonical lineage by a reorg (§5).

`observe_lineage(lineage, held)` folds a freshly-confirmed lineage into the state: it detects
tip progression (new capsules appended) and reorg (§5), and refreshes each capsule's status
from `HeldCheck`. A `Failed`/`Pending` record is preserved across a re-observe (so retry counts
survive) unless the `.dig` is now held.

`record_fetch_result(capsule, result)` sets `Held` on success, else `Failed` with an
incremented attempt count (retries accrue across ticks).

## 4. Fail-closed invariant (MANDATORY)

A Subscription MUST NEVER fetch or verify against an unconfirmable root:

- `ChainWatch` returning `Err(_)` (chain read failed) ⇒ the tick does NOTHING (`Skipped(ChainError)`);
- `ChainWatch` returning `Ok(None)` (no confirmed generation) ⇒ the tick does NOTHING
  (`Skipped(NoConfirmedGeneration)`);
- only `Ok(Some(lineage))` — a chain-CONFIRMED walk — drives observe + fetch.

The decision core `decide` is only ever handed a confirmed `Lineage`, so it structurally cannot
act on an unconfirmable root.

## 5. Reorg / permanence invariant (MANDATORY — §5.1)

`.dig` files are permanent, on-chain-anchored artifacts. When a reorg supersedes a root (drops
it from the canonical lineage returned this tick):

- the superseded capsule is moved to `orphaned` and RETAINED — never deleted;
- its `TipSync` status is preserved (an already-`Held` superseded root stays `Held`);
- the primitive exposes NO eviction/delete API — a held `.dig` can never be evicted by a reorg.

The canonical lineage each tick is whatever `ChainWatch` returns from the current unspent tip;
new tips append, dropped tips orphan, held content persists.

## 6. Fetch ordering (deterministic)

`decide(lineage, held)` emits `SyncAction::Fetch(capsule)` for every capsule not already held:

1. the current **tip first** (§ clause 2 — a client following the store gets current fastest);
2. then the remaining gaps **oldest → newest** (§ clause 3 backfill);
3. duplicate roots collapse — a store that reverted to a prior root fetches that `.dig` once.

## 7. Persisted subscription set

`SubscriptionSet` is an order-preserving, de-duplicated set of lower-case 64-hex store ids. Ids
are normalized (trimmed + lower-cased) on insert so a mixed-case duplicate collapses to one.
`add`/`remove` are idempotent and reject non-64-hex ids. The JSON codec (`encode`/`decode`) is a
schema-versioned document (`{ "version": 1, "stores": [...] }`); `decode` tolerates an empty,
garbage, or legacy bare `{ "stores": [...] }` input as an empty/best-effort set and drops
malformed entries. The document schema is additive-only (a version bump never removes or
repurposes a field), so an old reader ignores unknown fields and a new reader defaults them.

## 8. Watch cadence

The host owns the loop and the async runtime; the crate is runtime-agnostic (`reconcile_tick`
for one store, `Subscriptions::reconcile_all` for the set). `watch_interval_from_env` resolves
the poll interval from `DIG_WATCH_INTERVAL` (seconds), defaulting to 30 s and floored at 1 s so
a mis-set value cannot flood the chain source.
