use crate::layout::Layouts;
use crate::live::{self, Live};
use crate::procs;
use crate::reflow::{self, Views};
use crate::screen::Screen;
use crate::watcher::{self, Change};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use kampr_core::desk::{Desk, DeskReport, Hosting};
use kampr_core::provider::{
    AgentStatus, Found, Input, NativeExit, NativePane, PaneInfo, PaneStream, ProgramStatus, Provider,
    RawScrollback, Reflow, split_view,
};
use kampr_core::tile::Split;
use kampr_journal::process::{Harness, PaneProcess};
use kampr_keep::{Ending, Launch, SessionDir, Spec};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{broadcast, watch};

pub const PREFIX: &str = "n:";

/// herdr's own `pane.read recent` cap (#51), so a ring stitches native reads exactly as it does
/// herdr's.
const READ_CAP: u32 = 1000;
const LOST_DEPTH: usize = 64;
/// The herdr provider's cap on listed hits, for the same reason: `total` still counts them all.
const LISTED: usize = 32;
/// Left in a session by `close`, so its end is one somebody asked for and not one to read about.
const CLOSED: &str = "closed";

/// The node's screen-reading dialog detector, handed in because it lives above this crate.
pub type Blocked = Arc<dyn Fn(Option<&str>, &str) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct NativeConfig {
    pub root: PathBuf,
    pub keeper: PathBuf,
    /// The binary whose `attach` a herdr pane runs to show a session on the desk.
    pub attach: PathBuf,
    /// `None` is [`Launch::auto`], decided at the first create so a node that never makes a
    /// session never asks systemd anything.
    pub launch: Option<Launch>,
    pub blocked: Option<Blocked>,
    /// `[naming] send_argv`: a command line can carry a password, so it is not on the wire unless
    /// the operator says so, here as on a herdr pane.
    pub send_argv: bool,
    /// Whether a session closed on purpose or ended with status 0 is forgotten by itself. A fleet
    /// run is kept over the same keepers and its board needs every end it had.
    pub forget_ended: bool,
}

