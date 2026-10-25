use crate::client::Client;
use crate::directory::{Directory, Listed};
use crate::wire::{SessionInfo, SessionState};
use anyhow::{Result, bail};
use async_trait::async_trait;
use kampr_core::provider::{AgentStatus, Input, MuffitPane, PaneInfo, PaneStream, Provider, RawScrollback};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

pub const PREFIX: &str = "m:";
pub const AGENT: &str = "muffit";

pub fn pane_id(session: &str) -> String {
    format!("{PREFIX}{session}")
}

pub fn session_of(local: &str) -> Option<&str> {
    local.strip_prefix(PREFIX).filter(|s| !s.is_empty())
}

/// The muffit daemon's live sessions as panes of this node's herd. They have no terminal: nothing
/// streams them through the registry, and the node serves their conversation itself.
pub struct MuffitProvider {
    client: Client,
    directory: Arc<Directory>,
}

impl MuffitProvider {
    pub fn new(client: Client) -> Self {
        Self {
            directory: Directory::follow(client.clone()),
            client,
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn session(&self, local: &str) -> Option<SessionInfo> {
        self.directory.session(session_of(local)?)
    }

    pub fn revision(&self) -> watch::Receiver<u64> {
        self.directory.subscribe()
    }

    pub fn is_live(&self, local: &str) -> bool {
        session_of(local).is_some_and(|id| self.directory.is_live(id))
    }

    pub fn shutdown(&self) {
        self.directory.stop();
    }

    /// Waits for the session stream to list `session`, so an ack that names it names a pane the
    /// herd already has.
    pub async fn listed(&self, session: &str, within: Duration) -> bool {
        let mut revision = self.directory.subscribe();
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if self.directory.session(session).is_some() {
                return true;
            }
            if tokio::time::timeout_at(deadline, revision.changed())
                .await
                .is_err()
            {
                return false;
            }
        }
    }
}

pub fn agent_status(info: &SessionInfo) -> AgentStatus {
    if info.permission.is_some() {
        return AgentStatus::Blocked;
    }
    match info.state {
        SessionState::Running | SessionState::WaitingTool | SessionState::WaitingProvider => {
            AgentStatus::Working
        }
        SessionState::WaitingUser | SessionState::Completed | SessionState::Failed => AgentStatus::Done,
        SessionState::Created | SessionState::Paused => AgentStatus::Idle,
        SessionState::Stopped | SessionState::Unknown => AgentStatus::Unknown,
    }
}

/// A finished conversation is `idle`: nothing is running in it and nothing is waiting on it.
pub fn pane_info(listed: &Listed) -> PaneInfo {
    let info = &listed.info;
    PaneInfo {
        pane_id: pane_id(&info.id),
        cwd: info.cwd.clone(),
        label: info.title.clone(),
        agent: Some(AGENT.into()),
        agent_status: match listed.live {
            true => agent_status(info),
            false => AgentStatus::Idle,
        },
        cols: None,
        rows: 0,
        muffit: Some(MuffitPane {
            session_id: info.id.clone(),
        }),
        ..PaneInfo::default()
    }
}

#[async_trait]
impl Provider for MuffitProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        Ok(self.directory.sessions().iter().map(pane_info).collect())
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        bail!("{pane_id} is a muffit session and has no terminal to stream")
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        let Some(session) = session_of(pane_id) else {
            bail!("{pane_id} is not a muffit session");
        };
        let mut line = crate::input::Line::default();
        let steps = match input {
            Input::Bytes(bytes) => line.feed(&String::from_utf8_lossy(&bytes)),
            Input::Keys(keys) => line.keys(&keys),
        };
        for step in steps {
            match step {
                crate::input::Step::Interrupt => self.client.interrupt(session).await?,
                crate::input::Step::Submit(text) => self.client.input(session, &text).await?,
            }
        }
        Ok(())
    }

    async fn read_scrollback(&self, _pane_id: &str) -> Result<Option<RawScrollback>> {
        Ok(None)
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.directory.subscribe()
    }

    fn owns(&self, pane_id: &str) -> bool {
        pane_id.starts_with(PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_s_state_is_the_herd_s_status_as_docs_18_tabulates() {
        let prompt = Some(crate::wire::PermissionPrompt {
            id: 1,
            tool: "bash".into(),
            action: "run it".into(),
        });
        let cases = [
            (SessionState::Running, prompt.clone(), AgentStatus::Blocked),
            (SessionState::WaitingUser, prompt, AgentStatus::Blocked),
            (SessionState::Running, None, AgentStatus::Working),
            (SessionState::WaitingTool, None, AgentStatus::Working),
            (SessionState::WaitingProvider, None, AgentStatus::Working),
            (SessionState::WaitingUser, None, AgentStatus::Done),
            (SessionState::Completed, None, AgentStatus::Done),
            (SessionState::Failed, None, AgentStatus::Done),
            (SessionState::Created, None, AgentStatus::Idle),
            (SessionState::Paused, None, AgentStatus::Idle),
        ];
        for (state, permission, want) in cases {
            let info = SessionInfo {
                id: "s1".into(),
                state,
                turns: 0,
                title: None,
                cwd: None,
                branch: None,
                model: None,
                permission,
            };
            assert_eq!(agent_status(&info), want, "{state:?}");
        }
    }
}
