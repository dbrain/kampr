use crate::agent_view::{DeskAgents, View};
use crate::backoff::Backoff;
use crate::naming::Template;
use crate::procfs::{Foreground, Procfs};
use crate::provider::{AgentStatus, Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use crate::reporter::Reporter;
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use kampr_herdr::{
    Command, ForegroundProcess, Herdr, Observer, ProcessInfo, Snapshot, StreamEvent, Sub, rpc::Subscription,
};
use kampr_journal::{Harness, PaneProcess};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

/// Every event that can move a field the herd model carries.
///
/// **Nothing here fires when the desk client resizes** (probe #52): `layout.updated` covers
/// structural change only, and a native geometry change is detectable *only* by looking. The
/// sweep below is what covers that, and it is the reason this is not an events-only design.
///
/// What is deliberately absent is as load-bearing as what is present, because every subscribed
/// event costs a `session.snapshot`:
/// - `workspace.focused`, `tab.focused` and `pane.focused` — focus is not in the herd model, and
///   at the desk it moves constantly.
/// - `workspace.metadata_updated` — presentation tokens, which the model does not carry, and it
///   fires on TTL expiry rather than on anything a user did.
/// - `pane.scroll_changed` — per pane, one event per scroll; the only field it moves is
///   `scrollback_rows`, which the sweep carries.
/// - `pane.output_changed` — **not subscribable at all.** herdr emits it, but `events.subscribe`
///   refuses the name, and one bad name refuses the whole list (probe #54).
const TOPOLOGY_EVENTS: &[&str] = &[
    "layout.updated",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.exited",
    "pane.moved",
    "pane.agent_detected",
    "tab.created",
    "tab.closed",
    "tab.renamed",
    "tab.moved",
    "workspace.created",
    "workspace.updated",
    "workspace.closed",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
];

/// The one event the whole triage story rests on, and the one that cannot be subscribed to for
/// the session as a whole: herdr requires a `pane_id`, and a single entry without one rejects the
/// entire `events.subscribe` call (probe #54). So it is subscribed once per agent pane, and the
/// list is rebuilt whenever the agent-pane set moves.
const STATUS_EVENT: &str = "pane.agent_status_changed";

/// How many `pane.process_info` calls one sweep has in flight at a time. See the fan-out comment
/// in [`Inner::refresh_processes`] for the measurement behind it (#450).
const PROCESS_FANOUT: usize = 16;

/// Well past herdr's 1000-line read cap; over-asking clamps rather than failing.
const READ_CEILING: u64 = 4096;

#[derive(Debug, Clone)]
pub struct HerdrConfig {
    pub binary: String,
    pub backoff: Backoff,
    /// How long the herd may go un-re-derived while nobody is watching a pane.
    ///
    /// **Reconciliation, not the source of truth.** [`TOPOLOGY_EVENTS`] carries every structural
    /// change, and a socket that dies takes the subscription with it, so this sweep exists for the
    /// two things neither covers: a desk resize, which emits nothing at all (probe #52), and a
    /// herdr that is wedged with its socket still open. Both are worth a minute's staleness on a
    /// box nobody is looking at; neither is worth twenty snapshots a minute for ever.
    pub sweep: Duration,
    /// The same sweep while at least one pane is being watched.
    ///
    /// Somebody is looking, so a resize has to reach them now rather than eventually — this is the
    /// only thing that sees one, and it is what keeps the change-to-client latency of a resize
    /// where it was before the slow sweep existed.
    pub sweep_watched: Duration,
    /// How long an arriving event waits for the rest of its burst before one snapshot is taken for
    /// all of them. herdr replays the whole herd as `created` events the instant a subscription
    /// opens, so a burst is the normal case rather than the exception.
    pub settle: Duration,
    /// How often a live stream re-measures the pane's true PTY width. Nothing announces a
    /// PTY/rect divergence either (probe #68), so this is the only thing that notices one.
    pub width_poll: Duration,
    /// The floor between two `events.subscribe` calls.
    ///
    /// `pane.agent_status_changed` is per pane, so opening a workspace of ten agents changes the
    /// subscription set ten times. Without this each change is a fresh socket and a fresh
    /// subscribe, which is a burst aimed at herdr for no gain: the poll is still the source of
    /// truth, so collapsing a burst costs one interval of event latency on the new panes and
    /// nothing else.
    pub resubscribe_min: Duration,
    /// How often the harness processes are checked for having exited.
    ///
    /// **Far shorter than the sweep, because it is a different question.** The sweep asks herdr
    /// what the herd looks like and costs a socket round trip; this asks procfs whether a pid
    /// this node already holds is still that process, and costs a `stat` per agent pane. Nothing
    /// announces a harness exiting — and a pane whose agent was quit goes on advertising that
    /// agent's conversation until something notices — so it is the one thing worth looking for
    /// oftener than herdr is worth asking.
    pub liveness: Duration,
    /// The name this node writes back into herdr for every pane, or `None` to write none.
    ///
    /// **`None` is the default and the shipped state.** A title Kampr computes lands on the pane's
    /// border for whoever is sitting at that desk (probe #294), and marking somebody's screen
    /// because a phone is looking at it is the side effect ADR 0002 exists to refuse. An operator
    /// turns it on per node.
    pub report_names: Option<Template>,
    /// The shape this node imposes on herdr's **own** agents sidebar, or `None` to leave the
    /// desk's own order alone.
    ///
    /// **`None` is the default and the shipped state**, for the same reason `report_names` is:
    /// the operator at that desk did not ask a phone to sort their agents. It also depends on
    /// `report_names` — the sort is on a token Kampr reports, and with reporting off no such
    /// token exists — so the two are decided together in the node's config rather than here.
    pub desk_agents: Option<View>,
    /// Whether a pane's **whole command line** goes on the wire beside its process name.
    ///
    /// **Off.** `cmd` is what the naming complaint needed — six panes in one directory told apart —
    /// and `argv` is the part that carries `-phunter2` and `-H "Authorization: …"`. Every paired
    /// device receives the herd model, `readonly` included, at `hello` and on every patch, with no
    /// `watch` involved; and an alt-screen or cleared pane shows nothing on screen while `argv`
    /// names the job for its whole life. So this is not the screen a readonly device could already
    /// read, and it is not on unless an operator says so.
    pub send_argv: bool,
    /// Whether this socket going quiet is news about the machine, or one named session ending.
    ///
    /// **True for the node's own herdr, false for every session it discovered.** A named session is
    /// a whole separate server (#49) and a whole separate node in the herd, and an operator closes
    /// one the way they close a terminal — the discovery sweep drops it within its own interval and
    /// the herd says so. Raising the machine's alarm for that is a false one on a healthy host, and
    /// it is the line an operator reads first on the day something is actually wrong (#465).
    pub primary: bool,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            binary: "herdr".into(),
            backoff: Backoff::default(),
            sweep: Duration::from_secs(30),
            sweep_watched: Duration::from_secs(3),
            settle: Duration::from_millis(60),
            width_poll: Duration::from_secs(3),
            resubscribe_min: Duration::from_millis(500),
            liveness: Duration::from_millis(100),
            report_names: None,
            desk_agents: None,
            send_argv: false,
            primary: true,
        }
    }
}

/// How long herdr takes to answer, as a number an operator can read.
///
/// **The last reading is not that number.** herdr looks at a freshly accepted connection once and,
/// if the request is not whole at that instant, not again for ~100 ms — so *every* call is either
/// ~0.2 ms or ~100 ms, with nothing in between, and which one is a coin flip on the window between
/// this node's `connect(2)` and its finished write ([#445](#), narrowed to a 0.25 % stall rate by
/// [#450](#)). A single sample therefore showed operators a 100 ms herd on a few per cent of herd
/// rebuilds with nothing wrong at all.
///
/// The fast mode is the answer: 100 ms is a property of herdr's accept loop, not of the link. So
/// this keeps the best of a handful of recent readings, which is the service time — at eight
/// samples and #450's stall rate a reading that is all-slow is not a thing that happens, and a
/// herdr that is genuinely slow has no fast mode to be found.
#[derive(Debug, Default)]
struct Rtt {
    recent: [Option<f64>; Self::SAMPLES],
    next: usize,
}

impl Rtt {
    const SAMPLES: usize = 8;

    fn record(&mut self, ms: f64) {
        self.recent[self.next] = Some(ms);
        self.next = (self.next + 1) % Self::SAMPLES;
    }

    fn best(&self) -> Option<f64> {
        self.recent
            .iter()
            .flatten()
            .copied()
            .min_by(|a, b| a.total_cmp(b))
    }

    /// A herdr that went away and came back is a different server, and the readings taken of the
    /// one before it describe nothing.
    fn forget(&mut self) {
        *self = Self::default();
    }
}

/// Whether herdr is answering, and whether it ever has.
///
/// `ever` is the difference between "the herd is empty because everything closed" and "the herd
/// is empty because this node has never reached a herdr at all", and a client cannot say why
/// without being told which.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    pub online: bool,
    pub ever: bool,
    pub detail: Option<String>,
}

