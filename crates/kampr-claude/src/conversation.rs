use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kampr_core::conversation::abilities::{
    Abilities, AskAbility, Fact, InputAbility, Lifecycle, Steer, Stream,
};
use kampr_core::conversation::control::{Action, Control, ControlKind};
use kampr_core::conversation::facts::{Cost, Fix, Limits, RunState, SessionFacts, Task, TaskState, Trouble};
use kampr_core::conversation::{
    Ask, AskOption, Command, Conversation, Emit, Live, LiveTool, Reply, SuggestKind, Update, When,
};
use kampr_journal::Journal;
use kampr_keep::{PipeDir, PipeRead, SeqReader, SessionDir, write};
use tokio::sync::mpsc;

use crate::billing::{self, Account, OverageDecision, account_allowed, overage_decision, turn_key_source_ok};
use crate::config::Config;
use crate::config::{Billing as Policy, Overage};
use crate::wire::{self, Assistant, BlockStart, ContentBlock, Line, StreamEvent};

const COMMANDS: usize = 64;
/// How many turns one page of the transcript holds, opening on and paging back through it.
const PAGE: usize = 200;

/// A headless Claude pane: the stream-json child under a keeper's pipes, mapped to the seam.
pub struct ClaudeConversation {
    config: Config,
    session_path: PathBuf,
    policy: Policy,
    overage_policy: Overage,
    unattended_cap: u32,
    session_id: String,
    /// A transcript to read history from, when one is on disk.
    transcript: Option<PathBuf>,
}

impl ClaudeConversation {
    pub fn new(
        config: &Config,
        session_path: PathBuf,
        session_id: String,
        transcript: Option<PathBuf>,
    ) -> Self {
        Self {
            config: config.clone(),
            session_path,
            policy: config.billing,
            overage_policy: config.overage,
            unattended_cap: config.unattended_turns,
            session_id,
            transcript,
        }
    }

    fn abilities() -> Abilities {
        let mut abilities = Abilities::default();
        abilities.input.insert(InputAbility::Images);
        abilities.input.insert(InputAbility::Files);
        abilities.input.insert(InputAbility::Slash);
        abilities.input.insert(InputAbility::Mention);
        abilities.input.insert(InputAbility::QueueNext);
        abilities.input.insert(InputAbility::SendNow);
        abilities.input.insert(InputAbility::Unqueue);
        abilities.steer.insert(Steer::Interrupt);
        abilities.steer.insert(Steer::Continue);
        abilities.asks.insert(AskAbility::Permission);
        abilities.asks.insert(AskAbility::Always);
        abilities.streams.insert(Stream::Text);
        abilities.streams.insert(Stream::TaskOutput);
        abilities.streams.insert(Stream::SubLive);
        abilities.facts.insert(Fact::Context);
        abilities.facts.insert(Fact::Cost);
        abilities.facts.insert(Fact::Trouble);
        abilities.lifecycle.insert(Lifecycle::Create);
        abilities.lifecycle.insert(Lifecycle::Resume);
        abilities.lifecycle.insert(Lifecycle::End);
        abilities
    }

    /// Actions from `initialize.commands`: a verb the source itself listed. Kampr adds none of
    /// its own.
    fn actions_from(init: &wire::Init) -> Vec<Action> {
        init.slash_commands
            .iter()
            .map(|c| Action {
                id: c.name.clone(),
                label: c.name.clone(),
                detail: c.description.clone(),
                args: Vec::new(),
                group: None,
                confirm: None,
            })
            .collect()
    }

