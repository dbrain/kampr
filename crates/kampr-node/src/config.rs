use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 8790;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node_id: String,
    pub node_name: String,
    /// Where this node keeps devices, tokens and the audit log. Recorded here because the plugin
    /// dispatcher invokes `kampr status` and `kampr url` with only a config directory, and a node
    /// that answered from the wrong database would report nobody paired.
    #[serde(default)]
    pub state_dir: String,
    /// Where `node.key` lives. Recorded for the same reason as `state_dir`: the mesh identity is
    /// generated at `kampr init` into the config directory, and a node started with only a state
    /// directory would otherwise generate a second one and be a different node to its hub.
    #[serde(default)]
    pub config_dir: String,
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub herdr: Herdr,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub journals: Journals,
    #[serde(default)]
    pub mesh: Mesh,
    #[serde(default)]
    pub push: Push,
    #[serde(default)]
    pub android: Android,
    #[serde(default)]
    pub update: Update,
}

/// The Android app this node will let hold a passkey for it.
///
/// Credential Manager runs no WebAuthn ceremony for a native app until the relying party — this
/// node, at the operator's own domain — publishes `/.well-known/assetlinks.json` naming that app's
/// package and signing certificate. The defaults are the app kobup ships, so an operator who
/// installs the release APK configures nothing.
///
/// It is configurable because two other builds exist and neither is signed with the release key: a
/// debug build carries the machine's own `~/.android/debug.keystore`, and a build from source
/// carries whatever keystore made it. Kampr on such a phone prints its own fingerprint when the
/// node does not name it, and it goes here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Android {
    #[serde(rename = "package")]
    pub package_name: String,
    pub fingerprints: Vec<String>,
}

impl Default for Android {
    fn default() -> Self {
        Self {
            package_name: "dev.kampr.app".into(),
            fingerprints: vec![crate::assetlinks::RELEASE_FINGERPRINT.into()],
        }
    }
}

/// Whether this node asks GitHub what the latest release is.
///
/// **A legitimate off switch, not an edge case.** Some operators will not have their node reach
/// GitHub at all, and turning it off has to mean the request is never made — not that the answer
/// is hidden — because the node still reports the answer to every client and to a hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Update {
    pub check: bool,
    pub repo: String,
    /// The API root, for a mirror or a proxy. Empty is GitHub's own.
    pub api: String,
}

impl Default for Update {
    fn default() -> Self {
        Self {
            check: true,
            repo: "dbrain/kampr".into(),
            api: String::new(),
        }
    }
}

impl Update {
    pub fn latest_release_url(&self) -> String {
        let api = match self.api.trim_end_matches('/') {
            "" => "https://api.github.com",
            explicit => explicit,
        };
        format!("{api}/repos/{}/releases/latest", self.repo)
    }
}

/// The hub role, which is configuration rather than a build.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Mesh {
    /// Whether this node accepts inbound peer links at all. Enrolment already decides *who* may
    /// join; this is the switch that says the door is not there.
    ///
    /// **Off by default**, for the same reason `bind` is loopback: `/mesh` is a second
    /// unauthenticated protocol endpoint, and a node that only ever serves a phone should not be
    /// answering it on the public internet. `kampr mesh invite` refuses until this is on.
    pub accept: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    /// Loopback by default (P3.11). Kampr access is unrestricted code execution on this host, so
    /// reaching the LAN is a decision the operator makes with `kampr init --bind`, not one the
    /// default makes for them.
    pub bind: String,
    /// The one URL clients are expected to use. Empty means "work it out from the bind address"
    /// — fine for Tier 0, but a passkey is registered against an origin and a changed origin
    /// invalidates it, so anything above Tier 0 must set this explicitly.
    pub origin: String,
    pub extra_origins: Vec<String>,
    /// **Opt-in, never inferred.** A proxy's `X-Forwarded-*` headers are trivially forgeable by
    /// anyone who can reach the node directly.
    pub trust_proxy: bool,
    pub tls: Tls,
}

