use anyhow::{Context, Result, bail};
use kampr_node::{BUILD, Config, ConfigError};
use std::path::Path;

pub struct Update {
    pub check: bool,
    /// A tag, for going back to a release that worked. `None` is the latest.
    pub version: Option<String>,
}

pub async fn run(config_dir: &Path, state_override: Option<&Path>, args: Update) -> Result<()> {
    let config = match Config::load(config_dir) {
        Ok(config) => config,
        // A node that has never been through `kampr init` can still update itself, so a missing
        // config is defaults rather than a refusal.
        Err(ConfigError::Missing(_)) => Config::bootstrap("kampr"),
        // Anything else is a config that exists and says something. `[update] repo` decides both
        // the release URL and the identity the signature is pinned to, so falling back to the
        // defaults here would answer a config nobody can load by installing from somewhere the
        // operator did not name.
        Err(e) => bail!("{e}"),
    };
    let state_dir = config.resolve_state_dir(state_override);
    match args.check {
        true => report(&config, &state_dir).await,
        false => install(&config, args.version.as_deref()),
    }
}

async fn report(config: &Config, state_dir: &Path) -> Result<()> {
    let cache = kampr_node::update::check(config, state_dir).await?;
    match cache.against(BUILD) {
        Some(available) => {
            println!("kampr {BUILD} — {available} is available");
            println!("  take it with: kampr update");
            println!(
                "  release notes: https://github.com/{}/releases",
                config.update.repo
            );
        }
        None => println!(
            "kampr {BUILD} — up to date (the latest release is {})",
            cache.latest.as_deref().unwrap_or("unknown")
        ),
    }
    Ok(())
}

fn install(config: &Config, version: Option<&str>) -> Result<()> {
    let exe = std::env::current_exe().context("finding the binary this command is running from")?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let prefix = exe
        .parent()
        .map(Path::to_path_buf)
        .filter(|dir| dir.is_dir())
        .with_context(|| format!("{} has no directory to install into", exe.display()))?;
    refuse_unless_writable(&prefix)?;

    let wanted = version.unwrap_or("latest");
    println!(
        "kampr: updating {} from {} ({wanted})",
        exe.display(),
        config.update.repo
    );

    let staged = tempfile::Builder::new()
        .prefix("kampr-update")
        .tempdir()
        .context("a directory to unpack into")?;
    let script = staged.path().join("install.sh");
    std::fs::write(&script, kampr_node::installer::SCRIPT)
        .with_context(|| format!("writing {}", script.display()))?;

    let status = kampr_node::installer::command(&script, &config.update.repo)
        .env("KAMPR_PREFIX", &prefix)
        .env("KAMPR_VERSION", wanted)
        .env("KAMPR_MODE", "update")
        .status()
        .context("running the installer — is `sh` on this host?")?;

    if !status.success() {
        bail!(
            "kampr update did not finish; the installer said why above.\n\
             It verifies before it replaces anything and puts the previous binary back if the new \
             one does not run, so {} is still the kampr you had.",
            exe.display()
        );
    }
    Ok(())
}

/// Refuses before 16 MB is downloaded rather than after, and names the path — a kampr installed
/// by root, or into a read-only image, is a common enough shape to be worth its own sentence.
fn refuse_unless_writable(prefix: &Path) -> Result<()> {
    let probe = prefix.join(".kampr-update-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => bail!(
            "{} is not writable ({e}), so nothing has been changed.\n\
             Re-run as whoever owns it, or install into a directory you own with:\n  \
             curl -fsSL https://github.com/dbrain/kampr/releases/latest/download/install.sh -o install.sh && sh install.sh",
            prefix.display()
        ),
    }
}
