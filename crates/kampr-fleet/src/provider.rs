//! Fleet runs, presented to the node as panes.
//!
//! A fleet run is a native session with a cohort and no host (docs/17 §2): its keeper owns the pty
//! and forked the command, so the run survives a node restart and its state can be read at all
//! (#331, #332, #334). Nothing herdr knows about is involved, so a run has no workspace, no tab and
//! no place in the operator's own layout, and cannot clutter the desk on the machine it runs on.
//!
//! The sessions live under a root of their own, so `kampr ls` and the native provider never list
//! them, and the pane ids stay `fleet:<ULID>` on the wire.

use crate::env::FleetPath;
use crate::job::Job;
use crate::ladder::{Ladder, POLL, Seen, State};
use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use kampr_core::provider::{
    AgentStatus, FleetPane, Found, Input, PaneInfo, PaneStream, Provider, RawScrollback,
};
use kampr_keep::SessionDir;
use kampr_native::{NativeConfig, NativeProvider, NewSession};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::watch;

const PREFIX: &str = "fleet:";

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub cols: u16,
    pub rows: u16,
}

impl Default for Geometry {
    /// The floor ADR 0012 sets for a deliberate resize, used here as the size of a pane nobody
    /// else is looking at. A fleet pty has no desk attached and no operator geometry to lose, so
    /// this is Kampr's to choose (rule 3).
    fn default() -> Self {
        Self { cols: 100, rows: 30 }
    }
}

pub struct FleetProvider {
    inner: Arc<Inner>,
}

struct Inner {
    native: std::result::Result<NativeProvider, String>,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    topology: watch::Sender<u64>,
    /// Resolved once, at construction, so `kampr doctor` and the first run agree and neither pays
    /// for a login shell in the middle of doing something else.
    path: Option<FleetPath>,
    /// The shell a [`Job::Shell`] is handed to, resolved beside the `PATH` and for the same reason.
    shell: String,
}

struct Run {
    pane_id: String,
    native: String,
    info: Mutex<FleetPane>,
    geometry: Geometry,
    forgotten: AtomicBool,
}

impl FleetProvider {
    /// `native.root` is this provider's own; every session under it with a cohort is a run, and is
    /// served again from wherever it got to.
    pub fn new(native: Result<NativeConfig>, configured_path: Option<String>) -> Self {
        let inner = Arc::new(Inner {
            native: native
                .and_then(NativeProvider::new)
                .map_err(|e| format!("fleet runs are unavailable on this node: {e:#}")),
            runs: Mutex::new(HashMap::new()),
            topology: watch::channel(0).0,
            path: crate::env::fleet_path(configured_path),
            shell: crate::env::login_shell(),
        });
        if let Ok(native) = &inner.native {
            for session in kampr_keep::list(native.root()).unwrap_or_default() {
                Inner::adopt(&inner, session);
            }
        }
        Self { inner }
    }

    /// Starts a command and returns the pane id it will be served as.
    ///
    /// `cohort` groups the panes one fan-out produced. The caller assigns it, because the run
    /// spans hosts and no single node can.
    pub async fn start(
        &self,
        cohort: &str,
        job: &Job,
        cwd: Option<&str>,
        geometry: Geometry,
    ) -> Result<String> {
        if job.is_empty() {
            bail!("a fleet run needs a command");
        }
        let native = self.inner.native()?;
        let argv = job.argv(&self.inner.shell);
        let path = self.inner.path.as_ref().map(|p| p.value.clone());
        if let Job::Argv(_) = job {
            found_on(&argv[0], path.as_deref())?;
        }
        let pane = native
            .create(NewSession {
                argv,
                cwd: cwd.map(str::to_string),
                label: Some(job.line()),
                cols: geometry.cols,
                rows: geometry.rows,
                env: path.map(|p| ("PATH".to_string(), p)).into_iter().collect(),
                cohort: Some(cohort.to_string()),
            })
            .await?;
        let session = native
            .session(&pane)
            .ok_or_else(|| anyhow!("{pane} vanished as it started"))?;
        let run = Inner::adopt(&self.inner, session).ok_or_else(|| anyhow!("{pane} has no cohort"))?;
        Ok(run.pane_id.clone())
    }