struct Inner {
    herdr: Herdr,
    config: HerdrConfig,
    snapshot: watch::Sender<Arc<Snapshot>>,
    revision: watch::Sender<u64>,
    health: watch::Sender<Health>,
    widths: Mutex<HashMap<String, Measured>>,
    /// The harness process behind each agent pane. Held here rather than derived per caller
    /// because finding one costs a socket round trip, and because a pane's *identity* has to be
    /// as stable as the pane while the process behind it lives.
    processes: Mutex<HashMap<String, Running>>,
    /// The foreground job in each pane, agent or shell. Held rather than derived because it comes
    /// from the same `pane.process_info` round trip the harness does, and because six panes in one
    /// directory are otherwise indistinguishable.
    commands: Mutex<HashMap<String, Command>>,
    /// Every foreground pid in each pane, herdr's and this machine's alike, newest read wins —
    /// each held **with the start time it had when it was read**.
    ///
    /// It is what a pid-keyed session marker is intersected with, and a pid the kernel has handed
    /// on to somebody else would hand back somebody else's conversation. Re-walking on the sweep
    /// shrinks that window without closing it: the set is read on the sweep and used when the
    /// herd is rebuilt, and a pid can be reaped and re-issued in between. The start time closes
    /// it — [`HerdrProvider::pane_processes`] looks each pid up again and drops any whose start
    /// no longer matches, so an entry that outlives its process yields *nothing* instead of a
    /// stranger.
    pids: Mutex<HashMap<String, Vec<PaneProcess>>>,
    /// This machine's own procfs. herdr answers what a pane is only where the job leaves the
    /// shell's process group, which on a machine that sources ble.sh is never (probe #297); this
    /// is how the node answers the rest, and it answers nothing where `/proc` is not readable.
    procfs: Procfs,
    reporter: Reporter,
    desk_agents: DeskAgents,
    /// Whether this herdr has ever answered `pane.process_info`, which is the difference between
    /// a pane nothing has looked into and a pane nothing *can* look into. See
    /// [`Inner::agent_harness`].
    probed: AtomicBool,
    /// Panes with a live stream. The sweep reads it to pick its cadence and waits on it, so the
    /// first watcher speeds the herd up without waiting out the slow interval it is parked in.
    watching: watch::Sender<usize>,
    /// Bumped whenever a pane's PTY was moved by an op of this node's own, so a running width
    /// probe re-measures at once rather than at the far end of its interval. Node-wide because
    /// the probe is per-stream and a resize is rare; the cost of a spurious wake is one read.
    resized: watch::Sender<u64>,
    /// What herdr's round trip costs, taken from the sweep's own `session.snapshot` rather than
    /// from a `ping` of its own — one per session per herd rebuild, 2/min quiet and 19/min with
    /// four panes busy ([#448](#)), for a number the node was already in a position to know.
    rtt: Mutex<Rtt>,
    /// Why no pane on this node can be streamed, once a spawn has proved it.
    ///
    /// **Node-scoped because the fault is.** `Observer::spawn` failing is the configured binary
    /// missing or not executable, which nothing about a pane can cause and nothing about a pane
    /// can fix — so one watched pane proving it is proof for all of them, and every entry in the
    /// herd says so rather than only the one that happened to be opened.
    stream_fault: Mutex<Option<String>>,
}

/// One watched pane, for as long as its stream lives.
struct Watching(Arc<Inner>);

impl Watching {
    fn new(inner: &Arc<Inner>) -> Self {
        inner.watching.send_modify(|n| *n += 1);
        Self(inner.clone())
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.watching.send_modify(|n| *n = n.saturating_sub(1));
    }
}

/// What one agent pane's processes were found to be, and the harness they were looked up for.
#[derive(Debug, Clone)]
struct Running {
    agent: String,
    harness: Harness,
}

impl Running {
    fn new(agent: &str, pid: Option<u32>) -> Self {
        Self {
            agent: agent.to_string(),
            harness: match pid.map(PaneProcess::look_up) {
                // Herdr looked into the pane and this node asked it a moment later, so the pid it
                // named can already be gone. Holding one is worse than holding nothing: with no
                // start time there is nothing a later look can contradict, so the harness never
                // dies — and a pane whose harness never dies searches its working directory with
                // no lower bound, which serves whoever wrote in it last.
                Some(process) if process.start.is_none() && kampr_journal::process::observable() => {
                    Harness::Absent
                }
                Some(process) => Harness::Running(process),
                None => Harness::Absent,
            },
        }
    }

    /// Whether this entry still describes what is in the pane: the same harness, and a pid that
    /// is still the same live process rather than one the kernel has handed on. An entry that
    /// found nothing is never held — the whole point of asking again is that a harness starting
    /// is what a pane is waiting for.
    fn is_still(&self, agent: &str) -> bool {
        matches!(&self.harness, Harness::Running(p) if p.start.is_some())
            && self.agent == agent
            && self.alive()
    }

    /// Whether the process this entry names is still running.
    ///
    /// Procfs answers it without a socket, which is what lets it be asked at every read rather
    /// than once a sweep. An entry with no start time is one procfs never answered for, and
    /// nothing has been disproved: refusing every conversation on a host with no procfs would be
    /// a worse answer than trusting the last look.
    fn alive(&self) -> bool {
        let Harness::Running(process) = &self.harness else {
            return true;
        };
        process.start.is_none() || PaneProcess::look_up(process.pid).start == process.start
    }

    /// What the pane is running *now*, as opposed to what it was running when herdr was asked.
    ///
    /// A harness that has exited is [`Harness::Absent`] and not [`Harness::Unknown`]: this node
    /// looked, in procfs, and there is no harness there. `Unknown` would license a search of the
    /// working directory, which serves whichever transcript in it was written last — somebody
    /// else's, at the exact moment an agent has been quit.
    fn harness(&self) -> Harness {
        match self.alive() {
            true => self.harness.clone(),
            false => Harness::Absent,
        }
    }
}

/// The pane's column count as last read, and whether Kampr is holding it there.
///
/// There is no inference left here. `pane.selection.read` bounds on the real grid width (#509), so
/// [`Herdr::pane_width`] returns the exact number in about two calls when this cache is warm — and
/// what used to live in this struct was four hundred lines resolving a wrap into `n` or `n + 1`
/// against a running floor and a decaying proof, because nothing in herdr 0.8.2 would say (#221).
///
/// `held` earns its place for one reason only: it says whether letting go is a change worth
/// rebuilding the herd for.
#[derive(Debug, Clone, Copy, Default)]
struct Measured {
    cols: Option<u16>,
    held: bool,
}

pub struct HerdrProvider {
    inner: Arc<Inner>,
    topology_task: tokio::task::JoinHandle<()>,
    liveness_task: tokio::task::JoinHandle<()>,
}

impl Drop for HerdrProvider {
    fn drop(&mut self) {
        self.topology_task.abort();
        self.liveness_task.abort();
    }
}

impl HerdrProvider {
    /// **Never fails and never blocks on herdr.** The connection is a supervised loop that
    /// retries for as long as the process lives, so a node binds its port and serves its own
    /// "herdr is not running" state instead of exiting into a restart loop.
    pub fn spawn(herdr: Herdr, config: HerdrConfig) -> Self {
        let (snap_tx, _) = watch::channel(Arc::new(Snapshot::empty()));
        let (rev_tx, _) = watch::channel(0);
        let (health_tx, _) = watch::channel(Health::default());
        let inner = Arc::new(Inner {
            herdr,
            config,
            snapshot: snap_tx,
            revision: rev_tx,
            health: health_tx,
            widths: Mutex::new(HashMap::new()),
            processes: Mutex::new(HashMap::new()),
            commands: Mutex::new(HashMap::new()),
            pids: Mutex::new(HashMap::new()),
            rtt: Mutex::new(Rtt::default()),
            procfs: Procfs::default(),
            reporter: Reporter::new(),
            desk_agents: DeskAgents::new(),
            probed: AtomicBool::new(false),
            watching: watch::channel(0).0,
            resized: watch::channel(0).0,
            stream_fault: Mutex::new(None),
        });
        let topology_task = tokio::spawn(topology(inner.clone()));
        let liveness_task = tokio::spawn(liveness(inner.clone()));
        Self {
            inner,
            topology_task,
            liveness_task,
        }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.snapshot.borrow().clone()
    }

    pub async fn refresh(&self) -> Result<Arc<Snapshot>> {
        self.inner.refresh().await
    }

    /// The harness process behind a pane, as of the last refresh.
    pub fn agent_harness(&self, pane_id: &str) -> Harness {
        self.inner.agent_harness(pane_id)
    }

