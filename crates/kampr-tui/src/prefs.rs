use crate::sidebar::Order;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How many panes a herd remembers having been looked at. Enough for a switcher's empty query to
/// be a useful list, few enough that a pane long gone does not linger in the file.
const RECENT: usize = 20;

/// What this terminal remembers between launches, per device and never on the node: the node's
/// `prefs` follow the operator between machines, and these are about this desk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub order: Order,
    pub sidebar: Option<u16>,
    pub toast: Option<crate::alert::Setting>,
    /// Most recent first, keyed by the herd's origin: a pane id means nothing on another herd.
    pub recent: BTreeMap<String, Vec<String>>,
}

impl Prefs {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("tui.toml")
    }

    /// A missing or unreadable file is a first launch, not a reason to refuse to open.
    pub fn load(config_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(config_dir))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(Self::path(config_dir), text)
    }

    pub fn recent(&self, origin: &str) -> &[String] {
        self.recent.get(origin).map(Vec::as_slice).unwrap_or_default()
    }

    /// Whether anything moved, so a refocus on the pane already first writes nothing.
    pub fn viewed(&mut self, origin: &str, pane: &str) -> bool {
        let list = self.recent.entry(origin.to_string()).or_default();
        if list.first().is_some_and(|first| first == pane) {
            return false;
        }
        list.retain(|seen| seen != pane);
        list.insert(0, pane.to_string());
        list.truncate(RECENT);
        true
    }
}
