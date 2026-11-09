use crate::api::Event;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const KEEP_SECS: i64 = 14 * 24 * 3600;
pub const MAX_BYTES: u64 = 2 << 20;

const JOURNALLED: [&str; 8] = [
    "start",
    "die",
    "oom",
    "kill",
    "restart",
    "stop",
    "health_status",
    "destroy",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub at: i64,
    /// `timeNano`, which orders the several dies a restart policy fits into one second and is
    /// where a follower resumes.
    pub nano: i64,
    pub id: String,
    pub name: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
}

impl Entry {
    pub fn from_event(event: &Event) -> Option<Self> {
        let (action, health) = match event.action.split_once(": ") {
            Some((action, health)) => (action, Some(health.trim().to_string())),
            None => (event.action.as_str(), None),
        };
        if !JOURNALLED.contains(&action) || (!event.kind.is_empty() && event.kind != "container") {
            return None;
        }
        let attribute = |key: &str| event.actor.attributes.get(key);
        let number = |key: &str| attribute(key).and_then(|v| v.parse().ok());
        Some(Self {
            at: event.time,
            nano: match event.time_nano {
                0 => event.time * 1_000_000_000,
                nano => nano,
            },
            id: event.actor.id.clone(),
            name: attribute("name").cloned().unwrap_or_default(),
            action: action.to_string(),
            exit_code: number("exitCode"),
            signal: number("signal"),
            health,
        })
    }
}

pub struct Journal {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
    bytes: u64,
}

impl Journal {
    pub fn open(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Self {
            path: Some(path.to_path_buf()),
            bytes: text.len() as u64,
            entries: text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect(),
        }
    }

