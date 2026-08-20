use crate::secret;
use crate::tier::Tier;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;
use webauthn_rs::prelude::*;

#[derive(Debug, thiserror::Error)]
pub enum PasskeyError {
    #[error("this origin cannot do passkeys")]
    Unavailable,
    #[error("the challenge is unknown or has expired")]
    UnknownChallenge,
    #[error(transparent)]
    Webauthn(#[from] WebauthnError),
    #[error("relying party {0} is unusable: {1}")]
    BadRelyingParty(String, WebauthnError),
    #[error(transparent)]
    Random(#[from] secret::RandomError),
}

enum Challenge {
    Register(Box<PasskeyRegistration>),
    Authenticate(Box<PasskeyAuthentication>),
}

/// Ceremony state lives in memory, never in the database and never on the wire.
///
/// A challenge is single-use and short-lived by definition, so a node restart mid-enrolment is a
/// retry rather than a bug — and persisting it would hand an attacker with file access something
/// replayable.
pub struct Passkeys {
    webauthn: Webauthn,
    rp_id: String,
    challenges: Mutex<HashMap<String, (Instant, Challenge)>>,
    ttl: Duration,
}

impl std::fmt::Debug for Passkeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Passkeys").field("rp_id", &self.rp_id).finish()
    }
}

impl Passkeys {
    /// `None` whenever the origin cannot support passkeys, which is the whole point: the caller
    /// then has nothing to offer and says so, rather than building a button that fails at the
    /// last step.
    pub fn for_tier(tier: &Tier, ttl: Duration) -> Result<Option<Self>, PasskeyError> {
        let Some(rp_id) = tier.rp_id.clone() else {
            return Ok(None);
        };
        let origin = Url::parse(&tier.origin).map_err(|_| PasskeyError::Unavailable)?;
        let webauthn = WebauthnBuilder::new(&rp_id, &origin)
            .and_then(|b| b.rp_name("Kampr").build())
            .map_err(|e| PasskeyError::BadRelyingParty(rp_id.clone(), e))?;
        Ok(Some(Self {
            webauthn,
            rp_id,
            challenges: Mutex::new(HashMap::new()),
            ttl,
        }))
    }

    pub fn rp_id(&self) -> &str {
        &self.rp_id
    }

    pub fn start_registration(
        &self,
        user_id: Uuid,
        name: &str,
        existing: &[Passkey],
    ) -> Result<(String, CreationChallengeResponse), PasskeyError> {
        let exclude = existing.iter().map(|p| p.cred_id().clone()).collect::<Vec<_>>();
        let (challenge, state) =
            self.webauthn
                .start_passkey_registration(user_id, name, name, Some(exclude))?;
        let id = self.park(Challenge::Register(Box::new(state)))?;
        Ok((id, challenge))
    }

    pub fn finish_registration(
        &self,
        challenge_id: &str,
        credential: &RegisterPublicKeyCredential,
    ) -> Result<Passkey, PasskeyError> {
        let Some(Challenge::Register(state)) = self.take(challenge_id) else {
            return Err(PasskeyError::UnknownChallenge);
        };
        Ok(self.webauthn.finish_passkey_registration(credential, &state)?)
    }

    pub fn start_authentication(
        &self,
        credentials: &[Passkey],
    ) -> Result<(String, RequestChallengeResponse), PasskeyError> {
        let (challenge, state) = self.webauthn.start_passkey_authentication(credentials)?;
        let id = self.park(Challenge::Authenticate(Box::new(state)))?;
        Ok((id, challenge))
    }

    pub fn finish_authentication(
        &self,
        challenge_id: &str,
        credential: &PublicKeyCredential,
    ) -> Result<AuthenticationResult, PasskeyError> {
        let Some(Challenge::Authenticate(state)) = self.take(challenge_id) else {
            return Err(PasskeyError::UnknownChallenge);
        };
        Ok(self.webauthn.finish_passkey_authentication(credential, &state)?)
    }

    fn park(&self, challenge: Challenge) -> Result<String, PasskeyError> {
        let id = hex::encode(secret::random_bytes(16)?);
        let now = Instant::now();
        let mut challenges = self.challenges.lock().unwrap();
        challenges.retain(|_, (at, _)| now.duration_since(*at) < self.ttl);
        challenges.insert(id.clone(), (now, challenge));
        Ok(id)
    }

    fn take(&self, id: &str) -> Option<Challenge> {
        let now = Instant::now();
        let (at, challenge) = self.challenges.lock().unwrap().remove(id)?;
        (now.duration_since(at) < self.ttl).then_some(challenge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ip_origin_yields_no_passkey_engine_at_all() {
        let tier = Tier::detect("https://192.168.1.24:8790").unwrap();
        assert!(
            Passkeys::for_tier(&tier, Duration::from_secs(300))
                .unwrap()
                .is_none(),
            "an IP is not a registrable domain, and HTTPS does not change that"
        );
    }

    #[test]
    fn a_hostname_origin_yields_an_engine_bound_to_that_hostname() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_secs(300))
            .unwrap()
            .unwrap();
        assert_eq!(pk.rp_id(), "kampr.example.com");
    }

    #[test]
    fn a_registration_challenge_is_single_use() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_secs(300))
            .unwrap()
            .unwrap();
        let (id, challenge) = pk.start_registration(Uuid::new_v4(), "phone", &[]).unwrap();
        assert!(!challenge.public_key.challenge.is_empty());
        assert!(pk.take(&id).is_some());
        assert!(pk.take(&id).is_none());
    }

    #[test]
    fn an_expired_challenge_is_refused() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_nanos(1))
            .unwrap()
            .unwrap();
        let (id, _) = pk.start_registration(Uuid::new_v4(), "phone", &[]).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        assert!(pk.take(&id).is_none());
    }
}
