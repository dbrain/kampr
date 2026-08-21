use crate::pairing;
use crate::recovery;
use crate::report::{self, Local};
use anyhow::{Context, Result};
use kampr_auth::{NodeIdentity, Role};
use kampr_node::Config;
use std::path::Path;

pub struct Init {
    pub node_name: Option<String>,
    pub bind: Option<String>,
    pub origin: Option<String>,
    pub force: bool,
    pub new_identity: bool,
}

impl Init {
    fn overrides(&self) -> bool {
        self.node_name.is_some() || self.bind.is_some() || self.origin.is_some()
    }
}

/// First run has to work immediately: config, an identity, a URL, a pairing code. Everything
/// above that is a rung on the ladder, offered from `kampr setup` and never demanded here.
pub async fn run(dirs_config: &Path, dirs_state: &Path, options: Init) -> Result<()> {
    let path = Config::path(dirs_config);
    let existing = Config::load(dirs_config).ok();
    let rewrite = options.force || options.new_identity;
    if existing.is_some() && !rewrite && !options.overrides() {
        println!("kampr: already initialised at {}", path.display());
        println!("       `kampr setup` pairs a device; --bind and --origin change those in place.\n");
    }
    let mut config = match (&existing, rewrite) {
        (Some(config), false) => config.clone(),
        (Some(old), true) => {
            println!("{}\n", force_summary(&path, old, options.new_identity));
            rewritten(old, &options)
        }
        (None, _) => Config::bootstrap(&options.node_name.clone().unwrap_or_else(hostname)),
    };
    if let Some(name) = &options.node_name {
        config.node_name = name.clone();
    }
    if let Some(bind) = options.bind {
        config.server.bind = bind;
    }
    if let Some(origin) = options.origin {
        config.server.origin = origin;
    }
    config.state_dir = dirs_state.display().to_string();
    config.config_dir = dirs_config.display().to_string();
    config
        .bind_addr()
        .with_context(|| format!("server.bind {:?} is not host:port", config.server.bind))?;
    let path = config.save(dirs_config)?;
    kampr_auth::private_dir(dirs_state)?;

    let identity = NodeIdentity::load_or_create(&Config::node_key_path(dirs_config))?;
    // Generated here rather than at first serve, because rotating it invalidates every push
    // subscription already issued — so it has to exist before anything can subscribe against it.
    // It is written whatever the tier: the ladder is climbed later, and a node that reaches Tier 1
    // must not need a re-init to be able to notify.
    let vapid = kampr_push::Vapid::load_or_create(&Config::vapid_path(dirs_state), &config.push_subject())
        .context("generating the VAPID key")?;
    let local = Local::open(dirs_config, Some(dirs_state)).await?;
    let pair = pairing::create(&local, Role::Full).await?;
    let url = local.config.origin();

    println!("Kampr node {} ({})", local.config.node_name, local.config.node_id);
    println!("  config      {}", path.display());
    println!("  state       {}", dirs_state.display());
    println!("  identity    {}", identity.fingerprint());
    println!("  push key    {}", vapid.public_key_b64());
    println!();
    println!("  {url}");
    println!("{}", report::bind_summary(&local.config));
    println!();
    print!("{}", report::qr(&format!("{url}#pair={}", pair.code)));
    println!();
    println!("  pairing code   {}", pair.code);
    println!(
        "  valid for      {} minutes, one device",
        local.auth.policy().pairing_ttl.as_secs() / 60
    );
    pairing::arm(&local, &pair).await?;
    println!();
    println!("{}", report::tier_summary(local.auth.tier()));
    // Once, at the first init that has none. Re-running init must not silently retire the paper
    // record the operator already made.
    if !local.auth.has_recovery().await? {
        recovery::print_new_code(&local.auth.issue_recovery().await?);
    }
    println!();
    println!("Next:  kampr service install   keeps it running across reboots");
    Ok(())
}

