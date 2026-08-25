//! The two operator commands, driven as an operator drives them: the real binary, a throwaway
//! config and state directory, and nothing else on the machine touched.

use std::path::Path;
use std::process::{Command, Output, Stdio};

struct Cli {
    config: std::path::PathBuf,
    state: std::path::PathBuf,
    socket: std::path::PathBuf,
    xdg: std::path::PathBuf,
    nowhere: std::path::PathBuf,
    binary: std::path::PathBuf,
    env: Vec<(String, String)>,
    _dir: tempfile::TempDir,
}

impl Cli {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        Self {
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            socket: dir.path().join("herdr.sock"),
            xdg: dir.path().join("xdg"),
            nowhere: dir.path().join("nowhere"),
            binary: std::path::PathBuf::from(env!("CARGO_BIN_EXE_kampr")),
            env: Vec::new(),
            _dir: dir,
        }
    }

    /// A kampr somewhere other than the build directory, for the questions that are about where
    /// the binary sits.
    fn binary(mut self, path: &Path) -> Self {
        self.binary = path.to_path_buf();
        self
    }

    fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    fn against(mut self, socket: &Path) -> Self {
        self.socket = socket.to_path_buf();
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_stdin(args, "")
    }

    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        self.spawn(args, stdin, true)
    }

    /// Without `--state-dir`, which is how the documented next step after `kampr init` is
    /// actually typed: the config is supposed to remember where the state went.
    fn run_bare(&self, args: &[&str]) -> Output {
        self.spawn(args, "", false)
    }

    fn spawn(&self, args: &[&str], stdin: &str, state_dir: bool) -> Output {
        let mut command = Command::new(&self.binary);
        command.args(args).arg("--config-dir").arg(&self.config);
        if state_dir {
            command.arg("--state-dir").arg(&self.state);
        }
        // No test may reach the public internet, and `doctor`'s asset-links check asks Google.
        // Port 1 refuses instantly, which is the "could not ask" verdict; a test that wants one of
        // the other three points this at an `Upstream` of its own through `with_env`.
        command.env("KAMPR_ASSETLINKS_API", "http://127.0.0.1:1/v1/statements:list");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let mut child = command
            // Herdr sets this in every process it spawns, so a suite run from inside a pane would
            // otherwise resolve the herdr binary differently from one run in a plain terminal.
            .env_remove("HERDR_BIN_PATH")
            // The doctor reports on the environment the node would run in, so a test points it
            // at a socket that is not the developer's own herd.
            .env("HERDR_SOCKET_PATH", &self.socket)
            // Nothing a test runs may reach the developer's own systemd: the unit path is
            // redirected here, and both buses are pointed at a socket that does not exist, so a
            // stray `systemctl --user enable` or `loginctl enable-linger` fails instead of landing.
            .env("XDG_CONFIG_HOME", &self.xdg)
            .env("XDG_RUNTIME_DIR", self.nowhere.join("run"))
            .env("DBUS_SESSION_BUS_ADDRESS", self.bus())
            .env("DBUS_SYSTEM_BUS_ADDRESS", self.bus())
            .env("KAMPR_LINGER_DIR", self.nowhere.join("linger"))
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

    fn bus(&self) -> String {
        format!("unix:path={}", self.nowhere.join("bus").display())
    }

    fn unit(&self) -> std::path::PathBuf {
        self.xdg.join("systemd/user/kampr.service")
    }

    /// The binary half of herdr, pinned in config the way `kampr service install` pins it.
    fn pin_herdr(&self, binary: &Path) {
        let text = self.config_text();
        let pinned: String = text
            .lines()
            .map(|line| match line.starts_with("binary = ") {
                true => format!("binary = {:?}", binary.display().to_string()),
                false => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(pinned.contains(&binary.display().to_string()), "{pinned}");
        std::fs::write(self.config.join("config.toml"), pinned).expect("config.toml");
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.config.join("config.toml")).expect("config.toml")
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        serde_json::from_str(&stdout(&self.run(args))).expect("json")
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

/// #233: the node that served a correct herd, accepted input, reported green, and showed a
/// blank grid in every client, because the half of herdr that streams is a spawned binary and
/// nothing had ever run it. `--version` answering is not that half.
#[test]
fn doctor_fails_a_herdr_that_answers_version_and_cannot_observe() {
    let Some(herd) = Herd::start("blind").map(Herd::with_a_pane) else {
        eprintln!("skipped: herdr is not on PATH");
        return;
    };
    let cli = Cli::new().against(&herd.socket);
    cli.init();
    let shim = cli.nowhere.join("herdr");
    std::fs::create_dir_all(&cli.nowhere).expect("a directory for the shim");
    std::fs::write(
        &shim,
        "#!/bin/sh\ncase \"$1\" in --version) echo 'herdr 0.8.2' ;; *) exit 1 ;; esac\n",
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
        detail.contains("herdr 0.8.2"),
        "the version it did answer: {detail}"
    );
}

/// And the other side of it: against a real herdr the check is only green because a frame came
/// back off a real `terminal session observe`.
#[test]
fn doctor_proves_the_stream_against_a_live_herdr_rather_than_asking_its_version() {
    let Some(herd) = Herd::start("stream").map(Herd::with_a_pane) else {
        eprintln!("skipped: herdr is not on PATH");
        return;
    };
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
        "linger",
        "origin",
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

fn field(config: &str, key: &str) -> String {
    config
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key} = ")))
        .unwrap_or_else(|| panic!("no {key} in:\n{config}"))
        .trim_matches('"')
        .to_string()
}

/// The hub configuration `docs/07-mesh-deployment.md` §1 tells the operator to hand-edit in.
/// None of it has a CLI flag, so `--force` is the only command that can throw it away.
fn make_it_a_proxied_hub(cli: &Cli) {
    set_config(cli, "origin = \"\"", "origin = \"https://kampr.example.com\"");
    set_config(cli, "trust_proxy = false", "trust_proxy = true");
    set_config(
        cli,
        "extra_origins = []",
        "extra_origins = [\"https://kampr.lan\"]",
    );
    set_config(cli, "accept = false", "accept = true");
}

#[test]
fn force_keeps_the_identity_and_everything_a_proxy_depends_on() {
    let cli = Cli::new();
    cli.init();
    let node_id = field(&cli.config_text(), "node_id");
    make_it_a_proxied_hub(&cli);

    // Exactly what `kampr doctor` prints as the fix for a bind it does not like.
    let out = cli.run(&["init", "--bind", "127.0.0.1:8795", "--force"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let after = cli.config_text();
    assert_eq!(
        field(&after, "node_id"),
        node_id,
        "a new identity unenrols every passkey and every mesh peer:\n{after}"
    );
    assert!(
        after.contains("origin = \"https://kampr.example.com\""),
        "{after}"
    );
    assert!(after.contains("trust_proxy = true"), "{after}");
    assert!(after.contains("https://kampr.lan"), "{after}");
    assert!(after.contains("accept = true"), "{after}");
    assert!(after.contains("bind = \"127.0.0.1:8795\""), "{after}");
}

#[test]
fn force_says_what_it_keeps_and_what_it_resets_before_it_writes() {
    let cli = Cli::new();
    cli.init();
    let node_id = field(&cli.config_text(), "node_id");
    make_it_a_proxied_hub(&cli);
    let text = stdout(&cli.run(&["init", "--force"]));
    assert!(text.contains(&node_id), "{text}");
    assert!(text.contains("keeps"), "{text}");
    assert!(text.contains("resets"), "{text}");
}

#[test]
fn a_new_identity_is_a_separate_opt_in_that_names_what_it_costs() {
    let cli = Cli::new();
    cli.init();
    let node_id = field(&cli.config_text(), "node_id");
    make_it_a_proxied_hub(&cli);

    let out = cli.run(&["init", "--force", "--new-identity"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert_ne!(field(&cli.config_text(), "node_id"), node_id);
    assert!(
        text.contains(&node_id),
        "it has to name the identity it is throwing away:\n{text}"
    );
    assert!(text.contains("passkey") && text.contains("mesh"), "{text}");
    assert!(
        cli.config_text().contains("trust_proxy = true"),
        "a new identity is not a reason to drop the proxy configuration too"
    );
}

#[test]
fn changing_the_bind_needs_no_force_and_says_it_changed() {
    let cli = Cli::new();
    cli.init();
    make_it_a_proxied_hub(&cli);
    let out = cli.run(&["init", "--bind", "127.0.0.1:8795"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(cli.config_text().contains("bind = \"127.0.0.1:8795\""));
    let text = stdout(&out);
    assert!(
        !text.contains("re-run with --force"),
        "it did the thing that was asked; --force is not the way to do it:\n{text}"
    );
}

fn systemd_host() -> bool {
    cfg!(target_os = "linux") && Path::new("/run/systemd/system").is_dir()
}

#[test]
fn service_install_points_the_unit_at_the_state_directory_init_recorded() {
    if !systemd_host() {
        eprintln!("skipped: no systemd on this host");
        return;
    }
    let cli = Cli::new();
    cli.init();
    let out = cli.run_bare(&["service", "install"]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    let unit = std::fs::read_to_string(cli.unit()).expect("unit file");
    assert!(
        unit.contains(&format!("--state-dir {}", cli.state.display())),
        "the service must come up on the database init made, not on a fresh one:\n{unit}"
    );
}

#[test]
fn service_install_refuses_before_init_rather_than_arming_a_restart_loop() {
    if !systemd_host() {
        eprintln!("skipped: no systemd on this host");
        return;
    }
    let cli = Cli::new();
    let out = cli.run_bare(&["service", "install"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(stderr(&out).contains("kampr init"), "{}", stderr(&out));
    assert!(
        !cli.unit().exists(),
        "`Restart=on-failure` against a config that does not exist is a 5-second loop forever"
    );
}

#[test]
fn service_install_says_what_it_takes_to_survive_a_reboot() {
    if !systemd_host() {
        eprintln!("skipped: no systemd on this host");
        return;
    }
    let cli = Cli::new();
    cli.init();
    let out = cli.run_bare(&["service", "install"]);
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.contains("loginctl enable-linger"),
        "without linger the user manager is torn down at logout and never starts at boot:\n{text}"
    );
}

fn lingers() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    !user.is_empty() && Path::new("/var/lib/systemd/linger").join(user).exists()
}

#[test]
fn doctor_is_quiet_about_linger_when_no_unit_is_installed() {
    let cli = Cli::new();
    cli.init();
    let json = cli.json(&["doctor", "--json"]);
    assert_eq!(check(&json, "linger")["status"], "ok", "{json:#}");
}

#[test]
fn doctor_fails_when_a_unit_is_installed_and_the_user_does_not_linger() {
    if !systemd_host() || lingers() {
        eprintln!("skipped: needs a systemd host whose test user does not linger");
        return;
    }
    let cli = Cli::new();
    cli.init();
    std::fs::create_dir_all(cli.unit().parent().unwrap()).unwrap();
    std::fs::write(cli.unit(), "[Unit]\n").unwrap();

    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    let linger = check(&json, "linger");
    assert_eq!(linger["status"], "fail", "{json:#}");
    assert!(
        linger["fix"].as_str().unwrap().contains("loginctl enable-linger"),
        "{linger:#}"
    );
    assert!(
        !out.status.success(),
        "a node that dies at the next reboot is not healthy"
    );
}

#[test]
fn a_proxied_loopback_bind_is_never_told_to_open_itself_to_the_network() {
    let cli = Cli::new();
    let out = cli.run(&[
        "init",
        "--bind",
        "127.0.0.1:8795",
        "--origin",
        "https://kampr.example.com",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        !text.contains("--bind 0.0.0.0"),
        "the deployment doc spends a paragraph on why this bind must stay loopback:\n{text}"
    );
    assert!(text.contains("proxy"), "{text}");

    let status = stdout(&cli.run(&["status"]));
    assert!(!status.contains("--bind 0.0.0.0"), "{status}");
}

#[test]
fn an_unproxied_loopback_bind_still_says_how_to_reach_it_from_a_phone() {
    let cli = Cli::new();
    let text = cli.init();
    assert!(text.contains("--bind 0.0.0.0"), "{text}");
}

/// A downgrade: a database carrying a migration the running binary does not know. sqlx refuses it
/// by design, and the raw refusal names a number and nothing else.
fn write_a_migration_from_the_future(db: &Path) {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
            .await
            .expect("open");
        sqlx::query(
            "INSERT INTO _sqlx_migrations
             (version, description, installed_on, success, checksum, execution_time)
             VALUES (?, ?, CURRENT_TIMESTAMP, 1, ?, 0)",
        )
        .bind(9_999_999_i64)
        .bind("written by a newer kampr")
        .bind(vec![0u8; 32])
        .execute(&pool)
        .await
        .expect("insert");
        pool.close().await;
    });
}

#[test]
fn a_database_from_a_newer_kampr_is_explained_rather_than_printed_as_sqlx() {
    let cli = Cli::new();
    cli.init();
    write_a_migration_from_the_future(&cli.state.join("kampr.db"));

    for args in [
        vec!["status"],
        vec!["setup"],
        vec!["pair"],
        vec!["init", "--force"],
    ] {
        let out = cli.run(&args);
        let text = format!("{}{}", stdout(&out), stderr(&out));
        assert!(!out.status.success(), "{args:?} should fail:\n{text}");
        assert!(
            text.contains("newer kampr"),
            "{args:?} printed a raw sqlx string:\n{text}"
        );
        assert!(
            !text.contains("resolved migrations"),
            "{args:?} printed a raw sqlx string:\n{text}"
        );
    }
}

#[test]
fn doctor_does_not_prescribe_a_remedy_that_hits_the_same_wall() {
    let cli = Cli::new();
    cli.init();
    write_a_migration_from_the_future(&cli.state.join("kampr.db"));

    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    let devices = check(&json, "devices");
    assert_eq!(devices["status"], "fail", "{json:#}");
    let fix = devices["fix"].as_str().unwrap_or_default();
    assert!(
        !fix.contains("--force"),
        "`kampr init --force` opens the same database and dies with the same error: {fix}"
    );
    assert!(
        fix.contains("kampr.db"),
        "moving the database aside is the only thing that works: {fix}"
    );
    assert!(!out.status.success());
}

/// One socket, one canned HTTP response, on a port the test owns — a stand-in for a proxy host
/// that answers but does not lead here.
struct Upstream {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Upstream {
    fn serving(body: &'static str) -> Self {
        Self::routing(Box::leak(Box::new([("", body)])))
    }

    /// Path prefix to body, first match wins; `""` is the catch-all. Anything unmatched is a 404,
    /// which is what a proxy that answers `/.well-known` itself does.
    fn routing(routes: &'static [(&'static str, &'static str)]) -> Self {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let Ok(mut stream) = stream else { return };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                let mut scratch = [0u8; 2048];
                let read = stream.read(&mut scratch).unwrap_or(0);
                let request = String::from_utf8_lossy(&scratch[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let found = routes
                    .iter()
                    .find(|(prefix, _)| prefix.is_empty() || path.starts_with(prefix))
                    .map(|(_, body)| *body);
                let (status, body) = match found {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", "not found"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self { port, stop }
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().unwrap().port()
}

/// A bare TCP connect to the origin cannot tell Kampr from NPM's own "Congratulations" page, and
/// every reverse-proxy misconfiguration in the class this deployment hits reads as healthy.
#[test]
fn doctor_says_when_the_hostname_answers_but_does_not_lead_to_this_node() {
    let upstream = Upstream::serving(r#"{"node_id":"01SOMEONEELSE","node_name":"other"}"#);
    let cli = Cli::new();
    let out = cli.run(&["init", "--origin", &upstream.origin()]);
    assert!(out.status.success(), "{}", stderr(&out));

    let out = cli.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    let origin = check(&json, "origin");
    assert_eq!(origin["status"], "fail", "{json:#}");
    assert!(
        origin["detail"].as_str().unwrap().contains("01SOMEONEELSE"),
        "{origin:#}"
    );
    assert!(!out.status.success());
}

#[test]
fn doctor_says_when_the_hostname_answers_with_something_that_is_not_kampr_at_all() {
    let upstream = Upstream::serving("<html><body>Congratulations!</body></html>");
    let cli = Cli::new();
    cli.run(&["init", "--origin", &upstream.origin()]);
    let json = cli.json(&["doctor", "--json"]);
    let origin = check(&json, "origin");
    assert_eq!(origin["status"], "fail", "{json:#}");
    assert!(
        origin["fix"].as_str().unwrap().contains("Forward"),
        "the fix has to name the NPM field that is wrong: {origin:#}"
    );
}

#[test]
fn doctor_confirms_the_whole_path_when_the_hostname_does_lead_here() {
    let cli = Cli::new();
    cli.init();
    let node_id = field(&cli.config_text(), "node_id");
    let body: &'static str = Box::leak(format!(r#"{{"node_id":"{node_id}"}}"#).into_boxed_str());
    let upstream = Upstream::serving(body);
    cli.run(&["init", "--origin", &upstream.origin()]);

    let json = cli.json(&["doctor", "--json"]);
    let origin = check(&json, "origin");
    assert_eq!(origin["status"], "ok", "{json:#}");
}

/// A node that is simply not running is not a broken proxy, and saying so would drown the case
/// that is.
#[test]
fn doctor_does_not_call_a_stopped_node_a_broken_proxy() {
    let cli = Cli::new();
    cli.run(&["init", "--origin", &format!("http://127.0.0.1:{}", free_port())]);
    let json = cli.json(&["doctor", "--json"]);
    let origin = check(&json, "origin");
    assert_eq!(origin["status"], "warn", "{json:#}");
    assert!(
        origin["detail"].as_str().unwrap().contains("nothing answers"),
        "{origin:#}"
    );
}

/// Eleven checks and nothing said whether Android would let the app hold a passkey here — and
/// then the check that did asked the *node's own origin*, which is not the party that decides.
/// Credential Manager asks Google's validator, which fetches the file server-side from the public
/// internet; a hostname that resolves publicly to an RFC1918 address is one it cannot reach
/// however perfectly the node serves it, and the report stayed green through the whole of #170.
#[test]
fn doctor_fails_when_google_cannot_fetch_the_file_that_decides() {
    let google = statements_service(r#"{"errorCode":["ERROR_CODE_FETCH_ERROR"],"maxAge":"600s"}"#);
    let cli = Cli::new().with_env("KAMPR_ASSETLINKS_API", &statements_url(&google));
    cli.init();
    // #170 exactly: the origin serves the right document to anything on this network, and the
    // check that read it from here was green while every ceremony on the phone was refused.
    let upstream = Upstream::routing(Box::leak(Box::new([
        ("/.well-known/assetlinks.json", node_document()),
        ("", node_identity(&cli)),
    ])));
    cli.run(&["init", "--origin", &upstream.origin()]);
    force_a_registrable_origin(&cli, upstream.port);

    let json = cli.json(&["doctor", "--json"]);
    let check = check(&json, "assetlinks");
    assert_eq!(check["status"], "fail", "{json:#}");
    let detail = check["detail"].as_str().unwrap();
    assert!(detail.contains("ERROR_CODE_FETCH_ERROR"), "{check:#}");
    assert!(
        detail.contains("Google cannot read") && detail.contains("Credential Manager"),
        "it has to blame Google's fetch and say the phone gets refused: {check:#}",
    );
}

#[test]
fn doctor_confirms_the_asset_links_file_when_google_reads_the_certificate_this_node_names() {
    let google = statements_service(delegating(kampr_node::assetlinks::RELEASE_FINGERPRINT));
    let cli = Cli::new().with_env("KAMPR_ASSETLINKS_API", &statements_url(&google));
    cli.init();
    let upstream = Upstream::serving(node_identity(&cli));
    cli.run(&["init", "--origin", &upstream.origin()]);
    force_a_registrable_origin(&cli, upstream.port);

    let json = cli.json(&["doctor", "--json"]);
    let check = check(&json, "assetlinks");
    assert_eq!(check["status"], "ok", "{json:#}");
    let detail = check["detail"].as_str().unwrap();
    assert!(detail.contains("dev.kampr.app"), "{check:#}");
    assert!(
        detail.contains("https://localhost/.well-known/assetlinks.json"),
        "a green has to name the host that was actually verified: {check:#}",
    );
}

/// The file that decides is about to live on a different host from the node, so two copies exist
/// and only one of them is read. Nothing else in this report would ever see them diverge.
#[test]
fn doctor_names_both_certificates_when_the_copy_google_reads_has_drifted() {
    let theirs: &'static str = Box::leak("AB".repeat(32).into_boxed_str());
    let google = statements_service(delegating(theirs));
    let cli = Cli::new().with_env("KAMPR_ASSETLINKS_API", &statements_url(&google));
    cli.init();
    let upstream = Upstream::serving(node_identity(&cli));
    cli.run(&["init", "--origin", &upstream.origin()]);
    force_a_registrable_origin(&cli, upstream.port);

    let json = cli.json(&["doctor", "--json"]);
    let check = check(&json, "assetlinks");
    assert_eq!(check["status"], "fail", "{json:#}");
    let detail = check["detail"].as_str().unwrap();
    assert!(detail.contains("AB:AB:AB"), "the copy Google reads: {check:#}");
    assert!(
        detail.contains(kampr_node::assetlinks::RELEASE_FINGERPRINT),
        "and the one this node names: {check:#}",
    );
}

/// A doctor run on a machine with no route out is not a diagnosis of the node. The most expensive
/// bug this project has had was a check that turned an unasked question into a healthy answer, and
/// the inverse — an unasked question into a failure — would send the operator after a fault that
/// is not there.
#[test]
fn doctor_warns_rather_than_fails_when_this_machine_cannot_ask_google() {
    let cli = Cli::new().with_env(
        "KAMPR_ASSETLINKS_API",
        &format!("http://127.0.0.1:{}/v1/statements:list", free_port()),
    );
    cli.init();
    let upstream = Upstream::serving(node_identity(&cli));
    cli.run(&["init", "--origin", &upstream.origin()]);
    force_a_registrable_origin(&cli, upstream.port);

    let json = cli.json(&["doctor", "--json"]);
    let check = check(&json, "assetlinks");
    assert_eq!(check["status"], "warn", "{json:#}");
    assert!(
        check["detail"].as_str().unwrap().contains("unestablished"),
        "it has to say what could not be established: {check:#}",
    );
    assert!(
        check["fix"]
            .as_str()
            .unwrap()
            .contains("digitalassetlinks.googleapis.com"),
        "and point at the machine's own reach, not at the node: {check:#}",
    );
}

/// The shape `digitalassetlinks.googleapis.com` answers in, from a port a test controls.
fn statements_service(body: &'static str) -> Upstream {
    Upstream::routing(Box::leak(Box::new([("", body)])))
}

fn statements_url(google: &Upstream) -> String {
    format!("{}/v1/statements:list", google.origin())
}

fn delegating(fingerprint: &str) -> &'static str {
    Box::leak(
        format!(
            r#"{{"statements":[{{"relation":"delegate_permission/common.get_login_creds","target":{{"androidApp":{{"packageName":"dev.kampr.app","certificate":{{"sha256Fingerprint":"{fingerprint}"}}}}}}}}],"maxAge":"599s"}}"#
        )
        .into_boxed_str(),
    )
}

/// The document the node itself builds and serves, which is correct and which nothing decides by.
fn node_document() -> &'static str {
    Box::leak(
        format!(
            r#"[{{"relation":["delegate_permission/common.get_login_creds"],"target":{{"namespace":"android_app","package_name":"dev.kampr.app","sha256_cert_fingerprints":["{}"]}}}}]"#,
            kampr_node::assetlinks::RELEASE_FINGERPRINT
        )
        .into_boxed_str(),
    )
}

fn node_identity(cli: &Cli) -> &'static str {
    let node_id = field(&cli.config_text(), "node_id");
    Box::leak(format!(r#"{{"node_id":"{node_id}"}}"#).into_boxed_str())
}

/// A loopback *IP* is a secure context and still not a registrable domain, so the tier is 0 and
/// the check is deliberately quiet. `localhost` is the one origin that is both.
fn force_a_registrable_origin(cli: &Cli, port: u16) {
    let text = cli.config_text().replace(
        &format!("http://127.0.0.1:{port}"),
        &format!("http://localhost:{port}"),
    );
    std::fs::write(cli.config.join("config.toml"), text).expect("config");
}

#[test]
fn doctor_is_quiet_about_asset_links_on_a_node_that_cannot_do_passkeys_at_all() {
    let cli = Cli::new();
    cli.run(&["init", "--origin", "http://192.168.1.24:8790"]);
    let json = cli.json(&["doctor", "--json"]);
    let check = check(&json, "assetlinks");
    assert_eq!(check["status"], "ok", "{json:#}");
    assert!(
        check["fix"].is_null(),
        "there is nothing to fix at tier 0: {check:#}"
    );
}

/// A herdr installed where a service manager cannot see it. Both binaries land in the same
/// prefix, which is the whole of what makes the fallback work.
struct Prefix {
    dir: tempfile::TempDir,
}

impl Prefix {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a prefix");
        std::fs::create_dir_all(dir.path().join("bin")).expect("a bin dir");
        Self { dir }
    }

    fn bin(&self) -> std::path::PathBuf {
        self.dir.path().join("bin")
    }

    /// Answers `--version` the way herdr does and nothing else, which is all any of this asks of
    /// it: `session list --json` returning junk is already a supported state.
    fn herdr(&self) -> std::path::PathBuf {
        let path = self.bin().join("herdr");
        std::fs::write(&path, "#!/bin/sh\necho 'herdr 0.8.2'\n").expect("a herdr");
        chmod_x(&path);
        path
    }

    fn kampr(&self) -> std::path::PathBuf {
        let path = self.bin().join("kampr");
        std::fs::copy(env!("CARGO_BIN_EXE_kampr"), &path).expect("a kampr");
        chmod_x(&path);
        path
    }

    fn home(&self) -> String {
        self.dir.path().display().to_string()
    }
}

fn chmod_x(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A herdr installed for everyone would be found by the prefix search and make these tests say
/// nothing, so they skip rather than pass for the wrong reason.
fn system_herdr() -> bool {
    ["/usr/local/bin/herdr", "/opt/homebrew/bin/herdr"]
        .iter()
        .any(|p| Path::new(p).exists())
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
    let Some(herd) = Herd::start("observe") else {
        eprintln!("skipped: herdr is not on PATH");
        return;
    };
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

/// The operator whose node broke months ago and who does nothing but update: their `config.toml`
/// still says `binary = \"herdr\"` and their service's PATH still has no `~/.local/bin` in it.
#[test]
fn a_herdr_beside_the_kampr_binary_is_found_with_nothing_on_the_path_at_all() {
    if system_herdr() {
        eprintln!("skipped: this host has a herdr installed system-wide");
        return;
    }
    let prefix = Prefix::new();
    let herdr = prefix.herdr();
    let kampr = prefix.kampr();
    let cli = Cli::new()
        .binary(&kampr)
        .with_env("PATH", "")
        .with_env("HOME", &prefix.home());

    let init = cli.init();
    assert!(init.contains(&herdr.display().to_string()), "{init}");
    assert!(
        cli.config_text().contains(&herdr.display().to_string()),
        "init records what it resolved:\n{}",
        cli.config_text()
    );

    // Back to what a config written before any of this says, which is the state on the machines
    // that are broken today.
    set_config(
        &cli,
        &format!("binary = \"{}\"", herdr.display()),
        "binary = \"herdr\"",
    );
    let json = cli.json(&["doctor", "--json"]);
    let observe = check(&json, "observe");
    // Not green, and this is the point of the check: resolution is half of it, and there is no
    // herd here to run the other half against.
    assert_eq!(observe["status"], "warn", "{json:#}");
    let detail = observe["detail"].as_str().unwrap();
    assert!(detail.contains(&herdr.display().to_string()), "{detail}");
    assert!(detail.contains("beside the kampr binary"), "{detail}");
    assert!(detail.contains("herdr 0.8.2"), "it ran the binary: {detail}");
    assert!(detail.contains("not established"), "{detail}");
}

/// `HERDR_SOCKET_PATH` is pinned into the unit because the environment that installs a service is
/// not the environment that runs it. The binary is the other half of that fact.
#[test]
fn service_install_records_the_herdr_it_resolved_rather_than_leaving_a_bare_name() {
    let prefix = Prefix::new();
    let herdr = prefix.herdr();
    let cli = Cli::new().with_env("PATH", "").with_env("HOME", &prefix.home());
    cli.init();
    assert_eq!(
        field(&cli.config_text(), "binary"),
        "herdr",
        "nothing to resolve, so nothing is written down"
    );

    let found = Cli {
        env: vec![
            ("PATH".into(), prefix.bin().display().to_string()),
            ("HOME".into(), prefix.home()),
        ],
        ..cli
    };
    let out = found.run(&["service", "install"]);
    let said = format!("{}{}", stdout(&out), stderr(&out));
    assert!(said.contains(&herdr.display().to_string()), "{said}");
    assert_eq!(
        field(&found.config_text(), "binary"),
        herdr.display().to_string(),
        "the unit's manager has its own PATH, so the path is recorded where every entry point \
         reads it"
    );
}