impl Server {
    /// Whether this bind is reachable from anywhere but this machine.
    pub fn exposed(&self) -> bool {
        self.bind
            .parse::<SocketAddr>()
            .is_ok_and(|addr| !addr.ip().is_loopback())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tls {
    pub enabled: bool,
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Herdr {
    pub binary: String,
    /// Empty means herdr's own resolution order: `HERDR_SOCKET_PATH`, `HERDR_SESSION`, default.
    pub socket: String,
    /// Which named herdr sessions this node serves *beyond* the one `socket` resolves to — each
    /// a separate server with its own socket, and so a separate node in the herd model.
    ///
    /// Absent serves every session running on the host; an empty list serves only the configured
    /// one. Both states have to be expressible, which is why this is an option and not a list
    /// whose emptiness means "all".
    pub sessions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    /// Overrides the RP ID derived from the origin — for a node behind a proxy whose public
    /// hostname differs from the one it binds. Ignored when the origin cannot do passkeys at all.
    pub rp_id: String,
    pub token_days: u64,
    pub pairing_ttl_secs: u64,
    pub audit: bool,
}

/// Notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Push {
    /// The operator's off switch. Push is otherwise available whenever the origin is a secure
    /// context and a VAPID key exists — it is never inferred from anything else.
    pub enabled: bool,
    /// The VAPID `sub` claim: a `mailto:` or `https:` URI naming whoever runs this node. Empty
    /// derives one from the origin, which is what a self-hoster wants and what a push service
    /// will accept.
    pub subject: String,
}

impl Default for Push {
    fn default() -> Self {
        Self {
            enabled: true,
            subject: String::new(),
        }
    }
}

/// Where the harnesses keep their transcripts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Journals {
    /// The home directory holding `.claude` and `.codex`. Empty means this process's own `$HOME`;
    /// set it when the node runs as a service user whose home is not the operator's.
    pub home: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Frames a slow client may have queued before the node drops its patches and resets it.
    pub client_queue: usize,
    pub scrollback_max_rows: usize,
    /// Client sessions served at once. One token opens as many sockets as its holder likes and
    /// each is a session with its own queue, so a publicly reachable node needs a bound of its
    /// own rather than the proxy's.
    pub sockets: usize,
    /// Mesh handshakes in flight at once. Each one costs an argon2id pass at 19 MiB against a
    /// join code an anonymous caller chose, so this is what bounds that memory — a limiter keyed
    /// on the source address cannot, because addresses rotate.
    pub mesh_handshakes: usize,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            bind: format!("127.0.0.1:{DEFAULT_PORT}"),
            origin: String::new(),
            extra_origins: Vec::new(),
            trust_proxy: false,
            tls: Tls::default(),
        }
    }
}

impl Default for Herdr {
    fn default() -> Self {
        Self {
            binary: "herdr".into(),
            socket: String::new(),
            sessions: None,
        }
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            rp_id: String::new(),
            token_days: 30,
            pairing_ttl_secs: 600,
            audit: true,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            client_queue: 256,
            scrollback_max_rows: kampr_core::scrollback::DEFAULT_MAX_ROWS,
            sockets: 64,
            mesh_handshakes: 4,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no config at {0} — run `kampr init` first")]
    Missing(PathBuf),
    #[error("reading {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("parsing {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("writing {0}: {1}")]
    Encode(PathBuf, toml::ser::Error),
}

impl Config {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("config.toml")
    }

    pub fn bootstrap(node_name: &str) -> Self {
        Self {
            node_id: ulid::Ulid::generate().to_string(),
            node_name: node_name.to_string(),
            state_dir: String::new(),
            config_dir: String::new(),
            server: Server::default(),
            herdr: Herdr::default(),
            auth: Auth::default(),
            limits: Limits::default(),
            journals: Journals::default(),
            mesh: Mesh::default(),
            push: Push::default(),
            android: Android::default(),
            update: Update::default(),
        }
    }

    /// The VAPID subject: configured, or derived from the origin.
    pub fn push_subject(&self) -> String {
        match self.push.subject.trim() {
            "" => kampr_push::subject_for(&self.origin()),
            explicit => explicit.to_string(),
        }
    }

    pub fn journal_home(&self) -> PathBuf {
        if self.journals.home.is_empty() {
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
        } else {
            PathBuf::from(&self.journals.home)
        }
    }