/// `--force` rewrites the file from defaults, and the operator's answers are not defaults. The
/// identity, the bind, the origin, the proxy trust and the hub role have either no CLI flag at
/// all or a flag that was not passed, so throwing them away silently is throwing away work that
/// cannot be reconstructed — every enrolled passkey and every mesh peer is pinned to them.
fn rewritten(old: &Config, options: &Init) -> Config {
    let mut config = Config::bootstrap(&old.node_name);
    if !options.new_identity {
        config.node_id = old.node_id.clone();
    }
    config.server = old.server.clone();
    config.auth.rp_id = old.auth.rp_id.clone();
    config.mesh = old.mesh.clone();
    config
}

fn force_summary(path: &Path, old: &Config, new_identity: bool) -> String {
    let origin = match old.server.origin.as_str() {
        "" => "derived from the bind".to_string(),
        explicit => explicit.to_string(),
    };
    let mut lines = vec![format!("kampr: rewriting {}", path.display())];
    if new_identity {
        lines.push(format!(
            "  discards  node id {} — every enrolled passkey and every mesh peer pinned to it \
             stops working",
            old.node_id
        ));
        lines.push(format!(
            "  keeps     bind {}, origin {origin}, trust_proxy = {}, {} extra origin(s), [mesh]",
            old.server.bind,
            old.server.trust_proxy,
            old.server.extra_origins.len()
        ));
    } else {
        lines.push(format!(
            "  keeps     node id {}, bind {}, origin {origin}, trust_proxy = {}, {} extra \
             origin(s), [mesh]",
            old.node_id,
            old.server.bind,
            old.server.trust_proxy,
            old.server.extra_origins.len()
        ));
    }
    lines.push(
        "  resets    [herdr], [limits], [journals], [push] and [auth] except rp_id to their \
         defaults"
            .into(),
    );
    lines.join("\n")
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "kampr".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Config {
        let mut config = Config::bootstrap("front");
        config.server.origin = "https://kampr.example.com".into();
        config.server.trust_proxy = true;
        config.server.extra_origins = vec!["https://kampr.lan".into()];
        config.auth.rp_id = "kampr.example.com".into();
        config.mesh.accept = true;
        config.herdr.binary = "/opt/herdr".into();
        config
    }

    fn options(new_identity: bool) -> Init {
        Init {
            node_name: None,
            bind: None,
            origin: None,
            force: true,
            new_identity,
        }
    }

    #[test]
    fn force_carries_forward_everything_that_has_no_flag_to_restore_it_with() {
        let old = hub();
        let new = rewritten(&old, &options(false));
        assert_eq!(new.node_id, old.node_id);
        assert_eq!(new.server.origin, old.server.origin);
        assert!(new.server.trust_proxy);
        assert_eq!(new.server.extra_origins, old.server.extra_origins);
        assert_eq!(new.auth.rp_id, old.auth.rp_id);
        assert_eq!(new.mesh.accept, old.mesh.accept);
        assert_eq!(
            new.herdr.binary, "herdr",
            "the tuning sections are what --force is for"
        );
    }

    #[test]
    fn a_new_identity_is_the_only_thing_that_changes_under_new_identity() {
        let old = hub();
        let new = rewritten(&old, &options(true));
        assert_ne!(new.node_id, old.node_id);
        assert_eq!(new.server.origin, old.server.origin);
        assert!(new.server.trust_proxy);
    }

    #[test]
    fn the_summary_names_the_identity_and_what_losing_it_costs() {
        let old = hub();
        let path = Path::new("/tmp/config.toml");
        let kept = force_summary(path, &old, false);
        assert!(kept.contains(&old.node_id), "{kept}");
        assert!(kept.contains("keeps"), "{kept}");
        assert!(kept.contains("resets"), "{kept}");
        assert!(kept.contains("trust_proxy = true"), "{kept}");

        let discarded = force_summary(path, &old, true);
        assert!(discarded.contains(&old.node_id), "{discarded}");
        assert!(
            discarded.contains("passkey") && discarded.contains("mesh peer"),
            "{discarded}"
        );
    }
}
