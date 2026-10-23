use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use std::collections::{BTreeSet, HashMap};
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STOP_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Change {
    Sessions,
    Session(String),
}

/// Blocking, one thread per provider. `on` returns `false` once nobody is listening, which is
/// checked at least every [`STOP_POLL`] so a dropped provider does not leave the thread behind.
pub(crate) fn spawn(root: PathBuf, mut on: impl FnMut(BTreeSet<Change>) -> bool + Send + 'static) {
    std::thread::Builder::new()
        .name("kampr-native-watch".into())
        .spawn(move || {
            let mut watcher = match Watcher::new(&root) {
                Ok(watcher) => watcher,
                Err(e) => {
                    tracing::warn!(root = %root.display(), error = %e, "cannot watch native sessions");
                    return;
                }
            };
            loop {
                let changes = watcher.next(STOP_POLL);
                if !on(changes) {
                    return;
                }
            }
        })
        .expect("a watcher thread");
}

enum Watched {
    Root,
    Session(String),
    Hosts(String),
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

struct Watcher {
    root: PathBuf,
    fd: OwnedFd,
    watched: HashMap<i32, Watched>,
}

impl Watcher {
    fn new(root: &Path) -> std::io::Result<Self> {
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
        let wd = inotify::add_watch(
            &fd,
            root,
            WatchFlags::CREATE | WatchFlags::DELETE | WatchFlags::MOVED_TO | WatchFlags::MOVED_FROM,
        )?;
        let mut watcher = Self {
            root: root.to_path_buf(),
            fd,
            watched: HashMap::from([(wd, Watched::Root)]),
        };
        watcher.rescan();
        Ok(watcher)
    }

    /// Every directory event is followed by a rescan, because a session directory is created
    /// before anything is written into it and the watch on it may be added after its first files.
    fn rescan(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        let known = |kind: fn(&Watched) -> Option<&String>, id: &str| {
            self.watched.values().filter_map(kind).any(|w| w == id)
        };
        let mut added = Vec::new();
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let files = WatchFlags::CREATE
                | WatchFlags::DELETE
                | WatchFlags::MOVED_TO
                | WatchFlags::CLOSE_WRITE
                | WatchFlags::DELETE_SELF;
            if !known(Watched::session, &id)
                && let Ok(wd) = inotify::add_watch(&self.fd, &path, files)
            {
                added.push((wd, Watched::Session(id.clone())));
            }
            let hosts = path.join("hosts");
            if !known(Watched::hosts, &id)
                && hosts.is_dir()
                && let Ok(wd) = inotify::add_watch(&self.fd, &hosts, files)
            {
                added.push((wd, Watched::Hosts(id)));
            }
        }
        self.watched.extend(added);
    }

    fn next(&mut self, timeout: Duration) -> BTreeSet<Change> {
        let mut changes = BTreeSet::new();
        let spec = Timespec::try_from(timeout).unwrap_or(Timespec {
            tv_sec: 0,
            tv_nsec: 500_000_000,
        });
        let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
        if !matches!(poll(&mut fds, Some(&spec)), Ok(n) if n > 0) {
            return changes;
        }
        let mut buf = [MaybeUninit::<u8>::uninit(); 8192];
        let mut reader = inotify::Reader::new(&self.fd, &mut buf);
        let mut gone = Vec::new();
        loop {
            let event = match reader.next() {
                Ok(event) => event,
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => break,
            };
            let name = event
                .file_name()
                .and_then(|n| n.to_str().ok())
                .unwrap_or_default()
                .to_string();
            if event.events().contains(ReadFlags::IGNORED) {
                gone.push(event.wd());
            }
            match self.watched.get(&event.wd()) {
                Some(Watched::Root) => {
                    changes.insert(Change::Sessions);
                }
                Some(Watched::Session(id)) => {
                    if name.starts_with("log.") || name.starts_with("screen.") || name.ends_with(".tmp") {
                        continue;
                    }
                    if name == "hosts"
                        || name == "spec.json"
                        || event.events().contains(ReadFlags::DELETE_SELF)
                    {
                        changes.insert(Change::Sessions);
                    }
                    changes.insert(Change::Session(id.clone()));
                }
                Some(Watched::Hosts(id)) if !name.ends_with(".tmp") => {
                    changes.insert(Change::Session(id.clone()));
                }
                _ => {}
            }
        }
        for wd in gone {
            self.watched.remove(&wd);
        }
        if changes.contains(&Change::Sessions) {
            self.rescan();
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_new_session_and_what_is_written_into_it_are_both_seen() {
        let root = tempfile::tempdir().expect("root");
        let (tx, rx) = mpsc::channel();
        spawn(root.path().to_path_buf(), move |changes| {
            for change in changes {
                let _ = tx.send(change);
            }
            true
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
