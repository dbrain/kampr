use super::{ManageError, ManageOp, Manager, native_geometry};
use crate::hooks::launch;
use kampr_core::conversation::{Ending, NewSession, Refusal, Source};
use kampr_core::provider::{Input, Provider};
use kampr_proc::Table;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a terminal Claude has to exit before its conversation is resumed headless.
const LET_GO: Duration = Duration::from_secs(10);
/// How long an interrupted terminal Claude has to come to rest before it is hung up on anyway.
const SETTLE: Duration = Duration::from_secs(5);

const OFF: &str = "headless Claude is off on this node ([claude] headless = false)";

pub fn is_claude_op(op: &str) -> bool {
    op.starts_with("claude.") && !op.starts_with("claude.hooks.")
}

/// `close` is what every client sends a pane it is done with, a headless one included.
pub fn is_claude_close(op: &ManageOp, node_id: &str) -> bool {
    op.op == "close"
        && op
            .at
            .as_deref()
            .and_then(|at| at.strip_prefix(node_id))
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|local| local.starts_with(kampr_claude::provider::PREFIX))
}

fn refused(r: Refusal) -> ManageError {
    ManageError::BadRequest(r.message)
}

fn cwd(op: &ManageOp) -> Option<PathBuf> {
    op.cwd
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(PathBuf::from)
}

fn prompt(op: &ManageOp) -> Option<String> {
    op.prompt.clone().filter(|p| !p.trim().is_empty())
}

fn pane(keeper: &str) -> Value {
    json!({ "pane_id": kampr_claude::provider::pane_id(keeper) })
}

impl Manager<'_> {
    pub(super) async fn claude_op(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let source = self.claude.ok_or_else(|| ManageError::BadRequest(OFF.into()))?;
        match op.op.as_str() {
            "claude.create" => source
                .create(NewSession {
                    cwd: cwd(op),
                    prompt: prompt(op),
                })
                .await
                .map(|keeper| pane(&keeper))
                .map_err(refused),
            "claude.resumable" => {
                let stored = source.resumable(cwd(op).as_deref()).await;
                let listed: Vec<Value> = stored
                    .iter()
                    .map(|s| json!({ "id": s.id, "cwd": s.cwd, "title": s.title }))
                    .collect();
                Ok(json!({ "resumable": listed }))
            }
            "claude.resume" => {
                let id = op
                    .id
                    .as_deref()
                    .ok_or_else(|| ManageError::BadRequest("claude.resume needs `id`".into()))?;
                source
                    .resume(id, cwd(op).as_deref(), prompt(op).as_deref())
                    .await
                    .map(|keeper| pane(&keeper))
                    .map_err(refused)
            }
            "claude.end" | "close" => {
                let local = self.local_of(op, kampr_claude::provider::PREFIX)?;
                source.end(&local, Ending::End).await.map_err(refused)?;
                Ok(json!({ "ok": true }))
            }
            "claude.to_terminal" => self.to_terminal(source, op).await,
            "claude.from_terminal" => self.continue_headless(source, op).await,
            other => Err(ManageError::Unsupported(other.to_string())),
        }
    }

    fn local_of(&self, op: &ManageOp, prefix: &str) -> Result<String, ManageError> {
        let at = op
            .at
            .as_deref()
            .ok_or_else(|| ManageError::BadRequest(format!("{} needs `at`", op.op)))?;
        at.strip_prefix(self.node_id)
            .and_then(|rest| rest.strip_prefix('/'))
            .filter(|local| local.starts_with(prefix))
            .map(str::to_string)
            .ok_or_else(|| ManageError::UnknownTarget(at.to_string()))
    }

    /// The headless child is stopped first and `claude -r` started only once it has gone; the
    /// binary is found before, and a terminal that still does not start gives the conversation
    /// back to a headless child, since neither side running is the one outcome to avoid.
    async fn to_terminal(
        &self,
        source: &kampr_claude::ClaudeSource,
        op: &ManageOp,
    ) -> Result<Value, ManageError> {
        let native = self
            .native
            .ok_or_else(|| ManageError::BadRequest("this node does not keep native sessions".into()))?;
        let local = self.local_of(op, kampr_claude::provider::PREFIX)?;
        let (cols, rows) = native_geometry(op)?;
        let binary = source.binary().await.map_err(refused)?;
        let stops = source.busy(&local).map_err(refused)?;
        if !stops.is_empty() {
            if !confirmed(op) {
                return Err(busy(&stops));
            }
            source.interrupt(&local).await.map_err(refused)?;
        }
        let handed = source.hand_over(&local).await.map_err(refused)?;
        let argv = vec![binary, "-r".into(), handed.claim.id().to_string()];
        let created = native
            .create(kampr_native::NewSession {
                argv: launch::argv(argv, self.claude_settings),
                cwd: handed.cwd.as_ref().map(|c| c.display().to_string()),
                label: op.label.clone(),
                cols,
                rows,
                ..kampr_native::NewSession::default()
            })
            .await;
        match created {
            Ok(pane) => Ok(json!({ "pane_id": pane })),
            Err(e) => Err(ManageError::BadRequest(take_back(source, &handed, &e).await)),
        }
    }

    /// Only the Claude process is hung up, so a shell it was started from keeps its terminal.
    async fn continue_headless(
        &self,
        source: &kampr_claude::ClaudeSource,
        op: &ManageOp,
    ) -> Result<Value, ManageError> {
        let native = self
            .native
            .ok_or_else(|| ManageError::BadRequest("this node does not keep native sessions".into()))?;
        let local = self.local_of(op, kampr_native::PREFIX)?;
        let marker = self
            .journals
            .marker(&native.pane_processes(&local))
            .filter(|m| m.agent == "claude")
            .ok_or_else(|| {
                ManageError::BadRequest(format!(
                    "{local} is not running a Claude whose session this node knows"
                ))
            })?;
        let stops = terminal_stops(marker.status.as_deref());
        if !stops.is_empty() && !confirmed(op) {
            return Err(busy(&stops));
        }
        source.binary().await.map_err(refused)?;
        // A Claude not yet spoken to has no transcript for `-r` to open, and nothing to carry over.
        let claim = match marker.transcript {
            Some(_) => Some(source.claim(&marker.session).map_err(refused)?),
            None => None,
        };
        if !stops.is_empty() {
            self.interrupt_terminal(native, &local).await;
        }
        // A Claude that is the session itself ends it on purpose, so it leaves the herd rather
        // than standing there as a failed exit.
        let own = native
            .session(&local)
            .and_then(|s| s.keeper())
            .is_some_and(|k| k.child == marker.pid);
        if own {
            native
                .close(&local)
                .map_err(|e| ManageError::BadRequest(format!("{e:#}")))?;
        }
        hang_up(marker.pid, !own).await?;
        let continued = match claim {
            Some(claim) => source.resume_claimed(&claim, marker.cwd.as_deref(), None).await,
            None => {
                source
                    .create(NewSession {
                        cwd: marker.cwd.clone(),
                        prompt: None,
                    })
                    .await
            }
        };
        continued.map(|keeper| pane(&keeper)).map_err(refused)
    }
}