    /// Every process the pane has in the foreground, looked up fresh.
    ///
    /// **The seam a session marker is resolved through.** A harness writes a file keyed on its own
    /// pid from the moment it opens — minutes before it writes a transcript, and whether or not
    /// herdr has scraped an agent out of the screen — so intersecting this set with that directory
    /// says which session a pane is having, exactly, and immediately. It is the whole set and not
    /// the one that matched a name because the name is the part that fails: under ble.sh herdr
    /// reports only `bash`, and a harness launched through a wrapper is reported as the wrapper.
    ///
    /// Looked up rather than cached: the pid set is as old as the last sweep, and whether each pid
    /// is still that process is a question procfs answers for free at the moment of asking.
    pub fn pane_processes(&self, pane_id: &str) -> Vec<PaneProcess> {
        self.inner
            .pids
            .lock()
            .unwrap()
            .get(pane_id)
            .map(|pids| {
                pids.iter()
                    .filter_map(|was| {
                        let now = PaneProcess::look_up(was.pid);
                        // The pid the kernel has handed on to somebody else, dropped rather than
                        // answered: a marker keyed on it would resolve, and what it would resolve
                        // to is another pane's conversation. Two `None`s pass, and deliberately —
                        // a host with no readable procfs learns nothing either way, and refusing
                        // every pipeline there would be a worse answer than the last look.
                        (now.start == was.start).then_some(now)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Puts the desk's own agent order back, for a caller that is shutting this node down.
    ///
    /// **Not `Drop`.** Clearing is a socket round trip and `Drop` cannot wait on one; and the
    /// clear is unscoped (it wipes whatever view is active, whoever set it), so it has to be a
    /// deliberate call from a path that knows this node set one.
    pub async fn restore_desk(&self) {
        self.inner.desk_agents.restore(&self.inner.herdr).await;
    }

    pub fn health(&self) -> Health {
        self.inner.health.borrow().clone()
    }

    /// What herdr's socket costs this node, in milliseconds, or `None` before the first answer.
    ///
    /// Read rather than measured: the sweep's `session.snapshot` is timed as it goes past. See
    /// [`Rtt`] for why it is the best of a handful of readings and not the latest one.
    pub fn rtt_ms(&self) -> Option<f64> {
        self.inner.rtt.lock().unwrap().best()
    }

    pub fn watch_health(&self) -> watch::Receiver<Health> {
        self.inner.health.subscribe()
    }

    /// Adopts the width a `pane.size` just put on a pane, and re-measures every running stream.
    ///
    /// **This resizes nothing.** It is what the node believes *after* an op the operator confirmed
    /// has already moved the PTY — a head start on the next read rather than a claim, now that
    /// [`Herdr::pane_width`] reads the real width in about two calls and agreed with a
    /// `control`-driven resize within 3-5 ms when it was measured (#509).
    ///
    /// That latency is why `commanded` is gone. It existed because the *inference* underneath it
    /// measured the rows in the pane, and the rows already there were laid out at the width before
    /// the resize — so on the operator's own hub a matched hold put a pane at 289 columns and the
    /// stream came back at **292**, the pre-claim width, and stayed there while those rows sat in
    /// the read window. A read that bounds on the live grid cannot say that.
    ///
    /// Callers must have established that the size actually took. On an attached pane the desk
    /// takes its geometry straight back (#19), and recording a width the PTY does not have is the
    /// plausible-looking success this project has paid for before (#233).
    ///
    /// `held` says only whether a controller of Kampr's is standing on this pane right now, which
    /// is what makes letting go a change worth rebuilding the herd for.
    pub fn resized(&self, pane_id: &str, cols: u16, held: bool) {
        let mut widths = self.inner.widths.lock().unwrap();
        let entry = widths.entry(pane_id.to_string()).or_default();
        entry.cols = Some(cols);
        entry.held = held;
        drop(widths);
        self.inner.resized.send_modify(|n| *n += 1);
    }

    /// The hold on `pane_id` has let go.
    ///
    /// Called from the one place every hold ends, whichever way it ended: let go, superseded,
    /// or run out of deadline. The width is left where it is — the pane keeps the size it was
    /// given until something moves it (#219), and the next sweep reads whatever is true then.
    pub fn released(&self, pane_id: &str) {
        let mut widths = self.inner.widths.lock().unwrap();
        let Some(entry) = widths.get_mut(pane_id) else {
            return;
        };
        if !std::mem::take(&mut entry.held) {
            return;
        }
        drop(widths);
        self.inner.resized.send_modify(|n| *n += 1);
    }

    /// The pane's own column count, and `None` until it has been read once.
    ///
    /// The one honest column count in the system. The layout rect is not one (#68) and no method
    /// on the socket API *reports* one even in 0.9 (#221) — what 0.9 added is a read that bounds
    /// on it (#509). A caller that needs the pane's own width, to put it back after holding it at
    /// somebody's viewport, gets nothing rather than the rect until that read has happened.
    pub fn measured_cols(&self, pane_id: &str) -> Option<u16> {
        self.inner.measured_cols(pane_id)
    }

    /// `None` until herdr has answered once — an unknown version rather than a fabricated one.
    pub fn herdr_version(&self) -> Option<String> {
        let version = self.inner.snapshot.borrow().version.clone();
        (!version.is_empty()).then_some(version)
    }
}

impl Inner {
    fn sweep(&self) -> Duration {
        match *self.watching.borrow() > 0 {
            true => self.config.sweep_watched,
            false => self.config.sweep,
        }
    }

    async fn refresh(&self) -> Result<Arc<Snapshot>> {
        let asked = Instant::now();
        let snapshot = match self.herdr.snapshot().await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.went_offline(&e);
                return Err(e).with_context(|| format!("herdr socket {}", self.herdr.socket().display()));
            }
        };
        // The round trip the node was making anyway. A failed call is not a reading — it is a
        // timeout or a dead socket, and `online` is what says so.
        self.rtt
            .lock()
            .unwrap()
            .record(asked.elapsed().as_secs_f64() * 1000.0);
        self.came_online();
        let moved = self.refresh_processes(&snapshot).await;
        let changed = moved || fingerprint(&self.snapshot.borrow()) != fingerprint(&snapshot);
        if changed {
            self.snapshot.send_replace(snapshot.clone());
            self.revision.send_modify(|r| *r += 1);
        }
        if let Some(template) = &self.config.report_names {
            let panes: Vec<PaneInfo> = snapshot
                .panes
                .iter()
                .map(|p| pane_info(self, &snapshot, p))
                .collect();
            self.reporter.sweep(&self.herdr, template, &panes).await;
        }
        self.desk_agents
            .sweep(&self.herdr, self.config.desk_agents.as_ref())
            .await;
        Ok(snapshot)
    }

    /// The log line an operator sees at the default level, once per outage rather than once per
    /// retry — a socket that has been down for a week must not be a week of identical warnings.
    fn went_offline(&self, error: &anyhow::Error) {
        self.rtt.lock().unwrap().forget();
        let detail = format!("{}: {error}", self.herdr.socket().display());
        let first = {
            let h = self.health.borrow();
            h.online || h.detail.is_none()
        };
        self.health.send_if_modified(|h| {
            let changed = h.online;
            h.online = false;
            h.detail = Some(detail);
            changed
        });
        if first && !self.config.primary {
            info!(
                socket = %self.herdr.socket().display(),
                "a herdr session stopped answering; it leaves the herd at the next discovery sweep"
            );
        } else if first {
            warn!(
                socket = %self.herdr.socket().display(),
                error = %error,
                "herdr is not reachable; will keep retrying on the poll loop. Is the Herdr server running?"
            );
        }
    }

    fn came_online(&self) {
        let recovered = !self.health.borrow().online;
        self.health.send_if_modified(|h| {
            let changed = !h.online || !h.ever;
            h.online = true;
            h.ever = true;
            h.detail = None;
            changed
        });
        if recovered {
            info!(socket = %self.herdr.socket().display(), "herdr is answering");
        }
    }

    /// Records that `herdr terminal session observe` will not start, and publishes it.
    ///
    /// The revision bump is the whole point: it is what rebuilds the herd, which is what carries
    /// the reason to every client. A `warn!` in a journal tells nobody — one node logged 163 of
    /// them in a day while its operator, on a phone, watched a blank grid and reported a
    /// rendering bug (probe #233). Which is also why the loud line fires on the edge rather than
    /// on every retry: the log is for what changed, the herd is for what is.
    fn cannot_stream(&self, pane_id: &str, error: &anyhow::Error) {
        self.stream_faulted(pane_id, cannot_run_herdr(error));
    }

    fn stream_faulted(&self, pane_id: &str, detail: String) {
        let mut fault = self.stream_fault.lock().unwrap();
        if fault.as_deref() == Some(detail.as_str()) {
            debug!(pane = %pane_id, "observe still will not start; retrying");
            return;
        }
        *fault = Some(detail.clone());
        drop(fault);
        warn!(pane = %pane_id, "{detail}");
        self.revision.send_modify(|r| *r += 1);
    }

    fn can_stream(&self) {
        if self.stream_fault.lock().unwrap().take().is_none() {
            return;
        }
        info!(binary = %self.config.binary, "herdr observe runs again; panes can paint");
        self.revision.send_modify(|r| *r += 1);
    }

    fn stream_fault(&self) -> Option<String> {
        self.stream_fault.lock().unwrap().clone()
    }

    /// Which process each agent pane is running, refreshed against herdr.
    ///
    /// **The pane record carries no pid** (herdr 0.8.2), so this is a socket round trip per agent
    /// pane — and the reason it is not one per sweep is that procfs answers the only question
    /// that matters for free: an entry whose pid is still the same live process is still this
    /// pane's harness, and a harness that was quit takes its `/proc` entry with it.
    ///
    /// Returns whether anything moved, because a pane whose agent was restarted looks identical
    /// in the snapshot and is a different conversation.
    async fn refresh_processes(&self, snapshot: &Snapshot) -> bool {
        // Which panes need their *harness* re-derived, which is not every pane and not every
        // sweep: a pid that is still the same live process is still this pane's harness, and
        // procfs answers that for free. The command below is a different question with a
        // different answer every time a job starts, so it is read on every pass.
        let mut wanted = HashMap::new();
        for pane in &snapshot.panes {
            let Some(agent) = pane.agent.as_deref() else {
                continue;
            };
            let held = self.processes.lock().unwrap().get(&pane.pane_id).cloned();
            match held {
                Some(running) if running.is_still(agent) => continue,
                _ => {
                    wanted.insert(pane.pane_id.clone(), agent.to_string());
                }
            }
        }

        // **Every pane, every pass.** The read is not on a cadence of its own, because the thing
        // it answers has no cadence: a job starting in a pane is what changes `cmd`, and the same
        // output that starts it is what wakes this sweep. Putting a timer in front of it drops
        // exactly the read the event was asking for — measured at four runs in ten where a pane
        // kept a name from its shell's own startup (`kampr · node`) for the whole of a fifteen
        // second window, at a 3 s gate and at 30 s alike (#451). What the fan-out below buys is the
        // right saving: N round trips concurrently rather than N in a row.
        let asking: Vec<String> = snapshot.panes.iter().map(|pane| pane.pane_id.clone()).collect();

        // **Fanned out, because N sequential round trips are N independent coin flips.** herdr
        // looks at a freshly accepted connection once and then not again for ~100 ms (#445), so
        // a herd of N panes took N chances of that stall one after another. Probe #450 measured
        // 64 concurrent calls at 11.0 ms p50 against 12.7 ms sequential with **zero** stalls in
        // either arm: herdr's accept path takes the fan-out. The bound is well inside what was
        // measured rather than at it — a herd is not bounded and neither is the number of
        // sessions this process serves, and 16 already collapses a twenty-pane sweep to two
        // rounds. Ordering does not matter: every answer is keyed by its own pane.
        let read: Vec<(String, Result<ProcessInfo>)> = futures_util::stream::iter(asking)
            .map(|pane_id| async move {
                let info = self.herdr.process_info(&pane_id).await;
                (pane_id, info)
            })
            .buffer_unordered(PROCESS_FANOUT)
            .collect()
            .await;

        let mut found = Vec::new();
        let mut commands = Vec::new();
        let mut pids = Vec::new();
        for (pane_id, info) in read {
            // An error is *not* an absent harness: one says nothing looked, the other says the
            // pane is empty, and the difference decides whether the working directory may be
            // searched at all. The same holds for the command — an unanswered pane keeps the
            // name it had rather than losing it to a socket that blinked.
            match info {
                Ok(info) => {
                    self.probed.store(true, Ordering::Relaxed);
                    // Walked every sweep and never held: a `children` file goes on naming a
                    // child that has exited, so a job named from a walk taken a minute ago is a
                    // pane described as running something it finished.
                    let walked = info
                        .shell_pid
                        .map_or_else(Foreground::default, |shell| self.procfs.below(shell));
                    // herdr stays the source of truth where it has one. It has none whenever the
                    // foreground process group is the shell's — a pane at its prompt, and every
                    // pane on a machine that sources ble.sh (probe #297) — and that is the only
                    // case this answers.
                    let command = info.command().or_else(|| as_info(&walked.jobs).command());
                    commands.push((pane_id.clone(), command));
                    // Stamped as they are read. The start time is what makes the set safe to use
                    // after the read that produced it, without ever handing back a pid the kernel
                    // has re-issued.
                    pids.push((
                        pane_id.clone(),
                        foreground_pids(&info, &walked)
                            .into_iter()
                            .map(PaneProcess::look_up)
                            .collect(),
                    ));
                    if let Some(agent) = wanted.get(&pane_id) {
                        let harness = info
                            .harness(agent)
                            .or_else(|| as_info(&walked.all).harness(agent));
                        found.push((pane_id, Some(Running::new(agent, harness))));
                    }
                }
                Err(e) => {
                    debug!(pane = %pane_id, error = %e, "could not read the pane's processes");
                    if wanted.contains_key(&pane_id) {
                        found.push((pane_id, None));
                    }
                }
            }
        }
        let commands_moved = self.record_commands(snapshot, commands);
        self.record_pids(snapshot, pids);

        let agents: HashMap<&str, Option<&str>> = snapshot
            .panes
            .iter()
            .map(|p| (p.pane_id.as_str(), p.agent.as_deref()))
            .collect();
        let mut processes = self.processes.lock().unwrap();
        let before = processes.len();
        // A pane that stopped being an agent pane stops having a harness process, and a pane that
        // closed stops existing. Both leave an entry that would outlive what it describes.
        processes.retain(|pane_id, _| agents.get(pane_id.as_str()).is_some_and(Option::is_some));
        let mut moved = processes.len() != before;
        for (pane_id, running) in found {
            // An error answered nothing, so what was known is kept rather than dropped. A
            // dropped entry reads as `Unknown` — the weakest claim there is, and the one that
            // lets the working directory be searched — and the exit it may be hiding is caught
            // by [`Running::alive`] anyway.
            let Some(running) = running else {
                continue;
            };
            let replaced = processes.insert(pane_id, running.clone());
            // A pane whose harness has not changed has not moved — but one going from no harness
            // to a harness has, and nothing else in the snapshot says so: a fresh agent in the
            // pane the last one was quit in is identical in every field but the process.
            moved |= replaced.is_none_or(|old| old.harness != running.harness);
        }
        moved || commands_moved
    }

    /// Returns whether any pane's command moved, because nothing else in herdr's snapshot says
    /// so: a pane that started a build is identical in every field the fingerprint hashes, and a
    /// herd that does not re-derive is a name that never changes.
    fn record_commands(&self, snapshot: &Snapshot, read: Vec<(String, Option<Command>)>) -> bool {
        let live: std::collections::HashSet<&str> =
            snapshot.panes.iter().map(|p| p.pane_id.as_str()).collect();
        let mut commands = self.commands.lock().unwrap();
        let before = commands.len();
        commands.retain(|pane_id, _| live.contains(pane_id.as_str()));
        let mut moved = commands.len() != before;
        for (pane_id, command) in read {
            match command {
                Some(command) => {
                    moved |= commands.insert(pane_id, command.clone()).as_ref() != Some(&command)
                }
                None => moved |= commands.remove(&pane_id).is_some(),
            }
        }
        moved
    }

    fn command(&self, pane_id: &str) -> Option<Command> {
        self.commands.lock().unwrap().get(pane_id).cloned()
    }

    /// Replaced wholesale rather than merged: a pane that was not read this pass has no pid set
    /// worth keeping, and a pane that closed has none at all.
    fn record_pids(&self, snapshot: &Snapshot, read: Vec<(String, Vec<PaneProcess>)>) {
        let live: std::collections::HashSet<&str> =
            snapshot.panes.iter().map(|p| p.pane_id.as_str()).collect();
        let mut pids = self.pids.lock().unwrap();
        pids.retain(|pane_id, _| live.contains(pane_id.as_str()));
        for (pane_id, found) in read {
            pids.insert(pane_id, found);
        }
    }

    /// Probe #68/#84: in a headless session the PTY does not follow the layout rect, so the rect
    /// is fiction and observing at it crops every row. [`Herdr::pane_width`] reads the real one
    /// (#509), seeded with what this pane last measured — or, on the very first pass, with the
    /// rect, which is the width or one column more (#230) and so is right half the time for free.
    ///
    /// A read that does not answer leaves the last width standing rather than falling back to the
    /// rect. herdr being briefly unreachable is not evidence that a pane got narrower, and sizing
    /// a stream from the rect on that basis is how a pane ends up cropped by an outage.
    async fn observe_cols(&self, pane_id: &str, rect: u16) -> u16 {
        let hint = self
            .widths
            .lock()
            .unwrap()
            .get(pane_id)
            .and_then(|m| m.cols)
            .unwrap_or(rect);
        match self.herdr.pane_width(pane_id, Some(hint)).await {
            Ok(cols) => {
                let mut widths = self.widths.lock().unwrap();
                widths.entry(pane_id.to_string()).or_default().cols = Some(cols);
                cols
            }
            Err(e) => {
                let known = self.widths.lock().unwrap().get(pane_id).and_then(|m| m.cols);
                match known {
                    // A width already read and a herdr that briefly would not answer. The pane did
                    // not get narrower because the socket blinked, so the last one stands and
                    // nothing is said.
                    Some(cols) => {
                        debug!(pane = %pane_id, error = %e, "could not re-read the pane width");
                        cols
                    }
                    // **Never a width at all, so this is not a blip.** The only column count on the
                    // socket is `pane.selection.read`'s bound (#509) and a herdr below 0.9.0 does
                    // not have it — so the rect is all that is left, and #68 established the rect
                    // is fiction. Painting at it is a pane that looks right and is not, which is
                    // the shape #233 cost this project a day of phone reports over. So it paints,
                    // because a wrong grid still beats a blank one, and it *says so* — through the
                    // herd, which is the one channel a client actually renders.
                    None => {
                        self.stream_faulted(pane_id, cannot_measure(&self.config.binary, &e));
                        rect
                    }
                }
            }
        }
    }

    /// The pane's geometry as herdr has it *now*, rather than as the last sweep left it.
    ///
    /// **The width beside it is measured live and the rows were not, and that asymmetry is what
    /// made one resize cost two restarts** (probe #506). `observe_cols` reads the pane over the
    /// socket, so a respawn always carried a current width; the rows came from the cached
    /// snapshot, which a sweep updates on its own cadence. So a resize that moved both was noticed
    /// by the width probe first, the stream came back at the new width with the *old* row count,
    /// and the sweep then changed the rows under a stream that had only just started — a second
    /// full repaint at a second size, seconds after the first. Both halves come from the same
    /// moment now, and the snapshot arriving later agrees with what is already running.
    ///
    /// One extra round trip per respawn, which is the same cost `observe_cols` beside it already
    /// pays, and a respawn is rare. A read that does not answer leaves the caller with the
    /// snapshot's own answer, which is what it had before.
    async fn fresh_geometry(&self, pane_id: &str) -> Option<(u16, u16)> {
        let snapshot = self.herdr.snapshot().await.ok()?;
        observe_geometry(&snapshot, pane_id)
    }

    /// What is running in a pane, as far as this node knows.
    ///
    /// **`Unknown` is a claim about the host, not about the pane.** It means nothing here can see
    /// into a pane at all, and it is what lets the working directory be searched — which serves
    /// whichever transcript in that directory was written last, somebody else's as often as this
    /// pane's. A pane with no record is not that: the record is dropped whenever herdr stops
    /// calling the pane an agent pane, and herdr decides that by scraping the screen, so it comes
    /// and goes under a harness that never moved. Once one pane's processes have been read, this
    /// herdr has proved it answers the question, and a missing record is a pane not looked into
    /// yet rather than a host that cannot look.
    fn agent_harness(&self, pane_id: &str) -> Harness {
        match self.processes.lock().unwrap().get(pane_id) {
            Some(running) => running.harness(),
            None if self.probed.load(Ordering::Relaxed) => Harness::Absent,
            None => Harness::Unknown,
        }
    }

    /// How many rows `pane.read visible` returns — the count that turns `bottom` into the pane's
    /// last *content* row (#519). `visible` is outside the wheel-scroll harvest gate, which wants
    /// `recent`/`recent_unwrapped` with `format: "text"` (#513), and it leaves `done` standing
    /// (#515).
    async fn last_content_row(&self, pane_id: &str) -> Option<u32> {
        let read = self.herdr.read_visible(pane_id).await.ok()?;
        u32::try_from(rows_in(&read.text)).ok()
    }

    /// The width this pane was last read at, or nothing. [`Self::observe_cols`] falls back to the
    /// rect because an observe stream has to be sized at *something*; neither the herd model nor
    /// a scrollback label does, and reporting the rect there is reporting a width the PTY never
    /// had.
    fn measured_cols(&self, pane_id: &str) -> Option<u16> {
        self.widths.lock().unwrap().get(pane_id).and_then(|m| m.cols)
    }
}

/// Cuts every line at `width` **display cells**, which is what turns `pane.selection.read`'s
/// unwrapped output back into physical rows (probe #521).
///
/// A line shorter than the width is one row. A line longer than it was laid out at exactly full
/// width and is cut there, repeatedly. A wide glyph that will not straddle the last column leaves
/// that column empty, which is why the accounting is `column_bound`'s and not `chars().count()`.
fn resplit(text: &str, width: u16) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut row = String::new();
        let mut cells = 0u16;
        for ch in line.chars() {
            let w = kampr_term::column_bound(&ch.to_string());
            if cells + w > width {
                out.push(std::mem::take(&mut row));
                cells = 0;
            }
            row.push(ch);
            cells += w;
        }
        out.push(row);
    }
    out
}

/// Where a `pane.read recent` window's first row sits in herdr's own history.
///
/// **`bottom - K + 1` is the tempting answer and it is wrong.** `recent` stops at the pane's last
/// *non-blank* row, which is not the bottom of the grid: after a `\033[2J` over a 300-row history a
/// 48-row viewport reported `bottom = 302` while the read ended at absolute 255, and the two
/// formulas differ by 208 (probe #519). They coincide only when the prompt happens to sit on the
/// bottom row, which is the common case and not a reliable one.
///
/// So it is `L - K + 1`, where `L` is that last content row — derived from a `visible` read, which
/// drops trailing blanks the same way (probe #519):
///
/// ```text
/// L = bottom - viewport_rows + rows(visible)
/// ```
fn first_row_of(text: &str, scroll: &kampr_herdr::model::Scroll, visible_rows: Option<u32>) -> Option<u32> {
    let visible_rows = visible_rows?;
    let read_rows = u32::try_from(rows_in(text)).ok()?;
    let depth = u32::try_from(scroll.max_offset_from_bottom.checked_add(scroll.viewport_rows)?).ok()?;
    let bottom = depth.checked_sub(1)?;
    let last_content = bottom
        .checked_sub(u32::try_from(scroll.viewport_rows).ok()?)?
        .checked_add(visible_rows)?;
    last_content.checked_sub(read_rows.checked_sub(1)?)
}

/// Rows in a read, counted the way [`crate::scrollback`] counts them so the two cannot disagree.
fn rows_in(text: &str) -> usize {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.len()
}

#[async_trait]
impl Provider for HerdrProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        let snapshot = self.inner.snapshot.borrow().clone();
        Ok(snapshot
            .panes
            .iter()
            .map(|p| pane_info(&self.inner, &snapshot, p))
            .collect())
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(supervise(self.inner.clone(), pane_id.to_string(), tx));
        Ok(PaneStream::supervised(rx, task))
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        match input {
            // `pane.send_text` takes a JSON string, so input must be UTF-8 representable.
            // Every escape sequence herdr's key grammar rejects is (probe #8/#9).
            Input::Bytes(bytes) => {
                let text = String::from_utf8(bytes)
                    .map_err(|_| anyhow::anyhow!("pane input must be valid UTF-8"))?;
                self.inner.herdr.send_text(pane_id, &text).await
            }
            Input::Keys(keys) => self.inner.herdr.send_keys(pane_id, &keys).await,
        }
    }

    /// **The one search that is not limited to the rows this node happens to hold.**
    ///
    /// `pane.copy_search` covers herdr's whole retained scrollback, where `pane.read recent` caps
    /// at 1000 rows with no offset (#51/#510) — so a client that could only search its own ring
    /// was searching a window, and the TUI's `/ ? n N` were bound to nothing at all because there
    /// was no honest answer to give them.
    ///
    /// Hit text is read back one row at a time, which is why the list is capped: `total` still
    /// counts every match, so an operator is told there are four hundred without this making four
    /// hundred round trips to say so.
    async fn find(
        &self,
        pane_id: &str,
        query: &str,
        backward: bool,
        from: Option<u32>,
    ) -> Result<Option<crate::provider::Found>> {
        /// Enough to fill any results list an operator reads, and a ceiling on the round trips one
        /// keystroke can cost.
        const LISTED: usize = 32;

        if query.is_empty() {
            return Ok(None);
        }
        let found = self.inner.herdr.find(pane_id, query, backward, from).await?;
        // Read rather than taken from the cache, hinted by it. A row is read by asking for the
        // columns either side of it (#510), so a width that is stale by one truncates the row and
        // a width that is too large fails the call outright — and a pane nobody has watched has no
        // cached width at all, which is exactly the pane somebody searches before opening.
        let cols = self
            .inner
            .herdr
            .pane_width(pane_id, self.inner.measured_cols(pane_id))
            .await
            .unwrap_or(0);
        let mut hits = Vec::with_capacity(found.hits.len().min(LISTED));
        for hit in found.hits.iter().take(LISTED) {
            // A row whose text will not come back is still a position worth reporting: the match
            // is real and the operator can scroll to it. Answering nothing because one read failed
            // would be the whole search lost to one row.
            let text = self
                .inner
                .herdr
                .row_text(pane_id, hit.row, cols)
                .await
                .unwrap_or_default();
            hits.push(crate::provider::Hit {
                from_bottom: hit.from_bottom,
                col: hit.col,
                end_from_bottom: hit.end_from_bottom,
                end_col: hit.end_col,
                text,
            });
        }
        Ok(Some(crate::provider::Found {
            hits,
            total: found.total,
            current: found.current,
        }))
    }

    /// **`pane.selection.read` does not answer one line per physical row, and the fix is a
    /// re-split at the grid width in *display cells*.**
    ///
    /// It unwraps: a range spanning a soft-wrapped logical line comes back joined, so a 403-row
    /// pane of 94-character lines returned 122 lines. After a reflow it also joins rows that never
    /// wrapped at all. Both are undone by cutting every returned line at `W` cells, which
    /// reconstructed the physical rows exactly — 0 mismatches against `pane.read recent` on
    /// reflow-poisoned rows, on genuine wraps, and on a screen of `中` (probe #521).
    ///
    /// **Cells, not characters.** Splitting a CJK screen by character count produced 61 mismatched
    /// rows out of 63; the same screen split by width produced none. `column_bound` is the same
    /// accounting the emulator uses.
    async fn read_rows(&self, pane_id: &str, from: u32, to: u32) -> Result<Option<Vec<String>>> {
        if to < from {
            return Ok(None);
        }
        let Ok(cols) = self
            .inner
            .herdr
            .pane_width(pane_id, self.inner.measured_cols(pane_id))
            .await
        else {
            return Ok(None);
        };
        if cols == 0 {
            return Ok(None);
        }
        let text = self.inner.herdr.rows_text(pane_id, from, to, cols).await?;
        Ok(Some(resplit(&text, cols)))
    }

    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        let snapshot = self.inner.snapshot.borrow().clone();
        let pane = snapshot.pane(pane_id).context("unknown pane")?;
        if !pane.scrollback_is_safe_to_read() {
            return Ok(None);
        }
        let scroll = pane.scroll.context("pane reported no scroll state")?;
        // The ring is re-wrapped at this width, so it has to be a width the PTY was actually
        // read at, and never the rect (probe #68).
        let cols = self.inner.measured_cols(pane_id);
        // Over-asking clamps to herdr's own cap (probe #51), so the request is deliberately far
        // past it: `truncated` then means "history exists above this", independent of how fresh
        // the cached snapshot's ring depth happens to be.
        let read = self
            .inner
            .herdr
            .read_scrollback(pane_id, READ_CEILING + scroll.viewport_rows)
            .await?;
        Ok(Some(RawScrollback {
            first_row: first_row_of(&read.text, &scroll, self.inner.last_content_row(pane_id).await),
            text: read.text,
            cols,
            viewport_rows: scroll.viewport_rows as u16,
            truncated: read.truncated,
        }))
    }

    /// herdr answers for the pane and this only reads the snapshot already in hand, so it costs
    /// nothing and never touches the socket. `is_agent` is the whole of the distinction: a pane
    /// herdr has labelled with a harness, reporting no ring, is one whose harness has the screen —
    /// and an unlabelled pane reporting no ring is a pager, a pane that has not scrolled yet, or a
    /// harness herdr lost the label for, none of which may cost the ring its rows.
    fn harness_owns_the_screen(&self, pane_id: &str) -> bool {
        self.inner
            .snapshot
            .borrow()
            .pane(pane_id)
            .is_some_and(|p| p.is_agent() && !p.scrollback_is_safe_to_read())
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.inner.revision.subscribe()
    }
}

