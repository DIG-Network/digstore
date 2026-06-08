use digstore_core::Bytes32;
use std::collections::HashMap;
use std::sync::Mutex;

/// Simple per-store token bucket. Deterministic: `refill` adds tokens up to
/// capacity; `try_acquire` consumes one token, returning false (=> 429) when
/// the bucket is empty.
pub struct RateLimiter {
    capacity: u32,
    buckets: Mutex<HashMap<Bytes32, u32>>,
}

impl RateLimiter {
    pub fn new(capacity: u32) -> Self {
        RateLimiter {
            capacity,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Attempt to consume one token for a store. False => rate limited.
    pub fn try_acquire(&self, store_id: &Bytes32) -> bool {
        let mut b = self.buckets.lock().unwrap();
        let tokens = b.entry(*store_id).or_insert(self.capacity);
        if *tokens == 0 {
            false
        } else {
            *tokens -= 1;
            true
        }
    }

    /// Refill a store's bucket to capacity (called on a timer in production).
    pub fn refill(&self, store_id: &Bytes32) {
        let mut b = self.buckets.lock().unwrap();
        b.insert(*store_id, self.capacity);
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
        let rl = RateLimiter::new(2);
        let id = b32(1);
        assert!(rl.try_acquire(&id));
        assert!(rl.try_acquire(&id));
        assert!(!rl.try_acquire(&id), "third call must be rate limited (429)");
    }

    #[test]
    fn refill_restores_budget() {
        let rl = RateLimiter::new(1);
        let id = b32(2);
        assert!(rl.try_acquire(&id));
        assert!(!rl.try_acquire(&id));
        rl.refill(&id);
        assert!(rl.try_acquire(&id));
    }

    #[test]
    fn buckets_are_per_store() {
        let rl = RateLimiter::new(1);
        assert!(rl.try_acquire(&b32(1)));
        assert!(rl.try_acquire(&b32(2)), "different store has its own bucket");
    }
}
