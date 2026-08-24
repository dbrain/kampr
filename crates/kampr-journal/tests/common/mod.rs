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

pub fn claude_parser() -> Box<dyn TranscriptParser> {
    use kampr_journal::{ClaudeAdapter, JournalAdapter, TranscriptRoot};
    ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap()).parser()
}

pub fn codex_parser() -> Box<dyn TranscriptParser> {
    use kampr_journal::{CodexAdapter, JournalAdapter, TranscriptRoot};
    CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap()).parser()
}

static SCRATCH: AtomicU32 = AtomicU32::new(0);

pub fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("kampr-journal-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
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
}

impl Scratch {
    pub fn turns(&mut self) -> Vec<Turn> {
        self.journal.poll().expect("poll")
    }
}

pub fn scratch_claude(tag: &str, records: &[serde_json::Value]) -> Scratch {
    scratch_with(tag, "projects/-home-u-demo/session.jsonl", records, |root| {
        std::sync::Arc::new(kampr_journal::ClaudeAdapter::new(root))
    })
}

pub fn scratch_codex(tag: &str, records: &[serde_json::Value]) -> Scratch {
    let named = "sessions/2026/08/18/rollout-2026-08-18T14-11-36-01a01311-5036-7e52-8bef-ac91e2fe2b51.jsonl";
    scratch_with(tag, named, records, |root| {
        std::sync::Arc::new(kampr_journal::CodexAdapter::new(root))
    })
}

fn scratch_with(
    tag: &str,
    relative: &str,
    records: &[serde_json::Value],
    build: impl Fn(kampr_journal::TranscriptRoot) -> std::sync::Arc<dyn kampr_journal::JournalAdapter>,
) -> Scratch {
    use kampr_journal::{Registry, TranscriptRoot};
    let root = scratch_dir(tag);
    let transcript = root.join(relative);
    std::fs::create_dir_all(transcript.parent().expect("a directory")).expect("a directory");
    let body: String = records.iter().map(|r| r.to_string() + "\n").collect();
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
