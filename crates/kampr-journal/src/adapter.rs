use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::attach::{Fetched, Origin};
use crate::composer::ComposerReader;
use crate::error::JournalError;
use crate::facet::{FacetFeed, FacetFold, Facets};
use crate::live::ScreenReader;
use crate::marker::SessionMarker;
use crate::process::{Harness, PaneProcess};
use crate::root::TranscriptRoot;
use crate::sub::{self, SubRef};
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
    /// The containment root every path this adapter resolves — including one arriving inside an
    /// attachment id — is proved to be inside.
    fn root(&self) -> &TranscriptRoot;
    fn locate(&self, session: &SessionRef) -> Result<PathBuf, JournalError>;
    /// The transcript the pane's own harness process is writing.
    ///
    /// **This is the only exact answer a node gets.** Herdr 0.8.2 never populates
    /// `pane.agent_session` (probe #75), and a working directory names a *project*, not a
    /// session. A harness that publishes which session a pid is on answers it here; one that does
    /// not leaves it, and the caller falls back to a bounded search of the directory.
    fn locate_by_process(&self, _process: &PaneProcess) -> Result<PathBuf, JournalError> {
        Err(JournalError::NotFound(String::new()))
    }

    /// Which of a pane's processes is on a session of this harness, and what the harness itself
    /// wrote down about it.
    ///
    /// **The pipeline, not the name.** A pane's foreground job is matched on pid against what the
    /// harness records, so a pane herdr can only describe as `bash` (#297) is still identified
    /// exactly, and it is identified from the moment the session opens rather than from the moment
    /// it has written a transcript.
    ///
    /// The default is a harness that publishes no such map — every one but Claude today. That is
    /// this adapter having nothing to say, not a claim that the pane is running no agent.
    fn marker(&self, _pipeline: &[PaneProcess]) -> Option<SessionMarker> {
        None
    }

    /// A conversation one of this harness's own launched, named by a handle minted onto a turn.
    ///
    /// Containment is the whole of what this adds: the caller's string is resolved through this
    /// adapter's root before anything is opened. Proving it is a conversation the *pane* may see
    /// is [`Registry::open_sub`]'s half.
    fn open_sub(&self, sub: &SubRef) -> Result<Box<dyn Journal>, JournalError> {
        Ok(self.open_path(self.root().contain(&sub.path)?))
    }

    /// The newest transcript that declares `cwd` as its working directory, out of those still
    /// being written after `since`.
    ///
    /// Every candidate is verified against the directory it claims, so a wrong guess yields
    /// nothing rather than somebody else's conversation — and `since`, the pane harness's start
    /// time, is what makes "this directory's newest" mean "this run's", because every run in a
    /// directory leaves a transcript behind it.
    fn locate_by_cwd(&self, cwd: &Path, since: Option<SystemTime>) -> Result<PathBuf, JournalError>;
    fn parser(&self) -> Box<dyn TranscriptParser>;

    /// What this harness wrote down beside the conversation that is about the *session* rather
    /// than about a turn: a title, the mode it is in, the prompts still waiting, how long its
    /// turns took, where it was compacted.
    ///
    /// The default is a harness that fills none of them, and that is not a second-class harness —
    /// it is one nobody has measured an equivalent for. Filling a facet from a field that merely
    /// reads like one is worse than leaving it empty, because the client cannot tell the
    /// difference and the wire keeps the promise for ever.
    fn facets(&self, _transcript: &Path, _marker: Option<&SessionMarker>) -> Facets {
        Facets::default()
    }

    /// The same collection, resumable: a fold that keeps its accumulator and the byte it has
    /// reached, so a second look costs the records the transcript has grown by.
    ///
    /// `None` — the default — is a harness whose collector can only be read whole. It is not a
    /// harness whose facets are frozen: [`Registry::fold`] wraps it in one that re-reads the file,
    /// which is correct and costs exactly what [`Self::facets`] costs every time it is asked.
    fn fold(&self) -> Option<Box<dyn FacetFold>> {
        None
    }

    /// Reads an in-progress message off this harness's visible screen, for the harnesses whose
    /// screen somebody has actually probed. `None` — the default — means a pane running this
    /// harness serves its transcript and nothing more, which is what every harness did before
    /// live turns existed.
    fn screen(&self) -> Option<ScreenReader> {
        None
    }

    /// Reads what the operator has typed at the desk and not sent, for the harnesses whose
    /// composer somebody has actually probed. `None` — the default — is a harness that publishes
    /// no desk line at all, and a client that draws nothing for it.
    ///
    /// Separate from [`Self::screen`] and deliberately so: that one lifts the message the harness
    /// is painting, this one lifts the half-sentence a person left in the box, and conflating
    /// them would put one in the other's place on any harness where only one has been measured.
    fn composer(&self) -> Option<ComposerReader> {
        None
    }

    /// The `index`th attachment of one already-read record, decoded. The default is a harness
    /// whose transcripts have never been measured to carry one.
    fn attachment(&self, _record: &str, index: u32) -> Result<Fetched, JournalError> {
        Err(JournalError::NotFound(index.to_string()))
    }

    fn open(&self, session: &SessionRef) -> Result<Box<dyn Journal>, JournalError> {
        Ok(self.open_path(self.locate(session)?))
    }

    fn open_path(&self, path: PathBuf) -> Box<dyn Journal> {
        let mut parser = self.parser();
        parser.set_origin(Origin::new(self.agent(), self.root(), &path));
        Box::new(FileJournal::new(path, parser, self.screen()))
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

    /// Which harness a pane's processes are running, and the session it is on — asked of every
    /// registered adapter, and answered without herdr having scraped anything out of the pane.
    ///
    /// This is what makes a pane an agent pane the moment the agent opens. Two gates used to stand
    /// in the way and they were usually blamed on each other: the sweep only looked a pane up when
    /// herdr had already labelled it, and `has_conversation` meant a transcript file resolves —
    /// which it does not for the whole of the gap between a session opening and its first prompt.
    /// A [`SessionMarker`] carrying no transcript closes both, and says so as its own state rather
    /// than as an empty conversation.
    pub fn marker(&self, pipeline: &[PaneProcess]) -> Option<SessionMarker> {
        self.adapters
            .values()
            .find_map(|adapter| adapter.marker(pipeline))
    }

    /// What the harness the pane is running wrote down about this session, folded so that asking
    /// again costs the records the transcript has grown by — the pump asks for as long as a client
    /// is watching the pane.
    ///
    /// Nothing from any other adapter: a pane with no harness, or one whose harness is not
    /// registered here, gets a fold that reads nothing rather than somebody else's facets.
    pub fn fold(&self, pane_agent: Option<&str>) -> FacetFeed {
        FacetFeed::new(self.folder(pane_agent))
    }

    /// The same fold without the published-once comparison in front of it, for a caller that wants
    /// the collection as it stands rather than a frame when it moves — the herd's pane titles.
    pub fn folder(&self, pane_agent: Option<&str>) -> Box<dyn FacetFold> {
        let Some(adapter) = pane_agent.and_then(|agent| self.adapters.get(agent)) else {
            return Box::new(Nothing);
        };
        adapter.fold().unwrap_or_else(|| Box::new(Whole(adapter.clone())))
    }

    /// How to read the composer of a pane running `pane_agent`, for the harnesses whose composer
    /// has been measured. A harness nobody has probed publishes no desk line at all.
    pub fn composer(&self, pane_agent: Option<&str>) -> Option<ComposerReader> {
        pane_agent.and_then(|agent| self.adapters.get(agent))?.composer()
    }

    /// The conversation a `sub` handle names, proved to be one the pane asking may see.
    ///
    /// **Two independent checks, and both are load-bearing.** The handle arrives from the network,
    /// so the path inside it is resolved through the adapter's own [`TranscriptRoot`] —
    /// canonicalised, and proved to be inside it, which is what stops `../`, an absolute path and
    /// a symlink pointing out. That alone would still let a caller name *another* pane's
    /// transcript, which is inside the root and perfectly readable, so the result must also sit
    /// inside the session tree of the transcript this pane is on — a path the node derived itself
    /// and the request had no say in.
    pub fn open_sub(&self, id: &str, transcript: &Path) -> Result<Box<dyn Journal>, JournalError> {
        let handle = SubRef::decode(id)?;
        let adapter = self
            .adapters
            .get(&handle.agent)
            .ok_or_else(|| JournalError::NotFound(handle.agent.clone()))?;
        let resolved = adapter.root().contain(&handle.path)?;
        let tree = sub::tree(transcript)
            .canonicalize()
            .map_err(|_| JournalError::NotFound(handle.path.clone()))?;
        if !resolved.starts_with(&tree) {
            return Err(JournalError::Escape(handle.path.clone()));
        }
        adapter.open_sub(&SubRef {
            path: resolved.to_string_lossy().into_owned(),
            ..handle
        })
    }

    /// `Ok(None)` covers every "this pane simply has no conversation" case: no harness, no
    /// adapter for the harness, or nothing on disk that any handle resolves to.
    pub fn open(
        &self,
        pane_agent: Option<&str>,
        session: Option<&SessionRef>,
        cwd: Option<&Path>,
        harness: &Harness,
    ) -> Result<Option<Box<dyn Journal>>, JournalError> {
        let Some(adapter) = pane_agent.and_then(|agent| self.adapters.get(agent)) else {
            return Ok(None);
        };
        let Some(path) = self.locate(pane_agent, session, cwd, harness)? else {
            return Ok(None);
        };
        Ok(Some(adapter.open_path(path)))
    }

    /// The transcript [`Self::open`] would open, without opening it. This is what
    /// `has_conversation` is answered from: the file either resolves or it does not, and nothing
    /// short of looking can tell the difference.
    ///
    /// Three handles, strongest first, and **nothing** when none of them lands:
    ///
    /// 1. The session the pane announced, when it agrees with the pane's own harness. Herdr keeps
    ///    reporting the last session a pane announced (probe #38), so one whose `agent` disagrees
    ///    is stale and is dropped rather than followed.
    /// 2. The pane's harness **process**, which is what actually identifies a session. Exact
    ///    where the harness publishes the map, and it is the handle that moves the view when an
    ///    agent is quit and a fresh one started in the same pane. A harness that names
    ///    *this pane's* session but has written no transcript yet ends the ladder here
    ///    with nothing: an empty conversation is the answer, and the directory holds only
    ///    somebody else's (#311, #260).
    /// 3. The working directory, bounded by when that process started — never the directory
    ///    alone, because every run in a directory leaves a transcript and the newest of them
    ///    belongs to whoever ran last, not to this pane. Skipped entirely where the host has
    ///    looked into the pane and found no harness at all.
    pub fn locate(
        &self,
        pane_agent: Option<&str>,
        session: Option<&SessionRef>,
        cwd: Option<&Path>,
        harness: &Harness,
    ) -> Result<Option<PathBuf>, JournalError> {
        let Some(adapter) = pane_agent.and_then(|agent| self.adapters.get(agent)) else {
            return Ok(None);
        };
        let announced = session
            .filter(|s| Some(s.agent.as_str()) == pane_agent)
            .and_then(|s| adapter.locate(s).ok());
        if let Some(path) = announced {
            return Ok(Some(path));
        }
        let process = harness.process();
        if let Some(p) = process {
            match adapter.locate_by_process(p) {
                Ok(path) => return Ok(Some(path)),
                // The harness named *this pane's* session and it has written nothing yet
                // (#311). The directory cannot hold a better answer than the one already
                // in hand, and the newest transcript in it is somebody else's (#260), so
                // an empty conversation is the honest answer rather than a guess.
                Err(JournalError::Unwritten(_)) => return Ok(None),
                Err(_) => {}
            }
        }
        if !harness.may_search() {
            return Ok(None);
        }
        let since = process.and_then(|p| p.started);
        match cwd.map(|cwd| adapter.locate_by_cwd(cwd, since)) {
            Some(Ok(path)) => Ok(Some(path)),
            Some(Err(JournalError::NotFound(_))) | None => Ok(None),
            Some(Err(e)) => Err(e),
        }
    }
}

/// The fold of a harness that has no resumable one: the whole file, every time it is asked.
struct Whole(Arc<dyn JournalAdapter>);

impl FacetFold for Whole {
    fn facets(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
        self.0.facets(transcript, marker)
    }
}

struct Nothing;

impl FacetFold for Nothing {
    fn facets(&mut self, _transcript: &Path, _marker: Option<&SessionMarker>) -> Facets {
        Facets::default()
    }
}
