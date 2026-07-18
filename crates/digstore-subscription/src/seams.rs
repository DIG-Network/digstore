//! The four injected **seams** the Subscription core drives.
//!
//! The Subscription primitive owns the *policy* (which capsules to fetch, in what order,
//! how reorgs are handled) but none of the *mechanism* (chain reads, network `.dig`
//! fetches, held-module checks, disk I/O). Those are traits, provided by the consumer —
//! the same injected-seam pattern dig-node already uses (`AnchoredRootResolver`,
//! `GapFiller`, `HeldCheck`). This keeps the core networkless, runtime-agnostic, and
//! unit-testable with deterministic mocks.

use async_trait::async_trait;
use digstore_core::{Bytes32, Capsule};

use crate::lineage::Lineage;
use crate::set::SubscriptionSet;

/// The chain's answer for one store this tick.
///
/// The FAIL-CLOSED contract lives here: an implementation MUST return `Err` when the chain
/// read failed and `Ok(None)` when the store has no confirmed generation. Only a
/// chain-CONFIRMED walk yields `Ok(Some(lineage))`. The reconcile core never fetches on
/// `Err`/`Ok(None)`, so an unconfirmable root is never acted on.
pub type LineageResult = Result<Option<Lineage>, String>;

/// Resolve a store's confirmed capsule lineage (eve → tip) from the chain.
///
/// This generalizes dig-node's `AnchoredRootResolver` from "the single confirmed tip root"
/// to "the full confirmed lineage" — the extra history is what enables full-history
/// backfill. Production impls wrap a coinset walk (dig-store's `sync_datastore_with_history`).
#[async_trait]
pub trait ChainWatch: Send + Sync {
    /// Resolve the confirmed lineage for `store_id`, or the fail-closed no-op signals.
    async fn lineage(&self, store_id: &Bytes32) -> LineageResult;
}

/// Find + sync + verify + land the `.dig` for one capsule on the network.
///
/// This IS the "search the network for the `.dig` matching this tip and sync it down"
/// leg (#979 clauses 2 + 3), generalized from dig-node's `GapFiller` to ANY capsule in the
/// lineage rather than only the latest tip. It MUST verify the fetched module against the
/// chain-anchored root before landing it, and be idempotent (a call for an already-held
/// capsule is a cheap success).
#[async_trait]
pub trait CapsuleFetcher: Send + Sync {
    /// Pull + verify + cache the `.dig` for `capsule`. `Ok(())` on a verified, landed
    /// module; `Err` describes the failure (the loop records it + retries next tick).
    async fn fetch(&self, capsule: Capsule) -> Result<(), String>;
}

/// Whether the `.dig` for a capsule is already held locally.
///
/// A thin seam over the consumer's inventory (dig-node's `module_exists`) so the "is this
/// generation missing?" check is injectable in tests.
pub trait HeldCheck: Send + Sync {
    /// `true` iff the `.dig` for `capsule` is present locally.
    fn is_held(&self, capsule: &Capsule) -> bool;
}

/// Load + save the persisted [`SubscriptionSet`].
///
/// The set's add/remove/codec policy is pure (in [`crate::set`]); this seam is the thin
/// disk (or other durable store) I/O the consumer owns, keeping this crate filesystem-free.
pub trait Persistence: Send + Sync {
    /// Load the persisted set. A missing/corrupt store MUST yield an empty set, never an error.
    fn load(&self) -> SubscriptionSet;
    /// Persist the set durably (atomic write recommended).
    fn save(&self, set: &SubscriptionSet) -> Result<(), String>;
}
