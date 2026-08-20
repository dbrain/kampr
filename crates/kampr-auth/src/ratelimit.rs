use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub burst: f64,
    pub per_second: f64,
}

impl Policy {
    pub const fn new(burst: f64, per_second: f64) -> Self {
        Self { burst, per_second }
    }
}

/// Buckets kept at once. A key-rotating attacker must not be able to grow this, and the sweep is
/// O(n), so the cap is what stops the map from making every request more expensive than the last.
pub const MAX_BUCKETS: usize = 4096;

const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// A pairing code is short enough to guess if you are allowed to guess often, so the limiter is
/// part of the credential, not decoration around it.
#[derive(Debug)]
pub struct RateLimiter {
    policy: Policy,
    buckets: Mutex<HashMap<String, Bucket>>,
    swept: Mutex<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    seen: Instant,
}

impl RateLimiter {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            buckets: Mutex::new(HashMap::new()),
            swept: Mutex::new(Instant::now()),
        }
    }

    pub fn check(&self, key: &str) -> bool {
        self.check_at(key, Instant::now())
    }

    pub fn check_at(&self, key: &str, now: Instant) -> bool {
        self.maybe_sweep(now);
        let mut buckets = self.buckets.lock().unwrap();
        if buckets.len() >= MAX_BUCKETS && !buckets.contains_key(key) {
            evict_oldest(&mut buckets);
        }
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: self.policy.burst,
            seen: now,
        });
        let elapsed = now.duration_since(bucket.seen).as_secs_f64();
        bucket.seen = now;
        bucket.tokens = (bucket.tokens + elapsed * self.policy.per_second).min(self.policy.burst);
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    pub fn forget(&self, key: &str) {
        self.buckets.lock().unwrap().remove(key);
    }

    pub fn len(&self) -> usize {
        self.buckets.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn maybe_sweep(&self, now: Instant) {
        let mut swept = self.swept.lock().unwrap();
        if now.duration_since(*swept) < SWEEP_EVERY {
            return;
        }
        *swept = now;
        drop(swept);
        self.sweep_at(now);
    }

    /// Drops every bucket that has refilled to full, because such a bucket is indistinguishable
    /// from a key that was never seen.
    pub fn sweep_at(&self, now: Instant) {
        let mut buckets = self.buckets.lock().unwrap();
        let burst = self.policy.burst;
        let per_second = self.policy.per_second;
        buckets.retain(|_, b| {
            let refilled = b.tokens + now.duration_since(b.seen).as_secs_f64() * per_second;
            refilled < burst
        });
    }
}

fn evict_oldest(buckets: &mut HashMap<String, Bucket>) {
    if let Some(key) = buckets.iter().min_by_key(|(_, b)| b.seen).map(|(k, _)| k.clone()) {
        buckets.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_is_allowed_and_then_it_is_not() {
        let l = RateLimiter::new(Policy::new(3.0, 1.0));
        let t = Instant::now();
        assert_eq!((0..5).filter(|_| l.check_at("1.2.3.4", t)).count(), 3);
    }

    #[test]
    fn tokens_refill_with_time() {
        let l = RateLimiter::new(Policy::new(2.0, 1.0));
        let t = Instant::now();
        assert!(l.check_at("k", t) && l.check_at("k", t));
        assert!(!l.check_at("k", t));
        assert!(l.check_at("k", t + Duration::from_secs(1)));
    }

    #[test]
    fn a_key_rotating_attacker_cannot_grow_the_map_without_bound() {
        let l = RateLimiter::new(Policy::new(3.0, 1.0));
        let t = Instant::now();
        for n in 0..(MAX_BUCKETS * 3) {
            l.check_at(&format!("10.0.{}.{}", n / 256, n % 256), t);
        }
        assert!(
            l.len() <= MAX_BUCKETS,
            "a fresh key per request grew the map to {}",
            l.len()
        );
    }

    #[test]
    fn a_bucket_that_has_refilled_is_forgotten_rather_than_kept() {
        let l = RateLimiter::new(Policy::new(2.0, 1.0));
        let t = Instant::now();
        assert!(l.check_at("idle", t));
        assert!(l.check_at("busy", t) && l.check_at("busy", t));
        assert!(!l.check_at("busy", t));
        // Far enough ahead that "idle" is indistinguishable from a key never seen.
        l.sweep_at(t + Duration::from_secs(60));
        assert_eq!(l.len(), 0);
    }

    #[test]
    fn keys_do_not_share_a_bucket() {
        let l = RateLimiter::new(Policy::new(1.0, 0.0));
        let t = Instant::now();
        assert!(l.check_at("a", t));
        assert!(l.check_at("b", t));
        assert!(!l.check_at("a", t));
    }
}
