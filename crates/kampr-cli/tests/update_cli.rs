//! `kampr update`, driven the way an operator drives it: the real binary, replacing itself, out
//! of a release built on disk.
//!
//! The release is served by a `curl` on `PATH` that answers only for the canonical
//! `https://github.com/dbrain/kampr/releases/…` and 404s everything else — so these tests fetch
//! nothing, and the URL the command actually asks for is asserted rather than assumed. The
//! installer's own `KAMPR_BASE_URL` is deliberately *not* the seam here: `kampr update` clears it,
//! because a base decides the tarball and the checksums it is checked against together.
//!
//! Nothing here touches the network, the operator's `~/.local/bin`, or any systemd unit — `HOME`
//! and `XDG_CONFIG_HOME` are redirected into the test's own directory, so the unit the installer
//! looks for cannot exist.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A release on disk: the tarball the installer fetches and the checksums it verifies against.
struct Release {
    dir: PathBuf,
    asset: String,
    /// A directory holding the `curl` that serves this release, to go on the front of `PATH`.
    shim: PathBuf,
}

impl Release {
    /// `body` is the whole of the "binary" — a shell script, so a test can decide whether the
    /// thing that lands runs at all.
    fn built(root: &Path, body: &str) -> Self {
        let dir = root.join("release");
        let stage = root.join("stage");
        std::fs::create_dir_all(&dir).expect("a release dir");
        std::fs::create_dir_all(&stage).expect("a stage dir");
        let binary = stage.join("kampr");
        std::fs::write(&binary, body).expect("a fake binary");
        chmod_x(&binary);

        // The names the release workflow publishes, which are what install.sh builds a URL from.
        let arch = std::env::consts::ARCH;
        let os = match std::env::consts::OS {
            "macos" => "apple-darwin",
            _ => "unknown-linux-musl",
        };
        let asset = format!("kampr-{arch}-{os}.tar.gz");
        let tar = Command::new("tar")
            .args(["-czf"])
            .arg(dir.join(&asset))
            .arg("-C")
            .arg(&stage)
            .arg("kampr")
            .status()
            .expect("tar");
        assert!(tar.success(), "could not build {asset}");

        let release = Self {
            dir,
            asset,
            shim: root.join("shim"),
        };
        release.write_sums(&release.sha256());
        release.write_bundle();
        release.write_shim();
        release
    }

    /// Every release this project publishes is signed, and the installer refuses one from the
    /// canonical base that is not. The bundle is never verified here — cosign is not on a test
    /// runner — but its absence is fatal, so it has to be served.
    fn write_bundle(&self) {
        std::fs::write(self.dir.join("SHA256SUMS.cosign.bundle"), "{}\n").expect("a bundle");
    }

    fn drop_bundle(&self) {
        std::fs::remove_file(self.dir.join("SHA256SUMS.cosign.bundle")).expect("a bundle");
    }

    /// Stands in for `curl`, and refuses anything that is not the canonical release base — so a
    /// command that could be talked into fetching from anywhere else fails here rather than
    /// quietly succeeding against whatever the test set up.
    /// The shim in front of whatever this test runner already has, so `sh`, `tar` and
    /// `sha256sum` still resolve.
    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.shim.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn write_shim(&self) {
        std::fs::create_dir_all(&self.shim).expect("a shim dir");
        let curl = self.shim.join("curl");
        std::fs::write(
            &curl,
            format!(
                r#"#!/bin/sh
url=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
case "$url" in
  https://github.com/dbrain/kampr/releases/*) ;;
  *) echo "curl: refusing $url" >&2; exit 22 ;;
esac
name="${{url##*/}}"
[ -f "{dir}/$name" ] || exit 22
cp "{dir}/$name" "$out"
"#,
                dir = self.dir.display()
            ),
        )
        .expect("a curl shim");
        chmod_x(&curl);
    }

    fn sha256(&self) -> String {
        let out = Command::new("sha256sum")
            .arg(self.dir.join(&self.asset))
            .output()
            .expect("sha256sum");
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .expect("a digest")
            .to_string()
    }

    fn write_sums(&self, digest: &str) {
        std::fs::write(self.dir.join("SHA256SUMS"), format!("{digest}  {}\n", self.asset))
            .expect("SHA256SUMS");
    }

    fn drop_sums(&self) {
        std::fs::remove_file(self.dir.join("SHA256SUMS")).expect("SHA256SUMS");
    }
}

