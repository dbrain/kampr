use crate::procs;
use crate::replay::{self, Origin};
use crate::screen::{Feed, Screen};
use kampr_core::provider::{AgentStatus, PaneEvent, ProgramState, ProgramStatus};
use kampr_keep::{Replay, SessionDir, SizedReader};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};

const SCROLLBACK_ROWS: usize = 10_000;
const LIVE_DEPTH: usize = 512;
const CHUNK: usize = 64 * 1024;
const TICK: Duration = Duration::from_millis(250);
const CLAIMED: Duration = Duration::from_secs(2);
/// Claude's and codex's spinners repaint several times a second, so a working harness never goes
/// this long without output; a prompt that is only blinking its cursor writes nothing at all.
pub(crate) const WORKING: Duration = Duration::from_millis(1500);
/// Output this soon after a keystroke is the terminal echoing it, not the program working.
const READ: &str = "read";
const ECHO: Duration = Duration::from_millis(300);

pub(crate) struct Live {
    pub dir: SessionDir,
    state: Mutex<State>,
    events: broadcast::Sender<PaneEvent>,
    master: Mutex<Option<OwnedFd>>,
    activity: Mutex<Activity>,
    topology: watch::Sender<u64>,
    stop: AtomicBool,
}

struct State {
    screen: Screen,
    /// The log offset the screen has been fed to, and the one last checkpointed.
    offset: u64,
    saved: Option<u64>,
    replayed: bool,
    title: String,
    alternate: bool,
}

#[derive(Default)]
struct Activity {
    input: Option<Instant>,
    output: Option<Instant>,
    working: bool,
    done: bool,
    published: Option<AgentStatus>,
    typed_at_report: Option<u64>,
    job_seen: Option<Instant>,
    job: Option<(u32, Option<String>)>,
}

pub(crate) struct Joined {
    pub reset: PaneEvent,
    pub full: PaneEvent,
    pub events: broadcast::Receiver<PaneEvent>,
}

impl Live {
    pub fn start(dir: SessionDir, topology: watch::Sender<u64>) -> Arc<Self> {
        let (cols, rows) = size_of(&dir);
        let typed_at_report = read_at(&dir);
        let live = Arc::new(Self {
            dir,
            state: Mutex::new(State {
                screen: Screen::new(cols, rows, SCROLLBACK_ROWS),
                offset: 0,
                saved: None,
                replayed: false,
                title: String::new(),
                alternate: false,
            }),
            events: broadcast::channel(LIVE_DEPTH).0,
            master: Mutex::new(None),
            activity: Mutex::new(Activity {
                typed_at_report,
                ..Activity::default()
            }),
            topology,
            stop: AtomicBool::new(false),
        });
        let tail = live.clone();
        std::thread::Builder::new()
            .name(format!("kampr-native-{}", live.dir.id()))
            .spawn(move || tail.tail())
            .expect("a tail thread");
        live
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn join(&self) -> Joined {
        let mut state = self.state.lock().unwrap();
        if let Some(diff) = state.screen.frame_diff() {
            let _ = self.events.send(PaneEvent::Bytes {
                full: false,
                bytes: diff,
            });
        }
        let events = self.events.subscribe();
        let (cols, rows) = state.screen.size();
        Joined {
            reset: PaneEvent::Reset { cols, rows },
            full: PaneEvent::Bytes {
                full: true,
                bytes: state.screen.frame_full(),
            },
            events,
        }
    }

    pub fn with_screen<T>(&self, f: impl FnOnce(&Screen) -> T) -> T {
        f(&self.state.lock().unwrap().screen)
    }

    fn resize(&self, cols: u16, rows: u16) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.screen.size() == (cols, rows) {
            return false;
        }
        state.screen.resize(cols, rows);
        // A resized screen's next diff is drawn against a blank grid of the new size, which is
        // what a consumer holds after a `Reset`.
        let _ = self.events.send(PaneEvent::Reset { cols, rows });
        if let Some(bytes) = state.screen.frame_diff() {
            let _ = self.events.send(PaneEvent::Bytes { full: true, bytes });
        }
        true
    }

