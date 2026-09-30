use crate::live::Layout;

/// Where the terminal's caret sits on the visible grid, in cells.
///
/// **The caret is not decoration here; it is the measurement the whole read turns on.** Claude
/// 2.1.250 paints a rotating hint into an empty composer (`Try "refactor <filepath>"`) and Codex
/// 0.149.1 a fixed one (`Ask Codex to do anything`), in the very cells the operator's own words
/// would occupy — so nothing in the text separates a composer somebody is typing into from one
/// the harness is advertising itself in. The caret does: it rests at the input column while the
/// hint is showing and moves along with real typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub col: u16,
    pub row: u16,
}

/// What the grid says about itself beyond its text, which a composer read turns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub cols: u16,
    /// Every glyph right of the caret on its row is drawn faint. Claude's `Try "…"` hint and
    /// Codex's `Ask Codex to do anything` are SGR 2 in the cells the operator's words would take
    /// (#565); the operator's own words are not, whether or not `ctrl+a` has put the caret in
    /// front of them.
    pub faint_after_caret: bool,
}

/// The keys measured to edit a harness's box one character at a time, which is what lets a reply
/// box be the same line as the box rather than a second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EditKeys {
    pub back: &'static str,
    pub left: &'static str,
    pub right: &'static str,
    pub newline: &'static str,
}

/// Claude 2.1.285, Codex and agy alike (#564): one `\x7f` takes the character before the
/// caret, `←`/`→` move one character and cross a wrapped row, text lands at the caret, and a lone
/// `\n` writes a line without submitting.
pub const EDIT_KEYS: EditKeys = EditKeys {
    back: "\u{7f}",
    left: "\u{1b}[D",
    right: "\u{1b}[C",
    newline: "\n",
};

/// What one harness's box has been measured to do.
#[derive(Debug, Clone, Copy)]
pub struct Measured {
    /// Columns the harness keeps free right of its text before it wraps (#558).
    pub margin: usize,
    pub clear: Option<&'static str>,
    pub keys: Option<EditKeys>,
}

/// What the operator has typed into the harness's box and has not sent, where the caret is in it,
/// and the keystrokes measured to change it.
///
/// `text` is empty for a box that is drawn with nothing in it, which is a box a client can still
/// type into — as against no reading at all, which is a harness that is not taking keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    pub text: String,
    /// In characters from the front of `text`.
    pub caret: usize,
    /// `None` for a harness whose clearing keystroke has not been measured — which is a takeover
    /// that is not offered, never one that is guessed at.
    pub clear: Option<&'static str>,
    pub keys: Option<EditKeys>,
}

/// Reads one harness's composer. A bare fn for the same reason [`crate::ScreenReader`] is one: it
/// keeps no state, and every call sees the whole grid.
pub type ComposerReader = fn(&[&str], Caret, Frame) -> Option<Composed>;

/// Whether one harness is reading its keys yet. See [`listening`].
pub type ListeningReader = fn(&[&str], Caret) -> bool;

/// The operator's unsent line, or `None` when no composer is drawn with the caret in it.
///
/// Runs *downwards* from the composer marker, which is the opposite of the live preview beside it
/// and is not the same reading: that one lifts the block the harness is painting above the
/// composer, this one lifts what a person has typed into it.
///
/// The walk gathers the marked row and every wrapped continuation under it, and then insists the
/// caret lands inside what it gathered. That last check is what makes a partial read impossible:
/// Codex paints its model and directory two columns in, one blank row below the box, and a walk
/// that ran on into it would hand back a sentence with a path glued to the end.
///
/// **A row break is one of three things and the widths say which** (#564). All three
/// harnesses wrap at a word and draw nothing for the space they broke at, and a newline the
/// operator wrote looks the same on the screen. A row filled to the wrap column broke at a space
/// if it holds one, and inside a word too long for any row if it does not; one whose next row's
/// first word would not have fitted after it broke at a space; and one that stopped short of a
/// word that would have fitted was ended by the operator.
pub fn read(
    screen: &[&str],
    caret: Caret,
    frame: Frame,
    layout: &Layout,
    measured: &Measured,
) -> Option<Composed> {
    let (head, last) = holding_caret(screen, caret, layout)?;
    let caret_row = caret.row as usize;
    let composed = |text: String, caret: usize| Composed {
        text,
        caret,
        clear: measured.clear,
        keys: measured.keys,
    };
    if caret_row == head && caret.col as usize <= layout.input && frame.faint_after_caret {
        return Some(composed(String::new(), 0));
    }
    let limit = (frame.cols as usize).saturating_sub(measured.margin + 1);
    let rows: Vec<(usize, String)> = (head..=last)
        .map(|at| {
            let start = if at == head { layout.input } else { layout.indent };
            let mut seg: String = screen[at].chars().skip(start).collect();
            let kept = seg.trim_end().chars().count();
            let reach = match at == caret_row {
                true => kept.max((caret.col as usize).saturating_sub(start)),
                false => kept,
            };
            seg = seg.chars().chain(std::iter::repeat(' ')).take(reach).collect();
            (start, seg)
        })
        .collect();
    let mut text = String::new();
    let mut at = 0;
    for (index, (start, seg)) in rows.iter().enumerate() {
        let len = seg.chars().count();
        if head + index == caret_row {
            at = text.chars().count() + (caret.col as usize).saturating_sub(*start).min(len);
        }
        text.push_str(seg);
        if let Some((_, next)) = rows.get(index + 1) {
            let end = start + len.max(1) - 1;
            let word = next.chars().take_while(|c| !c.is_whitespace()).count();
            match end >= limit {
                true if seg.contains(char::is_whitespace) => text.push(' '),
                true => {}
                false if end + 1 + word > limit => text.push(' '),
                false => text.push('\n'),
            }
        }
    }
    Some(composed(text, at))
}

