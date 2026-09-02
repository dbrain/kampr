//! The `PATH` a fleet run is given, and why it is not this process's.
//!
//! A fleet run is forked by the node and the node is a service, so its `PATH` is the service
//! manager's: measured on this machine as `/usr/local/sbin:/usr/local/bin:/usr/bin` and nothing
//! else, against a login shell's which carries `~/.local/bin` and everything the operator's
//! profile puts there (#392). `kampr update` across the herd therefore looked for a binary in
//! three directories it was never installed into. It is the same lesson [`kampr_herdr::locate`]
//! already records for the herdr binary, one layer out: a service manager's `PATH` is not the
//! installing shell's.
//!
//! **Read once, then exec directly.** Running every command under `sh -lc` would work too and
//! costs three things this does not: the argv is re-parsed by a shell, so a filename with a space
//! in it stops being one argument; whatever the profile prints lands in the middle of the run's
//! output; and a slow profile is paid on every host on every run. Capturing the value once and
//! handing it to an ordinary `exec` keeps the argv exactly as the operator wrote it.

use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;
use tracing::{debug, warn};

/// How long the login shell is given to say what its `PATH` is. A profile that waits on something
/// is a profile that would otherwise hang the first fleet run on that host for ever; overrunning
/// this is not fatal, it just means the run gets this process's `PATH`, which is what it got
/// before any of this existed.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// Every place a fleet `PATH` can come from, as values rather than environment reads, so the order
/// is a table test rather than a process the suite has to fork. Same shape, and the same reason,
/// as `kampr_herdr::locate::Search`.
#[derive(Debug, Clone, Default)]
pub struct PathSearch {
    /// `fleet.path` in the node's config: the operator has said what they want and nothing
    /// overrides them. The escape hatch for a shell whose `PATH` a login shell does not build —
    /// zsh puts it in `.zshrc` as often as in `.zprofile`, and `-l` does not read `.zshrc`.
    pub configured: Option<String>,
    /// What the operator's own login shell answers.
    pub login: Option<String>,
    /// This process's, which is the service manager's.
    pub inherited: Option<String>,
    /// The directory this node's own binary is in.
    ///
    /// Measured on the operator's own herd (#419): `~/.local/bin` is where `kampr` and `herdr` are
    /// installed on all four hosts, and on two of them the **login shell's** `PATH` does not carry
    /// it either — the profile that adds it is `.bashrc`, which `-l` does not read. So the rung
    /// above is the right `PATH` and still cannot find the binary it is running from, and
    /// `kampr update` across the herd fails on those hosts with a bare `kampr` exactly as it did
    /// before any of this existed.
    ///
    /// Appended rather than preferred: a name the chosen `PATH` already resolves goes on resolving
    /// to the same file, so this can only add answers and never change one.
    pub own_bin: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathOrigin {
    Configured,
    Login,
    Inherited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetPath {
    pub value: String,
    pub origin: PathOrigin,
}

/// Empty is not an answer at any rung. A shell that printed nothing has told us nothing, and
/// putting an empty `PATH` on a child would break every run rather than the ones that were
/// already broken.
pub fn choose(search: &PathSearch) -> Option<FleetPath> {
    let rungs = [
        (PathOrigin::Configured, &search.configured),
        (PathOrigin::Login, &search.login),
        (PathOrigin::Inherited, &search.inherited),
    ];
    let chosen = rungs.iter().find_map(|(origin, value)| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| (*origin, v.to_string()))
    })?;
    let own = search.own_bin.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let (origin, value) = chosen;
    Some(FleetPath {
        value: once_each(&match own {
            Some(own) => format!("{value}:{own}"),
            None => value,
        }),
        origin,
    })
}

/// A login shell run from a shell that already had a `PATH` appends to it rather than replacing
/// it, so an operator's own reads back with `~/.local/bin` in it three times. A later duplicate is
/// never reached — a lookup takes the first match — so dropping it changes nothing except whether
/// a person can read the line `kampr doctor` prints.
fn once_each(path: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    path.split(':')
        .filter(|entry| seen.insert(*entry))
        .collect::<Vec<_>>()
        .join(":")
}

/// What a fleet run on this host will actually be given.
pub fn fleet_path(configured: Option<String>) -> Option<FleetPath> {
    choose(&PathSearch {
        configured,
        login: login_path().clone(),
        inherited: std::env::var("PATH").ok(),
        own_bin: own_bin(),
    })
}

