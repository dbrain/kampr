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
        d
    }

    pub fn reset(&mut self) {
        self.next = self.policy.initial;
    }

    pub async fn sleep(&mut self) {
        tokio::time::sleep(self.next_delay()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_until_the_ceiling_then_holds() {
        let mut b = Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(400),
        }
        .start();
        let seen: Vec<u64> = (0..5).map(|_| b.next_delay().as_millis() as u64).collect();
        assert_eq!(seen, [100, 200, 400, 400, 400]);
        b.reset();
        assert_eq!(b.next_delay().as_millis(), 100);
    }
}
