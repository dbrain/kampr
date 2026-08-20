use crate::note::Notification;
use crate::vapid::Vapid;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use kampr_auth::PushSubscription;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};
use web_push::{ContentEncoding, SubscriptionInfo, Urgency, WebPushMessageBuilder};

/// A blocked agent is worth waking someone for now or not at all, so a push that could not be
/// delivered inside this window is not worth keeping queued behind it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a push service holds an undelivered message. Long enough for a phone that is asleep,
/// short enough that yesterday's prompt does not arrive tomorrow.
const TTL_SECONDS: u32 = 900;

/// What a delivery attempt settled as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    /// The push service says this endpoint no longer exists (404/410). The row is dead and the
    /// caller deletes it — retrying is the one thing that is certainly wrong.
    Gone,
    /// Anything else: a rate limit, a network fault, an unreadable subscription.
    Failed,
}

/// Encrypts and posts. **Delivery is not retried.**
///
/// A push service already queues for a phone that is asleep, so a retry here only ever duplicates
/// a notification that was accepted; and the poll behind the whole feature means the next status
/// change produces a fresh one anyway. The failure that matters is `Gone`, and that is a delete.
pub struct Sender {
    vapid: Arc<Vapid>,
    http: reqwest::Client,
}

impl Sender {
    pub fn new(vapid: Arc<Vapid>) -> Self {
        Self {
            vapid,
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .user_agent("kampr")
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn send(&self, target: &PushSubscription, note: &Notification) -> Outcome {
        let info = SubscriptionInfo::new(
            target.endpoint.clone(),
            target.p256dh.clone(),
            target.auth.clone(),
        );
        let payload = match serde_json::to_vec(note) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(error = %e, "a notification would not serialise");
                return Outcome::Failed;
            }
        };
        let mut builder = WebPushMessageBuilder::new(&info);
        builder.set_payload(ContentEncoding::Aes128Gcm, &payload);
        builder.set_ttl(TTL_SECONDS);
        builder.set_urgency(Urgency::High);
        match self.vapid.sign(&info) {
            Ok(signature) => builder.set_vapid_signature(signature),
            Err(e) => {
                warn!(error = %e, endpoint = %target.endpoint, "could not sign a VAPID request");
                return Outcome::Failed;
            }
        }
        let message = match builder.build() {
            Ok(message) => message,
            // A subscription whose keys will not parse can never be delivered to, so it is as
            // dead as one the service has forgotten.
            Err(e) => {
                warn!(error = %e, endpoint = %target.endpoint, "unusable push subscription");
                return Outcome::Gone;
            }
        };

        let mut request = self
            .http
            .post(message.endpoint.to_string())
            .header("TTL", message.ttl.to_string())
            .header("Content-Type", "application/octet-stream");
        if let Some(urgency) = message.urgency {
            request = request.header("Urgency", urgency.to_string());
        }
        if let Some(body) = message.payload {
            // `crypto_headers` is empty for aes128gcm — the salt and the key ride in the body —
            // but it is applied rather than assumed, so an encoding change does not go silent.
            for (name, value) in body.crypto_headers {
                request = request.header(name, value);
            }
            request = request
                .header("Content-Encoding", body.content_encoding.to_str())
                .header(
                    "Authorization",
                    authorization(&self.vapid, &info).unwrap_or_default(),
                )
                .body(body.content);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Outcome::Delivered;
                }
                // RFC 8030: the endpoint is gone for good. Everything else may be transient and
                // the next status change will produce a fresh notification anyway.
                if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
                    debug!(endpoint = %target.endpoint, %status, "push endpoint is gone");
                    return Outcome::Gone;
                }
                let detail = response.text().await.unwrap_or_default();
                warn!(endpoint = %target.endpoint, %status, detail = %detail.trim(), "push refused");
                Outcome::Failed
            }
            Err(e) => {
                warn!(endpoint = %target.endpoint, error = %e, "push could not be delivered");
                Outcome::Failed
            }
        }
    }
}

/// `vapid t=<jwt>, k=<base64url public key>` — the scheme every push service expects, built here
/// rather than by the crate because the HTTP half is ours.
fn authorization(vapid: &Vapid, info: &SubscriptionInfo) -> Option<String> {
    let signature = vapid.sign(info).ok()?;
    Some(format!(
        "vapid t={}, k={}",
        signature.auth_t,
        URL_SAFE_NO_PAD.encode(&signature.auth_k)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::Blocked;

    fn vapid() -> Arc<Vapid> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(Vapid::load_or_create(&dir.path().join("vapid.pem"), "mailto:x@y").unwrap())
    }

    fn note() -> Notification {
        Notification::batch(vec![Blocked {
            pane: "01J/w1:p1".into(),
            node: "01J".into(),
            agent: Some("claude".into()),
            label: None,
            question: Some("Run the tests?".into()),
        }])
        .unwrap()
    }

    /// A subscription whose keys are junk can never be delivered to. Reporting it as a transient
    /// failure would leave a row that fails forever; `Gone` is what deletes it.
    #[tokio::test]
    async fn an_unusable_subscription_is_gone_rather_than_a_permanent_retry() {
        let sender = Sender::new(vapid());
        let outcome = sender
            .send(
                &PushSubscription {
                    id: "s1".into(),
                    device_id: "d1".into(),
                    kind: "webpush".into(),
                    endpoint: "https://push.example/abc".into(),
                    p256dh: "not-a-key".into(),
                    auth: "nor-this".into(),
                },
                &note(),
            )
            .await;
        assert_eq!(outcome, Outcome::Gone);
    }
}
