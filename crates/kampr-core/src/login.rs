//! The operator's login shell, and the environment it builds, read the way their own terminal has
//! it. A node run as a service has the service manager's environment: measured on this machine as
//! `/usr/local/sbin:/usr/local/bin:/usr/bin` and nothing else (#392), against a login shell's
//! which carries `~/.local/bin` and everything the operator's profile puts there. A fleet run, a
//! headless harness child and the lookup of a bare harness binary all read it from here.
//!
//! **The reading is interactive, which is #419 closing.** `-l` reads `.bash_profile` and
//! `.profile`; `~/.local/bin` (where `kampr`, `herdr` and `claude` are installed on half this
//! herd) is added by `.bashrc`, which only an interactive shell reads. Measured from `env -i`:
//!
//! | invocation | answers |
//! |---|---|
//! | `bash -c` | `/usr/local/sbin:/usr/local/bin:/usr/bin`, the service manager's (#392) |
//! | `bash -lc` | that, plus `~/.local/bin`, flatpak, jvm, perl, rustup, Toolbox |
//! | `bash -ic` | `~/.local/bin`, `~/.nvm/.../bin`, `~/go/bin`, the Android SDK, then the system three |
//! | `bash -lic` | the union of both, and the only reading that is a superset of the operator's shell |
//!
//! So `-lic` is asked first and `-lc` is the fallback for a shell that will not take it. `-ic` is
//! asked as well and its `PATH` appended, because the union above is a distribution's doing:
//! Arch's `.bash_profile` sources `.bashrc` and a Mac's does not. All three run concurrently
//! under one deadline, because a fallback that only starts when the first has given up doubles the
//! worst case. Measured here at 107 ms for `-lc` and 398 ms for `-lic`.
//!
//! An interactive shell with no tty is noisy on stderr, which is discarded; its stdout was
//! measured clean, and the marker holds either way.

use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::debug;

pub type Env = Vec<(String, String)>;

/// A profile that waits on something must not hold a node's start or a launch open. Overrunning
/// is not fatal: the caller falls back to this process's environment.
const DEADLINE: Duration = Duration::from_secs(5);

const READINGS: [&str; 2] = ["-lic", "-lc"];
const INTERACTIVE: &str = "-ic";

/// A profile can print a greeting first; what follows the marker is `env -0`'s alone.
const MARKER: &[u8] = b"\0kampr-login-env\0";
const PROBE: &str = r"printf '\0kampr-login-env\0'; exec env -0";

/// The capturing shell's own bookkeeping, true of it and not of a child started somewhere else.
const THE_SHELLS_OWN: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];

/// `$SHELL`, then the passwd entry, then `/bin/sh`: a service manager often sets no `$SHELL`.
pub fn shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(passwd_shell)
        .unwrap_or_else(|| "/bin/sh".to_string())
}

fn passwd_shell() -> Option<String> {
    // SAFETY: `getpwuid` returns a pointer into a static buffer, copied out before returning.
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

/// Blocking, for up to [`DEADLINE`]. `None` when no reading answered.
pub fn capture(shell: &str) -> Option<Env> {
    capture_with(shell, ask)
}

pub fn path_of(env: &[(String, String)]) -> Option<&str> {
    env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.as_str())
}

/// A login shell started from a shell that already had a `PATH` appends to it; a later duplicate is
/// never reached, so dropping it changes nothing but what a person reads.
pub fn once_each(path: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    path.split(':')
        .filter(|entry| seen.insert(*entry))
        .collect::<Vec<_>>()
        .join(":")
}

fn capture_with(shell: &str, ask: fn(&str, &str) -> Option<Env>) -> Option<Env> {
    let (tx, rx) = mpsc::channel();
    // Abandoned rather than killed if they overrun: `output()` drains the pipes, which is what
    // stops a chatty profile deadlocking the read.
    for (rank, flags) in READINGS.iter().chain([&INTERACTIVE]).enumerate() {
        let shell = shell.to_string();
        let flags = flags.to_string();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((rank, ask(&shell, &flags)));
        });
    }
    drop(tx);

    let deadline = Instant::now() + DEADLINE;
    let mut best: Option<(usize, Env)> = None;
    let mut interactive: Option<Env> = None;
    for _ in 0..=READINGS.len() {
        let Ok((rank, answer)) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) else {
            break;
        };
        let Some(answer) = answer else { continue };
        if rank == READINGS.len() {
            interactive = Some(answer);
        } else if best.as_ref().is_none_or(|(seen, _)| rank < *seen) {
            best = Some((rank, answer));
        }
        if best.as_ref().is_some_and(|(rank, _)| *rank == 0) && interactive.is_some() {
            break;
        }
    }
    let Some((rank, mut env)) = best else {
        return interactive;
    };
    debug!(%shell, flags = READINGS[rank], "read the login environment");
    if let Some(extra) = interactive.as_deref().and_then(path_of) {
        match env.iter_mut().find(|(k, _)| k == "PATH") {
            Some((_, path)) => *path = once_each(&format!("{path}:{extra}")),
            None => env.push(("PATH".into(), extra.into())),
        }
    }
    Some(env)
}