    /// Ends a run the way closing its terminal would. The pane stays listed with its final state,
    /// because a run somebody has not looked at yet is not finished being useful.
    pub fn stop(&self, pane_id: &str) -> Result<()> {
        let run = self.inner.run(pane_id)?;
        self.inner.native()?.close(&run.native)
    }

    /// Removes a finished run. Refuses a live one — forgetting a `pacman` that is still running
    /// would leave it with nobody able to answer it.
    pub fn forget(&self, pane_id: &str) -> Result<()> {
        let run = self.inner.run(pane_id)?;
        if !run.info.lock().expect("info").state.finished() {
            bail!("{pane_id} is still running");
        }
        self.inner.native()?.forget(&run.native)?;
        run.forgotten.store(true, Ordering::Relaxed);
        self.inner.runs.lock().expect("runs").remove(pane_id);
        self.inner.bump();
        Ok(())
    }

    pub fn cohort(&self, cohort: &str) -> Vec<PaneInfo> {
        self.inner
            .list()
            .into_iter()
            .filter(|p| p.fleet.as_ref().is_some_and(|f| f.cohort == cohort))
            .collect()
    }

    fn native_of(&self, pane_id: &str) -> Result<(&NativeProvider, String)> {
        let run = self.inner.run(pane_id)?;
        Ok((self.inner.native()?, run.native.clone()))
    }
}

impl Inner {
    fn native(&self) -> Result<&NativeProvider> {
        self.native.as_ref().map_err(|e| anyhow!("{e}"))
    }

    fn adopt(inner: &Arc<Self>, session: SessionDir) -> Option<Arc<Run>> {
        let spec = session.spec().ok()?;
        let cohort = spec.cohort.clone()?;
        let pane_id = format!("{PREFIX}{}", session.id().to_ascii_uppercase());
        if let Some(run) = inner.runs.lock().expect("runs").get(&pane_id) {
            return Some(run.clone());
        }
        let ended = session.exit().map(|exit| State::Exited {
            code: exit.code,
            signal: exit.signal,
        });
        let run = Arc::new(Run {
            pane_id: pane_id.clone(),
            native: format!("{}{}", kampr_native::PREFIX, session.id()),
            info: Mutex::new(FleetPane {
                cohort,
                command: spec.label.clone().unwrap_or_else(|| spec.argv.join(" ")),
                // **Blind until proven otherwise.** Whether the run can be read is an observation
                // the ladder makes over it, not something knowable the instant after a fork — see
                // `Seen::Readable` — and it is corrected within a poll or two for every run that is.
                blind: ended.is_none(),
                state: ended.clone().unwrap_or(State::Running),
                started_unix: spec.created_unix as i64,
            }),
            geometry: Geometry {
                cols: spec.cols,
                rows: spec.rows,
            },
            forgotten: AtomicBool::new(false),
        });
        inner.runs.lock().expect("runs").insert(pane_id, run.clone());
        inner.bump();
        if ended.is_none() {
            let weak = Arc::downgrade(inner);
            let driven = run.clone();
            std::thread::Builder::new()
                .name(format!("kampr-fleet-{}", session.id()))
                .spawn(move || drive(weak, driven, Ladder::new(session)))
                .ok()?;
        }
        Some(run)
    }

    fn list(&self) -> Vec<PaneInfo> {
        let mut panes: Vec<PaneInfo> = self
            .runs
            .lock()
            .expect("runs")
            .values()
            .map(|run| run.pane_info())
            .collect();
        panes.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
        panes
    }

    fn run(&self, pane_id: &str) -> Result<Arc<Run>> {
        self.runs
            .lock()
            .expect("runs")
            .get(pane_id)
            .cloned()
            .ok_or_else(|| anyhow!("{pane_id} is not a fleet run"))
    }

    fn bump(&self) {
        self.topology.send_modify(|n| *n += 1);
    }
}

