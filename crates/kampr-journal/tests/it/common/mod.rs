#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use kampr_journal::{Block, Journal, TranscriptParser, Turn};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn claude_root() -> PathBuf {
    fixtures().join("claude")
}

pub fn codex_root() -> PathBuf {
    fixtures().join("codex")
}

pub fn agy_root() -> PathBuf {
    fixtures().join("agy")
}

/// A `.claude`-shaped root of its own, so the sessions the facet tests need — a manual title, a
/// generated one, neither — do not have to be grafted onto the transcript several tests assert
/// byte offsets into.
pub fn facets_root() -> PathBuf {
    fixtures().join("facets")
}

pub fn facets_transcript(session: &str) -> PathBuf {
    facets_root()
        .join("projects/-home-u-facets")
        .join(format!("{session}.jsonl"))
}

pub const FACETS_TITLED: &str = "3c9e7a10-0000-4000-8000-0000000000f1";
pub const FACETS_GENERATED: &str = "3c9e7a10-0000-4000-8000-0000000000f2";
pub const FACETS_UNTITLED: &str = "3c9e7a10-0000-4000-8000-0000000000f3";
pub const FACETS_RECORDED: &str = "3c9e7a10-0000-4000-8000-0000000000f4";
pub const FACETS_QUEUE: &str = "3c9e7a10-0000-4000-8000-0000000000f5";
pub const FACETS_RUNNING: &str = "3c9e7a10-0000-4000-8000-0000000000f6";

pub const CLAUDE_SESSION: &str = "9f1c0b2e-0000-4000-8000-000000000001";
pub const CODEX_SESSION: &str = "01a01311-5036-7e52-8bef-ac91e2fe2b51";
pub const AGY_SESSION: &str = "ded9537c-7c10-4c47-9b02-8f8f688b9938";

pub fn claude_transcript() -> PathBuf {
    claude_root()
        .join("projects/-home-u-demo")
        .join(format!("{CLAUDE_SESSION}.jsonl"))
}

pub fn codex_transcript() -> PathBuf {
    codex_root().join(format!(
        "sessions/2026/08/18/rollout-2026-08-18T14-11-36-{CODEX_SESSION}.jsonl"
    ))
}

pub fn agy_transcript() -> PathBuf {
    agy_root().join(format!(
        "brain/{AGY_SESSION}/.system_generated/logs/transcript_full.jsonl"
    ))
}

/// A root per harness holding the shapes probe #322 measured facets out of, kept apart from the
/// corpora the parsing tests assert byte offsets and turn counts against.
pub fn harness_facets_root(harness: &str) -> PathBuf {
    fixtures().join("harness-facets").join(harness)
}

pub const CODEX_FACETS_SESSION: &str = "01a01db9-177e-7ae3-99e3-9c42d9b6fc3d";
pub const AGY_FACETS_SESSION: &str = "7d1a4c60-2b93-4f5e-9a01-6c3e88f2d114";

pub fn codex_facets_transcript() -> PathBuf {
    harness_facets_root("codex").join(format!(
        "sessions/2026/08/20/rollout-2026-08-20T15-51-04-{CODEX_FACETS_SESSION}.jsonl"
    ))
}

pub fn agy_facets_transcript() -> PathBuf {
    harness_facets_root("agy").join(format!(
        "brain/{AGY_FACETS_SESSION}/.system_generated/logs/transcript_full.jsonl"
    ))
}

pub fn claude_parser() -> Box<dyn TranscriptParser> {
    use kampr_journal::{ClaudeAdapter, JournalAdapter, TranscriptRoot};
    ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap()).parser()
}

pub fn codex_parser() -> Box<dyn TranscriptParser> {
    use kampr_journal::{CodexAdapter, JournalAdapter, TranscriptRoot};
    CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap()).parser()
}

static SCRATCH: AtomicU32 = AtomicU32::new(0);

/// A directory under the temp dir that goes away with the value that owns it.
///
/// **Bind it to a name.** `scratch_dir(t).join(x)` drops the guard at the end of the statement and
/// takes the directory with it, which is loud rather than silent — the next write fails — but it
/// is the one way to hold this wrong.
pub struct ScratchDir(PathBuf);

impl std::ops::Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for ScratchDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Debug for ScratchDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn scratch_dir(tag: &str) -> ScratchDir {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("kampr-journal-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

pub fn drain(journal: &mut dyn Journal) -> Vec<Turn> {
    journal.poll().expect("poll")
}

pub fn md_texts(turns: &[Turn]) -> Vec<&str> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Md { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

pub fn tool_blocks(turns: &[Turn]) -> Vec<&Block> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter(|b| matches!(b, Block::Tool { .. }))
        .collect()
}

pub fn diff_blocks(turns: &[Turn]) -> Vec<&Block> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter(|b| matches!(b, Block::Diff { .. }))
        .collect()
}

pub struct Scratch {
    pub root: PathBuf,
    pub transcript: PathBuf,
    pub journals: kampr_journal::Registry,
    pub journal: Box<dyn Journal>,
    /// The root sits *inside* this rather than being it, so a test that writes beside the root —
    /// an escape target, a symlink's destination — writes inside the guard too instead of into
    /// the shared temp directory under a name every parallel test would collide on.
    _dir: ScratchDir,
}

impl Scratch {
    pub fn turns(&mut self) -> Vec<Turn> {
        self.journal.poll().expect("poll")
    }
}

pub fn scratch_claude(tag: &str, records: &[serde_json::Value]) -> Scratch {
    scratch_claude_body(tag, &lines(records))
}

/// The transcript written verbatim, for the tests whose subject is the bytes between the records
/// rather than the records.
pub fn scratch_claude_body(tag: &str, body: &str) -> Scratch {
    scratch_with(tag, "projects/-home-u-demo/session.jsonl", body, |root| {
        std::sync::Arc::new(kampr_journal::ClaudeAdapter::new(root))
    })
}

pub fn scratch_codex(tag: &str, records: &[serde_json::Value]) -> Scratch {
    let named = "sessions/2026/08/18/rollout-2026-08-18T14-11-36-01a01311-5036-7e52-8bef-ac91e2fe2b51.jsonl";
    scratch_with(tag, named, &lines(records), |root| {
        std::sync::Arc::new(kampr_journal::CodexAdapter::new(root))
    })
}

pub fn lines(records: &[serde_json::Value]) -> String {
    records.iter().map(|r| r.to_string() + "\n").collect()
}

fn scratch_with(
    tag: &str,
    relative: &str,
    body: &str,
    build: impl Fn(kampr_journal::TranscriptRoot) -> std::sync::Arc<dyn kampr_journal::JournalAdapter>,
) -> Scratch {
    use kampr_journal::{Registry, TranscriptRoot};
    let dir = scratch_dir(tag);
    let root = dir.join("root");
    std::fs::create_dir_all(&root).expect("a root");
    let transcript = root.join(relative);
    std::fs::create_dir_all(transcript.parent().expect("a directory")).expect("a directory");
    std::fs::write(&transcript, body).expect("a transcript");
    let adapter = build(TranscriptRoot::new(&root).expect("a root"));
    let journal = adapter.open_path(transcript.clone());
    let mut journals = Registry::new();
    journals.register(adapter);
    Scratch {
        root,
        transcript,
        journals,
        journal,
        _dir: dir,
    }
}

pub fn attachments(turns: &[Turn]) -> Vec<&kampr_journal::Attachment> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Md { att: Some(att), .. } => Some(att),
            _ => None,
        })
        .collect()
}
