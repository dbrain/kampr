//! The two operator commands, driven as an operator drives them: the real binary, a throwaway
//! config and state directory, and nothing else on the machine touched.

use std::path::Path;
use std::process::{Command, Output, Stdio};

struct Cli {
    config: std::path::PathBuf,
    state: std::path::PathBuf,
    socket: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Cli {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        Self {
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            socket: dir.path().join("herdr.sock"),
            _dir: dir,
        }
    }

    fn against(mut self, socket: &Path) -> Self {
        self.socket = socket.to_path_buf();
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_stdin(args, "")
    }

    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kampr"))
            .args(args)
            .arg("--config-dir")
            .arg(&self.config)
            .arg("--state-dir")
            .arg(&self.state)
            // The doctor reports on the environment the node would run in, so a test points it
            // at a socket that is not the developer's own herd.
            .env("HERDR_SOCKET_PATH", &self.socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kampr");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("kampr output")
    }

    fn init(&self) -> String {
        let out = self.run(&["init"]);
        assert!(out.status.success(), "{}", stderr(&out));
        stdout(&out)
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code_after(text: &str, label: &str) -> String {
    text.lines()
        .find(|l| l.contains(label))
        .and_then(|l| l.split_whitespace().next_back())
        .unwrap_or_else(|| panic!("no {label} in:\n{text}"))
        .to_string()
}

fn devices(cli: &Cli) -> String {
    let out = cli.run(&["doctor", "--json"]);
    stdout(&out)
}

fn check<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no check {id} in {json:#}"))
}

#[test]
fn init_prints_a_recovery_code_once_and_never_again() {
    let cli = Cli::new();
    let first = cli.init();
    let code = code_after(&first, "RECOVERY CODE");
    assert_eq!(code.split('-').count(), 5, "{code}");

    let again = stdout(&cli.run(&["init"]));
    assert!(
        !again.contains("RECOVERY CODE"),
        "a second init must not reissue or reprint it:\n{again}"
    );
    assert!(!again.contains(&code));
}

#[test]
fn a_recovery_code_gets_back_in_when_every_device_is_gone() {
    let cli = Cli::new();
    let code = code_after(&cli.init(), "RECOVERY CODE");

    let redeemed = cli.run_with_stdin(&["recover"], &format!("{code}\n"));
    assert!(redeemed.status.success(), "{}", stderr(&redeemed));
    let text = stdout(&redeemed);
    assert!(text.contains("full access"), "{text}");
    assert!(
        text.contains("kmp_"),
        "the token it prints is what gets in:\n{text}"
    );
    let next = code_after(&text, "RECOVERY CODE");
    assert_ne!(next, code, "redemption must replace the code it spent");

    let spent = cli.run_with_stdin(&["recover"], &format!("{code}\n"));
    assert!(!spent.status.success(), "a spent code must not work twice");
    assert!(stdout(&spent).contains("not valid"), "{}", stdout(&spent));

    let with_new = cli.run_with_stdin(&["recover"], &format!("{next}\n"));
    assert!(with_new.status.success(), "{}", stderr(&with_new));
}

#[test]
fn a_lost_paper_record_can_be_replaced_from_the_console() {
    let cli = Cli::new();
    let code = code_after(&cli.init(), "RECOVERY CODE");
    let reissued = cli.run(&["recover", "--new"]);
    assert!(reissued.status.success(), "{}", stderr(&reissued));
    let fresh = code_after(&stdout(&reissued), "RECOVERY CODE");
    assert_ne!(fresh, code);

    let old = cli.run_with_stdin(&["recover"], &format!("{code}\n"));
    assert!(!old.status.success(), "reissuing must retire the old code");
}

/// A throwaway named session, stopped and deleted by the test that made it. `default` is never
/// touched, and a machine with no herdr skips rather than fails.
struct Herd {
    socket: std::path::PathBuf,
}

impl Herd {
    fn start(tag: &str) -> Option<Self> {
        which("herdr")?;
        let name = format!("kampr-doctor-{tag}-{}", std::process::id());
        assert_ne!(name, "default");
        let socket = herdr_home().join("sessions").join(&name).join("herdr.sock");
        Command::new("herdr")
            .args(["server", "--session", &name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..100 {
            if socket.exists() {
                std::thread::sleep(std::time::Duration::from_millis(300));
                return Some(Self { socket });
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        None
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

fn which(binary: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
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
    let Some(herd) = Herd::start("live") else {
        eprintln!("skipped: herdr is not on PATH");
        return;
    };
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

#[test]
fn doctor_reports_a_dead_herdr_as_broken() {
    let cli = Cli::new();
    cli.init();
    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    let herdr = check(&json, "herdr");
    assert_eq!(herdr["status"], "fail", "{herdr:#}");
    assert!(herdr["fix"].as_str().unwrap().contains("herdr"));
    assert!(!out.status.success(), "a node with no herd is not healthy");
}

#[test]
fn doctor_reports_a_healthy_node_as_healthy() {
    let cli = Cli::new();
    cli.init();
    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");

    assert_eq!(check(&json, "permissions")["status"], "ok", "{json:#}");
    assert_eq!(check(&json, "recovery")["status"], "ok");
    assert_eq!(check(&json, "bind")["status"], "ok");
    assert_eq!(check(&json, "tls")["status"], "ok");
    for id in [
        "config",
        "herdr",
        "sessions",
        "bind",
        "tls",
        "permissions",
        "bundle",
        "service",
        "tier",
        "devices",
        "recovery",
    ] {
        check(&json, id);
    }
}

#[test]
fn doctor_fails_when_a_credential_file_is_readable_by_anyone() {
    let cli = Cli::new();
    cli.init();
    let healthy: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    assert_eq!(check(&healthy, "permissions")["status"], "ok");

    // The audit found this exact shape: the main database locked down and the write-ahead log,
    // which carries the same digests, left open.
    loosen(&cli.state.join("kampr.db-wal"));
    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(check(&json, "permissions")["status"], "fail", "{json:#}");
    assert!(
        check(&json, "permissions")["detail"]
            .as_str()
            .unwrap()
            .contains("kampr.db-wal"),
        "{json:#}"
    );
    assert!(!out.status.success(), "a broken node must not exit zero");
}

#[test]
fn doctor_says_a_trusted_proxy_with_nothing_in_front_is_dangerous() {
    let cli = Cli::new();
    cli.init();
    set_config(&cli, "trust_proxy = false", "trust_proxy = true");
    set_config(&cli, "bind = \"127.0.0.1:8790\"", "bind = \"0.0.0.0:8790\"");
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    let tls = check(&json, "tls");
    assert_eq!(tls["status"], "fail", "{json:#}");
    assert!(
        tls["detail"].as_str().unwrap().contains("X-Forwarded-For"),
        "{tls:#}"
    );
}

#[test]
fn doctor_explains_why_an_ip_origin_can_never_do_passkeys() {
    let cli = Cli::new();
    cli.init();
    set_config(&cli, "bind = \"127.0.0.1:8790\"", "bind = \"0.0.0.0:8790\"");
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    let bind = check(&json, "bind");
    assert_eq!(bind["status"], "warn", "{json:#}");
    let tier = check(&json, "tier");
    let detail = tier["detail"].as_str().unwrap();
    assert!(detail.contains("passkeys"), "{tier:#}");
    assert!(
        detail.contains("registrable domain") || tier["fix"].as_str().unwrap_or("").contains("hostname"),
        "{tier:#}"
    );
}

#[test]
fn doctor_without_a_config_says_to_run_init() {
    let cli = Cli::new();
    let out = cli.run(&["doctor"]);
    let text = stdout(&out);
    assert!(text.contains("kampr init"), "{text}");
    assert!(!out.status.success());
}

#[test]
fn doctor_reports_no_recovery_code_on_a_node_that_predates_it() {
    let cli = Cli::new();
    cli.init();
    let json: serde_json::Value = serde_json::from_str(&devices(&cli)).expect("json");
    assert_eq!(check(&json, "recovery")["status"], "ok");

    let redeemed = cli.run_with_stdin(&["recover"], "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ\n");
    assert!(!redeemed.status.success());
    let json: serde_json::Value = serde_json::from_str(&devices(&cli)).expect("json");
    assert!(
        check(&json, "recovery")["detail"]
            .as_str()
            .unwrap()
            .contains("failed attempt"),
        "a wrong guess is a signal, not a silence: {json:#}"
    );
}

/// sqlite removes the write-ahead log on a clean close, so the state the audit found — a `-wal`
/// left behind readable by anyone — has to be recreated rather than waited for.
#[cfg(unix)]
fn loosen(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if !path.exists() {
        std::fs::write(path, b"").expect("create");
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
}

#[cfg(not(unix))]
fn loosen(_path: &Path) {}

fn set_config(cli: &Cli, from: &str, to: &str) {
    let path = cli.config.join("config.toml");
    let text = std::fs::read_to_string(&path).expect("config");
    assert!(text.contains(from), "{text}");
    std::fs::write(&path, text.replace(from, to)).expect("config");
}