/// What an operator reads on a phone when a node cannot run herdr.
///
/// It leads with the symptom they are looking at rather than with the call that failed — "could
/// not spawn observe" names a function nobody outside this file has heard of — and it names the
/// fix, because a journal line on the machine is exactly what they cannot reach. `{error:#}` and
/// not `{error}`: anything short of the whole chain drops the diagnosis and keeps the context.
/// The sub-floor fault: herdr answers, and cannot say how wide a pane is.
///
/// Distinct from [`cannot_run_herdr`] because the fix is different — that one is a binary that will
/// not start, this one is a binary that is too old — and reporting them the same way is what turns
/// a diagnosis into a shrug.
fn cannot_measure(binary: &str, error: &anyhow::Error) -> String {
    format!(
        "Panes on this node may be the wrong width: Kampr cannot measure one — {error:#}. \
         Reading a pane's column count needs herdr 0.9.0 or newer; below that there is nothing on \
         the socket that reports one, so the layout rect is all there is and it is not the pane's \
         real width. Update herdr ({binary}) — and restart its server, because the streaming half \
         is version-locked to it. `kampr doctor` on that machine says which version answered."
    )
}

fn cannot_run_herdr(error: &anyhow::Error) -> String {
    format!(
        "No pane on this node can show a screen: Kampr cannot run herdr — {error:#}. \
         Put herdr on the node's PATH, or set herdr.binary in its config to the full path; \
         kampr doctor on that machine says where it looked. \
         Kampr keeps retrying, and the panes come back on their own."
    )
}