/// The directory this process's own executable is in, which is where the operator installed it and
/// therefore where `herdr` is too.
fn own_bin() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.to_str()?.to_string())
}

/// The login shell's `PATH`, read once for the life of the process.
///
/// Cached rather than re-read because it cannot change under us in a way that matters: a profile
/// edited mid-session is picked up by restarting the node, which is what picks up every other
/// config change here.
pub fn login_path() -> &'static Option<String> {
    static CAPTURED: OnceLock<Option<String>> = OnceLock::new();
    CAPTURED.get_or_init(|| {
        let shell = login_shell();
        let captured = capture(&shell);
        match &captured {
            Some(path) => debug!(%shell, %path, "read the login shell's PATH"),
            None => warn!(%shell, "could not read the login shell's PATH; fleet runs get this process's"),
        }
        captured
    })
}

/// `$SHELL`, then the passwd entry, then `/bin/sh`. A service manager often sets no `$SHELL` at
/// all, which is exactly the case this has to answer.
pub fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(passwd_shell)
        .unwrap_or_else(|| "/bin/sh".to_string())
}

fn passwd_shell() -> Option<String> {
    // SAFETY: `getpwuid` returns a pointer into a static buffer this reads and copies before
    // anything else can call it — a `OnceLock` initialiser, once per process.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() {
            return None;
        }
        let shell = (*entry).pw_shell;
        if shell.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(shell).to_str().ok().map(str::to_string)
    }
}

