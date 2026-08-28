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

/// What the operator has typed at the desk and has not sent, and the keystroke measured to take
/// it back off the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    pub text: String,
    /// `None` for a harness whose clearing keystroke has not been measured — which is a takeover
    /// that is not offered, never one that is guessed at.
    pub clear: Option<&'static str>,
}

/// Reads one harness's composer. A bare fn for the same reason [`crate::ScreenReader`] is one: it
/// keeps no state, and every call sees the whole grid.
pub type ComposerReader = fn(&[&str], Caret) -> Option<Composed>;

/// The operator's unsent line, or `None` when the composer is empty or cannot be read.
///
/// Runs *downwards* from the composer marker, which is the opposite of the live preview beside it
/// and is not the same reading: that one lifts the block the harness is painting above the
/// composer, this one lifts what a person has typed into it.
///
/// The walk gathers the marked row and every wrapped continuation under it, and then insists the
/// caret lands inside what it gathered. That last check is what makes a partial read impossible:
/// Codex paints its model and directory two columns in, one blank row below the box, and a walk
/// that ran on into it would hand back a sentence with a path glued to the end.
pub fn read(screen: &[&str], caret: Caret, layout: &Layout, clear: Option<&'static str>) -> Option<Composed> {
    let head = screen.iter().rposition(|line| opens(line, layout.prompt))?;
    let mut last = head;
    for (at, line) in screen.iter().enumerate().skip(head + 1) {
        if !is_continuation(line, layout.indent) {
            break;
        }
        last = at;
    }
    let caret_row = caret.row as usize;
    if caret_row < head || caret_row > last {
        return None;
    }
    // The caret resting where the operator's first character would go is an empty composer,
    // whatever is painted to the right of it. It is also where `ctrl+a` leaves the caret on all
    // three harnesses with the line still full, which is a line this reports nothing for rather
    // than one it reports wrongly.
    if caret_row == head && caret.col as usize <= layout.input {
        return None;
    }
    let mut text = String::new();
    text.push_str(screen[head].strip_prefix(layout.prompt)?.trim_end());
    for line in &screen[head + 1..=last] {
        text.push_str(dedent(line, layout.indent).trim_end());
    }
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(Composed { text, clear })
}

/// Claude separates its `❯` from the text with a **non-breaking space** where Codex and agy use an
/// ordinary one, so a matcher that knew only `' '` would not recognise Claude's composer at all
/// the moment anything was typed into it. The separator itself is left on the front and taken off
/// by the trim that closes [`read`], U+00A0 being whitespace like any other.
fn opens(line: &str, marker: char) -> bool {
    match line.strip_prefix(marker) {
        Some(rest) => rest.is_empty() || rest.starts_with([' ', '\u{a0}']),
        None => false,
    }
}

fn is_continuation(line: &str, indent: usize) -> bool {
    line.bytes().take(indent).filter(|b| *b == b' ').count() == indent && !line.trim().is_empty()
}

fn dedent(line: &str, indent: usize) -> &str {
    let strip = line.bytes().take(indent).take_while(|b| *b == b' ').count();
    &line[strip..]
}

/// One pane's desk line across polls, and the rule that keeps an idle composer off the wire.
///
/// **The comparison is the point**, exactly as it is in [`crate::FacetFeed`]: a conversation is
/// polled several times a second and a desk line moves only when somebody at the keyboard moves
/// it, so publishing every poll would be a frame per tick per pane for a string that had not
/// changed. The first look at an empty composer is silence too — it says the same thing as never
/// having sent anything at all.
#[derive(Debug, Default)]
pub struct ComposerFeed {
    last: Option<Composed>,
    sent: bool,
}

impl ComposerFeed {
    /// The line as it is now, or `None` when nothing has moved since the last call. The inner
    /// `None` is a composer that has just been emptied, which the client has to be told about.
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
