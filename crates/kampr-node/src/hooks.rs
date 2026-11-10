mod card;
mod install;
pub mod launch;
mod listen;
mod ordered;
mod route;

use crate::config::Config;
use crate::herd::HerdModel;
use crate::pending::Pending;
use crate::sessions::Sessions;
use card::{Card, Pressed};
use kampr_core::provider::AgentStatus;
use kampr_core::wire::{HooksScope, HooksState, PendingSource, ServerMsg};
use route::{Proc, Routed};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::warn;

pub use install::command;

pub struct Hooks {
    socket: PathBuf,
    exe: PathBuf,
    home: PathBuf,
    declined_at: PathBuf,
    asks: AtomicBool,
    sessions: Arc<Sessions>,
    herd: watch::Receiver<Arc<HerdModel>>,
    state: Mutex<State>,
    changed: watch::Sender<u64>,
    installs: watch::Sender<u64>,
    heard: broadcast::Sender<(String, String)>,
}

#[derive(Default)]
struct State {
    routes: HashMap<String, (String, Proc)>,
    hooked: HashMap<String, Proc>,
    asks: HashMap<String, VecDeque<Ask>>,
    live: HashMap<String, Live>,
    decided: HashMap<String, String>,
    next: u64,
}

const GONE: &str = "that question has already been answered";

struct Ask {
    id: u64,
    tool_use_id: Option<String>,
    card: Card,
    reply: oneshot::Sender<String>,
    tied: bool,
    blocked: bool,
}

impl Ask {
    fn key(&self) -> String {
        let base = match &self.tool_use_id {
            Some(tool) => tool.clone(),
            None => format!("hook-{}", self.id),
        };
        match self.card.step() {
            Some(step) => format!("{base}#{step}"),
            None => base,
        }
    }
}

struct Live {
    message: String,
    text: String,
}

impl Hooks {
    pub fn start(
        config: &Config,
        state_dir: &Path,
        sessions: Arc<Sessions>,
        herd: watch::Receiver<Arc<HerdModel>>,
    ) -> (Arc<Self>, Vec<JoinHandle<()>>) {
        let socket = match config.hooks.socket.trim() {
            "" => kampr_keep::runtime()
                .map(|dir| dir.join(socket_name(state_dir)))
                .unwrap_or_else(|e| {
                    warn!(error = %e, "no private runtime directory, so Claude's hooks are not served");
                    PathBuf::new()
                }),
            path => PathBuf::from(path),
        };
        let hooks = Self::new(config, state_dir, sessions, herd.clone(), socket);
        let mut tasks = vec![tokio::spawn(follow_herd(Arc::downgrade(&hooks), herd))];
        if !hooks.socket.as_os_str().is_empty() {
            match listen::bind(&hooks.socket) {
                Ok(listener) => tasks.push(tokio::spawn(listen::serve(Arc::downgrade(&hooks), listener))),
                Err(e) => {
                    warn!(socket = %hooks.socket.display(), error = %e, "Claude's hooks are not served")
                }
            }
        }
        (hooks, tasks)
    }

