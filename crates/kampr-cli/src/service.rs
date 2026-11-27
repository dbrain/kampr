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
    let agent = Agent {
        binary: binary.to_path_buf(),
        config_dir: config_dir.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        socket: socket.unwrap_or(&default_socket).to_string(),
    };
    let home = std::env::var("HOME").context("HOME is not set")?;
    std::fs::write(&path, agent.render(&home, installer_docker().as_deref()))
        .with_context(|| format!("writing {}", path.display()))?;
    load_launchd(&path);
    Ok(Installed {
        path,
        reboot: Reboot::NeedsGuiLogin,
    })
}

/// `launchctl kickstart -k` restarts a job on the plist launchd loaded at bootstrap, so a changed
/// plist reaches the node only through a bootout and a bootstrap.
///
/// Both run in a process group of their own: `kampr update` is often a child of the node it
/// updates (a fleet run, the phone), and a bootout kills the job's process group, which would end
/// this command between the two and leave the node unloaded until the next login.
///
/// `bootout` returns once it has sent SIGTERM; launchd keeps the job until it exits, up to five
/// seconds before it kills it, and refuses a bootstrap meanwhile (37, "Operation already in
/// progress"). So the bootstrap retries.
fn load_launchd(path: &Path) -> bool {
    reload_launchd("launchctl", path)
}

const RELOAD_SCRIPT: &str = r#"
"$4" bootout "$1/$2" >/dev/null 2>&1
n=0
until "$4" bootstrap "$1" "$3"; do
  n=$((n + 1))
  [ "$n" -lt 75 ] || exit 1
  sleep 0.2
done
"#;

fn reload_launchd(launchctl: &str, path: &Path) -> bool {
    use std::os::unix::process::CommandExt;
    let domain = format!("gui/{}", uid());
    let plist_path = path.display().to_string();
    let loaded = Command::new("/bin/sh")
        .args(["-c", RELOAD_SCRIPT, "sh", &domain, LABEL, &plist_path, launchctl])
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !loaded {
        eprintln!("kampr: could not load {LABEL}; run `launchctl bootstrap {domain} {plist_path}` by hand");
    }
    loaded
}

fn installer_docker() -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path| which(&path, "docker"))
}

/// What an installed agent runs, read back out of its plist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Agent {
    binary: PathBuf,
    config_dir: PathBuf,
    state_dir: PathBuf,
    socket: String,
}

impl Agent {
    fn read(plist: &str) -> Option<Self> {
        let strings: Vec<&str> = plist
            .split_once("<key>ProgramArguments</key>")?
            .1
            .split_once("</array>")?
            .0
            .split("<string>")
            .filter_map(|s| s.split_once("</string>").map(|(v, _)| v))
            .collect();
        let after = |flag: &str| {
            let at = strings.iter().position(|s| *s == flag)?;
            strings.get(at + 1).map(PathBuf::from)
        };
        let socket = plist
            .split_once("<key>HERDR_SOCKET_PATH</key>")?
            .1
            .trim_start()
            .strip_prefix("<string>")?
            .split_once("</string>")?
            .0;
        Some(Self {
            binary: PathBuf::from(strings.first()?),
            config_dir: after("--config-dir")?,
            state_dir: after("--state-dir")?,
            socket: socket.to_string(),
        })
    }

    fn render(&self, home: &str, docker: Option<&Path>) -> String {
        LAUNCHD_PLIST
            .replace("@BIN@", &self.binary.display().to_string())
            .replace("@CONFIG_DIR@", &self.config_dir.display().to_string())
            .replace("@STATE_DIR@", &self.state_dir.display().to_string())
            .replace("@SOCKET@", &self.socket)
            .replace("@PATH@", &launchd_path(home, docker))
    }
}

fn refreshed(plist: &str, home: &str, docker: Option<&Path>) -> Option<String> {
    Some(Agent::read(plist)?.render(home, docker))
}

