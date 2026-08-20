use serde::Serialize;
use std::net::IpAddr;
use url::{Host, Url};

/// What the configured origin makes possible.
///
/// Two web-platform rules decide this and neither is negotiable. A WebAuthn RP ID must be a
/// registrable domain — an IP address is not one, and HTTPS does not change that. And service
/// workers, PWA install and the Push API all require a secure context, which plain HTTP on a LAN
/// IP is not. So the node detects what the origin can support and tells clients what to hide,
/// rather than offering a passkey button that fails at the last step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tier {
    pub tier: u8,
    pub origin: String,
    pub host: String,
    pub secure_context: bool,
    pub passkeys: bool,
    pub push: bool,
    pub installable: bool,
    /// The RP ID a passkey would be registered against. `None` whenever passkeys cannot work,
    /// which is the only honest value — a registration bound to the wrong RP ID is unusable.
    pub rp_id: Option<String>,
    /// Present exactly when something is unavailable, and says what would unlock it.
    pub unlocks: Option<&'static str>,
}

#[derive(Debug, thiserror::Error)]
pub enum TierError {
    #[error("origin {0} is not a URL: {1}")]
    NotAUrl(String, url::ParseError),
    #[error("origin {0} has no host")]
    NoHost(String),
    #[error("origin scheme {0} is neither http nor https")]
    BadScheme(String),
}

impl Tier {
    pub fn detect(origin: &str) -> Result<Self, TierError> {
        let url = Url::parse(origin).map_err(|e| TierError::NotAUrl(origin.into(), e))?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(TierError::BadScheme(url.scheme().into()));
        }
        let host = url.host().ok_or_else(|| TierError::NoHost(origin.into()))?;
        let host_string = host.to_string();
        let https = url.scheme() == "https";
        let loopback = is_loopback(&host);
        // `localhost` is the one non-domain origin the browser treats as both a secure context
        // and a valid RP ID; a loopback *IP* is a secure context but still not a registrable
        // domain, so it gets the secure half and not the passkey half.
        let registrable = matches!(&host, Host::Domain(d) if !d.is_empty());
        let secure_context = https || loopback;
        let passkeys = registrable && secure_context;

        let unlocks = match (registrable, https) {
            (true, true) => None,
            (true, false) if loopback => None,
            (true, false) => Some(
                "A certificate for this hostname unlocks passkeys, notifications and installing \
                 Kampr to the home screen.",
            ),
            (false, _) => Some(
                "Passkeys need a hostname, not an IP address — HTTPS on an IP is not enough. \
                 Point a name at this machine and give it a certificate to unlock passkeys, \
                 notifications and installing Kampr to the home screen.",
            ),
        };

        Ok(Self {
            tier: u8::from(passkeys),
            origin: canonical(&url),
            host: host_string.clone(),
            secure_context,
            passkeys,
            push: secure_context,
            installable: secure_context,
            rp_id: passkeys.then_some(host_string),
            unlocks,
        })
    }

    /// What the wire's `security.unlocks` carries: the capability *names* a hostname and a
    /// certificate would add. The prose in `unlocks` is the operator's copy, for the CLI; a
    /// client writes its own from these names.
    pub fn locked(&self) -> Vec<&'static str> {
        [
            ("passkeys", self.passkeys),
            ("push", self.push),
            ("installable", self.installable),
        ]
        .into_iter()
        .filter(|(_, available)| !available)
        .map(|(name, _)| name)
        .collect()
    }

    /// A deliberate override for a node behind a proxy whose public hostname differs from the
    /// origin clients type. A passkey registered on one RP ID does not work on another, so this
    /// is only ever set from configuration, never inferred.
    pub fn with_rp_id(mut self, rp_id: &str) -> Self {
        if self.passkeys {
            self.rp_id = Some(rp_id.to_string());
        }
        self
    }
}

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(d) => *d == "localhost" || d.ends_with(".localhost"),
        Host::Ipv4(ip) => IpAddr::V4(*ip).is_loopback(),
        Host::Ipv6(ip) => IpAddr::V6(*ip).is_loopback(),
    }
}

/// Scheme, host and non-default port only — the form a browser sends in `Origin`.
fn canonical(url: &Url) -> String {
    let mut out = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        out.push_str(&format!(":{port}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lan_ip_is_tier_zero_however_it_is_dressed() {
        for origin in ["http://192.168.1.24:8790", "https://192.168.1.24:8790"] {
            let t = Tier::detect(origin).unwrap();
            assert!(!t.passkeys, "{origin} must not offer passkeys");
            assert_eq!(t.rp_id, None);
            assert_eq!(t.tier, 0);
            assert!(t.unlocks.is_some());
        }
    }

    #[test]
    fn https_on_an_ip_is_a_secure_context_without_passkeys() {
        let t = Tier::detect("https://192.168.1.24:8790").unwrap();
        assert!(t.secure_context && t.push && t.installable);
        assert!(!t.passkeys);
    }

    #[test]
    fn a_hostname_with_a_certificate_is_tier_one() {
        let t = Tier::detect("https://kampr.home.example.com").unwrap();
        assert!(t.passkeys && t.push && t.installable);
        assert_eq!(t.rp_id.as_deref(), Some("kampr.home.example.com"));
        assert_eq!(t.tier, 1);
        assert_eq!(t.unlocks, None);
    }

    #[test]
    fn a_hostname_without_a_certificate_still_cannot_do_passkeys() {
        let t = Tier::detect("http://kampr.home.example.com:8790").unwrap();
        assert!(!t.passkeys && !t.secure_context);
        assert!(t.unlocks.unwrap().contains("certificate"));
    }

    #[test]
    fn localhost_is_the_one_exception() {
        let t = Tier::detect("http://localhost:8790").unwrap();
        assert!(t.passkeys && t.secure_context);
        assert_eq!(t.rp_id.as_deref(), Some("localhost"));

        let loopback_ip = Tier::detect("http://127.0.0.1:8790").unwrap();
        assert!(loopback_ip.secure_context, "a loopback IP is a secure context");
        assert!(!loopback_ip.passkeys, "but it is still not a registrable domain");
    }

    #[test]
    fn locked_names_what_the_wire_promises_and_nothing_else() {
        assert_eq!(
            Tier::detect("http://192.168.1.24:8790").unwrap().locked(),
            ["passkeys", "push", "installable"]
        );
        assert_eq!(
            Tier::detect("https://192.168.1.24:8790").unwrap().locked(),
            ["passkeys"],
            "https on an IP buys everything a secure context buys"
        );
        assert!(
            Tier::detect("https://kampr.home.example.com")
                .unwrap()
                .locked()
                .is_empty()
        );
    }

    #[test]
    fn rp_id_override_never_resurrects_an_impossible_passkey() {
        let t = Tier::detect("http://192.168.1.24:8790")
            .unwrap()
            .with_rp_id("kampr.example.com");
        assert_eq!(t.rp_id, None);
    }

    #[test]
    fn canonical_origin_drops_the_path_and_the_default_port() {
        assert_eq!(
            Tier::detect("https://kampr.example.com/setup").unwrap().origin,
            "https://kampr.example.com"
        );
        assert_eq!(
            Tier::detect("http://192.168.1.24:8790/").unwrap().origin,
            "http://192.168.1.24:8790"
        );
    }
}
