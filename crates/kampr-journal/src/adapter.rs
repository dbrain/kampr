use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::JournalError;
use crate::live::ScreenReader;
use crate::tail::{FileJournal, Journal, TranscriptParser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Id,
    Path,
}

/// Herdr's `agent_session` record, narrowed to what selecting a transcript needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub agent: String,
    pub kind: SessionKind,
    pub value: String,
}

impl SessionRef {
    pub fn id(agent: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            kind: SessionKind::Id,
            value: value.into(),
        }
    }

    pub fn path(agent: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            kind: SessionKind::Path,
            value: value.into(),
        }
    }
}

pub trait JournalAdapter: Send + Sync {
    fn agent(&self) -> &str;
    fn locate(&self, session: &SessionRef) -> Result<PathBuf, JournalError>;
    /// The newest transcript that declares `cwd` as its working directory.
    ///
    /// Herdr 0.8.2 detects both harnesses by scraping the screen and never populates
    /// `pane.agent_session`, so this is the resolution path that actually runs. Every candidate is
    /// verified against the directory it claims, so a wrong guess yields nothing rather than
    /// somebody else's conversation.
    fn locate_by_cwd(&self, cwd: &Path) -> Result<PathBuf, JournalError>;
    fn parser(&self) -> Box<dyn TranscriptParser>;

    /// Reads an in-progress message off this harness's visible screen, for the harnesses whose
    /// screen somebody has actually probed. `None` — the default — means a pane running this
    /// harness serves its transcript and nothing more, which is what every harness did before
    /// live turns existed.
    fn screen(&self) -> Option<ScreenReader> {
        None
    }

    fn open(&self, session: &SessionRef) -> Result<Box<dyn Journal>, JournalError> {
        Ok(self.open_path(self.locate(session)?))
    }

    fn open_path(&self, path: PathBuf) -> Box<dyn Journal> {
        Box::new(FileJournal::new(path, self.parser(), self.screen()))
    }
}

#[derive(Default)]
pub struct Registry {
    adapters: HashMap<String, Arc<dyn JournalAdapter>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, adapter: Arc<dyn JournalAdapter>) {
        self.adapters.insert(adapter.agent().to_string(), adapter);
    }

    pub fn get(&self, agent: &str) -> Option<&Arc<dyn JournalAdapter>> {
        self.adapters.get(agent)
    }

    /// Whether this node could serve a conversation for a pane running `pane_agent` *at all* —
    /// the adapter half of the question, and the cheap half.
    ///
    /// It is not the answer to `has_conversation`: a harness started a minute ago has no
    /// transcript, and a pane that claims one it cannot produce is a blank Conversation view and
    /// a `convo.load` that answers `not_found`. That question is [`Self::locate`], because only a
    /// path on disk settles it.
    pub fn serves(&self, pane_agent: Option<&str>) -> bool {
        pane_agent.is_some_and(|agent| self.adapters.contains_key(agent))
    }

    pub fn serves_any(&self) -> bool {
        !self.adapters.is_empty()
    }

    /// `Ok(None)` covers every "this pane simply has no conversation" case: no harness, no
    /// adapter for the harness, or nothing on disk that either handle resolves to.
    ///
    /// An announced session wins when it agrees with the pane's own harness. Herdr keeps
    /// reporting the last session a pane announced (probe #38), so a session whose `agent`
    /// disagrees is stale and is dropped in favour of the cwd rather than followed.
    pub fn open(
        &self,
        pane_agent: Option<&str>,
        session: Option<&SessionRef>,
        cwd: Option<&Path>,
    ) -> Result<Option<Box<dyn Journal>>, JournalError> {
        let Some(adapter) = pane_agent.and_then(|agent| self.adapters.get(agent)) else {
            return Ok(None);
        };
        let Some(path) = self.locate(pane_agent, session, cwd)? else {
            return Ok(None);
        };
        Ok(Some(adapter.open_path(path)))
    }

    /// The transcript [`Self::open`] would open, without opening it. This is what
    /// `has_conversation` is answered from: the file either resolves or it does not, and nothing
    /// short of looking can tell the difference.
    pub fn locate(
        &self,
        pane_agent: Option<&str>,
        session: Option<&SessionRef>,
        cwd: Option<&Path>,
    ) -> Result<Option<PathBuf>, JournalError> {
        let Some(adapter) = pane_agent.and_then(|agent| self.adapters.get(agent)) else {
            return Ok(None);
        };
        let announced = session
            .filter(|s| Some(s.agent.as_str()) == pane_agent)
            .and_then(|s| adapter.locate(s).ok());
        match announced {
            Some(path) => Ok(Some(path)),
            None => match cwd.map(|cwd| adapter.locate_by_cwd(cwd)) {
                Some(Ok(path)) => Ok(Some(path)),
                Some(Err(JournalError::NotFound(_))) | None => Ok(None),
                Some(Err(e)) => Err(e),
            },
        }
    }
}
