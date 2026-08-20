//! Authentication for a node that can type into every terminal on the host.
//!
//! The Herdr socket is filesystem-permission-scoped to one uid and that is its entire security
//! model, so anything reaching Kampr has unrestricted code execution on the machine. Two things
//! follow. Auth is a credential, not a header. And it cannot be one thing: a WebAuthn RP ID must
//! be a registrable domain, an IP address is not one, and HTTPS does not change that — so
//! [`Tier`] decides what the configured origin can actually support and the surface hides the
//! rest.

pub mod audit;
pub mod files;
pub mod identity;
pub mod passkey;
pub mod ratelimit;
pub mod secret;
pub mod store;
pub mod tier;

pub use audit::{AuditLog, Entry};
pub use files::private_dir;
pub use identity::{IdentityError, NodeIdentity};
pub use passkey::{PasskeyError, Passkeys};
pub use ratelimit::{Policy as RatePolicy, RateLimiter};
pub use store::{Credential, Device, Role, Store, StoreError};
pub use tier::{Tier, TierError};

use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::broadcast;
use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, Passkey, PublicKeyCredential,
    RegisterPublicKeyCredential, RequestChallengeResponse, Uuid,
};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("too many attempts")]
    RateLimited,
    #[error("that pairing code is not valid")]
    BadPairingCode,
    #[error("this device is not authorised")]
    Unauthorized,
    #[error("no passkey is enrolled for this credential")]
    UnknownCredential,
    #[error(transparent)]
    Passkey(#[from] PasskeyError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub pairing_ttl: Duration,
    /// How long an operator's keypress keeps pairing open.
    pub arming_window: Duration,
    /// How long a Tier 0 bearer token lives. Cleartext access to a machine that can type into
    /// every terminal on it should require a deliberate decision to keep going, so this expires
    /// on purpose. `None` disables the expiry — a choice, never a default.
    pub tier0_token_ttl: Option<Duration>,
    pub challenge_ttl: Duration,
    pub pairing_attempts: RatePolicy,
    pub auth_attempts: RatePolicy,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            pairing_ttl: Duration::from_secs(600),
            arming_window: Duration::from_secs(60),
            tier0_token_ttl: Some(Duration::from_secs(30 * 86_400)),
            challenge_ttl: Duration::from_secs(300),
            pairing_attempts: RatePolicy::new(5.0, 0.05),
            auth_attempts: RatePolicy::new(20.0, 0.5),
        }
    }
}

/// How a pairing code reaches the person who will type it.
///
/// This is the whole of the arming decision: a code printed into a Herdr pane is read by every
/// read-only device on the node, and a code handed back over an authenticated request is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Console,
    Authenticated,
}

/// A freshly minted pairing code, and whether it is redeemable as it stands.
///
/// `armed` is false whenever something could already be watching the screen it is printed on:
/// `kampr setup` runs as a Herdr popup pane and a read-only device receives every frame of every
/// pane, so once any device is enrolled the code needs a keypress at the console behind it.
#[derive(Debug, Clone)]
pub struct Pairing {
    pub code: String,
    pub armed: bool,
}

#[derive(Debug, Clone)]
pub struct Enrolment {
    pub device: Device,
    /// Shown once. Only its digest is stored, so it cannot be recovered.
    pub token: String,
}

pub struct Auth {
    store: Store,
    tier: Tier,
    audit: AuditLog,
    passkeys: Option<Arc<Passkeys>>,
    policy: Policy,
    pairing_limiter: RateLimiter,
    auth_limiter: RateLimiter,
    changes: broadcast::Sender<String>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("tier", &self.tier)
            .finish_non_exhaustive()
    }
}

pub fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

