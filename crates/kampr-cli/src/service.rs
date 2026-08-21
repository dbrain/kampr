use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const UNIT: &str = "kampr.service";
const LABEL: &str = "dev.kampr.node";

/// Herdr's `[[startup]]` hooks are one-shot, not supervised, so keeping the node alive across a
/// reboot or a crash needs a real service manager. This installs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Supervisor {
    Systemd,
    Launchd,
    Unsupported,
}

impl Supervisor {
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") {
            return Self::Launchd;
        }
        // sd_booted(3): `/run/systemd/system` exists only when systemd is the init system. WSL2
        // without `systemd=true`, OpenRC and a plain container all fail it, and on those hosts a
        // user unit is a file nothing will ever read.
        if Path::new("/run/systemd/system").is_dir() {
            Self::Systemd
        } else {
            Self::Unsupported
        }
    }
}

/// What happens to an installed unit at the next reboot, which is not the same question as
/// whether it is enabled: a `systemd --user` manager lives inside the caller's login session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reboot {
    Survives,
    NeedsLinger { user: String },
    LingerUnknown { user: String },
    NeedsGuiLogin,
}

impl Reboot {
    pub fn note(&self) -> Option<String> {
        match self {
            Self::Survives => None,
            Self::NeedsLinger { user } => Some(format!(
                "Required — without this the node does not come back after a reboot:\n\
                 \n  loginctl enable-linger {user}\n\n\
                 systemd tears your user manager down when your last session ends and does not \
                 start it at boot, so the unit above stops at logout and stays stopped."
            )),
            Self::LingerUnknown { user } => Some(format!(
                "Could not tell whether lingering is on for {user}. Without it a `systemd --user` \
                 manager stops at logout and is not started at boot:\n\n  loginctl enable-linger {user}"
            )),
            Self::NeedsGuiLogin => Some(
                "This is a launchd agent in `gui/$(id -u)`, a domain that only exists once someone \
                 logs in at the screen — a Mac that reboots to the login window does not start the \
                 node until you do. A headless Mac needs a LaunchDaemon in /Library/LaunchDaemons \
                 instead."
                    .into(),
            ),
        }
    }
}

#[derive(Debug)]
pub struct Installed {
    pub path: PathBuf,
    pub reboot: Reboot,
}

pub fn install(
    binary: &Path,
    config_dir: &Path,
    state_dir: &Path,
    socket: Option<&str>,
) -> Result<Installed> {
    match Supervisor::detect() {
        Supervisor::Unsupported => bail!(
            "this host has no systemd, so there is no user unit to install. WSL2 needs \
             `systemd=true` in /etc/wsl.conf and a `wsl --shutdown`; on OpenRC, in a container, \
             or anywhere else, run `kampr serve` under whatever already supervises this box."
        ),
        supervisor => {
            kampr_auth::private_dir(config_dir)?;
            kampr_auth::private_dir(state_dir)?;
            match supervisor {
                Supervisor::Systemd => install_systemd(binary, config_dir, state_dir, socket),
                _ => install_launchd(binary, config_dir, state_dir, socket),
            }
        }
    }
}

/// `/var/lib/systemd/linger/<user>` is the file logind reads at boot, so it answers even when the
/// bus is out of reach — which is exactly the case on the hosts where this matters most.
pub fn linger(user: &str) -> Option<bool> {
    // Overridable because it is the one input a test cannot sandbox: `XDG_*` and the bus addresses
    // redirect everything else, and this absolute path made the result depend on whether the host
    // running the suite happened to have lingering on.
    let dir = std::env::var_os("KAMPR_LINGER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/systemd/linger"));
    let dir = dir.as_path();
    if dir.is_dir() {
        return Some(dir.join(user).exists());
    }
    run_output("loginctl", &["show-user", user, "-p", "Linger"])
        .ok()
        .and_then(|out| out.split('=').nth(1).map(|v| v.trim() == "yes"))
}

pub fn username() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| run_output("id", &["-un"]).ok().filter(|u| !u.is_empty()))
        .unwrap_or_else(|| "$USER".into())
}

