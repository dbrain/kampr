use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const UNIT: &str = "kampr.service";
const LABEL: &str = "dev.kampr.node";

/// Herdr's `[[startup]]` hooks are one-shot, not supervised, so keeping the node alive across a
/// reboot or a crash needs a real service manager. This installs one.
pub enum Supervisor {
    Systemd,
    Launchd,
}

impl Supervisor {
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") {
            Self::Launchd
        } else {
            Self::Systemd
        }
    }
}

pub fn install(binary: &Path, config_dir: &Path, state_dir: &Path, socket: Option<&str>) -> Result<PathBuf> {
    kampr_auth::private_dir(config_dir)?;
    kampr_auth::private_dir(state_dir)?;
    match Supervisor::detect() {
        Supervisor::Systemd => install_systemd(binary, config_dir, state_dir, socket),
        Supervisor::Launchd => install_launchd(binary, config_dir, state_dir, socket),
    }
}

pub fn uninstall() -> Result<()> {
    match Supervisor::detect() {
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
) -> Result<PathBuf> {
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
    Ok(path)
}

fn install_launchd(
    binary: &Path,
    config_dir: &Path,
    state_dir: &Path,
    socket: Option<&str>,
) -> Result<PathBuf> {
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
    Ok(path)
}

pub fn status() -> String {
    match Supervisor::detect() {
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

const SYSTEMD_UNIT: &str = r#"[Unit]
Description=Kampr node
After=default.target
# A phone-only operator cannot run `systemctl reset-failed`, so never stop trying.
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=@BIN@ serve --config-dir @CONFIG_DIR@ --state-dir @STATE_DIR@
Restart=on-failure
RestartSec=5
# The node can type into every terminal in the herd, so deny escalation and give it a private /tmp.
# ProtectSystem is deliberately unset: the state directory may be anywhere the operator chose.
NoNewPrivileges=yes
PrivateTmp=yes
Environment=HERDR_SOCKET_PATH=@SOCKET@

[Install]
WantedBy=default.target
"#;

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

    fn packaged(name: &str) -> Option<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging")
            .join(name);
        std::fs::read_to_string(path).ok()
    }
}
