use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use kampr_auth::secret::{RandomError, random_bytes};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A recording can run for hours and its player asks for a range on every seek, so the lifetime
/// is long. It only bounds a leaked URL: revocation is the device's token checked again on every
/// request, never this.
pub const TTL: Duration = Duration::from_secs(12 * 60 * 60);
pub const CAP: usize = 512;

pub struct Grant {
    pub token: String,
    pub locator: String,
}

struct Entry {
    grant: Grant,
    minted: Instant,
}

/// Memory only: a node that restarts forgets every ticket, and a player asks for a fresh one.
pub struct StreamTickets {
    ttl: Duration,
    cap: usize,
    live: Mutex<HashMap<String, Entry>>,
}

impl Default for StreamTickets {
    fn default() -> Self {
        Self::new(TTL, CAP)
    }
}

impl StreamTickets {
    pub fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            ttl,
            cap: cap.max(1),
            live: Mutex::new(HashMap::new()),
        }
    }

    pub fn mint(&self, token: &str, locator: &str) -> Result<String, RandomError> {
        self.mint_at(token, locator, Instant::now())
    }

    pub fn redeem(&self, ticket: &str) -> Option<Grant> {
        self.redeem_at(ticket, Instant::now())
    }

    pub fn mint_at(&self, token: &str, locator: &str, now: Instant) -> Result<String, RandomError> {
        let ticket = URL_SAFE_NO_PAD.encode(random_bytes(32)?);
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if live.len() >= self.cap {
            live.retain(|_, e| now.duration_since(e.minted) < self.ttl);
        }
        while live.len() >= self.cap {
            let oldest = live.iter().min_by_key(|(_, e)| e.minted).map(|(k, _)| k.clone());
            match oldest {
                Some(oldest) => live.remove(&oldest),
                None => break,
            };
        }
        live.insert(
            ticket.clone(),
            Entry {
                grant: Grant {
                    token: token.to_string(),
                    locator: locator.to_string(),
                },
                minted: now,
            },
        );
        Ok(ticket)
    }

    pub fn redeem_at(&self, ticket: &str, now: Instant) -> Option<Grant> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let entry = live.get(ticket)?;
        if now.duration_since(entry.minted) >= self.ttl {
            live.remove(ticket);
            return None;
        }
        Some(Grant {
            token: entry.grant.token.clone(),
            locator: entry.grant.locator.clone(),
        })
    }

    pub fn len(&self) -> usize {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_is_256_bits_of_url_safe_text_and_names_what_it_was_minted_for() {
        let tickets = StreamTickets::default();
        let ticket = tickets.mint("kmp_token", "n/w1:p1/id").expect("a ticket");
        assert_eq!(URL_SAFE_NO_PAD.decode(&ticket).expect("base64url").len(), 32);
        assert!(!ticket.starts_with("kmp_"));
        let grant = tickets.redeem(&ticket).expect("live");
        assert_eq!(
            (grant.token.as_str(), grant.locator.as_str()),
            ("kmp_token", "n/w1:p1/id")
        );
        assert!(
            tickets.redeem(&ticket).is_some(),
            "a player redeems it on every range request"
        );
        assert_ne!(tickets.mint("kmp_token", "n/w1:p1/id").expect("another"), ticket);
    }

    #[test]
    fn a_ticket_dies_at_its_ttl_and_is_dropped_when_it_does() {
        let tickets = StreamTickets::new(Duration::from_secs(60), CAP);
        let t0 = Instant::now();
        let ticket = tickets.mint_at("tok", "loc", t0).expect("a ticket");
        assert!(tickets.redeem_at(&ticket, t0 + Duration::from_secs(59)).is_some());
        assert!(tickets.redeem_at(&ticket, t0 + Duration::from_secs(60)).is_none());
        assert!(tickets.is_empty());
    }

    #[test]
    fn at_the_cap_the_expired_go_first_and_then_the_oldest() {
        let tickets = StreamTickets::new(Duration::from_secs(60), 3);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let expired = tickets.mint_at("tok", "a", at(0)).expect("a");
        let oldest_live = tickets.mint_at("tok", "b", at(30)).expect("b");
        let newer = tickets.mint_at("tok", "c", at(40)).expect("c");

        let d = tickets.mint_at("tok", "d", at(70)).expect("d");
        assert_eq!(tickets.len(), 3);
        assert!(
            tickets.redeem_at(&expired, at(0)).is_none(),
            "the expired one went"
        );
        assert!(tickets.redeem_at(&oldest_live, at(70)).is_some());

        let e = tickets.mint_at("tok", "e", at(71)).expect("e");
        assert_eq!(tickets.len(), 3);
        assert!(
            tickets.redeem_at(&oldest_live, at(71)).is_none(),
            "then the oldest"
        );
        for kept in [&newer, &d, &e] {
            assert!(tickets.redeem_at(kept, at(71)).is_some());
        }
    }
}
