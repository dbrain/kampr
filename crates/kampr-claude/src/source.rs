use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use kampr_core::child_env::LoginEnv;
use kampr_core::conversation::{Conversation, Ending, NewSession, Refusal, Source, Stored};
use kampr_keep::{Launch, SessionDir, close};

use crate::binary;
use crate::claim::{Claim, Claims};
use crate::config::Config;
use crate::conversation::ClaudeConversation;
use crate::conversation::activity::Activity;
use crate::kept::Kept;
use crate::launch::{self, Plan};
use crate::provider::{self, Herd};
use crate::request;
use crate::stored::{Store, claude_id};

/// How long a headless child has to exit before a terminal may open its transcript.
const LET_GO: Duration = Duration::from_secs(10);
/// How long an interrupted turn has to end before the hand-over hangs up on it anyway.
const SETTLE: Duration = Duration::from_secs(5);

/// What a headless pane was running, once its child has gone and a terminal may take it. The
/// conversation stays claimed until this is dropped, so start its next writer first.
pub struct Handed {
    pub claim: Claim,
    pub cwd: Option<PathBuf>,
}

fn random_hex<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("entropy for a session id");
    bytes
}

fn keeper_id() -> String {
    let [a, b, c, d, e, f, g, h] = random_hex::<8>();
    format!(
        "c{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        a, b, c, d, e, f, g, h
    )
}

