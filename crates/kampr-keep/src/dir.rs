use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SPEC: &str = "spec.json";
pub const KEEPER: &str = "keeper.json";
pub const EXIT: &str = "exit.json";
pub const SIZE: &str = "size.json";
pub const CTL: &str = "ctl.sock";
pub const HOSTS: &str = "hosts";
const HOSTS_LOCK: &str = "hosts.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub id: String,
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_cap: Option<u64>,
    pub created_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cohort: Option<String>,
}

impl Spec {
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            id: new_id(),
            argv,
            cwd: None,
            env: inherited(std::env::vars()),
            cols: 80,
            rows: 24,
            label: None,
            log_cap: None,
            created_unix: now_unix(),
            cohort: None,
        }
    }
}

/// What a terminal, a multiplexer or a harness set to name the session they are running. Copied
/// into a new session they name the wrong one, and one of them is a token: `CLAUDECODE` alone makes
/// a `claude` started there refuse to run as if nested.
const SESSION_SCOPED: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SSE_PORT",
    "KAMPR_SESSION_ID",
    // The journal names an omp/pi pane's conversation by these, read live out of
    // `/proc/<pid>/environ`: inherited, a shell would be taken for the conversation it came from.
    "PI_SESSION_FILE",
    "PI_SUBAGENT_PARENT_SESSION",
    "PI_SESSION_ID",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "WINDOW",
    "WINDOWID",
    "KITTY_WINDOW_ID",
    "KITTY_PID",
    "WEZTERM_PANE",
    "ALACRITTY_WINDOW_ID",
    "TERM_SESSION_ID",
    "ATUIN_SESSION",
    "ATUIN_SHLVL",
    "ATUIN_TMUX_POPUP",
    "BLE_SESSION_ID",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "_",
];
const SESSION_SCOPED_PREFIXES: &[&str] = &["HERDR_", "VSCODE_"];

fn inherited(vars: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    vars.into_iter()
        .filter(|(k, _)| {
            !SESSION_SCOPED.contains(&k.as_str()) && !SESSION_SCOPED_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .collect()
}

pub fn new_id() -> String {
    ulid::Ulid::generate().to_string().to_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum How {
    #[serde(rename = "spawn")]
    Spawn,
    #[serde(rename = "re-exec")]
    ReExec,
    #[serde(rename = "fd store")]
    FdStore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperInfo {
    pub keeper: u32,
    pub child: u32,
    pub exe: String,
    pub started_unix: u64,
    pub how: How,
    /// The file at `exe` when this keeper started. `None` from a keeper older than the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub dev: u64,
    pub ino: u64,
}

impl Image {
    pub fn of(meta: &fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

/// A keeper that died without writing `exit.json` and left no child behind is `Lost`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Exited(Exit),
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    pub status_lost: bool,
    pub at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub holder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_socket: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// The log offset this host takes the terminal's questions from: a query that ends past it is
    /// its terminal's to answer, and one ending at or before it is not. `register_host` sets it.
    /// `None` is a file from an attach older than the field, which answers everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers_from: Option<u64>,
}

impl Host {
    pub fn covers(&self, query_end: u64) -> bool {
        self.answers_from.is_none_or(|from| query_end > from)
    }
}

pub fn root() -> io::Result<PathBuf> {
    let root = match std::env::var_os("KAMPR_KEEP_ROOT") {
        Some(root) => PathBuf::from(root),
        None => match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(run) => PathBuf::from(run).join("kampr").join("s"),
            None => PathBuf::from(format!("/tmp/kampr-{}", rustix::process::getuid().as_raw())).join("s"),
        },
    };
    make_private_dir(&root)?;
    Ok(root)
}

pub fn make_private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    check_private(path)
}

// Whoever can replace a directory on this path can serve their own `ctl.sock`, and the node would
// write the operator's keystrokes into the fd it hands back. The `/tmp` fallback makes that a real
// path: another uid can create `/tmp/kampr-<uid>` first.
pub(crate) fn check_private(path: &Path) -> io::Result<()> {
    let uid = rustix::process::getuid().as_raw();
    let own = fs::symlink_metadata(path)?;
    if !own.is_dir() || own.uid() != uid || own.mode() & 0o077 != 0 {
        return Err(refused(path, "is not a directory private to this uid"));
    }
    for ancestor in path.ancestors().skip(1).filter(|a| !a.as_os_str().is_empty()) {
        let meta = fs::metadata(ancestor)?;
        let sticky = meta.mode() & 0o1000 != 0;
        if (meta.uid() != uid && meta.uid() != 0) || (meta.mode() & 0o022 != 0 && !sticky) {
            return Err(refused(ancestor, "could be replaced by another uid"));
        }
    }
    Ok(())
}

fn refused(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} {why}, so no session is kept under it", path.display()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDir {
    path: PathBuf,
}

impl SessionDir {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn id(&self) -> &str {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn spec(&self) -> io::Result<Spec> {
        read_json(&self.path.join(SPEC))
    }

    pub fn keeper(&self) -> Option<KeeperInfo> {
        read_json(&self.path.join(KEEPER)).ok()
    }

    pub fn exit(&self) -> Option<Exit> {
        read_json(&self.path.join(EXIT)).ok()
    }

    /// `None` while the keeper or its child lives, and before a keeper has started at all.
    pub fn ending(&self) -> Option<Ending> {
        if let Some(exit) = self.exit() {
            return Some(Ending::Exited(exit));
        }
        if self.keeper().is_none() || self.alive() {
            return None;
        }
        // The keeper writes exit.json and then exits, so one gone by the second look may have
        // written it after the first.
        Some(self.exit().map_or(Ending::Lost, Ending::Exited))
    }

    pub fn size(&self) -> Option<Size> {
        read_json(&self.path.join(SIZE)).ok()
    }

    pub fn hosts(&self) -> Vec<Host> {
        let Ok(entries) = fs::read_dir(self.path.join(HOSTS)) else {
            return Vec::new();
        };
        let mut hosts: Vec<Host> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read_json::<Host>(&e.path()).ok())
            .filter(|h| pid_alive(h.pid))
            .collect();
        hosts.sort_by_key(|h| h.pid);
        hosts
    }

    /// The live hosts as of a moment no host was registering, so a decision about who answers a
    /// query the caller has already read out of the log cannot race the registration that
    /// would have taken the query from it.
    pub fn hosts_settled(&self) -> io::Result<Vec<Host>> {
        let _lock = hosts_lock(self)?;
        Ok(self.hosts())
    }

    pub fn alive(&self) -> bool {
        if self.path.join(EXIT).exists() {
            return false;
        }
        self.keeper()
            .is_some_and(|k| pid_alive(k.keeper) || pid_alive(k.child))
    }
}

pub fn list(root: &Path) -> io::Result<Vec<SessionDir>> {
    check_private(root)?;
    let mut found: Vec<SessionDir> = fs::read_dir(root)?
        .flatten()
        .filter(|e| e.path().join(SPEC).is_file())
        .map(|e| SessionDir::at(e.path()))
        .collect();
    found.sort_by(|a, b| a.id().cmp(b.id()));
    Ok(found)
}

// Fleet runs keep their sessions in roots of their own below this one, and an upgrade or a doctor
// that missed them would leave those keepers on an old binary for ever.
pub fn list_tree(root: &Path) -> io::Result<Vec<SessionDir>> {
    let mut found = list(root)?;
    for entry in fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir())
            && !path.join(SPEC).exists()
            && let Ok(nested) = list_tree(&path)
        {
            found.extend(nested);
        }
    }
    Ok(found)
}