fn ensure_linger() -> Reboot {
    let user = username();
    if linger(&user) != Some(true) {
        try_run("loginctl", &["enable-linger", &user]);
    }
    match linger(&user) {
        Some(true) => Reboot::Survives,
        Some(false) => Reboot::NeedsLinger { user },
        None => Reboot::LingerUnknown { user },
    }
}

pub fn uninstall() -> Result<()> {
    match Supervisor::detect() {
        Supervisor::Unsupported => Ok(()),
        Supervisor::Systemd => {
            let path = systemd_unit_path()?;
            quiet("systemctl", &["--user", "stop", UNIT]);
            quiet("systemctl", &["--user", "disable", UNIT]);
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            quiet("systemctl", &["--user", "daemon-reload"]);
            Ok(())
        }
        Supervisor::Launchd => {
            let path = launchd_plist_path()?;
            quiet("launchctl", &["bootout", &format!("gui/{}/{LABEL}", uid())]);
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            Ok(())
        }
    }
}

fn install_systemd(
    binary: &Path,
    config_dir: &Path,
    state_dir: &Path,
    socket: Option<&str>,
) -> Result<Installed> {
    let path = systemd_unit_path()?;
    std::fs::create_dir_all(path.parent().expect("unit path has a parent"))?;
    let unit = SYSTEMD_UNIT
        .replace("@BIN@", &binary.display().to_string())
        .replace("@CONFIG_DIR@", &config_dir.display().to_string())
        .replace("@STATE_DIR@", &state_dir.display().to_string())
        .replace("@SOCKET@", socket.unwrap_or("%h/.config/herdr/herdr.sock"));
    std::fs::write(&path, unit).with_context(|| format!("writing {}", path.display()))?;
    // The unit file is the durable artefact; a systemd that cannot be reached — no user session,
    // a container, a sandboxed environment — must not lose it.
    quiet("systemctl", &["--user", "daemon-reload"]);
    if !try_run("systemctl", &["--user", "enable", UNIT]) {
        eprintln!("kampr: could not enable {UNIT}; run `systemctl --user enable {UNIT}` by hand");
    }
    Ok(Installed {
        path,
        reboot: ensure_linger(),
    })
}

fn install_launchd(
    binary: &Path,
    config_dir: &Path,
    state_dir: &Path,
    socket: Option<&str>,
) -> Result<Installed> {
    let path = launchd_plist_path()?;
    std::fs::create_dir_all(path.parent().expect("plist path has a parent"))?;
    // launchd expands nothing, and an empty HERDR_SOCKET_PATH is worse than an absent one:
    // `Herdr::discover` takes the variable at face value and dials a socket named "".
    let default_socket = default_socket_path()?;
    let plist = LAUNCHD_PLIST
        .replace("@BIN@", &binary.display().to_string())
        .replace("@CONFIG_DIR@", &config_dir.display().to_string())
        .replace("@STATE_DIR@", &state_dir.display().to_string())
        .replace("@SOCKET@", socket.unwrap_or(&default_socket));
    std::fs::write(&path, plist).with_context(|| format!("writing {}", path.display()))?;
    quiet("launchctl", &["bootout", &format!("gui/{}/{LABEL}", uid())]);
    run(
        "launchctl",
        &[
            "bootstrap",
            &format!("gui/{}", uid()),
            &path.display().to_string(),
        ],
    )?;
    Ok(Installed {
        path,
        reboot: Reboot::NeedsGuiLogin,
    })
}

pub fn status() -> String {
    match Supervisor::detect() {
        Supervisor::Unsupported => "no service manager on this host".into(),
        Supervisor::Systemd => {
            run_output("systemctl", &["--user", "is-active", UNIT]).unwrap_or_else(|_| "unknown".into())
        }
        Supervisor::Launchd => run_output("launchctl", &["print", &format!("gui/{}/{LABEL}", uid())])
            .map(|_| "active".to_string())
            .unwrap_or_else(|_| "inactive".into()),
    }
}

