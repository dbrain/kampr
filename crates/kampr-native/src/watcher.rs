use kampr_watch::{Event, Key, Stop, Waited, Watch};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Change {
    Sessions,
    Session(String),
}

/// One blocking thread per provider, which ends as soon as the returned [`Stop`] is dropped.
pub(crate) fn spawn(root: PathBuf, mut on: impl FnMut(BTreeSet<Change>) + Send + 'static) -> Option<Stop> {
    let (mut sessions, stop) = match Sessions::new(&root) {
        Ok(watching) => watching,
        Err(e) => {
            tracing::warn!(root = %root.display(), error = %e, "cannot watch native sessions");
            return None;
        }
    };
    std::thread::Builder::new()
        .name("kampr-native-watch".into())
        .spawn(move || {
            while let Some(changes) = sessions.next() {
                on(changes);
            }
        })
        .expect("a watcher thread");
    Some(stop)
}

enum Watched {
    Root,
    Session(String),
    Hosts(String),
}

struct Sessions {
    root: PathBuf,
    watch: Watch,
    watched: HashMap<Key, Watched>,
}

impl Sessions {
    fn new(root: &Path) -> std::io::Result<(Self, Stop)> {
        let (mut watch, stop) = Watch::new()?;
        let key = watch.add(root)?;
        let mut sessions = Self {
            root: root.to_path_buf(),
            watch,
            watched: HashMap::from([(key, Watched::Root)]),
        };
        sessions.rescan();
        Ok((sessions, stop))
    }

    /// Every directory event is followed by a rescan, because a session directory is created
    /// before anything is written into it and the watch on it may be added after its first files.
    fn rescan(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let known = |want: fn(&Watched) -> Option<&String>| {
                self.watched.values().filter_map(want).any(|w| *w == id)
            };
            let (session, hosts) = (known(Watched::session), known(Watched::hosts));
            if !session && let Ok(key) = self.watch.add(&path) {
                self.watched.insert(key, Watched::Session(id.clone()));
            }
            let dir = path.join("hosts");
            if !hosts
                && dir.is_dir()
                && let Ok(key) = self.watch.add(&dir)
            {
                self.watched.insert(key, Watched::Hosts(id));
            }
        }
    }

    /// `None` once stopped.
    fn next(&mut self) -> Option<BTreeSet<Change>> {
        let events = match self.watch.wait(None) {
            Ok(Waited::Changed(events)) => events,
            Ok(Waited::TimedOut) => Vec::new(),
            Ok(Waited::Stopped) => return None,
            Err(e) => {
                tracing::warn!(root = %self.root.display(), error = %e, "the native session watch failed");
                return None;
            }
        };
        let mut changes = BTreeSet::new();
        for event in &events {
            self.change(event, &mut changes);
            if event.gone {
                self.watched.remove(&event.key);
            }
        }
        if changes.contains(&Change::Sessions) {
            self.rescan();
        }
        Some(changes)
    }

    /// kqueue names no entry, so a nameless event in a session may be any of its files.
    fn change(&self, event: &Event, changes: &mut BTreeSet<Change>) {
        let name = event.name.as_ref().map(|n| n.to_string_lossy());
        let named = |test: fn(&str) -> bool| name.as_deref().is_some_and(test);
        match self.watched.get(&event.key) {
            Some(Watched::Root) => {
                changes.insert(Change::Sessions);
            }
            Some(Watched::Session(id)) => {
                if !event.gone
                    && named(|n| n.starts_with("log.") || n.starts_with("screen.") || n.ends_with(".tmp"))
                {
                    return;
                }
                if event.gone || name.is_none() || named(|n| n == "hosts" || n == "spec.json") {
                    changes.insert(Change::Sessions);
                }
                changes.insert(Change::Session(id.clone()));
            }
            Some(Watched::Hosts(id)) if event.gone || !named(|n| n.ends_with(".tmp")) => {
                changes.insert(Change::Session(id.clone()));
            }
            _ => {}
        }
    }
}

impl Watched {
    fn session(&self) -> Option<&String> {
        match self {
            Self::Session(id) => Some(id),
            _ => None,
        }
    }

    fn hosts(&self) -> Option<&String> {
        match self {
            Self::Hosts(id) => Some(id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_new_session_and_what_is_written_into_it_are_both_seen() {
        let root = tempfile::tempdir().expect("root");
        let (tx, rx) = mpsc::channel();
        let _stop = spawn(root.path().to_path_buf(), move |changes| {
            for change in changes {
                let _ = tx.send(change);
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        let dir = root.path().join("s1");
        std::fs::create_dir(&dir).unwrap();
        let seen = |want: Change| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if rx.recv_timeout(Duration::from_millis(100)) == Ok(want.clone()) {
                    return true;
                }
            }
            false
        };
        assert!(seen(Change::Sessions));
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(dir.join("exit.json.tmp"), b"{}").unwrap();
        std::fs::rename(dir.join("exit.json.tmp"), dir.join("exit.json")).unwrap();
        assert!(seen(Change::Session("s1".into())));
        std::fs::write(dir.join("size.json"), b"{}").unwrap();
        assert!(seen(Change::Session("s1".into())));
        std::fs::create_dir(dir.join("hosts")).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(dir.join("hosts").join("1.json"), b"{}").unwrap();
        assert!(seen(Change::Session("s1".into())));
    }
}