/// The marker exists because a profile prints things — a message of the day, a version notice, a
/// `direnv` line. What is wanted is between the two NULs and everything else on the stream is
/// somebody saying hello.
fn capture(shell: &str) -> Option<String> {
    let shell = shell.to_string();
    let (tx, rx) = mpsc::sync_channel(1);
    // On a thread, and abandoned rather than killed if it overruns: `output()` drains the child's
    // pipes, which is what stops a chatty profile from deadlocking the read, and a child left to
    // finish a profile costs one short-lived process against a node that hangs.
    std::thread::spawn(move || {
        let out = std::process::Command::new(&shell)
            .args(["-lc", "printf '\\0%s\\0' \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(CAPTURE_TIMEOUT).ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    between_markers(&String::from_utf8_lossy(&out.stdout))
}

fn between_markers(text: &str) -> Option<String> {
    let mut parts = text.split('\0');
    parts.next()?;
    let path = parts.next()?.trim();
    (!path.is_empty()).then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_operators_own_answer_outranks_every_reading() {
        let search = PathSearch {
            configured: Some("/opt/kampr/bin".into()),
            login: Some("/home/u/.local/bin:/usr/bin".into()),
            inherited: Some("/usr/bin".into()),
            own_bin: None,
        };
        let chosen = choose(&search).expect("something to run with");
        assert_eq!(chosen.value, "/opt/kampr/bin");
        assert_eq!(chosen.origin, PathOrigin::Configured);
    }

    /// The whole point: the login shell's is preferred to this process's, because this process is
    /// a service and its own is the one that has `~/.local/bin` missing from it (#392).
    #[test]
    fn a_login_shells_path_beats_the_service_managers() {
        let chosen = choose(&PathSearch {
            configured: None,
            login: Some("/home/u/.local/bin:/usr/bin".into()),
            inherited: Some("/usr/local/sbin:/usr/local/bin:/usr/bin".into()),
            own_bin: None,
        })
        .expect("something to run with");
        assert_eq!(chosen.value, "/home/u/.local/bin:/usr/bin");
        assert_eq!(chosen.origin, PathOrigin::Login);
    }

    /// And when the shell could not be read at all, a fleet run still runs — with what it had
    /// before any of this existed, rather than with nothing.
    #[test]
    fn a_shell_that_said_nothing_leaves_the_run_exactly_as_it_was() {
        let chosen = choose(&PathSearch {
            configured: None,
            login: None,
            inherited: Some("/usr/bin".into()),
            own_bin: None,
        })
        .expect("something to run with");
        assert_eq!(chosen.origin, PathOrigin::Inherited);
        assert_eq!(choose(&PathSearch::default()), None);
    }

    /// An empty answer is not an answer. A shell that printed nothing would otherwise put an empty
    /// `PATH` on every child and break the runs that were working.
    #[test]
    fn an_empty_reading_is_skipped_rather_than_used() {
        let chosen = choose(&PathSearch {
            configured: Some(String::new()),
            login: Some("   ".into()),
            inherited: Some("/usr/bin".into()),
            own_bin: None,
        })
        .expect("something to run with");
        assert_eq!(chosen.origin, PathOrigin::Inherited);
    }

    /// Read from a shell that already had one, a login `PATH` comes back with the same directory
    /// in it several times. The first is the one every lookup uses.
    #[test]
    fn a_directory_named_twice_is_kept_once_and_in_its_first_place() {
        let chosen = choose(&PathSearch {
            configured: None,
            login: Some("/home/u/.local/bin:/usr/bin:/home/u/.local/bin:/usr/local/bin:/usr/bin".into()),
            inherited: None,
            own_bin: None,
        })
        .expect("something to run with");
        assert_eq!(chosen.value, "/home/u/.local/bin:/usr/bin:/usr/local/bin");
    }

    #[test]
    fn the_path_is_taken_from_between_the_markers_and_not_from_the_profiles_hello() {
        assert_eq!(
            between_markers("Welcome to node-07\n\0/home/u/.local/bin:/usr/bin\0"),
            Some("/home/u/.local/bin:/usr/bin".to_string()),
        );
        assert_eq!(between_markers("no markers at all"), None);
    }

    /// Measured on the operator's own herd: two of four hosts install `kampr` and `herdr` into
    /// `~/.local/bin` and their **login shell** does not carry it — `giftofthemagi2` answers
    /// `/home/dbrain/.bun/bin:/home/dbrain/.atuin/bin:/usr/local/sbin:...` and `artifactone` the
    /// same without `.bun`. So the rung that was supposed to fix this reads the right shell and
    /// still cannot resolve the binary the node is running from.
    #[test]
    fn a_login_shell_without_the_directory_the_node_was_installed_into_can_still_find_it() {
        let chosen = choose(&PathSearch {
            configured: None,
            login: Some("/home/u/.atuin/bin:/usr/local/sbin:/usr/local/bin:/usr/bin".into()),
            inherited: Some("/usr/bin".into()),
            own_bin: Some("/home/u/.local/bin".into()),
        })
        .expect("something to run with");
        assert_eq!(chosen.origin, PathOrigin::Login);
        assert_eq!(
            chosen.value,
            "/home/u/.atuin/bin:/usr/local/sbin:/usr/local/bin:/usr/bin:/home/u/.local/bin",
        );
    }

    /// Last, never first. A `kampr` the operator's own `PATH` already resolves goes on resolving to
    /// the same file — this rung may add an answer and may never change one — and that holds for
    /// the rung whose whole contract is that nothing overrides it.
    #[test]
    fn the_nodes_own_directory_never_displaces_a_command_the_chosen_path_already_finds() {
        let chosen = choose(&PathSearch {
            configured: Some("/opt/kampr/bin:/usr/bin".into()),
            login: None,
            inherited: None,
            own_bin: Some("/home/u/.local/bin".into()),
        })
        .expect("something to run with");
        assert_eq!(chosen.origin, PathOrigin::Configured);
        assert_eq!(chosen.value, "/opt/kampr/bin:/usr/bin:/home/u/.local/bin");

        let already = choose(&PathSearch {
            configured: None,
            login: Some("/home/u/.local/bin:/usr/bin".into()),
            inherited: None,
            own_bin: Some("/home/u/.local/bin".into()),
        })
        .expect("something to run with");
        assert_eq!(already.value, "/home/u/.local/bin:/usr/bin");
    }

    /// A binary run out of a build directory, or one whose parent cannot be read, leaves the rung
    /// empty — and an empty rung adds nothing rather than a trailing colon, which is `.` to every
    /// lookup that reads it.
    #[test]
    fn a_node_that_cannot_say_where_it_lives_adds_nothing_to_the_path() {
        let chosen = choose(&PathSearch {
            configured: None,
            login: Some("/usr/bin".into()),
            inherited: None,
            own_bin: Some("   ".into()),
        })
        .expect("something to run with");
        assert_eq!(chosen.value, "/usr/bin");
    }

    /// The node is running from somewhere, and that somewhere is the whole of this rung.
    #[test]
    fn a_running_node_knows_which_directory_it_was_started_from() {
        let own = own_bin().expect("a running test binary has a parent directory");
        assert!(std::path::Path::new(&own).is_dir(), "{own} is not a directory");
    }

    /// Not a fixed string: a machine's login shell is whatever it is. What has to hold is that
    /// something is always named, because the fallback is what a service manager with no `$SHELL`
    /// gets.
    #[test]
    fn a_shell_is_always_named_even_with_nothing_in_the_environment() {
        assert!(!login_shell().is_empty());
    }
}
