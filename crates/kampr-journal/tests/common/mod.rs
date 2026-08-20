#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use kampr_journal::{Block, Journal, Turn};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn claude_root() -> PathBuf {
    fixtures().join("claude")
}

pub fn codex_root() -> PathBuf {
    fixtures().join("codex")
}

pub const CLAUDE_SESSION: &str = "9f1c0b2e-0000-4000-8000-000000000001";
pub const CODEX_SESSION: &str = "01a01311-5036-7e52-8bef-ac91e2fe2b51";

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
            Block::Md { text } => Some(text.as_str()),
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
