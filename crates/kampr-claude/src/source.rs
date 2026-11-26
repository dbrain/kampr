use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use kampr_core::conversation::{Conversation, Ending, NewSession, Refusal, Source, Stored};
use kampr_keep::{Launch, SessionDir, close};

use crate::config::Config;
use crate::conversation::ClaudeConversation;
use crate::launch::{self, Plan};
use crate::provider::{self, Herd};

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("c{:x}-{}", t, COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// The headless Claude source: a `claude -p` under a keeper's pipes, the conversation seam, and
/// the billing safeguards that stand in front of every word it sends.
pub struct ClaudeSource {
    config: Config,
    herd: Arc<Herd>,
    root: PathBuf,
    keeper: PathBuf,
}

impl ClaudeSource {
    pub fn new(config: Config, herd: Arc<Herd>, root: PathBuf, keeper: PathBuf) -> Self {
        Self {
            config,
            herd,
            root,
            keeper,
        }
    }

    pub fn herd(&self) -> Arc<Herd> {
        self.herd.clone()
    }

    fn config_dir(&self) -> Option<PathBuf> {
        self.config
            .config_dir()
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))
    }

    /// The transcript a session id writes: `<config>/projects/<slug>/<id>.jsonl`. The slug is
    /// derived from a cwd we do not always have, so an id is found by scanning the projects.
    fn find_transcript(&self, session_id: &str) -> Option<PathBuf> {
        let projects = self.config_dir()?.join("projects");
        let name = format!("{session_id}.jsonl");
        for entry in std::fs::read_dir(&projects).ok()? {
            let entry = entry.ok()?;
            let candidate = entry.path().join(&name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    fn plan(&self, cwd: Option<String>, resume: Option<String>) -> Plan {
        Plan {
            binary: self.config.binary().to_string(),
            cwd,
            policy: self.config.billing,
            config_dir: self.config.config_dir().map(str::to_string),
            base_url: std::env::var("KAMPR_TEST")
                .ok()
                .as_deref()
                .map(|v| v == "1")
                .unwrap_or(false)
                .then(|| std::env::var("ANTHROPIC_BASE_URL").ok())
                .flatten(),
            permission_mode: (!self.config.mode.is_empty()).then(|| self.config.mode.clone()),
            max_budget_usd: self.config.turn_budget_usd,
            resume,
            session_id: String::new(),
        }
    }
}

#[async_trait]
impl Source for ClaudeSource {
    fn kind(&self) -> &'static str {
        provider::AGENT
    }

    async fn open(&self, pane: &str) -> Result<Box<dyn Conversation>, Refusal> {
        let session_id = provider::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a headless Claude session")))?;
        let session_path = self.root.join(session_id);
        if !session_path.join("spec.json").exists() {
            return Err(Refusal::new(format!("no keeper session {session_id}")));
        }
        // The one-opener rule: a session id found in a live sessions/<pid>.json is a terminal
        // Claude, and two writers on a transcript fork it silently.
        if let Some(dir) = self.config_dir() {
            let paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir.join("sessions"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .collect();
            for path in paths {
                let Ok(meta) = serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(path).unwrap_or_default(),
                ) else {
                    continue;
                };
                if meta.get("sessionId").and_then(serde_json::Value::as_str) == Some(session_id) {
                    return Err(Refusal::new(format!(
                        "{session_id} is a live terminal session; opening it headless would fork the transcript"
                    )));
                }
            }
        }
        let transcript = self.find_transcript(session_id);
        Ok(Box::new(ClaudeConversation::new(
            &self.config,
            session_path,
            session_id.to_string(),
            transcript,
        )))
    }

    async fn create(&self, new: NewSession) -> Result<String, Refusal> {
        let cwd = new
            .cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .filter(|c| !c.is_empty());
        let mut plan = self.plan(cwd, None);
        plan.session_id = new_id();
        let inherited = std::env::vars().collect::<Vec<_>>();
        match launch::spawn(&self.root, &plan, inherited, Launch::auto(), &self.keeper) {
            Ok(dir) => {
                let id = dir.id().to_string();
                self.herd.add(&id);
                Ok(id)
            }
            Err(e) => Err(Refusal::new(format!("{e:#}"))),
        }
    }

    async fn resumable(&self, _cwd: Option<&Path>) -> Vec<Stored> {
        let Some(projects) = self.config_dir().map(|d| d.join("projects")) else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&projects) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| match std::fs::read_dir(entry.path()) {
                Ok(inner) => Some(
                    inner
                        .flatten()
                        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
                        .filter_map(|e| {
                            let id = e.file_name().to_str()?.to_string();
                            Some(Stored {
                                id,
                                cwd: None,
                                title: None,
                            })
                        }),
                ),
                Err(_) => None,
            })
            .flatten()
            .collect()
    }

    async fn end(&self, pane: &str, how: Ending) -> Result<(), Refusal> {
        let session_id = provider::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a headless Claude session")))?;
        let session = SessionDir::at(self.root.join(session_id));
        match how {
            Ending::End => close(&session).map_err(|e| Refusal::new(format!("{e:#}")))?,
            // Hand to a terminal: stop the headless child so the transcript is free, and the node
            // starts `claude -r` in a native session on the same id.
            Ending::ToTerminal => {
                let _ = close(&session);
            }
        }
        self.herd.remove(session_id);
        Ok(())
    }
}