/// The other half of [`cannot_run_herdr`]: the binary starts and then goes away before it has
/// sent anything, which is a different fault with a different fix and must not be reported as a
/// missing binary.
fn observe_produced_nothing(reason: &str) -> String {
    format!(
        "No pane on this node can show a screen: `herdr terminal session observe` starts and then \
         stops without sending a frame — {reason}. That is a herdr too old for the subcommand, or \
         one that cannot read this node's socket; kampr doctor on that machine says which herdr it \
         runs and which socket it dials. \
         Kampr keeps retrying, and the panes come back on their own."
    )
}

/// Reuses herdr's own rules — the shell filter, the pipeline join, the launcher-aware name match
/// — against processes this machine found rather than processes herdr reported. The two pid
/// fields are deliberately left unset: they carry the process-group check that gave up on this
/// pane in the first place, and there is nothing here for it to give up on.
fn as_info(processes: &[ForegroundProcess]) -> ProcessInfo {
    ProcessInfo {
        foreground_processes: processes.to_vec(),
        ..ProcessInfo::default()
    }
}

/// Every pid the pane has in the foreground, herdr's answer first and this machine's below it.
///
/// herdr's own list is kept even where it is only the shell: a marker directory keyed on a pid
/// simply will not contain a shell's, and dropping it would drop the case where herdr sees the
/// harness and the walk cannot reach it.
fn foreground_pids(info: &ProcessInfo, walked: &Foreground) -> Vec<u32> {
    let mut pids: Vec<u32> = info
        .foreground_processes
        .iter()
        .map(|p| p.pid)
        .chain(walked.all.iter().map(|p| p.pid))
        .collect();
    let mut seen = std::collections::HashSet::new();
    pids.retain(|pid| seen.insert(*pid));
    pids
}

