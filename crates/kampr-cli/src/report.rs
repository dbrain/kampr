use anyhow::Result;
use kampr_auth::{AuditLog, Auth, NodeIdentity, Policy, Store, Tier};
use kampr_node::Config;
use std::path::Path;
use std::time::Duration;

/// Reads the same files the node serves from, so the CLI can answer without the node running and
/// without a device token of its own.
pub struct Local {
    pub config: Config,
    pub auth: Auth,
    pub identity: Option<NodeIdentity>,
    pub state_dir: std::path::PathBuf,
}

impl Local {
    pub async fn open(config_dir: &Path, state_dir: Option<&Path>) -> Result<Self> {
        let config = Config::load(config_dir)?;
        let state_dir = config.resolve_state_dir(state_dir);
        let mut tier = Tier::detect(&config.origin())?;
        if !config.auth.rp_id.is_empty() {
            tier = tier.with_rp_id(&config.auth.rp_id);
        }
        let store = Store::open(&Config::state_db(&state_dir)).await?;
        let audit = if config.auth.audit {
            AuditLog::open(&Config::audit_path(&state_dir))?
        } else {
            AuditLog::disabled()
        };
        let policy = Policy {
            pairing_ttl: Duration::from_secs(config.auth.pairing_ttl_secs),
            tier0_token_ttl: (config.auth.token_days > 0)
                .then(|| Duration::from_secs(config.auth.token_days * 86_400)),
            ..Policy::default()
        };
        Ok(Self {
            identity: NodeIdentity::load(&Config::node_key_path(config_dir))?,
            auth: Auth::new(store, tier, audit, policy)?,
            config,
            state_dir,
        })
    }
}

/// A QR of the URL, because typing `http://192.168.1.24:8790` on a phone is the single most
/// annoying step in the whole install.
pub fn qr(url: &str) -> String {
    use qrcode::QrCode;
    use qrcode::render::unicode;
    match QrCode::new(url.as_bytes()) {
        Ok(code) => code
            .render::<unicode::Dense1x2>()
            .quiet_zone(true)
            .dark_color(unicode::Dense1x2::Light)
            .light_color(unicode::Dense1x2::Dark)
            .build(),
        Err(_) => String::new(),
    }
}

pub async fn reachable(origin: &str) -> bool {
    let Some((host, port)) = host_port(origin) else {
        return false;
    };
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

fn host_port(origin: &str) -> Option<(String, u16)> {
    let url = url::Url::parse(origin).ok()?;
    let port = url.port_or_known_default()?;
    Some((url.host_str()?.to_string(), port))
}

/// The bind address is the single biggest thing about a Kampr node, and "0.0.0.0" does not say
/// what it means to anyone who is not already thinking about it.
pub fn bind_summary(config: &Config) -> String {
    if config.server.exposed() {
        format!(
            "  bind        {} — reachable from every device on this network.\n                           Anything that pairs here can type into every terminal on this host.",
            config.server.bind
        )
    } else {
        format!(
            "  bind        {} — this machine only. A phone on the LAN cannot reach it.\n                           `kampr init --bind 0.0.0.0:{}` opens it to the network.",
            config.server.bind,
            config
                .bind_addr()
                .map_or(kampr_node::config::DEFAULT_PORT, |a| a.port())
        )
    }
}

pub fn tier_summary(tier: &Tier) -> String {
    let mut lines = vec![format!(
        "  tier {}   {}",
        tier.tier,
        if tier.secure_context {
            "secure context"
        } else {
            "cleartext — anyone on this network can read the session"
        }
    )];
    lines.push(format!(
        "  passkeys {}   notifications {}   install {}",
        yes_no(tier.passkeys),
        yes_no(tier.push),
        yes_no(tier.installable)
    ));
    if let Some(unlocks) = tier.unlocks {
        lines.push(format!("  {unlocks}"));
    }
    lines.join("\n")
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qr_is_produced_for_a_lan_url() {
        let rendered = qr("http://192.168.1.24:8790");
        assert!(rendered.lines().count() > 10);
        assert!(rendered.contains('\u{2588}') || rendered.contains('\u{2580}'));
    }

    #[test]
    fn the_bind_summary_says_what_the_address_means() {
        let mut c = Config::bootstrap("x");
        let closed = bind_summary(&c);
        assert!(closed.contains("this machine only"), "{closed}");
        assert!(closed.contains("--bind"), "{closed}");

        c.server.bind = "0.0.0.0:8790".into();
        let open = bind_summary(&c);
        assert!(open.contains("every device on this network"), "{open}");
        assert!(open.contains("every terminal"), "{open}");
    }

    #[test]
    fn the_tier_summary_names_what_a_hostname_would_unlock() {
        let zero = tier_summary(&Tier::detect("http://192.168.1.24:8790").unwrap());
        assert!(zero.contains("tier 0"));
        assert!(zero.contains("cleartext"));
        assert!(zero.contains("passkeys no"));
        assert!(zero.contains("hostname"));

        let one = tier_summary(&Tier::detect("https://kampr.example.com").unwrap());
        assert!(one.contains("tier 1"));
        assert!(one.contains("passkeys yes"));
        assert!(!one.contains("cleartext"));
    }

    #[test]
    fn host_and_port_come_from_the_origin() {
        assert_eq!(
            host_port("http://192.168.1.24:8790"),
            Some(("192.168.1.24".into(), 8790))
        );
        assert_eq!(
            host_port("https://kampr.example.com"),
            Some(("kampr.example.com".into(), 443))
        );
        assert_eq!(host_port("nonsense"), None);
    }
}
