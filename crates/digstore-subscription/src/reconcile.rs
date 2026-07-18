//! The async **reconcile tick** — the one place that ties the chain seam → the state
//! machine → the network fetch seam together, best-effort + fail-closed.
//!
//! `reconcile_tick` drives ONE subscription for one tick; [`Subscriptions`] drives a whole
//! set. The host owns the loop/spawn (the crate stays runtime-agnostic — see
//! [`watch_interval_from_env`] for the interval policy the host applies).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use digstore_core::Bytes32;

use crate::decide::SyncAction;
use crate::seams::{CapsuleFetcher, ChainWatch, HeldCheck};
use crate::set::SubscriptionSet;
use crate::subscription::Subscription;

/// Default interval between chain-watch polls. Modest by design — a new generation confirms
/// on-chain in tens of seconds to minutes, so a ~30 s poll detects it promptly without
/// hammering the chain source. Overridable via `DIG_WATCH_INTERVAL` (seconds).
pub const DEFAULT_WATCH_INTERVAL_SECS: u64 = 30;

/// The floor on the configured watch interval (seconds) — so a mis-set env var can't turn
/// the loop into a chain-source flood.
pub const MIN_WATCH_INTERVAL_SECS: u64 = 1;

/// Resolve the watch-poll interval from `DIG_WATCH_INTERVAL` (seconds), floored at
/// [`MIN_WATCH_INTERVAL_SECS`] and defaulting to [`DEFAULT_WATCH_INTERVAL_SECS`].
pub fn watch_interval_from_env() -> Duration {
    let secs = std::env::var("DIG_WATCH_INTERVAL")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_WATCH_INTERVAL_SECS)
        .max(MIN_WATCH_INTERVAL_SECS);
    Duration::from_secs(secs)
}

/// Why a reconcile tick did no fetching for a store — the fail-closed no-ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The chain read failed — never fetch against an unconfirmable root.
    ChainError,
    /// The store has no confirmed on-chain generation yet.
    NoConfirmedGeneration,
}

/// The outcome of reconciling one store this tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// No work — the fail-closed no-op (chain error or no confirmed generation).
    Skipped(SkipReason),
    /// The lineage was confirmed and reconciled: `attempted` fetches, `fetched` succeeded.
    Synced { attempted: usize, fetched: usize },
}

/// The seams a reconcile tick drives.
#[derive(Clone)]
pub struct SubscriptionDeps {
    /// The confirmed-lineage source (fail-closed).
    pub chain: Arc<dyn ChainWatch>,
    /// The network `.dig` fetch actuator.
    pub fetcher: Arc<dyn CapsuleFetcher>,
    /// The held-module check.
    pub held: Arc<dyn HeldCheck>,
}

/// Reconcile ONE subscription for one tick: resolve its confirmed lineage, fold it into the
/// subscription's state (tip progression + reorg), then drive the ordered fetch worklist.
///
/// FAIL-CLOSED: a chain `Err` or a no-generation `Ok(None)` returns a [`TickOutcome::Skipped`]
/// with NO fetch call — an unconfirmable root is never acted on.
pub async fn reconcile_tick(sub: &mut Subscription, deps: &SubscriptionDeps) -> TickOutcome {
    let lineage = match deps.chain.lineage(&sub.store_id()).await {
        Err(_) => return TickOutcome::Skipped(SkipReason::ChainError),
        Ok(None) => return TickOutcome::Skipped(SkipReason::NoConfirmedGeneration),
        Ok(Some(lineage)) => lineage,
    };

    sub.observe_lineage(&lineage, deps.held.as_ref());

    let mut attempted = 0;
    let mut fetched = 0;
    for SyncAction::Fetch(capsule) in sub.plan(deps.held.as_ref()) {
        attempted += 1;
        sub.mark_pending(&capsule);
        let result = deps.fetcher.fetch(capsule).await;
        match &result {
            Ok(()) => {
                fetched += 1;
                tracing::info!(capsule = %capsule.canonical(), "subscription: synced .dig");
            }
            Err(e) => tracing::warn!(
                capsule = %capsule.canonical(),
                error = %e,
                "subscription: fetch failed; will retry next tick"
            ),
        }
        sub.record_fetch_result(&capsule, result);
    }
    TickOutcome::Synced { attempted, fetched }
}

