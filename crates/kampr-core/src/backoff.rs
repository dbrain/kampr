use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(200),
            max: Duration::from_secs(10),
        }
    }
}

impl Backoff {
    pub fn start(self) -> BackoffState {
        BackoffState {
            policy: self,
            next: self.initial,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BackoffState {
    policy: Backoff,
    next: Duration,
}

impl BackoffState {
    pub fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = (self.next * 2).min(self.policy.max);
        jitter(d)
    }

    pub fn reset(&mut self) {
        self.next = self.policy.initial;
    }

    pub async fn sleep(&mut self) {
        tokio::time::sleep(self.next_delay()).await;
    }
}

/// A restarted hub is re-dialled by every one of its peers at once, and a schedule with no
/// spread has them all arrive on the same tick for as long as the outage lasts.
fn jitter(d: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(d.as_nanos() as u64);
    let spread = (hasher.finish() % (2 * SPREAD_PER_MILLE + 1)) as i128 - SPREAD_PER_MILLE as i128;
    Duration::from_nanos((d.as_nanos() as i128 * (1000 + spread) / 1000) as u64)
}

const SPREAD_PER_MILLE: u64 = 200;

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Backoff {
        Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(400),
        }
    }

    #[test]
    fn doubles_until_the_ceiling_then_holds() {
        let mut b = policy().start();
        let schedule = [100u64, 200, 400, 400, 400];
        for want in schedule {
            let got = b.next_delay().as_millis() as u64;
            assert!(
                (want * 8 / 10..=want * 12 / 10).contains(&got),
                "{got}ms is not {want}ms give or take a fifth"
            );
        }
        b.reset();
        let got = b.next_delay().as_millis() as u64;
        assert!((80..=120).contains(&got), "{got}ms after a reset");
    }

    #[test]
    fn two_peers_dialling_the_same_hub_do_not_arrive_on_the_same_tick() {
        let delays: Vec<Duration> = (0..20).map(|_| policy().start().next_delay()).collect();
        let spread = delays.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(spread > 1, "twenty peers all waited {:?}", delays[0]);
    }
}
