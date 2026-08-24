use crate::note::Notification;
use crate::vapid::Vapid;
use kampr_auth::PushSubscription;
use std::net::{IpAddr, SocketAddr};
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

/// How much of a failing push service's answer is worth putting in the log. The body is written
/// by whoever owns the endpoint, and an endpoint is a string a client supplied.
const MAX_DETAIL: usize = 256;

/// Where a push endpoint is allowed to resolve to.
///
/// **A push endpoint is attacker-supplied and the node dials it from inside its own trust
/// boundary.** `https://` at subscribe time proves nothing about where the name points, so an
/// endpoint that resolves into the node's own network is refused rather than followed — and
/// resolution is checked at connection time, so a name that answers publicly once and privately
/// afterwards does not walk around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reach {
    #[default]
    Public,
    /// A test that runs its own push service on loopback. Nothing a released binary constructs
    /// asks for this.
    Loopback,
}

#[derive(Debug, thiserror::Error)]
pub enum SenderError {
    #[error("the push HTTP client could not be built: {0}")]
    Client(#[from] reqwest::Error),
}

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
    reach: Reach,
}

impl Sender {
    /// Fails rather than falling back to a default client: `unwrap_or_default` here quietly threw
    /// away [`REQUEST_TIMEOUT`], the redirect policy and the whole of [`Reach`], and left a sender
    /// that looked built.
    pub fn new(vapid: Arc<Vapid>, reach: Reach) -> Result<Self, SenderError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("kampr")
            // **A 302 is the way round the `https://` check.** Following one lets an
            // attacker-controlled endpoint aim this POST at loopback or at a link-local metadata
            // address from inside the node. A push service has no legitimate use for a redirect.
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(Resolver(reach)))
            .build()?;
        Ok(Self { vapid, http, reach })
    }

    pub async fn send(&self, target: &PushSubscription, note: &Notification) -> Outcome {
        // A literal address never reaches the resolver, so the same rule is applied to the host
        // the endpoint names.
        if let Some(refused) = private_literal(&target.endpoint, self.reach) {
            warn!(endpoint = %target.endpoint, address = %refused, "a push endpoint inside this node's own network is refused");
            return Outcome::Failed;
        }
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
            // **`crypto_headers` already carries the VAPID `Authorization`** once a signature has
            // been set. Adding one alongside it sends the header twice, and the edge in front of a
            // push service answers a bare nginx `400 Bad Request` that says nothing about why.
            for (name, value) in body.crypto_headers {
                request = request.header(name, value);
            }
            request = request
                .header("Content-Encoding", body.content_encoding.to_str())
                .body(body.content);
        }

        match request.send().await {
            Ok(mut response) => {
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
                // Read in chunks and stop: the body is written by whoever owns the endpoint, and
                // `text()` would pull all of it into memory before anything bounded it.
                let mut detail = String::new();
                while detail.len() < MAX_DETAIL {
                    match response.chunk().await {
                        Ok(Some(chunk)) => detail.push_str(&String::from_utf8_lossy(&chunk)),
                        _ => break,
                    }
                }
                let detail: String = detail.trim().chars().take(MAX_DETAIL).collect();
                warn!(endpoint = %target.endpoint, %status, %detail, "push refused");
                Outcome::Failed
            }
            Err(e) => {
                warn!(endpoint = %target.endpoint, error = %e, "push could not be delivered");
                Outcome::Failed
            }
        }
    }
}

/// A [`reqwest::dns::Resolve`] that refuses a name the moment any of its addresses is inside the
/// node's own network. All of them, not the first: a name that answers with one public address and
/// one private one is the rebinding case, and picking a survivor out of the set would serve it.
struct Resolver(Reach);

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let reach = self.0;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if let Some(refused) = addrs.iter().map(SocketAddr::ip).find(|ip| !reachable(*ip, reach)) {
                return Err(format!("{host} resolves to {refused}, inside this node's own network").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The address an endpoint names outright, when it names one this node will not dial.
fn private_literal(endpoint: &str, reach: Reach) -> Option<IpAddr> {
    let host = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // `[::1]:443`, and the bare-IPv6 case a URL cannot actually spell.
    let host = host.trim_start_matches('[');
    let host = match host.split_once(']') {
        Some((inside, _)) => inside,
        None => host.rsplit_once(':').map_or(host, |(before, _)| before),
    };
    let ip: IpAddr = host.parse().ok()?;
    (!reachable(ip, reach)).then_some(ip)
}

fn reachable(ip: IpAddr, reach: Reach) -> bool {
    if reach == Reach::Loopback {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // "this network", carrier-grade NAT, IETF protocol assignments, benchmarking,
                // and the reserved 240/4 — none of them a push service, all of them reachable
                // from inside a host.
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => reachable(IpAddr::V4(v4), reach),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    // unique local fc00::/7 and link-local fe80::/10
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80)
            }
        },
    }
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
        let sender = Sender::new(vapid(), Reach::Public).expect("a sender");
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

    /// The endpoint is a string a client handed over, and the node dials it from inside its own
    /// network. Every one of these is somewhere a push service is not.
    #[test]
    fn an_endpoint_naming_this_nodes_own_network_is_not_somewhere_a_push_service_lives() {
        for endpoint in [
            "https://127.0.0.1/push/x",
            "https://127.0.0.1:8790/push/x",
            "https://[::1]:8790/push/x",
            "https://169.254.169.254/latest/meta-data/",
            "https://10.0.0.5/push",
            "https://192.168.1.24:8790/push",
            "https://172.16.4.4/push",
            "https://100.64.0.1/push",
            "https://0.0.0.0/push",
            "https://[fd00::1]/push",
            "https://[fe80::1]/push",
            "https://[::ffff:127.0.0.1]/push",
        ] {
            assert!(
                private_literal(endpoint, Reach::Public).is_some(),
                "{endpoint} should be refused"
            );
            assert!(
                private_literal(endpoint, Reach::Loopback).is_none(),
                "{endpoint} is what a test's own push service looks like"
            );
        }
        for endpoint in [
            "https://push.example/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/abc",
            "https://93.184.216.34/push",
            "https://[2606:2800:220:1:248:1893:25c8:1946]/push",
        ] {
            assert!(
                private_literal(endpoint, Reach::Public).is_none(),
                "{endpoint} is an ordinary push service"
            );
        }
    }

    /// A sender that could not be built used to come back as `Client::default()` — no timeout, no
    /// redirect policy, no resolver — and looked exactly like one that had been.
    #[test]
    fn a_sender_is_built_or_it_is_an_error_rather_than_a_client_with_none_of_its_rules() {
        assert!(Sender::new(vapid(), Reach::Public).is_ok());
    }
}
