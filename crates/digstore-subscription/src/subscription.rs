//! The [`Subscription`] — a per-store managed object owning the history-tracking state
//! and per-capsule sync status (#979 clause 4: a distinct, isolated behaviour).
//!
//! It tracks the store's last-known confirmed lineage, the [`TipSync`] status of each
//! capsule, and — critically — the capsules that have been SUPERSEDED by a reorg. The
//! reorg/permanence invariant (§5.1) is structural: [`observe_lineage`](Subscription::observe_lineage)
//! never removes a status entry and this type exposes NO eviction API, so a superseded
//! root that was already held stays recorded as held and its `.dig` is never dropped.

use std::collections::BTreeMap;

use digstore_core::{Bytes32, Capsule};

use crate::decide::{decide, SyncAction};
use crate::lineage::Lineage;
use crate::seams::HeldCheck;

/// The sync state of one capsule (one historical or current tip) in a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TipSync {
    /// The `.dig` for this capsule is held locally.
    Held,
    /// The `.dig` is not held and no fetch has been attempted yet — a backfill worklist item.
    Missing,
    /// A fetch is in flight this tick.
    Pending,
    /// The last fetch failed; retried on a later tick. Carries the attempt count + last error.
    Failed { attempts: u32, last_error: String },
}

/// A subscription to one store: its tracked lineage, per-capsule sync status, and the
/// capsules superseded by a reorg (retained, never evicted).
#[derive(Debug, Clone)]
pub struct Subscription {
    store_id: Bytes32,
    /// The last-observed confirmed lineage, eve → tip.
    history: Vec<Capsule>,
    /// Per-capsule sync status, keyed by root hex (root hashes are content-addressed).
    status: BTreeMap<String, TipSync>,
    /// Capsules that were in a previously-observed lineage but dropped out of the current
    /// canonical one (a reorg superseded them). Retained so an already-held superseded
    /// `.dig` is never forgotten — §5.1 permanence.
    orphaned: Vec<Capsule>,
}

impl Subscription {
    /// A fresh subscription with no observed history yet.
    pub fn new(store_id: Bytes32) -> Self {
        Subscription {
            store_id,
            history: Vec::new(),
            status: BTreeMap::new(),
            orphaned: Vec::new(),
        }
    }

    /// The store this subscription follows.
    pub fn store_id(&self) -> Bytes32 {
        self.store_id
    }

    /// The current tip capsule (the newest observed generation), or `None` before the first
    /// confirmed lineage is observed.
    pub fn current_tip(&self) -> Option<Capsule> {
        self.history.last().copied()
    }

    /// The full ordered lineage view, eve → tip (#979 clause 3).
    pub fn history(&self) -> &[Capsule] {
        &self.history
    }

    /// The capsules superseded by a reorg — dropped from the canonical lineage but retained
    /// (their `.dig`, if held, is permanent).
    pub fn orphaned(&self) -> &[Capsule] {
        &self.orphaned
    }

    /// The sync status of the capsule with this root, if tracked.
    pub fn tip_status(&self, root: &Bytes32) -> Option<&TipSync> {
        self.status.get(&root.to_hex())
    }

    /// The backfill worklist: capsules in the current lineage whose `.dig` is not held
    /// (status [`TipSync::Missing`] or [`TipSync::Failed`]), tip-first then oldest → newest.
    pub fn missing(&self) -> Vec<Capsule> {
        let tip = self.current_tip();
        let is_missing = |c: &Capsule| {
            matches!(
                self.status.get(&c.root_hash.to_hex()),
                Some(TipSync::Missing) | Some(TipSync::Failed { .. })
            )
        };
        let mut out: Vec<Capsule> = Vec::new();
        if let Some(t) = tip {
            if is_missing(&t) {
                out.push(t);
            }
        }
        for c in &self.history {
            if Some(*c) != tip && is_missing(c) && !out.iter().any(|q| q.root_hash == c.root_hash) {
                out.push(*c);
            }
        }
        out
    }