/// What a supervisor knows about the unit: enough for `kampr doctor` to tell "never installed"
/// from "installed and dead", which are different problems with different fixes.
#[derive(Debug, Clone)]
pub struct State {
    pub installed: bool,
    pub enabled: String,
    pub active: String,
    pub running: bool,
    pub failed: bool,
    pub result: String,
    pub since: String,
}

pub fn details() -> State {
    match Supervisor::detect() {
        Supervisor::Systemd => systemd_state(),
        Supervisor::Launchd => launchd_state(),
        Supervisor::Unsupported => State {
            installed: false,
            enabled: "no service manager".into(),
            active: "unknown".into(),
            running: false,
            failed: false,
            result: "unknown".into(),
            since: "unknown".into(),
        },
    }
}

fn systemd_state() -> State {
    let output = run_output(
        "systemctl",
        &[
            "--user",
            "show",
            UNIT,
            "--property=LoadState",
            "--property=UnitFileState",
            "--property=ActiveState",
            "--property=SubState",
            "--property=Result",
            "--property=StateChangeTimestamp",
        ],
    )
    .unwrap_or_default();
    let field = |key: &str| {
        output
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let active = field("ActiveState");
    let result = field("Result");
    let unit_file = field("UnitFileState");
    State {
        // A unit that was written but never reloaded still exists on disk, and `LoadState` alone
        // would call it missing.
        installed: field("LoadState") == "loaded" || systemd_unit_path().is_ok_and(|p| p.exists()),
        enabled: if unit_file.is_empty() {
            "not enabled".into()
        } else {
            unit_file
        },
        running: active == "active" || active == "activating",
        failed: active == "failed" || (!result.is_empty() && result != "success"),
        active: if active.is_empty() {
            "unknown".into()
        } else {
            active
        },
        result: if result.is_empty() {
            "unknown".into()
        } else {
            result
        },
        since: {
            let at = field("StateChangeTimestamp");
            if at.is_empty() { "unknown".into() } else { at }
        },
    }
}

fn launchd_state() -> State {
    let installed = launchd_plist_path().is_ok_and(|p| p.exists());
    let print = run_output("launchctl", &["print", &format!("gui/{}/{LABEL}", uid())]).ok();
    let running = print.as_ref().is_some_and(|p| p.contains("state = running"));
    let last_exit = print
        .as_ref()
        .and_then(|p| {
            p.lines()
                .find_map(|l| l.trim().strip_prefix("last exit code = "))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".into());
    State {
        installed,
        enabled: if installed {
            "loaded".into()
        } else {
            "not loaded".into()
        },
        active: if running {
            "running".into()
        } else {
            "stopped".into()
        },
        running,
        failed: !matches!(last_exit.as_str(), "0" | "unknown"),
        result: last_exit,
        since: "unknown".into(),
    }
}

pub fn start_hint() -> &'static str {
    match Supervisor::detect() {
        Supervisor::Systemd => "systemctl --user start kampr.service",
        Supervisor::Launchd => "launchctl kickstart gui/$(id -u)/dev.kampr.node",
        Supervisor::Unsupported => "kampr serve",
    }
}

fn default_socket_path() -> Result<String> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(format!("{home}/.config/herdr/herdr.sock"))
}

fn systemd_unit_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join("systemd/user").join(UNIT))
}

fn launchd_plist_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn uid() -> String {
    run_output("id", &["-u"]).unwrap_or_else(|_| "0".into())
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}

/// Best effort, and noisy failure is not information: stopping a unit that was never installed is
/// the normal case for an uninstall.
fn quiet(program: &str, args: &[&str]) {
    try_run(program, args);
}

