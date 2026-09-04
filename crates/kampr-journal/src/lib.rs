pub mod adapter;
pub mod agy;
pub mod attach;
pub mod claude;
pub mod codex;
pub mod composer;
pub(crate) mod discover;
pub mod envelope;
pub mod error;
pub mod facet;
pub mod live;
pub mod marker;
pub mod model;
pub mod omp;
mod output;
pub mod presence;
pub mod process;
pub mod root;
pub(crate) mod scan;
mod store;
pub mod sub;
mod summary;
pub mod tail;

use std::path::Path;
use std::sync::Arc;

pub use adapter::{JournalAdapter, Registry, SessionKind, SessionRef};
pub use agy::AgyAdapter;
pub use attach::{Att, Fetched, FileRef, Locator, Origin, Source};
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use composer::{Caret, Composed, ComposerFeed, ComposerReader};
pub use error::JournalError;
pub use facet::{
    Compaction, FacetFeed, FacetFold, Facets, Mode, Queued, QueuedReader, Timing, Title, TitleSource, Titles,
};
pub use live::{Change, LIVE_ID, LiveBlock, ScreenReader, Watch, retired};
pub use marker::SessionMarker;
pub use model::{Attachment, Block, CodeRole, Page, Role, ToolState, Turn, TurnKind};
pub use omp::OmpAdapter;
pub use process::{Harness, PaneProcess, Started};
pub use root::TranscriptRoot;
pub use sub::SubRef;
pub use summary::marker_of;
pub use tail::{FileJournal, Journal, TranscriptParser};

/// The run state a harness wrote into its own terminal title, for the harnesses whose title has
/// been measured.
///
/// **Deliberately not on [`JournalAdapter`], and deliberately not behind a [`Registry`].** What a
/// title means is a property of the harness, not of this machine's install: a pane running `omp`
/// publishes its state in the title whether or not this node can find `~/.omp/agent` to read its
/// transcripts from, and hanging this off a registered adapter would take the status away on
/// exactly the machines where the conversation is already missing.
///
/// Two callers, and the second is the reason this is one function. The herd publishes it as the
/// pane's status; the herdr snapshot's fingerprint hashes it, so a pane that starts working wakes
/// the herd — and, just as important, an animating spinner does **not**: omp repaints its title
/// every 80 ms, and a fingerprint over the raw title would push twelve herd updates a second to
/// every phone.
pub fn title_status(agent: Option<&str>, title: Option<&str>) -> Option<&'static str> {
    match agent? {
        // `omp` alone: [#490](#) measured `pi`'s title as `π - <session> - <dir>`, with no run
        // state in it at all, so routing one through this would be asking a question whose answer
        // is known to be nothing.
        omp::AGENT => omp::title_status(title?),
        _ => None,
    }
}

/// Registers whichever harnesses have a transcript root on this machine. A missing root is not
/// an error: a node with no Codex installed simply serves no Codex conversations.
pub fn registry_from_home(home: &Path) -> Registry {
    let mut registry = Registry::new();
    if let Ok(root) = TranscriptRoot::new(home.join(".claude")) {
        registry.register(Arc::new(ClaudeAdapter::new(root)));
    }
    if let Ok(root) = TranscriptRoot::new(home.join(".codex")) {
        registry.register(Arc::new(CodexAdapter::new(root)));
    }
    if let Ok(root) = TranscriptRoot::new(home.join(agy::HOME)) {
        registry.register(Arc::new(AgyAdapter::new(root)));
    }
    if let Ok(root) = TranscriptRoot::new(home.join(omp::HOME)) {
        registry.register(Arc::new(OmpAdapter::new(root)));
    }
    // The harness omp forked, under a home of its own and the same layout. Registered only where
    // that directory exists, and it serves nothing on a machine where it does not.
    if let Ok(root) = TranscriptRoot::new(home.join(omp::PI_HOME)) {
        registry.register(Arc::new(OmpAdapter::named(omp::PI_AGENT, root)));
    }
    registry
}
