use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::process::PaneProcess;

/// The pane→session record the node keeps for a sticky harness.
///
/// A sticky harness (pi) names its session only while a tool is running and writes nothing to
/// disk that ties the process to the session ([#542](#)), so an idle pane has no handle the node
/// can re-derive — and herdr's own report path does not surface one either
/// ([#548](#)). The record of what the pane was last seen on is therefore the node's, and it has
/// to outlive the node: it is what an idle pane lands on after a restart, the way the in-memory
/// stickiness lands on it before one.
///
/// **The record is guarded by the process it was seen with.** A pid and start that changed is a
/// different agent in the same pane, and the record of the run before it is somebody else's; a
/// pane that `cd`'d or runs a different harness is the same. The guard is the whole of the
/// staleness check, and a record that fails it is refused rather than followed — the directory
/// stays out of a sticky pane's answer for exactly the reason the cwd handle was
/// ([#542](#)).
pub struct Ledger {
    path: Option<PathBuf>,
    entries: Mutex<HashMap<String, Entry>>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Entry {
    agent: String,
    cwd: String,
    session: PathBuf,
    pid: u32,
    start: Option<String>,
}

impl Ledger {
    /// The persistent half: the file under the node's state directory, empty where absent, and
    /// empty where the file is not this node's to read — a corrupt record is no record, and the
    /// node rebuilds it from the first live handle it sees.
    pub fn load(path: impl Into<PathBuf>) -> Arc<Self> {
        let path = path.into();
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Arc::new(Self {
            path: Some(path),
            entries: Mutex::new(entries),
        })
    }

    /// The half a test keeps in memory: the same records, no file.
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self {
            path: None,
            entries: Mutex::new(HashMap::new()),
        })
    }

    /// What the pane was last seen on, when the pane is the same pane: same harness, same
    /// directory, and — where the pane has a process — the same process. A pane with no process
    /// has nothing to contradict the record, and that is the case the record exists for: the
    /// agent between tools. The transcript has to still be there — a record of a deleted file is
    /// a record, not a conversation.
    pub fn lookup(
        &self,
        pane: &str,
        agent: &str,
        cwd: &str,
        process: Option<&PaneProcess>,
    ) -> Option<PathBuf> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(pane)?;
        if entry.agent != agent || entry.cwd != cwd {
            return None;
        }
        if let Some(process) = process
            && (entry.pid != process.pid || entry.start != process.start)
        {
            return None;
        }
        entry.session.is_file().then(|| entry.session.clone())
    }

    /// The pane was seen on a session. Written on change only, so a steady pane costs nothing.
    pub fn record(&self, pane: &str, agent: &str, cwd: &str, session: &Path, process: &PaneProcess) {
        let entry = Entry {
            agent: agent.to_string(),
            cwd: cwd.to_string(),
            session: session.to_path_buf(),
            pid: process.pid,
            start: process.start.clone(),
        };
        let mut entries = self.entries.lock().unwrap();
        if entries.get(pane) == Some(&entry) {
            return;
        }
        entries.insert(pane.to_string(), entry.clone());
        if let Some(path) = &self.path {
            let _ = std::fs::write(path, serde_json::to_vec(&*entries).unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32) -> PaneProcess {
        PaneProcess {
            pid,
            start: Some("12345".into()),
            ..Default::default()
        }
    }

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ledger-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_record_outlives_the_process_that_kept_it_in_memory() {
        let dir = scratch();
        let file = dir.join("s.jsonl");
        std::fs::write(&file, "{}").unwrap();
        let path = dir.join("ledger.json");

        Ledger::load(&path).record("n/w1:p1", "pi", "/home/u/x", &file, &process(4242));
        let again = Ledger::load(&path);
        assert_eq!(
            again.lookup("n/w1:p1", "pi", "/home/u/x", Some(&process(4242))),
            Some(file.clone())
        );
    }

    #[test]
    fn a_changed_process_is_a_different_agent_and_the_record_is_somebody_elses() {
        let dir = scratch();
        let file = dir.join("s.jsonl");
        std::fs::write(&file, "{}").unwrap();
        let ledger = Ledger::ephemeral();
        ledger.record("n/w1:p1", "pi", "/home/u/x", &file, &process(4242));
        assert!(
            ledger
                .lookup("n/w1:p1", "pi", "/home/u/x", Some(&process(4243)))
                .is_none()
        );
        assert!(
            ledger
                .lookup("n/w1:p1", "pi", "/home/u/y", Some(&process(4242)))
                .is_none()
        );
        assert!(
            ledger
                .lookup("n/w1:p1", "omp", "/home/u/x", Some(&process(4242)))
                .is_none()
        );
        assert!(
            ledger
                .lookup("n/w1:p2", "pi", "/home/u/x", Some(&process(4242)))
                .is_none()
        );
    }

    #[test]
    fn a_record_of_a_deleted_file_is_no_conversation() {
        let dir = scratch();
        let file = dir.join("s.jsonl");
        std::fs::write(&file, "{}").unwrap();
        let ledger = Ledger::ephemeral();
        ledger.record("n/w1:p1", "pi", "/home/u/x", &file, &process(4242));
        std::fs::remove_file(&file).unwrap();
        assert!(
            ledger
                .lookup("n/w1:p1", "pi", "/home/u/x", Some(&process(4242)))
                .is_none()
        );
    }

    #[test]
    fn a_corrupt_file_is_no_record_and_the_ledger_rebuilds() {
        let dir = scratch();
        let path = dir.join("ledger.json");
        std::fs::write(&path, "not json").unwrap();
        let ledger = Ledger::load(&path);
        assert!(
            ledger
                .lookup("n/w1:p1", "pi", "/home/u/x", Some(&process(4242)))
                .is_none()
        );
        let file = dir.join("s.jsonl");
        std::fs::write(&file, "{}").unwrap();
        ledger.record("n/w1:p1", "pi", "/home/u/x", &file, &process(4242));
        assert_eq!(
            Ledger::load(&path).lookup("n/w1:p1", "pi", "/home/u/x", Some(&process(4242))),
            Some(file)
        );
    }
}