fn try_run(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run_output(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The same file `packaging/kamprctl.sh` renders, included rather than repeated: two
/// copies of a unit are two units, and they drift silently.
const SYSTEMD_UNIT: &str = include_str!("../../../packaging/kampr.service");

const LAUNCHD_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.kampr.node</string>
  <key>ProgramArguments</key>
  <array>
    <string>@BIN@</string><string>serve</string>
    <string>--config-dir</string><string>@CONFIG_DIR@</string>
    <string>--state-dir</string><string>@STATE_DIR@</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>HERDR_SOCKET_PATH</key><string>@SOCKET@</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
</dict>
</plist>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_template_is_fully_substituted() {
        let unit = SYSTEMD_UNIT
            .replace("@BIN@", "/usr/local/bin/kampr")
            .replace("@CONFIG_DIR@", "/home/x/.config/kampr")
            .replace("@STATE_DIR@", "/home/x/.local/state/kampr")
            .replace("@SOCKET@", "/home/x/.config/herdr/herdr.sock");
        assert!(!unit.contains('@'), "{unit}");
        assert!(unit.contains("ExecStart=/usr/local/bin/kampr serve"));
        assert!(unit.contains("NoNewPrivileges=yes"));
    }

    /// Sessions created through `manage{session.create}` are the operator's agents, and they are
    /// children of this process. Measured on a real user manager, all three modes: with the
    /// default `control-group` and with `KillMode=mixed` a `systemctl --user restart` killed the
    /// detached child; only `process` left it running.
    #[test]
    fn the_unit_signals_the_node_and_not_the_sessions_it_created() {
        assert!(
            SYSTEMD_UNIT.contains("\nKillMode=process\n"),
            "any other KillMode SIGKILLs the whole cgroup, agents included: {SYSTEMD_UNIT}"
        );
        assert!(
            !SYSTEMD_UNIT.contains("\nPrivateTmp="),
            "a created session inherits the node's mount namespace, so a private /tmp is the \
             operator's agents writing somewhere their own shell cannot see: {SYSTEMD_UNIT}"
        );
    }

    #[test]
    fn the_plist_template_is_fully_substituted() {
        let plist = LAUNCHD_PLIST
            .replace("@BIN@", "/usr/local/bin/kampr")
            .replace("@CONFIG_DIR@", "/c")
            .replace("@STATE_DIR@", "/s")
            .replace("@SOCKET@", "/sock");
        assert!(!plist.contains('@'), "{plist}");
        assert!(plist.contains("<string>serve</string>"));
    }

    /// The service unit the CLI installs and the one the plugin's `kamprctl.sh` renders must not
    /// drift apart — they supervise the same process.
    #[test]
    fn the_embedded_unit_matches_the_packaged_template() {
        let Some(packaged) = packaged("kampr.service") else {
            return;
        };
        assert_eq!(packaged.trim(), SYSTEMD_UNIT.trim());
    }

    #[test]
    fn the_embedded_plist_matches_the_packaged_template() {
        let Some(packaged) = packaged("dev.kampr.node.plist") else {
            return;
        };
        assert_eq!(packaged.trim(), LAUNCHD_PLIST.trim());
    }

    #[test]
    fn an_installed_unit_without_linger_says_the_exact_command_and_why() {
        let note = Reboot::NeedsLinger {
            user: "dbrain".into(),
        }
        .note()
        .expect("a required next step");
        assert!(note.contains("loginctl enable-linger dbrain"), "{note}");
        assert!(note.contains("Required"), "{note}");
        assert!(note.contains("boot"), "{note}");

        assert_eq!(
            Reboot::Survives.note(),
            None,
            "nothing to say when it will come back"
        );

        let unknown = Reboot::LingerUnknown { user: "x".into() }.note().unwrap();
        assert!(unknown.contains("loginctl enable-linger x"), "{unknown}");

        let mac = Reboot::NeedsGuiLogin.note().unwrap();
        assert!(mac.contains("LaunchDaemon"), "{mac}");
        assert!(mac.contains("logs in at the screen"), "{mac}");
    }

    /// A unit written on a host with no systemd is a file nothing reads, and `installed …` is a
    /// lie the operator has no way to catch.
    #[test]
    fn a_host_with_no_service_manager_refuses_rather_than_writing_into_the_void() {
        if Supervisor::detect() != Supervisor::Unsupported {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let error = install(
            Path::new("/usr/local/bin/kampr"),
            &dir.path().join("config"),
            &dir.path().join("state"),
            None,
        )
        .expect_err("no supervisor to install into");
        assert!(error.to_string().contains("systemd"), "{error}");
    }

    fn packaged(name: &str) -> Option<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging")
            .join(name);
        std::fs::read_to_string(path).ok()
    }
}