impl Auth {
    pub fn new(store: Store, tier: Tier, audit: AuditLog, policy: Policy) -> Result<Self, AuthError> {
        let passkeys = Passkeys::for_tier(&tier, policy.challenge_ttl)?.map(Arc::new);
        Ok(Self {
            store,
            tier,
            audit,
            passkeys,
            policy,
            pairing_limiter: RateLimiter::new(policy.pairing_attempts),
            auth_limiter: RateLimiter::new(policy.auth_attempts),
            changes: broadcast::channel(64).0,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Device ids whose access just changed. A live session watches this so a revocation reaches
    /// the socket it is holding; the same session also re-reads the row on a timer, because
    /// `kampr setup` revokes from a different process and there is no channel between them.
    pub fn device_changes(&self) -> broadcast::Receiver<String> {
        self.changes.subscribe()
    }

    pub fn tier(&self) -> &Tier {
        &self.tier
    }

    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    pub fn passkeys(&self) -> Option<&Arc<Passkeys>> {
        self.passkeys.as_ref()
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// When a device's access runs out. Passkey-backed devices are not on a timer — the
    /// credential is phishing-resistant and revocable — but a Tier 0 bearer token is.
    fn expiry(&self, now: i64) -> Option<i64> {
        if self.tier.passkeys {
            return None;
        }
        self.policy.tier0_token_ttl.map(|ttl| now + ttl.as_secs() as i64)
    }

    pub async fn create_pairing(&self, role: Role, delivery: Delivery) -> Result<Pairing, AuthError> {
        let now = now();
        self.store.expire_pairings(now).await?;
        let expires_at = now + self.policy.pairing_ttl.as_secs() as i64;
        // Before the first device is enrolled nothing can be watching the pane the code prints
        // into, so the code stands on its own. After that it does not.
        let armed =
            delivery == Delivery::Authenticated || !self.store.devices().await?.iter().any(|d| d.active(now));
        let code = self
            .store
            .create_pairing(role, now, expires_at, armed.then_some(expires_at))
            .await?;
        self.audit.record(
            &Entry::new("pairing.created")
                .detail(serde_json::json!({ "role": role.as_str(), "armed": armed })),
        );
        Ok(Pairing { code, armed })
    }

    /// Opens the redemption window. Called only from a process with a console — the operator's
    /// keypress is the out-of-band channel a device watching the screen does not have.
    pub async fn arm_pairing(&self, code: &str) -> Result<bool, AuthError> {
        let now = now();
        let until = now + self.policy.arming_window.as_secs() as i64;
        let done = self.store.arm_pairing(code, now, until).await?;
        if done {
            self.audit.record(
                &Entry::new("pairing.armed")
                    .detail(serde_json::json!({ "seconds": self.policy.arming_window.as_secs() })),
            );
        }
        Ok(done)
    }

    /// A pairing code is short enough to guess if you may guess often, so the limiter is part of
    /// the credential rather than decoration around it.
    pub async fn redeem_pairing(
        &self,
        code: &str,
        device_name: &str,
        user_agent: Option<&str>,
        peer: &str,
    ) -> Result<Enrolment, AuthError> {
        if !self.pairing_limiter.check(peer) {
            self.audit.record(&Entry::new("pairing.rate_limited").peer(peer));
            return Err(AuthError::RateLimited);
        }
        let now = now();
        let Some(role) = self.store.claim_pairing(code, now).await? else {
            self.audit.record(&Entry::new("pairing.rejected").peer(peer));
            return Err(AuthError::BadPairingCode);
        };
        self.pairing_limiter.forget(peer);
        let enrolment = self.enrol(device_name, role, user_agent, now).await?;
        self.audit.record(
            &Entry::new("pairing.redeemed")
                .device(
                    &enrolment.device.id,
                    &enrolment.device.name,
                    enrolment.device.role.as_str(),
                )
                .peer(peer),
        );
        Ok(enrolment)
    }

    async fn enrol(
        &self,
        device_name: &str,
        role: Role,
        user_agent: Option<&str>,
        now: i64,
    ) -> Result<Enrolment, AuthError> {
        let expires_at = self.expiry(now);
        let device = self
            .store
            .create_device(
                device_name,
                role,
                now,
                expires_at,
                user_agent,
                Some(&self.tier.origin),
            )
            .await?;
        let token = self.store.mint_token(&device.id, now, expires_at).await?;
        Ok(Enrolment { device, token })
    }

    pub async fn authenticate(&self, token: &str, peer: &str) -> Result<Device, AuthError> {
        if !self.auth_limiter.check(peer) {
            self.audit.record(&Entry::new("auth.rate_limited").peer(peer));
            return Err(AuthError::RateLimited);
        }
        let now = now();
        let Some(device) = self.store.device_for_token(token, now).await? else {
            // Without this, walking the token space is the one thing on this surface that leaves
            // no trace at all.
            self.audit.record(&Entry::new("auth.rejected").peer(peer));
            return Err(AuthError::Unauthorized);
        };
        // A client reconnecting and polling is not an attacker, and the limiter here exists to
        // bound *guessing*. Charging it for success eventually locks out the only device that
        // ever had a valid token.
        self.auth_limiter.forget(peer);
        self.store.touch_device(&device.id, now).await?;
        Ok(device)
    }

    fn engine(&self) -> Result<&Passkeys, AuthError> {
        self.passkeys
            .as_deref()
            .ok_or(AuthError::Passkey(PasskeyError::Unavailable))
    }

    pub async fn start_passkey_registration(
        &self,
        device_name: &str,
    ) -> Result<(String, CreationChallengeResponse), AuthError> {
        let engine = self.engine()?;
        let existing = self.enrolled_passkeys(engine.rp_id()).await?;
        Ok(engine.start_registration(Uuid::new_v4(), device_name, &existing)?)
    }

    /// Enrolment mints the device *and* the credential together, so a passkey is never orphaned
    /// from the device list the operator revokes from.
    pub async fn finish_passkey_registration(
        &self,
        challenge_id: &str,
        credential: &RegisterPublicKeyCredential,
        device_name: &str,
        user_agent: Option<&str>,
        role: Role,
    ) -> Result<Enrolment, AuthError> {
        let engine = self.engine()?;
        let passkey = engine.finish_registration(challenge_id, credential)?;
        let now = now();
        let enrolment = self.enrol(device_name, role, user_agent, now).await?;
        self.store
            .save_credential(
                &Credential {
                    id: credential_id(&passkey),
                    device_id: enrolment.device.id.clone(),
                    rp_id: engine.rp_id().to_string(),
                    passkey: serde_json::to_string(&passkey).unwrap_or_default(),
                },
                now,
            )
            .await?;
        self.audit.record(&Entry::new("passkey.registered").device(
            &enrolment.device.id,
            &enrolment.device.name,
            enrolment.device.role.as_str(),
        ));
        Ok(enrolment)
    }

    pub async fn start_passkey_authentication(
        &self,
        peer: &str,
    ) -> Result<(String, RequestChallengeResponse), AuthError> {
        if !self.auth_limiter.check(peer) {
            return Err(AuthError::RateLimited);
        }
        let engine = self.engine()?;
        let credentials = self.enrolled_passkeys(engine.rp_id()).await?;
        if credentials.is_empty() {
            return Err(AuthError::UnknownCredential);
        }
        Ok(engine.start_authentication(&credentials)?)
    }

    pub async fn finish_passkey_authentication(
        &self,
        challenge_id: &str,
        credential: &PublicKeyCredential,
        peer: &str,
    ) -> Result<Enrolment, AuthError> {
        if !self.auth_limiter.check(peer) {
            return Err(AuthError::RateLimited);
        }
        let engine = self.engine()?;
        let result = engine.finish_authentication(challenge_id, credential)?;
        let now = now();
        let id = hex::encode(result.cred_id());
        let stored = self
            .store
            .credential(&id)
            .await?
            .ok_or(AuthError::UnknownCredential)?;
        let device = self
            .store
            .device(&stored.device_id)
            .await?
            .filter(|d| d.active(now))
            .ok_or(AuthError::Unauthorized)?;
        self.auth_limiter.forget(peer);
        self.refresh_counter(&stored, &result, now).await?;
        // The session token is device-bound exactly as a paired one is; the passkey is how the
        // device proved itself, not a separate kind of session.
        let token = self.store.mint_token(&device.id, now, self.expiry(now)).await?;
        self.store.touch_device(&device.id, now).await?;
        self.audit.record(
            &Entry::new("passkey.authenticated")
                .device(&device.id, &device.name, device.role.as_str())
                .peer(peer),
        );
        Ok(Enrolment { device, token })
    }

    async fn refresh_counter(
        &self,
        stored: &Credential,
        result: &AuthenticationResult,
        now: i64,
    ) -> Result<(), AuthError> {
        if !result.needs_update() {
            return Ok(());
        }
        let Ok(mut passkey) = serde_json::from_str::<Passkey>(&stored.passkey) else {
            return Ok(());
        };
        passkey.update_credential(result);
        let encoded = serde_json::to_string(&passkey).unwrap_or_else(|_| stored.passkey.clone());
        self.store.touch_credential(&stored.id, &encoded, now).await?;
        Ok(())
    }

    async fn enrolled_passkeys(&self, rp_id: &str) -> Result<Vec<Passkey>, AuthError> {
        Ok(self
            .store
            .credentials(rp_id, now())
            .await?
            .iter()
            .filter_map(|c| serde_json::from_str(&c.passkey).ok())
            .collect())
    }

    pub async fn devices(&self) -> Result<Vec<Device>, AuthError> {
        Ok(self.store.devices().await?)
    }

    pub async fn revoke(&self, id: &str, by: &Device) -> Result<bool, AuthError> {
        let done = self.store.revoke_device(id, now()).await?;
        if done {
            let _ = self.changes.send(id.to_string());
            self.audit.record(
                &Entry::new("device.revoked")
                    .device(&by.id, &by.name, by.role.as_str())
                    .detail(serde_json::json!({ "revoked": id })),
            );
        }
        Ok(done)
    }

    pub async fn set_role(&self, id: &str, role: Role, by: &Device) -> Result<bool, AuthError> {
        let done = self.store.set_role(id, role).await?;
        if done {
            let _ = self.changes.send(id.to_string());
            self.audit.record(
                &Entry::new("device.role")
                    .device(&by.id, &by.name, by.role.as_str())
                    .detail(serde_json::json!({ "target": id, "role": role.as_str() })),
            );
        }
        Ok(done)
    }

    /// Renews a Tier 0 device for another term. Deliberate, and audited, because the expiry
    /// exists to force exactly this decision.
    pub async fn renew(&self, id: &str, by: &Device) -> Result<bool, AuthError> {
        let expires_at = self.expiry(now());
        let done = self.store.extend_device(id, expires_at).await?;
        if done {
            let _ = self.changes.send(id.to_string());
            self.audit.record(
                &Entry::new("device.renewed")
                    .device(&by.id, &by.name, by.role.as_str())
                    .detail(serde_json::json!({ "target": id, "expires_at": expires_at })),
            );
        }
        Ok(done)
    }
}

fn credential_id(passkey: &Passkey) -> String {
    hex::encode(passkey.cred_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arms as it creates, because that is what an operator at the console does. Tests that care
    /// about the arming gate itself call [`Auth::create_pairing`] directly.
    async fn armed_code(a: &Auth, role: Role) -> String {
        let pairing = a.create_pairing(role, Delivery::Console).await.unwrap();
        if !pairing.armed {
            assert!(a.arm_pairing(&pairing.code).await.unwrap());
        }
        pairing.code
    }

    async fn auth(origin: &str) -> Auth {
        Auth::new(
            Store::open_memory().await.unwrap(),
            Tier::detect(origin).unwrap(),
            AuditLog::disabled(),
            Policy::default(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn pairing_mints_a_device_bound_token() {
        let a = auth("http://192.168.1.24:8790").await;
        let code = armed_code(&a, Role::Full).await;
        let e = a.redeem_pairing(&code, "phone", None, "1.2.3.4").await.unwrap();
        assert_eq!(a.authenticate(&e.token, "1.2.3.4").await.unwrap().id, e.device.id);
    }

    #[tokio::test]
    async fn a_tier_zero_token_expires_and_a_passkey_tier_token_does_not() {
        let zero = auth("http://192.168.1.24:8790").await;
        let code = armed_code(&zero, Role::Full).await;
        let e = zero
            .redeem_pairing(&code, "phone", None, "1.2.3.4")
            .await
            .unwrap();
        assert!(
            e.device.expires_at.is_some(),
            "cleartext access must be time-boxed"
        );

        let one = auth("https://kampr.example.com").await;
        let code = armed_code(&one, Role::Full).await;
        let e = one.redeem_pairing(&code, "phone", None, "1.2.3.4").await.unwrap();
        assert_eq!(e.device.expires_at, None);
    }

    #[tokio::test]
    async fn a_used_code_is_dead_and_a_wrong_one_never_lived() {
        let a = auth("http://192.168.1.24:8790").await;
        let code = armed_code(&a, Role::Full).await;
        a.redeem_pairing(&code, "phone", None, "1.2.3.4").await.unwrap();
        assert!(matches!(
            a.redeem_pairing(&code, "laptop", None, "1.2.3.4").await,
            Err(AuthError::BadPairingCode)
        ));
        assert!(matches!(
            a.redeem_pairing("ZZZZ-ZZZZ", "laptop", None, "5.6.7.8").await,
            Err(AuthError::BadPairingCode)
        ));
    }

    #[tokio::test]
    async fn a_client_that_keeps_authenticating_successfully_never_locks_itself_out() {
        let a = auth("http://192.168.1.24:8790").await;
        let code = armed_code(&a, Role::Full).await;
        let e = a.redeem_pairing(&code, "phone", None, "1.2.3.4").await.unwrap();
        for n in 0..60 {
            assert!(
                a.authenticate(&e.token, "1.2.3.4").await.is_ok(),
                "the limiter is for failures; a good token was refused on attempt {n}"
            );
        }
    }

    #[tokio::test]
    async fn token_probing_is_not_invisible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let a = Auth::new(
            Store::open_memory().await.unwrap(),
            Tier::detect("http://192.168.1.24:8790").unwrap(),
            AuditLog::open(&path).unwrap(),
            Policy::default(),
        )
        .unwrap();
        for _ in 0..40 {
            let _ = a.authenticate("kmp_nope", "9.9.9.9").await;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("auth.rejected"), "{text}");
        assert!(text.contains("auth.rate_limited"), "{text}");
    }

    #[tokio::test]
    async fn guessing_is_rate_limited_per_peer() {
        let a = auth("http://192.168.1.24:8790").await;
        let mut refused = 0;
        for _ in 0..20 {
            if matches!(
                a.redeem_pairing("ZZZZ-ZZZZ", "attacker", None, "9.9.9.9").await,
                Err(AuthError::RateLimited)
            ) {
                refused += 1;
            }
        }
        assert!(refused > 10, "a short code needs a limiter to be safe");
        assert!(
            !matches!(
                a.redeem_pairing("ZZZZ-ZZZZ", "elsewhere", None, "8.8.8.8").await,
                Err(AuthError::RateLimited)
            ),
            "one attacker must not lock everyone else out"
        );
    }

    #[tokio::test]
    async fn revocation_ends_the_session_immediately() {
        let a = auth("http://192.168.1.24:8790").await;
        let code = armed_code(&a, Role::Full).await;
        let e = a.redeem_pairing(&code, "phone", None, "1.2.3.4").await.unwrap();
        assert!(a.revoke(&e.device.id, &e.device).await.unwrap());
        assert!(matches!(
            a.authenticate(&e.token, "1.2.3.4").await,
            Err(AuthError::Unauthorized)
        ));
    }

    /// A read-only device receives every frame of every pane, and `kampr setup` prints the
    /// pairing code into a Herdr popup — so the code alone cannot be the whole credential once
    /// anything is watching. The operator's keypress at the console is the channel a read-only
    /// device does not have.
    #[tokio::test]
    async fn a_code_printed_on_a_watched_screen_is_inert_until_an_operator_arms_it() {
        let a = auth("http://192.168.1.24:8790").await;
        let first = a.create_pairing(Role::Full, Delivery::Console).await.unwrap();
        assert!(first.armed, "nothing is watching before the first device pairs");
        a.redeem_pairing(&first.code, "kiosk", None, "1.2.3.4")
            .await
            .unwrap();

        let second = a.create_pairing(Role::Full, Delivery::Console).await.unwrap();
        assert!(!second.armed);
        assert!(
            matches!(
                a.redeem_pairing(&second.code, "watcher", None, "5.6.7.8").await,
                Err(AuthError::BadPairingCode)
            ),
            "a device that read the code off the screen must not be able to redeem it"
        );

        assert!(
            a.create_pairing(Role::Full, Delivery::Authenticated)
                .await
                .unwrap()
                .armed,
            "a code handed to an authenticated writer was never on a watched screen"
        );
        assert!(a.arm_pairing(&second.code).await.unwrap());
        assert_eq!(
            a.redeem_pairing(&second.code, "laptop", None, "1.2.3.4")
                .await
                .unwrap()
                .device
                .role,
            Role::Full
        );
    }

    #[tokio::test]
    async fn a_readonly_pairing_yields_a_readonly_device() {
        let a = auth("http://192.168.1.24:8790").await;
        let code = armed_code(&a, Role::Readonly).await;
        let e = a.redeem_pairing(&code, "kiosk", None, "1.2.3.4").await.unwrap();
        assert_eq!(e.device.role, Role::Readonly);
        assert!(!e.device.role.writes());
    }

    #[tokio::test]
    async fn passkey_ceremonies_are_refused_outright_on_tier_zero() {
        let a = auth("http://192.168.1.24:8790").await;
        assert!(a.passkeys().is_none());
        assert!(matches!(
            a.start_passkey_registration("phone").await,
            Err(AuthError::Passkey(PasskeyError::Unavailable))
        ));
    }

    #[tokio::test]
    async fn a_hostname_node_offers_a_registration_challenge() {
        let a = auth("https://kampr.example.com").await;
        let (id, challenge) = a.start_passkey_registration("phone").await.unwrap();
        assert!(!id.is_empty());
        assert_eq!(challenge.public_key.rp.id, "kampr.example.com");
    }

    #[tokio::test]
    async fn passkey_authentication_needs_something_enrolled() {
        let a = auth("https://kampr.example.com").await;
        assert!(matches!(
            a.start_passkey_authentication("1.2.3.4").await,
            Err(AuthError::UnknownCredential)
        ));
    }
}
