use kampr_auth::Device;
use kampr_core::push::{Pulse, PulseRx};
use kampr_watch::{Event, Key, Waited, Watch};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// One watch per store however many sessions read it, alive while any of them holds it.
static WATCHES: LazyLock<Mutex<HashMap<PathBuf, Weak<Shared>>>> = LazyLock::new(Mutex::default);

struct Shared {
    pulse: Pulse,
    _stop: kampr_watch::Stop,
}

/// A write to the device store from any process: `kampr setup` revokes and `kampr mesh join`
/// enrols from their own, with no channel to this one but the file.
///
/// sqlite in WAL mode appends a commit to `-wal` and copies it into the main file at a checkpoint.
/// Both are watched, and so is their directory: a `-wal` that did not exist when the watch was
/// armed, or one unlinked by the last connection to close and made again by the next, is a new
/// inode that a watch on the old one never hears from, so either is re-armed when its directory
/// says it came or went. `None` when no watch could be made; the caller keeps a short timer.
pub struct StoreWatch {
    _watch: Arc<Shared>,
    pub changes: PulseRx,
}

pub fn store(db: &Path) -> Option<StoreWatch> {
    let mut watches = WATCHES.lock().unwrap();
    watches.retain(|_, w| w.strong_count() > 0);
    let watch = match watches.get(db).and_then(Weak::upgrade) {
        Some(watch) => watch,
        None => match spawn(db) {
            Ok(watch) => {
                watches.insert(db.to_path_buf(), Arc::downgrade(&watch));
                watch
            }
            Err(e) => {
                tracing::warn!(error = %e, db = %db.display(), "cannot watch the device store; re-reading it on a timer");
                return None;
            }
        },
    };
    Some(StoreWatch {
        changes: watch.pulse.subscribe(),
        _watch: watch,
    })
}

/// How often a watch that could not be put back on the store tries again, waking its listeners
/// each time: while it cannot see the files it is the short timer the callers would keep.
const REARM_RETRY: Duration = Duration::from_secs(2);

fn spawn(db: &Path) -> std::io::Result<Arc<Shared>> {
    let (mut watch, stop) = Watch::new()?;
    let files = Files::of(db);
    let mut armed = Some(files.arm(&mut watch)?);
    let pulse = Pulse::new();
    let bumps = pulse.clone();
    std::thread::Builder::new()
        .name("kampr-device-store".into())
        .spawn(move || {
            loop {
                let waited = watch.wait(armed.is_none().then_some(REARM_RETRY));
                let Some(current) = &armed else {
                    if matches!(waited, Ok(Waited::Stopped)) {
                        return;
                    }
                    armed = files.rearm(&mut watch, None);
                    bumps.bump();
                    continue;
                };
                let events = match waited {
                    Ok(Waited::Changed(events)) => events,
                    Ok(Waited::TimedOut) => continue,
                    Ok(Waited::Stopped) => return,
                    Err(e) => {
                        tracing::warn!(error = %e, "the device store's watch failed; only its timers are left");
                        return;
                    }
                };
                let (wake, rearm) = events
                    .iter()
                    .map(|e| files.judge(current, e))
                    .fold((false, false), |(w, r), (ew, er)| (w || ew, r || er));
                tracing::debug!(events = ?events, wake, rearm, "the device store's files moved");
                if rearm {
                    armed = files.rearm(&mut watch, armed.take());
                }
                if wake || armed.is_none() {
                    bumps.bump();
                }
            }
        })?;
    Ok(Arc::new(Shared { pulse, _stop: stop }))
}

/// Which file each of the store's names is: `(dev, ino)`, or `None` while it does not exist.
type Identities = [Option<(u64, u64)>; 2];

/// What the watch is on: the directory first, then each of the store's files that existed.
struct Armed {
    keys: Vec<Key>,
    ids: Identities,
}

/// The store's directory and the names in it that are the store.
struct Files {
    dir: PathBuf,
    names: [OsString; 2],
}

impl Files {
    fn of(db: &Path) -> Self {
        let mut wal = db.as_os_str().to_owned();
        wal.push("-wal");
        let name = |p: &Path| p.file_name().unwrap_or_default().to_os_string();
        Self {
            dir: db.parent().unwrap_or(Path::new(".")).to_path_buf(),
            names: [name(db), name(Path::new(&wal))],
        }
    }