/// The most of a command line that goes on the wire.
///
/// **This is a name in a sidebar, not a transcript.** Measured on the operator's own machine: a
/// pane running `claude --append-system-prompt <a five-kilobyte brief>` has a five-kilobyte
/// command line, and the default template renders `{argv|cmd}` straight into the pane's name —
/// which then goes to every device in a herd patch and back into herdr's own pane title. Until
/// the walk below, ble.sh hid every such line on that machine and nothing had ever met one.
const ARGV_CEILING: usize = 256;

/// One line, and a bounded one.
///
/// A command line carries whatever the shell was given, newlines included, and a name is rendered
/// on one row: an unfolded line does not truncate a title, it breaks the row it is drawn on.
fn as_a_name(line: &str) -> String {
    let mut folded = String::with_capacity(line.len().min(ARGV_CEILING + 4));
    let mut spaced = false;
    for c in line.chars() {
        match c.is_whitespace() || c.is_control() {
            true if spaced => continue,
            true => {
                folded.push(' ');
                spaced = true;
            }
            false => {
                folded.push(c);
                spaced = false;
            }
        }
        if folded.chars().count() > ARGV_CEILING {
            folded.pop();
            folded.push('…');
            return folded;
        }
    }
    folded.trim_end().to_string()
}

fn pane_info(inner: &Inner, snapshot: &Snapshot, pane: &kampr_herdr::Pane) -> PaneInfo {
    let (_, rect_rows) = snapshot.geometry(&pane.pane_id).unwrap_or((0, 0));
    // The rect is the desk's idea of the pane; the PTY is what the program inside it writes to,
    // and headless the two disagree. Rows herdr reports honestly, so they are taken from it and
    // the rect is only the fallback; a width nothing has read yet is reported as unknown.
    let cols = inner.measured_cols(&pane.pane_id);
    let rows = pane.scroll.map_or(rect_rows, |s| s.viewport_rows as u32);
    let command = inner.command(&pane.pane_id);
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|w| w.workspace_id == pane.workspace_id)
        .map(|w| w.label.clone().unwrap_or_else(|| w.number.to_string()));
    let tab = snapshot
        .tabs
        .iter()
        .find(|t| t.tab_id == pane.tab_id)
        .and_then(|t| t.label.clone());
    PaneInfo {
        pane_id: pane.pane_id.clone(),
        workspace_id: Some(pane.workspace_id.clone()),
        tab_id: Some(pane.tab_id.clone()),
        workspace,
        tab,
        cwd: pane.cwd.clone(),
        label: pane.label.clone(),
        agent: pane.agent.clone(),
        terminal_title: pane.title_text().map(str::to_string),
        agent_harness: inner.agent_harness(&pane.pane_id),
        agent_status: AgentStatus::from(pane.agent_status),
        cols,
        rows: rows as u16,
        scrollback_rows: if pane.scrollback_is_safe_to_read() {
            pane.scroll.map_or(0, |s| s.max_offset_from_bottom as u32)
        } else {
            0
        },
        cmd: command.as_ref().map(|c| as_a_name(&c.name)),
        argv: match inner.config.send_argv {
            true => command.map(|c| as_a_name(&c.line)),
            false => None,
        },
        fleet: None,
        detail: inner.stream_fault(),
    }
}

fn fingerprint(snapshot: &Snapshot) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in &snapshot.panes {
        p.pane_id.hash(&mut h);
        p.workspace_id.hash(&mut h);
        p.tab_id.hash(&mut h);
        p.cwd.hash(&mut h);
        p.label.hash(&mut h);
        p.agent.hash(&mut h);
        (p.agent_status as u8).hash(&mut h);
        // The state the harness wrote into its own title, for the harnesses that write one. It is
        // hashed as the *state* rather than as the title, because omp repaints a spinner frame
        // into its title every 80 ms and a herd rebuilt at 12 Hz is a herd pushed to every phone
        // at 12 Hz ([#486](#)) — while a pane that starts working must still wake one.
        kampr_journal::title_status(p.agent.as_deref(), p.title_text()).hash(&mut h);
        if let Some(s) = p.scroll {
            (s.max_offset_from_bottom, s.viewport_rows).hash(&mut h);
        }
    }
    for l in &snapshot.layouts {
        for lp in &l.panes {
            lp.pane_id.hash(&mut h);
            (lp.rect.width, lp.rect.height).hash(&mut h);
        }
    }
    h.finish()
}

/// The panes a `pane.agent_status_changed` subscription has to name, sorted so two snapshots
/// that carry the same agents compare equal whatever order herdr listed them in.
fn agent_panes(snapshot: &Snapshot) -> Vec<String> {
    let mut ids: Vec<String> = snapshot
        .panes
        .iter()
        .filter(|p| p.is_agent())
        .map(|p| p.pane_id.clone())
        .collect();
    ids.sort();
    ids
}

/// The whole subscription list: the session-wide topology kinds, plus one status entry per agent
/// pane. Building it in one place is what keeps probe #54 from coming back — every entry that
/// needs a `pane_id` gets one here or not at all.
pub fn subscriptions(agents: &[String]) -> Vec<Sub> {
    TOPOLOGY_EVENTS
        .iter()
        .map(|kind| Sub::kind(kind))
        .chain(agents.iter().map(|id| Sub::pane(STATUS_EVENT, id)))
        .collect()
}

/// Why the inner loop gave up its subscription.
enum Ended {
    /// The socket closed or a refresh failed — reconnect, with backoff.
    Broken,
    /// The agent-pane set moved, so the status entries are stale. Not a fault, so no backoff.
    PaneSetChanged,
}