    pub fn memory() -> Self {
        Self {
            path: None,
            entries: Vec::new(),
            bytes: 0,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn last_nano(&self) -> Option<i64> {
        self.entries.last().map(|e| e.nano)
    }

    /// `false` for an event already journalled, which is every event a resumed follower is
    /// replayed from `since`.
    pub fn record(&mut self, entry: Entry, now: i64) -> bool {
        if self.last_nano().is_some_and(|last| entry.nano <= last) {
            return false;
        }
        let line = serde_json::to_string(&entry).unwrap_or_default() + "\n";
        self.entries.push(entry);
        self.bytes += line.len() as u64;
        let stale = self.entries.first().is_some_and(|e| e.at < now - KEEP_SECS);
        if stale || self.bytes > MAX_BYTES {
            self.trim(now);
        } else {
            self.append(&line);
        }
        true
    }

    pub fn of(&self, id: &str, limit: usize) -> Vec<&Entry> {
        let mine: Vec<&Entry> = self.entries.iter().filter(|e| e.id == id).collect();
        mine[mine.len().saturating_sub(limit)..].to_vec()
    }

    fn append(&self, line: &str) {
        let Some(path) = &self.path else { return };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::OpenOptions::new().create(true).append(true).open(path))
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if let Err(e) = written {
            tracing::warn!(error = %e, path = %path.display(), "could not journal a docker event");
        }
    }

    /// Down to three quarters of the cap, so a full journal is rewritten once per quarter of it
    /// rather than on every event.
    fn trim(&mut self, now: i64) {
        self.entries.retain(|e| e.at >= now - KEEP_SECS);
        let lines: Vec<String> = self
            .entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default() + "\n")
            .collect();
        let mut keep = lines.len();
        let mut bytes: u64 = lines.iter().map(|l| l.len() as u64).sum();
        while bytes > MAX_BYTES / 4 * 3 && keep > 0 {
            keep -= 1;
            bytes -= lines[lines.len() - keep - 1].len() as u64;
        }
        let dropped = lines.len() - keep;
        self.entries.drain(..dropped);
        self.bytes = bytes;
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("jsonl.tmp");
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&tmp, lines[dropped..].concat()))
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            tracing::warn!(error = %e, path = %path.display(), "could not trim the docker journal");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn events(fixture: &str) -> Vec<Event> {
        fixture
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    pub(crate) fn journalled(fixture: &str) -> Vec<Entry> {
        events(fixture).iter().filter_map(Entry::from_event).collect()
    }

    fn entry(at: i64, action: &str) -> Entry {
        Entry {
            at,
            nano: at * 1_000_000_000,
            id: "c1".into(),
            name: "one".into(),
            action: action.into(),
            exit_code: None,
            signal: None,
            health: None,
        }
    }

    #[test]
    fn only_what_says_why_a_container_stopped_is_journalled() {
        let actions: Vec<_> = journalled(include_str!("../tests/fixtures/events.jsonl"))
            .into_iter()
            .map(|e| (e.name, e.action, e.exit_code, e.health))
            .collect();
        let want = |name: &str, action: &str, code: Option<i64>, health: Option<&str>| {
            (
                name.to_string(),
                action.to_string(),
                code,
                health.map(str::to_string),
            )
        };
        assert_eq!(
            actions,
            vec![
                want("kt-fx-run", "start", None, None),
                want("kt-fx-tty", "start", None, None),
                want("kt-fx-oom", "start", None, None),
                want("kt-fx-oom", "oom", None, None),
                want("kt-fx-exit", "start", None, None),
                want("kt-fx-oom", "die", Some(137), None),
                want("kt-fx-exit", "die", Some(3), None),
                want("kt-fx-run", "health_status", None, Some("healthy")),
            ]
        );
        let kill = journalled(include_str!("../tests/fixtures/events-life.jsonl"))
            .into_iter()
            .find(|e| e.action == "kill")
            .unwrap();
        assert_eq!(kill.signal, Some(15));
        assert_eq!(kill.at, 1794219974);
        assert!(kill.nano > kill.at * 1_000_000_000);
    }

    #[test]
    fn a_journal_survives_a_restart_and_resumes_after_its_last_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("docker/events.jsonl");
        let now = 1794219975;
        let mut journal = Journal::open(&path);
        for e in journalled(include_str!("../tests/fixtures/events-life.jsonl")) {
            assert!(journal.record(e, now));
        }
        let count = journal.entries().len();
        let last = journal.last_nano().unwrap();
        let reopened = Journal::open(&path);
        assert_eq!(reopened.entries(), journal.entries());
        assert_eq!(reopened.last_nano(), Some(last));

        let mut reopened = reopened;
        for e in journalled(include_str!("../tests/fixtures/events-life.jsonl")) {
            assert!(
                !reopened.record(e, now),
                "an event replayed by `since` is not journalled twice"
            );
        }
        assert_eq!(reopened.entries().len(), count);
    }

    #[test]
    fn a_journal_keeps_fourteen_days() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let now = 2_000_000_000;
        let mut journal = Journal::open(&path);
        journal.record(entry(now - KEEP_SECS - 10, "start"), now - KEEP_SECS - 10);
        journal.record(entry(now - 10, "die"), now);
        let kept: Vec<_> = Journal::open(&path)
            .entries()
            .iter()
            .map(|e| e.action.clone())
            .collect();
        assert_eq!(kept, vec!["die".to_string()]);
    }

    #[test]
    fn a_journal_keeps_under_two_megabytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let now = 2_000_000_000;
        let mut journal = Journal::open(&path);
        for n in 0..30_000 {
            journal.record(entry(now - 30_000 + n, "start"), now);
        }
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size <= MAX_BYTES, "{size}");
        let reopened = Journal::open(&path);
        assert_eq!(
            reopened.entries().last().unwrap().at,
            now - 1,
            "the newest are kept"
        );
        assert_eq!(reopened.entries().len(), journal.entries().len());
    }

    #[test]
    fn a_containers_events_are_its_own_newest_last() {
        let mut journal = Journal::memory();
        for e in journalled(include_str!("../tests/fixtures/events-life.jsonl")) {
            journal.record(e, 1794219975);
        }
        let loop_id = "kt-fx-loop";
        let mine = journal
            .entries()
            .iter()
            .find(|e| e.name == loop_id)
            .unwrap()
            .id
            .clone();
        let of = journal.of(&mine, 2);
        assert_eq!(of.len(), 2);
        assert!(of.iter().all(|e| e.id == mine));
        assert_eq!(of[1].action, "die");
        assert!(of[0].nano < of[1].nano);
    }
}