    pub fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let path = Self::path(config_dir);
        if !path.exists() {
            return Err(ConfigError::Missing(path));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| ConfigError::Io(path.clone(), e))?;
        toml::from_str(&text).map_err(|e| ConfigError::Parse(path, e))
    }

    pub fn save(&self, config_dir: &Path) -> Result<PathBuf, ConfigError> {
        let path = Self::path(config_dir);
        // The config directory holds `node.key`, so it is as private as the state directory.
        kampr_auth::private_dir(config_dir).map_err(|e| ConfigError::Io(path.clone(), e))?;
        let text = toml::to_string_pretty(self).map_err(|e| ConfigError::Encode(path.clone(), e))?;
        std::fs::write(&path, text).map_err(|e| ConfigError::Io(path.clone(), e))?;
        Ok(path)
    }

    pub fn bind_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        self.server.bind.parse()
    }

    /// The origin clients are told to use. Explicit configuration wins; otherwise the scheme
    /// comes from whether the node terminates TLS itself and the host from the bind address,
    /// with a wildcard bind resolved to this machine's LAN address rather than `0.0.0.0`.
    pub fn origin(&self) -> String {
        if !self.server.origin.is_empty() {
            return self.server.origin.trim_end_matches('/').to_string();
        }
        let scheme = if self.server.tls.enabled { "https" } else { "http" };
        let (host, port) = match self.bind_addr() {
            Ok(addr) => (visible_host(addr.ip()), addr.port()),
            Err(_) => ("127.0.0.1".to_string(), DEFAULT_PORT),
        };
        format!("{scheme}://{host}:{port}")
    }

    /// Every origin a browser may legitimately present.
    ///
    /// Derived from the *bind address*, never from the request's own `Host` — reflecting `Host`
    /// makes `Origin == Host` true for a DNS-rebinding attacker who points a domain at this
    /// node's address, which satisfies the same-origin gate with the attacker's own header.
    pub fn allowed_origins(&self) -> Vec<String> {
        let mut origins = vec![self.origin()];
        origins.extend(self.server.extra_origins.iter().cloned());
        let scheme = if self.server.tls.enabled { "https" } else { "http" };
        if let Ok(addr) = self.bind_addr() {
            let port = addr.port();
            let mut hosts = Vec::new();
            if addr.ip().is_loopback() || addr.ip().is_unspecified() {
                hosts.extend([
                    "127.0.0.1".to_string(),
                    "localhost".to_string(),
                    "[::1]".to_string(),
                ]);
            }
            if addr.ip().is_unspecified() {
                hosts.extend(lan_address().map(|ip| ip.to_string()));
            } else {
                hosts.push(addr.ip().to_string());
            }
            origins.extend(hosts.into_iter().map(|host| format!("{scheme}://{host}:{port}")));
        }
        origins.sort();
        origins.dedup();
        origins
    }

    /// The explicit argument wins, then what `kampr init` recorded, then the XDG default.
    pub fn resolve_state_dir(&self, explicit: Option<&Path>) -> PathBuf {
        explicit
            .map(Path::to_path_buf)
            .or_else(|| (!self.state_dir.is_empty()).then(|| PathBuf::from(&self.state_dir)))
            .unwrap_or_else(default_state_dir)
    }

    pub fn state_db(state_dir: &Path) -> PathBuf {
        state_dir.join("kampr.db")
    }

    /// The VAPID keypair. In the state directory beside the device database, because it is the
    /// same kind of secret and gets the same 0600.
    pub fn vapid_path(state_dir: &Path) -> PathBuf {
        state_dir.join("vapid.pem")
    }

    pub fn audit_path(state_dir: &Path) -> PathBuf {
        state_dir.join("audit.jsonl")
    }

    pub fn node_key_path(config_dir: &Path) -> PathBuf {
        config_dir.join("node.key")
    }

    /// Where this node's own mesh key is, from what `kampr init` recorded or the XDG default.
    pub fn key_path(&self) -> PathBuf {
        let dir = match self.config_dir.is_empty() {
            true => default_config_dir(),
            false => PathBuf::from(&self.config_dir),
        };
        Self::node_key_path(&dir)
    }
}

pub fn default_config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("kampr")
}

pub fn default_state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("kampr")
}

fn xdg(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable).map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(fallback)
    })
}

fn visible_host(ip: IpAddr) -> String {
    if !ip.is_unspecified() {
        return ip.to_string();
    }
    lan_address().map_or_else(|| "127.0.0.1".to_string(), |ip| ip.to_string())
}

