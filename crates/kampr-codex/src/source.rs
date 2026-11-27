//! The Codex app-server as a seam `Source`: one process per open pane, threads listed from
//! its `thread/list`, and the herd that lets `Source::end` reach a pane's process.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use kampr_core::child_env::{self, LoginEnv};
use kampr_core::conversation::{Conversation, Ending, NewSession, Refusal, Source, Stored};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::conversation::{FIRST_PROMPT_LIMIT, conversation};
use crate::wire::Pump;

/// What the herd keeps on a live pane: the end-control and the process itself.
struct Live {
    end: mpsc::Sender<()>,
    pid: u32,
}

/// The source's registry of the processes it owns, by thread id.
#[derive(Default)]
pub struct CodexHerd {
    live: Mutex<HashMap<String, Live>>,
}

impl CodexHerd {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, id: String, end: mpsc::Sender<()>, pid: u32) {
        self.live.lock().unwrap().insert(id, Live { end, pid });
    }

    pub fn remove(&self, id: &str) {
        self.live.lock().unwrap().remove(id);
    }

    /// Ask a live pane to end. `false` when the pane is not live here: the thread stays
    /// resumable, which is all a codex thread is between panes.
    pub async fn end(&self, id: &str) -> bool {
        let live = self.live.lock().unwrap().remove(id);
        match live {
            Some(live) => live.end.send(()).await.is_ok(),
            None => false,
        }
    }

    /// The pid of a live pane's process, for the node that owns the herd.
    pub fn pid(&self, id: &str) -> Option<u32> {
        self.live.lock().unwrap().get(id).map(|live| live.pid)
    }
}

/// The child's whole environment: given outright, or the operator's login one less what pays.
enum ChildEnv {
    Exactly(Vec<(String, String)>),
    Login(LoginEnv),
}

/// A seam `Source` over `codex app-server --stdio`.
///
/// The program, its arguments and the environment are injected, so the tests run the probe's
/// mock; [`Self::from_env`] is the production shape.
pub struct CodexSource {
    program: String,
    args: Vec<String>,
    env: ChildEnv,
    herd: Arc<CodexHerd>,
}

impl CodexSource {
    pub fn new(program: impl Into<String>, args: Vec<String>, env: Vec<(String, String)>) -> Self {
        Self {
            program: program.into(),
            args,
            env: ChildEnv::Exactly(env),
            herd: Arc::new(CodexHerd::new()),
        }
    }

    /// The production shape: `codex` (or `KAMPR_CODEX`) under `shell`'s login environment, so
    /// its tools and its `CODEX_HOME` (unset is `~/.codex`, the operator's own login) are what
    /// a terminal codex would have, less the keys and endpoints codex reads (probe row 697).
    pub fn from_env(shell: &str) -> Self {
        let program = std::env::var("KAMPR_CODEX").unwrap_or_else(|_| "codex".into());
        let mut source = Self::new(program, vec!["app-server".into(), "--stdio".into()], Vec::new());
        source.env = ChildEnv::Login(LoginEnv::new(shell));
        source
    }

    /// Extra flags for the app-server, after `app-server --stdio`.
    pub fn with_config(mut self, flags: Vec<String>) -> Self {
        self.args.extend(flags);
        self
    }

    /// The processes this source has open, for the node that must be able to end them.
    pub fn herd(&self) -> &CodexHerd {
        &self.herd
    }

    /// Spawn the server and get through the handshake.
    async fn boot(&self) -> Result<Pump, Refusal> {
        let env = match &self.env {
            ChildEnv::Exactly(env) => env.clone(),
            ChildEnv::Login(login) => child_env::CODEX.strip(login.get().await),
        };
        let pump = Pump::spawn(&self.program, &self.args, &env)
            .await
            .map_err(|e| Refusal::new(format!("the codex app-server will not start: {e}")))?;
        pump.handshake()
            .await
            .map_err(|e| Refusal::new(format!("the codex handshake failed: {e}")))?;
        Ok(pump)
    }
}

#[async_trait::async_trait]
impl Source for CodexSource {
    fn kind(&self) -> &'static str {
        "codex"
    }

    async fn open(&self, pane: &str) -> Result<Box<dyn Conversation>, Refusal> {
        let pump = self.boot().await?;
        let res = pump
            .call("thread/resume", json!({"threadId": pane}))
            .await
            .map_err(|e| Refusal::new(format!("no such codex thread: {e}")))?;
        let thread = res.get("thread").cloned().unwrap_or(Value::Null);
        Ok(conversation(pump, pane.to_string(), thread, self.herd.clone()))
    }

    async fn create(&self, new: NewSession) -> Result<String, Refusal> {
        let mut pump = self.boot().await?;
        let mut params = json!({});
        if let Some(cwd) = &new.cwd {
            params["cwd"] = Value::String(cwd.display().to_string());
        }
        let res = pump
            .call("thread/start", params)
            .await
            .map_err(|e| Refusal::new(format!("thread/start: {e}")))?;
        let thread_id = res
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| Refusal::new("thread/start returned no thread id"))?
            .to_string();
        if let Some(prompt) = &new.prompt {
            pump.call(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": prompt}],
                }),
            )
            .await
            .map_err(|e| Refusal::new(format!("turn/start: {e}")))?;
            pump.wait_turn_end(FIRST_PROMPT_LIMIT)
                .await
                .map_err(|e| Refusal::new(format!("the first turn did not finish: {e}")))?;
        }
        // The pane opens the thread itself; this process has done its part.
        drop(pump);
        Ok(thread_id)
    }

    async fn resumable(&self, cwd: Option<&Path>) -> Vec<Stored> {
        let Ok(pump) = self.boot().await else {
            return Vec::new();
        };
        let Ok(res) = pump.call("thread/list", json!({"limit": 50})).await else {
            return Vec::new();
        };
        drop(pump);
        let Some(threads) = res.get("data").and_then(Value::as_array) else {
            return Vec::new();
        };
        threads
            .iter()
            .filter(|t| {
                cwd.map(|cwd| {
                    t.get("cwd")
                        .and_then(Value::as_str)
                        .is_some_and(|c| Path::new(c) == cwd)
                })
                .unwrap_or(true)
            })
            .map(|t| Stored {
                id: t
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                cwd: t
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .map(std::path::PathBuf::from),
                title: t
                    .get("preview")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(String::from),
            })
            .filter(|s| !s.id.is_empty())
            .collect()
    }

    async fn end(&self, pane: &str, how: Ending) -> Result<(), Refusal> {
        match how {
            Ending::ToTerminal => Err(Refusal::new(
                "the codex app-server is headless; there is no terminal to hand it to",
            )),
            Ending::End => {
                self.herd.end(pane).await;
                Ok(())
            }
        }
    }
}
