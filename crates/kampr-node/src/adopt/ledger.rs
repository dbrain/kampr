use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// By name, so a container recreated under it (a new id) is adopted again.
    pub name: String,
    /// Minted once from the id it had when it was first adopted, and kept with the key.
    pub node_id: String,
}

/// `<state>/docker/adopted.json` and the child keys beside it, one per container name, and why
/// an adoption is failing while it is.
pub struct Ledger {
    dir: PathBuf,
    records: Mutex<Vec<Record>>,
    failures: Mutex<BTreeMap<String, String>>,
    revision: watch::Sender<u64>,
}

impl Ledger {
    pub fn open(dir: &Path) -> Self {
        let records = std::fs::read(dir.join("adopted.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            dir: dir.to_path_buf(),
            records: Mutex::new(records),
            failures: Mutex::default(),
            revision: watch::Sender::new(0),
        }
    }

    pub fn records(&self) -> Vec<Record> {
        self.records.lock().unwrap().clone()
    }

    pub fn get(&self, name: &str) -> Option<Record> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.name == name)
            .cloned()
    }

    pub fn child_of(&self, name: &str) -> Option<String> {
        self.get(name).map(|r| r.node_id)
    }

    pub fn remember(&self, record: Record) -> Result<()> {
        self.change(|records| {
            records.retain(|r| r.name != record.name);
            records.push(record);
        })
    }

    pub fn failure(&self, name: &str) -> Option<String> {
        self.failures.lock().unwrap().get(name).cloned()
    }

    /// Kept in memory only: a node starting looks at every adoption afresh.
    pub fn fail(&self, name: &str, why: Option<String>) {
        let mut failures = self.failures.lock().unwrap();
        let changed = match why {
            Some(why) => failures.insert(name.to_string(), why.clone()).as_ref() != Some(&why),
            None => failures.remove(name).is_some(),
        };
        drop(failures);
        if changed {
            self.revision.send_modify(|n| *n += 1);
        }
    }

    pub fn forget(&self, name: &str) -> Result<Option<Record>> {
        self.fail(name, None);
        let mut gone = None;
        self.change(|records| {
            if let Some(at) = records.iter().position(|r| r.name == name) {
                gone = Some(records.remove(at));
            }
        })?;
        Ok(gone)
    }

    pub fn key(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join("keys").join(format!("{safe}.key"))
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    fn change(&self, change: impl FnOnce(&mut Vec<Record>)) -> Result<()> {
        let mut records = self.records.lock().unwrap();
        let mut next = records.clone();
        change(&mut next);
        if next == *records {
            return Ok(());
        }
        kampr_auth::private_dir(&self.dir)?;
        let path = self.dir.join("adopted.json");
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, serde_json::to_vec_pretty(&next)?)
            .with_context(|| format!("writing {}", partial.display()))?;
        std::fs::rename(&partial, &path).with_context(|| format!("writing {}", path.display()))?;
        *records = next;
        drop(records);
        self.revision.send_modify(|n| *n += 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, node_id: &str) -> Record {
        Record {
            name: name.into(),
            node_id: node_id.into(),
        }
    }

    #[test]
    fn adoptions_survive_a_restart_and_a_forgotten_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let docker = dir.path().join("docker");
        let ledger = Ledger::open(&docker);
        assert!(ledger.records().is_empty());
        let changes = ledger.subscribe();
        ledger.remember(record("db", "01JH-c-aaaaaaaaaaaa")).unwrap();
        ledger.remember(record("web", "01JH-c-bbbbbbbbbbbb")).unwrap();
        ledger.remember(record("db", "01JH-c-aaaaaaaaaaaa")).unwrap();
        assert!(changes.has_changed().unwrap());

        let reopened = Ledger::open(&docker);
        assert_eq!(
            reopened.records(),
            [
                record("web", "01JH-c-bbbbbbbbbbbb"),
                record("db", "01JH-c-aaaaaaaaaaaa")
            ]
        );
        assert_eq!(reopened.child_of("db").as_deref(), Some("01JH-c-aaaaaaaaaaaa"));
        assert_eq!(
            reopened.forget("db").unwrap(),
            Some(record("db", "01JH-c-aaaaaaaaaaaa"))
        );
        assert_eq!(reopened.forget("db").unwrap(), None);
        assert_eq!(
            Ledger::open(&docker).records(),
            [record("web", "01JH-c-bbbbbbbbbbbb")]
        );
        assert!(Ledger::open(&docker).child_of("db").is_none());
    }

    #[test]
    fn a_failure_is_told_once_and_cleared_by_success_or_forgetting() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(dir.path());
        ledger.remember(record("db", "01JH-c-aaaaaaaaaaaa")).unwrap();
        let mut changes = ledger.subscribe();
        changes.mark_unchanged();

        ledger.fail("db", Some("gone".into()));
        assert!(changes.has_changed().unwrap());
        changes.mark_unchanged();
        ledger.fail("db", Some("gone".into()));
        assert!(
            !changes.has_changed().unwrap(),
            "the same failure again is no change"
        );
        assert_eq!(ledger.failure("db").as_deref(), Some("gone"));

        ledger.fail("db", None);
        assert!(changes.has_changed().unwrap());
        assert_eq!(ledger.failure("db"), None);

        ledger.fail("db", Some("again".into()));
        ledger.forget("db").unwrap();
        assert_eq!(ledger.failure("db"), None);
        assert!(Ledger::open(dir.path()).failure("db").is_none());
    }

    #[test]
    fn a_key_is_kept_per_container_name_inside_the_docker_directory() {
        let ledger = Ledger::open(Path::new("/s/docker"));
        assert_eq!(ledger.key("db-1"), Path::new("/s/docker/keys/db-1.key"));
        assert_eq!(ledger.key("../x"), Path::new("/s/docker/keys/.._x.key"));
    }
}
