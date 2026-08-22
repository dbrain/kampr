pub mod adapter;
pub mod claude;
pub mod codex;
mod discover;
pub mod error;
pub mod live;
pub mod model;
pub mod process;
pub mod root;
mod store;
mod summary;
pub mod tail;

use std::path::Path;
use std::sync::Arc;

pub use adapter::{JournalAdapter, Registry, SessionKind, SessionRef};
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use error::JournalError;
pub use live::{Change, LIVE_ID, LiveBlock, ScreenReader, Watch, retired};
pub use model::{Block, Page, Role, ToolState, Turn};
pub use process::{Harness, PaneProcess};
pub use root::TranscriptRoot;
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
    registry
}
