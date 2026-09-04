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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

/// The most rows one logical line is checked against. Herdr has been seen joining nineteen, and
/// a cap keeps a pathological screen from making the search quadratic in the viewport.
const MAX_JOIN: usize = 64;

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

/// What this pane's width readings have established while the rect has held still.
///
/// `floor` is a running max rather than the latest sample: terminal content shrinks between
/// repaints, and one narrow reading must not restart the stream at a width that crops the next
/// wide line. A `proof` is a width a wrap actually laid out at, and it overrides the floor and the
/// rect both — but only while it is still being re-proved, because nothing tells a node that the
/// PTY moved (probe #211). All three are dropped when the rect moves.
#[derive(Debug, Clone, Copy, Default)]
struct Measured {
    rect: u16,
    floor: u16,
    proof: Option<Proof>,
    /// The width Kampr is holding this pane at right now, or `None` when it is holding none.
    ///
    /// **This is the one width that is not inferred, and it has to outrank the inference.** A held
    /// controller *is* the pane's geometry (#18) and herdr refuses a second one (#21), so while a
    /// hold stands the PTY's width is known rather than measured. The reads behind [`Reading`]
    /// measure the rows *in the pane*, and the moment Kampr resizes one, every row already there
    /// was laid out at the width before — so the first definite reading after a claim proves the
    /// old width and `record` overwrites the commanded one with it, unconditionally.
    ///
    /// Measured on the operator's own hub: a matched hold put the pane at 289 columns and the
    /// observe stream came back up at **292**, the pre-claim width, read out of rows the resize
    /// had not yet scrolled away. The client's emulator then wrapped at 292 over a 289-column PTY
    /// for as long as those rows stayed in the read window — every wrapped line in the wrong
    /// place, and the caret chasing a row it was never on.
    ///
    /// Readings go on being recorded underneath it, so the proof is current the instant the hold
    /// ends and nothing has to be re-measured to get the stream back.
    commanded: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Proof {
    cols: u16,
    unconfirmed: u8,
}

/// How many readings in a row may fail to re-prove a width before it stops beating the rect.
///
/// A proof describes the screen it was read from, and screens do not last. The rect cannot be the
/// cue that one is out of date — a controller that resized the PTY and detached leaves the rect
/// exactly where it was — so the only honest limit is how long the node is willing to go on
/// asserting a measurement nothing has repeated. At the 3 s width poll this is a minute.
const PROOF_LIFETIME: u8 = 20;

impl Measured {
    fn record(&mut self, reading: Reading) {
        match reading.wrapped {
            // A proof dates the evidence: a floor carried over from a wider screen is stale the
            // moment the pane proves it is narrower than that.
            Some(Wrapped::At(cols)) => {
                self.floor = reading.floor;
                self.proof = Some(Proof { cols, unconfirmed: 0 });
            }
            // The break says the grid is `cols` or `cols + 1` and nothing in the read says which,
            // so it is the weaker evidence and must not displace a standing proof of either — the
            // proof already chose between those two widths on a screen that could tell them
            // apart. It settles nothing about a proof it disagrees with, or one the rows in hand
            // outgrew, so those go back to resolving upward.
            Some(Wrapped::AtOrOneWider(cols)) => {
                self.floor = self.floor.max(reading.floor);
                let wider = cols.saturating_add(1);
                match self.proof.as_mut() {
                    Some(proof) if (cols..=wider).contains(&proof.cols) && proof.cols >= reading.floor => {
                        proof.unconfirmed = 0;
                    }
                    _ => {
                        self.proof = Some(Proof {
                            cols: wider,
                            unconfirmed: 0,
                        });
                    }
                }
            }
            None => {
                self.floor = self.floor.max(reading.floor);
                if let Some(proof) = self.proof.as_mut() {
                    proof.unconfirmed += 1;
                    if proof.unconfirmed > PROOF_LIFETIME {
                        // Letting go of a proof leaves its width behind as a floor, so it can only
                        // ever widen the stream — a measurement that turns out to be wrong must
                        // not be able to crop a pane on its way out.
                        self.floor = self.floor.max(proof.cols);
                        self.proof = None;
                    }
                }
            }
        }
    }

    fn cols(&self) -> u16 {
        // The commanded width first, because it is the only one here that was not inferred.
        if let Some(commanded) = self.commanded {
            return commanded;
        }
        self.proof
            .map_or_else(|| self.rect.max(self.floor), |proof| proof.cols)
    }
}

/// One pair of reads, resolved into what it proves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Reading {
    /// A lower bound: herdr trims each rendered row, so short content reads narrow.
    floor: u16,
    /// Where the rows in hand were laid out at, when they were laid out at all.
    wrapped: Option<Wrapped>,
}