    /// Controls from `initialize` and the model. Guard 8: a money-costing control — fast mode,
    /// and any model outside the plan — carries a confirmation.
    fn controls_from(init: &wire::Init, policy: Policy) -> Vec<Control> {
        let mut controls = vec![Control {
            id: "mode".into(),
            label: "Permission mode".into(),
            kind: ControlKind::Choice,
            value: init.permission_mode.clone().map(serde_json::Value::String),
            options: ["default", "acceptEdits", "plan", "bypassPermissions"]
                .iter()
                .map(|m| kampr_core::conversation::control::ControlOption {
                    value: (*m).to_string(),
                    label: (*m).to_string(),
                    detail: None,
                })
                .collect(),
            group: Some("session".into()),
            confirm: None,
        }];
        controls.push(Control {
            id: "fast".into(),
            label: "Fast mode".into(),
            kind: ControlKind::Toggle,
            value: Some(serde_json::Value::Bool(false)),
            options: Vec::new(),
            group: Some("billing".into()),
            // Fast mode's billing against a plan is unverified, so it is treated as paid.
            confirm: Some("fast mode may be billed outside your plan".into()),
        });
        if let Some(model) = &init.model {
            let out_of_plan = policy == Policy::Plan && !model.contains("haiku");
            controls.push(Control {
                id: "model".into(),
                label: "Model".into(),
                kind: ControlKind::Choice,
                value: Some(serde_json::Value::String(model.clone())),
                options: Vec::new(),
                group: Some("billing".into()),
                confirm: out_of_plan.then(|| "this model is billed outside your plan".into()),
            });
        }
        controls
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preflight {
    Pending,
    Allowed,
    Refused,
}

struct Inner {
    preflight: Preflight,
    init: Option<wire::Init>,
    account: Option<Account>,
    overage: bool,
    unattended: u32,
    tasks: HashMap<String, Task>,
    followed: HashSet<String>,
    pending_ask: Option<String>,
    /// The id of the `get_context_usage` in flight, if one is.
    context_request: Option<String>,
    /// The id and query of the `file_suggestions` in flight, if one is.
    suggest_request: Option<(String, String)>,
    requests: u64,
    holding: bool,
}

impl Inner {
    fn new() -> Self {
        Self {
            preflight: Preflight::Pending,
            init: None,
            account: None,
            overage: false,
            unattended: 0,
            tasks: HashMap::new(),
            followed: HashSet::new(),
            pending_ask: None,
            context_request: None,
            suggest_request: None,
            requests: 0,
            holding: false,
        }
    }

    fn next_request(&mut self) -> String {
        self.requests += 1;
        format!("k{}", self.requests)
    }

    fn upsert_task(&mut self, task: wire::TaskInfo) {
        let Some(id) = task.task_id else {
            return;
        };
        let existing = self.tasks.get(&id).cloned().unwrap_or(Task {
            id: id.clone(),
            kind: task.task_type.clone().unwrap_or_else(|| "task".into()),
            call: None,
            parent: task.parent_task_id.clone(),
            title: None,
            since: None,
            backgrounded: false,
            state: TaskState::Running,
        });
        self.tasks.insert(
            id,
            Task {
                kind: task
                    .task_type
                    .clone()
                    .or(Some(existing.kind))
                    .unwrap_or("task".into()),
                backgrounded: task.is_backgrounded || existing.backgrounded,
                state: match task.status.as_deref() {
                    Some(s) if s == "completed" || s == "failed" || s == "stopped" => TaskState::Ended,
                    _ => TaskState::Running,
                },
                title: task.prompt.clone().or(existing.title),
                ..existing
            },
        );
    }

    fn tasks_list(&self) -> Vec<Task> {
        self.tasks.values().cloned().collect()
    }
}

#[async_trait::async_trait]
impl Conversation for ClaudeConversation {
    async fn run(self: Box<Self>, mut commands: mpsc::Receiver<Command>, mut emit: Emit) {
        let ClaudeConversation {
            config,
            session_path,
            policy,
            overage_policy,
            unattended_cap,
            session_id,
            transcript,
        } = *self;
        let session = SessionDir::at(session_path);
        let mut inner = Inner::new();

        if let Some(path) = &transcript
            && let Some(page) = transcript_page(path, None)
            && !push(&mut emit, Update::Page(page))
        {
            return;
        }

        if !push(&mut emit, Update::Abilities(Self::abilities())) {
            return;
        }

        // Guard 3, before a word is sent: `initialize` reads how the session would be paid for,
        // and it costs no turn.
        let init_id = inner.next_request();
        if write(
            &session,
            &wire::control_request(&init_id, "initialize", serde_json::json!({})),
        )
        .is_err()
        {
            return;
        }

        // The reader drains the child's stdout into a channel; the keeper's log is the source of
        // truth, so a restart re-reads from the log.
        let (line_tx, mut line_rx) = mpsc::channel::<String>(COMMANDS);
        let read_path = session.path().to_path_buf();
        let reader = tokio::task::spawn_blocking(move || {
            let Ok(mut reader) = SeqReader::at(&read_path, 0) else {
                return;
            };
            loop {
                match reader.read() {
                    Ok(PipeRead::Entry(entry)) => {
                        if entry.dir == PipeDir::Out && line_tx.blocking_send(entry.line).is_err() {
                            break;
                        }
                    }
                    Ok(PipeRead::CaughtUp) => {
                        // A quiet death writes no further line, and a failed send is the only
                        // other exit: without this check the thread outlives the conversation
                        // and pins the runtime's shutdown.
                        if line_tx.is_closed() {
                            break;
                        }
                        let _ = reader.wait(Duration::from_millis(100));
                    }
                    Ok(PipeRead::Truncated { .. }) => {}
                    Err(_) => break,
                }
            }
        });

        let mut write_line = |line: &str| write(&session, line);
        loop {
            tokio::select! {
                line = line_rx.recv() => {
                    let Some(line) = line else { break; };
                    if !process_line(&line, &mut inner, &mut write_line, &mut emit, &config, policy, overage_policy, unattended_cap, &session_id) {
                        break;
                    }
                }
                command = commands.recv() => {
                    let Some(command) = command else { continue; };
                    dispatch_command(command, &mut inner, &mut write_line, &mut emit, policy, &transcript);
                }
            }
        }

        if session.exit().is_some() {
            push(&mut emit, Update::State(RunState::Ended));
        }
        reader.abort();
    }
}

/// One line of the child's stdout, mapped to seam `Update`s with the per-turn guards applied.
/// Returns `false` when a guard has ended the session.
// too_many_arguments: write_line and config are reserved for guard wiring; a params struct would hide what the line handler reads
#[allow(clippy::too_many_arguments)]
fn process_line(
    line: &str,
    inner: &mut Inner,
    write_line: &mut dyn FnMut(&str) -> std::io::Result<()>,
    emit: &mut Emit,
    _config: &Config,
    policy: Policy,
    overage_policy: Overage,
    unattended_cap: u32,
    session_id: &str,
) -> bool {
    let Some(line) = Line::parse(line) else {
        return true;
    };
    match line {
        Line::Init(init) => {
            // Guard 4: a turn's `system/init` carries `apiKeySource`; under plan, anything but
            // "none" interrupts the turn and ends the session.
            if !turn_key_source_ok(policy, init.api_key_source.as_deref()) {
                return guard4_trouble(emit, init.api_key_source.as_deref().unwrap_or("a key"));
            }
            inner.init = Some(init.clone());
            if !push(emit, Update::Actions(ClaudeConversation::actions_from(&init))) {
                return false;
            }
            if !push(
                emit,
                Update::Controls(ClaudeConversation::controls_from(&init, policy)),
            ) {
                return false;
            }
            if !push(
                emit,
                Update::Session(session_facts(
                    session_id,
                    Some(&init),
                    inner.account.as_ref(),
                    inner.overage,
                )),
            ) {
                return false;
            }
            true
        }
        Line::Status(status) => {
            let state = match status.status.as_deref() {
                Some("requesting") => RunState::Running,
                _ => RunState::Idle,
            };
            push(emit, Update::State(state))
        }
        Line::Assistant(a) => assistant_updates(a, emit),
        Line::StreamEvent(e) => stream_event_updates(e, emit),
        Line::SubagentLive(live) => sub_live_updates(live, emit),
        Line::Result(result) => {
            // Guard 7: a turn the model started is counted, and the cap pauses the session.
            if billing::is_self_started(result.origin.as_deref()) {
                inner.unattended += 1;
            } else {
                inner.unattended = 0;
            }
            if inner.unattended > unattended_cap {
                return unattended_trouble(emit, inner.unattended, unattended_cap);
            }
            if !push(
                emit,
                Update::Cost(Cost {
                    usd: result.total_cost_usd,
                    input_tokens: result.usage.input_tokens,
                    output_tokens: result.usage.output_tokens,
                }),
            ) {
                return false;
            }
            if result.is_error {
                push(
                    emit,
                    Update::Trouble(Some(api_trouble(
                        &result.error,
                        result.terminal_reason.as_deref(),
                    ))),
                );
            }
            // The turn changed what the window holds: the pane's bar follows it.
            fetch_context(inner, write_line);
            push(emit, Update::State(RunState::Idle))
        }
        Line::RateLimit(limit) => {
            inner.overage = inner.overage || limit.is_using_overage;
            let five = limit.five_hour;
            let seven = limit.seven_day;
            let resets = five
                .as_ref()
                .and_then(|w| w.resets_at.clone())
                .or_else(|| seven.as_ref().and_then(|w| w.resets_at.clone()));
            let usage = serde_json::json!({
                "five_hour": five.map(|w| w.utilization),
                "seven_day": seven.map(|w| w.utilization),
            });
            // Guard 6: an overage is refused, held for ask, or allowed.
            match overage_decision(overage_policy, limit.is_using_overage) {
                OverageDecision::Continue => {}
                OverageDecision::Refuse => {
                    return overage_trouble(emit, "refused");
                }
                OverageDecision::Ask => {
                    inner.holding = true;
                    if !push(emit, Update::Asks(vec![overage_ask()])) {
                        return false;
                    }
                }
            }
            if !push(
                emit,
                Update::Limits(Limits {
                    plan: inner.account.as_ref().and_then(|a| a.subscription_type.clone()),
                    usage: Some(usage),
                    resets_at: resets,
                }),
            ) {
                return false;
            }
            if !push(
                emit,
                Update::Session(session_facts(
                    session_id,
                    inner.init.as_ref(),
                    inner.account.as_ref(),
                    inner.overage,
                )),
            ) {
                return false;
            }
            true
        }
        Line::TaskStarted(task) | Line::TaskUpdated(task) => {
            inner.upsert_task(task);
            push(emit, Update::Tasks(inner.tasks_list()))
        }
        Line::BackgroundTasks(tasks) => {
            for task in tasks {
                inner.upsert_task(task);
            }
            push(emit, Update::Tasks(inner.tasks_list()))
        }
        Line::TaskProgress(progress) => {
            if let Some(id) = &progress.task_id
                && let Some(task) = inner.tasks.get_mut(id)
            {
                task.title = progress.description.clone().or_else(|| task.title.clone());
            }
            push(emit, Update::Tasks(inner.tasks_list()))
        }
        Line::TaskNotification(notification) => {
            if let Some(id) = &notification.task_id
                && let Some(task) = inner.tasks.get_mut(id)
            {
                task.state = TaskState::Ended;
            }
            push(emit, Update::Tasks(inner.tasks_list()))
        }
        Line::ApiRetry(retry) => push(
            emit,
            Update::Trouble(Some(Trouble {
                kind: "api".into(),
                text: retry
                    .error
                    .clone()
                    .unwrap_or_else(|| "the api is retrying".into()),
                status: "retrying".into(),
                retry: Some(kampr_core::conversation::facts::Retry {
                    attempt: retry.attempt.unwrap_or(1) as u32,
                    of: retry.max_retries.unwrap_or(3) as u32,
                    in_ms: retry.retry_delay_ms.unwrap_or(0),
                }),
                fix: None,
            })),
        ),
        Line::ControlRequest(request) => {
            // A `can_use_tool` standing in the middle of a turn: an ask the pane answers.
            inner.pending_ask = Some(request.request_id.clone());
            push(emit, Update::Asks(vec![ask_of(&request)]))
        }
        Line::ControlResponse(response) => {
            // Guard 3: the answer to `initialize` names how the session would be paid for.
            if response.request_id == "k1" {
                let account = Account::from(
                    &response
                        .body
                        .get("account")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                );
                match account_allowed(policy, &account) {
                    Ok(()) => inner.preflight = Preflight::Allowed,
                    Err(trouble) => {
                        inner.preflight = Preflight::Refused;
                        return billing_trouble(emit, trouble.text);
                    }
                }
                inner.account = Some(account);
                if !push(
                    emit,
                    Update::Session(session_facts(
                        session_id,
                        inner.init.as_ref(),
                        inner.account.as_ref(),
                        inner.overage,
                    )),
                ) {
                    return false;
                }
                // The session is open: the pane's context bar starts from a read, not a guess.
                fetch_context(inner, write_line);
                true
            } else if inner
                .context_request
                .as_deref()
                .is_some_and(|id| id == response.request_id)
            {
                inner.context_request = None;
                if response.ok
                    && let Some(ctx) = context_use(&response.body)
                    && !push(emit, Update::Context(ctx))
                {
                    return false;
                }
                true
            } else if inner
                .suggest_request
                .as_ref()
                .is_some_and(|(id, _)| id == &response.request_id)
            {
                let (_, query) = inner.suggest_request.take().expect("just matched");
                if response.ok
                    && !push(
                        emit,
                        Update::Suggested {
                            kind: SuggestKind::Files,
                            query,
                            items: file_suggestions(&response.body),
                        },
                    )
                {
                    return false;
                }
                true
            } else {
                true
            }
        }
        Line::User(_) | Line::ThinkingTokens { .. } | Line::Other => true,
    }
}

/// A context read in flight: the pane's bar is what it answers with.
fn fetch_context(inner: &mut Inner, write_line: &mut dyn FnMut(&str) -> std::io::Result<()>) {
    let id = inner.next_request();
    inner.context_request = Some(id.clone());
    let _ = write_line(&wire::control_request(
        &id,
        "get_context_usage",
        serde_json::json!({}),
    ));
}

/// The `get_context_usage` answer, in the shape the binary sends it: `totalTokens` against
/// `maxTokens`, or the older binary's `context_breakdown` (probe #668).
fn context_use(body: &serde_json::Value) -> Option<kampr_core::conversation::ContextUse> {
    let breakdown = body.get("context_breakdown");
    let total = body
        .get("maxTokens")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            breakdown
                .and_then(|b| b.get("max_tokens"))
                .and_then(serde_json::Value::as_u64)
        })?;
    let used = body
        .get("totalTokens")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            breakdown.and_then(|b| {
                b.as_object().map(|fields| {
                    fields.iter().fold(0u64, |sum, (key, value)| {
                        sum + value
                            .as_u64()
                            .filter(|_| key.ends_with("tokens") && key != "max_tokens")
                            .unwrap_or(0)
                    })
                })
            })
        })?;
    Some(kampr_core::conversation::ContextUse { used, total })
}

