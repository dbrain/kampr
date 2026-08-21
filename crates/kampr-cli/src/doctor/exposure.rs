use super::{Check, cert};
use kampr_auth::Tier;
use kampr_node::Config;
use std::path::Path;

pub fn checks(config: &Config) -> Vec<Check> {
    vec![bind(config), tier(config), tls(config)]
}

fn bind(config: &Config) -> Check {
    let addr = &config.server.bind;
    let Ok(parsed) = config.bind_addr() else {
        return Check::fail("bind", format!("server.bind {addr:?} is not host:port"))
            .fix("kampr init --bind 127.0.0.1:8790");
    };
    if !config.server.exposed() {
        return Check::ok(
            "bind",
            format!("{addr} — this machine only, so no phone on the LAN can reach it"),
        );
    }
    Check::warn(
        "bind",
        format!(
            "{addr} — reachable from every device on this network, and anything that pairs \
             there can type into every terminal on this host"
        ),
    )
    .fix(format!(
        "kampr init --bind 127.0.0.1:{}   (if that was not deliberate)",
        parsed.port()
    ))
}

/// What the origin makes possible, and — when something is impossible — why, in the terms the
/// operator would otherwise have to guess at from a missing button.
fn tier(config: &Config) -> Check {
    let origin = config.origin();
    let Ok(mut tier) = Tier::detect(&origin) else {
        return Check::fail("tier", format!("origin {origin:?} is not a usable URL"))
            .fix("set server.origin in config.toml to the one URL clients will use");
    };
    if !config.auth.rp_id.is_empty() {
        tier = tier.with_rp_id(&config.auth.rp_id);
    }
    let capabilities = format!(
        "tier {} at {origin} — passkeys {}, notifications {}, install to home screen {}",
        tier.tier,
        yes_no(tier.passkeys),
        yes_no(tier.push),
        yes_no(tier.installable),
    );
    if tier.passkeys {
        return Check::ok("tier", capabilities);
    }
    let why = if tier.secure_context {
        "A WebAuthn RP ID must be a registrable domain and this origin is not one, so the \
         passkey button cannot appear however the node is configured."
    } else {
        "This origin is neither HTTPS nor loopback, so it is not a secure context: no service \
         worker, no push, no install — and a WebAuthn RP ID must be a registrable domain, which \
         an IP address can never be."
    };
    let check = Check::new(
        "tier",
        if tier.secure_context {
            super::Status::Ok
        } else {
            super::Status::Warn
        },
        format!("{capabilities}. {why}"),
    );
    check.fix("point a hostname at this machine, give it a certificate, and set server.origin to it")
}

/// Own certificate, or a proxy in front. `trust_proxy` without either is the dangerous one: the
/// headers it starts believing are forgeable by anyone who can reach the node directly.
fn tls(config: &Config) -> Check {
    if config.server.tls.enabled {
        return own_certificate(config);
    }
    if config.server.trust_proxy {
        return proxy(config);
    }
    if config.server.exposed() {
        return Check::warn(
            "tls",
            "no certificate and no proxy — every keystroke and every frame crosses this network \
             in the clear",
        )
        .fix("put a proxy with a certificate in front, or set server.tls in config.toml");
    }
    Check::ok(
        "tls",
        "no certificate, and none needed: nothing off this machine can reach the node",
    )
}

fn own_certificate(config: &Config) -> Check {
    let tls = &config.server.tls;
    for (label, path) in [("cert", &tls.cert), ("key", &tls.key)] {
        if path.is_empty() {
            return Check::fail(
                "tls",
                format!("server.tls.enabled is set but tls.{label} is empty"),
            )
            .fix("set server.tls.cert and server.tls.key in config.toml");
        }
        if !Path::new(path).exists() {
            return Check::fail("tls", format!("tls.{label} {path} does not exist"))
                .fix(format!("point server.tls.{label} at a readable PEM file"));
        }
    }
    if let Some(mode) = super::host::mode(Path::new(&tls.key)).filter(|m| m & 0o077 != 0) {
        return Check::fail(
            "tls",
            format!(
                "the private key {} is {:o}, readable beyond its owner",
                tls.key, mode
            ),
        )
        .fix(format!("chmod 600 {}", tls.key));
    }
    match cert::expiry(Path::new(&tls.cert)) {
        Err(e) => Check::fail("tls", format!("{} is not a usable certificate: {e}", tls.cert))
            .fix("replace it with the PEM certificate chain the proxy or CA issued"),
        Ok(days) if days < 0 => Check::fail("tls", format!("the certificate expired {} days ago", -days))
            .fix("renew it; browsers will refuse the origin until you do"),
        Ok(days) if days < 14 => Check::warn("tls", format!("the certificate expires in {days} days"))
            .fix("renew it before it does"),
        Ok(days) => Check::ok("tls", format!("own certificate, valid for another {days} days")),
    }
}

fn proxy(config: &Config) -> Check {
    let bind = &config.server.bind;
    if !config.server.exposed() {
        return Check::ok(
            "tls",
            format!(
                "no own certificate; trust_proxy is on and the node binds {bind}, so only a \
                 proxy on this machine can reach it"
            ),
        );
    }
    Check::fail(
        "tls",
        format!(
            "trust_proxy is on while the node itself answers on {bind} — anyone who can reach it \
             directly can forge X-Forwarded-For and get a fresh rate-limit bucket for every guess"
        ),
    )
    .fix("bind 127.0.0.1 and let the proxy reach it there, or set trust_proxy = false")
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::super::Status;
    use super::*;

    fn config() -> Config {
        Config::bootstrap("x")
    }

    #[test]
    fn a_loopback_node_with_no_certificate_is_healthy() {
        let c = config();
        assert_eq!(bind(&c).status, Status::Ok);
        assert_eq!(tls(&c).status, Status::Ok);
        assert_eq!(tier(&c).status, Status::Ok, "loopback is a secure context");
    }

    #[test]
    fn an_ip_origin_says_passkeys_are_impossible_rather_than_missing() {
        let mut c = config();
        c.server.origin = "http://192.168.1.24:8790".into();
        let check = tier(&c);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("passkeys no"), "{}", check.detail);
        assert!(check.detail.contains("registrable domain"), "{}", check.detail);
    }

    #[test]
    fn a_hostname_with_a_certificate_is_tier_one_and_says_nothing_is_locked() {
        let mut c = config();
        c.server.origin = "https://kampr.example.com".into();
        let check = tier(&c);
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("passkeys yes"), "{}", check.detail);
    }

    #[test]
    fn an_exposed_bind_is_a_warning_and_says_what_it_means() {
        let mut c = config();
        c.server.bind = "0.0.0.0:8790".into();
        let check = bind(&c);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("every terminal"), "{}", check.detail);
        assert!(check.fix.is_some());
    }

    #[test]
    fn a_trusted_proxy_in_front_of_nothing_is_a_failure() {
        let mut c = config();
        c.server.trust_proxy = true;
        assert_eq!(
            tls(&c).status,
            Status::Ok,
            "loopback: only a local proxy reaches it"
        );

        c.server.bind = "0.0.0.0:8790".into();
        let check = tls(&c);
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("X-Forwarded-For"), "{}", check.detail);
    }

    #[test]
    fn a_certificate_that_is_not_there_is_a_failure_rather_than_a_surprise_at_boot() {
        let mut c = config();
        c.server.tls.enabled = true;
        c.server.tls.cert = "/nowhere/cert.pem".into();
        c.server.tls.key = "/nowhere/key.pem".into();
        let check = tls(&c);
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("does not exist"), "{}", check.detail);
    }
}
