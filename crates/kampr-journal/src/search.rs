use crate::model::{Block, Role, Turn};

/// What a transcript search matched, on the machine that holds the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvoFound {
    /// Every matching turn in the whole transcript, not the ones listed. A client says both, the
    /// way it does for the scrollback search beside this one.
    pub total: u32,
    /// Newest first, and capped: the reader stands at the newest end and pages backwards from it.
    pub hits: Vec<ConvoHit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvoHit {
    pub turn: String,
    pub role: Role,
    pub at: Option<String>,
    /// Turns between this one and the newest. It is what lets a client say how far back a hit is
    /// without holding it, and what bounds the paging it has to do to reach one.
    pub from_end: u32,
    /// How many times this turn holds the query. A turn is one result however often it says it.
    pub hits: u32,
    /// The line the first match is on, clipped around the match. What makes a hit deeper than the
    /// client's own window worth listing: it can be read where it cannot yet be scrolled to.
    pub text: String,
}

/// The shortest query worth answering, and the same floor the client's own search keeps: a single
/// character matches most of a transcript and none of it usefully.
const FLOOR: usize = 2;

/// Characters of the matched line to hand over. Wide enough to read the match in its sentence,
/// narrow enough that fifty of them are not a page of prose — a tool's output is one `text` field
/// and can be thousands of columns.
const EXCERPT: usize = 200;

/// Characters of the line kept in front of the match when the line is clipped.
const LEAD: usize = 60;

/// **Every piece of text a turn shows, handed to `visit` one at a time.**
///
/// Pieces rather than one joined string, because joining allocated the whole rendered transcript
/// on every search: 380 ms over the operator's own 2459-turn session, with the pane's conversation
/// lock held for all of it (probe #541). A query that spans two blocks no longer matches, which is
/// also what the screen does — a match is highlighted inside one block or not at all.
///
/// What each block contributes is what the client *draws* of it: a card's own header rather than
/// the marker it replaced, a tool's label rather than its output, a launched conversation's type
/// and title rather than its turns — those belong to another transcript and are not on this screen
/// to be found. `Search.kt`'s `blockText` is the other half of this and the two have to agree.
fn said_by(turn: &Turn, visit: &mut impl FnMut(&str)) {
    for block in &turn.blocks {
        match block {
            Block::Md { text, att: None } => visit(text),
            Block::Md { att: Some(att), .. } => {
                if let Some(name) = &att.name {
                    visit(name);
                }
                if let Some(mime) = &att.mime {
                    visit(mime);
                }
            }
            Block::Code { text, .. } => visit(text),
            Block::Diff { path, text } => {
                if let Some(path) = path {
                    visit(path);
                }
                visit(text);
            }
            Block::Tool { name, summary, .. } => {
                visit(name);
                if let Some(summary) = summary {
                    visit(summary);
                }
            }
            Block::Sub { kind, title, .. } => {
                if let Some(kind) = kind {
                    visit(kind);
                }
                if let Some(title) = title {
                    visit(title);
                }
            }
        }
    }
}

pub fn search_turns(turns: &[Turn], query: &str, cap: usize) -> ConvoFound {
    if query.chars().count() < FLOOR {
        return ConvoFound {
            total: 0,
            hits: Vec::new(),
        };
    }
    let lowered: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut total = 0;
    let mut hits = Vec::new();
    let last = turns.len().saturating_sub(1);
    for (index, turn) in turns.iter().enumerate().rev() {
        // Known before the turn is read, because the list only grows: a turn past the cap is
        // counted and nothing else, so it is scanned as far as its first match and no further.
        // Everything else here — the occurrence count, the excerpt — is work only the listed
        // fifty are worth, and doing it for all of them was the rest of the 380 ms (#541).
        let listing = hits.len() < cap;
        let mut found = 0u32;
        let mut line: Option<String> = None;
        said_by(turn, &mut |said| {
            if found > 0 && !listing {
                return;
            }
            let mut from = 0;
            while let Some((at, len)) = next_match(said, query, &lowered, from) {
                if found == 0 {
                    if !listing {
                        found = 1;
                        return;
                    }
                    line = Some(excerpt(said, at));
                }
                found += 1;
                from = at + len.max(1);
            }
        });
        if found == 0 {
            continue;
        }
        total += 1;
        if listing {
            hits.push(ConvoHit {
                turn: turn.id.clone(),
                role: turn.role,
                at: turn.at.clone(),
                from_end: (last - index) as u32,
                hits: found,
                text: line.unwrap_or_default(),
            });
        }
    }
    ConvoFound { total, hits }
}

