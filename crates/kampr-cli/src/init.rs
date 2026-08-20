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
}

/// First run has to work immediately: config, an identity, a URL, a pairing code. Everything
/// above that is a rung on the ladder, offered from `kampr setup` and never demanded here.
pub async fn run(dirs_config: &Path, dirs_state: &Path, options: Init) -> Result<()> {
    let existing = Config::load(dirs_config).ok();
    if existing.is_some() && !options.force {
        println!(
            "kampr: already initialised at {}",
            Config::path(dirs_config).display()
        );
        println!("       re-run with --force to replace it, or `kampr setup` to pair a device.\n");
    }
    let mut config = match (existing, options.force) {
        (Some(config), false) => config,
        _ => Config::bootstrap(&options.node_name.clone().unwrap_or_else(hostname)),
    };
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
    let local = Local::open(dirs_config, Some(dirs_state)).await?;
    let pair = pairing::create(&local, Role::Full).await?;
    let url = local.config.origin();

    println!("Kampr node {} ({})", local.config.node_name, local.config.node_id);
    println!("  config      {}", path.display());
    println!("  state       {}", dirs_state.display());
    println!("  identity    {}", identity.fingerprint());
    println!();
    println!("  {url}");
    println!("{}", report::bind_summary(&local.config));
    println!();
    print!("{}", report::qr(&url));
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

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "kampr".into())
}
