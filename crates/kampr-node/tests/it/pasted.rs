//! What the operator pasted, shown back to them in their own turn rather than as a path.
//!
//! The node writes a paste beside the pane and types where it put it, because that is what the
//! agent on the other end can read. This is the other half: the *reader's* copy of that turn, and
//! the id that turns it back into the picture they attached.

use kampr_journal::{Block, JournalAdapter, Role, TranscriptRoot};
use kampr_node::pasted::Shown;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00kampr-pasted-body";

struct Fixture {
    home: tempfile::TempDir,
    transcript: PathBuf,
    adapter: Arc<kampr_journal::ClaudeAdapter>,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("a home");
        let root = home.path().join("claude");
        std::fs::create_dir_all(root.join("projects/-home-u-demo")).expect("a project directory");
        std::fs::create_dir_all(home.path().join("state/pastes")).expect("a pastes directory");
        let transcript = root.join("projects/-home-u-demo/session.jsonl");
        let adapter = Arc::new(kampr_journal::ClaudeAdapter::new(
            TranscriptRoot::new(&root).expect("a root"),
        ));
        Self {
            home,
            transcript,
            adapter,
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.home.path().join("state")
    }

    fn pastes(&self) -> PathBuf {
        self.state_dir().join("pastes")
    }

    /// The path the node would have typed into the pane, with the bytes really on disk.
    fn write_paste(&self, name: &str) -> PathBuf {
        let path = self.pastes().join(name);
        std::fs::write(&path, PNG).expect("a pasted file");
        path
    }

    fn sweep(&self, name: &str) {
        std::fs::remove_file(self.pastes().join(name)).expect("a paste to sweep");
    }

    fn said(&self, role: &str, text: &str) {
        std::fs::write(&self.transcript, record(role, text).to_string() + "\n").expect("a transcript");
    }

    fn turn(&self) -> kampr_journal::Turn {
        let mut journal = Shown::over(self.adapter.open_path(self.transcript.clone()), &self.state_dir());
        journal.poll().expect("poll");
        let mut page = journal.page_before(None, 20);
        assert_eq!(page.turns.len(), 1, "one turn was written");
        page.turns.remove(0)
    }

    fn blocks(&self) -> Vec<Block> {
        self.turn().blocks
    }
}

fn record(role: &str, text: &str) -> Value {
    json!({
        "type": role,
        "uuid": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
        "timestamp": "2026-08-20T02:56:27.681Z",
        "message": { "role": role, "content": [{ "type": "text", "text": text }] }
    })
}

fn only_attachment(blocks: &[Block]) -> &kampr_journal::Attachment {
    blocks
        .iter()
        .find_map(|b| match b {
            Block::Md { att: Some(att), .. } => Some(att),
            _ => None,
        })
        .expect("the turn carries a picture")
}

fn prose(blocks: &[Block]) -> Vec<&str> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Md { text, att: None } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The report, and the whole of it: *"ideally we show the image/attached thing in the conversation
/// pane rather than just a path"*. The path stays on the wire to the agent and leaves the reader's
/// own turn.
#[test]
fn a_pasted_picture_stands_in_the_operators_turn_where_its_path_was() {
    let f = Fixture::new();
    let path = f.write_paste("shot-1756.png");
    f.said("user", &format!("{} what is this", path.display()));

    let blocks = f.blocks();
    let att = only_attachment(&blocks);
    assert_eq!(att.kind, "image");
    assert_eq!(att.mime.as_deref(), Some("image/png"));
    assert_eq!(att.name.as_deref(), Some("shot-1756.png"));
    assert_eq!(att.bytes, Some(PNG.len() as u64));
    assert_eq!(
        prose(&blocks),
        vec!["what is this"],
        "the path left the reader's turn and the words they typed did not: {blocks:?}"
    );
    assert!(
        !format!("{blocks:?}").contains(&path.display().to_string()),
        "the path is still in the turn somewhere: {blocks:?}"
    );
}

