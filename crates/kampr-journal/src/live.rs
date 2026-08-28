use crate::model::{Block, Role, Turn};

/// The id every live preview carries. Turns are matched by id and replaced, so one reserved id is
/// the whole mechanism: the preview is revised as the message grows, and retired by a turn under
/// the same id carrying no blocks.
pub const LIVE_ID: &str = "live";

/// A message a harness is painting *now*, lifted off the visible screen.
///
/// **This is an approximation and it must lose to the transcript.** The screen is hard-wrapped to
/// the viewport, has already had its markdown rendered away to ANSI, and is clipped at the top
/// once a message outgrows the pane; the record the harness writes when the message finishes is
/// none of those things. `clipped` says which end of the message is known: with its own header
/// still on screen the text is known to start at the beginning, so a record need only *start
/// with* it to prove the preview redundant; once the header has scrolled away, all that can be
/// said is that a record *contains* it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBlock {
    pub text: String,
    pub clipped: bool,
}

/// Reads one harness's screen. A bare fn because it keeps no state — every call sees the whole
/// visible grid and nothing carries over.
pub type ScreenReader = fn(&[&str]) -> Option<LiveBlock>;

/// How a harness lays its screen out. Both harnesses probed so far agree on the shape — a marker
/// glyph in column zero opens a block and its wrapped remainder is indented — and disagree only
/// on the glyphs and on which head lines are the harness talking about itself.
pub struct Layout {
    /// Opens an assistant message.
    pub message: char,
    /// Opens the operator's own message, and also the composer at the foot of the screen.
    pub prompt: char,
    /// Opens a tool's output underneath its card.
    pub result: char,
    /// Columns a wrapped continuation line is indented by.
    pub indent: usize,
    /// The column the operator's own text starts at on the composer row — the marker and the one
    /// separator cell behind it. Measured at 2 on all three harnesses; it is what a caret resting
    /// on an empty composer is compared against, so it is a measurement and not an arithmetic.
    pub input: usize,
    /// Whether a head line is the harness rather than the answer — a spinner, a tool card.
    pub reject: fn(&str) -> bool,
}

