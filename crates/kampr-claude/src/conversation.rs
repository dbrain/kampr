use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kampr_core::conversation::abilities::{
    Abilities, AskAbility, Fact, InputAbility, Lifecycle, Steer, Stream,
};
use kampr_core::conversation::control::{Action, Control, ControlKind};
use kampr_core::conversation::facts::{Cost, Fix, Limits, RunState, SessionFacts, Task, TaskState, Trouble};
use kampr_core::conversation::{Ask, AskOption, Command, Conversation, Emit, Live, LiveTool, Reply, Update};
use kampr_journal::Journal;
use kampr_keep::{PipeDir, PipeRead, SeqReader, SessionDir, write};
use tokio::sync::mpsc;

use crate::billing::{self, Account, OverageDecision, account_allowed, overage_decision, turn_key_source_ok};
use crate::config::Config;
use crate::config::{Billing as Policy, Overage};
use crate::wire::{self, Assistant, BlockStart, ContentBlock, Line, StreamEvent};

const COMMANDS: usize = 64;

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
        abilities.input.insert(InputAbility::QueueNext);
        abilities.input.insert(InputAbility::SendNow);
        abilities.input.insert(InputAbility::Unqueue);
        abilities.steer.insert(Steer::Interrupt);
        abilities.steer.insert(Steer::Continue);
        abilities.asks.insert(AskAbility::Permission);
        abilities.asks.insert(AskAbility::Always);
        abilities.streams.insert(Stream::Text);
        abilities.streams.insert(Stream::TaskOutput);
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
                    .or_else(|| Some(existing.kind))
                    .unwrap_or("task".into()),
                backgrounded: task.is_backgrounded || existing.backgrounded,
                state: match task.status.as_deref() {
                    Some(s) if s == "completed" || s == "failed" || s == "stopped" => TaskState::Ended,
                    _ => TaskState::Running,
                },
                title: task.prompt.clone().or_else(|| existing.title),
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

        if let Some(path) = transcript {
            if let Some(page) = history_page(&path) {
                if !push(&mut emit, Update::Page(page)) {
                    return;
                }
            }
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
                        let _ = reader.wait(Duration::from_millis(100));
                    }
                    Ok(PipeRead::Truncated { .. }) => {}
                    Err(_) => break,
                }
            }
        });

        loop {
            tokio::select! {
                line = line_rx.recv() => {
                    let Some(line) = line else { break; };
                    if !process_line(&line, &mut inner, &session, &mut emit, &config, policy, overage_policy, unattended_cap, &session_id) {
                        break;
                    }
                }
                command = commands.recv() => {
                    let Some(command) = command else { continue; };
                    dispatch_command(command, &mut inner, &session, &mut emit, policy);
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
fn process_line(
    line: &str,
    inner: &mut Inner,
    _session: &SessionDir,
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
            if let Some(id) = &progress.task_id {
                if let Some(task) = inner.tasks.get_mut(id) {
                    task.title = progress.description.clone().or_else(|| task.title.clone());
                }
            }
            push(emit, Update::Tasks(inner.tasks_list()))
        }
        Line::TaskNotification(notification) => {
            if let Some(id) = &notification.task_id {
                if let Some(task) = inner.tasks.get_mut(id) {
                    task.state = TaskState::Ended;
                }
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
            }
            true
        }
        Line::User(_) | Line::ThinkingTokens { .. } | Line::Other => true,
    }
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

fn stream_event_updates(e: StreamEvent, emit: &mut Emit) -> bool {
    match e {
        StreamEvent::TextDelta { text } => push(
            emit,
            Update::Live(Live {
                text,
                ..Live::default()
            }),
        ),
        StreamEvent::ThinkingDelta { .. } => push(emit, Update::Live(Live::default())),
        StreamEvent::InputJsonDelta { partial_json } => push(
            emit,
            Update::Live(Live {
                tools: vec![LiveTool {
                    id: String::new(),
                    tool: String::new(),
                    args: partial_json,
                    state: kampr_journal::ToolState::Running,
                }],
                ..Live::default()
            }),
        ),
        StreamEvent::BlockStart {
            block: BlockStart::ToolUse { id, name },
            ..
        } => push(
            emit,
            Update::Live(Live {
                tools: vec![LiveTool {
                    id,
                    tool: name,
                    args: String::new(),
                    state: kampr_journal::ToolState::Running,
                }],
                ..Live::default()
            }),
        ),
        StreamEvent::MessageDelta { .. } | StreamEvent::BlockStart { .. } | StreamEvent::Other => true,
    }
}

/// A command from the pane, mapped to a line to the child's stdin. A command the source has no
/// word for is dropped.
fn dispatch_command(
    command: Command,
    inner: &mut Inner,
    session: &SessionDir,
    emit: &mut Emit,
    policy: Policy,
) {
    match command {
        Command::Input { text, images, .. } => {
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
            let _ = write(session, &wire::user_message(&text, &images));
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
            let _ = write(session, &wire::control_response(&id, behavior, input));
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
            let _ = write(session, &wire::control_request(&id, subtype, body));
        }
        Command::Act { action, args } => {
            let id = inner.next_request();
            let _ = write(session, &wire::control_request(&id, &action, args));
        }
        Command::Task { id, from } => {
            inner.followed.insert(id.clone());
            let rid = inner.next_request();
            let _ = write(
                session,
                &wire::control_request(
                    &rid,
                    "get_task_output",
                    serde_json::json!({ "task_id": id, "from": from }),
                ),
            );
        }
        Command::Unfollow { id } => {
            inner.followed.remove(&id);
        }
        Command::Steer(kind, text) => match kind {
            Steer::Interrupt => {
                // A turn in flight is interrupted by closing its tool; the seam has no finer word.
                push(emit, Update::State(RunState::Idle));
            }
            Steer::Continue => {
                if inner.preflight == Preflight::Allowed {
                    let _ = write(session, &wire::user_message(text.as_deref().unwrap_or(""), &[]));
                }
            }
            _ => {}
        },
        Command::Older { before } => {
            if let Some(page) = inner_transcript_page(session, before) {
                push(emit, Update::Older(page));
            }
        }
        _ => {}
    }
    let _ = policy;
}

/// The history edge: the transcript through the existing `ClaudeParser`.
fn history_page(path: &Path) -> Option<kampr_journal::Page> {
    let mut journal = kampr_journal::FileJournal::new(
        path.to_path_buf(),
        Box::new(kampr_journal::claude::ClaudeParser::default()),
        None,
    );
    let _ = journal.poll();
    Some(journal.page_before(None, 200))
}

fn inner_transcript_page(_session: &SessionDir, _before: Option<String>) -> Option<kampr_journal::Page> {
    None
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

    /// Feed lines straight through the seam's line processor and collect the `Update`s it speaks.
    fn speak(lines: &[&str], policy: Policy, overage: Overage, cap: u32) -> Vec<Update> {
        let mut inner = Inner::new();
        let updates = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let session = SessionDir::at("/tmp");
        let keep = updates.clone();
        let mut emit: Emit = Box::new(move |update: Update| {
            keep.lock().unwrap().push(update);
            true
        });
        for line in lines {
            if !process_line(
                line,
                &mut inner,
                &session,
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
        updates.lock().unwrap().clone()
    }

    fn billing_trouble<'a>(updates: &'a [Update]) -> Option<&'a String> {
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
        let updates = speak(&[INIT_KEY], Policy::Plan, Overage::Refuse, 20);
        let trouble = billing_trouble(&updates).expect("a billing trouble");
        assert!(trouble.contains("key in the environment"), "{trouble}");
        // under api billing the same turn is allowed.
        let ok = speak(&[INIT_KEY], Policy::Api, Overage::Refuse, 20);
        assert!(billing_trouble(&ok).is_none(), "{ok:?}");
    }

    // guard 3: the answer to initialize names a key; under plan the session is refused before a word.
    #[test]
    fn guard3_an_initialize_that_names_a_key_refuses_under_plan() {
        let init = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"apiKeySource":"ANTHROPIC_API_KEY","tokenSource":"claude.ai"}}}}"#;
        let updates = speak(&[init], Policy::Plan, Overage::Refuse, 20);
        let trouble = billing_trouble(&updates).expect("a billing trouble");
        assert!(trouble.contains("key in the environment"), "{trouble}");

        let plan = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"subscriptionType":"Claude Max"}}}}"#;
        let allowed = speak(&[plan], Policy::Plan, Overage::Refuse, 20);
        assert!(billing_trouble(&allowed).is_none(), "{allowed:?}");
        assert!(allowed.iter().any(|u| matches!(u, Update::Session(_))));
    }

    // guard 6: an overage mid-turn is refused by default, held for ask, allowed for allow.
    #[test]
    fn guard6_an_overage_is_refused_by_default() {
        let overage = r#"{"type":"rate_limit_event","rate_limit_event":{"is_using_overage":true,"overage_status":"active"}}"#;
        let refused = speak(&[overage], Policy::Plan, Overage::Refuse, 20);
        assert!(billing_trouble(&refused).is_some(), "{refused:?}");

        let asked = speak(&[overage], Policy::Plan, Overage::Ask, 20);
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

        let allowed = speak(&[overage], Policy::Plan, Overage::Allow, 20);
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
        let updates = speak(&lines, Policy::Plan, Overage::Refuse, 2);
        let trouble = billing_trouble(&updates).expect("the cap trouble");
        assert!(trouble.contains("self-started"), "{trouble}");
        // an operator prompt resets the count, so the same cap is not hit.
        let mixed = speak(&[task, user, task], Policy::Plan, Overage::Refuse, 2);
        assert!(billing_trouble(&mixed).is_none(), "{mixed:?}");
    }

    // guard 9: a plan session's billing is on the screen, and a key session names the key.
    #[test]
    fn guard9_the_billing_is_on_the_screen() {
        let plan_account = r#"{"type":"control_response","response":{"subtype":"success","request_id":"k1","response":{"account":{"subscriptionType":"Claude Max"}}}}"#;
        let plan = speak(&[INIT_PLAN, plan_account], Policy::Plan, Overage::Refuse, 20);
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
        let key = speak(&[INIT_KEY, key_account], Policy::Any, Overage::Refuse, 20);
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
        let updates = speak(&[INIT_PLAN], Policy::Plan, Overage::Refuse, 20);
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
        let updates = speak(&[assistant, result], Policy::Any, Overage::Refuse, 20);
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

    // a failing turn's result says success; the assistant's own error is what the pane is told.
    #[test]
    fn a_failing_turn_is_a_trouble_not_an_answer() {
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"error":"API Error: 401 authentication_error"}"#;
        let updates = speak(&[failed], Policy::Any, Overage::Refuse, 20);
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Trouble(Some(t)) if t.kind == "api")),
            "{updates:?}"
        );
    }
}
