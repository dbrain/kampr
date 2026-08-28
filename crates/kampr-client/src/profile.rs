use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What this machine's `kampr` knows about the herds it can reach.
///
/// It sits beside the node's own `config.toml` and is **not** part of it: a laptop with no node
/// has this file and nothing else, and a node's config must stay a description of the node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientConfig {
    /// Which profile a bare `kampr` opens when there is no node on this machine. Absent means
    /// the only one there is.
    pub default: Option<String>,
    pub profiles: BTreeMap<String, Profile>,
    /// The device this CLI minted for the node on this machine, so a second run reuses it rather
    /// than filling the device list with one row per invocation.
    pub local: Option<LocalDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub origin: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalDevice {
    pub node_id: String,
    pub device_id: String,
    pub token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} is not readable as TOML: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("writing {0}: {1}")]
    Encode(PathBuf, toml::ser::Error),
}

impl ClientConfig {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("client.toml")
    }

    /// A missing file is an empty one: a machine that has never paired anything is the ordinary
    /// starting state, not an error.
    pub fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let path = Self::path(config_dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::Io(path, e)),
        };
        toml::from_str(&text).map_err(|e| ConfigError::Parse(path, e))
    }

    /// 0600 in a 0700 directory, because it holds device tokens in the clear — the same thing
    /// every enrolled client holds and the same protection the node's own key gets.
    pub fn save(&self, config_dir: &Path) -> Result<PathBuf, ConfigError> {
        let path = Self::path(config_dir);
        kampr_auth::private_dir(config_dir).map_err(|e| ConfigError::Io(path.clone(), e))?;
        let text = toml::to_string_pretty(self).map_err(|e| ConfigError::Encode(path.clone(), e))?;
        std::fs::write(&path, text).map_err(|e| ConfigError::Io(path.clone(), e))?;
        kampr_auth::files::chmod(&path, 0o600).map_err(|e| ConfigError::Io(path.clone(), e))?;
        Ok(path)
    }

    /// The profile a bare `kampr` opens: the one named as default, or the only one there is.
    pub fn chosen(&self) -> Option<(&String, &Profile)> {
        if let Some(name) = &self.default
            && let Some(profile) = self.profiles.get(name)
        {
            return Some((name, profile));
        }
        match self.profiles.len() {
            1 => self.profiles.iter().next(),
            _ => None,
        }
    }
}
