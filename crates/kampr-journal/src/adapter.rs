use std::collections::HashMap;
use std::path::PathBuf;
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
    fn parser(&self) -> Box<dyn TranscriptParser>;

    fn open(&self, session: &SessionRef) -> Result<Box<dyn Journal>, JournalError> {
        Ok(Box::new(FileJournal::new(self.locate(session)?, self.parser())))
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

    pub fn has_conversation(&self, pane_agent: Option<&str>, session: Option<&SessionRef>) -> bool {
        self.select(pane_agent, session).is_some()
    }

    /// Herdr keeps reporting the last agent session a pane announced, so a relaunched pane can
    /// still advertise the harness it used to run (probe #38). A session whose `agent` disagrees
    /// with the pane's own `agent` is stale and yields no conversation rather than the wrong one.
    pub fn select(
        &self,
        pane_agent: Option<&str>,
        session: Option<&SessionRef>,
    ) -> Option<&Arc<dyn JournalAdapter>> {
        let pane_agent = pane_agent?;
        let session = session?;
        if session.agent != pane_agent {
            return None;
        }
        self.adapters.get(pane_agent)
    }

    /// `Ok(None)` covers every "this pane simply has no conversation" case: no harness, no
    /// adapter for the harness, or a stale session announcement.
    pub fn open(
        &self,
        pane_agent: Option<&str>,
        session: Option<&SessionRef>,
    ) -> Result<Option<Box<dyn Journal>>, JournalError> {
        let Some(adapter) = self.select(pane_agent, session) else {
            return Ok(None);
        };
        adapter
            .open(session.expect("select requires a session"))
            .map(Some)
    }
}