    pub fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        self.write_master(bytes)?;
        let reports = self.state.lock().unwrap().screen.program_reports();
        let mut activity = self.activity.lock().unwrap();
        activity.input = Some(Instant::now());
        activity.done = false;
        if activity.typed_at_report != Some(reports) {
            activity.typed_at_report = Some(reports);
            mark_read(&self.dir, reports);
        }
        drop(activity);
        self.tick();
        Ok(())
    }

    /// Returns once the tail has met the resize in the journal, so the screen is at the size
    /// asked for when the caller next looks.
    pub fn claim(&self, cols: u16, rows: u16, holder: &str) -> std::io::Result<()> {
        kampr_keep::claim_size(&self.dir, cols, rows, holder)?;
        let deadline = Instant::now() + CLAIMED;
        while self.with_screen(Screen::size) != (cols, rows) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    pub fn program(&self) -> Option<ProgramStatus> {
        self.state.lock().unwrap().screen.program().cloned()
    }

    /// The harness's own report when it makes them (#600), the output window otherwise. Its `done`
    /// is read, as the window's is, once somebody types; it has no error word, so an error is idle.
    pub fn status(&self) -> AgentStatus {
        let exited = self.dir.exit().is_some();
        let (program, reports) = {
            let state = self.state.lock().unwrap();
            (
                state.screen.program().map(|p| p.state),
                state.screen.program_reports(),
            )
        };
        let mut activity = self.activity.lock().unwrap();
        if let Some(state) = program.filter(|_| !exited) {
            return match state {
                ProgramState::Working => AgentStatus::Working,
                ProgramState::Blocked => AgentStatus::Blocked,
                ProgramState::Done if activity.typed_at_report != Some(reports) => AgentStatus::Done,
                ProgramState::Done | ProgramState::Idle | ProgramState::Error => AgentStatus::Idle,
            };
        }
        let working = !exited && activity.output.is_some_and(|at| at.elapsed() < WORKING);
        if activity.working && !working {
            activity.done = true;
        }
        activity.working = working;
        match (working, activity.done) {
            (true, _) => AgentStatus::Working,
            (false, true) => AgentStatus::Done,
            (false, false) => AgentStatus::Idle,
        }
    }

    fn tick(&self) {
        let status = self.status();
        let mut activity = self.activity.lock().unwrap();
        if activity.published != Some(status) {
            activity.published = Some(status);
            drop(activity);
            self.bump();
        }
    }

    fn bump(&self) {
        self.topology.send_modify(|n| *n += 1);
    }

    /// What the herd was last told the job is, so that [`Live::follow_job`] corrects the herd
    /// rather than its own last look: a job that lives between two of its looks, as `cat` ended at
    /// once by ctrl-d does, is otherwise published and never taken back.
    pub(crate) fn published_job(&self, job: Option<(u32, Option<String>)>) {
        self.activity.lock().unwrap().job = job;
    }

    /// A job that starts or ends may print nothing the status sees, as a `cat` ended by ctrl-d
    /// does when the shell's prompt lands inside the echo window, and the herd would go on naming it.
    fn follow_job(&self) {
        let now = Instant::now();
        {
            let mut activity = self.activity.lock().unwrap();
            if activity
                .job_seen
                .is_some_and(|at| now.saturating_duration_since(at) < TICK)
            {
                return;
            }
            activity.job_seen = Some(now);
        }
        let job = self
            .dir
            .keeper()
            .and_then(|k| procs::foreground(k.child))
            .map(|fg| (fg.pid, procs::job(&fg)));
        let mut activity = self.activity.lock().unwrap();
        if activity.job != job {
            activity.job = job;
            drop(activity);
            self.bump();
        }
    }

    fn master(&self) -> std::io::Result<OwnedFd> {
        let mut master = self.master.lock().unwrap();
        if let Some(fd) = master.as_ref() {
            return fd.try_clone();
        }
        let fd = kampr_keep::borrow(&self.dir)?;
        let out = fd.try_clone()?;
        *master = Some(fd);
        Ok(out)
    }

    fn write_master(&self, mut bytes: &[u8]) -> std::io::Result<()> {
        let fd = self.master()?;
        while !bytes.is_empty() {
            match rustix::io::write(&fd, bytes) {
                Ok(n) => bytes = &bytes[n..],
                Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => {}
                Err(e) => {
                    *self.master.lock().unwrap() = None;
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }

    fn tail(self: Arc<Self>) {
        let Some(mut reader) = self.resume() else {
            return;
        };
        let mut replay_end = reader.end().unwrap_or(0);
        let mut buf = vec![0u8; CHUNK];
        while !self.stop.load(Ordering::Relaxed) {
            let from = reader.offset();
            match reader.read(&mut buf) {
                Ok(Replay::Data(n)) => {
                    let split = replay_end.saturating_sub(from).min(n as u64) as usize;
                    if split > 0 {
                        let mut state = self.state.lock().unwrap();
                        state.screen.feed(&buf[..split]);
                        state.offset = from + split as u64;
                    }
                    if split < n {
                        self.feed_live(from + split as u64, &buf[split..n]);
                    }
                }
                Ok(Replay::Resize(to)) => {
                    self.resize(to.cols, to.rows);
                }
                Ok(Replay::Truncated { .. }) => {
                    let Some(resumed) = self.resume() else {
                        return;
                    };
                    reader = resumed;
                    replay_end = replay_end.max(reader.end().unwrap_or(0));
                }
                Ok(Replay::CaughtUp) => {
                    self.caught_up();
                    self.tick();
                    self.follow_job();
                    self.checkpoint(false);
                    if self.dir.exit().is_some() && reader.end().is_ok_and(|end| end <= reader.offset()) {
                        self.checkpoint(true);
                        return;
                    }
                    if reader.wait(TICK).is_err() {
                        std::thread::sleep(TICK);
                    }
                }
                Err(e) => {
                    if !self.dir.path().exists() {
                        return;
                    }
                    tracing::debug!(session = %self.dir.id(), error = %e, "reading a native session's log");
                    std::thread::sleep(TICK);
                }
            }
        }
        self.checkpoint(true);
    }

    /// The screen from the newest retained checkpoint, or the log's oldest byte. A consumer
    /// already following gets a `Reset`, since what it holds is of the screen this replaces.
    fn resume(&self) -> Option<SizedReader> {
        loop {
            match replay::resume(self.dir.path(), SCROLLBACK_ROWS) {
                Ok(resumed) => {
                    let mut state = self.state.lock().unwrap();
                    let (cols, rows) = resumed.screen.size();
                    state.screen = resumed.screen;
                    state.offset = resumed.reader.offset();
                    state.saved = match resumed.origin {
                        Origin::Checkpoint(at) => Some(at),
                        Origin::Start | Origin::MidStream(_) => None,
                    };
                    let _ = self.events.send(PaneEvent::Reset { cols, rows });
                    return Some(resumed.reader);
                }
                Err(e) => {
                    if self.stop.load(Ordering::Relaxed) || !self.dir.path().exists() {
                        return None;
                    }
                    tracing::warn!(session = %self.dir.id(), error = %e, "cannot read a native session's log");
                    std::thread::sleep(TICK);
                }
            }
        }
    }

    /// `finishing` writes one whatever the distance from the last, for a node going away. A
    /// resize journalled at the offset but not yet applied would make the state wrong for the
    /// offset, so a screen whose size the journal disagrees with is not saved.
    pub fn checkpoint(&self, finishing: bool) {
        let (offset, bytes) = {
            let mut state = self.state.lock().unwrap();
            let due = match state.saved {
                Some(saved) if saved == state.offset => false,
                Some(saved) => finishing || state.offset >= saved + replay::EVERY,
                None => finishing || state.offset >= replay::EVERY,
            };
            if !due || state.offset == 0 {
                return;
            }
            let journal = kampr_keep::size_at(self.dir.path(), state.offset)
                .ok()
                .map(|s| (s.cols, s.rows));
            if journal != Some(state.screen.size()) {
                return;
            }
            let Some(bytes) = state.screen.checkpoint() else {
                return;
            };
            state.saved = Some(state.offset);
            (state.offset, bytes)
        };
        if let Err(e) = replay::write(self.dir.path(), offset, &bytes) {
            tracing::debug!(session = %self.dir.id(), error = %e, "writing a checkpoint");
        }
    }

    fn caught_up(&self) {
        let mut state = self.state.lock().unwrap();
        if state.replayed {
            return;
        }
        state.replayed = true;
        state.title = state.screen.title().to_string();
        state.alternate = state.screen.alternate_screen();
        if let Some(bytes) = state.screen.frame_diff() {
            let _ = self.events.send(PaneEvent::Bytes { full: false, bytes });
        }
    }

    /// Only bytes the program writes from now on are answered: a query in the replayed log was
    /// asked of whoever was there when it was written, and answering it again types the answer
    /// into whatever is reading the terminal today.
    fn feed_live(&self, at: u64, bytes: &[u8]) {
        let (fed, moved) = {
            let mut state = self.state.lock().unwrap();
            let fed = state.screen.feed(bytes);
            state.offset = at + bytes.len() as u64;
            if let Some(diff) = state.screen.frame_diff() {
                let _ = self.events.send(PaneEvent::Bytes {
                    full: false,
                    bytes: diff,
                });
            }
            let title = state.screen.title() != state.title;
            let alternate = state.screen.alternate_screen() != state.alternate;
            if title || alternate {
                state.title = state.screen.title().to_string();
                state.alternate = state.screen.alternate_screen();
            }
            (fed, title || alternate)
        };
        {
            let mut activity = self.activity.lock().unwrap();
            let now = Instant::now();
            if activity
                .input
                .is_none_or(|at| now.saturating_duration_since(at) > ECHO)
            {
                activity.output = Some(now);
            }
        }
        let replies = self.ours(at, &fed);
        if !replies.is_empty()
            && let Err(e) = self.write_master(&replies)
        {
            tracing::debug!(session = %self.dir.id(), error = %e, "answering a terminal query");
        }
        if moved {
            self.bump();
        }
        self.tick();
    }

    /// A hosting terminal answers for itself, `OSC 7501` through `kampr attach`, every query that
    /// ends past the offset it registered at; answering one of those as well would hand the
    /// program two replies to one question. The rest are the node's (docs/17 §4).
    fn ours(&self, at: u64, fed: &Feed) -> Vec<u8> {
        if fed.asked.is_empty() {
            return Vec::new();
        }
        let hosts = self.dir.hosts_settled().unwrap_or_else(|_| self.dir.hosts());
        fed.asked
            .iter()
            .filter(|ask| !hosts.iter().any(|h| h.covers(at + ask.end as u64)))
            .flat_map(|ask| fed.replies[ask.reply.clone()].iter().copied())
            .collect()
    }
}

pub(crate) fn size_of(dir: &SessionDir) -> (u16, u16) {
    match dir.size() {
        Some(size) => (size.cols, size.rows),
        None => dir.spec().map_or((80, 24), |spec| (spec.cols, spec.rows)),
    }
}

// The harness's `done` stays until somebody types, and a node restart must not bring back one that
// was read already.
fn read_at(dir: &SessionDir) -> Option<u64> {
    std::fs::read_to_string(dir.path().join(READ))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn mark_read(dir: &SessionDir, reports: u64) {
    let path = dir.path().join(READ);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, reports.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}
