use clap::Args;
use std::path::{Path, PathBuf};

/// Durable state never lives in the plugin root. A GitHub-installed plugin root is a managed
/// checkout that Herdr replaces wholesale on reinstall, and taking the device database with it
/// would silently unpair every phone.
#[derive(Debug, Clone, Args)]
pub struct Dirs {
    #[arg(long, env = "KAMPR_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,
    #[arg(long, env = "KAMPR_STATE_DIR")]
    pub state_dir: Option<PathBuf>,
}

impl Dirs {
    pub fn config(&self) -> PathBuf {
        self.config_dir
            .clone()
            .unwrap_or_else(kampr_node::config::default_config_dir)
    }

    /// `None` when the caller did not say. The config records where the node actually keeps its
    /// state, and that answer beats a guess.
    pub fn state_override(&self) -> Option<&Path> {
        self.state_dir.as_deref()
    }

    pub fn state(&self) -> PathBuf {
        self.state_dir
            .clone()
            .unwrap_or_else(kampr_node::config::default_state_dir)
    }
}