/// A live set of [`Subscription`]s keyed by store id, driven together each tick.
///
/// Membership is synced from a [`SubscriptionSet`] (the persisted worklist): a newly
/// subscribed store gets a fresh [`Subscription`]; an unsubscribed store is dropped from the
/// live set (its retained `.dig` files are the consumer's business — this only stops watching).
#[derive(Debug, Default)]
pub struct Subscriptions {
    subs: BTreeMap<String, Subscription>,
}

impl Subscriptions {
    /// An empty live set.
    pub fn new() -> Self {
        Subscriptions::default()
    }

    /// Reconcile membership against the persisted set: add subscriptions for newly-listed
    /// stores, drop those no longer listed. Existing subscriptions (and their tracked state)
    /// are preserved.
    pub fn sync_membership(&mut self, set: &SubscriptionSet) {
        let listed: Vec<String> = set.stores().to_vec();
        self.subs.retain(|k, _| listed.contains(k));
        for store_hex in listed {
            if let Some(store_id) = parse_store_id(&store_hex) {
                self.subs
                    .entry(store_hex)
                    .or_insert_with(|| Subscription::new(store_id));
            }
        }
    }

    /// The live subscriptions (store hex → subscription), for status reporting.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Subscription)> {
        self.subs.iter()
    }

    /// The subscription for a store id, if watched.
    pub fn get(&self, store_hex: &str) -> Option<&Subscription> {
        self.subs.get(store_hex)
    }

    /// How many stores are watched.
    pub fn len(&self) -> usize {
        self.subs.len()
    }

    /// Whether no stores are watched.
    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }

    /// Reconcile every watched subscription once, returning each store's outcome.
    pub async fn reconcile_all(&mut self, deps: &SubscriptionDeps) -> Vec<(Bytes32, TickOutcome)> {
        let mut out = Vec::with_capacity(self.subs.len());
        for sub in self.subs.values_mut() {
            let outcome = reconcile_tick(sub, deps).await;
            out.push((sub.store_id(), outcome));
        }
        out
    }
}

