use crate::client::{self, Client, Socket};
use crate::live::Live;
use crate::mapper::{self, Workers};
use crate::wire::{PermissionPrompt, ServerFrame, StatusSnapshot, TranscriptEntry, TranscriptPage};
use kampr_journal::facet::{Running, Title, TitleSource};
use kampr_journal::{ConvoFound, Facets, Page, Turn};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::debug;

pub const PAGE: u32 = 100;
const READ_ALL: u32 = 500;
const HANDSHAKE: Duration = Duration::from_secs(10);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const LAST_RETRY: Duration = Duration::from_secs(5);
// A worker can't be attached (kampr.md §6), so an open one is polled.
const WORKER_POLL: Duration = Duration::from_millis(1500);
const CAPABILITIES: &[&str] = &[
    "input",
    "interrupt",
    "history",
    "status",
    "subscribe",
    "permissions",
];

#[derive(Debug)]
pub enum Command {
    Input { text: String, msg_id: String },
    Older { before: Option<String> },
    Find { query: String, cap: usize },
    Sub { id: String, before: Option<String> },
}

#[derive(Debug, Clone)]
pub enum Update {
    Page(Page),
    Turns(Vec<Turn>),
    Older(Page),
    Facets(Facets),
    Pending(Option<PermissionPrompt>),
    Found { query: String, found: ConvoFound },
    SubPage { id: String, page: Page },
    SubTurns { id: String, turns: Vec<Turn> },
    Missing(String),
    Failed(String),
}

/// A transcript as far as it has been read, and the turns the client has been sent from it.
#[derive(Default)]
struct Transcript {
    entries: BTreeMap<u64, TranscriptEntry>,
    watermark: Option<u64>,
    has_older: bool,
    sent: HashMap<String, Turn>,
}

struct Changes {
    added: Vec<Turn>,
    revised: Vec<Turn>,
}

impl Transcript {
    fn absorb(&mut self, page: TranscriptPage, older: bool) {
        if older || self.entries.is_empty() {
            self.has_older = page.has_older;
        }
        for entry in page.transcript {
            self.watermark = self.watermark.max(Some(entry.seq));
            self.entries.insert(entry.seq, entry);
        }
        self.watermark = self.watermark.max(page.watermark);
    }

    fn turns(&self, workers: &Workers) -> Vec<Turn> {
        mapper::turns(self.entries.values(), workers)
    }

    fn cursor(&self) -> Option<String> {
        self.has_older
            .then(|| self.entries.keys().next().map(|seq| mapper::turn_id(*seq)))
            .flatten()
    }

    fn oldest(&self) -> Option<u64> {
        self.entries.keys().next().copied()
    }

    fn page(&mut self, workers: &Workers) -> Page {
        let turns = self.turns(workers);
        self.sent = turns.iter().map(|t| (t.id.clone(), t.clone())).collect();
        Page {
            turns,
            cursor: self.cursor(),
            more: self.has_older,
        }
    }

    /// What moved since the client was last sent anything, and the withdrawals of what is gone.
    fn changes(&mut self, workers: &Workers) -> Changes {
        let turns = self.turns(workers);
        let mut added = Vec::new();
        let mut revised = Vec::new();
        for turn in &turns {
            match self.sent.get(&turn.id) {
                Some(held) if held == turn => {}
                Some(_) => revised.push(turn.clone()),
                None => added.push(turn.clone()),
            }
        }
        let gone: Vec<String> = self
            .sent
            .keys()
            .filter(|id| !turns.iter().any(|t| &t.id == *id))
            .cloned()
            .collect();
        for id in gone {
            let held = self.sent.remove(&id).expect("listed above");
            revised.push(Turn::new(id, held.role, None));
        }
        for turn in added.iter().chain(revised.iter()) {
            match turn.blocks.is_empty() {
                true => self.sent.remove(&turn.id),
                false => self.sent.insert(turn.id.clone(), turn.clone()),
            };
        }
        Changes { added, revised }
    }
}

struct Worker {
    id: String,
    transcript: Transcript,
}

