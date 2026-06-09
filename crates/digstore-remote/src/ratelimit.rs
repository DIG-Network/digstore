use digstore_core::Bytes32;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default window over which a drained bucket refills back to full capacity.
const DEFAULT_REFILL_WINDOW: Duration = Duration::from_secs(60);

/// Hard cap on the number of per-store buckets tracked simultaneously. Bounds
/// memory so an attacker cannot exhaust host memory by hitting a flood of
/// distinct (and possibly nonexistent) store ids. When the cap is reached, idle
/// (fully refilled) buckets are evicted; they cost nothing to recreate at full.
const MAX_BUCKETS: usize = 100_000;

struct Bucket {
    /// Fractional tokens available (continuous so partial refills accrue).
    tokens: f64,
    /// Last time tokens were recomputed.
    last: Instant,
}

/// Per-store token bucket with **time-based refill**. Each `try_acquire` first
/// credits the bucket with `capacity * elapsed / refill_window` tokens (capped at
/// `capacity`), then consumes one. A drained store recovers automatically over
/// `refill_window`, so a one-time request burst can no longer lock a store out
/// permanently. The bucket map is bounded by [`MAX_BUCKETS`].
pub struct RateLimiter {
    capacity: u32,
    /// Tokens regenerated per second.
    refill_per_sec: f64,
    buckets: Mutex<HashMap<Bytes32, Bucket>>,
}

impl RateLimiter {
    /// New limiter holding `capacity` tokens, refilling fully over the default
    /// 60-second window.
    pub fn new(capacity: u32) -> Self {
        Self::with_window(capacity, DEFAULT_REFILL_WINDOW)
    }

    /// New limiter with an explicit refill window (the time for an empty bucket
    /// to return to `capacity`).
    pub fn with_window(capacity: u32, window: Duration) -> Self {
        let secs = window.as_secs_f64().max(f64::MIN_POSITIVE);
        RateLimiter {
            capacity,
            refill_per_sec: capacity as f64 / secs,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Attempt to consume one token for a store. False => rate limited (429).
    pub fn try_acquire(&self, store_id: &Bytes32) -> bool {
        let now = Instant::now();
        let mut b = self.buckets.lock().unwrap();

        if !b.contains_key(store_id) && b.len() >= MAX_BUCKETS {
            self.evict_idle(&mut b);
        }

        let cap = self.capacity as f64;
        let bucket = b.entry(*store_id).or_insert(Bucket {
            tokens: cap,
            last: now,
        });
        // Credit elapsed time, capped at capacity.
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(cap);
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Refill a store's bucket to capacity immediately.
    pub fn refill(&self, store_id: &Bytes32) {
        let mut b = self.buckets.lock().unwrap();
        b.insert(
            *store_id,
            Bucket {
                tokens: self.capacity as f64,
                last: Instant::now(),
            },
        );
    }

    /// Drop buckets that are at (or above) full capacity after crediting elapsed
    /// time — these are indistinguishable from a freshly created bucket, so
    /// evicting them is lossless and reclaims memory under flood conditions.
    fn evict_idle(&self, b: &mut HashMap<Bytes32, Bucket>) {
        let now = Instant::now();
        let cap = self.capacity as f64;
        b.retain(|_, bucket| {
            let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
            let tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(cap);
            tokens < cap
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn b32(x: u8) -> Bytes32 {
        Bytes32([x; 32])
    }

    #[test]
    fn allows_up_to_capacity_then_limits() {
        // Long window so no measurable refill occurs during the test.
        let rl = RateLimiter::with_window(2, Duration::from_secs(3600));
        let id = b32(1);
        assert!(rl.try_acquire(&id));
        assert!(rl.try_acquire(&id));
        assert!(!rl.try_acquire(&id), "third call must be rate limited (429)");
    }

    #[test]
    fn refill_restores_budget() {
        let rl = RateLimiter::with_window(1, Duration::from_secs(3600));
        let id = b32(2);
        assert!(rl.try_acquire(&id));
        assert!(!rl.try_acquire(&id));
        rl.refill(&id);
        assert!(rl.try_acquire(&id));
    }

    #[test]
    fn buckets_are_per_store() {
        let rl = RateLimiter::with_window(1, Duration::from_secs(3600));
        assert!(rl.try_acquire(&b32(1)));
        assert!(rl.try_acquire(&b32(2)), "different store has its own bucket");
    }

    #[test]
    fn time_based_refill_recovers_a_drained_bucket() {
        // 100 tokens per second => one token every 10ms.
        let rl = RateLimiter::with_window(10, Duration::from_millis(100));
        let id = b32(3);
        for _ in 0..10 {
            assert!(rl.try_acquire(&id));
        }
        assert!(!rl.try_acquire(&id), "drained");
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            rl.try_acquire(&id),
            "tokens must regenerate over time (no permanent lockout)"
        );
    }
}
