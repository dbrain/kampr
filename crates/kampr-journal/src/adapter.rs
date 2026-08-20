use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::JournalError;
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

    fn open(&self, session: &SessionRef) -> Result<Box<dyn Journal>, JournalError> {
        Ok(self.open_path(self.locate(session)?))
    }

    fn open_path(&self, path: PathBuf) -> Box<dyn Journal> {
        Box::new(FileJournal::new(path, self.parser()))
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

    /// Exactly what the wire document says `has_conversation` means: a journal adapter exists for
    /// this harness. Deriving it from adapter registration is what keeps it from disagreeing with
    /// `caps.conversation`, which is [`Self::serves_any`] — a pane can never claim more than the
    /// node does.
    pub fn has_conversation(&self, pane_agent: Option<&str>) -> bool {
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
        let announced = session
            .filter(|s| Some(s.agent.as_str()) == pane_agent)
            .and_then(|s| adapter.locate(s).ok());
        let path = match announced {
            Some(path) => path,
            None => match cwd.map(|cwd| adapter.locate_by_cwd(cwd)) {
                Some(Ok(path)) => path,
                Some(Err(JournalError::NotFound(_))) | None => return Ok(None),
                Some(Err(e)) => return Err(e),
            },
        };
        Ok(Some(adapter.open_path(path)))
    }
}