/// The address this machine would use to reach the outside world. `connect` on a UDP socket only
/// picks a route — it sends nothing and needs nothing to be reachable — which is the one portable
/// way to ask "which of my interfaces would a phone on the LAN find me on".
pub fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_binds_off_loopback_without_being_asked_to() {
        assert!(
            Config::bootstrap("x").bind_addr().unwrap().ip().is_loopback(),
            "P3.11: loopback by default, LAN by explicit opt-in"
        );
        assert!(!Server::default().exposed());
        let mut wide = Config::bootstrap("x");
        wide.server.bind = format!("0.0.0.0:{DEFAULT_PORT}");
        assert!(wide.server.exposed());
        wide.server.bind = format!("192.168.1.24:{DEFAULT_PORT}");
        assert!(wide.server.exposed());
    }

    #[test]
    fn a_bootstrap_config_round_trips_through_toml() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("config");
        let dir = dir.as_path();
        let config = Config::bootstrap("comingclean");
        config.save(dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700,
                "node.key lives here"
            );
        }
        let loaded = Config::load(dir).unwrap();
        assert_eq!(loaded.node_id, config.node_id);
        assert_eq!(loaded.node_name, "comingclean");
        assert_eq!(loaded.server.bind, config.server.bind);
        assert!(!loaded.server.trust_proxy, "proxy trust is never inferred");
    }

    #[test]
    fn the_state_directory_survives_a_call_that_only_gives_the_config_directory() {
        let mut config = Config::bootstrap("x");
        assert_eq!(config.resolve_state_dir(None), default_state_dir());
        config.state_dir = "/var/lib/kampr".into();
        assert_eq!(config.resolve_state_dir(None), PathBuf::from("/var/lib/kampr"));
        assert_eq!(
            config.resolve_state_dir(Some(Path::new("/elsewhere"))),
            PathBuf::from("/elsewhere")
        );
    }

    #[test]
    fn a_node_finds_its_own_key_from_what_init_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::bootstrap("x");
        config.config_dir = dir.path().display().to_string();
        assert_eq!(config.key_path(), dir.path().join("node.key"));
        assert_eq!(
            Config::bootstrap("x").key_path(),
            default_config_dir().join("node.key"),
            "an unrecorded config directory falls back to the XDG default, never to a second key"
        );
    }

    #[test]
    fn the_hub_role_is_configuration_and_is_off_until_it_is_asked_for() {
        assert!(!Config::bootstrap("x").mesh.accept);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            Config::path(dir.path()),
            "node_id = \"01J\"\nnode_name = \"x\"\n[mesh]\naccept = true\n",
        )
        .unwrap();
        assert!(Config::load(dir.path()).unwrap().mesh.accept);
    }

    #[test]
    fn a_missing_config_says_what_to_run() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Config::load(dir.path()), Err(ConfigError::Missing(_))));
    }

    #[test]
    fn absent_sections_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(Config::path(dir.path()), "node_id = \"01J\"\nnode_name = \"x\"\n").unwrap();
        let c = Config::load(dir.path()).unwrap();
        assert_eq!(c.server.bind, format!("127.0.0.1:{DEFAULT_PORT}"));
        assert_eq!(c.herdr.binary, "herdr");
        assert_eq!(c.auth.token_days, 30);
    }

    #[test]
    fn an_explicit_origin_wins_over_the_bind_address() {
        let mut c = Config::bootstrap("x");
        c.server.origin = "https://kampr.example.com/".into();
        assert_eq!(c.origin(), "https://kampr.example.com");
    }

    #[test]
    fn a_wildcard_bind_never_advertises_itself_as_zero() {
        let mut c = Config::bootstrap("x");
        c.server.bind = format!("0.0.0.0:{DEFAULT_PORT}");
        let origin = c.origin();
        assert!(origin.starts_with("http://"), "{origin}");
        assert!(!origin.contains("0.0.0.0"), "{origin}");
        assert!(origin.ends_with(&format!(":{DEFAULT_PORT}")), "{origin}");
    }

    #[test]
    fn the_origin_allowlist_comes_from_the_bind_and_never_from_a_request() {
        let mut c = Config::bootstrap("x");
        let loopback = c.allowed_origins();
        assert!(loopback.contains(&format!("http://127.0.0.1:{DEFAULT_PORT}")));
        assert!(loopback.contains(&format!("http://localhost:{DEFAULT_PORT}")));
        assert!(!loopback.iter().any(|o| o.contains("0.0.0.0")));

        c.server.bind = "192.168.1.24:9000".into();
        c.server.extra_origins = vec!["http://kampr.local:9000".into()];
        let lan = c.allowed_origins();
        assert!(lan.contains(&"http://192.168.1.24:9000".to_string()));
        assert!(lan.contains(&"http://kampr.local:9000".to_string()));
        assert!(
            !lan.contains(&"http://127.0.0.1:9000".to_string()),
            "a specific bind advertises only itself"
        );
    }

    #[test]
    fn own_tls_changes_the_scheme_it_advertises() {
        let mut c = Config::bootstrap("x");
        c.server.bind = "127.0.0.1:9000".into();
        c.server.tls.enabled = true;
        assert_eq!(c.origin(), "https://127.0.0.1:9000");
    }
}
