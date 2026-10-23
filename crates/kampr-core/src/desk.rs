//! A native session shown in a herdr pane (docs/17 §4): which herdr panes are hosting one, and
//! what herdr is told about them so the desk draws each as an agent.

use crate::provider::AgentStatus;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hosting {
    pub socket: PathBuf,
    pub herdr_pane: String,
    pub host_pid: u32,
    pub native_pane: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskReport {
    pub agent: String,
    pub status: AgentStatus,
    pub title: String,
    pub resume_argv: Vec<String>,
}

pub trait Desk: Send + Sync {
    fn hosting(&self) -> Vec<Hosting>;
    fn report(&self, native_pane: &str) -> Option<DeskReport>;
    fn changes(&self) -> watch::Receiver<u64>;
}

#[derive(Clone)]
pub struct DeskHosts(pub Arc<dyn Desk>);

impl std::fmt::Debug for DeskHosts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeskHosts")
    }
}

impl DeskHosts {
    pub fn on(&self, socket: &Path) -> Vec<Hosting> {
        self.0
            .hosting()
            .into_iter()
            .filter(|h| same_socket(&h.socket, socket))
            .collect()
    }
}

/// A pane's `HERDR_SOCKET_PATH` and a node's configured socket can name one socket two ways.
pub fn same_socket(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canonical(a) == canonical(b)
}

/// herdr's agent states have no `done`: it synthesises that itself from an `idle` on an unfocused
/// pane (#578).
pub fn herdr_state(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Idle | AgentStatus::Done | AgentStatus::Unknown => "idle",
    }
}

/// 0.9.1 accepts `resume_argv` and ignores it; 0.9.2 is the first to run it (#583).
pub fn resumes(version: &str) -> bool {
    let mut parts = version
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    (next(), next(), next()) >= (0, 9, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_herdr_that_runs_resume_argv_is_sent_one() {
        for (version, expected) in [
            ("0.9.1", false),
            ("0.9.2", true),
            ("0.9.5", true),
            ("0.10.0", true),
            ("1.0.0", true),
            ("0.8.9", false),
            ("", false),
        ] {
            assert_eq!(resumes(version), expected, "{version}");
        }
    }
}
