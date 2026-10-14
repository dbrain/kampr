//! `kampr doctor` against a real herdr. Every test here spawns a throwaway session, so this
//! module is what `make test-fast` leaves out.

use crate::cli::{Cli, Prefix, check, stdout, system_herdr};
use std::path::Path;
use std::process::{Command, Stdio};

/// A throwaway named session, stopped and deleted by the test that made it. `default` is never
/// touched.
struct Herd {
    socket: std::path::PathBuf,
}

/// How long a spawned herdr is given to open its socket before the wait is called a defect.
const LISTENS_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

impl Herd {
    // `herdr server` *is* the server: it runs until the socket is told to stop, which is what
    // `Drop` does. Waiting on it here would block for the life of the session.
    #[allow(clippy::zombie_processes)]
    fn start(tag: &str) -> Self {
        let herdr = kampr_testkit::herdr_on_path();
        let name = format!("kampr-doctor-{tag}-{}", std::process::id());
        assert_ne!(name, "default");
        let socket = herdr_home().join("sessions").join(&name).join("herdr.sock");
        Command::new("herdr")
            .args(["server", "--session", &name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawning `{} server --session {name}`: {e}", herdr.display()));
        for _ in 0..100 {
            if socket.exists() {
                std::thread::sleep(std::time::Duration::from_millis(300));
                return Self { socket };
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        kampr_testkit::herdr_never_listened(&name, &socket, LISTENS_WITHIN)
    }
}

impl Herd {
    /// A new named session is running within milliseconds and holds no panes at all (#240), and
    /// `terminal session observe` needs one.
    fn with_a_pane(self) -> Self {
        let made = Command::new("herdr")
            .args(["workspace", "create", "--label", "kampr-doctor", "--cwd"])
            .arg(std::env::temp_dir())
            .env("HERDR_SOCKET_PATH", &self.socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("herdr workspace create");
        assert!(made.success(), "could not make a pane to observe");
        std::thread::sleep(std::time::Duration::from_millis(500));
        self
    }
}

impl Drop for Herd {
    fn drop(&mut self) {
        let _ = Command::new("herdr")
            .args(["--session", session_name(&self.socket), "server", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::thread::sleep(std::time::Duration::from_millis(300));
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn session_name(socket: &Path) -> &str {
    socket
        .parent()
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("default")
}

fn herdr_home() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").expect("HOME")).join(".config"))
        .join("herdr")
}

/// The manifest declares a floor and nothing enforced it. This is that floor, against a herdr
/// that is actually running.
#[test]
fn doctor_reads_the_version_off_a_live_herdr_and_checks_it_against_the_floor() {
    let herd = Herd::start("live");
    let cli = Cli::new().against(&herd.socket);
    cli.init();
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    let herdr = check(&json, "herdr");
    assert_eq!(herdr["status"], "ok", "{herdr:#}");
    let detail = herdr["detail"].as_str().unwrap();
    assert!(detail.contains("protocol"), "{detail}");
    assert!(detail.contains("floor"), "{detail}");
}

/// **The answer to "why does Kampr say this agent is idle".**
///
/// A harness with no screen-detection manifest can only report its state through an integration,
/// and herdr 0.9 still ships no `omp` manifest (#511) — so an omp pane reads `idle` through a whole
/// working turn (#485) and nothing on the pane says why. `integration.list` is the one place that
/// does, and it is a pure read.
///
/// It reports and does not nag. `not_installed` is never a warning, which is the line herdr draws
/// for its own attention badge, and Kampr installs nothing either way.
#[test]
fn doctor_says_which_agents_can_tell_herdr_their_state_and_does_not_nag_about_the_rest() {
    let herd = Herd::start("integrations");
    let cli = Cli::new().against(&herd.socket);
    cli.init();
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    let check = check(&json, "integrations");
    assert_eq!(
        check["status"], "ok",
        "an integration nobody installed is not a fault: {check:#}"
    );
    let detail = check["detail"].as_str().unwrap();
    assert!(detail.contains("installed"), "{detail}");
    assert!(detail.contains("without one"), "{detail}");
    assert!(
        !detail.contains("out of date"),
        "nothing was outdated, so nothing may say so: {detail}"
    );
}

/// #233: the node that served a correct herd, accepted input, reported green, and showed a
/// blank grid in every client, because the half of herdr that streams is a spawned binary and
/// nothing had ever run it. `--version` answering is not that half.
#[test]
fn doctor_fails_a_herdr_that_answers_version_and_cannot_observe() {
    let herd = Herd::start("blind").with_a_pane();
    let cli = Cli::new().against(&herd.socket);
    cli.init();
    let shim = cli.nowhere.join("herdr");
    std::fs::create_dir_all(&cli.nowhere).expect("a directory for the shim");
    std::fs::write(
        &shim,
        "#!/bin/sh\ncase \"$1\" in --version) echo 'herdr 0.9.0' ;; *) exit 1 ;; esac\n",
    )
    .expect("a shim herdr");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    cli.pin_herdr(&shim);

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    let observe = check(&json, "observe");
    assert_eq!(observe["status"], "fail", "{observe:#}");
    let detail = observe["detail"].as_str().unwrap();
    assert!(detail.contains("does not stream"), "{detail}");
    assert!(
        detail.contains("herdr 0.9.0"),
        "the version it did answer: {detail}"
    );
}

/// And the other side of it: against a real herdr the check is only green because a frame came
/// back off a real `terminal session observe`.
#[test]
fn doctor_proves_the_stream_against_a_live_herdr_rather_than_asking_its_version() {
    let herd = Herd::start("stream").with_a_pane();
    let cli = Cli::new().against(&herd.socket);
    cli.init();
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    let observe = check(&json, "observe");
    assert_eq!(observe["status"], "ok", "{observe:#}");
    let detail = observe["detail"].as_str().unwrap();
    assert!(
        detail.contains("streamed a"),
        "no frame was ever asked for: {detail}"
    );
}

/// The machine this came from: the socket answers, so the `herdr` line was green and the node
/// passed `kampr doctor` — while the binary that streams every grid was nowhere the node could
/// find it, and every pane was blank for ever.
#[test]
fn doctor_fails_when_the_binary_that_streams_every_grid_is_nowhere_to_be_found() {
    if system_herdr() {
        eprintln!("skipped: this host has a herdr installed system-wide");
        return;
    }
    let herd = Herd::start("observe");
    let empty = Prefix::new();
    let cli = Cli::new()
        .against(&herd.socket)
        .with_env("PATH", "")
        .with_env("HOME", &empty.home());
    cli.init();

    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(
        check(&json, "herdr")["status"],
        "ok",
        "the socket is the half that was never broken: {json:#}"
    );
    let observe = check(&json, "observe");
    assert_eq!(observe["status"], "fail", "{json:#}");
    let detail = observe["detail"].as_str().unwrap();
    assert!(detail.contains("herdr"), "{detail}");
    assert!(detail.contains("blank") || detail.contains("stream"), "{detail}");
    assert!(
        observe["fix"].as_str().unwrap().contains("install herdr"),
        "{observe:#}"
    );
    assert!(
        !out.status.success(),
        "a node that can never show a pane is not a healthy node"
    );
}