    pub fn new(
        config: &Config,
        state_dir: &Path,
        sessions: Arc<Sessions>,
        herd: watch::Receiver<Arc<HerdModel>>,
        socket: PathBuf,
    ) -> Arc<Self> {
        let declined_at = state_dir.join("claude-hooks.json");
        Arc::new(Self {
            exe: kampr_keep::own_exe().unwrap_or_else(|_| PathBuf::from("kampr")),
            home: config.journal_home(),
            asks: AtomicBool::new(!declined(&declined_at)),
            declined_at,
            sessions,
            herd,
            state: Mutex::new(State::default()),
            changed: watch::channel(0).0,
            installs: watch::channel(0).0,
            heard: broadcast::channel(64).0,
            socket,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn subscribe_installs(&self) -> watch::Receiver<u64> {
        self.installs.subscribe()
    }

    pub fn heard(&self) -> broadcast::Receiver<(String, String)> {
        self.heard.subscribe()
    }

    pub fn hooked(&self, pane: &str) -> bool {
        let claude = self.state.lock().unwrap().hooked.get(pane).cloned();
        claude.is_some_and(|c| c.alive())
    }

    pub fn question(&self, pane: &str) -> Option<Pending> {
        let state = self.state.lock().unwrap();
        let ask = state.asks.get(pane)?.front()?;
        let mut view = ask.card.view();
        view.id = Some(ask.key());
        Some(view)
    }

    pub fn decided(&self, pane: &str, id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .decided
            .get(pane)
            .is_some_and(|d| d == id)
    }

    pub fn live(&self, pane: &str) -> Option<String> {
        self.state.lock().unwrap().live.get(pane).map(|l| l.text.clone())
    }

    pub fn answer(
        &self,
        pane: &str,
        key: &str,
        id: Option<&str>,
        text: Option<&str>,
    ) -> Option<Result<(), String>> {
        self.act(pane, id, |card| card.press(key, text))
    }

    pub fn submit(&self, pane: &str, id: Option<&str>) -> Option<Result<(), String>> {
        self.act(pane, id, Card::submit)
    }

    // An `id` is a press for one particular question; one that is not standing any more must
    // decide nothing, or a late press approves a request the operator never saw.
    fn act(
        &self,
        pane: &str,
        id: Option<&str>,
        how: impl FnOnce(&mut Card) -> Result<Pressed, String>,
    ) -> Option<Result<(), String>> {
        let mut state = self.state.lock().unwrap();
        let Some(queue) = state.asks.get_mut(pane).filter(|q| !q.is_empty()) else {
            return id.map(|_| Err(GONE.into()));
        };
        let head = queue.front_mut()?;
        let shown = head.key();
        if id.is_some_and(|id| id != shown) {
            return Some(Err(GONE.into()));
        }
        let outcome = match how(&mut head.card) {
            Err(why) => return Some(Err(why)),
            Ok(Pressed::Moved) => Ok(()),
            Ok(Pressed::Decided(reply)) => {
                if let Some(ask) = queue.pop_front() {
                    let _ = ask.reply.send(reply);
                }
                state.decided.insert(pane.to_string(), shown);
                Ok(())
            }
        };
        drop(state);
        self.bump();
        Some(outcome)
    }

    pub fn launch_settings(&self) -> Option<String> {
        if self.socket.as_os_str().is_empty() || !self.asks.load(Ordering::Relaxed) {
            return None;
        }
        Some(self.settings())
    }

    fn settings(&self) -> String {
        install::settings(&command(&self.exe, &self.socket))
    }

    pub fn frame(&self, pane: &str, project: Option<&Path>, claude: Option<u32>) -> ServerMsg {
        let files = install::Files::new(&self.home, project, &self.exe, &self.socket);
        let (state, scope) = match files.status() {
            (HooksState::Absent, _)
                if claude.is_some_and(|pid| launch::started_with(pid, &self.settings())) =>
            {
                (HooksState::Current, None)
            }
            found => found,
        };
        ServerMsg::ClaudeHooks {
            pane: pane.to_string(),
            state,
            scope,
            ask: self.asks.load(Ordering::Relaxed),
            project: project.map(|p| p.display().to_string()),
        }
    }

    pub fn install(&self, project: Option<&Path>, mode: &str) -> Result<(), String> {
        let scope = match mode {
            "user" => HooksScope::User,
            "project" => HooksScope::Project,
            other => {
                return Err(format!(
                    "{other:?} is not a place hooks are installed: user or project"
                ));
            }
        };
        let files = install::Files::new(&self.home, project, &self.exe, &self.socket);
        let done = files.install(scope);
        self.installs.send_modify(|n| *n = n.wrapping_add(1));
        done
    }

    pub fn decline(&self) -> Result<(), String> {
        std::fs::write(&self.declined_at, json!({ "ask": false }).to_string())
            .map_err(|e| format!("{} could not be written: {e}", self.declined_at.display()))?;
        self.asks.store(false, Ordering::Relaxed);
        self.installs.send_modify(|n| *n = n.wrapping_add(1));
        Ok(())
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn ask(&self, pane: &str, event: &Value, card: Card) -> (u64, oneshot::Receiver<String>) {
        let (reply, answered) = oneshot::channel();
        // Only the main agent's dialog is the pane's: a subagent's is drawn nowhere on it (#636)
        // and a form's end is told by ElicitationResult, Esc included (#638).
        let tied = !card.subagent() && !card.elicits();
        // Esc fires nothing (#626), so a request that arrives on a pane already blocked must be let
        // go when it stops being blocked, with no blocked edge of its own to see first.
        let blocked =
            tied && self.herd.borrow().pane(pane).map(|p| p.agent_status) == Some(AgentStatus::Blocked);
        let mut state = self.state.lock().unwrap();
        state.next += 1;
        let id = state.next;
        state.asks.entry(pane.to_string()).or_default().push_back(Ask {
            id,
            tool_use_id: event["tool_use_id"].as_str().map(str::to_string),
            card,
            reply,
            tied,
            blocked,
        });
        drop(state);
        self.bump();
        (id, answered)
    }

    fn release(&self, pane: &str, which: impl Fn(&Ask) -> bool) {
        let mut state = self.state.lock().unwrap();
        let Some(queue) = state.asks.get_mut(pane) else {
            return;
        };
        let before = queue.len();
        queue.retain(|ask| !which(ask));
        let moved = queue.len() != before;
        drop(state);
        if moved {
            self.bump();
        }
    }

    fn quiet(&self, pane: &str) {
        let had = self.state.lock().unwrap().live.remove(pane).is_some();
        if had {
            self.bump();
        }
    }

    fn display(&self, pane: &str, event: &Value) {
        let message = event["message_id"].as_str().unwrap_or_default();
        let delta = event["delta"].as_str().unwrap_or_default();
        let mut state = self.state.lock().unwrap();
        let live = state.live.entry(pane.to_string()).or_insert_with(|| Live {
            message: message.to_string(),
            text: String::new(),
        });
        if live.message != message {
            live.message = message.to_string();
            live.text.clear();
        }
        if !live.text.is_empty() && !delta.starts_with(char::is_whitespace) && !live.text.ends_with('\n') {
            live.text.push_str("\n\n");
        }
        live.text.push_str(delta);
        drop(state);
        self.bump();
    }

    async fn handle(self: Arc<Self>, pid: Option<u32>, event: Value, stream: tokio::net::UnixStream) {
        let session = event["session_id"].as_str().map(str::to_string);
        let Some(pane) = self.clone().route(session.as_deref(), pid).await else {
            return;
        };
        match event["hook_event_name"].as_str().unwrap_or_default() {
            "PermissionRequest" => {
                let Some(card) = Card::of(&event) else { return };
                let (id, answered) = self.ask(&pane, &event, card);
                listen::hold(stream, answered).await;
                self.release(&pane, |ask| ask.id == id);
            }
            // Claude draws no form until this hook returns (#637), so it is answered with nothing
            // at once and the form is the desk's (#638).
            "Elicitation" => drop(self.ask(&pane, &event, Card::elicitation(&event))),
            "ElicitationResult" => self.release(&pane, |ask| ask.card.elicits()),
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                let tool = event["tool_use_id"].as_str();
                self.release(&pane, |ask| tool.is_some() && ask.tool_use_id.as_deref() == tool);
            }
            "UserPromptSubmit" => {
                self.release(&pane, |_| true);
                self.quiet(&pane);
                if let Some(prompt) = event["prompt"].as_str() {
                    let _ = self.heard.send((pane, prompt.to_string()));
                }
            }
            "Stop" | "StopFailure" | "SessionEnd" => {
                self.release(&pane, |_| true);
                self.quiet(&pane);
            }
            "MessageDisplay" => self.display(&pane, &event),
            _ => {}
        }
    }

    async fn route(self: Arc<Self>, session: Option<&str>, pid: Option<u32>) -> Option<String> {
        let pid = pid?;
        let herd = self.herd.borrow().clone();
        let known = session.and_then(|s| self.state.lock().unwrap().routes.get(s).cloned());
        let sessions = self.sessions.clone();
        let Routed { pane, claude } = tokio::task::spawn_blocking(move || match known {
            Some((pane, claude)) if herd.pane(&pane).is_some() && claude.holds(pid) => {
                Some(Routed { pane, claude })
            }
            _ => route::pane_of(&sessions, &herd, pid),
        })
        .await
        .ok()??;
        let mut state = self.state.lock().unwrap();
        if let Some(session) = session {
            state
                .routes
                .insert(session.to_string(), (pane.clone(), claude.clone()));
        }
        state.hooked.insert(pane.clone(), claude);
        Some(pane)
    }
}

#[cfg(test)]
pub(crate) fn unserved() -> Arc<Hooks> {
    let mut config = Config::bootstrap("hooks");
    config.muffit.enabled = Some(false);
    config.docker.enabled = Some(false);
    config.herdr.socket = "/nowhere/kampr-hooks-test.sock".into();
    config.herdr.binary = "/nowhere/kampr-hooks-test-herdr".into();
    config.native.root = "/nowhere/native".into();
    let state = Path::new("/nowhere/state");
    let sessions = Sessions::open(&config, state);
    let herd = watch::channel(Arc::new(HerdModel::default())).1;
    Hooks::new(&config, state, sessions, herd, PathBuf::new())
}

#[cfg(test)]
impl Hooks {
    pub(crate) fn asking(&self, pane: &str, event: &Value) {
        let card = Card::of(event).expect("a request");
        drop(self.ask(pane, event, card));
    }
}

pub fn pending(pane: &str, asked: Option<Pending>) -> ServerMsg {
    ServerMsg::Pending {
        pane: pane.to_string(),
        question: asked.as_ref().map(|a| a.question.clone()),
        header: asked.as_ref().and_then(|a| a.header.clone()),
        multi: asked.as_ref().is_some_and(|a| a.multi),
        context: asked.as_ref().and_then(|a| a.context.clone()),
        id: asked.as_ref().and_then(|a| a.id.clone()),
        options: asked.map(|a| a.options).unwrap_or_default(),
        source: PendingSource::Hook,
    }
}

// Every node a test suite starts is a node of this uid, and one of them must never take the
// socket the operator's own Claude sessions are pointed at.
fn socket_name(state_dir: &Path) -> String {
    if state_dir == kampr_core::local_node::default_state_dir() {
        return "hooks.sock".into();
    }
    let hash = state_dir
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("hooks-{hash:016x}.sock")
}

fn declined(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|v| v["ask"] == json!(false))
}

// Esc at the dialog fires no hook at all (#626), so the pane leaving `blocked` is the only word
// that a question answered at the desk has gone.
async fn follow_herd(hooks: std::sync::Weak<Hooks>, mut herd: watch::Receiver<Arc<HerdModel>>) {
    while herd.changed().await.is_ok() {
        let model = herd.borrow_and_update().clone();
        let Some(hooks) = hooks.upgrade() else { return };
        let mut state = hooks.state.lock().unwrap();
        let before: usize = state.asks.values().map(VecDeque::len).sum();
        for (pane, queue) in state.asks.iter_mut() {
            match model.pane(pane).map(|p| p.agent_status) {
                None => queue.clear(),
                Some(AgentStatus::Blocked) => queue
                    .iter_mut()
                    .filter(|ask| ask.tied)
                    .for_each(|ask| ask.blocked = true),
                Some(_) => queue.retain(|ask| !ask.blocked),
            }
        }
        state.asks.retain(|_, queue| !queue.is_empty());
        let after: usize = state.asks.values().map(VecDeque::len).sum();
        let gone: Vec<String> = state
            .hooked
            .keys()
            .filter(|pane| model.pane(pane).is_none())
            .cloned()
            .collect();
        for pane in &gone {
            state.live.remove(pane);
            state.decided.remove(pane);
            state.hooked.remove(pane);
            state.routes.retain(|_, (routed, _)| routed != pane);
        }
        let moved = after != before || !gone.is_empty();
        drop(state);
        if moved {
            hooks.bump();
        }
    }
}
