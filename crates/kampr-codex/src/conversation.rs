//! A Codex thread as a seam `Conversation`: one app-server process per open pane, its
//! notifications mapped to `Update`s as they arrive.

use std::collections::HashMap;
use std::sync::Arc;

use kampr_core::conversation::{
    Abilities, Ask, AskAbility, AskOption, Billing, Block, Command, Conversation, Emit, Fact, InputAbility,
    Lifecycle, Live, Notice, Page, Reply, Role, RunState, SessionFacts, Steer, Stream, Trouble, Turn, Update,
    When,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::source::CodexHerd;
use crate::wire::{Notification, Pump, Request};

pub(crate) const FIRST_PROMPT_LIMIT: std::time::Duration = std::time::Duration::from_secs(300);

/// The codex thread a pane is on: its process, its id, and the thread object the
/// server described it with.
pub struct CodexConversation {
    pump: Pump,
    thread_id: String,
    thread: Value,
    herd: Arc<CodexHerd>,
}

/// What the run loop is holding open right now.
struct Inner {
    active_turn: Option<String>,
    live: String,
    /// The turns of the in-flight turn, as their items complete.
    turn_items: Vec<Turn>,
}

impl Inner {
    fn new() -> Self {
        Self {
            active_turn: None,
            live: String::new(),
            turn_items: Vec::new(),
        }
    }
}

/// A oneshot that can sit in a `select!` loop: it resolves once, then stays ready.
struct Exit(Option<oneshot::Receiver<()>>);

impl std::future::Future for Exit {
    type Output = bool;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<bool> {
        let rx = match &mut self.0 {
            Some(rx) => rx,
            None => return std::task::Poll::Ready(false),
        };
        match std::pin::Pin::new(rx).poll(cx) {
            std::task::Poll::Ready(_) => {
                self.0 = None;
                std::task::Poll::Ready(true)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

#[async_trait::async_trait]
impl Conversation for CodexConversation {
    async fn run(self: Box<Self>, mut commands: mpsc::Receiver<Command>, mut emit: Emit) {
        let CodexConversation {
            mut pump,
            thread_id,
            thread,
            herd,
        } = *self;
        let mut inner = Inner::new();
        let mut asks: HashMap<String, Request> = HashMap::new();
        let mut notifs = pump.take_notifs();
        let mut asks_rx = pump.take_asks();
        let mut exit = Exit(Some(pump.take_exit()));

        // Register the process with the herd, so `Source::end` can reach it.
        let (ctrl, mut ctrl_rx) = mpsc::channel::<()>(1);
        herd.register(thread_id.clone(), ctrl, pump.pid());

        if !emit(Update::Abilities(abilities())) || !emit(Update::Session(session_facts(&thread_id, &thread)))
        {
            return finish(pump, &herd, &thread_id, &mut emit).await;
        }
        // The thread's history, newest first.
        if let Some(page) = history(&pump, &thread_id).await
            && !emit(Update::Page(page))
        {
            return finish(pump, &herd, &thread_id, &mut emit).await;
        }

        loop {
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    if !dispatch(command, &pump, &thread_id, &mut inner, &mut asks, &mut emit).await {
                        break;
                    }
                }
                notif = notifs.recv() => {
                    let Some(notif) = notif else { break };
                    if !on_notification(&notif, &mut inner, &mut emit) {
                        break;
                    }
                }
                ask = asks_rx.recv() => {
                    let Some(ask) = ask else { break };
                    if !on_ask(ask, &mut asks, &mut emit) {
                        break;
                    }
                }
                _ = &mut exit => {
                    let _ = emit(Update::Failed("the codex app-server exited".into()));
                    break;
                }
                _ = ctrl_rx.recv() => {
                    // `Source::end` reached us: interrupt the running turn, if any, and go.
                    if let Some(active) = inner.active_turn.clone() {
                        let _ = pump
                            .call("turn/interrupt", json!({"threadId": thread_id, "turnId": active}))
                            .await;
                    }
                    break;
                }
            }
        }

        finish(pump, &herd, &thread_id, &mut emit).await;
    }
}

/// Close the pipe, leave the herd, say the session has ended.
async fn finish(mut pump: Pump, herd: &CodexHerd, thread_id: &str, emit: &mut Emit) {
    herd.remove(thread_id);
    pump.shutdown().await;
    (emit)(Update::State(RunState::Ended));
}

/// What this source can do, as this source says it.
fn abilities() -> Abilities {
    let mut a = Abilities::default();
    a.input.insert(InputAbility::SendNow);
    a.steer.insert(Steer::Interrupt);
    a.asks.insert(AskAbility::Permission);
    a.streams.insert(Stream::Text);
    a.facts.insert(Fact::Trouble);
    a.facts.insert(Fact::Account);
    a.lifecycle.insert(Lifecycle::Create);
    a.lifecycle.insert(Lifecycle::Resume);
    a.lifecycle.insert(Lifecycle::End);
    a
}

fn session_facts(id: &str, thread: &Value) -> SessionFacts {
    SessionFacts {
        id: id.into(),
        cwd: thread.get("cwd").and_then(Value::as_str).map(String::from),
        title: thread
            .get("preview")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        model: thread.get("model").and_then(Value::as_str).map(String::from),
        mode: None,
        version: thread.get("cliVersion").and_then(Value::as_str).map(String::from),
        account: thread.get("account").and_then(Value::as_str).map(String::from),
        billing: Billing::default(),
        started: None,
    }
}

/// The thread's newest turns, as the pane's first page.
async fn history(pump: &Pump, thread_id: &str) -> Option<Page> {
    let res = pump
        .call("thread/turns/list", json!({"threadId": thread_id, "limit": 30}))
        .await
        .ok()?;
    page_from(&res)
}

/// A `thread/turns/list` result as a seam page: the codex turns, newest first, each
/// flattened to its items' turns; the cursor is the server's opaque `backwardsCursor`.
fn page_from(res: &Value) -> Option<Page> {
    let turns = res.get("data")?.as_array()?;
    if turns.is_empty() {
        return None;
    }
    let turns = turns.iter().filter_map(flatten_turn).flatten().collect();
    let more = res.get("nextCursor").and_then(Value::as_str).is_some();
    let cursor = res
        .get("backwardsCursor")
        .and_then(Value::as_str)
        .map(String::from);
    Some(Page { turns, cursor, more })
}

/// A stored codex turn, newest item first, as the seam turns it contains: its user
/// message and its finished items, in order.
fn flatten_turn(turn: &Value) -> Option<Vec<Turn>> {
    let items = turn.get("items")?.as_array()?;
    let mut out = Vec::new();
    for item in items {
        if let Some(t) = item_to_turn(item) {
            out.push(t);
        }
    }
    Some(out)
}

/// A finished item as a seam turn.
fn item_to_turn(item: &Value) -> Option<Turn> {
    match item.get("type")?.as_str()? {
        "userMessage" => {
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() {
                return None;
            }
            let mut turn = Turn::new(
                item.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .to_string(),
                Role::User,
                at_ms(item.get("startedAtMs")),
            );
            turn.blocks.push(Block::md(text));
            Some(turn)
        }
        "agentMessage" => {
            let text = item.get("text")?.as_str()?;
            if text.is_empty() {
                return None;
            }
            let mut turn = Turn::new(
                item.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .to_string(),
                Role::Assistant,
                at_ms(item.get("startedAtMs")),
            );
            turn.blocks.push(Block::md(text));
            Some(turn)
        }
        _ => None,
    }
}

/// The epoch milliseconds a server timestamp carries, as the ISO-8601 the seam uses.
fn at_ms(value: Option<&Value>) -> Option<String> {
    let ms = value?.as_u64()? as i128;
    let dt = time::OffsetDateTime::from_unix_timestamp_nanos(ms * 1_000_000).ok()?;
    dt.format(&time::format_description::well_known::Rfc3339).ok()
}

async fn dispatch(
    command: Command,
    pump: &Pump,
    thread_id: &str,
    inner: &mut Inner,
    asks: &mut HashMap<String, Request>,
    emit: &mut Emit,
) -> bool {
    match command {
        Command::Input {
            text,
            msg_id,
            images: _,
            files: _,
            when,
        } => match inner.active_turn.clone() {
            Some(active) if when == When::Now => steer_turn(pump, thread_id, &active, &text, emit).await,
            Some(_) => emit(Update::Missing(
                "a turn is running and codex has no queue behind it; interrupt it first".into(),
            )),
            None => start_turn(pump, thread_id, &text, &msg_id, emit).await,
        },
        Command::Steer(Steer::Interrupt, _) => {
            if let Some(active) = inner.active_turn.clone() {
                let _ = pump
                    .call("turn/interrupt", json!({"threadId": thread_id, "turnId": active}))
                    .await;
            }
            true
        }
        Command::Steer(_, Some(text)) => match inner.active_turn.clone() {
            Some(active) => steer_turn(pump, thread_id, &active, &text, emit).await,
            None => start_turn(pump, thread_id, &text, "steer", emit).await,
        },
        Command::Steer(_, None) => true,
        Command::Older { before } => older(pump, thread_id, before, emit).await,
        Command::Answer { ask, reply } => answer(pump, asks, ask, reply, emit).await,
        Command::Find { .. } => emit(Update::Missing("search is not wired in the codex source".into())),
        _ => emit(Update::Missing("not wired in the codex source".into())),
    }
}

async fn start_turn(pump: &Pump, thread_id: &str, text: &str, msg_id: &str, emit: &mut Emit) -> bool {
    match pump
        .call(
            "turn/start",
            json!({
                "threadId": thread_id,
                "clientUserMessageId": msg_id,
                "input": [{"type": "text", "text": text}],
            }),
        )
        .await
    {
        Ok(_) => true,
        Err(e) => emit(Update::Failed(format!("turn/start: {e}"))),
    }
}

async fn steer_turn(pump: &Pump, thread_id: &str, active: &str, text: &str, emit: &mut Emit) -> bool {
    match pump
        .call(
            "turn/steer",
            json!({
                "threadId": thread_id,
                "expectedTurnId": active,
                "input": [{"type": "text", "text": text}],
            }),
        )
        .await
    {
        Ok(_) => true,
        Err(_) => {
            // The turn may have finished between our check and this call: start a fresh one.
            start_turn(pump, thread_id, text, "steer", emit).await
        }
    }
}

async fn older(pump: &Pump, thread_id: &str, before: Option<String>, emit: &mut Emit) -> bool {
    let mut params = json!({"threadId": thread_id, "limit": 30});
    if let Some(cursor) = before {
        params["cursor"] = Value::String(cursor);
    }
    match pump.call("thread/turns/list", params).await {
        Ok(res) => match page_from(&res) {
            Some(page) => emit(Update::Older(page)),
            None => emit(Update::Missing("no older turns".into())),
        },
        Err(e) => emit(Update::Failed(format!("thread/turns/list: {e}"))),
    }
}

async fn answer(
    pump: &Pump,
    asks: &mut HashMap<String, Request>,
    ask: String,
    reply: Reply,
    emit: &mut Emit,
) -> bool {
    let Some(server) = asks.remove(&ask) else {
        return emit(Update::Missing("that ask is no longer open".into()));
    };
    // The schema leaves the reply's shape free-form (probe row 10); this is the best
    // effort, and the server's `item/commandExecutionApproval` reads it as a decision.
    let result = match reply {
        Reply::Allow { .. } => json!({"decision": "approved"}),
        Reply::Deny { message } => json!({"decision": "denied", "message": message}),
        _ => json!({"decision": "denied"}),
    };
    let ok = match pump.respond(server.id, result).await {
        Ok(()) => true,
        Err(e) => {
            return emit(Update::Failed(format!(
                "the answer did not reach the server: {e}"
            )));
        }
    };
    emit(Update::Asks(Vec::new())) && ok
}

/// The server asked us something (a permission, in the shapes the probe measured): show it.
fn on_ask(ask: Request, asks: &mut HashMap<String, Request>, emit: &mut Emit) -> bool {
    let id = ask.id.to_string();
    asks.insert(id.clone(), ask.clone());
    let question = ask
        .params
        .get("command")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .map(|v| v.as_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .or_else(|| ask.params.get("reason").and_then(Value::as_str).map(String::from))
        .or_else(|| {
            ask.params
                .get("message")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .unwrap_or_else(|| ask.method.clone());
    (emit)(Update::Asks(vec![Ask {
        id: Some(id),
        kind: AskAbility::Permission,
        by: None,
        header: None,
        question,
        multi: false,
        context: None,
        options: vec![
            AskOption {
                key: "allow".into(),
                label: "Allow".into(),
                detail: None,
                chosen: true,
                text: false,
            },
            AskOption {
                key: "deny".into(),
                label: "Deny".into(),
                detail: None,
                chosen: false,
                text: true,
            },
        ],
        fields: Vec::new(),
    }]))
}

/// A server notification, mapped to seam updates.
fn on_notification(notif: &Notification, inner: &mut Inner, emit: &mut Emit) -> bool {
    let p = &notif.params;
    match notif.method.as_str() {
        "thread/status/changed" => {
            let t = p
                .get("status")
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            (emit)(Update::State(match t {
                "active" => RunState::Running,
                "systemError" => RunState::Paused,
                _ => RunState::Idle,
            }))
        }
        "turn/started" => {
            inner.active_turn = p
                .get("turn")
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
                .map(String::from);
            inner.live.clear();
            inner.turn_items.clear();
            (emit)(Update::State(RunState::Running)) && (emit)(Update::Trouble(None))
        }
        "item/started" => {
            let item = p.get("item");
            if item.and_then(|i| i.get("type")).and_then(Value::as_str) == Some("agentMessage") {
                inner.live = item
                    .and_then(|i| i.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                (emit)(Update::Live(Live {
                    text: inner.live.clone(),
                    thinking: None,
                    tools: Vec::new(),
                }))
            } else {
                true
            }
        }
        "item/agentMessage/delta" => match p.get("delta").and_then(Value::as_str) {
            Some(delta) => {
                inner.live.push_str(delta);
                (emit)(Update::Live(Live {
                    text: inner.live.clone(),
                    thinking: None,
                    tools: Vec::new(),
                }))
            }
            None => true,
        },
        "item/completed" => {
            if let Some(item) = p.get("item")
                && let Some(turn) = item_to_turn(item)
            {
                inner.turn_items.push(turn);
            }
            true
        }
        "turn/completed" => {
            let turn = p.get("turn");
            let status = turn
                .and_then(|t| t.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("completed");
            inner.active_turn = None;
            let mut ok = true;
            if !inner.turn_items.is_empty() {
                ok = ok && (emit)(Update::Turns(std::mem::take(&mut inner.turn_items)));
            }
            ok = ok && (emit)(Update::State(RunState::Idle));
            let trouble = (status == "failed").then(|| {
                let text = turn
                    .and_then(|t| t.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("the turn failed")
                    .to_string();
                Trouble {
                    kind: "provider".into(),
                    text,
                    status: "failed".into(),
                    retry: None,
                    fix: None,
                }
            });
            (emit)(Update::Trouble(trouble)) && ok
        }
        "warning" => match p.get("message").and_then(Value::as_str) {
            Some(text) => (emit)(Update::Notice(Notice { text: text.into() })),
            None => true,
        },
        "error" => match p
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
        {
            Some(text) => (emit)(Update::Trouble(Some(Trouble {
                kind: "provider".into(),
                text: text.into(),
                status: "failed".into(),
                retry: None,
                fix: None,
            }))),
            None => true,
        },
        _ => true,
    }
}

/// The `Source`'s open side needs to build one; keep the construction in one place.
pub fn conversation(
    pump: Pump,
    thread_id: String,
    thread: Value,
    herd: Arc<CodexHerd>,
) -> Box<dyn Conversation> {
    Box::new(CodexConversation {
        pump,
        thread_id,
        thread,
        herd,
    })
}
