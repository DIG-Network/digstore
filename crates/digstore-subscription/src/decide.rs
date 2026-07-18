//! The pure decision core — given a confirmed lineage + what is held, decide which
//! capsules to fetch and in what order. No chain, no network, no I/O: fully unit-testable.
//!
//! This generalizes dig-node's `decide_watch` from "gap-fill the ONE confirmed tip" to
//! "gap-fill EVERY historical capsule not held" (#979 clause 3, full-history backfill).
//!
//! Ordering policy (deterministic + tested):
//! 1. **the current tip first** — clause 2: a new tip is the most-wanted `.dig`, fetched
//!    ahead of history so a client following the store gets current fastest;
//! 2. **then the remaining gaps, oldest → newest** — clause 3 backfill, in chronological
//!    order so history fills forward predictably.
//!
//! FAIL-CLOSED: [`decide`] only ever runs on a chain-CONFIRMED [`Lineage`]. A chain error
//! or a no-generation store never produces a lineage (the [`ChainWatch`](crate::ChainWatch)
//! seam returns `Err`/`Ok(None)`), so this function is never handed an unconfirmable root.

use digstore_core::Capsule;

use crate::lineage::Lineage;
use crate::seams::HeldCheck;

/// One unit of sync work the watcher should perform: fetch the `.dig` for this capsule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    /// Find + sync + verify + land the `.dig` for this capsule (via [`CapsuleFetcher`](crate::CapsuleFetcher)).
    Fetch(Capsule),
}

/// Decide the ordered fetch worklist for a confirmed lineage: every capsule not already
/// held, tip-first then oldest → newest, with duplicate roots collapsed (a store that
/// reverted to a prior root only needs that `.dig` fetched once).
pub fn decide(lineage: &Lineage, held: &dyn HeldCheck) -> Vec<SyncAction> {
    let tip = lineage.tip();
    let mut actions = Vec::new();
    let mut queued: Vec<Capsule> = Vec::new();

    let mut push_if_needed = |capsule: Capsule, actions: &mut Vec<SyncAction>| {
        // Skip what we already hold, and any root already queued this tick (dedup so a
        // repeated root — a revert — is fetched once).
        if held.is_held(&capsule) || queued.iter().any(|c| c.root_hash == capsule.root_hash) {
            return;
        }
        queued.push(capsule);
        actions.push(SyncAction::Fetch(capsule));
    };

    // 1. The current tip first (clause 2).
    push_if_needed(tip, &mut actions);
    // 2. Then historical gaps, oldest → newest (clause 3). The tip is already queued, so
    //    its (possibly repeated) root is skipped by the dedup.
    for &capsule in lineage.capsules() {
        push_if_needed(capsule, &mut actions);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    fn cap(root: u8) -> Capsule {
        Capsule {
            store_id: Bytes32([1; 32]),
            root_hash: Bytes32([root; 32]),
        }
    }

    /// A held-check backed by an explicit list of held roots.
    struct Held(Vec<u8>);
    impl HeldCheck for Held {
        fn is_held(&self, capsule: &Capsule) -> bool {
            self.0.contains(&capsule.root_hash.0[0])
        }
    }

    fn lineage(roots: &[u8]) -> Lineage {
        Lineage::try_new(roots.iter().map(|&r| cap(r)).collect()).unwrap()
    }

    fn fetched_roots(actions: &[SyncAction]) -> Vec<u8> {
        actions
            .iter()
            .map(|SyncAction::Fetch(c)| c.root_hash.0[0])
            .collect()
    }

    /// **Proves:** a fully-held lineage produces no work.
    /// **Catches:** a core that re-fetches held generations.
    #[test]
    fn nothing_to_do_when_all_held() {
        let actions = decide(&lineage(&[1, 2, 3]), &Held(vec![1, 2, 3]));
        assert!(actions.is_empty());
    }

    /// **Proves:** the tip is fetched FIRST, then history oldest → newest (clause 2 + 3).
    /// **Catches:** a core that backfills history before the current tip.
    #[test]
    fn tip_first_then_history_oldest_to_newest() {
        // Hold nothing; lineage eve=10, 20, 30, tip=40.
        let actions = decide(&lineage(&[10, 20, 30, 40]), &Held(vec![]));
        assert_eq!(
            fetched_roots(&actions),
            vec![40, 10, 20, 30],
            "tip first, then oldest→newest backfill"
        );
    }

    /// **Proves:** only the MISSING historical capsules are queued (backfill skips held).
    /// **Catches:** a backfill that ignores the held inventory.
    #[test]
    fn backfills_only_missing_history() {
        // Hold the tip (40) and one mid gen (20); 10 + 30 remain missing.
        let actions = decide(&lineage(&[10, 20, 30, 40]), &Held(vec![40, 20]));
        assert_eq!(fetched_roots(&actions), vec![10, 30]);
    }

    /// **Proves:** a repeated root (a revert to a prior generation) is fetched once.
    /// **Catches:** a core that double-fetches the same `.dig`.
    #[test]
    fn dedupes_repeated_roots() {
        // eve=10, 20, then reverted back to 10 as the tip.
        let actions = decide(&lineage(&[10, 20, 10]), &Held(vec![]));
        assert_eq!(
            fetched_roots(&actions),
            vec![10, 20],
            "root 10 fetched once"
        );
    }
}
