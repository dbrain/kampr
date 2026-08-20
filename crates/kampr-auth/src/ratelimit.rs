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

/// A pairing code is short enough to guess if you are allowed to guess often, so the limiter is
/// part of the credential, not decoration around it.
#[derive(Debug)]
pub struct RateLimiter {
    policy: Policy,
    buckets: Mutex<HashMap<String, Bucket>>,
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
        }
    }

    pub fn check(&self, key: &str) -> bool {
        self.check_at(key, Instant::now())
    }

    pub fn check_at(&self, key: &str, now: Instant) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        if buckets.len() > 4096 {
            buckets.retain(|_, b| now.duration_since(b.seen) < Duration::from_secs(300));
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
    fn keys_do_not_share_a_bucket() {
        let l = RateLimiter::new(Policy::new(1.0, 0.0));
        let t = Instant::now();
        assert!(l.check_at("a", t));
        assert!(l.check_at("b", t));
        assert!(!l.check_at("a", t));
    }
}