fn ask(shell: &str, flags: &str) -> Option<Env> {
    let out = std::process::Command::new(shell)
        .args([flags, PROBE])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse(&out.stdout)
}

fn parse(stdout: &[u8]) -> Option<Env> {
    let at = stdout.windows(MARKER.len()).position(|w| w == MARKER)?;
    let env: Env = stdout[at + MARKER.len()..]
        .split(|b| *b == 0)
        .filter_map(|kv| std::str::from_utf8(kv).ok()?.split_once('='))
        .filter(|(k, _)| !k.is_empty() && !THE_SHELLS_OWN.contains(k))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (!env.is_empty()).then_some(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_greeting_is_skipped_and_the_shells_own_names_and_odd_values_are_kept_right() {
        let mut out = b"welcome back\n".to_vec();
        out.extend_from_slice(MARKER);
        out.extend_from_slice(b"PATH=/p:/usr/bin\0PWD=/somewhere\0SHLVL=2\0_=/usr/bin/env\0");
        out.extend_from_slice(b"MULTI=a\nb=c\0");
        let env = parse(&out).expect("an environment");
        assert_eq!(
            env,
            [
                ("PATH".to_string(), "/p:/usr/bin".to_string()),
                ("MULTI".to_string(), "a\nb=c".to_string())
            ]
        );
        assert_eq!(parse(b"no marker at all"), None);
    }

    #[test]
    fn a_shell_is_always_named_even_with_nothing_in_the_environment() {
        assert!(!shell().is_empty());
        assert!(passwd_shell().is_some_and(|s| s.starts_with('/')));
    }

    #[test]
    fn a_directory_named_twice_is_kept_once_and_in_its_first_place() {
        assert_eq!(once_each("/a:/b:/a:/c:/b"), "/a:/b:/c");
    }

    #[test]
    fn a_path_only_bashrc_adds_is_read_and_a_shell_that_says_nothing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bashrc-bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".bashrc"),
            format!("export PATH='{}':\"$PATH\"\n", bin.display()),
        )
        .unwrap();
        std::fs::write(home.join(".profile"), "export FROM_PROFILE=1\n").unwrap();
        let shell = dir.path().join("login-bash");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nHOME='{}' PATH=/usr/bin:/bin exec /bin/bash \"$@\"\n",
                home.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = capture(&shell.display().to_string()).expect("an environment");
        let path = path_of(&env).expect("a PATH");
        assert!(path.split(':').any(|d| d == bin.display().to_string()), "{path}");
        assert!(
            env.iter().any(|(k, v)| k == "FROM_PROFILE" && v == "1"),
            "{env:?}"
        );
        assert_eq!(capture("/bin/false"), None);
    }

    /// Against this machine's own shell, from a cleared environment: whatever an interactive shell
    /// has and a login shell lacks, the capture must carry (#419). Skips loudly when there is
    /// nothing to prove, rather than passing quietly (#233).
    #[test]
    fn the_capture_carries_what_the_operators_own_terminal_resolves() {
        fn from_a_service_manager(shell: &str, flags: &str) -> Option<Env> {
            let out = std::process::Command::new(shell)
                .env_clear()
                .env("HOME", std::env::var("HOME").unwrap_or_default())
                .args([flags, PROBE])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .ok()?;
            out.status.success().then(|| parse(&out.stdout)).flatten()
        }
        let shell = shell();
        let path = |env: Option<Env>| env.as_deref().and_then(path_of).map(str::to_string);
        let (Some(first), Some(interactive), Some(login)) = (
            path(capture_with(&shell, from_a_service_manager)),
            path(from_a_service_manager(&shell, INTERACTIVE)),
            path(from_a_service_manager(&shell, "-lc")),
        ) else {
            eprintln!("SKIPPED: {shell} did not answer all three readings; nothing was asserted");
            return;
        };
        let only_interactive: Vec<&str> = interactive
            .split(':')
            .filter(|dir| !login.split(':').any(|d| d == *dir))
            .collect();
        if only_interactive.is_empty() {
            eprintln!("SKIPPED: this machine's interactive shell adds no directory a login shell lacks");
            return;
        }
        let missing: Vec<&str> = only_interactive
            .into_iter()
            .filter(|dir| !first.split(':').any(|d| d == *dir))
            .collect();
        assert!(
            missing.is_empty(),
            "{shell} resolves {missing:?} in a terminal and the capture lacks them"
        );
    }
}
