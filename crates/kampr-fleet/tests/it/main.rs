mod live;
mod provider;

use kampr_core::provider::{PaneInfo, Provider};
use kampr_fleet::{FleetProvider, Geometry, Job, State};
use kampr_native::{Launch, NativeConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PATIENCE: Duration = Duration::from_secs(10);

/// A provider over a root of the test's own, with real keepers launched detached and every one of
/// them killed on the way out: a run survives its provider on purpose, so nothing else would.
pub struct Fleet {
    pub provider: FleetProvider,
    root: Option<tempfile::TempDir>,
    path: Option<String>,
}

impl std::ops::Deref for Fleet {
    type Target = FleetProvider;
    fn deref(&self) -> &FleetProvider {
        &self.provider
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        if let Some(root) = &self.root {
            kill_keepers(root.path());
        }
    }
}

impl Default for Fleet {
    fn default() -> Self {
        Self::with_path(None)
    }
}

impl Fleet {
    pub fn with_path(path: Option<String>) -> Self {
        let root = tempfile::Builder::new()
            .prefix("kf")
            .tempdir_in("/tmp")
            .expect("a root");
        let provider = FleetProvider::new(Ok(config(root.path())), path.clone());
        Self {
            provider,
            root: Some(root),
            path,
        }
    }

    /// A new provider over the same root, as a node restart builds one, with every run left going.
    pub fn restart(mut self) -> Self {
        let root = self.root.take().expect("a root");
        let path = self.path.take();
        drop(self);
        let provider = FleetProvider::new(Ok(config(root.path())), path.clone());
        Self {
            provider,
            root: Some(root),
            path,
        }
    }

    pub async fn run(&self, job: &Job) -> String {
        self.provider
            .start("c", job, None, Geometry::default())
            .await
            .expect("a run")
    }

    pub async fn pane(&self, pane_id: &str) -> PaneInfo {
        self.list_panes()
            .await
            .expect("a list")
            .into_iter()
            .find(|p| p.pane_id == pane_id)
            .expect("the pane")
    }

    pub fn session(&self, pane_id: &str) -> kampr_keep::SessionDir {
        let root = self.root.as_ref().expect("a root").path();
        let id = pane_id.strip_prefix("fleet:").expect("a fleet pane");
        kampr_keep::list(root)
            .expect("the sessions")
            .into_iter()
            .find(|s| s.id().eq_ignore_ascii_case(id))
            .expect("the run's session")
    }

    pub async fn state(&self, pane_id: &str) -> State {
        self.pane(pane_id).await.fleet.expect("a fleet marker").state
    }

    pub async fn wait_for(&self, pane_id: &str, want: impl Fn(&State) -> bool) -> PaneInfo {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        let mut seen = Vec::new();
        while tokio::time::Instant::now() < deadline {
            let pane = self.pane(pane_id).await;
            let state = pane.fleet.as_ref().expect("a fleet marker").state.clone();
            if want(&state) {
                return pane;
            }
            if seen.last() != Some(&state) {
                seen.push(state);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{pane_id} never reached the state asked for; saw {seen:?}");
    }

    /// Every distinct state the run passed through for `within`, or until it finished.
    pub async fn states_for(&self, pane_id: &str, within: Duration) -> Vec<State> {
        let deadline = tokio::time::Instant::now() + within;
        let mut seen: Vec<State> = Vec::new();
        while tokio::time::Instant::now() < deadline {
            let state = self.state(pane_id).await;
            let finished = state.finished();
            if seen.last() != Some(&state) {
                seen.push(state);
            }
            if finished {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        seen
    }

    /// The transcript once it holds `needle`: the screen is rebuilt from the log by a reader of
    /// its own, which can be a moment behind the ladder that saw the run end.
    pub async fn transcript_with(&self, pane_id: &str, needle: &str) -> String {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let text = self
                .read_scrollback(pane_id)
                .await
                .expect("a read")
                .map(|s| s.text)
                .unwrap_or_default();
            if text.contains(needle) || tokio::time::Instant::now() >= deadline {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn config(root: &Path) -> NativeConfig {
    let mut config =
        NativeConfig::new(Some(root.to_path_buf()), Some(kampr_testkit::keeper())).expect("a config");
    config.launch = Some(Launch::Detached);
    config
}

fn kill_keepers(root: &Path) {
    for session in kampr_keep::list(root).unwrap_or_default() {
        if let Some(keeper) = session.keeper() {
            unsafe {
                libc::kill(-(keeper.child as i32), libc::SIGKILL);
                libc::kill(keeper.keeper as i32, libc::SIGKILL);
            }
        }
    }
}

pub fn sh(script: &str) -> Job {
    Job::Argv(vec!["sh".into(), "-c".into(), script.into()])
}

/// A line the way the operator would type it into their own terminal, run by their own shell with
/// neither `-l` nor `-i`. On this machine that is a `bash` whose `.bashrc` sources ble.sh.
pub fn typed(line: &str) -> Job {
    Job::Shell(line.into())
}

pub fn keeper_child(fleet: &Fleet, pane_id: &str) -> Option<u32> {
    let root: PathBuf = fleet.root.as_ref()?.path().to_path_buf();
    let id = pane_id.strip_prefix("fleet:")?.to_ascii_lowercase();
    kampr_keep::SessionDir::at(root.join(id))
        .keeper()
        .map(|k| k.child)
}
