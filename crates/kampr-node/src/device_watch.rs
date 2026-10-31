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

fn spawn(db: &Path) -> std::io::Result<Arc<Shared>> {
    let (mut watch, stop) = Watch::new()?;
    let files = Files::of(db);
    let mut armed = files.arm(&mut watch)?;
    let pulse = Pulse::new();
    let bumps = pulse.clone();
    std::thread::Builder::new()
        .name("kampr-device-store".into())
        .spawn(move || {
            loop {
                let events = match watch.wait(None) {
                    Ok(Waited::Changed(events)) => events,
                    Ok(Waited::TimedOut) => continue,
                    Ok(Waited::Stopped) => return,
                    Err(e) => {
                        tracing::warn!(error = %e, "the device store's watch failed; only its timers are left");
                        return;
                    }
                };
                if !events.iter().any(|e| files.concerns(&armed, e)) {
                    continue;
                }
                if events.iter().any(|e| files.moved(&armed, e)) {
                    for key in armed.drain(..) {
                        watch.remove(key);
                    }
                    match files.arm(&mut watch) {
                        Ok(keys) => armed = keys,
                        Err(e) => {
                            tracing::warn!(error = %e, "re-arming the device store's watch; only its timers are left");
                            bumps.bump();
                            return;
                        }
                    }
                }
                bumps.bump();
            }
        })?;
    Ok(Arc::new(Shared { pulse, _stop: stop }))
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

    /// The directory first, so `armed[0]` is its key.
    fn arm(&self, watch: &mut Watch) -> std::io::Result<Vec<Key>> {
        let mut keys = vec![watch.add(&self.dir)?];
        for name in &self.names {
            let path = self.dir.join(name);
            if path.exists() {
                keys.push(watch.add(&path)?);
            }
        }
        Ok(keys)
    }

    /// kqueue names nothing in a directory, so a nameless event there may be the store's.
    fn ours(&self, event: &Event) -> bool {
        event.name.as_ref().is_none_or(|n| self.names.contains(n))
    }

    fn concerns(&self, armed: &[Key], event: &Event) -> bool {
        event.key != armed[0] || self.ours(event)
    }

    fn moved(&self, armed: &[Key], event: &Event) -> bool {
        event.gone || (event.key == armed[0] && self.ours(event))
    }
}

/// When a device stops being active with nothing written anywhere: a Tier 0 token running out.
pub fn expiry(device: &Device) -> Option<Instant> {
    let at = UNIX_EPOCH + Duration::from_secs(u64::try_from(device.expires_at?).unwrap_or(0));
    Some(Instant::now() + at.duration_since(SystemTime::now()).unwrap_or_default())
}
