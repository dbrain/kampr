//! Fleet runs, presented to the node as panes.
//!
//! The whole point of implementing [`Provider`] rather than asking herdr to make panes is that
//! **nothing herdr knows about is involved**. A fleet run has no workspace, no tab and no place in
//! the operator's own layout, so it cannot clutter the desk on the machine it runs on — the
//! grouping is structural rather than a filter every client has to remember to apply. It is also
//! the only arrangement in which the state can be read at all (probes #331, #332, #334).

use crate::env::FleetPath;
use crate::exec::{Geometry, Killer, RunEvent, State, Supervisor, Writer};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use kampr_core::provider::AgentStatus;
use kampr_core::provider::{FleetPane, Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

/// Output replayed to a watcher that arrives late, or reconnects.
///
/// A fleet run's transcript is not scrollback and is not stitched (ADR 0004 governs the other
/// kind); it is the bytes this process has seen, capped so a run that prints for an hour cannot
/// grow without bound. A watcher joining past the cap sees a transcript missing its head, which is
/// the same thing a truncated ring means everywhere else here.
const REPLAY_CAP: usize = 256 * 1024;

const LIVE_DEPTH: usize = 512;

pub struct FleetProvider {
    runs: Mutex<HashMap<String, Arc<Run>>>,
    topology: watch::Sender<u64>,
    /// Resolved once, at construction, so `kampr doctor` and the first run agree and neither pays
    /// for a login shell in the middle of doing something else.
    path: Option<FleetPath>,
}

struct Run {
    pane_id: String,
    info: Mutex<FleetPane>,
    replay: Mutex<Vec<u8>>,
    live: broadcast::Sender<Vec<u8>>,
    writer: Writer,
    killer: Killer,
    geometry: Geometry,
}

impl Default for FleetProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FleetProvider {
    pub fn new() -> Self {
        Self::with_path(None)
    }

    pub fn with_path(configured: Option<String>) -> Self {
        Self {
            path: crate::env::fleet_path(configured),
            runs: Mutex::new(HashMap::new()),
            topology: watch::channel(0).0,
        }
    }

    /// Starts a command and returns the pane id it will be served as.
    ///
    /// `cohort` groups the panes one fan-out produced. The caller assigns it, because the run
    /// spans hosts and no single node can.
    pub fn start(
        self: &Arc<Self>,
        cohort: &str,
        argv: &[String],
        cwd: Option<&str>,
        geometry: Geometry,
    ) -> Result<String> {
        if argv.is_empty() {
            return Err(anyhow!("a fleet run needs a command"));
        }
        let supervisor =
            Supervisor::spawn(argv, cwd, geometry, self.path.as_ref().map(|p| p.value.as_str()))?;
        let pane_id = format!("fleet:{}", ulid::Ulid::generate());
        // **Blind until proven otherwise.** Whether the node can read this job is an observation
        // the supervisor makes over the run, not something knowable the instant after a fork — see
        // `RunEvent::Readable`. Starting here errs toward saying "I cannot see" rather than falsely
        // claiming to, and it is corrected within a poll or two for every run that is readable.
        let blind = true;
        let run = Arc::new(Run {
            pane_id: pane_id.clone(),
            info: Mutex::new(FleetPane {
                cohort: cohort.to_string(),
                command: argv.join(" "),
                state: State::Running,
                blind,
                started_unix: now_unix(),
            }),
            replay: Mutex::new(Vec::new()),
            live: broadcast::channel(LIVE_DEPTH).0,
            writer: supervisor.writer(),
            killer: supervisor.killer(),
            geometry,
        });

        self.runs
            .lock()
            .expect("runs")
            .insert(pane_id.clone(), Arc::clone(&run));
        self.bump();

        let (events, mut rx) = tokio::sync::mpsc::channel(256);
        let provider = Arc::clone(self);
        let driven = Arc::clone(&run);
        tokio::spawn(async move {
            let driver = tokio::spawn(supervisor.drive(events));
            while let Some(event) = rx.recv().await {
                match event {
                    RunEvent::Bytes(bytes) => {
                        {
                            let mut replay = driven.replay.lock().expect("replay");
                            replay.extend_from_slice(&bytes);
                            if replay.len() > REPLAY_CAP {
                                let drop_to = replay.len() - REPLAY_CAP;
                                replay.drain(..drop_to);
                            }
                        }
                        let _ = driven.live.send(bytes);
                    }
                    RunEvent::State(state) => {
                        driven.info.lock().expect("info").state = state;
                        provider.bump();
                    }
                    RunEvent::Readable => {
                        driven.info.lock().expect("info").blind = false;
                        provider.bump();
                    }
                }
            }
            let _ = driver.await;
            provider.bump();
        });

        Ok(pane_id)
    }

    /// Ends a run the way closing its terminal would. The pane stays listed with its final state,
    /// because a run somebody has not looked at yet is not finished being useful.
    pub fn stop(&self, pane_id: &str) -> Result<()> {
        let run = self.run(pane_id)?;
        run.killer.hangup();
        Ok(())
    }

    /// Removes a finished run. Refuses a live one — forgetting a `pacman` that is still running
    /// would leave it with nothing reading its pty and nobody able to answer it.
    pub fn forget(&self, pane_id: &str) -> Result<()> {
        let mut runs = self.runs.lock().expect("runs");
        let Some(run) = runs.get(pane_id) else {
            return Err(anyhow!("{pane_id} is not a fleet run"));
        };
        if !run.info.lock().expect("info").state.finished() {
            return Err(anyhow!("{pane_id} is still running"));
        }
        runs.remove(pane_id);
        drop(runs);
        self.bump();
        Ok(())
    }

    pub fn cohort(&self, cohort: &str) -> Vec<PaneInfo> {
        self.list()
            .into_iter()
            .filter(|p| p.fleet.as_ref().is_some_and(|f| f.cohort == cohort))
            .collect()
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

#[async_trait]
impl Provider for FleetProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        Ok(self.list())
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        let run = self.run(pane_id)?;
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let mut live = run.live.subscribe();
        let replay = run.replay.lock().expect("replay").clone();
        let geometry = run.geometry;

        let task = tokio::spawn(async move {
            if tx
                .send(PaneEvent::Reset {
                    cols: geometry.cols,
                    rows: geometry.rows,
                })
                .await
                .is_err()
            {
                return;
            }
            if tx
                .send(PaneEvent::Bytes {
                    full: true,
                    bytes: replay,
                })
                .await
                .is_err()
            {
                return;
            }
            loop {
                match live.recv().await {
                    Ok(bytes) => {
                        if tx.send(PaneEvent::Bytes { full: false, bytes }).await.is_err() {
                            return;
                        }
                    }
                    // A watcher that fell behind rejoins from the transcript rather than from a
                    // hole, the same rule the mesh fanout keeps one hop later.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = tx
                            .send(PaneEvent::Reset {
                                cols: geometry.cols,
                                rows: geometry.rows,
                            })
                            .await;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Ok(PaneStream::supervised(rx, task))
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        let run = self.run(pane_id)?;
        let bytes = match input {
            Input::Bytes(bytes) => bytes,
            Input::Keys(keys) => keys.iter().filter_map(|k| key_bytes(k)).flatten().collect(),
        };
        run.writer.write(&bytes)?;
        Ok(())
    }

    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        let run = self.run(pane_id)?;
        let bytes = run.replay.lock().expect("replay").clone();
        Ok(Some(RawScrollback {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            cols: Some(run.geometry.cols),
            viewport_rows: run.geometry.rows,
            truncated: bytes.len() >= REPLAY_CAP,
        }))
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.topology.subscribe()
    }

    /// Authoritative rather than a guess at the id's shape: a pane is this provider's if this
    /// provider is running it.
    fn owns(&self, pane_id: &str) -> bool {
        self.runs.lock().expect("runs").contains_key(pane_id)
    }
}

/// The handful of keys a fleet answer needs. A fleet pane is not a terminal somebody is living
/// in — it is a question with an answer — so this is deliberately not a keymap.
fn key_bytes(key: &str) -> Option<Vec<u8>> {
    Some(match key {
        "enter" | "return" => vec![b'\r'],
        "escape" | "esc" => vec![0x1b],
        "backspace" => vec![0x7f],
        "tab" => vec![b'\t'],
        "ctrl+c" => vec![0x03],
        "ctrl+d" => vec![0x04],
        "up" => b"\x1b[A".to_vec(),
        "down" => b"\x1b[B".to_vec(),
        "right" => b"\x1b[C".to_vec(),
        "left" => b"\x1b[D".to_vec(),
        _ => return None,
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