/// Rewrites the installed agent from this build's template, keeping the binary, directories and
/// socket it names, and reloads it. `install.sh` runs it on update so a Mac gets a template change
/// (the `PATH` the node finds `docker` on) without reinstalling. `false` is nothing to refresh.
pub fn refresh() -> Result<bool> {
    if Supervisor::detect() != Supervisor::Launchd {
        return Ok(false);
    }
    let path = launchd_plist_path()?;
    let Ok(plist) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let home = std::env::var("HOME").context("HOME is not set")?;
    let new = refreshed(&plist, &home, installer_docker().as_deref())
        .with_context(|| format!("{} does not name a binary and its directories", path.display()))?;
    std::fs::write(&path, new).with_context(|| format!("writing {}", path.display()))?;
    if !load_launchd(&path) {
        bail!("{} was rewritten but did not load", path.display());
    }
    Ok(true)
}

pub fn status() -> String {
    match Supervisor::detect() {
        Supervisor::Unsupported => "no service manager on this host".into(),
        Supervisor::Systemd => {
            run_output("systemctl", &["--user", "is-active", UNIT]).unwrap_or_else(|_| "unknown".into())
        }
        Supervisor::Launchd => match launchd_state() {
            state if state.running => "active".into(),
            _ => "inactive".into(),
        },
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
    /// A LaunchDaemon in launchd's `system` domain, which starts at boot without anyone logging in.
    pub daemon: bool,
}

impl State {
    fn not_installed(enabled: &str) -> Self {
        Self {
            installed: false,
            enabled: enabled.into(),
            active: "unknown".into(),
            running: false,
            failed: false,
            result: "unknown".into(),
            since: "unknown".into(),
            daemon: false,
        }
    }

    fn launchctl(&self, verb: &str) -> String {
        match self.daemon {
            true => format!("sudo launchctl {verb} system/{LABEL}"),
            false => format!("launchctl {verb} gui/$(id -u)/{LABEL}"),
        }
    }

    /// Where to read why the unit stopped.
    pub fn status_hint(&self, supervisor: Supervisor) -> String {
        match supervisor {
            Supervisor::Systemd => format!("systemctl --user status {UNIT}"),
            Supervisor::Launchd => self.launchctl("print"),
            Supervisor::Unsupported => "kampr serve".into(),
        }
    }

    pub fn restart_hint(&self, supervisor: Supervisor) -> String {
        match supervisor {
            Supervisor::Systemd => format!("KAMPR_LOG=debug systemctl --user restart {UNIT}"),
            Supervisor::Launchd => self.launchctl("kickstart -k"),
            Supervisor::Unsupported => "kampr serve".into(),
        }
    }

    pub fn start_hint(&self, supervisor: Supervisor) -> String {
        match supervisor {
            Supervisor::Systemd => format!("systemctl --user start {UNIT}"),
            Supervisor::Launchd => self.launchctl("kickstart"),
            Supervisor::Unsupported => "kampr serve".into(),
        }
    }

    pub fn stop_hint(&self, supervisor: Supervisor) -> String {
        match supervisor {
            Supervisor::Systemd => format!("systemctl --user stop {UNIT}"),
            Supervisor::Launchd => self.launchctl("bootout"),
            Supervisor::Unsupported => "stop whatever runs `kampr serve`".into(),
        }
    }
}

pub fn details() -> State {
    match Supervisor::detect() {
        Supervisor::Systemd => systemd_state(),
        Supervisor::Launchd => launchd_state(),
        Supervisor::Unsupported => State::not_installed("no service manager"),
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
        daemon: false,
    }
}

/// The agent `kampr service install` writes, then a LaunchDaemon, which is what a Mac that reboots
/// to the login window needs instead.
fn launchd_state() -> State {
    let print = |target: &str| run_output("launchctl", &["print", target]).unwrap_or_default();
    let agent = print(&format!("gui/{}/{LABEL}", uid()));
    let mut state = parse_launchd(&agent);
    state.installed |= launchd_plist_path().is_ok_and(|p| p.exists());
    if state.installed {
        return state;
    }
    let mut daemon = parse_launchd(&print(&format!("system/{LABEL}")));
    daemon.installed |= Path::new(LAUNCH_DAEMON).exists();
    daemon.daemon = true;
    match daemon.installed {
        true => daemon,
        false => state,
    }
}

/// `launchctl print` names an agent that has run since it was loaded `(never exited)`, which is
/// every healthy LaunchAgent after a login, and newer releases follow a code with its name
/// (`78: EX_CONFIG`). Only a nonzero code is a failure.
fn parse_launchd(print: &str) -> State {
    let loaded = !print.trim().is_empty();
    let running = print.lines().any(|l| l.trim() == "state = running");
    let last_exit = print
        .lines()
        .find_map(|l| l.trim().strip_prefix("last exit code = "))
        .map(str::trim);
    let code = last_exit.and_then(|e| e.split(':').next()?.trim().parse::<i64>().ok());
    let result = match (last_exit, code) {
        (_, Some(_)) => last_exit.unwrap_or_default().to_string(),
        (Some(other), None) => other.trim_matches(|c| c == '(' || c == ')').to_string(),
        (None, None) => "unknown".into(),
    };
    State {
        installed: loaded,
        enabled: if loaded { "loaded" } else { "not loaded" }.into(),
        active: if running { "running" } else { "stopped" }.into(),
        running,
        failed: code.is_some_and(|c| c != 0),
        result,
        since: "unknown".into(),
        daemon: false,
    }
}

/// The `PATH` the node will actually have under its own unit, which is not the one that installed
/// it: a `systemd --user` manager's is `/usr/local/bin:/usr/bin:/bin` and friends, with no
/// `~/.local/bin` — where `install.sh` puts both kampr and herdr.
pub fn manager_path() -> Option<String> {
    match Supervisor::detect() {
        Supervisor::Systemd => run_output("systemctl", &["--user", "show-environment"])
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("PATH="))
            .map(str::to_string),
        Supervisor::Launchd => Some(plist_path_env(
            &std::fs::read_to_string(launchd_plist_path().ok()?).ok()?,
        )),
        Supervisor::Unsupported => None,
    }
}

