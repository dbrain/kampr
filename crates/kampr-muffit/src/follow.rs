use crate::client::{self, Client, Socket};
use crate::live::Live;
use crate::mapper::{self, Workers};
use crate::wire::{Image, PermissionPrompt, ServerFrame, StatusSnapshot, TranscriptEntry, TranscriptPage};
use kampr_core::conversation::{
    Action, ArgSpec, ContextUse, Control, ControlKind, ControlOption, Cost, Live as SeamLive, LiveTool,
    RunState, SessionFacts, Steer, Task, TaskEnd, TaskState, Trouble,
};
use kampr_journal::facet::{Running, Title, TitleSource};
use kampr_journal::{ConvoFound, Facets, Page, Turn};
use serde_json::{Value, json};
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
pub(crate) const CAPABILITIES: &[&str] = &[
    "input",
    "interrupt",
    "history",
    "status",
    "subscribe",
    "permissions",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Input {
        text: String,
        msg_id: String,
        images: Vec<Image>,
    },
    Older {
        before: Option<String>,
    },
    Find {
        query: String,
        cap: usize,
    },
    Sub {
        id: String,
        before: Option<String>,
    },
    Steer {
        kind: Steer,
        text: Option<String>,
    },
    Set {
        control: String,
        value: Value,
    },
    Act {
        action: String,
        args: Value,
    },
    FollowTask {
        id: String,
        from: u64,
    },
    Unfollow {
        id: String,
    },
}

