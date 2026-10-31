use kampr_core::push::{FileWatch, PulseRx, watch_files};
use std::collections::BTreeSet;
use std::path::PathBuf;
use tracing::debug;

/// The transcripts the herd shows and the directories a missing one will appear in, watched as
/// one set and re-armed whenever a rebuild names a different one.
#[derive(Default)]
pub(crate) struct HerdFiles {
    armed: Option<(BTreeSet<PathBuf>, FileWatch, PulseRx)>,
}

impl HerdFiles {
    pub(crate) fn seen(&mut self) {
        if let Some((_, _, changes)) = &mut self.armed {
            changes.seen();
        }
    }

    /// Whether the watch was put on a different set, so a write between the read that named it
    /// and now may have been missed.
    pub(crate) fn arm(&mut self, paths: BTreeSet<PathBuf>) -> bool {
        let paths: BTreeSet<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
        if self.armed.as_ref().is_some_and(|(had, _, _)| *had == paths) {
            return false;
        }
        let list: Vec<PathBuf> = paths.iter().cloned().collect();
        self.armed = match watch_files(&list) {
            Ok(watch) => {
                let changes = watch.changes();
                Some((paths, watch, changes))
            }
            Err(e) => {
                debug!(error = %e, "cannot watch the herd's transcripts; the sweep stands in");
                None
            }
        };
        true
    }

    pub(crate) fn armed(&self) -> bool {
        self.armed.is_some()
    }

    /// Never resolves without a watch.
    pub(crate) async fn changed(&mut self) {
        match &mut self.armed {
            Some((_, _, changes)) => changes.changed().await,
            None => std::future::pending().await,
        }
    }
}
