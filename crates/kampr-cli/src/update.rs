use anyhow::{Context, Result, bail};
use kampr_node::{BUILD, Config};
use std::path::Path;

/// The installer, as it shipped in this binary.
///
/// **Embedded rather than fetched.** `install.sh` is the thing that decides whether a download is
/// genuine, and fetching the verifier over the same channel as the artefact means whoever can
/// serve a binary can serve a verifier that accepts it. This copy came out of the release the
/// operator already verified when they installed it, and it cannot drift from the one in the tree
/// because it *is* the one in the tree.
const INSTALLER: &str = include_str!("../../../packaging/install.sh");

pub struct Update {
    pub check: bool,
    /// A tag, for going back to a release that worked. `None` is the latest.
    pub version: Option<String>,
}

pub async fn run(config_dir: &Path, state_override: Option<&Path>, args: Update) -> Result<()> {
    // A node that has never been through `kampr init` can still update itself, so a missing
    // config is defaults rather than a refusal.
    let config = Config::load(config_dir).unwrap_or_else(|_| Config::bootstrap("kampr"));
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
    std::fs::write(&script, INSTALLER).with_context(|| format!("writing {}", script.display()))?;

    let status = std::process::Command::new("sh")
        .arg(&script)
        .env("KAMPR_PREFIX", &prefix)
        .env("KAMPR_REPO", &config.update.repo)
        .env("KAMPR_VERSION", wanted)
        .env("KAMPR_MODE", "update")
        // The escape hatch exists for an operator installing a binary they built themselves. It
        // must not be reachable by an environment variable that happened to be set in the shell
        // that ran this, because what it switches off is the only thing standing between a
        // tampered download and a process that can type into every terminal on this host.
        .env_remove("KAMPR_ALLOW_UNVERIFIED")
        // The same rule, and the larger hole of the two: the base supplies the tarball *and* the
        // SHA256SUMS it is checked against, so a base set in the caller's environment chooses what
        // kampr becomes and gets "checksum verified: yes" printed underneath it. An operator
        // installing from their own base runs `install.sh` and sets it there, deliberately.
        .env_remove("KAMPR_BASE_URL")
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