/// A composer is drawn and the caret is in it: the harness is reading its keys. A reply written to
/// Claude before this is true does not submit, and one written after it does (#535).
pub fn listening(screen: &[&str], caret: Caret, layout: &Layout) -> bool {
    holding_caret(screen, caret, layout).is_some()
}

/// The composer's rows — its marked row and every wrapped continuation under it — when the caret
/// is on one of them.
fn holding_caret(screen: &[&str], caret: Caret, layout: &Layout) -> Option<(usize, usize)> {
    let head = screen.iter().rposition(|line| opens(line, layout.prompt))?;
    let mut last = head;
    for (at, line) in screen.iter().enumerate().skip(head + 1) {
        if !is_continuation(line, layout.indent) {
            break;
        }
        last = at;
    }
    (head..=last)
        .contains(&(caret.row as usize))
        .then_some((head, last))
}

/// Claude separates its `❯` from the text with a **non-breaking space** where Codex and agy use an
/// ordinary one, so a matcher that knew only `' '` would not recognise Claude's composer at all
/// the moment anything was typed into it.
fn opens(line: &str, marker: char) -> bool {
    match line.strip_prefix(marker) {
        Some(rest) => rest.is_empty() || rest.starts_with([' ', '\u{a0}']),
        None => false,
    }
}

fn is_continuation(line: &str, indent: usize) -> bool {
    line.bytes().take(indent).filter(|b| *b == b' ').count() == indent && !line.trim().is_empty()
}

/// One pane's desk line across polls, and the rule that keeps an idle composer off the wire.
///
/// **The comparison is the point**, exactly as it is in [`crate::FacetFeed`]: a conversation is
/// polled several times a second and a desk line moves only when somebody at the keyboard moves
/// it, so publishing every poll would be a frame per tick per pane for a string that had not
/// changed. The first look at a pane with no composer drawn is silence too — it says the same
/// thing as never having sent anything at all. A drawn empty one is not: it is a box a client can
/// start typing into.
#[derive(Debug, Default)]
pub struct ComposerFeed {
    last: Option<Composed>,
    sent: bool,
}

impl ComposerFeed {
    /// The line as it is now, or `None` when nothing has moved since the last call. The inner
    /// `None` is a composer that has just stopped being drawn, which the client has to be told
    /// about.
    ///
    /// **The whole of [`Composed`] is compared, not only its words.** A pane whose agent is quit
    /// and a different one started in its place can hold the same half-sentence it held before,
    /// and the keystroke that clears it is not the same keystroke — `ctrl+u` empties Codex's box
    /// and takes one visual row of Claude's, and `ctrl+c` empties Claude's and arms an *exit* on
    /// agy. Comparing the text alone would leave the client holding the old harness's key.
    pub fn moved(&mut self, now: Option<Composed>) -> Option<Option<Composed>> {
        if now == self.last && self.sent {
            return None;
        }
        if now.is_none() && !self.sent {
            return None;
        }
        self.last = now.clone();
        self.sent = true;
        Some(now)
    }
}