fn chmod_x(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

/// A kampr installed somewhere of its own, so `kampr update` replaces *that* copy and never the
/// one cargo built.
struct Installed {
    home: tempfile::TempDir,
    binary: PathBuf,
}

impl Installed {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("a home");
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).expect("a bin dir");
        let binary = bin.join("kampr");
        std::fs::copy(env!("CARGO_BIN_EXE_kampr"), &binary).expect("a kampr to update");
        chmod_x(&binary);
        Self { home, binary }
    }

    fn run(&self, args: &[&str], release: Option<&Release>) -> Output {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .arg("--config-dir")
            .arg(self.home.path().join("config"))
            .arg("--state-dir")
            .arg(self.home.path().join("state"))
            // Redirected so the installer's systemd and launchd lookups land in this directory
            // rather than on the developer's own unit.
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg"))
            .env_remove("KAMPR_ALLOW_UNVERIFIED")
            .env_remove("KAMPR_BASE_URL");
        if let Some(release) = release {
            command.env("PATH", release.path());
        }
        output(&mut command)
    }

    fn version(&self) -> String {
        let out = output(Command::new(&self.binary).arg("--version"));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn digest(&self) -> String {
        let out = Command::new("sha256sum")
            .arg(&self.binary)
            .output()
            .expect("sha256sum");
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .expect("a digest")
            .to_string()
    }

    fn state(&self) -> PathBuf {
        self.home.path().join("state")
    }
}