/// A version 4 UUID, which is what `--session-id` takes.
fn conversation_id() -> String {
    let mut b = random_hex::<16>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The headless Claude source: a `claude -p` under a keeper's pipes, the conversation seam, and
/// the billing safeguards that stand in front of every word it sends.
///
/// A pane is `c:<keeper id>`. `create` starts a new conversation, `resume` continues a stored
/// one by its Claude session id (what `resumable` lists), and `end` stops either.
pub struct ClaudeSource {
    config: Config,
    herd: Arc<Herd>,
    root: PathBuf,
    keeper: PathBuf,
    store: Store,
    login: LoginEnv,
    binary: tokio::sync::Mutex<Option<String>>,
    claims: Claims,
}

impl ClaudeSource {
    /// `shell` is the login shell a bare `[claude] binary` is looked up by, and whose
    /// environment the child is given.
    pub fn new(config: Config, herd: Arc<Herd>, root: PathBuf, keeper: PathBuf, shell: String) -> Self {
        let store = Store::new(&config);
        Self {
            config,
            herd,
            root,
            keeper,
            store,
            login: LoginEnv::new(shell),
            binary: tokio::sync::Mutex::new(None),
            claims: Claims::default(),
        }
    }

    /// The Claude binary as an absolute path, or why there is none. Asked once and kept while
    /// it is still there, so an update that moves it is found again.
    pub async fn binary(&self) -> Result<String, Refusal> {
        let mut kept = self.binary.lock().await;
        if let Some(path) = kept.as_ref().filter(|p| binary::runnable(Path::new(p))) {
            return Ok(path.clone());
        }
        let found = binary::resolve(self.config.binary(), &self.login.get().await)?;
        *kept = Some(found.clone());
        Ok(found)
    }

    pub fn herd(&self) -> Arc<Herd> {
        self.herd.clone()
    }

    fn plan(&self, binary: String, cwd: Option<&Path>, resume: Option<String>) -> Plan {
        Plan {
            binary,
            cwd: cwd.map(|p| p.display().to_string()).filter(|c| !c.is_empty()),
            policy: self.config.billing,
            config_dir: self.config.config_dir().map(str::to_string),
            base_url: std::env::var("KAMPR_TEST")
                .is_ok_and(|v| v == "1")
                .then(|| std::env::var("ANTHROPIC_BASE_URL").ok())
                .flatten(),
            permission_mode: (!self.config.mode.is_empty()).then(|| self.config.mode.clone()),
            max_budget_usd: self.config.turn_budget_usd,
            conversation: resume.is_none().then(conversation_id),
            resume,
            id: keeper_id(),
        }
    }

    async fn start(&self, plan: &Plan, prompt: Option<&str>) -> Result<String, Refusal> {
        let login = self.login.get().await;
        let dir = launch::spawn(&self.root, plan, login, Launch::auto(), &self.keeper)
            .map_err(|e| Refusal::new(format!("{e:#}")))?;
        if let Some(prompt) = prompt {
            Kept::at(dir.path().to_path_buf())
                .set_opening(prompt)
                .map_err(|e| {
                    let _ = close(&dir);
                    Refusal::new(format!(
                        "the session started but its first prompt could not be kept: {e}"
                    ))
                })?;
        }
        let id = dir.id().to_string();
        self.herd.add(&id);
        Ok(id)
    }

    /// Continue a stored conversation (an `id` `resumable` listed) as a new pane, and return its
    /// pane's session id. Two writers on one transcript fork it silently (#655), so a
    /// conversation a live process holds open is refused.
    pub async fn resume(
        &self,
        id: &str,
        cwd: Option<&Path>,
        prompt: Option<&str>,
    ) -> Result<String, Refusal> {
        let claim = self.claim(id)?;
        self.resume_claimed(&claim, cwd, prompt).await
    }

    /// Holds a Claude session against every other hand-over and resume until it is dropped.
    pub fn claim(&self, claude: &str) -> Result<Claim, Refusal> {
        self.claims.take(claude)
    }

    /// `resume` for a caller that already holds the conversation, such as one taking back a
    /// hand-over whose terminal did not start.
    pub async fn resume_claimed(
        &self,
        claim: &Claim,
        cwd: Option<&Path>,
        prompt: Option<&str>,
    ) -> Result<String, Refusal> {
        let id = claim.id();
        let Some(transcript) = self.store.find(id) else {
            return Err(Refusal::new(format!("there is no stored conversation {id}")));
        };
        if let Some(pid) = self.store.foreign_opener(id, None).or_else(|| self.held_here(id)) {
            return Err(Refusal::new(format!(
                "{id} is open in process {pid}; resuming it here would fork the transcript"
            )));
        }
        // Claude finds a session by its working directory, so it resumes where it ran.
        let stored = cwd
            .is_none()
            .then(|| crate::stored::cwd_of(&transcript))
            .flatten();
        let binary = self.binary().await?;
        self.start(
            &self.plan(binary, cwd.or(stored.as_deref()), Some(id.to_string())),
            prompt,
        )
        .await
    }

    /// Stop a headless pane so a terminal can continue its conversation. Two writers fork the
    /// transcript (#655), so this returns only once the child has exited, and refuses a pane
    /// that is no longer in the herd or whose conversation another process has open.
    pub async fn hand_over(&self, pane: &str) -> Result<Handed, Refusal> {
        let keeper_id = self.running(pane)?;
        let session = SessionDir::at(self.root.join(keeper_id));
        let spec = session
            .spec()
            .map_err(|_| Refusal::new(format!("no keeper session {keeper_id}")))?;
        let claude = claude_id(&spec.argv)
            .ok_or_else(|| Refusal::new(format!("{keeper_id} was not started with a Claude session id")))?;
        let claim = self.claim(claude)?;
        if let Some(pid) = self
            .store
            .foreign_opener(claude, session.keeper().map(|k| k.child))
        {
            return Err(Refusal::new(format!(
                "{claude} is open in process {pid}; a terminal on it would fork the transcript"
            )));
        }
        self.end(pane, Ending::ToTerminal).await?;
        let deadline = Instant::now() + LET_GO;
        while session.alive() {
            if Instant::now() > deadline {
                self.herd.add(keeper_id);
                return Err(Refusal::new(format!(
                    "{pane} did not stop within {}s, so no terminal was started",
                    LET_GO.as_secs()
                )));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(Handed {
            claim,
            cwd: spec.cwd.map(PathBuf::from),
        })
    }

    fn running<'a>(&self, pane: &'a str) -> Result<&'a str, Refusal> {
        let keeper_id = provider::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a headless Claude session")))?;
        if !self.herd.sessions().iter().any(|s| s == keeper_id) {
            return Err(Refusal::new(format!(
                "{pane} is not running here; it may have been handed to a terminal already"
            )));
        }
        Ok(keeper_id)
    }

    /// What handing this pane over now would stop; empty when its child is at rest.
    pub fn busy(&self, pane: &str) -> Result<Vec<String>, Refusal> {
        Ok(Activity::read(&self.root.join(self.running(pane)?)).stopping())
    }

    /// Interrupting the turn also settles whatever it was asking. A turn that has not ended by
    /// the deadline ends with its child, which the hand-over stops next.
    pub async fn interrupt(&self, pane: &str) -> Result<(), Refusal> {
        let path = self.root.join(self.running(pane)?);
        let line = request::control_request(
            &format!("handover.{}", keeper_id()),
            "interrupt",
            serde_json::json!({}),
        );
        kampr_keep::write(&SessionDir::at(&path), &line)
            .map_err(|e| Refusal::new(format!("the turn could not be interrupted: {e}")))?;
        let deadline = Instant::now() + SETTLE;
        while Activity::read(&path).turn && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// The pid of the pane of this node that already runs a Claude session.
    fn held_here(&self, claude: &str) -> Option<u32> {
        self.herd.sessions().into_iter().find_map(|id| {
            let dir = SessionDir::at(self.root.join(id));
            let spec = dir.spec().ok()?;
            (claude_id(&spec.argv) == Some(claude)).then(|| dir.keeper().map_or(0, |k| k.child))
        })
    }
}

#[async_trait]
impl Source for ClaudeSource {
    fn kind(&self) -> &'static str {
        provider::AGENT
    }

    async fn open(&self, pane: &str) -> Result<Box<dyn Conversation>, Refusal> {
        let keeper_id = provider::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a headless Claude session")))?;
        let session_path = self.root.join(keeper_id);
        let session = SessionDir::at(&session_path);
        let spec = session
            .spec()
            .map_err(|_| Refusal::new(format!("no keeper session {keeper_id}")))?;
        let claude = claude_id(&spec.argv)
            .ok_or_else(|| Refusal::new(format!("{keeper_id} was not started with a Claude session id")))?
            .to_string();
        if let Some(pid) = self
            .store
            .foreign_opener(&claude, session.keeper().map(|k| k.child))
        {
            return Err(Refusal::new(format!(
                "{claude} is open in a terminal Claude (process {pid}); opening it headless would fork the transcript"
            )));
        }
        Ok(Box::new(ClaudeConversation::new(
            &self.config,
            session_path,
            claude,
            self.store.clone(),
        )))
    }

    async fn create(&self, new: NewSession) -> Result<String, Refusal> {
        let binary = self.binary().await?;
        self.start(
            &self.plan(binary, new.cwd.as_deref(), None),
            new.prompt.as_deref(),
        )
        .await
    }

    /// The Claude session ids on disk that can be resumed, newest first. A conversation a live
    /// process holds open is not offered.
    async fn resumable(&self, cwd: Option<&Path>) -> Vec<Stored> {
        let held: HashSet<String> = self
            .herd
            .sessions()
            .into_iter()
            .filter_map(|id| {
                let spec = SessionDir::at(self.root.join(id)).spec().ok()?;
                claude_id(&spec.argv).map(str::to_string)
            })
            .collect();
        self.store
            .stored(cwd, &held)
            .into_iter()
            .filter(|s| self.store.foreign_opener(&s.id, None).is_none())
            .collect()
    }

    async fn end(&self, pane: &str, how: Ending) -> Result<(), Refusal> {
        let session_id = provider::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a headless Claude session")))?;
        let session = SessionDir::at(self.root.join(session_id));
        // Handing to a terminal stops the headless child the same way: the transcript is free
        // once it has gone, and the node starts `claude -r` in a native session on the same id.
        let ended = close(&session);
        if matches!(how, Ending::End) {
            ended.map_err(|e| Refusal::new(format!("{e:#}")))?;
        }
        self.herd.remove(session_id);
        Ok(())
    }
}