impl NativeConfig {
    pub fn new(root: Option<PathBuf>, keeper: Option<PathBuf>) -> Result<Self> {
        let root = match root {
            Some(root) => root,
            None => kampr_keep::root().context("the native session root")?,
        };
        let keeper = match keeper {
            Some(keeper) => keeper,
            None => kampr_keep::own_exe().context("finding this binary")?,
        };
        Ok(Self {
            root,
            attach: keeper.clone(),
            keeper,
            launch: None,
            blocked: None,
            send_argv: false,
            forget_ended: false,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewSession {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub label: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
    pub cohort: Option<String>,
}

pub struct NativeProvider {
    inner: Arc<Inner>,
}

struct Inner {
    config: NativeConfig,
    lives: Mutex<HashMap<String, Arc<Live>>>,
    holds: Mutex<HashMap<String, Hold>>,
    lease: AtomicU64,
    lost: broadcast::Sender<(String, u64)>,
    topology: watch::Sender<u64>,
    views: Views,
    layouts: Mutex<Layouts>,
}

struct Hold {
    token: u64,
    found: (u16, u16),
    holder: String,
}

impl NativeProvider {
    pub fn new(config: NativeConfig) -> Result<Self> {
        kampr_keep::make_private_dir(&config.root)
            .with_context(|| format!("creating {}", config.root.display()))?;
        let inner = Arc::new(Inner {
            lives: Mutex::new(HashMap::new()),
            holds: Mutex::new(HashMap::new()),
            lease: AtomicU64::new(1),
            lost: broadcast::channel(LOST_DEPTH).0,
            topology: watch::channel(0).0,
            views: Views::default(),
            layouts: Mutex::new(Layouts::load(&config.root)),
            config,
        });
        inner.rescan();
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        watcher::spawn(inner.config.root.clone(), move |changes: BTreeSet<Change>| {
            let Some(inner) = weak.upgrade() else {
                return false;
            };
            inner.changed(changes);
            true
        });
        Ok(Self { inner })
    }

    pub fn root(&self) -> &Path {
        &self.inner.config.root
    }

    /// `env` first because herdr refuses a `resume_argv` that starts with a path, and does not
    /// give a restored pane the `env` it was split with (#583), so the root rides in the argv.
    pub fn resume_argv(&self, session_id: &str) -> Vec<String> {
        vec![
            "env".to_string(),
            format!("KAMPR_KEEP_ROOT={}", self.inner.config.root.display()),
            self.inner.config.attach.display().to_string(),
            "attach".to_string(),
            session_id.to_string(),
        ]
    }

    /// The leading space and the `unset` keep the line out of the shell's history (#575).
    pub fn attach_line(&self, session_id: &str) -> String {
        let argv: Vec<String> = self.resume_argv(session_id).iter().map(|a| quoted(a)).collect();
        format!(" unset HISTFILE; exec {}\r", argv.join(" "))
    }

    pub fn shutdown(&self) {
        for live in self.inner.lives.lock().unwrap().drain().map(|(_, l)| l) {
            live.stop();
            live.checkpoint(true);
        }
    }

    pub async fn create(&self, new: NewSession) -> Result<String> {
        if new.argv.is_empty() {
            bail!("a native session needs a command");
        }
        let mut spec = Spec::new(new.argv);
        spec.cwd = new.cwd.or_else(|| std::env::var("HOME").ok());
        spec.label = new.label.filter(|l| !l.trim().is_empty());
        spec.cols = new.cols;
        spec.rows = new.rows;
        for (key, value) in new.env {
            spec.env.retain(|(have, _)| *have != key);
            spec.env.push((key, value));
        }
        spec.cohort = new.cohort;
        let config = self.inner.config.clone();
        let session = tokio::task::spawn_blocking(move || {
            let launch = config.launch.unwrap_or_else(Launch::auto);
            kampr_keep::spawn(&config.root, spec, launch, &config.keeper)
        })
        .await??;
        let pane = format!("{PREFIX}{}", session.id());
        self.inner.adopt(session);
        self.inner.bump();
        Ok(pane)
    }

    /// `new` beside `at`, in the tree their clients lay them out by. A session split from one in
    /// no tree starts one.
    pub fn split(&self, at: &str, new: &str, split: Split, ratio: u16) -> Result<()> {
        self.inner.layouts.lock().unwrap().split(at, new, split, ratio)?;
        self.inner.bump();
        Ok(())
    }

    pub fn close(&self, pane: &str) -> Result<()> {
        let session = self.require(pane)?;
        std::fs::write(session.path().join(CLOSED), b"").with_context(|| format!("marking {pane} closed"))?;
        kampr_keep::close(&session).with_context(|| format!("hanging up {pane}"))?;
        self.inner.sweep(&session);
        Ok(())
    }

    /// Refuses a live session, as `fleet.forget` does: forgetting one would leave a program
    /// running with nothing able to reach it.
    pub fn forget(&self, pane: &str) -> Result<()> {
        let session = self.require(pane)?;
        if session.alive() {
            bail!("{pane} is still running; close it first");
        }
        self.inner.forget(&session)
    }

    pub async fn resize(&self, pane: &str, cols: u16, rows: u16, holder: &str) -> Result<()> {
        let live = self.live(pane)?;
        let holder = holder.to_string();
        let claimed = live.clone();
        tokio::task::spawn_blocking(move || claimed.claim(cols, rows, &holder))
            .await?
            .with_context(|| format!("resizing {pane}"))?;
        self.inner.bump();
        Ok(())
    }

    /// A claim that remembers the size it found, so letting go puts the session back (ADR 0013).
    /// A hold that displaces another carries the first one's found size forward.
    pub async fn hold(&self, pane: &str, cols: u16, rows: u16, holder: &str) -> Result<(u64, (u16, u16))> {
        let session = self.require(pane)?;
        let carried = self.inner.holds.lock().unwrap().remove(pane);
        if let Some(hold) = &carried {
            self.taken(pane, hold.token);
        }
        let (found, found_holder) = match carried {
            Some(hold) => (hold.found, hold.holder),
            None => (
                self.size(pane).unwrap_or((cols, rows)),
                session.size().map_or_else(|| "spawn".to_string(), |s| s.holder),
            ),
        };
        self.resize(pane, cols, rows, holder).await?;
        let token = self.inner.lease.fetch_add(1, Ordering::Relaxed);
        self.inner.holds.lock().unwrap().insert(
            pane.to_string(),
            Hold {
                token,
                found,
                holder: found_holder,
            },
        );
        Ok((token, found))
    }

    /// A plain claim supersedes a standing match without putting anything back: the operator
    /// asked for this size by name.
    pub fn unhold(&self, pane: &str) {
        let removed = self.inner.holds.lock().unwrap().remove(pane);
        if let Some(hold) = removed {
            self.taken(pane, hold.token);
        }
    }

    /// Every hold ended by somebody other than its holder, as the pane and the token it ended.
    pub fn subscribe_lost(&self) -> broadcast::Receiver<(String, u64)> {
        self.inner.lost.subscribe()
    }

    fn taken(&self, pane: &str, token: u64) {
        let _ = self.inner.lost.send((pane.to_string(), token));
    }

    pub fn found_under(&self, pane: &str, token: u64) -> Option<(u16, u16)> {
        let holds = self.inner.holds.lock().unwrap();
        holds.get(pane).filter(|h| h.token == token).map(|h| h.found)
    }

    /// `None` lets go of whatever hold stands; a token lets go only of its own, so a viewer
    /// displaced by a newer one cannot take the newer one's hold down with it.
    pub async fn let_go(&self, pane: &str, token: Option<u64>) -> Result<bool> {
        let hold = {
            let mut holds = self.inner.holds.lock().unwrap();
            match holds.get(pane) {
                Some(h) if token.is_none_or(|t| t == h.token) => holds.remove(pane),
                _ => None,
            }
        };
        let Some(hold) = hold else {
            return Ok(false);
        };
        if token.is_none() {
            self.taken(pane, hold.token);
        }
        let session = self.require(pane)?;
        if session.alive() {
            // An attach that started under the hold left the size alone (attach.rs), so the size
            // found can be from before it; a live attach's terminal is what is showing it now.
            let mut desks = session.hosts();
            let found_by = desks
                .iter()
                .position(|h| kampr_keep::attach_holder(h.pid) == hold.holder);
            let desk = match found_by {
                Some(at) => Some(desks.swap_remove(at)),
                None => desks.pop(),
            };
            let (cols, rows, holder) = match desk {
                Some(desk) => (desk.cols, desk.rows, kampr_keep::attach_holder(desk.pid)),
                None => (hold.found.0, hold.found.1, hold.holder),
            };
            self.resize(pane, cols, rows, &holder).await?;
        }
        Ok(true)
    }

    pub fn rename(&self, pane: &str, label: Option<&str>) -> Result<()> {
        let session = self.require(pane)?;
        let mut spec = session.spec()?;
        spec.label = label.map(str::to_string).filter(|l| !l.trim().is_empty());
        let path = session.path().join("spec.json");
        let tmp = session.path().join("spec.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&spec)?)?;
        std::fs::rename(&tmp, &path)?;
        self.inner.bump();
        Ok(())
    }

    pub fn size(&self, pane: &str) -> Option<(u16, u16)> {
        match self.live(pane) {
            Ok(live) => Some(live.with_screen(|s| s.size())),
            Err(_) => self.session(pane).map(|s| live::size_of(&s)),
        }
    }

    pub fn program(&self, pane: &str) -> Option<ProgramStatus> {
        self.live(pane).ok()?.program()
    }

    pub fn visible(&self, pane: &str) -> Option<String> {
        Some(self.live(pane).ok()?.with_screen(visible))
    }

    pub fn session(&self, pane: &str) -> Option<SessionDir> {
        let id = pane.strip_prefix(PREFIX)?;
        if id.is_empty() || id.contains('/') || id.starts_with('.') {
            return None;
        }
        let session = SessionDir::at(self.inner.config.root.join(id));
        session.path().join("spec.json").is_file().then_some(session)
    }

    /// The keeper's child and everything below it, because a harness started from a shell is a
    /// grandchild and the marker is keyed on its pid, not the shell's.
    pub fn pane_processes(&self, pane: &str) -> Vec<PaneProcess> {
        self.running_child(pane)
            .map(|child| {
                procs::descendants(child)
                    .into_iter()
                    .map(PaneProcess::look_up)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn agent_harness(&self, pane: &str) -> Harness {
        match self
            .running_child(pane)
            .and_then(procs::foreground)
            .as_ref()
            .and_then(procs::harness)
        {
            Some((_, process)) => Harness::Running(process),
            None => Harness::Absent,
        }
    }

    fn running_child(&self, pane: &str) -> Option<u32> {
        let session = self.session(pane).filter(SessionDir::alive)?;
        session.keeper().map(|k| k.child)
    }

    /// A key [`kampr_core::provider::view_key`] made: this session's screen re-laid at a width.
    fn view(&self, key: &str) -> Result<Option<(Arc<Live>, reflow::Shared)>> {
        let (pane, Some(cols)) = split_view(key) else {
            return Ok(None);
        };
        let live = self.live(pane)?;
        let view = self.inner.views.get(live.dir.id(), cols);
        Ok(Some((live, view)))
    }

    fn require(&self, pane: &str) -> Result<SessionDir> {
        self.session(pane)
            .ok_or_else(|| anyhow!("{pane} is not a native session"))
    }

    fn live(&self, pane: &str) -> Result<Arc<Live>> {
        let session = self.require(pane)?;
        if let Some(live) = self.inner.lives.lock().unwrap().get(session.id()) {
            return Ok(live.clone());
        }
        Ok(self.inner.adopt(session))
    }
}

impl Drop for NativeProvider {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Inner {
    fn bump(&self) {
        self.topology.send_modify(|n| *n += 1);
    }

    fn adopt(&self, session: SessionDir) -> Arc<Live> {
        let mut lives = self.lives.lock().unwrap();
        lives
            .entry(session.id().to_string())
            .or_insert_with(|| Live::start(session, self.topology.clone()))
            .clone()
    }

    fn forget(&self, session: &SessionDir) -> Result<()> {
        if let Some(live) = self.lives.lock().unwrap().remove(session.id()) {
            live.stop();
        }
        self.holds
            .lock()
            .unwrap()
            .remove(&format!("{PREFIX}{}", session.id()));
        remove_settled(session.path()).with_context(|| format!("removing {}", session.path().display()))?;
        self.bump();
        Ok(())
    }

    /// A session that ended because somebody closed it, or whose program finished cleanly, has
    /// nothing left to say, as a herdr pane is gone when its shell exits. A failure is kept so its
    /// status can be read, until somebody forgets it.
    fn sweep(&self, session: &SessionDir) {
        if !self.config.forget_ended {
            return;
        }
        let Some(exit) = session.exit() else {
            return;
        };
        if exit.code != Some(0) && !session.path().join(CLOSED).exists() {
            return;
        }
        if let Err(e) = self.forget(session) {
            tracing::warn!(session = session.id(), error = %format!("{e:#}"), "cannot forget an ended session");
        }
    }

    fn rescan(&self) {
        let Ok(found) = kampr_keep::list(&self.config.root) else {
            return;
        };
        for session in &found {
            self.sweep(session);
        }
        let found: Vec<SessionDir> = found.into_iter().filter(|s| s.path().exists()).collect();
        let ids: BTreeSet<String> = found.iter().map(|s| s.id().to_string()).collect();
        // An ended session is replayed when somebody looks at it, not at every node start.
        for session in found.into_iter().filter(|s| s.exit().is_none()) {
            self.adopt(session);
        }
        self.lives.lock().unwrap().retain(|id, live| {
            let keep = ids.contains(id);
            if !keep {
                live.stop();
            }
            keep
        });
    }

    fn changed(&self, changes: BTreeSet<Change>) {
        if changes.is_empty() {
            return;
        }
        for change in &changes {
            match change {
                Change::Sessions => self.rescan(),
                Change::Session(id) => self.sweep(&SessionDir::at(self.config.root.join(id))),
            }
        }
        self.bump();
    }

    fn pane_info(&self, session: &SessionDir, live: Option<&Live>) -> Option<PaneInfo> {
        let spec = session.spec().ok()?;
        let keeper = session.keeper();
        let ending = session.ending();
        let lost = ending == Some(Ending::Lost);
        let exit = match ending {
            Some(Ending::Exited(exit)) => Some(exit),
            _ => None,
        };
        let fg = match (&exit, &keeper) {
            (None, Some(k)) if !lost => procs::foreground(k.child),
            _ => None,
        };
        let job = fg.as_ref().map(|fg| (fg.pid, procs::job(fg)));
        if let Some(live) = live {
            live.published_job(job.clone());
        }
        let harness = fg.as_ref().and_then(procs::harness);
        let agent = harness.as_ref().map(|(name, _)| name.to_string());
        let hosts = session.hosts();
        let (cols, rows, scrollback_rows, title, screen) = match live {
            Some(live) => live.with_screen(|s| {
                let (cols, rows) = s.size();
                let history = if s.alternate_screen() { 0 } else { s.history_rows() };
                (cols, rows, history, s.title().to_string(), visible(s))
            }),
            None => {
                let (cols, rows) = live::size_of(session);
                (cols, rows, 0, String::new(), String::new())
            }
        };
        let program = live.and_then(Live::program).filter(|_| exit.is_none());
        let mut status = live.map_or(AgentStatus::Idle, Live::status);
        if agent.is_some()
            && program.is_none()
            && status != AgentStatus::Working
            && self
                .config
                .blocked
                .as_ref()
                .is_some_and(|blocked| blocked(agent.as_deref(), &screen))
        {
            status = AgentStatus::Blocked;
        }
        let argv_line = spec.argv.join(" ");
        Some(PaneInfo {
            pane_id: format!("{PREFIX}{}", session.id()),
            cwd: fg
                .as_ref()
                .and_then(|f| procs::cwd(f.pid))
                .or_else(|| spec.cwd.clone()),
            label: spec
                .label
                .clone()
                .or_else(|| self.config.send_argv.then(|| argv_line.clone())),
            terminal_title: agent.as_ref().map(|_| title).filter(|t| !t.is_empty()),
            agent,
            agent_harness: match harness {
                Some((_, process)) => Harness::Running(process),
                None => Harness::Absent,
            },
            agent_status: status,
            cols: Some(cols),
            rows,
            scrollback_rows,
            cmd: match job {
                Some((_, job)) => job,
                None => Some(
                    spec.argv
                        .first()
                        .map(|a| a.rsplit('/').next().unwrap_or(a).to_string())
                        .unwrap_or_default(),
                ),
            },
            argv: self.config.send_argv.then(|| match &fg {
                Some(fg) if !fg.argv.is_empty() => fg.argv.join(" "),
                _ => argv_line.clone(),
            }),
            native: Some(NativePane {
                session_id: session.id().to_string(),
                on_desk: hosts.iter().find_map(|h| h.herdr_pane.clone()),
                detached: hosts.is_empty(),
                size_holder: session.size().map(|s| s.holder),
                exited: match exit {
                    Some(exit) => Some(NativeExit {
                        code: exit.code,
                        signal: exit.signal,
                    }),
                    None if lost => Some(NativeExit {
                        code: None,
                        signal: None,
                    }),
                    None => None,
                },
                program,
            }),
            ..PaneInfo::default()
        })
    }
}

#[async_trait]
impl Provider for NativeProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        let inner = self.inner.clone();
        Ok(tokio::task::spawn_blocking(move || {
            let sessions = kampr_keep::list(&inner.config.root).unwrap_or_default();
            let lives = inner.lives.lock().unwrap().clone();
            let mut panes: Vec<PaneInfo> = sessions
                .iter()
                .filter_map(|session| inner.pane_info(session, lives.get(session.id()).map(Arc::as_ref)))
                .collect();
            let mut layouts = inner.layouts.lock().unwrap();
            let present = sessions.iter().map(|s| format!("{PREFIX}{}", s.id())).collect();
            if let Err(e) = layouts.keep(&present) {
                tracing::warn!(error = %format!("{e:#}"), "cannot keep the native layouts");
            }
            let mut tiles = layouts.tiles();
            for pane in &mut panes {
                pane.tile = tiles.remove(&pane.pane_id);
            }
            panes
        })
        .await?)
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        if let Some((live, view)) = self.view(pane_id)? {
            return Ok(reflow::stream(live, view));
        }
        let live = self.live(pane_id)?;
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let task = tokio::spawn(async move {
            loop {
                let joined = live.join();
                let mut events = joined.events;
                if tx.send(joined.reset).await.is_err() || tx.send(joined.full).await.is_err() {
                    return;
                }
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                        // A watcher that fell behind rejoins from a full grid rather than a hole.
                        Err(broadcast::error::RecvError::Lagged(_)) => break,
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        });
        Ok(PaneStream::supervised(rx, task))
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        let live = self.live(pane_id)?;
        // A master borrowed before the end still takes writes, so they would vanish unrefused.
        if let Some(exit) = live.dir.exit() {
            bail!("{pane_id} has ended ({})", crate::exit_status(&exit));
        }
        let bytes = match input {
            Input::Bytes(bytes) => bytes,
            Input::Keys(keys) => {
                let kitty = live.with_screen(Screen::keyboard_flags);
                keys.iter()
                    .map(|k| {
                        kampr_core::keys::encode_for(k, kitty).ok_or_else(|| anyhow!("unknown key {k:?}"))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .concat()
            }
        };
        tokio::task::spawn_blocking(move || live.write(&bytes))
            .await?
            .with_context(|| format!("the keeper of {pane_id} is not answering"))?;
        Ok(())
    }

    /// `None` while the alternate screen is up, as herdr answers (#244): the rows a full-screen
    /// program draws are not history, and the primary screen's are not what is on show.
    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        if let Some((live, view)) = self.view(pane_id)? {
            return Ok(live.with_screen(|s| {
                (!s.alternate_screen()).then(|| view.lock().unwrap().scrollback(s, READ_CAP as usize))
            }));
        }
        let live = self.live(pane_id)?;
        Ok(live.with_screen(|s| {
            if s.alternate_screen() {
                return None;
            }
            let end = s.first_row() + s.history_rows();
            Some(s.scrollback(Some(end.saturating_sub(READ_CAP).max(s.first_row()))))
        }))
    }

    async fn read_rows(&self, pane_id: &str, from: u32, to: u32) -> Result<Option<Vec<String>>> {
        if let Some((live, view)) = self.view(pane_id)? {
            return Ok(live.with_screen(|s| view.lock().unwrap().read_rows(s, from, to)));
        }
        let live = self.live(pane_id)?;
        Ok(live.with_screen(|s| s.read_rows(from, to)))
    }

    async fn find(
        &self,
        pane_id: &str,
        query: &str,
        backward: bool,
        from: Option<u32>,
    ) -> Result<Option<Found>> {
        if query.is_empty() {
            return Ok(None);
        }
        let mut found = match self.view(pane_id)? {
            Some((live, view)) => live.with_screen(|s| view.lock().unwrap().find(s, query, backward, from)),
            None => self.live(pane_id)?.with_screen(|s| s.find(query, backward, from)),
        };
        found.hits.truncate(LISTED);
        Ok(Some(found))
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.inner.topology.subscribe()
    }

    /// A pager takes the alternate screen for a moment and gives it back (#244); only a harness
    /// holds it for its whole life, so only a harness supersedes the ring.
    fn harness_owns_the_screen(&self, pane_id: &str) -> bool {
        let Ok(live) = self.live(split_view(pane_id).0) else {
            return false;
        };
        live.with_screen(|s| s.alternate_screen())
            && live
                .dir
                .keeper()
                .and_then(|k| procs::foreground(k.child))
                .is_some_and(|fg| procs::harness(&fg).is_some())
    }

    fn reflow(&self, pane_id: &str) -> Reflow {
        match self.live(split_view(pane_id).0) {
            Ok(live) => self.inner.views.mode(&live),
            Err(_) => Reflow::Unsupported,
        }
    }

    fn owns(&self, pane_id: &str) -> bool {
        pane_id.starts_with(PREFIX)
    }
}

impl Desk for NativeProvider {
    fn hosting(&self) -> Vec<Hosting> {
        kampr_keep::list(&self.inner.config.root)
            .unwrap_or_default()
            .iter()
            .filter(|s| s.exit().is_none())
            .flat_map(|session| {
                session.hosts().into_iter().filter_map(|host| {
                    Some(Hosting {
                        socket: PathBuf::from(host.herdr_socket?),
                        herdr_pane: host.herdr_pane?,
                        host_pid: host.pid,
                        native_pane: format!("{PREFIX}{}", session.id()),
                    })
                })
            })
            .collect()
    }

    fn report(&self, native_pane: &str) -> Option<DeskReport> {
        let session = self.session(native_pane)?;
        let live = self.inner.lives.lock().unwrap().get(session.id()).cloned();
        let info = self.inner.pane_info(&session, live.as_deref())?;
        Some(DeskReport {
            agent: info.agent.clone().unwrap_or_else(|| "kampr".to_string()),
            status: info.agent_status,
            title: info
                .label
                .clone()
                .or_else(|| info.cmd.clone())
                .unwrap_or_else(|| session.id().to_string()),
            resume_argv: self.resume_argv(session.id()),
        })
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.inner.topology.subscribe()
    }
}

fn quoted(arg: &str) -> String {
    match arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c))
    {
        true => arg.to_string(),
        false => format!("'{}'", arg.replace('\'', r"'\''")),
    }
}

fn visible(s: &Screen) -> String {
    let end = s.first_row() + s.history_rows();
    let rows = u32::from(s.size().1);
    s.read_rows(end, end + rows - 1).unwrap_or_default().join("\n")
}

/// A session that has just ended is still being written to as it goes: the keeper takes its socket
/// down and the follower lays its last checkpoint, and a removal racing either fails part way and
/// leaves a directory nothing lists or forgets again.
fn remove_settled(path: &Path) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !path.exists() => return Ok(()),
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

pub fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

#[cfg(test)]
mod tests {
    use super::remove_settled;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn a_directory_still_being_written_to_is_removed_once_its_writers_stop() {
        let root = tempfile::tempdir().expect("a root");
        let dir = root.path().join("01ja");
        std::fs::create_dir(&dir).expect("a session");
        for n in 0..200 {
            std::fs::write(dir.join(format!("seg-{n}")), b"x").expect("a file");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let (dir, stop) = (dir.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0;
                while !stop.load(Ordering::Relaxed) {
                    let _ = std::fs::write(dir.join(format!("late-{n}")), b"x");
                    n += 1;
                }
            })
        };
        let stopper = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                stop.store(true, Ordering::Relaxed);
            })
        };
        remove_settled(&dir).expect("removed");
        stopper.join().unwrap();
        writer.join().unwrap();
        assert!(!dir.exists());
    }
}
