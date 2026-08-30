//! What the operator pasted, shown back to them where they pasted it.
//!
//! A paste becomes a path: the node writes the bytes beside the pane and types where it put them,
//! because an agent over ssh reads a local path perfectly well. That is right for the agent and
//! wrong for the reader, who typed *"what is this"* under a screenshot and gets back a line of
//! `/home/…/pastes/shot-1756...png` in their own message.
//!
//! **The node is the only thing that can fix that, and it is not a guess.** Detecting a path in
//! prose is a guess about English and `filePathOf` on the client refuses to make one — but this
//! node wrote these files, into a directory it owns, and it checks the file is still there before
//! it says anything. So the reader's own turn carries the picture and the node keeps typing the
//! path to the agent, which is the half that has to stay a path.

use std::path::{Path, PathBuf};

use kampr_journal::attach::{FileRef, Source};
use kampr_journal::{Attachment, Block, Journal, JournalError, Page, Role, Turn};

/// Extensions the attachment route will serve inline, which is the node's own short list
/// (`attach::kind_of`). Anything else is a file to be downloaded, and saying so is what keeps a
/// client from offering "Show image" over a button that cannot be one.
const PICTURES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// A journal whose turns are dressed on the way out.
///
/// A decorator rather than a call at each send, because there are seven places a turn reaches the
/// wire — a page, a re-opened page, the tail, a revision, a preview, a sub-conversation and its
/// tail — and a rule applied at six of them is the defect this shape cannot have.
pub struct Shown {
    inner: Box<dyn Journal>,
    pastes: PathBuf,
}

impl Shown {
    pub fn over(inner: Box<dyn Journal>, state_dir: &Path) -> Box<dyn Journal> {
        Box::new(Self {
            inner,
            pastes: state_dir.join("pastes"),
        })
    }
}

impl Journal for Shown {
    fn poll(&mut self) -> Result<Vec<Turn>, JournalError> {
        let pastes = self.pastes.clone();
        Ok(dress(self.inner.poll()?, &pastes))
    }

    fn page_before(&self, before: Option<&str>, limit: usize) -> Page {
        let page = self.inner.page_before(before, limit);
        Page {
            turns: dress(page.turns, &self.pastes),
            cursor: page.cursor,
            more: page.more,
        }
    }

    fn path(&self) -> &Path {
        self.inner.path()
    }

    fn turn_ids(&self) -> Vec<String> {
        self.inner.turn_ids()
    }

    fn preview(&self, screen: &[&str]) -> Option<Turn> {
        let dressed = dress(vec![self.inner.preview(screen)?], &self.pastes);
        dressed.into_iter().next()
    }
}

fn dress(turns: Vec<Turn>, pastes: &Path) -> Vec<Turn> {
    turns.into_iter().map(|turn| dress_turn(turn, pastes)).collect()
}

/// **Only the operator's own turns.** An agent that quotes the path back is talking about a file,
/// and rewriting its prose into a picture would be putting words in its mouth; the reader needs to
/// see what they attached once, on the message they attached it to.
fn dress_turn(mut turn: Turn, pastes: &Path) -> Turn {
    if turn.role != Role::User {
        return turn;
    }
    if !turn
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Md { att: None, .. }))
    {
        return turn;
    }
    let mut blocks = Vec::with_capacity(turn.blocks.len());
    for block in turn.blocks.drain(..) {
        match block {
            Block::Md { text, att: None } => blocks.extend(split(&text, pastes)),
            other => blocks.push(other),
        }
    }
    turn.blocks = blocks;
    turn
}

/// The block, or the blocks it becomes: a picture where a path this node wrote stood, and the
/// words around it in the order they were written.
fn split(text: &str, pastes: &Path) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut prose = String::new();
    let mut found = false;
    for token in text.split_inclusive(char::is_whitespace) {
        let bare = token.trim_end();
        match ours(bare, pastes) {
            Some(att) => {
                found = true;
                let trimmed = prose.trim();
                if !trimmed.is_empty() {
                    out.push(Block::md(trimmed));
                }
                prose.clear();
                out.push(Block::Md {
                    text: kampr_journal::marker_of(&att),
                    att: Some(att),
                });
            }
            None => prose.push_str(token),
        }
    }
    if !found {
        return vec![Block::md(text)];
    }
    let trimmed = prose.trim();
    if !trimmed.is_empty() {
        out.push(Block::md(trimmed));
    }
    out
}

/// A file this node wrote out of a paste, or nothing.
///
/// Three things have to be true and all three are facts rather than readings of the text: the
/// token is an absolute path, its parent is *this node's* pastes directory, and the file is still
/// on disk. The lifetime in `paste::write` is a day and the count is 64, so the third is the one
/// that stops an old turn offering a picture that was swept away weeks ago.
fn ours(token: &str, pastes: &Path) -> Option<Attachment> {
    let path = Path::new(token);
    if !path.is_absolute() || path.parent() != Some(pastes) {
        return None;
    }
    let stat = std::fs::metadata(path).ok()?;
    if !stat.is_file() {
        return None;
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let picture = PICTURES.contains(&extension.as_str());
    Some(Attachment {
        id: Source::Paste(FileRef::new(path)).encode(),
        kind: match picture {
            true => kampr_journal::attach::IMAGE.to_string(),
            false => kampr_journal::attach::FILE.to_string(),
        },
        mime: picture.then(|| match extension.as_str() {
            "jpg" | "jpeg" => "image/jpeg".to_string(),
            other => format!("image/{other}"),
        }),
        bytes: Some(stat.len()),
        name: path.file_name().and_then(|n| n.to_str()).map(str::to_string),
    })
}
