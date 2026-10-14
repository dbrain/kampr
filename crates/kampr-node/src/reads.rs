use crate::herd::HerdModel;
use kampr_core::provider::AgentStatus;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::warn;

/// Recency-bounded rather than pruned against the herd: a herd rebuilt while herdr is unreachable
/// carries no panes, and forgetting every read then would re-raise them all when it came back.
const KEEP: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Read {
    pane: String,
    at: Option<String>,
}

/// The finishes the operator has read on some device, which herdr has no way to be told.
///
/// Read is keyed by the pane's `updated_at` as well as its id, the same rule a client's own
/// `SeenDone` keeps, so the next finish is unread again. A pane with no `updated_at` has nothing
/// to re-arm against and is matched by id alone.
pub struct Reads {
    path: PathBuf,
    read: Mutex<Vec<Read>>,
}

impl Reads {
    pub fn load(path: PathBuf) -> Self {
        let read = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                warn!(path = %path.display(), error = %e, "the read ledger is unreadable; starting empty");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        Self {
            path,
            read: Mutex::new(read),
        }
    }

    pub fn mark(&self, pane: &str, at: Option<String>) {
        let mut read = self.read.lock().unwrap();
        read.retain(|r| r.pane != pane);
        read.push(Read {
            pane: pane.to_string(),
            at,
        });
        let over = read.len().saturating_sub(KEEP);
        read.drain(..over);
        self.store(&read);
    }

    /// Publishes a read finish as what it is to the operator: an agent with nothing waiting.
    pub fn overlay(&self, model: &mut HerdModel) {
        let read = self.read.lock().unwrap();
        for pane in model
            .panes
            .iter_mut()
            .filter(|p| p.agent_status == AgentStatus::Done)
        {
            let seen = read
                .iter()
                .any(|r| r.pane == pane.id && (pane.updated_at.is_none() || r.at == pane.updated_at));
            if seen {
                pane.agent_status = AgentStatus::Idle;
            }
        }
    }

    fn store(&self, read: &[Read]) {
        let tmp = self.path.with_extension("json.tmp");
        let written = serde_json::to_vec(read)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = written {
            warn!(path = %self.path.display(), error = %e, "could not save the read ledger");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_read_is_the_one_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let reads = Reads::load(dir.path().join("reads.json"));
        for i in 0..=KEEP {
            reads.mark(&format!("01J/w{i}:p1"), None);
        }
        reads.mark("01J/w1:p1", None);
        let kept = Reads::load(dir.path().join("reads.json"));
        let panes: Vec<String> = kept.read.lock().unwrap().iter().map(|r| r.pane.clone()).collect();
        assert_eq!(panes.len(), KEEP);
        assert!(!panes.contains(&"01J/w0:p1".to_string()));
        assert_eq!(panes.last().map(String::as_str), Some("01J/w1:p1"));
    }
}