/// The block the harness is painting at the foot of its screen, if that block is prose.
///
/// Runs upwards from the composer: the harness's own trailing chrome is skipped, the wrapped body
/// is gathered, and the walk stops at the line that opens the block. Stopping anywhere other than
/// the message marker or the top of the screen means the walk left the message it was reading —
/// the operator's own prompt, a rule, a tool's output — and that is not a preview.
pub fn read(screen: &[&str], layout: &Layout) -> Option<LiveBlock> {
    let end = screen
        .iter()
        .rposition(|line| starts_with(line, layout.prompt))
        .unwrap_or(screen.len());
    let mut body: Vec<&str> = Vec::new();
    let mut head: Option<&str> = None;
    let mut clipped = true;
    for line in screen[..end].iter().rev() {
        if let Some(rest) = opened_by(line, layout.message) {
            head = Some(rest);
            clipped = false;
            break;
        }
        let blank = line.trim().is_empty();
        let continuation = line.starts_with(&" ".repeat(layout.indent));
        if !blank && !continuation {
            // Anything else in column zero is a boundary. Reached before a single line of body it
            // is the harness's own footer and is skipped; reached after, the walk has run out of
            // the block it was reading.
            if body.is_empty() {
                continue;
            }
            return None;
        }
        if blank && body.is_empty() {
            continue;
        }
        body.push(line);
    }
    if let Some(head) = head
        && (layout.reject)(head)
    {
        return None;
    }
    // A tool's output is indented like a wrapped line, so a clipped walk can end up holding it.
    if body
        .iter()
        .any(|line| line.trim_start().starts_with(layout.result))
    {
        return None;
    }

    let mut text = String::new();
    if let Some(head) = head {
        text.push_str(head.trim_end());
        text.push('\n');
    }
    for line in body.iter().rev() {
        text.push_str(dedent(line, layout.indent).trim_end());
        text.push('\n');
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(LiveBlock { text, clipped })
}

/// The preview to publish for this screen, or `None` when there is nothing to preview *or* the
/// transcript already carries what the screen shows.
///
/// `recorded` is the newest turns' text, newest first. The comparison is on
/// [`fingerprint`]s because the two sides cannot be compared as written: the transcript holds
/// markdown source and the screen holds it rendered, wrapped, and re-indented.
pub fn preview(reader: Option<ScreenReader>, screen: &[&str], recorded: &[String]) -> Option<Turn> {
    let block = reader?(screen)?;
    let live = fingerprint(&block.text);
    if live.is_empty() {
        return None;
    }
    let known = recorded.iter().map(|t| fingerprint(t)).any(|known| {
        if block.clipped {
            known.contains(&live)
        } else {
            known.starts_with(&live)
        }
    });
    if known {
        return None;
    }
    let mut turn = Turn::new(LIVE_ID, Role::Assistant, None);
    turn.blocks.push(Block::md(block.text));
    Some(turn)
}

/// A turn under the live id carrying nothing, which is how a preview is withdrawn.
pub fn retired() -> Turn {
    Turn::new(LIVE_ID, Role::Assistant, None)
}

/// Letters and digits, lowercased, and nothing else — with a link's target dropped first.
///
/// The screen and the transcript never agree character for character — one is `**Bullet one —
/// structure.**` and the other is `Bullet one — structure.` re-wrapped at the viewport — so any
/// comparison that keeps punctuation, emphasis markers or whitespace answers "different" for text
/// that is plainly the same message. A link is the one case punctuation alone does not cover:
/// Codex writes `Created [notes.md](/tmp/…/notes.md)` and paints `Created notes.md`, and the
/// target is real alphanumeric text that only ever exists on one of the two sides.
fn fingerprint(text: &str) -> String {
    flatten_links(text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn flatten_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        out.push_str(&rest[..at]);
        rest = &rest[at + 2..];
        match rest.find(')') {
            Some(end) => rest = &rest[end + 1..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn opened_by(line: &str, marker: char) -> Option<&str> {
    let rest = line.strip_prefix(marker)?;
    match rest.is_empty() || rest.starts_with(' ') {
        true => Some(rest.trim_start()),
        false => None,
    }
}

fn starts_with(line: &str, marker: char) -> bool {
    opened_by(line, marker).is_some()
}

fn dedent(line: &str, indent: usize) -> &str {
    let strip = line.bytes().take(indent).take_while(|b| *b == b' ').count();
    &line[strip..]
}

/// What one poll of the screen decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Nothing to send. Either the screen has not moved, or the block under it has not yet earned
    /// a preview.
    Held,
    Show(Turn),
    /// Withdraw the preview currently on the client: a turn under the live id carrying nothing.
    Retire,
}

/// One pane's preview across polls, and the rule that keeps a harness's own chatter off the wire.
///
/// **A block earns a preview by growing.** Claude 2.1.239 opens an in-flight tool card, a
/// background-command notice and a real answer with the same `●`, so no vocabulary separates them
/// — but only one of the three is longer on the next poll than it was on this one. Waiting one
/// poll costs a fifth of a second on a message that takes seconds to write, and it is the only
/// rule here that does not depend on knowing a particular harness's screen.
#[derive(Debug, Default)]
pub struct Watch {
    seen: Option<String>,
    sent: Option<String>,
}

impl Watch {
    pub fn observe(&mut self, preview: Option<Turn>) -> Change {
        let Some(turn) = preview else {
            return self.stop();
        };
        let text = match turn.blocks.first() {
            Some(Block::Md { text, .. }) => text.clone(),
            _ => return self.stop(),
        };
        let moving = self.seen.as_deref().is_some_and(|seen| advanced(seen, &text));
        let static_block = self.seen.as_deref() == Some(text.as_str());
        self.seen = Some(text.clone());
        if !moving {
            // Unchanged, so still whatever it was — a one-line notice that never becomes a
            // message, or a published message that has stopped growing and is waiting for its
            // record. Anything else is a block this has never watched move, and whatever is on
            // the client belongs to the block that just left.
            return if static_block {
                Change::Held
            } else {
                self.withdraw()
            };
        }
        if self.sent.as_deref() == Some(text.as_str()) {
            return Change::Held;
        }
        self.sent = Some(text);
        Change::Show(turn)
    }

    /// The pane stopped working, or its transcript went away: whatever is shown is withdrawn and
    /// the next block starts from nothing.
    pub fn stop(&mut self) -> Change {
        self.seen = None;
        self.withdraw()
    }

    pub fn showing(&self) -> bool {
        self.sent.is_some()
    }

    fn withdraw(&mut self) -> Change {
        match self.sent.take() {
            Some(_) => Change::Retire,
            None => Change::Held,
        }
    }
}

/// Whether `text` is the same block as `seen`, further on.
///
/// A message shorter than the pane simply extends. One longer than the pane **slides**: its header
/// scrolls off the top while new lines arrive at the bottom, so successive views share a middle
/// and not a prefix — and treating that as a new block withdraws a preview in the middle of the
/// message it is previewing. The last line already seen is the anchor, because it is the one line
/// that must still be on screen if this is the same message.
fn advanced(seen: &str, text: &str) -> bool {
    if text == seen {
        return false;
    }
    if text.len() > seen.len() && text.starts_with(seen) {
        return true;
    }
    match seen.lines().rev().find(|l| l.trim().len() >= ANCHOR) {
        Some(anchor) => text.contains(anchor),
        None => false,
    }
}

/// How much of a line has to match before it can anchor a slide. A whole wrapped row is far longer
/// than this; a stray `-` being typed at the head of a list item is not.
const ANCHOR: usize = 16;