/// Notices a harness exiting, which nothing else on this node does.
///
/// Herdr announces panes; the processes inside them it answers only when asked, and asking is a
/// socket round trip — which is why the sweep that does it is measured in seconds. Whether a pid
/// this node already holds is still alive is a `stat`, so it is asked far oftener and *published*
/// the moment the answer changes. Publishing is the half that matters: the herd carries
/// `has_conversation`, the conversation cache behind it is keyed on the process, and a pane whose
/// agent has been quit keeps advertising that agent's transcript until something rebuilds the
/// model.
async fn liveness(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(inner.config.liveness).await;
        let died = {
            let mut processes = inner.processes.lock().unwrap();
            let mut died = false;
            for running in processes.values_mut() {
                if !running.alive() {
                    running.harness = Harness::Absent;
                    died = true;
                }
            }
            died
        };
        if died {
            inner.revision.send_modify(|r| *r += 1);
        }
    }
}

async fn topology(inner: Arc<Inner>) {
    let mut backoff = inner.config.backoff.start();
    loop {
        if let Err(e) = inner.refresh().await {
            debug!(error = %e, "herdr snapshot failed; retrying");
            backoff.sleep().await;
            continue;
        }
        let agents = agent_panes(&inner.snapshot.borrow());
        match inner.herdr.subscribe(&subscriptions(&agents)).await {
            Ok(sub) => {
                backoff.reset();
                let subscribed_at = tokio::time::Instant::now();
                let ended = follow(&inner, sub, &agents).await;
                match ended {
                    Ended::PaneSetChanged => {
                        // Collapse a burst. A workspace opening ten agent panes moves the set ten
                        // times, and each move would otherwise be its own socket and its own
                        // subscribe. The poll is still the source of truth, so waiting out the
                        // window costs latency on the new panes and nothing else.
                        tokio::time::sleep_until(subscribed_at + inner.config.resubscribe_min).await;
                        continue;
                    }
                    Ended::Broken => debug!("herdr event subscription ended; reconnecting"),
                }
            }
            // A pane that closed between the snapshot and the subscribe answers `pane_not_found`
            // and takes the whole call with it (probe #107). That is a race, not a fault, and the
            // next pass re-derives the set from a fresh snapshot.
            Err(e) => debug!(error = %e, "events.subscribe failed"),
        }
        backoff.sleep().await;
    }
}

/// Drives one subscription until it breaks or the pane set it named goes stale.
///
/// **Events poke the sweep; they never replace it.** Every event ends in the same `refresh` the
/// sweep would have done, so a missed event costs one sweep and never correctness — which is what
/// makes the per-pane status subscription safe to lose and safe to rebuild, and what lets the
/// sweep itself be slow.
async fn follow(inner: &Arc<Inner>, mut sub: Subscription, agents: &[String]) -> Ended {
    // `read_line` is not cancel-safe, so the subscription gets its own task and the sweep races a
    // channel receive instead of the socket read itself.
    let (events, mut rx) = mpsc::channel::<()>(64);
    let reader = tokio::spawn(async move {
        while let Ok(Some(_)) = sub.next().await {
            if events.send(()).await.is_err() {
                return;
            }
        }
    });
    let mut watching = inner.watching.subscribe();
    let ended = loop {
        let live = tokio::select! {
            event = rx.recv() => {
                if event.is_none() {
                    break Ended::Broken;
                }
                // Wait out the burst, then take everything that arrived during it. A subscription
                // opening replays the whole herd as `created` events, and one snapshot answers all
                // of them.
                tokio::time::sleep(inner.config.settle).await;
                while rx.try_recv().is_ok() {}
                true
            }
            // The first watcher arriving must not have to wait out the slow sweep it interrupted.
            _ = watching.changed() => true,
            _ = tokio::time::sleep(inner.sweep()) => true,
        };
        if !live || inner.refresh().await.is_err() {
            break Ended::Broken;
        }
        if agent_panes(&inner.snapshot.borrow()) != agents {
            break Ended::PaneSetChanged;
        }
    };
    reader.abort();
    ended
}

enum Stop {
    Closed(String),
    /// A frame went missing. Probe #53: only the *first* frame of a stream is `full`, so every
    /// later one is a cursor-addressed partial repaint and a single lost or undecodable frame
    /// leaves the emulator disagreeing with the pane for the life of the stream, in cells herdr
    /// believes it has already delivered. Nothing downstream can repair that — republishing the
    /// node's own grid republishes the stale cells — so the stream is restarted, and the fresh
    /// one opens with a `full` frame, which is the only thing that can resynchronise it.
    FrameGap {
        expected: u64,
        got: u64,
    },
    GeometryChanged,
    /// The PTY turned out to be wider than the stream was sized for — probe #68, where the rect
    /// is fiction and the divergence only shows once content fills the real width.
    WidthChanged {
        was: u16,
        now: u16,
    },
    ConsumerGone,
}

/// The rect the layout claims, the width the stream actually runs at, and the PTY's own rows. The
/// first two differ whenever the PTY did not follow the rect, so change detection has to watch the
/// rect while the observer runs at the measured width.
#[derive(Debug, Clone, Copy)]
struct Geometry {
    rect: u16,
    cols: u16,
    rows: u16,
}

/// Owns restart. `terminal.closed` is routine — a pane that runs `clear`, a herdr restart, a desk
/// resize — so every one of them comes back as a `Reset`, never an error.
async fn supervise(inner: Arc<Inner>, pane_id: String, tx: mpsc::Sender<PaneEvent>) {
    let _watching = Watching::new(&inner);
    let mut snapshots = inner.snapshot.subscribe();
    let mut backoff = inner.config.backoff.start();
    loop {
        let Some((rect, rows)) = resolve_geometry(&pane_id, &mut snapshots).await else {
            return;
        };
        let (rect, rows) = inner.fresh_geometry(&pane_id).await.unwrap_or((rect, rows));
        let cols = inner.observe_cols(&pane_id, rect).await;
        let observer = Observer::spawn(
            &inner.config.binary,
            inner.herdr.socket(),
            &pane_id,
            cols as u32,
            rows as u32,
        );
        // **A spawn that returns `Ok` proves that `fork` and `exec` worked, and nothing else.**
        // Clearing the fault here called the binary half healthy before a single frame had
        // arrived, so a herdr that execs and exits — too old for the subcommand, unable to read
        // the socket — promised every client a grid, delivered nothing, and said nothing. That is
        // probe #233's symptom with the guard keyed on the wrong event; the fault clears at the
        // first frame instead, in `run_observer`.
        let mut observer = match observer {
            Ok(o) => o,
            Err(e) => {
                inner.cannot_stream(&pane_id, &e);
                backoff.sleep().await;
                continue;
            }
        };
        // **After the spawn, never before.** The geometry is a promise that rows are coming, and
        // a node that sends it before it has a stream has told every client to lay out a grid it
        // will then leave blank with a cursor blinking in it — for ever, silently, which is what
        // this cost on two machines for months.
        if tx.send(PaneEvent::Reset { cols, rows }).await.is_err() {
            return;
        }
        // The probe is its own task: two socket round-trips inside the stream loop would stall
        // the frames it is meant to be sizing.
        let (width_tx, width_rx) = watch::channel(cols);
        let prober = tokio::spawn(probe_width(inner.clone(), pane_id.clone(), rect, width_tx));
        let (stop, streamed) = run_observer(
            &inner,
            &mut observer,
            &tx,
            &mut snapshots,
            width_rx,
            &pane_id,
            Geometry { rect, cols, rows },
        )
        .await;
        prober.abort();
        observer.shutdown().await;
        // A gap must not reset the backoff, or a herdr whose sequence numbers are not what this
        // reads them to be would respawn `observe` as fast as it can start.
        if streamed && !matches!(stop, Stop::FrameGap { .. }) {
            backoff.reset();
        }
        match stop {
            Stop::ConsumerGone => return,
            Stop::GeometryChanged => debug!(pane = %pane_id, "native geometry changed; restarting"),
            Stop::WidthChanged { was, now } => {
                debug!(pane = %pane_id, was, now, "the pane is wider than the rect claimed; restarting")
            }
            Stop::FrameGap { expected, got } => {
                warn!(pane = %pane_id, expected, got, "a frame went missing; restarting the stream to resynchronise");
                backoff.sleep().await;
            }
            Stop::Closed(reason) => {
                debug!(pane = %pane_id, %reason, "observer closed; restarting");
                // A stream that started and closed without ever delivering a frame is the
                // binary half of the node being broken while the socket half answers perfectly
                // (probe #233) — the state where every client lays out a correctly-sized blank
                // grid and every surface reports the node healthy.
                if !streamed {
                    inner.stream_faulted(&pane_id, observe_produced_nothing(&reason));
                }
                backoff.sleep().await;
            }
        }
    }
}

async fn resolve_geometry(
    pane_id: &str,
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
) -> Option<(u16, u16)> {
    loop {
        if let Some(g) = observe_geometry(&snapshots.borrow_and_update(), pane_id) {
            return Some(g);
        }
        snapshots.changed().await.ok()?;
    }
}

/// The layout rect's width, and the height the PTY is actually running at.
///
/// **The rect's height is not the pane's.** A `down` split halves the rect and leaves the PTY at
/// the size it already had (probe #205), and `observe --rows` crops to the rows it is handed
/// rather than following the screen down (probe #206) — so sizing a stream from the rect serves
/// the top of the pane and nothing else, for as long as the stream lives. Herdr reports the PTY
/// honestly under `scroll.viewport_rows`; the rect is only the fallback, exactly as it is for the
/// rows the herd model carries.
///
/// The width stays the rect's here because it is a seed, not an answer: [`Inner::observe_cols`]
/// measures the real one and keys its cache on the rect it was asked about.
fn observe_geometry(snapshot: &Snapshot, pane_id: &str) -> Option<(u16, u16)> {
    let (rect, rect_rows) = snapshot.geometry(pane_id)?;
    let rows = snapshot
        .pane(pane_id)
        .and_then(|p| p.scroll)
        .map_or(rect_rows, |s| s.viewport_rows as u32);
    (rect > 0 && rows > 0).then_some((rect as u16, rows as u16))
}