/// **The id is the whole security story**, so it is checked against the thing that mints it rather
/// than against a copy of the rule.
#[test]
fn the_id_resolves_to_that_file_and_to_nothing_outside_the_pastes_directory() {
    let f = Fixture::new();
    let path = f.write_paste("shot-1756.png");
    f.said("user", &format!("look {}", path.display()));

    let blocks = f.blocks();
    let id = &only_attachment(&blocks).id;
    let Ok(kampr_journal::Source::Paste(file)) = kampr_journal::Source::decode(id) else {
        panic!("a paste id decodes as one: {id}");
    };
    assert_eq!(file.path, path);

    let served = kampr_node::attach::serve_paste(&file, &f.pastes());
    assert_eq!(served.status(), 200, "the node hands back its own paste");

    // The transcript itself, asked for through the form that is *not* gated on a device that may
    // type. Refused because it is not in the directory, with the same 404 a missing paste gives.
    let elsewhere = kampr_journal::FileRef::new(f.transcript.clone());
    assert_eq!(
        kampr_node::attach::serve_paste(&elsewhere, &f.pastes()).status(),
        404,
        "a path outside the directory is not a paste",
    );

    let escaping = kampr_journal::FileRef::new(f.pastes().join("../../claude/projects"));
    assert_eq!(
        kampr_node::attach::serve_paste(&escaping, &f.pastes()).status(),
        404,
        "a `..` out of the directory is refused after canonicalisation, not before",
    );
}

/// A path in prose that this node did not write is a path, and stays one. The client refuses to
/// guess at paths in sentences for exactly this reason, and the node's advantage over it is only
/// that it knows which files it wrote — not that it is better at reading English.
#[test]
fn a_path_this_node_never_wrote_is_left_exactly_as_it_was_typed() {
    let f = Fixture::new();
    f.said("user", "look at /etc/passwd and at /home/u/.ssh/id_rsa please");
    let blocks = f.blocks();
    assert_eq!(
        prose(&blocks),
        vec!["look at /etc/passwd and at /home/u/.ssh/id_rsa please"],
    );
    assert!(
        blocks.iter().all(|b| matches!(b, Block::Md { att: None, .. })),
        "nothing was offered as an attachment: {blocks:?}",
    );
}

/// **A sweep is a day away and a turn is for ever.** The lifetime in `paste::write` is 24 hours and
/// the count is 64, so a turn from last week names a file that is not there — and offering a
/// picture that 404s is worse than showing the path that was really typed.
#[test]
fn a_paste_that_has_been_swept_away_shows_the_path_it_always_did() {
    let f = Fixture::new();
    let path = f.write_paste("shot-1756.png");
    f.said("user", &format!("{} what is this", path.display()));
    f.sweep("shot-1756.png");

    let blocks = f.blocks();
    assert!(
        blocks.iter().all(|b| matches!(b, Block::Md { att: None, .. })),
        "a swept paste was offered as a picture: {blocks:?}",
    );
    assert_eq!(prose(&blocks), vec![format!("{} what is this", path.display())]);
}

/// An agent that quotes the path back is talking about a file. Rewriting its words into a picture
/// would be putting them in its mouth, and the reader has already seen the picture on their own
/// turn a line above.
#[test]
fn the_agents_own_turn_keeps_the_path_because_the_path_is_what_it_is_talking_about() {
    let f = Fixture::new();
    let path = f.write_paste("shot-1756.png");
    f.said("assistant", &format!("I read {}", path.display()));

    let turn = f.turn();
    assert_eq!(turn.role, Role::Assistant);
    assert!(
        turn.blocks
            .iter()
            .all(|b| matches!(b, Block::Md { att: None, .. })),
        "the agent's prose was rewritten: {:?}",
        turn.blocks,
    );
    assert!(format!("{:?}", turn.blocks).contains(&path.display().to_string()));
}
