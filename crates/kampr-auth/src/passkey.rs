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

/// Ceremony state a node holds at once. `/auth/webauthn/authenticate/start` is unauthenticated
/// and parks the whole enrolled-credential list for the challenge TTL, so without a ceiling the
/// only thing between that and unbounded memory is the per-peer limiter — and peers are cheap.
pub const MAX_CHALLENGES: usize = 128;

/// Which authenticator API is going to be handed this challenge. Not a fingerprint of the
/// requester — the client says which it is, and the only thing it can choose is which of two
/// ceremonies it is asked to perform. Neither is verified any differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Client {
    #[default]
    Browser,
    CredentialManager,
}

impl Client {
    /// What `platform` on `/auth/webauthn/register/start` means. Anything unrecognised is a
    /// browser, because that is the ceremony every other client has ever completed.
    pub fn from_platform(platform: Option<&str>) -> Self {
        match platform {
            Some("android") => Self::CredentialManager,
            _ => Self::Browser,
        }
    }
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
    ///
    /// `android` is the signing certificates of the apps this node's `assetlinks.json` delegates
    /// to. Credential Manager signs `android:apk-key-hash:…` rather than an `https://` origin, so
    /// an engine that has not been told about them refuses a native ceremony at the last step.
    /// Unreadable entries widen nothing.
    pub fn for_tier(tier: &Tier, ttl: Duration, android: &[String]) -> Result<Option<Self>, PasskeyError> {
        let Some(rp_id) = tier.rp_id.clone() else {
            return Ok(None);
        };
        let origin = Url::parse(&tier.origin).map_err(|_| PasskeyError::Unavailable)?;
        let apps: Vec<Url> = android
            .iter()
            .filter_map(|f| crate::android::credential_manager_origin(f))
            .filter_map(|o| Url::parse(&o).ok())
            .collect();
        let webauthn = WebauthnBuilder::new(&rp_id, &origin)
            .map(|b| apps.iter().fold(b, |b, app| b.append_allowed_origin(app)))
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

    /// Test-visible: an origin this engine does not hold is a ceremony that fails after the
    /// phone has already asked its owner to approve it.
    pub fn allowed_origins(&self) -> Vec<String> {
        self.webauthn
            .get_allowed_origins()
            .iter()
            .map(Url::to_string)
            .collect()
    }

    /// The option set is the caller's, because the two callers cannot use the same one. A browser
    /// gets `webauthn-rs`'s general passkey ceremony, which is what the web client was verified
    /// against. Credential Manager cannot satisfy that ceremony — GMS performs no authenticator
    /// selection and has no answer for `credProtect` — so a phone gets the discoverable platform
    /// credential the crate ships for exactly this reason.
    pub fn start_registration(
        &self,
        user_id: Uuid,
        name: &str,
        existing: &[Passkey],
        client: Client,
    ) -> Result<(String, CreationChallengeResponse), PasskeyError> {
        let exclude = Some(existing.iter().map(|p| p.cred_id().clone()).collect::<Vec<_>>());
        let (challenge, state) = match client {
            Client::Browser => self
                .webauthn
                .start_passkey_registration(user_id, name, name, exclude)?,
            Client::CredentialManager => self
                .webauthn
                .start_google_passkey_in_google_password_manager_only_registration(
                    user_id, name, name, exclude,
                )?,
        };
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
        while challenges.len() >= MAX_CHALLENGES {
            let Some(oldest) = challenges
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            challenges.remove(&oldest);
        }
        challenges.insert(id.clone(), (now, challenge));
        Ok(id)
    }

    pub fn parked(&self) -> usize {
        self.challenges.lock().unwrap().len()
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
            Passkeys::for_tier(&tier, Duration::from_secs(300), &[])
                .unwrap()
                .is_none(),
            "an IP is not a registrable domain, and HTTPS does not change that"
        );
    }

    /// The half of Android support that is not the `assetlinks.json` file. Credential Manager
    /// signs `android:apk-key-hash:…` into the client data, and an engine that does not allow it
    /// refuses the ceremony *after* the phone has already asked its owner to approve it.
    #[test]
    fn a_phone_app_is_an_allowed_origin_when_its_certificate_is_configured() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let fingerprint =
            "8B:BF:39:60:61:89:30:A4:45:F3:D7:09:1E:7B:1B:05:0F:8A:FD:AF:24:EB:F1:EB:2E:3D:13:88:09:FC:79:59";
        let pk = Passkeys::for_tier(&tier, Duration::from_secs(300), &[fingerprint.to_string()])
            .unwrap()
            .unwrap();
        let origins = pk.allowed_origins();
        assert!(
            origins.iter().any(|o| o == "https://kampr.example.com/"),
            "the browser must still work: {origins:?}"
        );
        assert!(
            origins
                .iter()
                .any(|o| o == "android:apk-key-hash:i785YGGJMKRF89cJHnsbBQ-K_a8k6_HrLj0TiAn8eVk"),
            "the app the node delegates to has to be allowed to speak: {origins:?}"
        );
    }

    #[test]
    fn an_app_whose_certificate_is_unreadable_is_no_origin_at_all() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(
            &tier,
            Duration::from_secs(300),
            &["not-a-fingerprint".to_string()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            pk.allowed_origins(),
            vec!["https://kampr.example.com/".to_string()],
            "junk in the config widens nothing"
        );
    }

    #[test]
    fn a_hostname_origin_yields_an_engine_bound_to_that_hostname() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_secs(300), &[])
            .unwrap()
            .unwrap();
        assert_eq!(pk.rp_id(), "kampr.example.com");
    }

    #[test]
    fn a_registration_challenge_is_single_use() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_secs(300), &[])
            .unwrap()
            .unwrap();
        let (id, challenge) = pk
            .start_registration(Uuid::new_v4(), "phone", &[], Client::Browser)
            .unwrap();
        assert!(!challenge.public_key.challenge.is_empty());
        assert!(pk.take(&id).is_some());
        assert!(pk.take(&id).is_none());
    }

    #[test]
    fn parked_ceremony_state_is_capped() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_secs(300), &[])
            .unwrap()
            .unwrap();
        for _ in 0..(MAX_CHALLENGES * 2) {
            pk.start_registration(Uuid::new_v4(), "phone", &[], Client::Browser)
                .unwrap();
        }
        assert!(
            pk.parked() <= MAX_CHALLENGES,
            "an unauthenticated request must not park state for five minutes without a ceiling"
        );
    }

    #[test]
    fn an_expired_challenge_is_refused() {
        let tier = Tier::detect("https://kampr.example.com").unwrap();
        let pk = Passkeys::for_tier(&tier, Duration::from_nanos(1), &[])
            .unwrap()
            .unwrap();
        let (id, _) = pk
            .start_registration(Uuid::new_v4(), "phone", &[], Client::Browser)
            .unwrap();
        std::thread::sleep(Duration::from_millis(2));
        assert!(pk.take(&id).is_none());
    }
}