/// Re-measures the pane's true width while its stream runs. Nothing announces a PTY/rect
/// divergence (probe #68), so this poll is the only thing that notices content growing past the
/// width the stream was started at.
async fn probe_width(inner: Arc<Inner>, pane_id: String, rect: u16, tx: watch::Sender<u16>) {
    let mut poll = tokio::time::interval(inner.config.width_poll);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.tick().await;
    let mut resized = inner.resized.subscribe();
    loop {
        // The interval is what notices content outgrowing the stream; the bump is `pane.size`
        // saying the PTY moved under it, which nothing else in herdr announces (probe #68) and
        // which the operator is about to type into.
        tokio::select! {
            _ = poll.tick() => {}
            changed = resized.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
        let cols = inner.observe_cols(&pane_id, rect).await;
        if tx.send(cols).is_err() {
            return;
        }
    }
}

async fn run_observer(
    inner: &Arc<Inner>,
    observer: &mut Observer,
    tx: &mpsc::Sender<PaneEvent>,
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
    mut width: watch::Receiver<u16>,
    pane_id: &str,
    geometry: Geometry,
) -> (Stop, bool) {
    let mut streamed = false;
    let mut last_seq: Option<u64> = None;
    loop {
        tokio::select! {
            event = observer.events.recv() => match event {
                Some(StreamEvent::Frame { seq, full, bytes, .. }) => {
                    if let Some(last) = last_seq
                        && seq != last + 1
                    {
                        return (Stop::FrameGap { expected: last + 1, got: seq }, streamed);
                    }
                    last_seq = Some(seq);
                    // The frame, not the spawn: this is the first thing that proves the binary
                    // half of the node reaches herdr at all.
                    if !streamed {
                        inner.can_stream();
                    }
                    streamed = true;
                    if tx.send(PaneEvent::Bytes { full, bytes }).await.is_err() {
                        return (Stop::ConsumerGone, streamed);
                    }
                }
                Some(StreamEvent::Closed { reason }) => return (Stop::Closed(reason), streamed),
                None => return (Stop::Closed("observe exited".into()), streamed),
            },
            changed = snapshots.changed() => {
                if changed.is_err() {
                    return (Stop::ConsumerGone, streamed);
                }
                let now = observe_geometry(&snapshots.borrow_and_update(), pane_id);
                if let Some(now) = now
                    && now != (geometry.rect, geometry.rows)
                {
                    return (Stop::GeometryChanged, streamed);
                }
            }
            changed = width.changed() => {
                if changed.is_err() {
                    return (Stop::ConsumerGone, streamed);
                }
                let now = *width.borrow_and_update();
                if now != geometry.cols {
                    return (Stop::WidthChanged { was: geometry.cols, now }, streamed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ARGV_CEILING, Measured, as_a_name};

    /// A command line can carry a whole brief — measured at five kilobytes on the operator's own
    /// machine — and it is rendered as a pane's name by the default template.
    #[test]
    fn a_command_line_long_enough_to_be_a_document_is_still_only_a_name() {
        let long = format!("claude --append-system-prompt {}", "brief ".repeat(2000));
        let name = as_a_name(&long);
        assert_eq!(name.chars().count(), ARGV_CEILING + 1);
        assert!(name.starts_with("claude --append-system-prompt brief"));
        assert!(name.ends_with('…'), "and it says it was cut: {name}");
    }

    /// The same line carries newlines and tabs, and a name is drawn on one row.
    #[test]
    fn a_command_line_with_a_newline_in_it_is_folded_into_one_row() {
        assert_eq!(
            as_a_name("claude -p 'do\n\n  this\ttoo'  "),
            "claude -p 'do this too'"
        );
    }

    #[test]
    fn a_line_short_enough_to_render_is_left_exactly_as_it_is() {
        assert_eq!(as_a_name("cargo test -p kampr-core"), "cargo test -p kampr-core");
    }
    use super::{STATUS_EVENT, TOPOLOGY_EVENTS, agent_panes, fingerprint, subscriptions};
    use kampr_herdr::Snapshot;

    fn snapshot(panes: &[(&str, Option<&str>)]) -> Snapshot {
        let json = serde_json::json!({
            "version": "0.8.2",
            "protocol": 20,
            "focused_pane_id": null,
            "panes": panes.iter().map(|(id, agent)| serde_json::json!({
                "pane_id": id,
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "cwd": null,
                "label": null,
                "agent": agent,
                "agent_status": "idle",
                "agent_session": null,
                "scroll": null,
            })).collect::<Vec<_>>(),
        });
        serde_json::from_value(json).expect("snapshot fixture")
    }

    /// Probe #54: one entry missing a required `pane_id` rejects the whole `events.subscribe`
    /// call, which is why the status event was unsubscribed at all. Every status entry has to
    /// carry one.
    #[test]
    fn every_status_entry_names_a_pane_and_no_topology_entry_does() {
        let subs = subscriptions(&["w1:p1".to_string(), "w3:p2".to_string()]);
        assert_eq!(subs.len(), TOPOLOGY_EVENTS.len() + 2);
        for sub in &subs {
            if sub.kind == STATUS_EVENT {
                assert!(sub.pane_id.is_some(), "{STATUS_EVENT} without a pane_id");
            } else {
                assert_eq!(sub.pane_id, None, "{} must not carry one", sub.kind);
            }
        }
        let named: Vec<&str> = subs
            .iter()
            .filter(|s| s.kind == STATUS_EVENT)
            .filter_map(|s| s.pane_id.as_deref())
            .collect();
        assert_eq!(named, ["w1:p1", "w3:p2"]);
    }

    fn titled(agent: &str, title: &str) -> Snapshot {
        let json = serde_json::json!({
            "version": "0.8.2",
            "protocol": 20,
            "focused_pane_id": null,
            "panes": [{
                "pane_id": "w1:p1",
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "cwd": null,
                "label": null,
                "agent": agent,
                "agent_status": "idle",
                "agent_session": null,
                "scroll": null,
                "terminal_title": title,
            }],
        });
        serde_json::from_value(json).expect("snapshot fixture")
    }

    /// A harness herdr has no rules for publishes its state in its own terminal title, and the
    /// herd is only rebuilt when this changes — so the state has to be in it. **The frame must
    /// not be**: omp repaints its title every 80 ms while it works, and 29 revisions in 6 s of
    /// polling is what that measured as through herdr.
    #[test]
    fn a_spinner_frame_is_not_a_change_and_starting_to_work_is() {
        let idle = fingerprint(&titled("omp", "π > project"));
        assert_eq!(
            fingerprint(&titled("omp", "π ⠹ project")),
            fingerprint(&titled("omp", "π ⠼ project")),
            "two frames of the same spinner are the same pane"
        );
        assert_ne!(idle, fingerprint(&titled("omp", "π ⠹ project")));
        assert_ne!(idle, fingerprint(&titled("omp", "π ! project")));
        // A harness nobody has measured a title for is untouched by any of it.
        assert_eq!(
            fingerprint(&titled("claude", "π > project")),
            fingerprint(&titled("claude", "π ⠹ project"))
        );
    }

    #[test]
    fn a_herd_with_no_agents_still_subscribes_to_topology() {
        assert_eq!(subscriptions(&[]).len(), TOPOLOGY_EVENTS.len());
    }

    /// The resubscribe trigger. Shell panes are not agents and must not move it, or every new
    /// terminal costs a fresh socket for a status event that can never fire.
    #[test]
    fn only_agent_panes_move_the_subscription_set() {
        let shell = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", None)]);
        assert_eq!(agent_panes(&shell), ["w1:p1"]);

        let another_shell = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", None), ("w1:p3", None)]);
        assert_eq!(
            agent_panes(&another_shell),
            agent_panes(&shell),
            "a new shell pane is not a resubscribe"
        );

        let promoted = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", Some("codex"))]);
        assert_ne!(
            agent_panes(&promoted),
            agent_panes(&shell),
            "a shell that became an agent is"
        );
    }

    /// Herdr does not promise an order, and an order change is not a pane-set change — comparing
    /// unsorted lists would resubscribe on every poll.
    #[test]
    fn the_pane_set_is_order_independent() {
        let a = snapshot(&[("w1:p2", Some("codex")), ("w1:p1", Some("claude"))]);
        let b = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", Some("codex"))]);
        assert_eq!(agent_panes(&a), agent_panes(&b));
    }

    /// **The hold's own bookkeeping, which is all that is left of `Measured`.**
    ///
    /// `held` exists for exactly one reason: letting go of a hold is a change the herd has to be
    /// rebuilt for, and letting go of nothing is not. Everything else that used to live in this
    /// struct — the floor, the decaying proof, the commanded override — was there to reconcile an
    /// inference with a resize, and there is no inference any more (#509).
    #[test]
    fn releasing_a_hold_is_a_change_and_releasing_nothing_is_not() {
        let mut m = Measured {
            cols: Some(119),
            held: true,
        };
        assert!(std::mem::take(&mut m.held), "a standing hold releases once");
        assert!(!std::mem::take(&mut m.held), "and not twice");
        assert_eq!(m.cols, Some(119), "and the width outlives the hold (#219)");
    }
}
