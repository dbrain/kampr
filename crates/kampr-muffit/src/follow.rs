use crate::abilities::{self, abilities};
use crate::answers;
use crate::catalog;
use crate::client::{self, Client, Socket};
use crate::live::Live;
use crate::mapper::{self, Workers};
use crate::wire::{
    BASELINE, Image, PermissionPrompt, Question, ServerFrame, StatusSnapshot, TranscriptEntry, TranscriptPage,
};
use kampr_core::conversation::{
    Abilities, Action, Billing, ContextUse, Control, ControlKind, ControlOption, Cost, Limits,
    Live as SeamLive, LiveTool, Reply, RunState, SessionFacts, Steer, SuggestKind, Suggestion, Task, TaskEnd,
    TaskState, Trouble,
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
// An open worker is polled: before 3.5 it can't be attached, and its observe socket (kampr.md §6)
// only says when it records something, which brings the next read forward.
const WORKER_POLL: Duration = Duration::from_millis(1500);
pub(crate) const CAPABILITIES: &[&str] = &[
    "input",
    "interrupt",
    "history",
    "status",
    "subscribe",
    "permissions",
    "events",
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
    Answer {
        ask: String,
        reply: Reply,
    },
    Suggest {
        kind: SuggestKind,
        query: String,
    },
    Unqueue {
        msg_id: String,
    },
}

#[derive(Debug, Clone)]
pub enum Update {
    Abilities(Abilities),
    Page(Page),
    Turns(Vec<Turn>),
    Older(Page),
    Facets(Facets),
    Asks {
        permission: Option<PermissionPrompt>,
        question: Option<Question>,
    },
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
    Limits(Limits),
    Cost(Cost),
    Live(SeamLive),
    Suggested {
        kind: SuggestKind,
        query: String,
        items: Vec<Suggestion>,
    },
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
    pending: Option<(Option<PermissionPrompt>, Option<Question>)>,
    worker: Option<Worker>,
    worker_socket: Option<Socket>,
    version: u32,
    abilities: Abilities,
    actions: Vec<Action>,
    catalog: bool,
    limits: Option<Limits>,
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
            worker_socket: None,
            version: BASELINE,
            abilities: abilities(BASELINE),
            actions: catalog::fallback(),
            catalog: false,
            limits: None,
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
        if !(state.emit)(Update::Abilities(state.abilities.clone()))
            || !(state.emit)(Update::Actions(state.actions.clone()))
        {
            return;
        }
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
                match state.attach(false).await {
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
                frame = next_frame(&mut state.worker_socket) => {
                    let keep = match frame {
                        Some(ServerFrame::Event { .. }) => state.poll_worker().await,
                        Some(_) => true,
                        None => {
                            state.worker_socket = None;
                            true
                        }
                    };
                    if !keep {
                        return;
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
    async fn attach(&mut self, hold: bool) -> anyhow::Result<Option<Socket>> {
        let path = format!("/sessions/{}/attach", self.session);
        // Watching never holds the session: leaving can't end somebody else's (kampr.md §3). Only a
        // verb the operator chose holds it, restoring a stored one the way typing does.
        let query = (!hold).then_some("observe=true");
        let mut socket = self.client.socket(&path, query, CAPABILITIES).await?;
        let cursor = loop {
            let frame = tokio::time::timeout(HANDSHAKE, client::next(&mut socket))
                .await
                .map_err(|_| anyhow::anyhow!("muffit sent no hello"))?
                .ok_or_else(|| anyhow::anyhow!("muffit closed the attach before its hello"))?;
            match frame {
                ServerFrame::Hello {
                    version,
                    cursor,
                    session,
                } => {
                    if !self.greeted(version.unwrap_or(BASELINE))
                        || !self.prompt(session.permission, session.question)
                    {
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
        if self.version >= abilities::CATALOG {
            client::send(
                &mut socket,
                &json!({ "type": "control", "verb": "commands.list", "args": {} }),
            )
            .await?;
        }
        Ok(Some(socket))
    }

    /// The daemon's version, and what the pane may draw for it.
    fn greeted(&mut self, version: u32) -> bool {
        self.version = version;
        let abilities = abilities(version);
        if abilities == self.abilities {
            return true;
        }
        self.abilities = abilities.clone();
        (self.emit)(Update::Abilities(abilities))
    }

    /// The daemon's own record of a prompt raised and settled (wire 3.10), which can beat the
    /// next status push to the pane.
    fn event(&mut self, event: &Value) -> bool {
        let (permission, question) = self.pending.clone().unwrap_or_default();
        if let Some(raised) = event.get("PromptRaised") {
            return match serde_json::from_value::<PermissionPrompt>(raised.clone()) {
                Ok(prompt) => self.prompt(Some(prompt), question),
                Err(_) => true,
            };
        }
        let settled = event["PromptSettled"]["prompt"]["id"].as_u64();
        match (settled, &permission) {
            (Some(id), Some(standing)) if standing.id == id => self.prompt(None, question),
            _ => true,
        }
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
            ServerFrame::Event { event } => self.event(&event),
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
                    Some(held) => {
                        held.state = state;
                        if !args.is_empty() {
                            held.args = args.clone();
                        }
                    }
                    None => self.seam_live.tools.push(LiveTool {
                        id: id.clone(),
                        tool: tool.clone(),
                        args: args.clone(),
                        state,
                    }),
                }
            }
            crate::wire::ContentBlock::ToolCallDelta { call_id, tool, delta } => {
                match self.seam_live.tools.iter_mut().find(|held| held.id == *call_id) {
                    Some(held) => held.args.push_str(delta),
                    None => self.seam_live.tools.push(LiveTool {
                        id: call_id.clone(),
                        tool: tool.clone(),
                        args: delta.clone(),
                        state: kampr_journal::ToolState::Running,
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
        if !self.prompt(snapshot.permission.clone(), snapshot.question.clone()) {
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
        let tasks = tasks_of(&snapshot.subagents, &snapshot.shell_tasks);
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
        // muffit names no model window; the budget its context strategy cuts back from is the
        // size it keeps the window to. With no budget the total is 0, which the pane leaves out.
        let context = snapshot.context.as_ref().map(|c| ContextUse {
            used: c.measured_prompt_tokens.unwrap_or(0) as u64,
            total: c.pressure.as_ref().and_then(|p| p.budget).unwrap_or(0) as u64,
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
        if let Some(limits) = snapshot
            .rate_limit
            .map(limits_of)
            .filter(|limits| self.limits.as_ref() != Some(limits))
        {
            self.limits = Some(limits.clone());
            if !(self.emit)(Update::Limits(limits)) {
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
        if verb == "commands.list" {
            if ok && let Some(actions) = catalog::from_catalog(&payload) {
                self.catalog = true;
                self.actions = actions.clone();
                return (self.emit)(Update::Actions(actions));
            }
            return true;
        }
        if !ok && verb.starts_with("question.") {
            let permission = self.pending.clone().unwrap_or_default().0;
            if !self.prompt(permission, None) {
                return false;
            }
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

    fn prompt(&mut self, permission: Option<PermissionPrompt>, question: Option<Question>) -> bool {
        let pending = (permission, question);
        if self.pending.as_ref() == Some(&pending) {
            return true;
        }
        self.pending = Some(pending.clone());
        (self.emit)(Update::Asks {
            permission: pending.0,
            question: pending.1,
        })
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
                if self.catalog
                    && images.is_empty()
                    && let Some((action, args)) = catalog::slash(&self.actions, &text)
                {
                    return self.act(action, args, socket).await;
                }
                let mut frame = json!({ "type": "input", "text": text, "msg_id": msg_id });
                if !images.is_empty() {
                    frame["images"] = json!(images);
                }
                // Typing into a finished conversation is what restores it (docs/18), and over the
                // socket the message keeps its msg_id.
                match self.attached(socket, false).await {
                    Ok(true) => {}
                    Ok(false) => return false,
                    Err(e) => {
                        debug!(session = %self.session, error = %format!("{e:#}"), "attaching to send")
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
            Command::Act { action, args } => self.act(action, args, socket).await,
            Command::Answer { ask, reply } => self.answer(ask, reply, socket).await,
            Command::Suggest { kind, query } => {
                let items = match kind {
                    SuggestKind::Commands if self.catalog => catalog::suggestions(&self.actions, &query),
                    _ => Vec::new(),
                };
                (self.emit)(Update::Suggested { kind, query, items })
            }
            Command::Unqueue { msg_id } => match self.version >= abilities::VERBS {
                true => {
                    self.control_verb("queue.unqueue", json!({ "msg_id": msg_id }), socket)
                        .await
                }
                false => self.refused("queue.unqueue"),
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

    async fn act(&mut self, action: String, args: Value, socket: &mut Option<Socket>) -> bool {
        if !self.actions.iter().any(|a| a.id == action) {
            return (self.emit)(Update::Acted {
                action,
                ok: false,
                text: Some("no such action".into()),
            });
        }
        self.control_verb(catalog::verb_of(&action), args, socket).await
    }

    fn refused(&mut self, action: &str) -> bool {
        (self.emit)(Update::Acted {
            action: action.into(),
            ok: false,
            text: Some("this muffit daemon is too old for that".into()),
        })
    }

    /// An answer to whichever ask stands (`answers::reply_of` named it). Withdrawing the model's
    /// question has no REST form, so it goes over the socket.
    async fn answer(&mut self, ask: String, reply: Reply, socket: &mut Option<Socket>) -> bool {
        let (permission, question) = self.pending.clone().unwrap_or_default();
        if let (Reply::Dismiss, Some(asked)) = (&reply, &question)
            && ask == format!("q{}", asked.id)
        {
            return self
                .control_verb("question.withdraw", json!({ "id": asked.id }), socket)
                .await;
        }
        let posted = answers::post(&self.client, &self.session, question.as_ref(), &ask, &reply).await;
        match posted {
            answers::Outcome::Taken => true,
            answers::Outcome::Gone => {
                self.pending = None;
                match ask.starts_with('q') {
                    true => self.prompt(permission, None),
                    false => self.prompt(None, question),
                }
            }
            answers::Outcome::Refused(why) => (self.emit)(Update::Failed(why)),
        }
    }

    async fn steer(&mut self, kind: Steer, text: Option<String>, socket: &mut Option<Socket>) -> bool {
        let frame = match kind {
            Steer::Interrupt => Some(json!({ "type": "interrupt" })),
            Steer::Continue => Some(json!({ "type": "continue" })),
            _ => None,
        };
        if let Some(frame) = frame {
            match self.attached(socket, false).await {
                Ok(true) => {}
                Ok(false) => return false,
                Err(e) => {
                    debug!(session = %self.session, error = %format!("{e:#}"), "attaching to steer");
                    return (self.emit)(Update::Failed(format!("muffit did not take the steer: {e:#}")));
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
        // pause, resume and stop have no socket form: `handle_control` answers them `unknown verb`.
        if let Some(verb) = match kind {
            Steer::Pause => Some("pause"),
            Steer::Resume => Some("resume"),
            Steer::Stop => Some("stop"),
            _ => None,
        } {
            return match self.client.post_session(&self.session, verb, None).await {
                Ok(body) => self.acted(verb, true, body),
                Err(e) => (self.emit)(Update::Acted {
                    action: verb.into(),
                    ok: false,
                    text: Some(format!("{e:#}")),
                }),
            };
        }
        let since = match kind {
            Steer::StopTask => abilities::SHELL_TASKS,
            Steer::Rename | Steer::Clear | Steer::Fork => abilities::VERBS,
            _ => BASELINE,
        };
        if self.version < since {
            return self.refused(&format!("{kind:?}").to_lowercase());
        }
        let text = text.unwrap_or_default();
        let (verb, args) = match kind {
            Steer::Compact => ("context.compact", json!({})),
            // `job.stop` reads its id as a string ("job1"), whatever kampr.md's example shows.
            Steer::StopTask => ("job.stop", json!({ "id": text })),
            Steer::Rename => ("rename", json!({ "title": text })),
            Steer::Clear => ("clear", json!({})),
            Steer::Fork => ("fork", json!({})),
            _ => {
                return (self.emit)(Update::Acted {
                    action: format!("{kind:?}").to_lowercase(),
                    ok: false,
                    text: Some("muffit has no such steer".into()),
                });
            }
        };
        self.control_verb(verb, args, socket).await
    }

    /// A control verb on the session's socket, attached for it if need be, where the daemon answers
    /// `command_result`. Without one, only a verb the daemon also routes over REST can go.
    async fn control_verb(&mut self, verb: &str, args: Value, socket: &mut Option<Socket>) -> bool {
        let frame = json!({ "type": "control", "verb": verb, "args": args });
        if let Some(open) = socket.as_mut() {
            match client::send(open, &frame).await {
                Ok(()) => return true,
                Err(e) => {
                    debug!(session = %self.session, verb, error = %format!("{e:#}"), "control verb over the socket");
                    *socket = None;
                }
            }
        }
        match self.attached(socket, true).await {
            Ok(false) => return false,
            Ok(true) => {
                if let Some(open) = socket.as_mut()
                    && client::send(open, &frame).await.is_ok()
                {
                    return true;
                }
            }
            Err(e) => {
                debug!(session = %self.session, verb, error = %format!("{e:#}"), "attaching for a control verb")
            }
        }
        let Some((route, body)) = self.rest_form(verb, &args) else {
            return (self.emit)(Update::Acted {
                action: verb.into(),
                ok: false,
                text: Some(format!(
                    "muffit takes {verb} only on the session's socket, and this pane is not attached to it"
                )),
            });
        };
        match self.client.post_session(&self.session, route, Some(body)).await {
            Ok(body) => self.acted(verb, true, body),
            Err(e) => (self.emit)(Update::Acted {
                action: verb.into(),
                ok: false,
                text: Some(format!("{e:#}")),
            }),
        }
    }

    /// The daemon's REST route for a socket verb, where `muffit-server/src/lib.rs` has one.
    fn rest_form(&self, verb: &str, args: &Value) -> Option<(&'static str, Value)> {
        match verb {
            "rename" => Some(("title", json!({ "title": args["title"] }))),
            // The socket's fork branches from the head by default; REST must name the point.
            "fork" => {
                let from_seq = args
                    .get("to_seq")
                    .and_then(Value::as_u64)
                    .or(self.transcript.watermark)?;
                Some(("fork", json!({ "from_seq": from_seq })))
            }
            _ => None,
        }
    }

    /// The session's socket, attached if there is none: watched, or held when `hold` and the
    /// daemon will not let it be watched (a stored session). `Ok(false)` when the client is gone.
    async fn attached(&mut self, socket: &mut Option<Socket>, hold: bool) -> anyhow::Result<bool> {
        if socket.is_some() {
            return Ok(true);
        }
        let opened = match self.attach(false).await {
            Err(e) if hold => {
                debug!(session = %self.session, error = %format!("{e:#}"), "not watchable; holding it");
                self.attach(true).await?
            }
            watched => watched?,
        };
        match opened {
            Some(opened) => {
                *socket = Some(opened);
                Ok(self.reread().await)
            }
            None => Ok(false),
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
        self.worker_socket = match self.version >= abilities::SUB_LIVE {
            true => observe_worker(&self.client, &id).await,
            false => None,
        };
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

/// A worker's own observe socket (wire 3.5), which pushes each event the worker records, so
/// its transcript is re-read when it moves rather than at the next poll. `None` leaves the poll.
async fn observe_worker(daemon: &Client, id: &str) -> Option<Socket> {
    let path = format!("/sessions/{id}/attach");
    let opened = async {
        let mut socket = daemon
            .socket(&path, Some("observe=true"), &["status", "subscribe"])
            .await?;
        let cursor = loop {
            match tokio::time::timeout(HANDSHAKE, client::next(&mut socket)).await? {
                Some(ServerFrame::Hello { cursor, .. }) => break cursor,
                Some(ServerFrame::Error { message }) => anyhow::bail!("{message}"),
                Some(_) => {}
                None => anyhow::bail!("closed before its hello"),
            }
        };
        client::send(
            &mut socket,
            &json!({ "type": "subscribe", "events_after": cursor }),
        )
        .await?;
        anyhow::Ok(socket)
    };
    match opened.await {
        Ok(socket) => Some(socket),
        Err(e) => {
            debug!(worker = id, error = %format!("{e:#}"), "observing a muffit worker; polling it");
            None
        }
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

fn tasks_of(subagents: &[crate::wire::Subagent], shells: &[crate::wire::ShellTask]) -> Vec<Task> {
    let shells = shells.iter().map(|shell| {
        let running = shell.state == "running";
        Task {
            id: shell.id.clone(),
            kind: "shell".into(),
            call: None,
            parent: None,
            title: Some(shell.command.clone()).filter(|c| !c.is_empty()),
            since: None,
            backgrounded: running,
            state: if running {
                TaskState::Running
            } else {
                TaskState::Ended
            },
        }
    });
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
        .chain(shells)
        .collect()
}

fn limits_of(limit: crate::wire::RateLimit) -> Limits {
    Limits {
        plan: None,
        usage: Some(Value::from(if limit.limited {
            "rate-limited"
        } else {
            "not rate-limited"
        })),
        resets_at: limit
            .limited
            .then_some(limit.retry_after_ms)
            .flatten()
            .map(|ms| format!("in {} s", ms.div_ceil(1000))),
    }
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
        billing: Billing {
            provider: snapshot
                .billed_to
                .as_deref()
                .map(|to| to.split_once(": ").map_or(to, |(name, _)| name).to_string()),
            key: None,
            rate_limited: snapshot.rate_limit.is_some_and(|r| r.limited),
        },
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