pub struct HostGuard {
    path: PathBuf,
    host: Host,
}

impl HostGuard {
    pub fn update(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        self.host.cols = cols;
        self.host.rows = rows;
        write_json(&self.path, &self.host)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn host(&self) -> &Host {
        &self.host
    }
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn register_host(session: &SessionDir, mut host: Host) -> io::Result<HostGuard> {
    let dir = session.path.join(HOSTS);
    make_private_dir(&dir)?;
    let path = dir.join(format!("{}.json", host.pid));
    let _lock = hosts_lock(session)?;
    host.answers_from = Some(crate::log::bounds(&session.path)?.1);
    write_json(&path, &host)?;
    Ok(HostGuard { path, host })
}

fn hosts_lock(session: &SessionDir) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(session.path.join(HOSTS_LOCK))?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)?;
    Ok(file)
}

pub(crate) fn pid_alive(pid: u32) -> bool {
    crate::sys::pid_alive(pid)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, serde_json::to_vec(value).map_err(io::Error::other)?)?;
    fs::rename(&tmp, path)
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::inherited;

    #[test]
    fn a_new_session_inherits_the_environment_but_not_the_session_it_was_made_from() {
        let vars = [
            "PATH",
            "HOME",
            "SSH_AUTH_SOCK",
            "CLAUDE_CODE_DISABLE_MOUSE_CLICKS",
            "CLAUDECODE",
            "CLAUDE_PID",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "CLAUDE_CODE_MESSAGING_SOCKET",
            "CLAUDE_CODE_ENTRYPOINT",
            "PI_SESSION_FILE",
            "PI_SUBAGENT_PARENT_SESSION",
            "PI_SESSION_ID",
            "TMUX",
            "TMUX_PANE",
            "KAMPR_SESSION_ID",
            "HERDR_PANE_ID",
            "ATUIN_SESSION",
            "BLE_SESSION_ID",
            "VSCODE_IPC_HOOK_CLI",
            "PWD",
            "OLDPWD",
            "SHLVL",
            "_",
        ]
        .map(|k| (k.to_string(), "v".to_string()));
        let kept: Vec<String> = inherited(vars).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            kept,
            [
                "PATH",
                "HOME",
                "SSH_AUTH_SOCK",
                "CLAUDE_CODE_DISABLE_MOUSE_CLICKS"
            ]
        );
    }
}