/// The stride a join was laid out at, and how exactly the break dates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrapped {
    /// A break a narrow character made, which is the grid width itself: one more column would
    /// have held that character.
    At(u16),
    /// A break a double-width glyph made: the grid is that wide **or one column wider**, because
    /// the glyph will not straddle the last column. A screen of nothing but wide glyphs reads
    /// back identically on both of those grids (probe #220).
    AtOrOneWider(u16),
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
    /// has already moved the PTY, and it is the one width in the system that was not inferred: a
    /// wrap is the only evidence the socket API offers (#84), so a pane with nothing wrapped on it
    /// — every full-screen agent — went on being streamed at the width proved before the resize
    /// until something printed a long enough line. Herdr reflows what is on the screen, so a shell
    /// pane re-proved itself within a poll and an agent's pane never did.
    ///
    /// Callers must have established that the size actually took. On an attached pane the desk
    /// takes its geometry straight back (#19), and recording a width the PTY does not have is the
    /// plausible-looking success this project has paid for before (#233).
    /// `held` is whether a controller of Kampr's is standing on this pane at `cols` right now. It
    /// is the difference between a width that is *known* and one that merely *was* — a `once`
    /// resize is handed straight back by an attached desk (#19), where a hold is the geometry
    /// until it lets go (#18). Only the second may outrank a reading; see [`Measured::commanded`].
    pub fn resized(&self, pane_id: &str, cols: u16, held: bool) {
        let Some((rect, _)) = self.inner.snapshot.borrow().geometry(pane_id) else {
            return;
        };
        let rect = rect as u16;
        let mut widths = self.inner.widths.lock().unwrap();
        let entry = widths.entry(pane_id.to_string()).or_default();
        if entry.rect != rect {
            // A rect change ages the *inference*; it says nothing about a controller Kampr is
            // still holding, so the commanded width crosses it.
            *entry = Measured {
                rect,
                commanded: entry.commanded,
                ..Measured::default()
            };
        }
        entry.proof = Some(Proof { cols, unconfirmed: 0 });
        entry.commanded = held.then_some(cols);
        drop(widths);
        self.inner.resized.send_modify(|n| *n += 1);
    }

    /// The hold on `pane_id` has let go, so its width stops being commanded and the inference
    /// underneath it — kept warm the whole time — is the answer again.
    ///
    /// Called from the one place every hold ends, whichever way it ended: let go, superseded,
    /// or run out of deadline.
    pub fn released(&self, pane_id: &str) {
        let mut widths = self.inner.widths.lock().unwrap();
        let Some(entry) = widths.get_mut(pane_id) else {
            return;
        };
        if entry.commanded.take().is_none() {
            return;
        }
        drop(widths);
        self.inner.resized.send_modify(|n| *n += 1);
    }

    /// The width a wrap has actually proved for this pane, and `None` when nothing has.
    ///
    /// The one honest column count in the system. The layout rect is not one (#68) and no method
    /// on the socket API reports one anywhere (#221), so a caller that needs the pane's *own*
    /// width — to put it back after holding the pane at somebody's viewport, say — gets nothing
    /// rather than the rect. Same rule as [`Inner::proven_cols`]: a proof taken against a
    /// different rect is not this pane's width any more.
    pub fn measured_cols(&self, pane_id: &str) -> Option<u16> {
        let (rect, _) = self.inner.snapshot.borrow().geometry(pane_id)?;
        self.inner.proven_cols(pane_id, rect as u16)
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
    /// is fiction and observing at it crops every row. The reads render at the **true** PTY
    /// width, so they, not the rect, are what an observe stream is sized from.
    async fn observe_cols(&self, pane_id: &str, rect: u16) -> u16 {
        let reading = self.read_width(pane_id).await;
        let mut widths = self.widths.lock().unwrap();
        let entry = widths.entry(pane_id.to_string()).or_default();
        if entry.rect != rect {
            // A rect change ages the *inference*; it says nothing about a controller Kampr is
            // still holding, so the commanded width crosses it.
            *entry = Measured {
                rect,
                commanded: entry.commanded,
                ..Measured::default()
            };
        }
        // A read that never reached herdr is not evidence either way, so it must not age a proof.
        if let Some(reading) = reading {
            entry.record(reading);
        }
        entry.cols()
    }

    async fn read_width(&self, pane_id: &str) -> Option<Reading> {
        let rows = self
            .snapshot
            .borrow()
            .pane(pane_id)
            .and_then(|p| p.scroll)
            .map_or(0, |s| s.viewport_rows);
        if rows == 0 {
            return None;
        }
        match self.herdr.read_wrapped_and_logical(pane_id, rows).await {
            Ok((physical, logical)) => Some(reading(&physical.text, &logical.text)),
            Err(e) => {
                debug!(pane = %pane_id, error = %e, "could not measure the pane width");
                None
            }
        }
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

    /// The width a wrap has actually proved, or nothing. [`Self::observe_cols`] falls back to the
    /// rect because an observe stream has to be sized at *something*; neither the herd model nor
    /// a scrollback label does, and reporting the rect there is reporting a width the PTY never
    /// had.
    fn proven_cols(&self, pane_id: &str, rect: u16) -> Option<u16> {
        let widths = self.widths.lock().unwrap();
        widths
            .get(pane_id)
            .filter(|m| m.rect == rect)
            .and_then(|m| m.proof)
            .map(|proof| proof.cols)
    }
}

/// Resolves one pair of reads into a width.
///
/// `recent` returns *physical* rows, wrapped at the PTY's own width; `recent_unwrapped` returns
/// the logical lines they came from, with every row but the last of a join padded back out to the
/// grid width. So a logical line that spans more than one of the rows in hand gives the width
/// away exactly: it is the stride the rows were laid out at, and it can be checked by rebuilding
/// the line from those rows.
///
/// **The two reads are not the same window** (probe #211). Both ask for `viewport_rows`, but a
/// logical line is as many rows tall as it wrapped, so the logical read reaches further back into
/// history than the physical one — and the older lines it reaches back to have no rows here to
/// measure against. So they are walked from the bottom, where both reads are anchored, and the
/// walk stops at the first line that cannot be rebuilt from the rows left. Anything above that is
/// another screen, and this is not a measurement of it.
///
/// **Stopping is the whole of the safety, and stepping over the line instead would undo it.** A
/// line that cannot be rebuilt is a line whose height is unknown, so there is no count of rows to
/// step over with it: every join above it would be paired with rows belonging to some other line
/// and would prove a width the pane never had, which is the defect this walk exists to prevent.
/// What was worth loosening was not the stop but what counts as rebuilding it (probe #229 — a
/// wide glyph cutting one row of a join a column short).
///
/// Without a join there is no proof, only a floor: herdr trims each row's trailing blanks, so a
/// screen holding a short prompt reads narrow. A floor is never an over-estimate, which is what
/// makes combining it with the rect by `max` safe.
///
/// A break a wide glyph made is a column ambiguous and is kept as such, because a reading is not
/// the only evidence there is: [`Measured::record`] settles it against what this pane has already
/// proved rather than guessing here.
fn reading(physical: &str, logical: &str) -> Reading {
    let rows: Vec<&str> = physical.lines().collect();
    let lines: Vec<&str> = logical.lines().collect();
    let floor = rows.iter().map(|row| columns(row)).max().unwrap_or(0);
    let (mut row, mut line) = (rows.len(), lines.len());
    let (mut at, mut or_one_wider) = (None::<u16>, None::<u16>);
    while row > 0 && line > 0 {
        line -= 1;
        if lines[line] == rows[row - 1] {
            row -= 1;
            continue;
        }
        let Some(join) = joined(&rows[..row], lines[line]) else {
            break;
        };
        let proved = match join.could_be_one_wider {
            true => &mut or_one_wider,
            false => &mut at,
        };
        *proved = Some(proved.unwrap_or(0).max(join.cols));
        row -= join.rows;
    }
    Reading {
        floor,
        wrapped: at.map(Wrapped::At).or(or_one_wider.map(Wrapped::AtOrOneWider)),
    }
}

struct Join {
    cols: u16,
    rows: usize,
    /// The grid could be a column wider than `cols`: a double-width glyph will not straddle the
    /// last column, so a run of wide glyphs lays out identically on a grid of `cols` and on one
    /// of `cols + 1` and the read cannot separate them (probe #220).
    could_be_one_wider: bool,
}

/// The trailing rows of `rows` that rebuild `line`, and the grid they were laid out on.
///
/// Nothing here trusts herdr's idea of where a line began: the rows are only accepted when
/// laying them out on one grid and concatenating them reproduces the logical line character for
/// character.
///
/// The rows of a join do not all occupy the same number of columns — one a wide glyph cut short
/// is a column narrower than the rest — so the grid cannot be divided out of the total. What the
/// total does pin down is the pair it must be one of: every row occupies the grid width or one
/// less, so a run of `n` rows accounting for `p` columns is on a grid of `p / n` or `p / n + 1`.
/// Both are tried. A grid only one of them rebuilds is proved outright, by the rows that filled
/// it; a grid both rebuild is the ambiguity of probe #220.
fn joined(rows: &[&str], line: &str) -> Option<Join> {
    let total = columns(line);
    for span in 2..=rows.len().min(MAX_JOIN) {
        let run = &rows[rows.len() - span..];
        let (last, rest) = run.split_last()?;
        let Some(padded) = total.checked_sub(columns(last)) else {
            continue;
        };
        let stride = padded / rest.len() as u16;
        if stride == 0 {
            continue;
        }
        let mut fits = [stride, stride + 1]
            .into_iter()
            .filter(|grid| rebuilds(run, *grid, line));
        let Some(cols) = fits.next() else {
            continue;
        };
        return Some(Join {
            cols,
            rows: span,
            could_be_one_wider: fits.next().is_some(),
        });
    }
    None
}

/// Whether `run` laid out on a `grid`-column pane is exactly `line`.
///
/// Every row but the last carries the columns it occupied: the grid width, or one short of it
/// where the next row starts on a glyph too wide for the column left over — herdr does not pad
/// that column back (probe #220). The last row carries what it wrote and nothing more.
fn rebuilds(run: &[&str], grid: u16, line: &str) -> bool {
    let mut rebuilt = String::with_capacity(line.len());
    let Some((last, rest)) = run.split_last() else {
        return false;
    };
    for (i, row) in rest.iter().enumerate() {
        let width = columns(row);
        if width > grid || rebuilt.len() > line.len() {
            return false;
        }
        let occupied = match width + 1 == grid && starts_on_a_wide_glyph(run[i + 1]) {
            true => width,
            false => grid,
        };
        rebuilt.push_str(row);
        rebuilt.extend(std::iter::repeat_n(' ', (occupied - width) as usize));
    }
    rebuilt.push_str(last);
    rebuilt == line
}

/// The first *character*, not the first cluster: a base plus a variation selector is two columns
/// and reads as one here (probe #222). It costs a join that is never found rather than a width
/// that is wrong, because a grid nothing rebuilds proves nothing.
fn starts_on_a_wide_glyph(row: &str) -> bool {
    row.chars().next().and_then(|c| c.width()) == Some(2)
}

/// Columns, not characters. A row of double-width glyphs is half as many characters as it is
/// columns, and counting the characters called a 93-column pane 46 columns wide (probe #211).
fn columns(text: &str) -> u16 {
    text.width().min(u16::MAX as usize) as u16
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

    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        let snapshot = self.inner.snapshot.borrow().clone();
        let pane = snapshot.pane(pane_id).context("unknown pane")?;
        if !pane.scrollback_is_safe_to_read() {
            return Ok(None);
        }
        let scroll = pane.scroll.context("pane reported no scroll state")?;
        // The ring is re-wrapped at this width, so it has to be a width the PTY was actually
        // proved to have wrapped at, and never the rect (probe #68).
        let (rect, _) = snapshot.geometry(pane_id).context("pane has no layout rect")?;
        let cols = self.inner.proven_cols(pane_id, rect as u16);
        // Over-asking clamps to herdr's own cap (probe #51), so the request is deliberately far
        // past it: `truncated` then means "history exists above this", independent of how fresh
        // the cached snapshot's ring depth happens to be.
        let read = self
            .inner
            .herdr
            .read_scrollback(pane_id, READ_CEILING + scroll.viewport_rows)
            .await?;
        Ok(Some(RawScrollback {
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
    let (rect, rect_rows) = snapshot.geometry(&pane.pane_id).unwrap_or((0, 0));
    // The rect is the desk's idea of the pane; the PTY is what the program inside it writes to,
    // and headless the two disagree. Rows herdr reports honestly, so they are taken from it and
    // the rect is only the fallback; a width nothing has proved is reported as unknown.
    let cols = inner.proven_cols(&pane.pane_id, rect as u16);
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
    use super::{
        ARGV_CEILING, Measured, PROOF_LIFETIME, Proof, Reading, Wrapped, as_a_name, columns, reading,
    };

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
    use super::{STATUS_EVENT, TOPOLOGY_EVENTS, agent_panes, subscriptions};
    use kampr_herdr::Snapshot;
    use unicode_width::UnicodeWidthChar;

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

    const PROMPT: &str = "[14:33:33 dbrain@comingclean ~]$";

    fn reads(rows: &[String], lines: &[String]) -> (String, String) {
        (rows.join("\n"), lines.join("\n"))
    }

    fn dots(n: usize) -> String {
        format!("{n:>3}{}", ".".repeat(77))
    }

    fn wide(width: usize, total: usize) -> (String, String) {
        let logical = "#".repeat(total);
        let mut physical: Vec<String> = logical
            .as_bytes()
            .chunks(width)
            .map(|c| String::from_utf8(c.to_vec()).unwrap())
            .collect();
        physical.insert(0, "$ printf".into());
        (physical.join("\n"), format!("$ printf\n{logical}"))
    }

    /// `total` double-width glyphs laid out on a `cols`-column grid, as herdr reads them back.
    fn cjk(cols: usize, total: usize) -> (String, String) {
        let per_row = cols / 2;
        let mut physical: Vec<String> = (0..total)
            .step_by(per_row)
            .map(|i| "日".repeat(per_row.min(total - i)))
            .collect();
        physical.insert(0, "$ printf".into());
        (physical.join("\n"), format!("$ printf\n{}", "日".repeat(total)))
    }

    #[test]
    fn a_wrapped_line_proves_the_width_the_rect_is_lying_about() {
        // Measured live: rect 47 after a split, PTY still 93 (probes #68, #84).
        let (physical, logical) = wide(93, 400);
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 93,
                wrapped: Some(Wrapped::At(93))
            }
        );
        let m = Measured {
            rect: 47,
            floor: 93,
            commanded: None,
            proof: Some(Proof {
                cols: 93,
                unconfirmed: 0,
            }),
        };
        assert_eq!(m.cols(), 93, "the proof beats the rect in both directions");
    }

    #[test]
    fn short_content_proves_nothing_and_falls_back_to_the_rect() {
        let screen = "$ stty size\n40 93";
        assert_eq!(
            reading(screen, screen),
            Reading {
                floor: 11,
                wrapped: None
            },
            "nothing wrapped, so the widest row is only a floor"
        );
        let m = Measured {
            rect: 94,
            floor: 11,
            commanded: None,
            proof: None,
        };
        assert_eq!(m.cols(), 94, "and a floor never narrows the stream");
        assert_eq!(reading("", ""), Reading::default());
    }

    /// Probe #69: even unsplit the rect is one column wider than the PTY, because the rect is
    /// the pane's outer box and the column it keeps back is the scrollbar's (#230). The same
    /// proof fixes it — `observe` padded that column rather than cropping, but the grid was
    /// still a column wider than the pane.
    #[test]
    fn a_proof_below_the_rect_is_still_the_truth() {
        let (physical, logical) = wide(93, 400);
        let mut m = Measured {
            rect: 94,
            ..Measured::default()
        };
        m.record(reading(&physical, &logical));
        assert_eq!(m.cols(), 93);
    }

    #[test]
    fn a_floor_wider_than_the_rect_still_widens_the_stream() {
        // No wrap witness yet, but content already exceeds the rect: never crop what has been
        // seen, and let the next poll prove the rest.
        let m = Measured {
            rect: 47,
            floor: 80,
            commanded: None,
            proof: None,
        };
        assert_eq!(m.cols(), 80);
    }

    /// Probe #211, the defect: `recent` and `recent_unwrapped` are two reads of `viewport_rows`
    /// **lines**, and a logical line is as many rows tall as it wrapped — so the logical read
    /// reaches further back than the physical one, and the line it reaches back to has no row in
    /// the physical read to prove a width against. Measured live: a 372-column line above 39 rows
    /// of 80, and the old rule called 80 the PTY's width while `stty` said 93.
    #[test]
    fn a_logical_line_no_row_in_the_read_accounts_for_proves_nothing() {
        let rows: Vec<String> = (1..=39)
            .map(dots)
            .chain(std::iter::once(PROMPT.to_string()))
            .collect();
        let lines: Vec<String> = std::iter::once("#".repeat(372))
            .chain((4..=39).map(dots))
            .chain(std::iter::once(PROMPT.to_string()))
            .collect();
        let (physical, logical) = reads(&rows, &lines);
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 80,
                wrapped: None
            },
            "a wrap nothing in the physical read shows is not a measurement of this screen"
        );
    }

    /// Measured live: 120 double-width glyphs on a 93-column PTY. The rows read back 46
    /// characters long and 92 **columns** wide, because the last column cannot hold half a glyph.
    /// Counting characters called that pane 46 columns wide and cropped half of every row.
    #[test]
    fn a_wide_glyph_row_is_measured_in_columns_and_the_last_column_is_given_back() {
        let rows = ["日".repeat(46), "日".repeat(46), "日".repeat(28), PROMPT.into()];
        let lines = ["日".repeat(120), PROMPT.into()];
        let (physical, logical) = reads(&rows, &lines);
        let mut m = Measured {
            rect: 47,
            ..Measured::default()
        };
        m.record(reading(&physical, &logical));
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 92,
                wrapped: Some(Wrapped::AtOrOneWider(92))
            },
            "the break is at 92 because the next glyph needed two columns"
        );
        assert_eq!(
            m.cols(),
            93,
            "and with nothing else known the grid is the wider of the two it could be"
        );
    }

    /// A wrap whose boundary is unambiguous outranks one that could be a column short: measured
    /// live against a PTY held at 60 columns, where the ASCII line broke at exactly 60 and the
    /// CJK line broke at 60 with a wide glyph waiting.
    #[test]
    fn an_exact_boundary_outranks_a_wide_glyph_one() {
        let rows = [
            "#".repeat(60),
            "#".repeat(60),
            "日".repeat(30),
            "日".repeat(10),
            PROMPT.into(),
        ];
        let lines = ["#".repeat(120), "日".repeat(40), PROMPT.into()];
        let (physical, logical) = reads(&rows, &lines);
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 60,
                wrapped: Some(Wrapped::At(60))
            },
        );
    }

    /// Measured live on one pane resized between the two: 200 `日` come back as four rows of 92
    /// columns and one of 32, over a logical line of 400, **byte for byte the same** on a
    /// 92-column PTY and on a 93-column one (probe #220). Half a glyph will not sit in the last
    /// column, so the wide-glyph layout is identical on a grid of `2n` and one of `2n + 1`, and
    /// no amount of looking at these two reads can separate them.
    #[test]
    fn a_screen_of_wide_glyphs_reads_the_same_on_both_grids_it_could_be_on() {
        assert_eq!(cjk(92, 200), cjk(93, 200));
    }

    /// The residual of [#218](probe log): the wide-glyph break resolves upward, so an *even* PTY
    /// showing nothing but wide glyphs reads a column too wide. It cannot be settled from the
    /// read — but it does not have to be, because the reading before it settled it: a break at
    /// 92 says the grid is 92 or 93, and a pane that has already proved either one is not
    /// contradicted by it. Measured live at both widths: the ASCII phase wraps at exactly the
    /// PTY (rows of 92 on a 92, rows of 93 on a 93) and the CJK phase that follows reads 92 on
    /// both.
    #[test]
    fn a_wide_glyph_break_confirms_the_width_the_pane_already_proved() {
        for pty in [92, 93] {
            let mut m = Measured {
                rect: 47,
                ..Measured::default()
            };
            let (physical, logical) = wide(pty as usize, 400);
            m.record(reading(&physical, &logical));
            assert_eq!(m.cols(), pty, "the ASCII wrap proves the width outright");

            let (physical, logical) = cjk(pty as usize, 200);
            m.record(reading(&physical, &logical));
            assert_eq!(
                m.cols(),
                pty,
                "a break that agrees with the proof must not widen it"
            );
        }
    }

    /// A break the standing proof disagrees with is a different screen, and nothing is known
    /// about it but the bound — so it goes back to resolving upward, because observing above the
    /// PTY pads and observing below it crops (probe #87).
    #[test]
    fn a_wide_glyph_break_no_proof_agrees_with_still_resolves_upward() {
        let mut m = Measured {
            rect: 47,
            ..Measured::default()
        };
        let (physical, logical) = wide(60, 400);
        m.record(reading(&physical, &logical));
        let (physical, logical) = cjk(92, 200);
        m.record(reading(&physical, &logical));
        assert_eq!(m.cols(), 93, "60 is neither 92 nor 93");
    }

    /// The rows in hand can settle the break upward on their own: a row wider than the stride is
    /// a grid wider than the stride, whatever the proof said last.
    #[test]
    fn a_row_wider_than_the_break_settles_it_against_the_proof() {
        let mut m = Measured {
            rect: 47,
            ..Measured::default()
        };
        let (physical, logical) = wide(92, 400);
        m.record(reading(&physical, &logical));
        let rows = [
            "#".repeat(93),
            "日".repeat(46),
            "日".repeat(46),
            "日".repeat(28),
            PROMPT.into(),
        ];
        let lines = ["#".repeat(93), "日".repeat(120), PROMPT.into()];
        let (physical, logical) = reads(&rows, &lines);
        m.record(reading(&physical, &logical));
        assert_eq!(m.cols(), 93, "a 93-column row is not on a 92-column grid");
    }

    /// Herdr pads a row out to the grid width when it joins it to the next one, so the blanks a
    /// row was trimmed of come back in the logical line. Measured live: `aaa`, 90 spaces, `zzz`
    /// reads as two rows of 3 characters and one logical line of 96.
    #[test]
    fn a_line_that_wrapped_across_trimmed_blanks_still_proves_the_width() {
        let rows = ["aaa".to_string(), "zzz".to_string(), PROMPT.into()];
        let lines = [format!("aaa{}zzz", " ".repeat(90)), PROMPT.into()];
        let (physical, logical) = reads(&rows, &lines);
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 32,
                wrapped: Some(Wrapped::At(93))
            },
            "the widest row is 32 and the width is 93"
        );
    }

    /// Herdr also joins rows that never wrapped — measured live on any pane whose output has
    /// scrolled — and it lays them out at the grid width all the same. That is not a wrap, but it
    /// is the same measurement, and it is one the rows in hand can be checked against.
    #[test]
    fn rows_herdr_joined_without_a_wrap_still_measure_the_grid() {
        let rows = [dots(37), dots(38), dots(39), PROMPT.into()];
        let joined = format!(
            "{}{}{}{}",
            format_args!("{}{}", dots(37), " ".repeat(13)),
            format_args!("{}{}", dots(38), " ".repeat(13)),
            format_args!("{}{}", dots(39), " ".repeat(13)),
            PROMPT
        );
        let (physical, logical) = reads(&rows, &[joined]);
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 80,
                wrapped: Some(Wrapped::At(93))
            }
        );
    }

    /// Lays `text` out on a `grid`-column pane the way herdr does — wrapping before a glyph that
    /// will not fit — and returns the pair of reads it comes back as, with `tail` on its own
    /// physical row and glued to the end of the logical line the way a prompt is.
    fn laid_out(grid: u16, text: &str, tail: &str) -> (Vec<String>, String) {
        let (mut rows, mut row, mut width) = (Vec::new(), String::new(), 0u16);
        let mut logical = String::new();
        for c in text.chars() {
            let w = c.width().unwrap_or(0) as u16;
            if width + w > grid {
                // A wrapped row occupies exactly the columns it wrote: the grid width, or one
                // short of it when a double-width glyph could not straddle the last column, and
                // herdr does not pad that column back (probe #220).
                logical.push_str(&row);
                rows.push(std::mem::take(&mut row));
                width = 0;
            }
            row.push(c);
            width += w;
        }
        logical.push_str(&row);
        logical.extend(std::iter::repeat_n(' ', (grid - width) as usize));
        rows.push(row);
        rows.push(tail.to_string());
        logical.push_str(tail);
        (rows, logical)
    }

    /// Measured live on a 93-column pane: `'a' * 92`, a `日`, `'b' * 91`, another
    /// `日` and 1600 `c`, printed without a newline so the prompt joins the line. The rows
    /// come back 92, eighteen of 93, then the remainder, over **one logical line of 1891
    /// columns** — the
    /// wide glyph would not straddle the last column, so one row of the join is a column short
    /// of the rest. No single stride rebuilds that, so the walk stopped on the bottom-most line
    /// it looked at and the poll measured nothing at all.
    #[test]
    fn a_wrap_a_wide_glyph_cut_a_column_short_still_measures_the_pane() {
        let text = format!("{}日{}日{}", "a".repeat(92), "b".repeat(91), "c".repeat(1600));
        let (rows, logical) = laid_out(93, &text, PROMPT);
        assert_eq!(
            rows.iter().map(|r| columns(r)).collect::<Vec<_>>(),
            [&[92u16][..], &[93; 18][..], &[21, 32][..]].concat(),
            "the fixture is the shape the live read had"
        );
        assert_eq!(columns(&logical), 1891);
        assert_eq!(
            reading(&rows.join("\n"), &logical),
            Reading {
                floor: 93,
                wrapped: Some(Wrapped::At(93))
            },
            "the rows that filled the grid prove it outright"
        );
    }

    /// The other half of the same live capture: 400 `日` on the same pane, where the join
    /// runs eight rows of 92 and then a *padded* short row before the prompt. [#220](probe log)
    /// says a screen of nothing but wide glyphs cannot separate 92 from 93 — but that is a screen
    /// whose last row is the end of the wrap. Here a row after it was laid out at the full grid,
    /// and that row settles it.
    #[test]
    fn a_padded_row_after_a_wide_glyph_wrap_settles_the_column_it_left_open() {
        let (rows, logical) = laid_out(93, &"日".repeat(400), PROMPT);
        assert_eq!(
            rows.iter().map(|r| columns(r)).collect::<Vec<_>>(),
            [&[92u16; 8][..], &[64, 32][..]].concat()
        );
        assert_eq!(columns(&logical), 861);
        assert_eq!(
            reading(&rows.join("\n"), &logical),
            Reading {
                floor: 92,
                wrapped: Some(Wrapped::At(93))
            }
        );
    }

    #[test]
    fn a_read_that_cannot_be_reconciled_stops_rather_than_guessing() {
        let (physical, _) = wide(93, 400);
        assert_eq!(
            reading(&physical, "something else entirely\nand another line"),
            Reading {
                floor: 93,
                wrapped: None
            }
        );
    }

    /// Probe #211's second half. A proof is evidence about the screen it was read from, and the
    /// node has no event that tells it the PTY moved — so a proof that stops being re-proved
    /// stops overriding the rect. What it leaves behind is a floor, which is why letting go of a
    /// proof can only ever widen the stream.
    #[test]
    fn a_proof_that_is_never_re_proved_gives_the_rect_back_the_stream() {
        let mut m = Measured {
            rect: 120,
            ..Measured::default()
        };
        m.record(Reading {
            floor: 93,
            wrapped: Some(Wrapped::At(93)),
        });
        assert_eq!(m.cols(), 93);
        for _ in 0..PROOF_LIFETIME {
            m.record(Reading {
                floor: 20,
                wrapped: None,
            });
            assert_eq!(m.cols(), 93, "a proof does not expire on the first quiet read");
        }
        m.record(Reading {
            floor: 20,
            wrapped: None,
        });
        assert_eq!(m.cols(), 120, "and what it leaves behind never crops");
    }

    #[test]
    fn an_expired_proof_still_holds_the_stream_open_against_a_narrower_rect() {
        let mut m = Measured {
            rect: 47,
            ..Measured::default()
        };
        m.record(Reading {
            floor: 93,
            wrapped: Some(Wrapped::At(93)),
        });
        for _ in 0..=PROOF_LIFETIME {
            m.record(Reading {
                floor: 20,
                wrapped: None,
            });
        }
        assert_eq!(m.cols(), 93, "the rect is still fiction; the floor is not");
    }

    /// The operator, on 0.1.58: *"trying to type commands and it's bouncing up and down and all
    /// around"*, on a pane a desk-sized browser was matching.
    ///
    /// **Measured on the operator's own hub**, straight off the process table: `control` holding
    /// the pane at `289x69` while the observe child had come back up at `292x69`. The width
    /// inference reads the rows *in the pane*, and every one of them had been laid out at 292
    /// before the claim resized the PTY to 289 — so the first definite reading after the claim
    /// proved 292 and overwrote the width Kampr had just commanded. The client's emulator then
    /// wrapped three columns wider than the shell did, which puts every wrapped line on the wrong
    /// row and the caret on a row it was never on.
    ///
    /// A held controller *is* the geometry (#18) and herdr refuses a second (#21), so while the
    /// hold stands there is nothing to infer.
    #[test]
    fn a_reading_of_rows_written_before_a_claim_does_not_beat_the_width_it_commanded() {
        let mut m = Measured {
            rect: 292,
            commanded: Some(289),
            ..Measured::default()
        };
        // The rows still in the pane, laid out at the width it had a moment ago.
        m.record(Reading {
            floor: 292,
            wrapped: Some(Wrapped::At(292)),
        });
        assert_eq!(
            m.cols(),
            289,
            "the stream went back to the width the rows were written at, not the width the PTY has",
        );
    }

    /// **The decay, which is what actually bit.** A quiet pane offers no wrap to measure, so every
    /// reading is a floor and nothing else; after `PROOF_LIFETIME` of them a proof is dropped and
    /// `cols` falls back to the layout rect — which is the intended rule for a pane nobody is
    /// holding (`a_proof_that_is_never_re_proved_gives_the_rect_back_the_stream`) and exactly
    /// wrong for one Kampr has a controller on. The operator's pane sat quiet for minutes after
    /// the claim and the stream went back to the rect's 292 over a 289-column PTY.
    ///
    /// The rect is fiction (#68); a held controller is not (#18).
    #[test]
    fn a_commanded_width_does_not_decay_back_to_the_rect_while_the_hold_stands() {
        let mut m = Measured {
            rect: 292,
            commanded: Some(289),
            ..Measured::default()
        };
        for _ in 0..=PROOF_LIFETIME + 1 {
            m.record(Reading {
                floor: 20,
                wrapped: None,
            });
        }
        assert_eq!(
            m.cols(),
            289,
            "the stream decayed back to the layout rect while Kampr was still holding the pane",
        );
    }

    /// And the inference is kept warm underneath, so letting go needs no re-measurement: the
    /// moment the hold ends the pane's own width is already proved.
    #[test]
    fn the_reading_underneath_a_hold_takes_over_the_instant_it_is_released() {
        let mut m = Measured {
            rect: 292,
            commanded: Some(289),
            ..Measured::default()
        };
        m.record(Reading {
            floor: 292,
            wrapped: Some(Wrapped::At(292)),
        });
        assert_eq!(m.cols(), 289);
        m.commanded = None;
        assert_eq!(
            m.cols(),
            292,
            "the pane's own width had to be re-measured from scratch"
        );
    }

    /// Measured live: a controller that claimed the pane at 60 columns and then went away left
    /// the PTY at 60 with the layout rect never moving — so the rect cannot be the cue that a
    /// proof is out of date, and a fresh proof has to be able to narrow the stream.
    #[test]
    fn a_fresh_proof_re_bases_the_floor_a_wider_screen_left_behind() {
        let mut m = Measured {
            rect: 47,
            ..Measured::default()
        };
        m.record(Reading {
            floor: 93,
            wrapped: Some(Wrapped::At(93)),
        });
        m.record(Reading {
            floor: 60,
            wrapped: Some(Wrapped::At(60)),
        });
        assert_eq!(m.cols(), 60);
    }
}
