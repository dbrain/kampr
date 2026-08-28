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
pub use facet::{Compaction, FacetFeed, FacetFold, Facets, Mode, Queued, Timing, Title, TitleSource, Titles};
pub use live::{Change, LIVE_ID, LiveBlock, ScreenReader, Watch, retired};
pub use marker::SessionMarker;
pub use model::{Attachment, Block, Page, Role, ToolState, Turn, TurnKind};
pub use process::{Harness, PaneProcess};
pub use root::TranscriptRoot;
pub use sub::SubRef;
pub use tail::{FileJournal, Journal, TranscriptParser};

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
    registry
}