/// Retries `ETXTBSY`. Cargo runs these tests in threads of one process: while one of them is
/// copying a fresh kampr into place, another one forking inherits that write handle, and the
/// exec that follows is refused. It is a property of this harness, not of the command.
fn output(command: &mut Command) -> Output {
    for _ in 0..100 {
        match command.output() {
            Ok(out) => return out,
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(e) => panic!("running kampr: {e}"),
        }
    }
    panic!("kampr stayed busy for five seconds")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const WORKS: &str = "#!/bin/sh\necho 'kampr 99.9.9'\n";
const BROKEN: &str = "#!/bin/sh\nexit 1\n";
const TAMPERED: &str = "#!/bin/sh\necho 'kampr 0.0.0-tampered'\n";

#[test]
fn update_replaces_the_binary_it_is_running_from() {
    let installed = Installed::new();
    let release = Release::built(installed.home.path(), WORKS);
    assert_ne!(installed.version(), "kampr 99.9.9");

    let out = installed.run(&["update"], Some(&release));
    let said = text(&out);
    assert!(out.status.success(), "kampr update failed:\n{said}");
    assert_eq!(
        installed.version(),
        "kampr 99.9.9",
        "the binary that ran the update is not the binary that got replaced:\n{said}"
    );
    assert!(
        said.contains("checksum verified: yes"),
        "the update installed without saying it had verified anything:\n{said}"
    );
    // The first-run ladder is for a first run. An update that reprinted it would be telling an
    // operator with a paired node to go and pair a node.
    assert!(
        !said.contains("kampr init"),
        "an update printed the first-run epilogue:\n{said}"
    );
}

/// The one that matters: a tampered release must leave the host exactly as it found it. A
/// half-replaced kampr can type into every terminal on the machine.
#[test]
fn a_checksum_that_does_not_match_installs_nothing_at_all() {
    let installed = Installed::new();
    let release = Release::built(installed.home.path(), WORKS);
    release.write_sums(&"0".repeat(64));
    let before = installed.digest();

    let out = installed.run(&["update"], Some(&release));
    let said = text(&out);
    assert!(
        !out.status.success(),
        "a bad checksum was installed anyway:\n{said}"
    );
    assert!(
        said.contains("checksum mismatch"),
        "the refusal did not say what was wrong:\n{said}"
    );
    assert_eq!(
        installed.digest(),
        before,
        "the binary was replaced despite the checksum refusal:\n{said}"
    );
    assert_ne!(installed.version(), "kampr 99.9.9");
}

/// A release with no checksums is not a release to install from, and the escape hatch that exists
/// for a hand-built binary must not be reachable through this command.
#[test]
fn a_release_with_no_checksums_is_refused_and_the_bypass_is_not_inherited() {
    let installed = Installed::new();
    let release = Release::built(installed.home.path(), WORKS);
    release.drop_sums();
    let before = installed.digest();

    let out = installed.run(&["update"], Some(&release));
    let said = text(&out);
    assert!(
        !out.status.success(),
        "an unverified binary was installed:\n{said}"
    );
    assert!(
        said.contains("SHA256SUMS"),
        "the refusal did not say what was missing:\n{said}"
    );
    assert_eq!(installed.digest(), before);

    // Even with the bypass set in the environment that ran the command.
    let mut command = Command::new(&installed.binary);
    command
        .arg("update")
        .env("HOME", installed.home.path())
        .env("XDG_CONFIG_HOME", installed.home.path().join("xdg"))
        .env("PATH", release.path())
        .env("KAMPR_ALLOW_UNVERIFIED", "1");
    let out = output(&mut command);
    let said = text(&out);
    assert!(
        !out.status.success(),
        "KAMPR_ALLOW_UNVERIFIED in the caller's environment turned off verification for a command \
         that replaces the binary with access to every terminal on this host:\n{said}"
    );
    assert_eq!(installed.digest(), before);
}

/// Verification says the bytes are the ones that were published; it says nothing about whether
/// they run here. A release that does not run must leave a working kampr behind.
#[test]
fn a_new_binary_that_does_not_run_is_put_back() {
    let installed = Installed::new();
    let release = Release::built(installed.home.path(), BROKEN);
    let before = installed.version();

    let out = installed.run(&["update"], Some(&release));
    let said = text(&out);
    assert!(
        !out.status.success(),
        "a binary that does not run was accepted:\n{said}"
    );
    assert!(
        said.contains("back in place"),
        "the rollback happened silently, or not at all:\n{said}"
    );
    assert_eq!(
        installed.version(),
        before,
        "the host was left without a working kampr:\n{said}"
    );
}

/// `--check` answers from the same file the node's own once-a-day check writes, so asking at a
/// shell costs nothing and does not reset the node's cadence.
#[test]
fn check_reads_the_answer_the_node_already_cached() {
    let installed = Installed::new();
    std::fs::create_dir_all(installed.state()).expect("a state dir");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    std::fs::write(
        installed.state().join("update.json"),
        format!(r#"{{"latest":"v99.9.9","checked_at":{now},"ok":true}}"#),
    )
    .expect("a cache");

    let out = installed.run(&["update", "--check"], None);
    let said = text(&out);
    assert!(out.status.success(), "--check failed:\n{said}");
    assert!(
        said.contains("99.9.9"),
        "--check did not name the release the node had already found:\n{said}"
    );
    assert!(
        said.contains("kampr update"),
        "--check said an update exists without saying how to take it:\n{said}"
    );
    assert_eq!(
        installed.digest(),
        installed.digest(),
        "--check must not install anything"
    );
    assert_ne!(installed.version(), "kampr 99.9.9", "--check installed something");
}

/// The quiet case at the shell, and the one that must not read as an error.
#[test]
fn check_on_a_current_node_says_so_and_succeeds() {
    let installed = Installed::new();
    std::fs::create_dir_all(installed.state()).expect("a state dir");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    let build = installed.version().replace("kampr ", "");
    std::fs::write(
        installed.state().join("update.json"),
        format!(r#"{{"latest":"v{build}","checked_at":{now},"ok":true}}"#),
    )
    .expect("a cache");

    let out = installed.run(&["update", "--check"], None);
    let said = text(&out);
    assert!(out.status.success(), "--check on a current node failed:\n{said}");
    assert!(
        said.contains("up to date"),
        "a current node did not say it was current:\n{said}"
    );
}

/// Turning the check off in config turns off the request, so a `--check` that then went and asked
/// anyway would be the command going round the operator's own decision.
#[test]
fn check_refuses_rather_than_asking_when_discovery_is_off() {
    let installed = Installed::new();
    let config_dir = installed.home.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("a config dir");
    // With an answer already on disk from before the switch was thrown. Off has to mean the node
    // has nothing to say, not that it stops asking and keeps reporting — the wire goes quiet the
    // moment the switch is thrown, and these two must not disagree with it.
    std::fs::create_dir_all(installed.state()).expect("a state dir");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    std::fs::write(
        installed.state().join("update.json"),
        format!(r#"{{"latest":"v99.9.9","checked_at":{now},"ok":true}}"#),
    )
    .expect("a stale cache");
    std::fs::write(
        config_dir.join("config.toml"),
        format!(
            "node_id = \"01JTEST\"\nnode_name = \"front\"\nstate_dir = {:?}\n\n[update]\ncheck = false\n",
            installed.state().display().to_string()
        ),
    )
    .expect("a config");

    let out = installed.run(&["update", "--check"], None);
    let said = text(&out);
    assert!(!out.status.success(), "--check ignored the off switch:\n{said}");
    assert!(
        said.contains("check = false") || said.contains("is off"),
        "the refusal did not say why:\n{said}"
    );
    assert!(
        !said.contains("99.9.9"),
        "the switch was off and it reported an answer anyway:\n{said}"
    );

    let out = installed.run(&["status"], None);
    let said = text(&out);
    assert!(
        !said.contains("99.9.9"),
        "status ignored the off switch and reported a release the wire is not carrying:\n{said}"
    );
}

/// The command names the release it is going for before it fetches 16 MB of it, so a rollback is
/// something the operator can see going right.
#[test]
fn update_names_the_version_it_was_asked_for() {
    let installed = Installed::new();
    let release = Release::built(installed.home.path(), WORKS);
    let out = installed.run(&["update", "--version", "v0.1.0"], Some(&release));
    let said = text(&out);
    assert!(
        said.contains("v0.1.0"),
        "the command did not say which release it was installing:\n{said}"
    );
}

/// `kampr status` is where an operator looks when something is off, and it answers from the
/// cache — never from a request, because status has to work on a machine with no route out.
#[test]
fn status_names_an_available_release_without_asking_anyone() {
    let installed = Installed::new();
    std::fs::create_dir_all(installed.state()).expect("a state dir");
    let init = installed.run(&["init"], None);
    assert!(init.status.success(), "kampr init failed:\n{}", text(&init));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    std::fs::write(
        installed.state().join("update.json"),
        format!(r#"{{"latest":"v99.9.9","checked_at":{now},"ok":true}}"#),
    )
    .expect("a cache");

    let out = installed.run(&["status"], None);
    let said = text(&out);
    assert!(out.status.success(), "kampr status failed:\n{said}");
    assert!(
        said.lines()
            .any(|line| line.starts_with("kampr ") && line.contains("99.9.9")),
        "status did not say a release was waiting on the line that names the build:\n{said}"
    );
}

/// The hole the checksum cannot see: a base decides where the tarball comes from *and* where the
/// SHA256SUMS it is checked against comes from, so an attacker who can leave one environment
/// variable in the shell that later runs `kampr update` gets a binary of their choosing installed
/// with "checksum verified: yes" printed underneath it. `kampr update` clears the variable for the
/// same reason it clears `KAMPR_ALLOW_UNVERIFIED`.
#[test]
fn a_base_url_in_the_environment_cannot_choose_what_this_host_installs() {
    let installed = Installed::new();
    let genuine = Release::built(installed.home.path(), WORKS);

    // A whole release of the attacker's own: their tarball, and checksums that match it.
    let theirs = installed.home.path().join("theirs");
    let stage = theirs.join("stage");
    std::fs::create_dir_all(&stage).expect("a stage dir");
    let binary = stage.join("kampr");
    std::fs::write(&binary, TAMPERED).expect("a tampered binary");
    chmod_x(&binary);
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(theirs.join(&genuine.asset))
        .arg("-C")
        .arg(&stage)
        .arg("kampr")
        .status()
        .expect("tar");
    assert!(tar.success(), "could not build the attacker's release");
    let digest = Command::new("sha256sum")
        .arg(theirs.join(&genuine.asset))
        .output()
        .expect("sha256sum");
    let digest = String::from_utf8_lossy(&digest.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string();
    std::fs::write(
        theirs.join("SHA256SUMS"),
        format!("{digest}  {}\n", genuine.asset),
    )
    .expect("their SHA256SUMS");

    let mut command = Command::new(&installed.binary);
    command
        .arg("update")
        .arg("--config-dir")
        .arg(installed.home.path().join("config"))
        .arg("--state-dir")
        .arg(installed.home.path().join("state"))
        .env("HOME", installed.home.path())
        .env("XDG_CONFIG_HOME", installed.home.path().join("xdg"))
        .env("PATH", genuine.path())
        .env("KAMPR_BASE_URL", format!("file://{}", theirs.display()));
    let out = output(&mut command);
    let said = text(&out);

    assert_ne!(
        installed.version(),
        "kampr 0.0.0-tampered",
        "one environment variable redirected the whole verification chain and this host is now \
         running a binary an attacker chose:\n{said}"
    );
    assert!(
        !said.contains("0.0.0-tampered"),
        "the installer went to the base the environment named:\n{said}"
    );
}

/// An absent signature is the free downgrade: serve a tarball, serve checksums that match it, and
/// publish no bundle. At the canonical base that is not an unsigned release — it is not this
/// project's release, because the workflow signs every one of them.
#[test]
fn a_release_from_the_canonical_base_with_no_signature_is_refused() {
    let installed = Installed::new();
    let release = Release::built(installed.home.path(), WORKS);
    release.drop_bundle();
    let before = installed.digest();

    let out = installed.run(&["update"], Some(&release));
    let said = text(&out);
    assert!(
        !out.status.success(),
        "an unsigned release was installed:\n{said}"
    );
    assert!(
        said.contains("signature"),
        "the refusal did not say what was missing:\n{said}"
    );
    assert_eq!(
        installed.digest(),
        before,
        "the binary was replaced anyway:\n{said}"
    );
}
