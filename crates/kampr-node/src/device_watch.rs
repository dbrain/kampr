use kampr_auth::Device;
use kampr_core::push::{FileWatch, PulseRx, watch_files};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// One watch per store however many sessions read it, alive while any of them holds it.
static WATCHES: LazyLock<Mutex<HashMap<PathBuf, Weak<FileWatch>>>> = LazyLock::new(Mutex::default);

/// A write to the device store from any process: `kampr setup` revokes from its own, with no
/// channel to this one but the file.
///
/// sqlite in WAL mode appends a commit to `-wal` and copies it into the main file at a checkpoint,
/// and this node's pool holds a connection for its whole life, so `-wal` is never unlinked under
/// it (`Store::open`). `None` when no watch could be made; the caller keeps a short timer.
pub struct StoreWatch {
    _watch: Arc<FileWatch>,
    pub changes: PulseRx,
}

pub fn store(db: &Path) -> Option<StoreWatch> {
    let mut watches = WATCHES.lock().unwrap();
    watches.retain(|_, w| w.strong_count() > 0);
    let watch = match watches.get(db).and_then(Weak::upgrade) {
        Some(watch) => watch,
        None => {
            let mut wal = db.as_os_str().to_owned();
            wal.push("-wal");
            let paths: Vec<PathBuf> = [db.to_path_buf(), PathBuf::from(wal)]
                .into_iter()
                .filter(|p| p.exists())
                .collect();
            let watch = match watch_files(&paths) {
                Ok(watch) => Arc::new(watch),
                Err(e) => {
                    tracing::warn!(error = %e, db = %db.display(), "cannot watch the device store; re-reading it on a timer");
                    return None;
                }
            };
            watches.insert(db.to_path_buf(), Arc::downgrade(&watch));
            watch
        }
    };
    Some(StoreWatch {
        changes: watch.changes(),
        _watch: watch,
    })
}

/// When a device stops being active with nothing written anywhere: a Tier 0 token running out.
pub fn expiry(device: &Device) -> Option<Instant> {
    let at = UNIX_EPOCH + Duration::from_secs(u64::try_from(device.expires_at?).unwrap_or(0));
    Some(Instant::now() + at.duration_since(SystemTime::now()).unwrap_or_default())
}
