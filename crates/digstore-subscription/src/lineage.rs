//! The confirmed **lineage** of a store singleton — its ordered capsule history.
//!
//! A [`Lineage`] is the chain-confirmed sequence of capsules `(store_id, root)` a store
//! has ever been, oldest (eve) → newest (the current unspent tip). It is exactly the
//! ordered `history` that a chain walk (dig-store's `sync_datastore_with_history`, which
//! returns `StoreHistory { current, history }`) produces — modelled here as a plain,
//! chain-free value so the Subscription core stays networkless and does not depend on the
//! spend-heavy `digstore-chain` crate. A consumer adapts its `StoreHistory.history`
//! (`Vec<Capsule>`) straight into [`Lineage::try_new`].
//!
//! A `Lineage` is only ever constructed from a CHAIN-CONFIRMED walk. The fail-closed
//! invariant lives at the [`ChainWatch`](crate::ChainWatch) seam: a chain error or a store
//! with no confirmed generation yields NO lineage (never an empty/partial one), so the
//! decision core is never handed an unconfirmable root to act on.

use digstore_core::{Bytes32, Capsule};

/// The ordered, chain-confirmed capsule history of one store singleton, eve → tip.
///
/// Invariants (enforced by [`try_new`](Lineage::try_new)): non-empty, and every capsule
/// shares the same `store_id`. The last element is the current tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lineage {
    store_id: Bytes32,
    capsules: Vec<Capsule>,
}

impl Lineage {
    /// Build a lineage from an ordered (eve → tip) capsule list.
    ///
    /// Returns `Err` if the list is empty (a confirmed lineage always has at least the eve
    /// generation) or if any capsule's `store_id` disagrees with the first — a mixed-store
    /// walk is corrupt input and must not drive a sync.
    pub fn try_new(capsules: Vec<Capsule>) -> Result<Lineage, String> {
        let store_id = capsules
            .first()
            .ok_or_else(|| "lineage must have at least the eve capsule".to_string())?
            .store_id;
        if let Some(bad) = capsules.iter().find(|c| c.store_id != store_id) {
            return Err(format!(
                "lineage mixes store ids: {} vs {}",
                store_id.to_hex(),
                bad.store_id.to_hex()
            ));
        }
        Ok(Lineage { store_id, capsules })
    }

    /// The store this lineage belongs to.
    pub fn store_id(&self) -> Bytes32 {
        self.store_id
    }

    /// The full ordered capsule history, eve → tip.
    pub fn capsules(&self) -> &[Capsule] {
        &self.capsules
    }

    /// The current tip capsule (the last, newest generation).
    pub fn tip(&self) -> Capsule {
        *self
            .capsules
            .last()
            .expect("lineage is non-empty by construction")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(store: u8, root: u8) -> Capsule {
        Capsule {
            store_id: Bytes32([store; 32]),
            root_hash: Bytes32([root; 32]),
        }
    }

    /// **Proves:** a well-formed lineage exposes its store id + ordered capsules + tip.
    #[test]
    fn builds_and_exposes_tip() {
        let l = Lineage::try_new(vec![cap(1, 10), cap(1, 20), cap(1, 30)]).unwrap();
        assert_eq!(l.store_id(), Bytes32([1; 32]));
        assert_eq!(l.capsules().len(), 3);
        assert_eq!(l.tip(), cap(1, 30), "tip is the newest generation");
    }

    /// **Proves:** an empty lineage is rejected (a confirmed lineage always has the eve gen).
    #[test]
    fn rejects_empty() {
        assert!(Lineage::try_new(vec![]).is_err());
    }

    /// **Proves:** a lineage mixing store ids is rejected as corrupt (never drives a sync).
    #[test]
    fn rejects_mixed_store_ids() {
        assert!(Lineage::try_new(vec![cap(1, 10), cap(2, 20)]).is_err());
    }
}
