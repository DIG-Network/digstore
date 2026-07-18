//! The canonical DIG Network **Subscription** primitive.
//!
//! A [`Subscription`] follows ONE CHIP-0035 store singleton and keeps a local set of `.dig`
//! files in sync with that singleton's on-chain history. It is the reusable engine behind
//! "follow this store and keep it current + complete" — instantiated per store, owning its
//! watch state, its full-history tracking, and its per-tip sync/backfill (#979).
//!
//! Contract (user-normative, #979):
//! 1. **Watch** the chain for the store's singleton tip progressing (a new generation/root).
//! 2. **On tip progression**, find the `.dig` matching the new tip (capsule = `storeId:rootHash`)
//!    on the network and sync it down.
//! 3. **Track the ENTIRE singleton history** and backfill every historical tip's `.dig` where
//!    possible (best-effort; a tip whose `.dig` can't be found is recorded missing + retried).
//! 4. Be a **distinct, isolated behaviour** — one managed object per store.
//!
//! # Architecture — a networkless core over four seams
//!
//! The primitive owns the POLICY (which capsules to fetch, in what order, how reorgs are
//! handled) and none of the MECHANISM. The mechanism is four injected traits —
//! [`ChainWatch`], [`CapsuleFetcher`], [`HeldCheck`], [`Persistence`] — so the decision core
//! ([`decide`]) and the state machine ([`Subscription`]) are pure and unit-testable with no
//! chain and no network. Consumers (dig-node in Phase 2) provide the real seam impls.
//!
//! # Two load-bearing invariants
//!
//! - **Fail-closed** ([`reconcile_tick`]): a chain error or a store with no confirmed
//!   generation does NOTHING — an unconfirmable root is never fetched or verified against.
//! - **Reorg / permanence** (§5.1, [`Subscription::observe_lineage`]): a root superseded by a
//!   reorg is retained (moved to [`Subscription::orphaned`]); an already-held `.dig` is never
//!   evicted. `.dig` files are permanent, on-chain-anchored artifacts.

#![forbid(unsafe_code)]

pub mod decide;
pub mod lineage;
pub mod reconcile;
pub mod seams;
pub mod set;
pub mod subscription;

pub use decide::{decide, SyncAction};
pub use lineage::Lineage;
pub use reconcile::{
    reconcile_tick, watch_interval_from_env, SkipReason, SubscriptionDeps, Subscriptions,
    TickOutcome, DEFAULT_WATCH_INTERVAL_SECS, MIN_WATCH_INTERVAL_SECS,
};
pub use seams::{CapsuleFetcher, ChainWatch, HeldCheck, LineageResult, Persistence};
pub use set::{decode, encode, normalize_store_id, SubscriptionSet, SubscriptionsDoc};
pub use subscription::{Subscription, TipSync};

// The canonical capsule identity a Subscription follows, re-exported for consumers.
pub use digstore_core::{Bytes32, Capsule};