/// The `file_suggestions` answer: the binary's own file list for the pane's @-completion.
fn file_suggestions(body: &serde_json::Value) -> Vec<kampr_core::conversation::Suggestion> {
    let items = body
        .get("suggestions")
        .or_else(|| body.get("results"))
        .and_then(serde_json::Value::as_array);
    items
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let label = item
                        .as_str()
                        .or_else(|| item.get("path").and_then(serde_json::Value::as_str))
                        .or_else(|| item.get("name").and_then(serde_json::Value::as_str))?
                        .to_string();
                    Some(kampr_core::conversation::Suggestion {
                        id: label.clone(),
                        label,
                        detail: item
                            .get("description")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn assistant_updates(a: Assistant, emit: &mut Emit) -> bool {
    let mut live = Live::default();
    for block in &a.content {
        match block {
            ContentBlock::Text { text } => live.text.push_str(text),
            ContentBlock::ToolUse { id, name, input } => live.tools.push(LiveTool {
                id: id.clone(),
                tool: name.clone(),
                args: input.to_string(),
                state: kampr_journal::ToolState::Running,
            }),
            ContentBlock::Thinking { .. } => {}
            ContentBlock::Other => {}
        }
    }
    if !push(emit, Update::Live(live)) {
        return false;
    }
    if a.is_api_error_message || a.error.is_some() {
        push(
            emit,
            Update::Trouble(Some(Trouble {
                kind: "api".into(),
                text: a.error.clone().unwrap_or_else(|| "an api error".into()),
                status: "error".into(),
                retry: None,
                fix: None,
            })),
        )
    } else {
        true
    }
}

/// The live edge a stream event is: the pane's words while a turn writes them. `None` is an
/// event the live edge does not draw.
fn live_of(e: StreamEvent) -> Option<Live> {
    match e {
        StreamEvent::TextDelta { text } => Some(Live {
            text,
            ..Live::default()
        }),
        StreamEvent::ThinkingDelta { .. } => Some(Live::default()),
        StreamEvent::InputJsonDelta { partial_json } => Some(Live {
            tools: vec![LiveTool {
                id: String::new(),
                tool: String::new(),
                args: partial_json,
                state: kampr_journal::ToolState::Running,
            }],
            ..Live::default()
        }),
        StreamEvent::BlockStart {
            block: BlockStart::ToolUse { id, name },
            ..
        } => Some(Live {
            tools: vec![LiveTool {
                id,
                tool: name,
                args: String::new(),
                state: kampr_journal::ToolState::Running,
            }],
            ..Live::default()
        }),
        StreamEvent::MessageDelta { .. } | StreamEvent::BlockStart { .. } | StreamEvent::Other => None,
    }
}

fn stream_event_updates(e: StreamEvent, emit: &mut Emit) -> bool {
    match live_of(e) {
        Some(live) => push(emit, Update::Live(live)),
        None => true,
    }
}

/// A subagent's live edge: the same mapping as the main turn's, filed under the subagent's name
/// so nothing files a subagent's words as the parent's.
fn sub_live_updates(live: wire::SubagentLive, emit: &mut Emit) -> bool {
    let Some(id) = live.parent_tool_use_id.or(live.task_id) else {
        // Unnamed: no subagent pane to file it under, and no guess to make.
        return true;
    };
    match live_of(live.event) {
        Some(edge) => push(emit, Update::SubLive { id, live: edge }),
        None => true,
    }
}

/// A command from the pane, mapped to a line to the child's stdin. A command the source has no
/// word for is dropped.
fn dispatch_command(
    command: Command,
    inner: &mut Inner,
    write_line: &mut dyn FnMut(&str) -> std::io::Result<()>,
    emit: &mut Emit,
    policy: Policy,
    transcript: &Option<PathBuf>,
) {
    match command {
        Command::Input {
            text,
            msg_id,
            images,
            files,
            when,
        } => {
            // No word is sent until guard 3 has allowed the account.
            if inner.preflight != Preflight::Allowed {
                return;
            }
            let images: Vec<wire::Image> = images
                .into_iter()
                .map(|i| wire::Image {
                    mime: i.mime,
                    data: i.data,
                })
                .collect();
            // What the composer held is @-refs the agent reads; the pane's queue word rides on
            // the line the binary queues.
            let text = if files.is_empty() {
                text
            } else {
                let refs = files
                    .iter()
                    .map(|f| format!("@{}", f.path.display()))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{text} {refs}").trim().to_string()
            };
            let priority = match when {
                When::Now => Some("now"),
                When::Next => Some("next"),
                When::Idle => None,
            };
            let _ = write_line(&wire::user_message(&text, &images, Some(&msg_id), priority));
        }
        Command::Answer { reply, .. } => {
            let Some(id) = inner.pending_ask.take() else {
                return;
            };
            let (behavior, input) = match reply {
                Reply::Allow { input, .. } => ("allow", input.unwrap_or(serde_json::Value::Null)),
                Reply::Deny { .. } => ("deny", serde_json::Value::Null),
                _ => ("deny", serde_json::Value::Null),
            };
            let _ = write_line(&wire::control_response(&id, behavior, input));
            push(emit, Update::Asks(Vec::new()));
        }
        Command::Set { control, value } => {
            let id = inner.next_request();
            let (subtype, body) = match control.as_str() {
                "model" => ("set_model", serde_json::json!({ "model": value })),
                "mode" => ("set_permission_mode", serde_json::json!({ "mode": value })),
                "fast" => (
                    "set_fast_mode",
                    serde_json::json!({ "enabled": value.as_bool().unwrap_or(false) }),
                ),
                _ => return,
            };
            let _ = write_line(&wire::control_request(&id, subtype, body));
        }
        Command::Act { action, args } => {
            let id = inner.next_request();
            let _ = write_line(&wire::control_request(&id, &action, args));
        }
        Command::Task { id, from } => {
            inner.followed.insert(id.clone());
            let rid = inner.next_request();
            let _ = write_line(&wire::control_request(
                &rid,
                "get_task_output",
                serde_json::json!({ "task_id": id, "from": from }),
            ));
        }
        Command::Unfollow { id } => {
            inner.followed.remove(&id);
        }
        Command::Steer(kind, text) => match kind {
            Steer::Interrupt => {
                // The binary's own interrupt: it aborts the turn in flight, and the turn's
                // result line is what reports the state back.
                let id = inner.next_request();
                let _ = write_line(&wire::control_request(&id, "interrupt", serde_json::json!({})));
            }
            Steer::Continue if inner.preflight == Preflight::Allowed => {
                let _ = write_line(&wire::user_message(
                    text.as_deref().unwrap_or(""),
                    &[],
                    None,
                    None,
                ));
            }
            _ => {}
        },
        Command::Unqueue { msg_id } => {
            let id = inner.next_request();
            let _ = write_line(&wire::control_request(
                &id,
                "cancel_async_message",
                serde_json::json!({ "message_uuid": msg_id }),
            ));
        }
        Command::Suggest { kind, query } => match kind {
            SuggestKind::Commands => {
                // The catalog the session listed at open: completion the pane can do itself.
                let items = inner
                    .init
                    .as_ref()
                    .map(|init| {
                        init.slash_commands
                            .iter()
                            .filter(|c| {
                                query.is_empty()
                                    || c.name.to_ascii_lowercase().contains(&query.to_ascii_lowercase())
                            })
                            .map(|c| kampr_core::conversation::Suggestion {
                                id: c.name.clone(),
                                label: c.name.clone(),
                                detail: c.description.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                push(
                    emit,
                    Update::Suggested {
                        kind: SuggestKind::Commands,
                        query,
                        items,
                    },
                );
            }
            SuggestKind::Files => {
                let id = inner.next_request();
                let _ = write_line(&wire::control_request(
                    &id,
                    "file_suggestions",
                    serde_json::json!({ "query": query }),
                ));
                inner.suggest_request = Some((id, query));
            }
        },
        Command::Refresh(Fact::Context) => fetch_context(inner, write_line),
        Command::Refresh(_) => {}
        Command::Older { before } => {
            if let Some(path) = transcript
                && let Some(page) = transcript_page(path, before.as_deref())
            {
                push(emit, Update::Older(page));
            }
        }
        Command::Find { query, cap } => {
            if let Some(path) = transcript
                && let Some(found) = transcript_search(path, &query, cap)
            {
                push(emit, Update::Found { query, found });
            }
        }
        _ => {}
    }
    let _ = policy;
}

/// The session's transcript as a journal: the history the pane opens on, the pages it pages
/// back through and the finds it searches, all through the existing `ClaudeParser`. A
/// transcript that cannot be read is an empty one, not an error: the pane asks again later,
/// and the child is still writing the file.
fn transcript_journal(path: &Path) -> kampr_journal::FileJournal {
    let mut journal = kampr_journal::FileJournal::new(
        path.to_path_buf(),
        Box::new(kampr_journal::claude::ClaudeParser::default()),
        None,
    );
    let _ = journal.poll();
    journal
}

/// A page of the transcript, back from `before`, or from the newest turn when it is `None`.
fn transcript_page(path: &Path, before: Option<&str>) -> Option<kampr_journal::Page> {
    Some(transcript_journal(path).page_before(before, PAGE))
}

/// A search of the whole transcript.
fn transcript_search(path: &Path, query: &str, cap: usize) -> Option<kampr_journal::ConvoFound> {
    Some(transcript_journal(path).search(query, cap))
}

fn session_facts(
    session_id: &str,
    init: Option<&wire::Init>,
    account: Option<&Account>,
    overage: bool,
) -> SessionFacts {
    SessionFacts {
        id: session_id.to_string(),
        cwd: init.and_then(|i| i.cwd.clone()),
        title: None,
        model: init.and_then(|i| i.model.clone()),
        mode: init.and_then(|i| i.permission_mode.clone()),
        version: init.and_then(|i| i.version.clone()),
        account: account.and_then(|a| a.email.clone()),
        billing: billing::billing_of(account.unwrap_or(&Account::default()), overage),
        started: None,
    }
}

fn ask_of(request: &wire::ControlRequest) -> Ask {
    Ask {
        id: Some(request.request_id.clone()),
        kind: AskAbility::Permission,
        by: None,
        header: request.tool_name.clone(),
        question: request
            .tool_name
            .clone()
            .unwrap_or_else(|| "allow this tool?".into()),
        multi: false,
        context: None,
        options: [("1", "Allow"), ("2", "Always allow"), ("3", "Deny")]
            .into_iter()
            .map(|(key, label)| AskOption {
                key: key.into(),
                label: label.into(),
                detail: None,
                chosen: false,
                text: false,
            })
            .collect(),
    }
}

fn overage_ask() -> Ask {
    Ask {
        id: None,
        kind: AskAbility::Always,
        by: Some("billing".into()),
        header: Some("usage credits".into()),
        question: "this turn is running on usage credits; allow it to continue?".into(),
        multi: false,
        context: None,
        options: vec![
            AskOption {
                key: "1".into(),
                label: "Allow".into(),
                detail: None,
                chosen: false,
                text: false,
            },
            AskOption {
                key: "2".into(),
                label: "Stop".into(),
                detail: None,
                chosen: false,
                text: false,
            },
        ],
    }
}

fn push(emit: &mut Emit, update: Update) -> bool {
    (emit)(update)
}

fn guard4_trouble(emit: &mut Emit, source: &str) -> bool {
    push(
        emit,
        Update::Trouble(Some(Trouble {
            kind: "billing".into(),
            text: format!(
                "this turn is paid by a key in the environment ({source}); the policy is plan-only"
            ),
            status: "stopped".into(),
            retry: None,
            fix: Some(Fix::Billing),
        })),
    )
}

fn unattended_trouble(emit: &mut Emit, count: u32, cap: u32) -> bool {
    push(
        emit,
        Update::Trouble(Some(Trouble {
            kind: "billing".into(),
            text: format!("{count} self-started turns since your last prompt; the cap is {cap}"),
            status: "paused".into(),
            retry: None,
            fix: Some(Fix::Billing),
        })),
    )
}

fn overage_trouble(emit: &mut Emit, what: &str) -> bool {
    push(
        emit,
        Update::Trouble(Some(Trouble {
            kind: "billing".into(),
            text: format!("this turn is running on usage credits; overage is {what}"),
            status: "stopped".into(),
            retry: None,
            fix: Some(Fix::Billing),
        })),
    )
}

fn billing_trouble(emit: &mut Emit, text: String) -> bool {
    push(
        emit,
        Update::Trouble(Some(Trouble {
            kind: "billing".into(),
            text,
            status: "stopped".into(),
            retry: None,
            fix: Some(Fix::Billing),
        })),
    )
}

fn api_trouble(error: &Option<String>, terminal_reason: Option<&str>) -> Trouble {
    Trouble {
        kind: "api".into(),
        text: error
            .clone()
            .unwrap_or_else(|| terminal_reason.unwrap_or("the turn failed").to_string()),
        status: "error".into(),
        retry: None,
        fix: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::conversation::{FileRef, SuggestKind, Suggestion, When};

    /// The seam's events in order: lines from the child, commands from the pane.
    enum Event<'a> {
        Line(&'a str),
        Command(Command),
    }

    fn script(
        events: &[Event<'_>],
        policy: Policy,
        overage: Overage,
        cap: u32,
    ) -> (Vec<String>, Vec<Update>) {
        script_with(events, None, policy, overage, cap)
    }

    /// The same harness with a transcript on disk: the Older pages and the finds read it.
    fn script_with(
        events: &[Event<'_>],
        transcript: Option<PathBuf>,
        policy: Policy,
        overage: Overage,
        cap: u32,
    ) -> (Vec<String>, Vec<Update>) {
        let mut inner = Inner::new();
        // The run loop spends `k1` on `initialize` before the first line; the harness mirrors
        // it, so the ids the tests see are the ids the pane sees.
        let _ = inner.next_request();
        let updates = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let keep = updates.clone();
        let keep_lines = written.clone();
        let mut emit: Emit = Box::new(move |update: Update| {
            keep.lock().unwrap().push(update);
            true
        });
        let mut write_line = |line: &str| {
            keep_lines.lock().unwrap().push(line.to_string());
            Ok(())
        };
        for event in events {
            match event {
                Event::Line(line) => {
                    if !process_line(
                        line,
                        &mut inner,
                        &mut write_line,
                        &mut emit,
                        &Config::default(),
                        policy,
                        overage,
                        cap,
                        "s1",
                    ) {
                        break;
                    }
                }
                Event::Command(command) => dispatch_command(
                    command.clone(),
                    &mut inner,
                    &mut write_line,
                    &mut emit,
                    policy,
                    &transcript,
                ),
            }
        }
        (written.lock().unwrap().clone(), updates.lock().unwrap().clone())
    }

    /// Feed lines straight through the seam's line processor and collect the `Update`s it speaks
    /// and the stdin lines it writes.
    fn speak(lines: &[&str], policy: Policy, overage: Overage, cap: u32) -> (Vec<String>, Vec<Update>) {
        script(
            &lines.iter().map(|l| Event::Line(*l)).collect::<Vec<_>>(),
            policy,
            overage,
            cap,
        )
    }

    /// Dispatch a command straight through the seam's command dispatcher and collect the stdin
    /// lines it writes and the `Update`s it speaks.
    fn dispatch(command: Command, policy: Policy) -> (Vec<String>, Vec<Update>) {
        script(&[Event::Command(command)], policy, Overage::Refuse, 20)
    }

    fn billing_trouble(updates: &[Update]) -> Option<&String> {
        updates.iter().find_map(|u| match u {
            Update::Trouble(Some(t)) if t.fix == Some(Fix::Billing) => Some(&t.text),
            _ => None,
        })
    }

    const INIT_PLAN: &str = r#"{"type":"system","subtype":"init","session_id":"s1","apiKeySource":"none","model":"sonnet","permissionMode":"plan"}"#;
    const INIT_KEY: &str = r#"{"type":"system","subtype":"init","session_id":"s1","apiKeySource":"ANTHROPIC_API_KEY","model":"sonnet"}"#;

    // guard 4: a plan turn whose system/init names a key source is interrupted and ends.
    #[test]
    fn guard4_a_plan_turn_with_a_key_source_ends_the_session() {
        let (_, updates) = speak(&[INIT_KEY], Policy::Plan, Overage::Refuse, 20);
        let trouble = billing_trouble(&updates).expect("a billing trouble");
        assert!(trouble.contains("key in the environment"), "{trouble}");
        // under api billing the same turn is allowed.
        let (_, ok) = speak(&[INIT_KEY], Policy::Api, Overage::Refuse, 20);
        assert!(billing_trouble(&ok).is_none(), "{ok:?}");
    }

    // guard 3: the answer to initialize names a key; under plan the session is refused before a word.
    #[test]
    fn guard3_an_initialize_that_names_a_key_refuses_under_plan() {
        let init = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"apiKeySource":"ANTHROPIC_API_KEY","tokenSource":"claude.ai"}}}}"#;
        let (_, updates) = speak(&[init], Policy::Plan, Overage::Refuse, 20);
        let trouble = billing_trouble(&updates).expect("a billing trouble");
        assert!(trouble.contains("key in the environment"), "{trouble}");

        let plan = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"subscriptionType":"Claude Max"}}}}"#;
        let (_, allowed) = speak(&[plan], Policy::Plan, Overage::Refuse, 20);
        assert!(billing_trouble(&allowed).is_none(), "{allowed:?}");
        assert!(allowed.iter().any(|u| matches!(u, Update::Session(_))));
    }

    // guard 6: an overage mid-turn is refused by default, held for ask, allowed for allow.
    #[test]
    fn guard6_an_overage_is_refused_by_default() {
        let overage = r#"{"type":"rate_limit_event","rate_limit_event":{"is_using_overage":true,"overage_status":"active"}}"#;
        let (_, refused) = speak(&[overage], Policy::Plan, Overage::Refuse, 20);
        assert!(billing_trouble(&refused).is_some(), "{refused:?}");

        let (_, asked) = speak(&[overage], Policy::Plan, Overage::Ask, 20);
        assert!(
            billing_trouble(&asked).is_none(),
            "ask holds, it does not refuse: {asked:?}"
        );
        assert!(
            asked
                .iter()
                .any(|u| matches!(u, Update::Asks(a) if !a.is_empty())),
            "{asked:?}"
        );

        let (_, allowed) = speak(&[overage], Policy::Plan, Overage::Allow, 20);
        assert!(billing_trouble(&allowed).is_none(), "{allowed:?}");
    }

    // guard 7: self-started turns are counted, and the cap pauses the session.
    #[test]
    fn guard7_the_unattended_cap_pauses_the_session() {
        let task = r#"{"type":"result","subtype":"success","is_error":false,"origin":"task_notification","total_cost_usd":0.01}"#;
        let user =
            r#"{"type":"result","subtype":"success","is_error":false,"origin":"user","total_cost_usd":0.01}"#;
        // one more self-started turn than the cap.
        let lines = vec![task, task, task];
        let (_, updates) = speak(&lines, Policy::Plan, Overage::Refuse, 2);
        let trouble = billing_trouble(&updates).expect("the cap trouble");
        assert!(trouble.contains("self-started"), "{trouble}");
        // an operator prompt resets the count, so the same cap is not hit.
        let (_, mixed) = speak(&[task, user, task], Policy::Plan, Overage::Refuse, 2);
        assert!(billing_trouble(&mixed).is_none(), "{mixed:?}");
    }

    // guard 9: a plan session's billing is on the screen, and a key session names the key.
    #[test]
    fn guard9_the_billing_is_on_the_screen() {
        let plan_account = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"subscriptionType":"Claude Max"}}}}"#;
        let (_, plan) = speak(&[INIT_PLAN, plan_account], Policy::Plan, Overage::Refuse, 20);
        let session = plan
            .iter()
            .rev()
            .find_map(|u| match u {
                Update::Session(s) => Some(s),
                _ => None,
            })
            .expect("a session update");
        assert_eq!(session.billing.provider.as_deref(), Some("Claude Max"));
        assert!(session.billing.key.is_none());

        let key_account = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"apiKeySource":"ANTHROPIC_API_KEY","tokenSource":"claude.ai"}}}}"#;
        let (_, key) = speak(&[INIT_KEY, key_account], Policy::Any, Overage::Refuse, 20);
        let session = key
            .iter()
            .rev()
            .find_map(|u| match u {
                Update::Session(s) => Some(s),
                _ => None,
            })
            .expect("a session update");
        assert_eq!(session.billing.key.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    // guard 8: a money-costing control (fast mode) carries a confirmation.
    #[test]
    fn guard8_a_money_control_carries_a_confirmation() {
        let (_, updates) = speak(&[INIT_PLAN], Policy::Plan, Overage::Refuse, 20);
        let controls = updates
            .iter()
            .find_map(|u| match u {
                Update::Controls(c) => Some(c),
                _ => None,
            })
            .expect("controls");
        let fast = controls
            .iter()
            .find(|c| c.id == "fast")
            .expect("a fast mode control");
        assert!(fast.confirm.is_some(), "{fast:?}");
        let mode = controls.iter().find(|c| c.id == "mode").expect("a mode control");
        assert!(
            mode.confirm.is_none(),
            "the mode control is not a money control: {mode:?}"
        );
    }

    // the live edge: an assistant text and a result produce a Live, a Cost, and an idle State.
    #[test]
    fn the_live_edge_maps_an_assistant_and_a_result() {
        let assistant =
            r#"{"type":"assistant","message":{"model":"sonnet","content":[{"type":"text","text":"hello"}]}}"#;
        let result = r#"{"type":"result","subtype":"success","is_error":false,"origin":"user","total_cost_usd":0.012,"usage":{"input_tokens":10,"output_tokens":5}}"#;
        let (_, updates) = speak(&[assistant, result], Policy::Any, Overage::Refuse, 20);
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Live(l) if l.text == "hello")),
            "{updates:?}"
        );
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Cost(c) if (c.usd - 0.012).abs() < 1e-9)),
            "{updates:?}"
        );
        assert!(
            updates.iter().any(|u| matches!(u, Update::State(RunState::Idle))),
            "{updates:?}"
        );
    }

    // The pane's interrupt is the binary's own `interrupt` control request: it is the word the
    // child understands, and the turn's result line is what reports the state back.
    #[test]
    fn an_interrupt_steer_sends_the_interrupt_control_request() {
        let (written, updates) = dispatch(Command::Steer(Steer::Interrupt, None), Policy::Any);
        let line = written
            .iter()
            .find(|l| l.contains("control_request"))
            .expect("an interrupt control request is written to the child");
        let v: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
        assert_eq!(v["request"]["subtype"], "interrupt", "{line}");
        assert!(
            !updates.iter().any(|u| matches!(u, Update::State(_))),
            "the state comes from the turn's own result line, not a local flip: {updates:?}"
        );
    }

    // Subagent-live: the binary's `subagent_live` line is a subagent's live edge, filed under
    // the subagent's name — the parent pane does not draw it as its own words.
    #[test]
    fn a_subagent_live_line_is_a_sublive_under_the_subagents_name() {
        let live = r#"{"type":"subagent_live","parent_tool_use_id":"tu1","task_id":"t1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"digging"}}}"#;
        let (_, updates) = speak(&[live], Policy::Any, Overage::Refuse, 20);
        let (id, edge) = updates
            .iter()
            .find_map(|u| match u {
                Update::SubLive { id, live } => Some((id.clone(), live.clone())),
                _ => None,
            })
            .expect("a SubLive for the pane");
        assert_eq!(id, "tu1");
        assert_eq!(edge.text, "digging");
        assert!(
            !updates.iter().any(|u| matches!(u, Update::Live(_))),
            "a subagent's words are not the parent's: {updates:?}"
        );
    }

    // A line that names no `parent_tool_use_id` falls back to the task ledger's id, and a tool
    // start is a tool the pane can draw under the subagent.
    #[test]
    fn a_subagent_live_tool_start_falls_back_to_the_task_id() {
        let live = r#"{"type":"subagent_live","task_id":"t2","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu9","name":"Read"}}}"#;
        let (_, updates) = speak(&[live], Policy::Any, Overage::Refuse, 20);
        let (id, edge) = updates
            .iter()
            .find_map(|u| match u {
                Update::SubLive { id, live } => Some((id.clone(), live.clone())),
                _ => None,
            })
            .expect("a SubLive for the pane");
        assert_eq!(id, "t2");
        assert_eq!(edge.tools.len(), 1);
        assert_eq!(edge.tools[0].id, "tu9");
        assert_eq!(edge.tools[0].tool, "Read");
    }

    // A live edge no name carries is dropped, not guessed at.
    #[test]
    fn an_unnamed_subagent_live_line_is_dropped() {
        let live = r#"{"type":"subagent_live","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"?"}}}"#;
        let (_, updates) = speak(&[live], Policy::Any, Overage::Refuse, 20);
        assert!(updates.is_empty(), "{updates:?}");
    }

    #[test]
    fn the_source_declares_subagent_live_streaming() {
        let abilities = ClaudeConversation::abilities();
        assert!(abilities.streams.contains(&Stream::SubLive));
    }

    // Search and older turns: the session's transcript, the file the child writes, is what the
    // pane pages back through and searches.
    /// A transcript on disk: two exchanges, in the shape the child writes them.
    fn transcript_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a transcript dir");
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"type":"user","uuid":"u1","timestamp":"2025-01-01T00:00:00Z","message":{"content":"hello there agent"}}"#,
                r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2025-01-01T00:00:01Z","message":{"content":[{"type":"text","text":"general kenobi standing by"}]}}"#,
                r#"{"type":"user","uuid":"u2","timestamp":"2025-01-01T00:00:02Z","message":{"content":"what is the answer to life"}}"#,
                r#"{"type":"assistant","uuid":"a2","parentUuid":"u2","timestamp":"2025-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"forty two needles on the dial"}]}}"#,
            ]
            .join("\n")
            + "\n",
        )
        .expect("the transcript");
        (dir, path)
    }

    fn turn_text(turn: &kampr_journal::Turn) -> String {
        turn.blocks
            .iter()
            .filter_map(|b| match b {
                kampr_journal::Block::Md { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn an_older_command_pages_the_transcript() {
        let (_dir, path) = transcript_file();
        let (_, updates) = script_with(
            &[Event::Command(Command::Older { before: None })],
            Some(path),
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let page = updates
            .iter()
            .find_map(|u| match u {
                Update::Older(page) => Some(page),
                _ => None,
            })
            .expect("an Older page for the pane");
        assert!(
            page.turns
                .iter()
                .any(|t| turn_text(t).contains("hello there agent")),
            "{page:?}"
        );
        assert!(
            page.turns
                .iter()
                .any(|t| turn_text(t).contains("forty two needles")),
            "{page:?}"
        );
    }

    // The cursor a page hands back reaches the page above it: the older page holds nothing the
    // first page held, and `more` says there is still transcript above it.
    #[test]
    fn an_older_command_pages_back_from_the_cursor() {
        let dir = tempfile::tempdir().expect("a transcript dir");
        let path = dir.path().join("session.jsonl");
        // 210 exchanges: more than a page, so the first page is not the whole transcript.
        let mut lines = Vec::new();
        for i in 1..=210 {
            lines.push(format!(
                r#"{{"type":"user","uuid":"u{i}","timestamp":"2025-01-01T00:00:00Z","message":{{"content":"prompt number {i}"}}}}"#
            ));
            lines.push(format!(
                r#"{{"type":"assistant","uuid":"a{i}","parentUuid":"u{i}","timestamp":"2025-01-01T00:00:01Z","message":{{"content":[{{"type":"text","text":"answer number {i}"}}]}}}}"#
            ));
        }
        std::fs::write(&path, lines.join("\n") + "\n").expect("the transcript");

        let (_, first_updates) = script_with(
            &[Event::Command(Command::Older { before: None })],
            Some(path.clone()),
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let first = first_updates
            .iter()
            .find_map(|u| match u {
                Update::Older(page) => Some(page.clone()),
                _ => None,
            })
            .expect("the first page");
        assert!(first.more, "there is transcript above the first page: {first:?}");
        let cursor = first.cursor.clone().expect("a cursor to page back from");
        let oldest = first.turns.first().expect("a turn on the first page");

        let (_, updates) = script_with(
            &[Event::Command(Command::Older { before: Some(cursor) })],
            Some(path),
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let older = updates
            .iter()
            .find_map(|u| match u {
                Update::Older(page) => Some(page),
                _ => None,
            })
            .expect("the older page");
        assert!(
            older.turns.iter().all(|t| t.id != oldest.id),
            "the older page holds nothing the first page held: {older:?}"
        );
        assert!(
            older
                .turns
                .iter()
                .any(|t| turn_text(t).contains("prompt number 1")),
            "the older page reaches further back: {older:?}"
        );
    }

    #[test]
    fn a_find_command_searches_the_transcript() {
        let (_dir, path) = transcript_file();
        let (_, updates) = script_with(
            &[Event::Command(Command::Find {
                query: "needle".into(),
                cap: 10,
            })],
            Some(path),
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let found = updates
            .iter()
            .find_map(|u| match u {
                Update::Found { query, found } => Some((query.clone(), found.clone())),
                _ => None,
            })
            .expect("a Found for the pane");
        assert_eq!(found.0, "needle");
        assert_eq!(found.1.total, 1, "{found:?}");
        assert_eq!(found.1.hits.len(), 1, "{found:?}");
        assert!(found.1.hits[0].text.contains("needle"), "{found:?}");
    }

    // No transcript on disk: the pane is not answered with a guess.
    #[test]
    fn older_and_find_without_a_transcript_are_dropped() {
        let (_, updates) = script_with(
            &[
                Event::Command(Command::Older { before: None }),
                Event::Command(Command::Find {
                    query: "needle".into(),
                    cap: 10,
                }),
            ],
            None,
            Policy::Any,
            Overage::Refuse,
            20,
        );
        assert!(
            updates
                .iter()
                .all(|u| !matches!(u, Update::Older(_) | Update::Found { .. })),
            "{updates:?}"
        );
    }

    const INIT_ALLOWED: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"subscriptionType":"Claude Max"}}}}"#;
    const TURN_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"hi","total_cost_usd":0.01,"usage":{"input_tokens":10,"output_tokens":5}}"#;

    fn input(text: &str, when: When) -> Command {
        Command::Input {
            text: text.into(),
            msg_id: "m1".into(),
            images: Vec::new(),
            files: Vec::new(),
            when,
        }
    }

    // Context usage is a zero-cost read: the pane is told it at open, and after every turn.
    #[test]
    fn the_context_usage_is_read_at_open_and_after_a_turn() {
        let (written, _) = script(
            &[Event::Line(INIT_ALLOWED), Event::Line(TURN_RESULT)],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let asks = written.iter().filter(|l| l.contains("get_context_usage")).count();
        assert_eq!(asks, 2, "one at open, one after the turn: {written:?}");
    }

    #[test]
    fn a_context_answer_reaches_the_pane() {
        let answer = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k2","response":{"totalTokens":17172,"maxTokens":1000000}}}"#;
        let (_, updates) = script(
            &[Event::Line(INIT_ALLOWED), Event::Line(answer)],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let ctx = updates
            .iter()
            .find_map(|u| match u {
                Update::Context(c) => Some(*c),
                _ => None,
            })
            .expect("the pane is told the context use");
        assert_eq!((ctx.used, ctx.total), (17172, 1000000));
    }

    // The pre-2.1.326 binary answers `get_context_usage` with a `context_breakdown`
    // (probe #668); the pane is told the same use from it.
    #[test]
    fn a_context_answer_in_the_breakdown_shape_reaches_the_pane() {
        let answer = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k2","response":{"context_breakdown":{"input_tokens":10,"output_tokens":5,"max_tokens":200000}}}}"#;
        let (_, updates) = script(
            &[Event::Line(INIT_ALLOWED), Event::Line(answer)],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let ctx = updates
            .iter()
            .find_map(|u| match u {
                Update::Context(c) => Some(*c),
                _ => None,
            })
            .expect("the pane is told the context use");
        assert_eq!((ctx.used, ctx.total), (15, 200000));
    }

    #[test]
    fn a_context_refresh_reads_again() {
        let (written, _) = script(
            &[
                Event::Line(INIT_ALLOWED),
                Event::Command(Command::Refresh(Fact::Context)),
            ],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let asks = written.iter().filter(|l| l.contains("get_context_usage")).count();
        assert_eq!(asks, 2, "the refresh is a new read: {written:?}");
    }

    // Slash completion is local: the catalog the session listed at open.
    #[test]
    fn a_commands_suggest_completes_from_the_init_catalog() {
        let init = r#"{"type":"system","subtype":"init","session_id":"s1","apiKeySource":"none","slash_commands":[{"name":"/model","description":"pick a model"},{"name":"/compact","description":"shrink the context"}]}"#;
        let (_, updates) = script(
            &[
                Event::Line(init),
                Event::Command(Command::Suggest {
                    kind: SuggestKind::Commands,
                    query: "co".into(),
                }),
            ],
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let items = updates
            .iter()
            .find_map(|u| match u {
                Update::Suggested { items, .. } => Some(items),
                _ => None,
            })
            .expect("the pane is given the suggestions");
        assert_eq!(
            items,
            &[Suggestion {
                id: "/compact".into(),
                label: "/compact".into(),
                detail: Some("shrink the context".into()),
            }]
        );
    }

    // File completion is the binary's: a zero-cost control request, its answer the pane's list.
    #[test]
    fn a_files_suggest_asks_the_binary_and_its_answer_reaches_the_pane() {
        let answer = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k2","response":{"suggestions":[{"path":"mock.py","name":"mock.py"}]}}}"#;
        let (written, updates) = script(
            &[
                Event::Command(Command::Suggest {
                    kind: SuggestKind::Files,
                    query: "mock".into(),
                }),
                Event::Line(answer),
            ],
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let line = written
            .iter()
            .find(|l| l.contains("file_suggestions"))
            .expect("a file_suggestions request is written to the child");
        let v: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
        assert_eq!(v["request"]["query"], "mock", "{line}");
        let items = updates
            .iter()
            .find_map(|u| match u {
                Update::Suggested { items, .. } => Some(items),
                _ => None,
            })
            .expect("the pane is given the suggestions");
        assert_eq!(items.first().map(|i| i.label.as_str()), Some("mock.py"));
    }

    // Queueing: the when the pane chose rides on the user line, and unqueue is the binary's
    // own cancel.
    #[test]
    fn a_send_now_marks_the_user_line_now() {
        let (written, _) = script(
            &[Event::Line(INIT_ALLOWED), Event::Command(input("go", When::Now))],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let v: serde_json::Value = serde_json::from_str(&written[1]).expect("a user line: {written:?}");
        assert_eq!(v["priority"], "now", "{v}");
        assert_eq!(v["uuid"], "m1", "{v}");
    }

    #[test]
    fn a_queued_input_is_next_and_an_idle_one_is_plain() {
        let (written, _) = script(
            &[Event::Line(INIT_ALLOWED), Event::Command(input("go", When::Next))],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let v: serde_json::Value = serde_json::from_str(&written[1]).expect("a user line: {written:?}");
        assert_eq!(v["priority"], "next", "{v}");

        let (written, _) = script(
            &[Event::Line(INIT_ALLOWED), Event::Command(input("go", When::Idle))],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let v: serde_json::Value = serde_json::from_str(&written[1]).expect("a user line: {written:?}");
        assert!(v.get("priority").is_none(), "{v}");
    }

    #[test]
    fn an_unqueue_cancels_the_queued_message() {
        let (written, _) = script(
            &[Event::Command(Command::Unqueue { msg_id: "m9".into() })],
            Policy::Any,
            Overage::Refuse,
            20,
        );
        let v: serde_json::Value = serde_json::from_str(&written[0]).expect("a control request: {written:?}");
        assert_eq!(v["request"]["subtype"], "cancel_async_message", "{v}");
        assert_eq!(v["request"]["message_uuid"], "m9", "{v}");
    }

    // Files: what the composer held becomes @-refs the agent can read.
    #[test]
    fn input_files_become_at_refs_in_the_prompt() {
        let command = Command::Input {
            text: "read this".into(),
            msg_id: "m1".into(),
            images: Vec::new(),
            files: vec![FileRef {
                path: "/tmp/a.txt".into(),
            }],
            when: When::Idle,
        };
        let (written, _) = script(
            &[Event::Line(INIT_ALLOWED), Event::Command(command)],
            Policy::Plan,
            Overage::Refuse,
            20,
        );
        let v: serde_json::Value = serde_json::from_str(&written[1]).expect("a user line: {written:?}");
        let text = v["message"]["content"].as_str().expect("a plain-text prompt");
        assert!(text.contains("@/tmp/a.txt"), "{text}");
    }

    // a failing turn's result says success; the assistant's own error is what the pane is told.
    #[test]
    fn a_failing_turn_is_a_trouble_not_an_answer() {
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"error":"API Error: 401 authentication_error"}"#;
        let (_, updates) = speak(&[failed], Policy::Any, Overage::Refuse, 20);
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Trouble(Some(t)) if t.kind == "api")),
            "{updates:?}"
        );
    }
}