    /// Observe a freshly-confirmed lineage: detect tip progression AND reorg, and refresh
    /// per-capsule status against the held inventory.
    ///
    /// Reorg/permanence (§5.1): any capsule in the PRIOR lineage that is absent from the new
    /// one is moved to [`orphaned`](Subscription::orphaned) — its status entry is kept (a held
    /// superseded root stays [`TipSync::Held`]), and nothing is ever deleted. New capsules are
    /// seeded [`TipSync::Held`] or [`TipSync::Missing`] from the held check; an entry already
    /// recorded as [`TipSync::Failed`] is left intact (so its retry count survives) unless the
    /// `.dig` is now held.
    pub fn observe_lineage(&mut self, lineage: &Lineage, held: &dyn HeldCheck) {
        let new_roots: Vec<String> = lineage
            .capsules()
            .iter()
            .map(|c| c.root_hash.to_hex())
            .collect();

        // Capsules dropped from the canonical lineage by a reorg → orphan them (retain).
        for prior in &self.history {
            let still_canonical = new_roots.contains(&prior.root_hash.to_hex());
            let already_orphan = self.orphaned.iter().any(|o| o.root_hash == prior.root_hash);
            if !still_canonical && !already_orphan {
                self.orphaned.push(*prior);
            }
        }

        // Refresh status for every capsule in the new lineage.
        for capsule in lineage.capsules() {
            let key = capsule.root_hash.to_hex();
            if held.is_held(capsule) {
                self.status.insert(key, TipSync::Held);
            } else {
                match self.status.get(&key) {
                    // Preserve an in-flight retry record; only downgrade to Missing if unknown.
                    Some(TipSync::Failed { .. }) | Some(TipSync::Pending) => {}
                    _ => {
                        self.status.insert(key, TipSync::Missing);
                    }
                }
            }
        }

        self.history = lineage.capsules().to_vec();
    }

    /// Mark the capsule's fetch as in-flight this tick.
    pub fn mark_pending(&mut self, capsule: &Capsule) {
        self.status
            .insert(capsule.root_hash.to_hex(), TipSync::Pending);
    }

    /// Record the outcome of a fetch: [`TipSync::Held`] on success, else [`TipSync::Failed`]
    /// with an incremented attempt count (so retries accumulate across ticks).
    pub fn record_fetch_result(&mut self, capsule: &Capsule, result: Result<(), String>) {
        let key = capsule.root_hash.to_hex();
        match result {
            Ok(()) => {
                self.status.insert(key, TipSync::Held);
            }
            Err(e) => {
                let attempts = match self.status.get(&key) {
                    Some(TipSync::Failed { attempts, .. }) => attempts + 1,
                    _ => 1,
                };
                self.status.insert(
                    key,
                    TipSync::Failed {
                        attempts,
                        last_error: e,
                    },
                );
            }
        }
    }

