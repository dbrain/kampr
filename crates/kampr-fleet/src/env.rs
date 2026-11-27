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
//! **Read once, hand it to a shell that reads no profile at all.** Running every command under
//! `sh -lc` would work and costs two things this does not: whatever the profile prints lands in
//! the middle of the run's output, and a slow profile is paid on every host on every run.
//! Capturing the value once per node process ([`kampr_core::login`], which also says why the
//! reading is interactive) and putting it in the child's environment pays it once, and
//! [`crate::job`] holds why the shell that then runs the command is neither a login nor an
//! interactive one.

use kampr_core::login::once_each;
use std::sync::OnceLock;
use tracing::{debug, warn};

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
    /// Prepending it also fixes the case where the `PATH` *does* name a `kampr` that is not this
    /// one: an adopted container mounts the host's `/home`, so the login `PATH` answers the
    /// host's `kampr` — the wrong architecture for the container — and a shell that stops at the
    /// first match execs it and dies. The node runs from `own_bin`, so that is the copy that is
    /// right for this host, and first is the only place a shell is certain to look.
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
            // First, not last: a shell stops at the first `PATH` entry that names a command, and
            // the chosen `PATH` may answer `kampr` with a file that is not the one this node runs
            // from — an adopted container mounts the host's `/home`, and its `kampr` is the wrong
            // architecture for the container. The node runs from `own_bin`, so that copy is the
            // one that is right for this host, and it has to outrank the mounted one.
            Some(own) => format!("{own}:{value}"),
            None => value,
        }),
        origin,
    })
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
        let shell = kampr_core::login::shell();
        let captured = kampr_core::login::capture(&shell)
            .as_deref()
            .and_then(kampr_core::login::path_of)
            .map(str::to_string);
        match &captured {
            Some(path) => debug!(%shell, %path, "read the login shell's PATH"),
            None => warn!(%shell, "could not read the login shell's PATH; fleet runs get this process's"),
        }
        captured
    })
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
            "/home/u/.local/bin:/home/u/.atuin/bin:/usr/local/sbin:/usr/local/bin:/usr/bin",
        );
    }

    /// First, because a shell stops at the first `PATH` entry that names `kampr`. The chosen
    /// `PATH` may resolve it to a file that is not the one the node runs from, and the node is
    /// running from `own_bin` — so `own_bin` is the `kampr` that is right for this host, and it
    /// has to outrank whatever the `PATH` already answers.
    #[test]
    fn the_nodes_own_directory_outranks_what_the_chosen_path_already_resolves() {
        let chosen = choose(&PathSearch {
            configured: Some("/opt/kampr/bin:/usr/bin".into()),
            login: None,
            inherited: None,
            own_bin: Some("/home/u/.local/bin".into()),
        })
        .expect("something to run with");
        assert_eq!(chosen.origin, PathOrigin::Configured);
        assert_eq!(chosen.value, "/home/u/.local/bin:/opt/kampr/bin:/usr/bin");

        // And when the two are the same directory, putting it first changes nothing.
        let already = choose(&PathSearch {
            configured: None,
            login: Some("/home/u/.local/bin:/usr/bin".into()),
            inherited: None,
            own_bin: Some("/home/u/.local/bin".into()),
        })
        .expect("something to run with");
        assert_eq!(already.value, "/home/u/.local/bin:/usr/bin");
    }

    /// The case that broke `kampr update` across the herd. An adopted container mounts the host's
    /// `/home`, so the login `PATH` names the host's `kampr` — first — and a shell execs it and
    /// dies with `no such file or directory` (the host's binary is the wrong architecture for the
    /// container) without ever reaching the copy the container's node runs from. That copy is
    /// `own_bin`, and it has to come first.
    #[test]
    fn a_mounted_copy_of_the_operators_home_cannot_shadow_the_nodes_own_kampr() {
        let chosen = choose(&PathSearch {
            configured: None,
            login: Some("/home/dbrain/.local/bin:/usr/bin".into()),
            inherited: None,
            own_bin: Some("/.kampr".into()),
        })
        .expect("something to run with");
        assert_eq!(
            chosen.value, "/.kampr:/home/dbrain/.local/bin:/usr/bin",
            "the directory the node runs from must come before a mounted /home"
        );
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
}
