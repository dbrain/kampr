use crate::report::{self, Local};
use crate::service;
use anyhow::Result;
use kampr_auth::Role;
use std::io::{IsTerminal, Write};
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The setup ladder.
///
/// Not a wizard: the node already works, and every rung here is an optional upgrade that says
/// what it unlocks. It runs inside a Herdr popup pane, and popups are not always a terminal — a
/// plugin action can be invoked with its output piped — so it prints the whole ladder and exits
/// when there is nobody to read a keystroke.
pub async fn run(config_dir: &Path, state_dir: Option<&Path>) -> Result<()> {
    let local = Local::open(config_dir, state_dir).await?;
    show(&local).await?;

    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        println!("\n(not a terminal — showing the ladder only. Run `kampr setup` in a shell to act.)");
        return Ok(());
    }

    loop {
        println!();
        println!("  1  pair a device            2  pair a read-only device");
        println!("  3  list devices             4  revoke a device");
        println!("  5  install the service      6  refresh");
        println!("  q  quit");
        let choice = prompt("  > ")?;
        match choice.trim() {
            "1" => pair(&local, Role::Full).await?,
            "2" => pair(&local, Role::Readonly).await?,
            "3" => devices(&local).await?,
            "4" => revoke(&local).await?,
            "5" => install(config_dir, &local.state_dir)?,
            "6" => show(&local).await?,
            "q" | "" => return Ok(()),
            other => println!("  ? {other}"),
        }
    }
}

async fn show(local: &Local) -> Result<()> {
    let url = local.config.origin();
    let up = report::reachable(&url).await;
    println!("Kampr — {} ({})", local.config.node_name, local.config.node_id);
    println!("  node        {}", if up { "running" } else { "not reachable" });
    println!("  service     {}", service::status());
    println!("  url         {url}");
    if let Some(identity) = &local.identity {
        println!("  identity    {}", identity.fingerprint());
    }
    println!("{}", report::tier_summary(local.auth.tier()));
    let devices = local.auth.devices().await?;
    let active = devices.iter().filter(|d| d.active(kampr_auth::now())).count();
    println!("  devices     {active} paired");
    Ok(())
}

async fn pair(local: &Local, role: Role) -> Result<()> {
    let code = local.auth.create_pairing(role).await?;
    let url = local.config.origin();
    println!();
    print!("{}", report::qr(&url));
    println!("  {url}");
    println!("  code   {code}   ({}, one device)", role.as_str());
    println!(
        "  valid  {} minutes",
        local.auth.policy().pairing_ttl.as_secs() / 60
    );
    Ok(())
}

async fn devices(local: &Local) -> Result<()> {
    let devices = local.auth.devices().await?;
    if devices.is_empty() {
        println!("  no devices paired yet");
        return Ok(());
    }
    println!();
    for (n, device) in devices.iter().enumerate() {
        let state = if device.revoked_at.is_some() {
            "revoked".to_string()
        } else if let Some(expires) = device.expires_at {
            format!("expires {}", stamp(expires))
        } else {
            "no expiry".to_string()
        };
        println!(
            "  {:>2}  {:<20} {:<9} {:<24} last seen {}",
            n + 1,
            device.name,
            device.role.as_str(),
            state,
            device.last_seen_at.map_or("never".into(), stamp)
        );
    }
    Ok(())
}

async fn revoke(local: &Local) -> Result<()> {
    let devices = local.auth.devices().await?;
    devices_listing(&devices);
    let choice = prompt("  revoke which? ")?;
    let Some(index) = choice.trim().parse::<usize>().ok().filter(|n| *n >= 1) else {
        return Ok(());
    };
    let Some(device) = devices.get(index - 1) else {
        println!("  no such device");
        return Ok(());
    };
    // Revoking from the console is an operator action, so it is recorded against the device it
    // removed rather than against a device that did not make the request.
    if local.auth.revoke(&device.id, device).await? {
        println!("  revoked {}", device.name);
    }
    Ok(())
}

fn devices_listing(devices: &[kampr_auth::Device]) {
    for (n, device) in devices.iter().enumerate() {
        println!("  {:>2}  {} ({})", n + 1, device.name, device.role.as_str());
    }
}

fn install(config_dir: &Path, state_dir: &Path) -> Result<()> {
    let binary = std::env::current_exe()?;
    let socket = std::env::var("HERDR_SOCKET_PATH").ok();
    let path = service::install(&binary, config_dir, state_dir, socket.as_deref())?;
    println!("  installed {}", path.display());
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line)
}

fn stamp(unix: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| unix.to_string())
}