impl Manager<'_> {
    /// Escape is Claude's own interrupt, and the transcript records the turn as interrupted
    /// rather than cut off; a Claude that does not come to rest is hung up on all the same.
    async fn interrupt_terminal(&self, native: &kampr_native::NativeProvider, local: &str) {
        let _ = native.write_pane(local, Input::Keys(vec!["Escape".into()])).await;
        let deadline = Instant::now() + SETTLE;
        while Instant::now() < deadline {
            let status = self
                .journals
                .marker(&native.pane_processes(local))
                .and_then(|m| m.status);
            if terminal_stops(status.as_deref()).is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn confirmed(op: &ManageOp) -> bool {
    op.interrupt == Some(true)
}

fn busy(stops: &[String]) -> ManageError {
    let listed = match stops {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    ManageError::Busy(format!("Claude is busy: moving it now stops {listed}."))
}

/// What the marker's status says a hand-over would stop; an idle Claude, or one too old to say,
/// loses nothing.
fn terminal_stops(status: Option<&str>) -> Vec<String> {
    let what = match status {
        None | Some("idle") => return Vec::new(),
        Some("busy") => "the turn in progress".to_string(),
        Some("waiting") => "the question or permission it is waiting on".to_string(),
        Some("shell") => "the ! command it is running".to_string(),
        Some(other) => format!("what it is doing ({other})"),
    };
    vec![what]
}

async fn take_back(
    source: &kampr_claude::ClaudeSource,
    handed: &kampr_claude::Handed,
    failed: &anyhow::Error,
) -> String {
    let back = source
        .resume_claimed(&handed.claim, handed.cwd.as_deref(), None)
        .await;
    match back {
        Ok(keeper) => format!(
            "the terminal did not start ({failed:#}), so the conversation was resumed without a terminal as {}",
            kampr_claude::provider::pane_id(&keeper)
        ),
        Err(r) => format!(
            "the terminal did not start ({failed:#}), and resuming the conversation without a terminal failed too: {}",
            r.message
        ),
    }
}

async fn hang_up(pid: u32, signal: bool) -> Result<(), ManageError> {
    let Some(raw) = i32::try_from(pid).ok().and_then(rustix::process::Pid::from_raw) else {
        return Err(ManageError::BadRequest(format!("{pid} is not a process")));
    };
    if signal {
        let _ = rustix::process::kill_process(raw, rustix::process::Signal::HUP);
    }
    let deadline = Instant::now() + LET_GO;
    while kampr_proc::here().argv(pid).is_some() {
        if Instant::now() > deadline {
            return Err(ManageError::BadRequest(format!(
                "Claude (process {pid}) did not stop within {}s, so nothing was resumed",
                LET_GO.as_secs()
            )));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}