/// Ends with the run, or with the provider: a node going away leaves the run to its keeper and
/// the next node's ladder picks it up from the log.
fn drive(inner: Weak<Inner>, run: Arc<Run>, mut ladder: Ladder) {
    loop {
        let seen = ladder.step();
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if run.forgotten.load(Ordering::Relaxed) {
            return;
        }
        let mut finished = false;
        for event in seen {
            let mut info = run.info.lock().expect("info");
            match event {
                Seen::Readable => info.blind = false,
                Seen::State(state) => {
                    finished |= state.finished();
                    info.state = state;
                }
            }
            drop(info);
            inner.bump();
        }
        if finished {
            return;
        }
        drop(inner);
        std::thread::sleep(POLL);
    }
}

impl Run {
    fn pane_info(&self) -> PaneInfo {
        let fleet = self.info.lock().expect("info").clone();
        PaneInfo {
            pane_id: self.pane_id.clone(),
            label: Some(fleet.command.clone()),
            // The existing blocked-first ordering in the sidebar is keyed on this, so a waiting
            // fleet host sorts above a working one without the board owning a second sort.
            agent_status: match &fleet.state {
                State::Waiting(_) => AgentStatus::Blocked,
                State::Running => AgentStatus::Working,
                State::Quiet { .. } => AgentStatus::Unknown,
                State::Exited { .. } => AgentStatus::Done,
            },
            cols: Some(self.geometry.cols),
            rows: self.geometry.rows,
            cmd: Some(
                fleet
                    .command
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            ),
            argv: Some(fleet.command.clone()),
            detail: fleet.blind.then(|| {
                "this run changes user, so the node cannot read whether it is waiting (probe \
                 #334) — it will show as quiet rather than as a question. Answers still reach it."
                    .to_string()
            }),
            fleet: Some(fleet),
            ..PaneInfo::default()
        }
    }
}

/// `execvp`'s bare "No such file or directory" is the same sentence from every host in the herd
/// and says nothing about the thing that is actually wrong — that this is not the `PATH` the
/// operator installs into (#392). A shell job never gets here: the shell says `command not found`.
fn found_on(program: &str, path: Option<&str>) -> Result<()> {
    if program.contains('/') {
        return Ok(());
    }
    let search = path.map(str::to_string).or_else(|| std::env::var("PATH").ok());
    let executable = |dir: &str| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(Path::new(dir).join(program))
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    match search.as_deref().unwrap_or_default().split(':').any(executable) {
        true => Ok(()),
        false => bail!(
            "`{program}` is not on this node's fleet PATH ({})",
            path.unwrap_or("unset")
        ),
    }
}

#[async_trait]
impl Provider for FleetProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        Ok(self.inner.list())
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        let (native, pane) = self.native_of(pane_id)?;
        native.watch_pane(&pane).await
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        let (native, pane) = self.native_of(pane_id)?;
        let bytes = match input {
            Input::Bytes(bytes) => bytes,
            Input::Keys(keys) => keys
                .iter()
                .filter_map(|k| kampr_core::keys::encode(k))
                .flatten()
                .collect(),
        };
        native.write_pane(&pane, Input::Bytes(bytes)).await
    }

    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        let (native, pane) = self.native_of(pane_id)?;
        native.read_scrollback(&pane).await
    }

    async fn read_rows(&self, pane_id: &str, from: u32, to: u32) -> Result<Option<Vec<String>>> {
        let (native, pane) = self.native_of(pane_id)?;
        native.read_rows(&pane, from, to).await
    }

    async fn find(
        &self,
        pane_id: &str,
        query: &str,
        backward: bool,
        from: Option<u32>,
    ) -> Result<Option<Found>> {
        let (native, pane) = self.native_of(pane_id)?;
        native.find(&pane, query, backward, from).await
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.inner.topology.subscribe()
    }

    /// Authoritative rather than a guess at the id's shape: a pane is this provider's if this
    /// provider is running it.
    fn owns(&self, pane_id: &str) -> bool {
        self.inner.runs.lock().expect("runs").contains_key(pane_id)
    }
}