pub struct Follow {
    pub client: Client,
    pub session: String,
    pub commands: mpsc::Receiver<Command>,
    /// Whether the session is live. Attaching restores a stored session (kampr.md §3), so a
    /// finished conversation is read over REST and only dialled once something makes it live.
    pub alive: Box<dyn Fn() -> bool + Send>,
    /// Bumps when the daemon's session list moves, which is when `alive` may have changed.
    pub revision: tokio::sync::watch::Receiver<u64>,
}

struct State<E> {
    client: Client,
    session: String,
    emit: E,
    transcript: Transcript,
    live: Live,
    workers: Workers,
    facets: Option<Facets>,
    pending: Option<Option<PermissionPrompt>>,
    worker: Option<Worker>,
}

impl Follow {
    /// Serves the session's conversation through `emit` until it answers `false` or the
    /// commands close.
    pub async fn run<E: FnMut(Update) -> bool + Send>(self, emit: E) {
        let Follow {
            client,
            session,
            mut commands,
            alive,
            mut revision,
        } = self;
        let mut state = State {
            client,
            session,
            emit,
            transcript: Transcript::default(),
            live: Live::default(),
            workers: Workers::default(),
            facets: None,
            pending: None,
            worker: None,
        };
        let mut retry = FIRST_RETRY;
        let mut paged = false;
        let mut socket: Option<Socket> = None;
        let mut next_try = Instant::now();
        let mut poll = tokio::time::interval(WORKER_POLL);
        loop {
            if !paged && Instant::now() >= next_try {
                match state.first_page().await {
                    Some(true) => paged = true,
                    Some(false) => return,
                    None => {
                        next_try = Instant::now() + retry;
                        retry = (retry * 2).min(LAST_RETRY);
                    }
                }
            }
            if paged && socket.is_none() && Instant::now() >= next_try && !alive() {
                next_try = Instant::now() + LAST_RETRY;
            }
            if paged && socket.is_none() && Instant::now() >= next_try {
                match state.attach().await {
                    Ok(Some(opened)) => {
                        retry = FIRST_RETRY;
                        socket = Some(opened);
                        if !state.reread().await {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(e) => {
                        debug!(session = %state.session, error = %format!("{e:#}"), "muffit attach failed");
                        next_try = Instant::now() + retry;
                        retry = (retry * 2).min(LAST_RETRY);
                    }
                }
            }
            let waiting = socket.is_none() || !paged;
            tokio::select! {
                frame = next_frame(&mut socket) => {
                    let keep = match frame {
                        Some(frame) => state.frame(frame).await,
                        None => {
                            socket = None;
                            next_try = Instant::now() + retry;
                            state.withdraw_live()
                        }
                    };
                    if !keep {
                        return;
                    }
                }
                command = commands.recv() => {
                    let Some(command) = command else {
                        return;
                    };
                    if !state.command(command, &mut socket).await {
                        return;
                    }
                }
                _ = tokio::time::sleep_until(next_try), if waiting => {}
                changed = revision.changed(), if socket.is_none() => {
                    if changed.is_err() {
                        return;
                    }
                    if alive() {
                        next_try = next_try.min(Instant::now());
                    }
                }
                _ = poll.tick(), if state.worker.is_some() => {
                    if !state.poll_worker().await {
                        return;
                    }
                }
            }
        }
    }
}

async fn next_frame(socket: &mut Option<Socket>) -> Option<ServerFrame> {
    match socket {
        Some(socket) => client::next(socket).await,
        None => std::future::pending().await,
    }
}

impl<E: FnMut(Update) -> bool + Send> State<E> {
    /// `None` when the daemon could not be read, `Some(false)` when the client is gone.
    async fn first_page(&mut self) -> Option<bool> {
        let page = match self.client.transcript(&self.session, None, None, PAGE).await {
            Ok(page) => page,
            Err(e) => {
                debug!(session = %self.session, error = %format!("{e:#}"), "reading a muffit transcript");
                return None;
            }
        };
        self.transcript.absorb(page, true);
        let page = self.transcript.page(&self.workers);
        Some((self.emit)(Update::Page(page)))
    }

    /// `Ok(None)` when the client is gone.
    async fn attach(&mut self) -> anyhow::Result<Option<Socket>> {
        let path = format!("/sessions/{}/attach", self.session);
        let mut socket = self.client.socket(&path, CAPABILITIES).await?;
        let cursor = loop {
            let frame = tokio::time::timeout(HANDSHAKE, client::next(&mut socket))
                .await
                .map_err(|_| anyhow::anyhow!("muffit sent no hello"))?
                .ok_or_else(|| anyhow::anyhow!("muffit closed the attach before its hello"))?;
            match frame {
                ServerFrame::Hello { cursor, session } => {
                    if !self.prompt(session.permission) {
                        return Ok(None);
                    }
                    if self.facets.is_none() && !self.facets(session.title.as_deref(), &[]) {
                        return Ok(None);
                    }
                    break cursor;
                }
                ServerFrame::Error { message } => anyhow::bail!("muffit refused the attach: {message}"),
                _ => {}
            }
        };
        client::send(
            &mut socket,
            &json!({ "type": "subscribe", "events_after": cursor }),
        )
        .await?;
        Ok(Some(socket))
    }

    async fn frame(&mut self, frame: ServerFrame) -> bool {
        match frame {
            ServerFrame::Content { blocks } => {
                for block in &blocks {
                    match block {
                        crate::wire::ContentBlock::Recorded { .. } => {
                            if !self.reread().await {
                                return false;
                            }
                        }
                        block => self.live.push(block),
                    }
                }
                match self.live.turn(&self.workers) {
                    Some(turn) => (self.emit)(Update::Turns(vec![turn])),
                    None => true,
                }
            }
            ServerFrame::Turn {} | ServerFrame::Lagged {} => self.reread().await,
            ServerFrame::Status { snapshot } => self.status(snapshot),
            _ => true,
        }
    }

    fn status(&mut self, snapshot: StatusSnapshot) -> bool {
        if !self.prompt(snapshot.permission.clone()) {
            return false;
        }
        if !self.facets(snapshot.title.as_deref(), &snapshot.subagents) {
            return false;
        }
        let workers = Workers::of(&snapshot.subagents);
        if workers == self.workers {
            return true;
        }
        self.workers = workers;
        self.send_changes()
    }

    fn prompt(&mut self, prompt: Option<PermissionPrompt>) -> bool {
        if self.pending.as_ref() == Some(&prompt) {
            return true;
        }
        self.pending = Some(prompt.clone());
        (self.emit)(Update::Pending(prompt))
    }

    fn facets(&mut self, title: Option<&str>, subagents: &[crate::wire::Subagent]) -> bool {
        let facets = facets(title, subagents);
        if self.facets.as_ref() == Some(&facets) {
            return true;
        }
        self.facets = Some(facets.clone());
        (self.emit)(Update::Facets(facets))
    }

    /// The transcript after the watermark: the turns it adds or revises, then the live turn it
    /// covers withdrawn.
    async fn reread(&mut self) -> bool {
        loop {
            let after = self.transcript.watermark;
            let page = match after {
                Some(_) => self.client.transcript(&self.session, None, after, READ_ALL).await,
                None => self.client.transcript(&self.session, None, None, READ_ALL).await,
            };
            match page {
                Ok(page) => {
                    let full = page.transcript.len() >= READ_ALL as usize && after.is_some();
                    self.transcript.absorb(page, false);
                    if !full {
                        break;
                    }
                }
                Err(e) => {
                    debug!(session = %self.session, error = %format!("{e:#}"), "re-reading a muffit transcript");
                    return true;
                }
            }
        }
        self.send_changes() && self.withdraw_live()
    }

    fn send_changes(&mut self) -> bool {
        let changes = self.transcript.changes(&self.workers);
        let mut turns = changes.revised;
        turns.extend(changes.added);
        turns.sort_by_key(|t| mapper::seq_of(&t.id));
        turns.is_empty() || (self.emit)(Update::Turns(turns))
    }

    fn withdraw_live(&mut self) -> bool {
        match self.live.reset() {
            Some(withdrawn) => (self.emit)(Update::Turns(vec![withdrawn])),
            None => true,
        }
    }

    async fn command(&mut self, command: Command, socket: &mut Option<Socket>) -> bool {
        match command {
            Command::Input { text, msg_id } => {
                let frame = json!({ "type": "input", "text": text, "msg_id": msg_id });
                // Typing into a finished conversation is what restores it (docs/18), and over the
                // socket the message keeps its msg_id.
                if socket.is_none() {
                    match self.attach().await {
                        Ok(Some(opened)) => {
                            *socket = Some(opened);
                            if !self.reread().await {
                                return false;
                            }
                        }
                        Ok(None) => return false,
                        Err(e) => {
                            debug!(session = %self.session, error = %format!("{e:#}"), "attaching to send")
                        }
                    }
                }
                let sent = match socket.as_mut() {
                    Some(socket) => client::send(socket, &frame).await,
                    None => self.client.input(&self.session, &text).await,
                };
                match sent {
                    Ok(()) => true,
                    Err(e) => (self.emit)(Update::Failed(format!("muffit did not take the message: {e:#}"))),
                }
            }
            Command::Older { before } => self.older(before).await,
            Command::Find { query, cap } => {
                let turns = self.transcript.turns(&self.workers);
                let found = kampr_journal::search_turns(&turns, &query, cap);
                (self.emit)(Update::Found { query, found })
            }
            Command::Sub { id, before } => self.open_worker(id, before).await,
        }
    }

    async fn older(&mut self, before: Option<String>) -> bool {
        let before = match before.as_deref() {
            Some(cursor) => match mapper::seq_of(cursor) {
                Some(seq) => Some(seq),
                None => return (self.emit)(Update::Missing("that is not a cursor this pane gave".into())),
            },
            None => self.transcript.oldest(),
        };
        let page = match self.client.transcript(&self.session, before, None, PAGE).await {
            Ok(page) => page,
            Err(e) => return (self.emit)(Update::Failed(format!("muffit could not page back: {e:#}"))),
        };
        let reached = page.transcript.first().map(|e| e.seq);
        self.transcript.absorb(page, true);
        let changes = self.transcript.changes(&self.workers);
        let mut older: Vec<Turn> = changes.added;
        older.sort_by_key(|t| mapper::seq_of(&t.id));
        let cursor = self
            .transcript
            .has_older
            .then(|| reached.or(before).map(mapper::turn_id))
            .flatten();
        let page = Page {
            turns: older,
            cursor,
            more: self.transcript.has_older,
        };
        if !(self.emit)(Update::Older(page)) {
            return false;
        }
        changes.revised.is_empty() || (self.emit)(Update::Turns(changes.revised))
    }

    async fn open_worker(&mut self, id: String, before: Option<String>) -> bool {
        if !id.starts_with(&format!("{}.s", self.session)) {
            return (self.emit)(Update::Missing("no such conversation".into()));
        }
        let before = before.as_deref().and_then(mapper::seq_of);
        let page = match self.client.transcript(&id, before, None, READ_ALL).await {
            Ok(page) => page,
            Err(_) => return (self.emit)(Update::Missing("no such conversation".into())),
        };
        let mut transcript = Transcript::default();
        transcript.absorb(page, true);
        let page = transcript.page(&Workers::default());
        self.worker = Some(Worker {
            id: id.clone(),
            transcript,
        });
        (self.emit)(Update::SubPage { id, page })
    }

    async fn poll_worker(&mut self) -> bool {
        let Some(worker) = self.worker.as_mut() else {
            return true;
        };
        let after = worker.transcript.watermark;
        let Ok(page) = self.client.transcript(&worker.id, None, after, READ_ALL).await else {
            return true;
        };
        worker.transcript.absorb(page, false);
        let changes = worker.transcript.changes(&Workers::default());
        let mut turns = changes.revised;
        turns.extend(changes.added);
        turns.sort_by_key(|t| mapper::seq_of(&t.id));
        let id = worker.id.clone();
        turns.is_empty() || (self.emit)(Update::SubTurns { id, turns })
    }
}

pub fn facets(title: Option<&str>, subagents: &[crate::wire::Subagent]) -> Facets {
    Facets {
        title: title.map(str::trim).filter(|t| !t.is_empty()).map(|text| Title {
            text: text.to_string(),
            source: TitleSource::Generated,
        }),
        running: subagents
            .iter()
            .filter(|s| s.liveness != crate::wire::Liveness::Dead)
            .map(|s| Running {
                call: s.call_id.clone().unwrap_or_else(|| s.id.clone()),
                kind: "worker".into(),
                name: None,
                title: s.title.clone(),
                since: None,
            })
            .collect(),
        ..Facets::default()
    }
}