#[derive(Debug, Clone)]
pub enum Update {
    Page(Page),
    Turns(Vec<Turn>),
    Older(Page),
    Facets(Facets),
    Pending(Option<PermissionPrompt>),
    Found {
        query: String,
        found: ConvoFound,
    },
    SubPage {
        id: String,
        page: Page,
    },
    SubTurns {
        id: String,
        turns: Vec<Turn>,
    },
    State(RunState),
    Tasks(Vec<Task>),
    TaskOutput {
        id: String,
        at: u64,
        text: String,
        ended: Option<TaskEnd>,
    },
    Trouble(Option<Trouble>),
    Session(SessionFacts),
    Controls(Vec<Control>),
    Actions(Vec<Action>),
    Context(ContextUse),
    Cost(Cost),
    Live(SeamLive),
    Acted {
        action: String,
        ok: bool,
        text: Option<String>,
    },
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
    /// Whether the session is live. Attaching restores a finished session (kampr.md §3), so one
    /// that left the live list mid-watch is read over REST and only dialled once something makes
    /// it live.
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
    seam_live: SeamLive,
    workers: Workers,
    facets: Option<Facets>,
    pending: Option<Option<PermissionPrompt>>,
    worker: Option<Worker>,
    run_state: Option<RunState>,
    tasks: Option<Vec<Task>>,
    facts: Option<SessionFacts>,
    controls: Option<Vec<Control>>,
    model_options: Vec<ControlOption>,
    context: Option<ContextUse>,
    cost: Option<Cost>,
    followed: Option<(String, u64)>,
    trouble: Option<Trouble>,
    last: Option<StatusSnapshot>,
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
            seam_live: SeamLive::default(),
            workers: Workers::default(),
            facets: None,
            pending: None,
            worker: None,
            run_state: None,
            tasks: None,
            facts: None,
            controls: None,
            model_options: Vec::new(),
            context: None,
            cost: None,
            followed: None,
            trouble: None,
            last: None,
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
        // Watching never holds the session: leaving can't end somebody else's (kampr.md §3).
        let mut socket = self
            .client
            .socket(&path, Some("observe=true"), CAPABILITIES)
            .await?;
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
        client::send(
            &mut socket,
            &json!({ "type": "control", "verb": "models.list", "args": {} }),
        )
        .await?;
        Ok(Some(socket))
    }

    async fn frame(&mut self, frame: ServerFrame) -> bool {
        match frame {
            ServerFrame::Content { blocks } => self.content(&blocks).await,
            ServerFrame::Turn {} | ServerFrame::Lagged {} => self.reread().await,
            ServerFrame::Status { snapshot } => self.status(snapshot),
            ServerFrame::Error { message } => {
                self.trouble = Some(Trouble {
                    kind: "error".into(),
                    text: message,
                    status: "open".into(),
                    retry: None,
                    fix: None,
                });
                (self.emit)(Update::Trouble(self.trouble.clone()))
            }
            ServerFrame::CommandResult { verb, ok, payload } => self.command_result(verb, ok, payload),
            _ => true,
        }
    }

    async fn content(&mut self, blocks: &[crate::wire::ContentBlock]) -> bool {
        for block in blocks {
            match block {
                crate::wire::ContentBlock::Recorded { .. } => {
                    if !self.reread().await {
                        return false;
                    }
                }
                block => {
                    self.live.push(block);
                    self.seam_live(block);
                }
            }
        }
        let live = (self.emit)(Update::Live(self.seam_live.clone()));
        match self.live.turn(&self.workers) {
            Some(turn) => live && (self.emit)(Update::Turns(vec![turn])),
            None => live,
        }
    }

    fn seam_live(&mut self, block: &crate::wire::ContentBlock) {
        match block {
            crate::wire::ContentBlock::Text { text } => self.seam_live.text.push_str(text),
            crate::wire::ContentBlock::ToolCall {
                id,
                tool,
                args,
                status,
            } => {
                let state = match status {
                    crate::wire::ToolCallStatus::Started | crate::wire::ToolCallStatus::Running => {
                        kampr_journal::ToolState::Running
                    }
                    crate::wire::ToolCallStatus::Finished => kampr_journal::ToolState::Done,
                    crate::wire::ToolCallStatus::Failed => kampr_journal::ToolState::Error,
                    crate::wire::ToolCallStatus::Unknown => kampr_journal::ToolState::Running,
                };
                match self.seam_live.tools.iter_mut().find(|tool| tool.id == *id) {
                    Some(tool) => tool.state = state,
                    None => self.seam_live.tools.push(LiveTool {
                        id: id.clone(),
                        tool: tool.clone(),
                        args: args.clone(),
                        state,
                    }),
                }
            }
            crate::wire::ContentBlock::ToolOutputChunk { call_id, text, seq } => {
                if let Some((id, last)) = &mut self.followed
                    && call_id == id
                    && *seq >= *last
                {
                    *last = *seq;
                    (self.emit)(Update::TaskOutput {
                        id: id.clone(),
                        at: *seq,
                        text: text.clone(),
                        ended: None,
                    });
                }
            }
            crate::wire::ContentBlock::ToolResult {
                call_id,
                summary,
                exit_status,
                ..
            } => {
                if let Some((id, at)) = self.followed.as_ref().filter(|(id, _)| id == call_id).cloned() {
                    self.followed = None;
                    (self.emit)(Update::TaskOutput {
                        id,
                        at,
                        text: String::new(),
                        ended: Some(TaskEnd {
                            ok: exit_status.is_none_or(|status| status == 0),
                            summary: Some(summary.clone()),
                        }),
                    });
                }
            }
            _ => {}
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
        let workers_changed = workers != self.workers;
        self.workers = workers;
        self.last = Some(snapshot.clone());
        let state = run_state(snapshot.state);
        if self.run_state != Some(state) {
            self.run_state = Some(state);
            if !(self.emit)(Update::State(state)) {
                return false;
            }
        }
        let tasks = tasks_of(&snapshot.subagents);
        if self.tasks.as_ref() != Some(&tasks) {
            self.tasks = Some(tasks.clone());
            if !(self.emit)(Update::Tasks(tasks)) {
                return false;
            }
        }
        let facts = facts_of(&snapshot);
        if self.facts.as_ref() != Some(&facts) {
            self.facts = Some(facts.clone());
            if !(self.emit)(Update::Session(facts)) {
                return false;
            }
        }
        let context = snapshot.context.as_ref().map(|c| ContextUse {
            used: c.measured_prompt_tokens.unwrap_or(0) as u64,
            total: 0,
        });
        if self.context != context {
            self.context = context;
            if let Some(context) = context
                && !(self.emit)(Update::Context(context))
            {
                return false;
            }
        }
        let cost = Cost {
            usd: 0.0,
            input_tokens: snapshot.prompt_tokens as u64,
            output_tokens: snapshot.completion_tokens as u64,
        };
        if self.cost != Some(cost) {
            self.cost = Some(cost);
            if !(self.emit)(Update::Cost(cost)) {
                return false;
            }
        }
        self.controls(&snapshot);
        !workers_changed || self.send_changes()
    }

    fn controls(&mut self, snapshot: &StatusSnapshot) {
        let controls = vec![
            Control {
                id: "model".into(),
                label: "Model".into(),
                kind: ControlKind::Choice,
                value: snapshot.model.clone().map(Value::from),
                options: self.model_options.clone(),
                group: None,
                confirm: None,
            },
            Control {
                id: "thinking".into(),
                label: "Thinking".into(),
                kind: ControlKind::Choice,
                value: snapshot.thinking.clone().map(Value::from),
                options: ["off", "low", "medium", "high"]
                    .iter()
                    .map(|level| ControlOption {
                        value: (*level).into(),
                        label: (*level).into(),
                        detail: None,
                    })
                    .collect(),
                group: None,
                confirm: None,
            },
            Control {
                id: "mode".into(),
                label: "Plan".into(),
                kind: ControlKind::Toggle,
                value: Some(Value::from(snapshot.plan)),
                options: Vec::new(),
                group: None,
                confirm: None,
            },
            Control {
                id: "permissions".into(),
                label: "Permissions".into(),
                kind: ControlKind::Choice,
                value: snapshot.permissions.clone().map(Value::from),
                options: ["yolo", "auto", "ask-outside-project", "ask-all"]
                    .iter()
                    .map(|mode| ControlOption {
                        value: (*mode).into(),
                        label: (*mode).into(),
                        detail: None,
                    })
                    .collect(),
                group: None,
                confirm: None,
            },
        ];
        if self.controls.as_ref() != Some(&controls) {
            self.controls = Some(controls.clone());
            (self.emit)(Update::Controls(controls));
        }
    }

    fn command_result(&mut self, verb: String, ok: bool, payload: Value) -> bool {
        if verb == "models.list" {
            if ok {
                self.model_options = payload["providers"]
                    .as_array()
                    .map(|providers| {
                        providers
                            .iter()
                            .filter_map(|provider| {
                                provider["name"].as_str().map(|name| ControlOption {
                                    value: name.into(),
                                    label: name.into(),
                                    detail: provider["model"].as_str().map(str::to_string),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(snapshot) = self.last.clone() {
                    self.controls(&snapshot);
                }
            }
            return true;
        }
        let text = payload
            .as_str()
            .map(str::to_string)
            .or_else(|| payload.get("text").and_then(Value::as_str).map(str::to_string));
        (self.emit)(Update::Acted {
            action: verb,
            ok,
            text,
        })
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
        let had_live = !self.seam_live.text.is_empty() || !self.seam_live.tools.is_empty();
        self.seam_live = SeamLive::default();
        let cleared = if had_live {
            (self.emit)(Update::Live(SeamLive::default()))
        } else {
            true
        };
        match self.live.reset() {
            Some(withdrawn) => cleared && (self.emit)(Update::Turns(vec![withdrawn])),
            None => cleared,
        }
    }

    async fn command(&mut self, command: Command, socket: &mut Option<Socket>) -> bool {
        match command {
            Command::Input { text, msg_id, images } => {
                let mut frame = json!({ "type": "input", "text": text, "msg_id": msg_id });
                if !images.is_empty() {
                    frame["images"] = json!(images);
                }
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
                    None => {
                        self.client
                            .input(&self.session, &text, &images, Some(&msg_id))
                            .await
                    }
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
            Command::Steer { kind, text } => self.steer(kind, text, socket).await,
            Command::Set { control, value } => match set_verb(&control, &value) {
                Some((verb, args)) => self.control_verb(&verb, args, socket).await,
                None => (self.emit)(Update::Acted {
                    action: control,
                    ok: false,
                    text: Some("no such control".into()),
                }),
            },
            Command::Act { action, args } => match act_verb(&action, &args) {
                Some((verb, verb_args)) => self.control_verb(&verb, verb_args, socket).await,
                None => (self.emit)(Update::Acted {
                    action,
                    ok: false,
                    text: Some("no such action".into()),
                }),
            },
            Command::FollowTask { id, from } => {
                self.followed = Some((id, from));
                true
            }
            Command::Unfollow { id } => {
                if self
                    .followed
                    .as_ref()
                    .is_some_and(|(followed, _)| followed == &id)
                {
                    self.followed = None;
                }
                true
            }
        }
    }

    async fn steer(&mut self, kind: Steer, _text: Option<String>, socket: &mut Option<Socket>) -> bool {
        let frame = match kind {
            Steer::Interrupt => Some(json!({ "type": "interrupt" })),
            Steer::Continue => Some(json!({ "type": "continue" })),
            _ => None,
        };
        if let Some(frame) = frame {
            if socket.is_none() {
                match self.attach().await {
                    Ok(Some(opened)) => *socket = Some(opened),
                    Ok(None) => return false,
                    Err(e) => {
                        debug!(session = %self.session, error = %format!("{e:#}"), "attaching to steer");
                        return (self.emit)(Update::Failed(format!("muffit did not take the steer: {e:#}")));
                    }
                }
            }
            return match socket.as_mut() {
                Some(socket) => match client::send(socket, &frame).await {
                    Ok(()) => true,
                    Err(e) => (self.emit)(Update::Failed(format!("muffit did not take the steer: {e:#}"))),
                },
                None => (self.emit)(Update::Failed("no socket to steer on".into())),
            };
        }
        // pause, resume and stop have no socket form; the daemon takes them over REST only.
        let verb = match kind {
            Steer::Pause => "pause",
            Steer::Resume => "resume",
            Steer::Stop => "stop",
            Steer::Compact => "context.compact",
            _ => {
                return (self.emit)(Update::Acted {
                    action: format!("{kind:?}").to_lowercase(),
                    ok: false,
                    text: Some("muffit has no such steer".into()),
                });
            }
        };
        self.control_verb(verb, json!({}), socket).await
    }

    /// A control verb on the attached socket, where the daemon answers `command_result`, over
    /// REST when there is no socket.
    async fn control_verb(&mut self, verb: &str, args: Value, socket: &mut Option<Socket>) -> bool {
        if let Some(socket) = socket.as_mut() {
            match client::send(socket, &json!({ "type": "control", "verb": verb, "args": args })).await {
                Ok(()) => return true,
                Err(e) => {
                    debug!(session = %self.session, verb, error = %format!("{e:#}"), "control verb over the socket");
                }
            }
        }
        match self.client.control_verb(&self.session, verb, Some(args)).await {
            Ok(body) => self.acted(verb, true, body),
            Err(e) => (self.emit)(Update::Acted {
                action: verb.into(),
                ok: false,
                text: Some(format!("{e:#}")),
            }),
        }
    }

    fn acted(&mut self, verb: &str, ok: bool, body: Value) -> bool {
        let text = body
            .as_str()
            .map(str::to_string)
            .or_else(|| body.get("error").and_then(Value::as_str).map(str::to_string))
            .or_else(|| body.get("text").and_then(Value::as_str).map(str::to_string));
        (self.emit)(Update::Acted {
            action: verb.into(),
            ok,
            text,
        })
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

fn run_state(state: crate::wire::SessionState) -> RunState {
    match state {
        crate::wire::SessionState::Running
        | crate::wire::SessionState::WaitingTool
        | crate::wire::SessionState::WaitingProvider => RunState::Running,
        crate::wire::SessionState::WaitingUser => RunState::Waiting,
        crate::wire::SessionState::Paused => RunState::Paused,
        crate::wire::SessionState::Created => RunState::Idle,
        crate::wire::SessionState::Completed
        | crate::wire::SessionState::Failed
        | crate::wire::SessionState::Stopped
        | crate::wire::SessionState::Unknown => RunState::Ended,
    }
}

fn tasks_of(subagents: &[crate::wire::Subagent]) -> Vec<Task> {
    subagents
        .iter()
        .map(|subagent| {
            let ended = subagent
                .state
                .as_deref()
                .is_some_and(|state| state == "completed" || state == "failed");
            Task {
                id: subagent.id.clone(),
                kind: "worker".into(),
                call: subagent.call_id.clone(),
                parent: None,
                title: subagent.title.clone(),
                since: None,
                backgrounded: !ended,
                state: if ended {
                    TaskState::Ended
                } else {
                    TaskState::Running
                },
            }
        })
        .collect()
}

fn facts_of(snapshot: &StatusSnapshot) -> SessionFacts {
    SessionFacts {
        id: snapshot.session_id.clone(),
        cwd: snapshot.cwd.clone(),
        title: snapshot.title.clone(),
        model: snapshot.model.clone(),
        mode: Some(if snapshot.plan { "plan" } else { "normal" }.into()),
        version: None,
        account: None,
        billing: Default::default(),
        started: None,
    }
}

fn set_verb(control: &str, value: &Value) -> Option<(String, Value)> {
    match control {
        "model" => Some(("model.swap".into(), json!({ "spec": value }))),
        "thinking" => Some(("thinking.set".into(), json!({ "level": value }))),
        "mode" => Some((
            "mode.set".into(),
            json!({ "mode": if value.as_bool() == Some(true) { "plan" } else { "normal" } }),
        )),
        "permissions" => Some(("permissions.set".into(), json!({ "mode": value }))),
        _ => None,
    }
}

fn act_verb(action: &str, args: &Value) -> Option<(String, Value)> {
    match action {
        "compact" => Some(("context.compact".into(), json!({}))),
        "cost" => Some(("cost".into(), json!({}))),
        "revert" => Some(("revert".into(), json!({}))),
        "changesets" => Some(("changesets".into(), json!({}))),
        "btw" => Some((
            "btw".into(),
            json!({ "question": args.get("question").cloned().unwrap_or(Value::Null) }),
        )),
        "rewind" => Some(("rewind".into(), args.clone())),
        "context" => Some(("context".into(), args.clone())),
        _ => None,
    }
}

/// The verbs the daemon's control surface lists, as actions (kampr.md §7).
pub fn actions() -> Vec<Action> {
    vec![
        Action {
            id: "compact".into(),
            label: "Compact the context".into(),
            detail: None,
            args: Vec::new(),
            group: None,
            confirm: None,
        },
        Action {
            id: "cost".into(),
            label: "Show the cost".into(),
            detail: None,
            args: Vec::new(),
            group: None,
            confirm: None,
        },
        Action {
            id: "revert".into(),
            label: "Revert the last change".into(),
            detail: None,
            args: Vec::new(),
            group: None,
            confirm: None,
        },
        Action {
            id: "changesets".into(),
            label: "List the changesets".into(),
            detail: None,
            args: Vec::new(),
            group: None,
            confirm: None,
        },
        Action {
            id: "rewind".into(),
            label: "Rewind".into(),
            detail: Some("to a changeset or a message".to_string()),
            args: vec![
                ArgSpec {
                    name: "changeset".into(),
                    label: "Changeset".into(),
                    required: false,
                    options: Vec::new(),
                },
                ArgSpec {
                    name: "to_seq".into(),
                    label: "Message".into(),
                    required: false,
                    options: Vec::new(),
                },
            ],
            group: None,
            confirm: Some("The work after it is moved to a backup.".into()),
        },
        Action {
            id: "btw".into(),
            label: "Ask a side question".into(),
            detail: None,
            args: vec![ArgSpec {
                name: "question".into(),
                label: "Question".into(),
                required: true,
                options: Vec::new(),
            }],
            group: None,
            confirm: None,
        },
        Action {
            id: "context".into(),
            label: "Explain the context".into(),
            detail: None,
            args: vec![ArgSpec {
                name: "strategy".into(),
                label: "Strategy".into(),
                required: false,
                options: Vec::new(),
            }],
            group: None,
            confirm: None,
        },
    ]
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