/// The next case-insensitive occurrence at or after `from`, as a byte offset and its length.
///
/// Two paths, and the ASCII one is the whole of why this is not a `str::contains` loop over a
/// lowercased copy: `to_lowercase` is not length-preserving, so the offsets an excerpt is cut at
/// have to be offsets into the text it is cut from. Bytes are scanned for the first character in
/// either case and only then compared, because per-character `to_lowercase()` over tens of
/// megabytes is where the 380 ms went (#541).
fn next_match(haystack: &str, query: &str, lowered: &[char], from: usize) -> Option<(usize, usize)> {
    if haystack.len() < query.len() || from >= haystack.len() {
        return None;
    }
    // The needle's own alphabet decides, not the haystack's. UTF-8 puts every byte of a
    // multi-byte character above 0x7f, so an ASCII needle can only match at an ASCII byte and can
    // never straddle a character — and every offset it returns is a character boundary. Asking
    // the *haystack* to be ASCII too was the whole of the remaining cost: a transcript is full of
    // em dashes and box drawing, so nearly every piece of it fell to the character-wise path
    // (#541).
    if query.is_ascii() {
        let hay = haystack.as_bytes();
        let needle = query.as_bytes();
        // Candidates come off `memchr2`, a vector scan for the first character in either case.
        // Worth the dependency and measured to be: 45 ms for a hand-rolled byte loop against 28 ms
        // for this, over the operator's own 28 MiB session searched for a word that is not in it —
        // the query that has to read all of it (#541).
        let (lower, upper) = (needle[0].to_ascii_lowercase(), needle[0].to_ascii_uppercase());
        let end = hay.len() - needle.len();
        let mut at = from;
        while at <= end {
            at += memchr::memchr2(lower, upper, &hay[at..=end])?;
            if hay[at..at + needle.len()].eq_ignore_ascii_case(needle) {
                return Some((at, needle.len()));
            }
            at += 1;
        }
        return None;
    }
    let mut at = from;
    while at < haystack.len() {
        if let Some(len) = matches_at(&haystack[at..], lowered) {
            return Some((at, len));
        }
        at += haystack[at..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// The byte length of the match starting here, when the query is what starts here.
fn matches_at(haystack: &str, lowered: &[char]) -> Option<usize> {
    let mut want = lowered.iter();
    let mut len = 0;
    for c in haystack.chars() {
        for got in c.to_lowercase() {
            match want.next() {
                Some(&expected) if expected == got => {}
                Some(_) => return None,
                None => return Some(len),
            }
        }
        len += c.len_utf8();
    }
    want.next().is_none().then_some(len)
}

/// The line the match is on, clipped around it. `at` is a byte offset into `text`.
fn excerpt(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |nl| nl + 1);
    let end = text[at..].find('\n').map_or(text.len(), |nl| at + nl);
    let line = text[start..end].trim();
    if line.chars().count() <= EXCERPT {
        return line.to_string();
    }
    // Counted from the match rather than from the line, so a hit four thousand columns in is still
    // in the middle of what comes back.
    let line_start = start + text[start..end].len() - text[start..end].trim_start().len();
    let lead: usize = text[line_start..at]
        .chars()
        .rev()
        .take(LEAD)
        .map(char::len_utf8)
        .sum();
    let from = at - lead;
    let cut: String = text[from..end].chars().take(EXCERPT - 2).collect();
    let mut out = String::new();
    if from > line_start {
        out.push('…');
    }
    out.push_str(cut.trim_end());
    if from + cut.len() < end {
        out.push('…');
    }
    out
}