    fn identities(&self) -> Identities {
        use std::os::unix::fs::MetadataExt;
        self.names.clone().map(|name| {
            std::fs::metadata(self.dir.join(name))
                .ok()
                .map(|m| (m.dev(), m.ino()))
        })
    }

    /// A file that is gone by the time it is watched is armed as absent, so its coming back is
    /// what the directory reports next rather than an error that ends the watch.
    fn arm(&self, watch: &mut Watch) -> std::io::Result<Armed> {
        use std::os::unix::fs::MetadataExt;
        let mut keys = vec![watch.add(&self.dir)?];
        let mut ids = [None, None];
        for (id, name) in ids.iter_mut().zip(&self.names) {
            let path = self.dir.join(name);
            match watch.add(&path) {
                Ok(key) => {
                    keys.push(key);
                    *id = std::fs::metadata(&path).ok().map(|m| (m.dev(), m.ino()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    for key in keys {
                        watch.remove(key);
                    }
                    return Err(e);
                }
            }
        }
        Ok(Armed { keys, ids })
    }

    fn rearm(&self, watch: &mut Watch, armed: Option<Armed>) -> Option<Armed> {
        for key in armed.into_iter().flat_map(|a| a.keys) {
            watch.remove(key);
        }
        self.arm(watch)
            .inspect_err(
                |e| tracing::warn!(error = %e, "re-arming the device store's watch; trying again shortly"),
            )
            .ok()
    }

    /// Whether an event is news about the store, and whether the watch must be put on the files
    /// that are there now. inotify names the entry a directory event is about; kqueue names
    /// nothing, so there the files are looked at and only one that came, went or was replaced
    /// counts, and every other file written beside the store wakes nobody.
    fn judge(&self, armed: &Armed, event: &Event) -> (bool, bool) {
        if event.key != armed.keys[0] || event.gone {
            return (true, event.gone);
        }
        match &event.name {
            Some(name) => {
                let ours = self.names.contains(name);
                (ours, ours)
            }
            None => {
                let moved = self.identities() != armed.ids;
                (moved, moved)
            }
        }
    }
}

/// When a device stops being active with nothing written anywhere: a Tier 0 token running out.
pub fn expiry(device: &Device) -> Option<Instant> {
    let at = UNIX_EPOCH + Duration::from_secs(u64::try_from(device.expires_at?).unwrap_or(0));
    Some(Instant::now() + at.duration_since(SystemTime::now()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::Files;
    use kampr_watch::{Event, Watch};

    /// kqueue's directory events name nothing, so this is the macOS path asked on any host: a file
    /// beside the store wakes nobody, and the store's files coming, going or being replaced wake
    /// and re-arm.
    #[test]
    fn a_nameless_directory_event_is_news_only_when_the_store_files_changed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kampr.db");
        let wal = dir.path().join("kampr.db-wal");
        std::fs::write(&db, b"").unwrap();
        let files = Files::of(&db);
        let (mut watch, _stop) = Watch::new().unwrap();
        let mut armed = files.arm(&mut watch).unwrap();
        let mut nameless = |why: &str, expected: bool| {
            let event = Event {
                key: armed.keys[0],
                name: None,
                gone: false,
            };
            assert_eq!(files.judge(&armed, &event), (expected, expected), "{why}");
            if expected {
                for key in armed.keys.drain(..) {
                    watch.remove(key);
                }
                armed = files.arm(&mut watch).unwrap();
            }
        };

        std::fs::write(dir.path().join("reads.json"), b"{}").unwrap();
        nameless("a file beside the store", false);
        std::fs::write(&wal, b"").unwrap();
        nameless("the wal appearing", true);
        std::fs::write(&wal, b"a commit").unwrap();
        nameless("a commit into the wal, which its own watch reports", false);
        // Kept rather than removed, so the new wal cannot be handed the old one's inode.
        std::fs::rename(&wal, dir.path().join("kampr.db-wal.old")).unwrap();
        nameless("the wal going", true);
        std::fs::write(&wal, b"").unwrap();
        nameless("the wal made again", true);
    }
}