    /// The ordered fetch worklist for the current lineage (delegates to the pure
    /// [`decide`](crate::decide) core). Empty before any lineage is observed.
    pub fn plan(&self, held: &dyn HeldCheck) -> Vec<SyncAction> {
        match Lineage::try_new(self.history.clone()) {
            Ok(lineage) => decide(&lineage, held),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(root: u8) -> Capsule {
        Capsule {
            store_id: Bytes32([1; 32]),
            root_hash: Bytes32([root; 32]),
        }
    }

    fn lineage(roots: &[u8]) -> Lineage {
        Lineage::try_new(roots.iter().map(|&r| cap(r)).collect()).unwrap()
    }

    /// A held-check over an explicit set of held roots.
    struct Held(Vec<u8>);
    impl HeldCheck for Held {
        fn is_held(&self, capsule: &Capsule) -> bool {
            self.0.contains(&capsule.root_hash.0[0])
        }
    }

    /// **Proves:** observing a lineage seeds status (held vs missing) + exposes tip + history.
    #[test]
    fn observe_seeds_status_and_views() {
        let mut sub = Subscription::new(Bytes32([1; 32]));
        sub.observe_lineage(&lineage(&[10, 20, 30]), &Held(vec![10]));
        assert_eq!(sub.current_tip(), Some(cap(30)));
        assert_eq!(sub.history(), &[cap(10), cap(20), cap(30)]);
        assert_eq!(sub.tip_status(&Bytes32([10; 32])), Some(&TipSync::Held));
        assert_eq!(sub.tip_status(&Bytes32([20; 32])), Some(&TipSync::Missing));
        assert_eq!(
            sub.missing(),
            vec![cap(30), cap(20)],
            "tip-first backfill worklist"
        );
    }

    /// **Proves:** a new tip progresses the lineage and appends the new missing capsule.
    #[test]
    fn tip_progression_appends() {
        let mut sub = Subscription::new(Bytes32([1; 32]));
        sub.observe_lineage(&lineage(&[10]), &Held(vec![10]));
        sub.observe_lineage(&lineage(&[10, 20]), &Held(vec![10]));
        assert_eq!(sub.current_tip(), Some(cap(20)));
        assert_eq!(sub.tip_status(&Bytes32([20; 32])), Some(&TipSync::Missing));
    }

    /// **Proves the reorg/permanence invariant (§5.1):** when a reorg drops an already-HELD
    /// capsule out of the canonical lineage, it is moved to `orphaned`, its status stays
    /// `Held`, and it is NEVER evicted.
    /// **Catches:** a tracker that forgets or downgrades a superseded held `.dig`.
    #[test]
    fn reorg_never_evicts_a_held_capsule() {
        let mut sub = Subscription::new(Bytes32([1; 32]));
        // First lineage: eve=10, tip=20, both held.
        sub.observe_lineage(&lineage(&[10, 20]), &Held(vec![10, 20]));
        assert_eq!(sub.tip_status(&Bytes32([20; 32])), Some(&TipSync::Held));

        // Reorg: 20 is superseded by a fork 10 → 30 (20 leaves the canonical lineage).
        sub.observe_lineage(&lineage(&[10, 30]), &Held(vec![10, 20]));

        assert_eq!(sub.current_tip(), Some(cap(30)), "new canonical tip");
        assert_eq!(
            sub.tip_status(&Bytes32([20; 32])),
            Some(&TipSync::Held),
            "superseded root stays Held — never evicted (§5.1)"
        );
        assert!(
            sub.orphaned().contains(&cap(20)),
            "superseded capsule retained as orphaned, not deleted"
        );
    }

    /// **Proves:** a failed fetch record survives a re-observe (retry count is not reset),
    /// and a success flips it to Held.
    #[test]
    fn failed_record_survives_reobserve_then_succeeds() {
        let mut sub = Subscription::new(Bytes32([1; 32]));
        sub.observe_lineage(&lineage(&[10]), &Held(vec![]));
        sub.record_fetch_result(&cap(10), Err("no peer".into()));
        assert_eq!(
            sub.tip_status(&Bytes32([10; 32])),
            Some(&TipSync::Failed {
                attempts: 1,
                last_error: "no peer".into()
            })
        );
        // Re-observe (still not held) must preserve the Failed record.
        sub.observe_lineage(&lineage(&[10]), &Held(vec![]));
        assert!(matches!(
            sub.tip_status(&Bytes32([10; 32])),
            Some(TipSync::Failed { attempts: 1, .. })
        ));
        // A second failure increments attempts.
        sub.record_fetch_result(&cap(10), Err("still no peer".into()));
        assert!(matches!(
            sub.tip_status(&Bytes32([10; 32])),
            Some(TipSync::Failed { attempts: 2, .. })
        ));
        // Success flips to Held.
        sub.record_fetch_result(&cap(10), Ok(()));
        assert_eq!(sub.tip_status(&Bytes32([10; 32])), Some(&TipSync::Held));
    }

    /// **Proves:** `plan` before any observed lineage is empty (no unconfirmable work).
    #[test]
    fn plan_empty_before_first_observe() {
        let sub = Subscription::new(Bytes32([1; 32]));
        assert!(sub.plan(&Held(vec![])).is_empty());
    }
}
