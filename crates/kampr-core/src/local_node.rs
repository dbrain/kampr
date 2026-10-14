//! The part of a node's `config.toml` that a client on the same machine reads to reach it: who it
//! is, where it listens, and where its device store is. The node's own `Config` is built from
//! these same types, so the two halves cannot read the file differently.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 8790;

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

impl Server {
    pub fn exposed(&self) -> bool {
        self.bind_addr().is_ok_and(|addr| !addr.ip().is_loopback())
    }

    pub fn bind_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        self.bind.parse()
    }

    pub fn scheme(&self) -> &'static str {
        if self.tls.enabled { "https" } else { "http" }
    }

    /// Explicit configuration wins; otherwise the host comes from the bind address, with a
    /// wildcard bind resolved to this machine's LAN address rather than `0.0.0.0`.
    pub fn origin(&self) -> String {
        if !self.origin.is_empty() {
            return self.origin.trim_end_matches('/').to_string();
        }
        let (host, port) = match self.bind_addr() {
            Ok(addr) => (visible_host(addr.ip()), addr.port()),
            Err(_) => ("127.0.0.1".to_string(), DEFAULT_PORT),
        };
        format!("{}://{host}:{port}", self.scheme())
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
pub struct Auth {
    /// Overrides the RP ID derived from the origin — for a node behind a proxy whose public
    /// hostname differs from the one it binds. Ignored when the origin cannot do passkeys at all.
    pub rp_id: String,
    pub token_days: u64,
    pub pairing_ttl_secs: u64,
    pub audit: bool,
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

#[derive(Debug, Clone, Deserialize)]
pub struct LocalNode {
    pub node_id: String,
    pub node_name: String,
    #[serde(default)]
    pub state_dir: String,
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub auth: Auth,
}

#[derive(Debug, thiserror::Error)]
pub enum LocalNodeError {
    #[error("reading {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("parsing {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
}

impl LocalNode {
    pub fn load(config_dir: &Path) -> Result<Self, LocalNodeError> {
        let path = config_path(config_dir);
        let text = std::fs::read_to_string(&path).map_err(|e| LocalNodeError::Io(path.clone(), e))?;
        toml::from_str(&text).map_err(|e| LocalNodeError::Parse(path, e))
    }

    pub fn origin(&self) -> String {
        self.server.origin()
    }

    pub fn resolve_state_dir(&self, explicit: Option<&Path>) -> PathBuf {
        resolve_state_dir(&self.state_dir, explicit)
    }
}

pub fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("config.toml")
}

pub fn state_db(state_dir: &Path) -> PathBuf {
    state_dir.join("kampr.db")
}

pub fn audit_path(state_dir: &Path) -> PathBuf {
    state_dir.join("audit.jsonl")
}

/// The explicit argument wins, then what `kampr init` recorded, then the XDG default.
pub fn resolve_state_dir(recorded: &str, explicit: Option<&Path>) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| (!recorded.is_empty()).then(|| PathBuf::from(recorded)))
        .unwrap_or_else(default_state_dir)
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