/// launchd's own `PATH` for a job that sets none, which is what a plist written before it carried
/// one gave the node.
const LAUNCHD_DEFAULT_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn plist_path_env(plist: &str) -> String {
    plist
        .split_once("<key>PATH</key>")
        .and_then(|(_, rest)| {
            rest.trim_start()
                .strip_prefix("<string>")?
                .split_once("</string>")
        })
        .map_or_else(|| LAUNCHD_DEFAULT_PATH.to_string(), |(path, _)| path.to_string())
}

/// launchd hands a job its default `PATH` and nothing from the login shell, so the node could not
/// run `kampr`, `herdr` or `docker` from where installers put them: `~/.local/bin`, Homebrew's
/// two prefixes (Docker Desktop and OrbStack link `docker` into `/usr/local/bin`), and wherever
/// the `docker` the installer can see lives.
fn launchd_path(home: &str, docker: Option<&Path>) -> String {
    let mut entries: Vec<String> = vec![
        format!("{home}/.local/bin"),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
    ];
    entries.extend(docker.and_then(Path::parent).map(|dir| dir.display().to_string()));
    entries.extend(LAUNCHD_DEFAULT_PATH.split(':').map(str::to_string));
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert(entry.clone()));
    entries.join(":")
}

/// The file a bare `name` runs on `path`, asked the way `exec` asks it: the first entry holding a
/// file this user may execute.
pub fn which(path: &str, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    path.split(':')
        .filter(|entry| !entry.is_empty())
        .map(|entry| Path::new(entry).join(name))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// The kampr the unit runs, which need not be the one running this command.
pub fn unit_binary() -> Option<PathBuf> {
    match Supervisor::detect() {
        Supervisor::Systemd => {
            exec_start_path(&run_output("systemctl", &["--user", "show", UNIT, "--property=ExecStart"]).ok()?)
        }
        Supervisor::Launchd => {
            Agent::read(&std::fs::read_to_string(launchd_plist_path().ok()?).ok()?).map(|a| a.binary)
        }
        Supervisor::Unsupported => None,
    }
}

fn exec_start_path(shown: &str) -> Option<PathBuf> {
    let path = shown
        .split_once("path=")?
        .1
        .split(&[' ', ';'][..])
        .next()
        .filter(|p| !p.is_empty())?;
    Some(PathBuf::from(path))
}

/// Who a Mac lets in at the screen: a LaunchAgent's `gui` domain exists only once that happens.
#[derive(Debug, Clone, Default)]
pub struct MacLogin {
    pub auto_login: Option<String>,
    pub console: Option<String>,
}

impl MacLogin {
    pub fn read() -> Self {
        let answer = |program: &str, args: &[&str]| {
            run_output(program, args)
                .ok()
                .filter(|v| !v.is_empty() && v != "root")
        };
        Self {
            auto_login: answer(
                "defaults",
                &[
                    "read",
                    "/Library/Preferences/com.apple.loginwindow",
                    "autoLoginUser",
                ],
            ),
            console: answer("stat", &["-f%Su", "/dev/console"]),
        }
    }
}

/// For the unit `kampr service install` writes, before there is any state to read.
pub fn start_hint() -> String {
    State::not_installed("").start_hint(Supervisor::detect())
}

pub fn stop_hint() -> String {
    State::not_installed("").stop_hint(Supervisor::detect())
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

const LAUNCH_DAEMON: &str = "/Library/LaunchDaemons/dev.kampr.node.plist";

fn launchd_plist_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn uid() -> String {
    run_output("id", &["-u"]).unwrap_or_else(|_| "0".into())
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
  <dict>
    <key>HERDR_SOCKET_PATH</key><string>@SOCKET@</string>
    <key>PATH</key><string>@PATH@</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <!-- 63 is 0o077. launchd takes this as a decimal integer, and sqlite creates the
       write-ahead log beside the database at whatever umask the process was given. -->
  <key>Umask</key><integer>63</integer>
</dict>
</plist>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// `systemctl show` renders ExecStart as a record, not a command line.
    #[test]
    fn the_binary_the_unit_runs_is_read_out_of_the_exec_start_record() {
        let shown = "ExecStart={ path=/home/x/.local/bin/kampr ; argv[]=/home/x/.local/bin/kampr \
                     serve --config-dir /home/x/.config/kampr ; ignore_errors=no ; pid=766702 }";
        assert_eq!(
            exec_start_path(shown),
            Some(PathBuf::from("/home/x/.local/bin/kampr"))
        );
        assert_eq!(
            exec_start_path("ExecStart="),
            None,
            "a unit nothing loaded names no binary"
        );
        assert_eq!(exec_start_path(""), None);
    }

    #[test]
    fn the_unit_template_is_fully_substituted() {
        let unit = SYSTEMD_UNIT
            .replace("@BIN@", "/usr/local/bin/kampr")
            .replace("@CONFIG_DIR@", "/home/x/.config/kampr")
            .replace("@STATE_DIR@", "/home/x/.local/state/kampr")
            .replace("@SOCKET@", "/home/x/.config/herdr/herdr.sock");
        assert!(!unit.contains('@'), "{unit}");
        assert!(unit.contains("ExecStart=/usr/local/bin/kampr serve"));
    }

    /// The operator, after a reboot: *"agents tell me they can't run sudo … i dont want this - it
    /// should act like any other terminal."*
    ///
    /// **`NoNewPrivileges` is inherited by every descendant and cannot be dropped again.** A herdr
    /// session created through `manage{session.create}` is a child of this process — the same fact
    /// `KillMode=process` exists for — so the bit reaches the session server, every pane in it, and
    /// every agent in those panes. After a reboot, where the node is what brings herdr up, that is
    /// every session on the machine, and `sudo` is a setuid binary: it fails outright.
    ///
    /// It was never worth what it cost. The comment defending it said the node "already drives
    /// every pane on the machine", which is the argument *against* it: an attacker who can type
    /// into every terminal does not need to escalate, and the operator who owns those terminals
    /// does. A pane Kampr serves is the operator's own shell and has to behave like one.
    #[test]
    fn the_unit_does_not_deny_escalation_to_the_operators_own_shells() {
        // Directives, not the word: the unit carries a comment naming this setting so nobody
        // reinstates it, and a test that matched the prose would forbid the explanation as well.
        let set: Vec<&str> = SYSTEMD_UNIT
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
            .filter(|line| line.starts_with("NoNewPrivileges"))
            .collect();
        assert!(
            set.is_empty(),
            "the node's panes are the operator's terminals; this bit reaches every one of them \
             and cannot be dropped once set: {set:?}",
        );
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
    fn a_reload_waits_out_a_bootout_that_has_not_finished() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().display();
        let launchctl = dir.path().join("launchctl");
        std::fs::write(
            &launchctl,
            format!(
                "#!/bin/sh\n\
                 case \"$1\" in\n\
                 bootout) echo 3 > {state}/busy ;;\n\
                 bootstrap)\n\
                   n=$(cat {state}/busy 2>/dev/null || echo 0)\n\
                   if [ \"$n\" -gt 0 ]; then echo $((n - 1)) > {state}/busy; exit 37; fi\n\
                   touch {state}/loaded ;;\n\
                 esac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&launchctl, std::fs::Permissions::from_mode(0o755)).unwrap();

        let loaded = reload_launchd(launchctl.to_str().unwrap(), Path::new("/x/dev.kampr.node.plist"));

        assert!(loaded);
        assert!(dir.path().join("loaded").exists());
    }

    #[test]
    fn the_plist_template_is_fully_substituted() {
        let plist = LAUNCHD_PLIST
            .replace("@BIN@", "/usr/local/bin/kampr")
            .replace("@CONFIG_DIR@", "/c")
            .replace("@STATE_DIR@", "/s")
            .replace("@SOCKET@", "/sock")
            .replace("@PATH@", "/bin");
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

    /// Every Mac reported FAIL after each login: a LaunchAgent that has run since then prints
    /// `(never exited)`, and anything but "0" was read as a failed service.
    #[test]
    fn only_a_nonzero_exit_code_is_a_failed_agent() {
        let print = |state: &str, exit: Option<&str>| {
            let mut out = format!("gui/501/dev.kampr.node = {{\n\tactive count = 1\n\tstate = {state}\n");
            if let Some(exit) = exit {
                out += &format!("\tlast exit code = {exit}\n");
            }
            out + "}\n"
        };
        for (name, text, running, failed, result) in [
            (
                "never exited",
                print("running", Some("(never exited)")),
                true,
                false,
                "never exited",
            ),
            ("clean exit", print("not running", Some("0")), false, false, "0"),
            ("exit 1", print("not running", Some("1")), false, true, "1"),
            (
                "named code",
                print("not running", Some("78: EX_CONFIG")),
                false,
                true,
                "78: EX_CONFIG",
            ),
            ("no line", print("running", None), true, false, "unknown"),
        ] {
            let state = parse_launchd(&text);
            assert!(state.installed, "{name}");
            assert_eq!(state.running, running, "{name}");
            assert_eq!(state.failed, failed, "{name}");
            assert_eq!(state.result, result, "{name}");
        }
        let absent = parse_launchd("");
        assert!(!absent.installed && !absent.failed && !absent.running);
    }

    #[test]
    fn a_launchd_unit_is_pointed_at_with_launchctl_in_its_own_domain() {
        let agent = parse_launchd("state = running");
        for hint in [
            agent.status_hint(Supervisor::Launchd),
            agent.restart_hint(Supervisor::Launchd),
            agent.start_hint(Supervisor::Launchd),
            agent.stop_hint(Supervisor::Launchd),
        ] {
            assert!(hint.starts_with("launchctl "), "{hint}");
            assert!(hint.ends_with("gui/$(id -u)/dev.kampr.node"), "{hint}");
        }
        assert!(agent.restart_hint(Supervisor::Launchd).contains("kickstart -k"));
        let daemon = State {
            daemon: true,
            ..agent.clone()
        };
        assert_eq!(
            daemon.status_hint(Supervisor::Launchd),
            "sudo launchctl print system/dev.kampr.node"
        );
        assert_eq!(
            agent.status_hint(Supervisor::Systemd),
            "systemctl --user status kampr.service"
        );
    }

    /// launchd's default PATH has none of the places `docker` is installed, so adopt and shell-in
    /// failed with "starting docker: No such file or directory".
    #[test]
    fn the_plist_gives_the_node_a_path_that_finds_docker_and_kampr() {
        let path = launchd_path("/Users/x", Some(Path::new("/Users/x/.orbstack/bin/docker")));
        let entries: Vec<&str> = path.split(':').collect();
        for want in [
            "/Users/x/.local/bin",
            "/usr/local/bin",
            "/opt/homebrew/bin",
            "/Users/x/.orbstack/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ] {
            assert!(entries.contains(&want), "{want} in {path}");
        }
        let linked = launchd_path("/Users/x", Some(Path::new("/usr/local/bin/docker")));
        assert_eq!(linked.matches("/usr/local/bin").count(), 1, "{linked}");

        let plist = LAUNCHD_PLIST
            .replace("@BIN@", "/k")
            .replace("@CONFIG_DIR@", "/c")
            .replace("@STATE_DIR@", "/s")
            .replace("@SOCKET@", "/sock")
            .replace("@PATH@", &path);
        assert_eq!(
            plist_path_env(&plist),
            path,
            "what doctor reads back is what was written"
        );
        assert_eq!(
            plist_path_env(&LAUNCHD_PLIST.replace("<key>PATH</key><string>@PATH@</string>", "")),
            LAUNCHD_DEFAULT_PATH,
            "a plist from before the PATH runs on launchd's"
        );
    }

    /// Every plist written before it carried a PATH, as launchd still holds it on a Mac that
    /// updated: a refresh keeps what it names and gains what the template now has.
    #[test]
    fn a_refresh_keeps_what_the_installed_plist_names_and_gains_the_path() {
        let old = LAUNCHD_PLIST
            .replace("\n    <key>PATH</key><string>@PATH@</string>", "")
            .replace("@BIN@", "/Users/x/.local/bin/kampr")
            .replace("@CONFIG_DIR@", "/Users/x/kc")
            .replace("@STATE_DIR@", "/Users/x/ks")
            .replace("@SOCKET@", "/Users/x/h.sock");
        assert!(!old.contains("<key>PATH</key>"), "{old}");
        let installed = Agent::read(&old).expect("the values an old plist carries");
        assert_eq!(installed.binary, Path::new("/Users/x/.local/bin/kampr"));

        let new = refreshed(&old, "/Users/x", Some(Path::new("/opt/homebrew/bin/docker")))
            .expect("a refreshed plist");
        for kept in [
            "/Users/x/.local/bin/kampr",
            "/Users/x/kc",
            "/Users/x/ks",
            "/Users/x/h.sock",
        ] {
            assert!(new.contains(&format!("<string>{kept}</string>")), "{kept}: {new}");
        }
        assert_eq!(
            plist_path_env(&new),
            launchd_path("/Users/x", Some(Path::new("/opt/homebrew/bin/docker")))
        );
        assert_eq!(refreshed("not a plist", "/Users/x", None), None);
    }

    fn packaged(name: &str) -> Option<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging")
            .join(name);
        std::fs::read_to_string(path).ok()
    }
}