/// Parse a 64-hex store id into a [`Bytes32`]. Ids in the set are already sanitized, but a
/// hand-edited file is never trusted.
fn parse_store_id(store_hex: &str) -> Option<Bytes32> {
    Bytes32::from_hex(store_hex)
        .ok()
        .filter(|_| store_hex.len() == 64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lineage::Lineage;
    use crate::seams::LineageResult;
    use async_trait::async_trait;
    use digstore_core::Capsule;
    use std::sync::Mutex;

    fn cap(store: u8, root: u8) -> Capsule {
        Capsule {
            store_id: Bytes32([store; 32]),
            root_hash: Bytes32([root; 32]),
        }
    }

    /// A chain source scripted per store id.
    struct MockChain(BTreeMap<String, LineageResult>);
    #[async_trait]
    impl ChainWatch for MockChain {
        async fn lineage(&self, store_id: &Bytes32) -> LineageResult {
            match self.0.get(&store_id.to_hex()) {
                Some(Ok(Some(l))) => Ok(Some(l.clone())),
                Some(Ok(None)) => Ok(None),
                Some(Err(e)) => Err(e.clone()),
                None => Ok(None),
            }
        }
    }

    /// A held-check over explicit (store,root) first-byte pairs.
    struct Held(Vec<(u8, u8)>);
    impl HeldCheck for Held {
        fn is_held(&self, c: &Capsule) -> bool {
            self.0.contains(&(c.store_id.0[0], c.root_hash.0[0]))
        }
    }

    /// A fetcher recording every capsule it was asked to pull, with a scripted result.
    struct RecordingFetcher {
        calls: Mutex<Vec<Capsule>>,
        fail: bool,
    }
    #[async_trait]
    impl CapsuleFetcher for RecordingFetcher {
        async fn fetch(&self, capsule: Capsule) -> Result<(), String> {
            self.calls.lock().unwrap().push(capsule);
            if self.fail {
                Err("pull failed".into())
            } else {
                Ok(())
            }
        }
    }

    fn deps(chain: MockChain, held: Held, fetcher: Arc<RecordingFetcher>) -> SubscriptionDeps {
        SubscriptionDeps {
            chain: Arc::new(chain),
            fetcher,
            held: Arc::new(held),
        }
    }

    /// **Proves the fail-closed invariant:** a chain error triggers NO fetch and reports the
    /// skip reason. **Catches:** a tick that pulls against an unconfirmable root.
    #[tokio::test]
    async fn chain_error_fetches_nothing() {
        let mut chain = BTreeMap::new();
        chain.insert(Bytes32([1; 32]).to_hex(), Err("coinset 503".into()));
        let fetcher = Arc::new(RecordingFetcher {
            calls: Mutex::new(vec![]),
            fail: false,
        });
        let d = deps(MockChain(chain), Held(vec![]), fetcher.clone());
        let mut sub = Subscription::new(Bytes32([1; 32]));

        let outcome = reconcile_tick(&mut sub, &d).await;
        assert_eq!(outcome, TickOutcome::Skipped(SkipReason::ChainError));
        assert!(
            fetcher.calls.lock().unwrap().is_empty(),
            "no fetch on chain error"
        );
    }

    /// **Proves the fail-closed invariant:** a store with no confirmed generation fetches
    /// nothing.
    #[tokio::test]
    async fn no_generation_fetches_nothing() {
        let mut chain = BTreeMap::new();
        chain.insert(Bytes32([1; 32]).to_hex(), Ok(None));
        let fetcher = Arc::new(RecordingFetcher {
            calls: Mutex::new(vec![]),
            fail: false,
        });
        let d = deps(MockChain(chain), Held(vec![]), fetcher.clone());
        let mut sub = Subscription::new(Bytes32([1; 32]));

        let outcome = reconcile_tick(&mut sub, &d).await;
        assert_eq!(
            outcome,
            TickOutcome::Skipped(SkipReason::NoConfirmedGeneration)
        );
        assert!(fetcher.calls.lock().unwrap().is_empty());
    }

    /// **Proves full-history backfill:** a confirmed 3-generation lineage with nothing held
    /// fetches ALL three `.dig` (tip first), and reports the summary.
    /// **Catches:** a watcher that only pulls the latest tip (the old single-tip behaviour).
    #[tokio::test]
    async fn confirmed_lineage_backfills_whole_history() {
        let lineage = Lineage::try_new(vec![cap(1, 10), cap(1, 20), cap(1, 30)]).unwrap();
        let mut chain = BTreeMap::new();
        chain.insert(Bytes32([1; 32]).to_hex(), Ok(Some(lineage)));
        let fetcher = Arc::new(RecordingFetcher {
            calls: Mutex::new(vec![]),
            fail: false,
        });
        let d = deps(MockChain(chain), Held(vec![]), fetcher.clone());
        let mut sub = Subscription::new(Bytes32([1; 32]));

        let outcome = reconcile_tick(&mut sub, &d).await;
        assert_eq!(
            outcome,
            TickOutcome::Synced {
                attempted: 3,
                fetched: 3
            }
        );
        assert_eq!(
            *fetcher.calls.lock().unwrap(),
            vec![cap(1, 30), cap(1, 10), cap(1, 20)],
            "tip first, then oldest→newest backfill"
        );
    }

    /// **Proves interruption-retry (best-effort):** a failed fetch is retried next tick and
    /// its attempt count accrues.
    #[tokio::test]
    async fn failed_fetch_retries_next_tick() {
        let lineage = Lineage::try_new(vec![cap(1, 10)]).unwrap();
        let mut chain = BTreeMap::new();
        chain.insert(Bytes32([1; 32]).to_hex(), Ok(Some(lineage)));
        let fetcher = Arc::new(RecordingFetcher {
            calls: Mutex::new(vec![]),
            fail: true,
        });
        let d = deps(MockChain(chain), Held(vec![]), fetcher.clone());
        let mut sub = Subscription::new(Bytes32([1; 32]));

        let t1 = reconcile_tick(&mut sub, &d).await;
        assert_eq!(
            t1,
            TickOutcome::Synced {
                attempted: 1,
                fetched: 0
            }
        );
        let t2 = reconcile_tick(&mut sub, &d).await;
        assert_eq!(
            t2,
            TickOutcome::Synced {
                attempted: 1,
                fetched: 0
            },
            "retried"
        );
        assert_eq!(fetcher.calls.lock().unwrap().len(), 2, "pulled twice");
    }

    /// **Proves:** membership syncs from the persisted set (add + drop), preserving existing
    /// subscription state.
    #[test]
    fn membership_add_and_drop() {
        let mut subs = Subscriptions::new();
        let mut set = SubscriptionSet::new();
        set.add(&Bytes32([1; 32]).to_hex()).unwrap();
        set.add(&Bytes32([2; 32]).to_hex()).unwrap();
        subs.sync_membership(&set);
        assert_eq!(subs.len(), 2);

        // Unsubscribe store 1.
        set.remove(&Bytes32([1; 32]).to_hex()).unwrap();
        subs.sync_membership(&set);
        assert_eq!(subs.len(), 1);
        assert!(subs.get(&Bytes32([2; 32]).to_hex()).is_some());
    }

    /// **Proves:** the `Subscriptions` driver reconciles a whole watched set in one pass,
    /// returning a per-store outcome (fail-closed skip AND confirmed backfill in the same tick).
    #[tokio::test]
    async fn reconcile_all_drives_the_whole_set() {
        let lineage = Lineage::try_new(vec![cap(1, 10), cap(1, 20)]).unwrap();
        let mut chain = BTreeMap::new();
        chain.insert(Bytes32([1; 32]).to_hex(), Ok(Some(lineage)));
        chain.insert(Bytes32([2; 32]).to_hex(), Err("chain down".into()));
        let fetcher = Arc::new(RecordingFetcher {
            calls: Mutex::new(vec![]),
            fail: false,
        });
        let d = deps(MockChain(chain), Held(vec![]), fetcher.clone());

        let mut subs = Subscriptions::new();
        assert!(subs.is_empty());
        let mut set = SubscriptionSet::new();
        set.add(&Bytes32([1; 32]).to_hex()).unwrap();
        set.add(&Bytes32([2; 32]).to_hex()).unwrap();
        subs.sync_membership(&set);

        let outcomes = subs.reconcile_all(&d).await;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(
            outcomes[0],
            (
                Bytes32([1; 32]),
                TickOutcome::Synced {
                    attempted: 2,
                    fetched: 2
                }
            ),
            "store 1: confirmed lineage backfilled"
        );
        assert_eq!(
            outcomes[1],
            (
                Bytes32([2; 32]),
                TickOutcome::Skipped(SkipReason::ChainError)
            ),
            "store 2: fail-closed skip"
        );
        // Status is queryable via the live set + iter.
        assert_eq!(subs.iter().count(), 2);
        let s1 = subs.get(&Bytes32([1; 32]).to_hex()).unwrap();
        assert_eq!(s1.current_tip(), Some(cap(1, 20)));
    }

    /// **Proves:** the interval env var is parsed + floored (a bogus/zero value never floods).
    #[test]
    fn interval_floor_and_default() {
        std::env::remove_var("DIG_WATCH_INTERVAL");
        assert_eq!(
            watch_interval_from_env(),
            Duration::from_secs(DEFAULT_WATCH_INTERVAL_SECS)
        );
        std::env::set_var("DIG_WATCH_INTERVAL", "0");
        assert_eq!(
            watch_interval_from_env(),
            Duration::from_secs(DEFAULT_WATCH_INTERVAL_SECS)
        );
        std::env::set_var("DIG_WATCH_INTERVAL", "5");
        assert_eq!(watch_interval_from_env(), Duration::from_secs(5));
        std::env::remove_var("DIG_WATCH_INTERVAL");
    }
}
