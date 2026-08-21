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
        self.spawn(args, stdin, true)
    }

    /// Without `--state-dir`, which is how the documented next step after `kampr init` is
    /// actually typed: the config is supposed to remember where the state went.
    fn run_bare(&self, args: &[&str]) -> Output {
        self.spawn(args, "", false)
    }

    fn spawn(&self, args: &[&str], stdin: &str, state_dir: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kampr"));
        command.args(args).arg("--config-dir").arg(&self.config);
        if state_dir {
            command.arg("--state-dir").arg(&self.state);
        }
        let mut child = command
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
                let _ = stream.read(&mut scratch);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
