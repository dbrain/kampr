//! W8 — the conversation view, the pending strip, and the reply box.
//!
//! A pane opens on its **terminal**; the conversation is a view the operator asks for with
//! `prefix shift+v`, offered on the pane entry's `converses` — the adapter half of the question,
//! true from the moment the harness opens rather than from the moment it first writes.

mod composer;
mod markdown;
mod search;
mod stamps;

pub use composer::{Composer, Typed};
pub use search::{Search, Took as Searched};

use crate::image::{Attachment, Images};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use kampr_client::{Event, Pending, Role};
use kampr_core::wire::ConvoMatch;
use markdown::markdown;
use pulldown_cmark::Alignment;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde::Deserialize;
use serde_json::Value;
use stamps::when;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// The node's reserved id for the message a harness is still writing. It is scraped off the
/// pane's screen — hard-wrapped, markdown rendered away, clipped at the top once it outgrows the
/// pane — so it is never paged from, never cited, and never kept across a reload.
const LIVE: &str = "live";

/// The one `turn.kind` the wire carries.
const COMPACT: &str = "compact";

/// The one `code.role` the wire carries: the block is the call's result rather than its input.
const OUTPUT: &str = "output";

/// Kampr's own `kind`, and the one that never rides the wire: the node sends the queue as
/// `convo.facets` and this is what a folded prompt is called once it is a turn.
const QUEUED: &str = "queued";

/// What an attachment is allowed to take of a pane once its bytes have actually landed. A header
/// that never resolves keeps the single row its marker text already needed.
const IMAGE_ROWS: u16 = 12;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct Att {
    id: String,
    kind: String,
    mime: Option<String>,
    bytes: Option<u64>,
    name: Option<String>,
}

impl Att {
    fn header(&self) -> Attachment {
        Attachment {
            id: self.id.clone(),
            kind: self.kind.clone(),
            mime: self.mime.clone(),
            bytes: self.bytes,
            name: self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "b", rename_all = "lowercase")]
pub(super) enum Block {
    Md {
        #[serde(default)]
        text: String,
        #[serde(default)]
        att: Option<Att>,
    },
    Code {
        #[serde(default)]
        lang: Option<String>,
        #[serde(default)]
        text: String,
        /// Additive, and `output` is its one value today: this block is the call's own result
        /// rather than its input. An open string rather than an enum, so a value this build has
        /// never heard of renders as the code block it already renders instead of failing a page.
        #[serde(default)]
        role: Option<String>,
    },
    Tool {
        #[serde(default)]
        name: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        lines: Option<u32>,
        #[serde(default)]
        state: String,
    },
    Diff {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        text: String,
    },
    /// A conversation this turn **launched**, offered for opening rather than spoken here. Its
    /// own turns are deliberately not inlined: drawing them under this turn would be saying the
    /// pane's agent said what a subagent said.
    Sub {
        id: String,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    /// A `b` this build has never heard of. The wire is additive, so an unknown block is ignored
    /// rather than failing the page it arrived in.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
struct Turn {
    id: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    at: Option<String>,
    /// Additive, and absent on every turn but one. A `kind` this build has never heard of is a
    /// turn like any other, which is what lets a later one ship without a client release.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    blocks: Vec<Block>,
}

impl Turn {
    /// A turn with no blocks is **withdrawn**, not empty: rendering it leaves a blank card where
    /// the preview was. It stays in the list because a page merges by id, and a withdrawn id is
    /// still the anchor a later page positions against.
    fn visible(&self) -> bool {
        !self.blocks.is_empty()
    }

    /// `user` means a person typed this. The node has already taken the harness's own text out —
    /// 45% of what claude files under a user role is the harness talking to itself (#286) — so
    /// what arrives is trusted rather than filtered again.
    fn person(&self) -> bool {
        self.role == "user" && !self.summary()
    }

    /// The harness's own summary of the conversation it dropped, which `/compact` files under a
    /// **user** record (#259). Nobody spoke it and nobody typed it.
    fn summary(&self) -> bool {
        self.kind.as_deref() == Some(COMPACT)
    }

    /// A prompt the harness has taken and not yet answered. **Not a record and not this client's
    /// guess**: it is folded from the harness's own `queue-operation` records, so it stands for a
    /// prompt typed at the desk exactly as it does for one sent from here.
    fn queued(&self) -> bool {
        self.kind.as_deref() == Some(QUEUED)
    }

    /// **What a client draws**, which is what a search over it may promise: a card's own header
    /// rather than the marker it replaced, a tool's label rather than its output, a launched
    /// conversation's type and title rather than its turns — those belong to another transcript
    /// and are not on this screen to be found.
    fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| match block {
                Block::Md { text, att: None } => text.clone(),
                Block::Md { att: Some(att), .. } => [att.name.clone(), att.mime.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<String>>()
                    .join(" "),
                Block::Code { text, .. } => text.clone(),
                Block::Tool { name, summary, .. } => match summary {
                    Some(summary) => format!("{name} {summary}"),
                    None => name.clone(),
                },
                Block::Diff { path, text } => match path {
                    Some(path) => format!("{path}\n{text}"),
                    None => text.clone(),
                },
                Block::Sub { kind, title, .. } => headline(kind.as_deref(), title.as_deref()),
                Block::Unknown => String::new(),
            })
            .collect::<Vec<String>>()
            .join("\n")
    }

    fn lines(&self) -> usize {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Md { text, .. } => Some(text.lines().count()),
                _ => None,
            })
            .sum()
    }
}

pub(super) enum Piece {
    Line(Line<'static>),
    /// The rule above a code block, carrying the block's **own** text. A copy takes this rather
    /// than the rows that were painted: those are clipped at the pane's width, and a command
    /// pasted with its tail missing is worse than one that could not be copied at all.
    Code {
        head: Line<'static>,
        text: String,
    },
    Image {
        att: Attachment,
        marker: Line<'static>,
        /// The second id form: a path a tool call named, asked for by path rather than by a
        /// record id. `None` is an ordinary `att` header off an `md` block.
        file: Option<String>,
    },
}

/// The rectangles this surface drew that a click resolves against.
#[derive(Debug, Clone, Default)]
pub struct Marks {
    /// A pending prompt's option chips, in the order the prompt offered them.
    pub options: Vec<Rect>,
    /// An attachment's marker row, by id.
    pub attachments: Vec<(String, Rect)>,
}

struct Laid {
    width: u16,
    revision: u64,
    /// Where each turn starts, so a search hit can be aimed at by the id it names rather than by
    /// a row number that means nothing until the page holding it has landed.
    heads: Vec<(String, usize)>,
    /// A read-only device is never offered a path out of a tool call — the node answers one
    /// `403` — so the affordance is laid against the **live** role and re-laid when it moves.
    writes: bool,
    pieces: Vec<Piece>,
}

#[derive(Default)]
struct Transcript {
    turns: Vec<Turn>,
    cursor: Option<String>,
    more: bool,
    pending: Option<Pending>,
    /// What the operator has half-typed at the pane's own keyboard and not sent. `input` is
    /// `pane.send_text` and **appends**, so a reply sent from here joins onto this — which is
    /// invisible unless it is said. Empty arrives as `text: null` and takes the line down, the
    /// same shape `pending` uses, because neither has a resolved event.
    desk: Option<String>,
    /// The keystroke measured to empty **that harness's** composer, carried rather than looked up
    /// because the three do not agree and a guessed key deletes part of somebody's sentence or
    /// quits their agent. Absent where nobody has measured one, and then no takeover is offered.
    clear: Option<String>,
    /// Lines held back from the bottom. Zero is pinned to the newest turn, which is where a
    /// conversation belongs.
    scroll: usize,
    revision: u64,
    /// Attachment ids whose bytes are here and inline-drawable. A header that resolves earns its
    /// rows; one that answered `404` — expected, since an id names a record in a transcript —
    /// keeps the one row its marker needs.
    inline: HashSet<String>,
    /// What each attachment's marker says beside it — `too large`, a size, an invitation to
    /// save. Filled from [`Images::offer`] as the fetches land.
    notes: HashMap<String, String>,
    requested: HashSet<String>,
    /// A turn a search is aiming at, standing until the page holding it has landed and been laid
    /// out: a hit forty turns back names a turn this client does not hold yet, and the walk to it
    /// is pages arriving one at a time.
    aiming: Option<String>,
    /// The summaries the reader has opened. A summary is drawn shut, so this is the departure
    /// from the default rather than the state of every turn.
    open: HashSet<String>,
    /// What the harness has queued, held apart from the turns because it is not a record: it is
    /// republished whole whenever it moves, so it is replaced rather than merged.
    queued: Vec<Turn>,
    laid: Option<Laid>,
}

impl Transcript {
    fn at(&self, id: &str) -> Option<usize> {
        self.turns.iter().position(|t| t.id == id)
    }

    /// A page **merges by id**: a turn whose id is already held is replaced in place, and one
    /// that is not goes where the page puts it — after the last id the two have in common,
    /// before the next. Sharing nothing at all is the only case where position is a guess, and
    /// there the page is prepended whole.
    ///
    /// Unconditional prepending was the older rule. It is right for `convo.load`, which pages
    /// backwards, and wrong for a transcript re-read after a pump restart, which pages forwards:
    /// the turns the client is missing are then the *newest*, every one of them lands at the top
    /// of a view scrolled to the bottom, and none is ever seen — the node has already recorded
    /// them as delivered.
    ///
    /// The order is the page's, never the stamps': a resumed session carries records timed
    /// before the ones above them, and sorting on `at` shuffles a real conversation.
    fn merge(&mut self, turns: Vec<Turn>) {
        let mut after: Option<usize> = None;
        let mut waiting: Vec<Turn> = Vec::new();
        for turn in turns {
            let Some(at) = self.at(&turn.id) else {
                waiting.push(turn);
                continue;
            };
            self.turns[at] = turn;
            after = Some(at);
            if !waiting.is_empty() {
                let held = waiting.len();
                for (offset, turn) in waiting.drain(..).enumerate() {
                    self.turns.insert(at + offset, turn);
                }
                after = Some(at + held);
            }
        }
        let at = after.map(|a| a + 1).unwrap_or(0);
        for (offset, turn) in waiting.into_iter().enumerate() {
            self.turns.insert(at + offset, turn);
        }
        self.touch();
    }

    /// `convo.turn` appends what it does not hold and revises in place what it does. A tool turn
    /// whose result has landed arrives this way and **must not** be appended, or every tool
    /// renders twice.
    fn revise(&mut self, turns: Vec<Turn>) {
        for turn in turns {
            match self.at(&turn.id) {
                Some(at) => self.turns[at] = turn,
                None => self.turns.push(turn),
            }
        }
        self.touch();
    }

    fn touch(&mut self) {
        self.revision += 1;
        self.laid = None;
    }

    /// Opens every summary this transcript holds, or puts every one of them away, and says whether
    /// that changed anything. **Nothing to move is not consumed**: the two keys this costs are the
    /// agent's own everywhere else on the surface.
    fn unfold(&mut self, open: bool) -> bool {
        let ids: Vec<String> = self
            .turns
            .iter()
            .filter(|t| t.summary() && t.visible() && self.open.contains(&t.id) != open)
            .map(|t| t.id.clone())
            .collect();
        if ids.is_empty() {
            return false;
        }
        for id in ids {
            match open {
                true => self.open.insert(id),
                false => self.open.remove(&id),
            };
        }
        self.touch();
        true
    }

    /// The newest code block at or above the bottom edge of the view. **Not "on screen"**: a
    /// block the reader has scrolled past the top of is still the one they were last looking at,
    /// and only what lies below the fold is a block they have not reached yet.
    fn code(&self) -> Option<&str> {
        let laid = self.laid.as_ref()?;
        let bottom = laid.pieces.len().saturating_sub(self.scroll);
        laid.pieces
            .iter()
            .take(bottom)
            .rev()
            .find_map(|piece| match piece {
                // **Without the newline a fence ends with.** A command copied with its return still
                // on it is a command the paste *runs*, in whatever shell the operator pasted it into,
                // and the wire's own `code` blocks do not carry one either.
                Piece::Code { text, .. } => Some(text.trim_end_matches('\n')),
                _ => None,
            })
    }

    fn launches(&self) -> Vec<Launch> {
        self.turns
            .iter()
            .filter(|turn| turn.visible())
            .flat_map(|turn| turn.blocks.iter())
            .filter_map(|block| match block {
                Block::Sub { id, kind, title } => Some(Launch {
                    id: id.clone(),
                    head: headline(kind.as_deref(), title.as_deref()),
                }),
                _ => None,
            })
            .collect()
    }

    fn showing(&self) -> bool {
        self.turns.iter().chain(self.queued.iter()).any(Turn::visible)
    }

    fn height(&self) -> usize {
        self.laid.as_ref().map(|l| l.pieces.len()).unwrap_or(0)
    }
}

/// A conversation the pane's agent launched, as a `sub` block offers it: the handle to hand back
/// and the words the card shows for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub id: String,
    pub head: String,
}

/// What a page-back at the top of the transcript on screen is a page of. The pane's own
/// conversation and a launched one are paged by **different verbs** — `convo.load` and
/// `convo.sub` — so a client that kept one cursor would ask the wrong node question at the top of
/// a subagent's transcript.
/// What aiming the transcript at a turn took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aimed {
    /// It is held; the next frame is on it.
    Shown,
    /// It is older than what is held: this is the cursor to page back with, and the aim stands
    /// until the page that holds it lands.
    Paging(String),
    /// Older than what is held, with nothing left to page with.
    Gone,
}

/// The line a match is on, clipped around it rather than shipped whole — a tool's output is one
/// block and can be thousands of columns.
fn line_of(text: &str, needle: &str) -> String {
    let lower = text.to_lowercase();
    let at = lower.find(needle).unwrap_or(0);
    text.lines()
        .find(|line| line.to_lowercase().contains(needle))
        .map(|line| line.trim().chars().take(160).collect())
        .unwrap_or_else(|| text[at..].chars().take(160).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum More {
    Pane(String),
    Launched { id: String, before: String },
}

/// One a reader has open. **One per pane and never a stack**: the node follows a single launched
/// conversation at a time and asking for another replaces it, so a client holding two would be
/// drawing a transcript nothing is feeding.
struct Reading {
    id: String,
    head: String,
    held: Transcript,
}

#[derive(Debug, Default)]
pub struct Convo {
    panes: HashMap<String, Transcript>,
    launched: HashMap<String, Reading>,
}

impl std::fmt::Debug for Reading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reading").field("id", &self.id).finish()
    }
}

fn headline(kind: Option<&str>, title: Option<&str>) -> String {
    let words: Vec<&str> = [kind, title]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect();
    match words.is_empty() {
        true => "a conversation this turn launched".to_string(),
        false => words.join(" · "),
    }
}

impl std::fmt::Debug for Transcript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transcript")
            .field("turns", &self.turns.len())
            .field("cursor", &self.cursor)
            .field("more", &self.more)
            .finish()
    }
}

impl Convo {
    pub fn new() -> Self {
        Self::default()
    }

    /// `convo`, `convo.turn` and `pending`. **A page merges by id and `fresh` replaces**; a turn
    /// with no blocks is withdrawn rather than empty; a `pending` with a null question clears the
    /// strip, because there is no resolved event.
    pub fn absorb(&mut self, event: &Event) {
        match event {
            Event::ConvoFacets { pane, facets } => {
                let held = self.panes.entry(pane.clone()).or_default();
                // Position is the only identity a queued prompt has — the harness records no id
                // and works the queue by position — so a prompt leaving the head renumbers what
                // is behind it. That costs a re-lay and nothing else, because none of these are
                // merged against anything.
                held.queued = facets
                    .queued
                    .iter()
                    .enumerate()
                    .map(|(at, prompt)| Turn {
                        id: format!("queued:{at}"),
                        role: "user".into(),
                        at: prompt.at.clone(),
                        kind: Some(QUEUED.into()),
                        blocks: vec![Block::Md {
                            text: prompt.text.clone(),
                            att: None,
                        }],
                    })
                    .collect();
                held.touch();
            }
            Event::Convo(page) => {
                // **A page wearing a handle is not this pane's conversation.** It is the one the
                // reader opened, and merging it into the pane's would put a subagent's words in
                // the pane agent's mouth — which is the reason the turns were never inlined.
                let Some(held) = self.receiving(&page.pane, page.sub.as_deref()) else {
                    return;
                };
                // `fresh` is the node saying it could not name the turns to withdraw: a socket it
                // has no history for, because this client reconnected or that node restarted.
                if page.fresh {
                    held.turns.clear();
                    held.scroll = 0;
                }
                held.merge(decode(&page.turns));
                held.cursor = page.cursor.clone();
                held.more = page.more;
            }
            Event::ConvoTurn { pane, turns, sub } => {
                let Some(held) = self.receiving(pane, sub.as_deref()) else {
                    return;
                };
                held.revise(decode(turns));
            }
            Event::ConvoComposer { pane, text, clear } => {
                let held = self.panes.entry(pane.clone()).or_default();
                held.desk = text.clone().filter(|line| !line.trim().is_empty());
                held.clear = clear.clone();
                held.laid = None;
            }
            Event::Pending(pending) => {
                let held = self.panes.entry(pending.pane.clone()).or_default();
                held.pending = pending.outstanding().then(|| pending.clone());
                held.laid = None;
            }
            // Nothing carried across a dropped socket is trustworthy, and a question least of
            // all: `pending` is published on a blocked-state edge, so the node's first attempt at
            // a pane that is *still* blocked carries nothing and a reconnect would otherwise
            // triage the previous connection's question — and answer a key into a pane with
            // nothing matching to answer it. The turns are kept; they are dated, not wrong.
            Event::Disconnected { .. } => {
                for held in self.panes.values_mut() {
                    held.pending = None;
                    held.laid = None;
                }
            }
            _ => {}
        }
    }

    /// Which transcript a frame belongs to. `None` is a frame for a launched conversation
    /// **nobody is reading** — the reader left while it was in flight — and it is dropped rather
    /// than filed anywhere, because the only other place to put it is a lie.
    fn receiving(&mut self, pane: &str, sub: Option<&str>) -> Option<&mut Transcript> {
        let Some(id) = sub else {
            return Some(self.panes.entry(pane.to_string()).or_default());
        };
        let reading = self.launched.get_mut(pane).filter(|r| r.id == id)?;
        Some(&mut reading.held)
    }

    /// The transcript on screen for this pane, which is a launched conversation while one is open
    /// and the pane's own otherwise. Every surface that scrolls, pages or folds goes through here
    /// so that a key pressed at what is drawn moves what is drawn.
    fn showing_mut(&mut self, pane: &str) -> Option<&mut Transcript> {
        match self.launched.get_mut(pane) {
            Some(reading) => Some(&mut reading.held),
            None => self.panes.get_mut(pane),
        }
    }

    /// The line half-typed at the pane's own keyboard, and the key measured to empty it there.
    /// **Both or neither**: the words are worth taking only where they can also be cleared, or
    /// the reply joins onto them all over again.
    pub fn desk_line(&self, pane: &str) -> Option<(&str, Option<&str>)> {
        let held = self.panes.get(pane)?;
        Some((held.desk.as_deref()?, held.clear.as_deref()))
    }

    /// Puts the view on the turn a search hit names, and says what that took.
    ///
    /// **The coordinate is a turn id and not a row**, because a hit the node found may be in a
    /// page this client has never held — and then the walk to it is the same `convo.load` the
    /// reader makes by scrolling, which is what keeps what is held contiguous.
    pub fn aim(&mut self, pane: &str, turn: &str) -> Aimed {
        let Some(held) = self.showing_mut(pane) else {
            return Aimed::Gone;
        };
        held.aiming = Some(turn.to_string());
        if held.at(turn).is_some() {
            held.laid = None;
            return Aimed::Shown;
        }
        match held.more.then(|| held.cursor.clone()).flatten() {
            Some(before) => Aimed::Paging(before),
            None => Aimed::Gone,
        }
    }

    /// The next page to ask for while an aim is still standing, or `None` when there is nothing
    /// left to walk — the turn landed, or the transcript ran out before it did.
    pub fn walking(&self, pane: &str) -> Option<String> {
        // The pane's own transcript, because `convo.load` is the only verb this walk has and a
        // launched conversation is paged by another one. Nothing aims past what a launched
        // conversation holds: the search inside one is over the turns that are here.
        let held = self.panes.get(pane)?;
        let want = held.aiming.as_deref()?;
        if held.at(want).is_some() {
            return None;
        }
        held.more.then(|| held.cursor.clone()).flatten()
    }

    /// The same search over the turns this client holds, for a node that never promised the verb.
    /// **Newest first**, the order the node answers in and the direction paging goes.
    pub fn search_held(&self, pane: &str, query: &str) -> Vec<ConvoMatch> {
        let held = match self.launched.get(pane) {
            Some(reading) => &reading.held,
            None => match self.panes.get(pane) {
                Some(held) => held,
                None => return Vec::new(),
            },
        };
        let needle = query.to_lowercase();
        let visible: Vec<&Turn> = held.turns.iter().filter(|turn| turn.visible()).collect();
        let last = visible.len().saturating_sub(1);
        visible
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(at, turn)| {
                let text = turn.text();
                let hits = text.to_lowercase().matches(&needle).count() as u32;
                (hits > 0).then(|| ConvoMatch {
                    turn: turn.id.clone(),
                    role: turn.role.clone(),
                    at: turn.at.clone(),
                    from_end: (last - at) as u32,
                    hits,
                    text: line_of(&text, &needle),
                })
            })
            .collect()
    }

    /// What the copy key takes off the transcript on screen, which is a launched conversation's
    /// while one is open.
    pub fn code(&self, pane: &str) -> Option<String> {
        let held = match self.launched.get(pane) {
            Some(reading) => &reading.held,
            None => self.panes.get(pane)?,
        };
        held.code().map(str::to_string)
    }

    /// Every conversation this pane's transcript says its agent launched, oldest first.
    pub fn launches(&self, pane: &str) -> Vec<Launch> {
        let Some(held) = self.panes.get(pane) else {
            return Vec::new();
        };
        held.launches()
    }

    /// The handle of the launched conversation being read, if one is.
    pub fn reading(&self, pane: &str) -> Option<&str> {
        self.launched.get(pane).map(|r| r.id.as_str())
    }

    /// Opens one, empty: the page is asked for separately and the view swaps at once so the
    /// keystroke is answered by the screen rather than by the round trip.
    pub fn open_launched(&mut self, pane: &str, launch: &Launch) {
        self.launched.insert(
            pane.to_string(),
            Reading {
                id: launch.id.clone(),
                head: launch.head.clone(),
                held: Transcript::default(),
            },
        );
    }

    /// Back to the pane's own conversation, and whether there was one to come back from.
    pub fn leave_launched(&mut self, pane: &str) -> bool {
        self.launched.remove(pane).is_some()
    }

    pub fn pending(&self, pane: &str) -> Option<&Pending> {
        self.panes.get(pane)?.pending.as_ref()
    }

    /// The key to answer with, when the operator pressed one the prompt offered. **Never
    /// synthesises an Enter** — the node decides whether a submit key follows, per harness (#43).
    pub fn answer(&self, pane: &str, key: char) -> Option<String> {
        self.pending(pane)?
            .options
            .iter()
            .find(|option| option.key.chars().eq(std::iter::once(key)))
            .map(|option| option.key.clone())
    }

    /// **`true` is consumed, `false` is not ours.** The two are not the same thing: an arrow key
    /// this surface took and the app read as unhandled would fall through to the pane and land in
    /// the agent's PTY. Nothing here produces an app-level action — the transcript is its own
    /// scrolling surface, and the pane's ring is a different one that must not move with it.
    pub fn key(&mut self, pane: &str, key: KeyEvent) -> bool {
        let Some(held) = self.showing_mut(pane) else {
            return false;
        };
        let page = held.height().clamp(1, 20);
        match key.code {
            // The only thing on this surface that opens, and the transcript takes the key only
            // where there is a summary to move — a pane that was never compacted goes on handing
            // its arrows to the agent's own prompt.
            KeyCode::Right => return held.unfold(true),
            KeyCode::Left => return held.unfold(false),
            KeyCode::Up => held.scroll = held.scroll.saturating_add(1),
            KeyCode::Down => held.scroll = held.scroll.saturating_sub(1),
            KeyCode::PageDown => held.scroll = held.scroll.saturating_sub(page),
            KeyCode::Home => held.scroll = usize::MAX,
            KeyCode::End => held.scroll = 0,
            // At the top of what is held, `convo.load` takes over: the key is handed back rather
            // than swallowed so the app asks the node for the page before it.
            KeyCode::PageUp if held.scroll >= held.height().saturating_sub(1) => return false,
            KeyCode::PageUp => held.scroll = held.scroll.saturating_add(page),
            _ => return false,
        }
        true
    }

    /// The wheel over this pane's transcript. Same window `up`/`down` move, and the same reason it
    /// is separate from the pane's ring: two scrolling surfaces that must not move together.
    pub fn wheel(&mut self, pane: &str, up: bool, by: usize) -> bool {
        let Some(held) = self.showing_mut(pane) else {
            return false;
        };
        held.scroll = match up {
            true => held.scroll.saturating_add(by),
            false => held.scroll.saturating_sub(by),
        };
        true
    }

    /// Whether a transcript for this pane has produced anything to show. A prompt counts: a
    /// blocked agent whose first page has not landed is exactly the pane this client exists to
    /// answer, and the strip is the only surface that answers it.
    pub fn has(&self, pane: &str) -> bool {
        self.panes
            .get(pane)
            .is_some_and(|held| held.showing() || held.pending.is_some())
    }

    /// The `before` cursor for a `convo.load`. **Absent is not `more: false`.** A page with no
    /// cursor cannot be paged from however loudly `more` says there is history behind it; a page
    /// with a cursor and `more: false` has reached the start of the transcript.
    pub fn load_more(&mut self, pane: &str) -> Option<More> {
        match self.launched.get(pane) {
            Some(reading) => {
                let before = reading.held.more.then(|| reading.held.cursor.clone()).flatten()?;
                Some(More::Launched {
                    id: reading.id.clone(),
                    before,
                })
            }
            None => {
                let held = self.panes.get(pane)?;
                let before = held.more.then(|| held.cursor.clone()).flatten()?;
                Some(More::Pane(before))
            }
        }
    }

    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        pane: &str,
        theme: &Theme,
        images: &mut Images,
        role: Role,
    ) -> Marks {
        if area.width == 0 || area.height == 0 {
            return Marks::default();
        }
        // **Created on demand.** A pane whose harness this node has an adapter for may be opened
        // on its conversation before anything has been written — the gap between a session
        // starting and its first prompt — and an empty transcript that says so is the answer,
        // not a fall-through to the grid.
        let held = self.panes.entry(pane.to_string()).or_default();
        // **The strips stay the pane's** while a launched conversation is being read. The
        // question is the pane's to answer and the reply box types into the pane, so a subagent's
        // transcript borrows the screen and nothing else.
        let mut strip = held
            .pending
            .as_ref()
            .map(|p| prompt(p, theme, area.width))
            .unwrap_or_default();
        if let Some(line) = held.desk.as_deref() {
            strip.lines.insert(0, waiting(line, theme, area.width));
            strip.chip_row += 1;
        }
        let rows = (strip.lines.len() as u16).min(area.height);
        let mut body = Rect {
            height: area.height - rows,
            ..area
        };
        if let Some(head) = self.launched.get(pane).map(|r| r.head.clone())
            && body.height > 0
        {
            buf.set_line(body.x, body.y, &reading_head(&head, theme), body.width);
            body = Rect {
                y: body.y + 1,
                height: body.height - 1,
                ..body
            };
        }
        let Some(held) = self.showing_mut(pane) else {
            return Marks::default();
        };
        held.lay(body.width, theme, role.writes());
        let mut marks = held.paint(buf, body, pane, images, role);
        let chips = strip.chips;
        for (row, line) in strip.lines.into_iter().enumerate() {
            let y = area.y + body.height + row as u16;
            fill(buf, area.x, y, area.width, theme.blocked_bg);
            buf.set_line(area.x, y, &line, area.width);
        }
        // The chips are on one known line of the strip, which is only on screen if there was room.
        if rows > strip.chip_row {
            let y = area.y + body.height + strip.chip_row;
            marks.options = chips
                .into_iter()
                .map(|(x, width)| Rect {
                    x: area.x + x,
                    y,
                    width,
                    height: 1,
                })
                .collect();
        }
        marks
    }
}

/// The one row that says whose conversation is on screen, pinned above the transcript rather than
/// scrolling with it: a reader who has paged away from the top must still be able to tell that
/// what they are reading is not the pane's own.
fn reading_head(head: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  ◂ {head}"),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  esc to come back".to_string(), Style::default().fg(theme.mute)),
    ])
}

fn decode(turns: &[Value]) -> Vec<Turn> {
    turns
        .iter()
        .filter_map(|turn| serde_json::from_value(turn.clone()).ok())
        .collect()
}

fn fill(buf: &mut Buffer, x: u16, y: u16, width: u16, bg: Color) {
    for column in x..x.saturating_add(width) {
        if let Some(cell) = buf.cell_mut((column, y)) {
            cell.set_bg(bg);
        }
    }
}

impl Transcript {
    fn lay(&mut self, width: u16, theme: &Theme, writes: bool) {
        if self
            .laid
            .as_ref()
            .is_some_and(|l| l.width == width && l.revision == self.revision && l.writes == writes)
        {
            return;
        }
        let mut pieces = Vec::new();
        match self.showing() {
            true => pieces.push(Piece::Line(edge(self.cursor.as_deref(), self.more, theme))),
            false => pieces.push(Piece::Line(Line::styled(
                "  nothing in this transcript yet".to_string(),
                Style::default().fg(theme.mute),
            ))),
        }
        let newest = self.launches().pop().map(|launch| launch.id);
        let at = Laying {
            width,
            theme,
            inline: &self.inline,
            notes: &self.notes,
            writes,
            newest_launch: newest.as_deref(),
        };
        // The queue stands at the foot, after everything recorded: it is what has not happened
        // yet, and the transcript is pinned to its own end.
        let mut heads = Vec::new();
        for turn in self
            .turns
            .iter()
            .chain(self.queued.iter())
            .filter(|t| t.visible())
        {
            heads.push((turn.id.clone(), pieces.len()));
            lay_turn(turn, &at, &self.open, &mut pieces);
        }
        self.laid = Some(Laid {
            width,
            revision: self.revision,
            heads,
            writes,
            pieces,
        });
    }

    fn paint(&mut self, buf: &mut Buffer, area: Rect, pane: &str, images: &mut Images, role: Role) -> Marks {
        let Some(laid) = self.laid.as_ref() else {
            return Marks::default();
        };
        let rows = area.height as usize;
        let ceiling = laid.pieces.len().saturating_sub(rows);
        // The aim is spent here rather than where it was taken, because this is the first place
        // that knows both where the turn was drawn and how much of it a screen this tall holds.
        if let Some(at) = self
            .aiming
            .as_deref()
            .and_then(|want| laid.heads.iter().find(|(id, _)| id == want))
            .map(|(_, at)| *at)
        {
            self.scroll = ceiling.saturating_sub(at);
            self.aiming = None;
        }
        self.scroll = self.scroll.min(ceiling);
        let top = ceiling - self.scroll;
        let mut marks = Marks::default();
        let mut seen: Vec<(Attachment, Option<String>)> = Vec::new();
        for (row, piece) in laid.pieces.iter().skip(top).take(rows).enumerate() {
            let y = area.y + row as u16;
            match piece {
                Piece::Line(line) | Piece::Code { head: line, .. } => {
                    buf.set_line(area.x, y, line, area.width);
                }
                Piece::Image { att, marker, file } => {
                    let drawn = self.inline.contains(&att.id)
                        && images.draw(
                            buf,
                            Rect {
                                x: area.x,
                                y,
                                width: area.width,
                                height: IMAGE_ROWS.min(area.height - row as u16),
                            },
                            pane,
                            &att.id,
                        );
                    if !drawn {
                        buf.set_line(area.x, y, marker, area.width);
                        marks.attachments.push((
                            att.id.clone(),
                            Rect {
                                x: area.x,
                                y,
                                width: area.width,
                                height: 1,
                            },
                        ));
                    }
                    seen.push((att.clone(), file.clone()));
                }
            }
        }
        for (att, file) in &seen {
            // The turn is on screen, so the bytes are worth the round trip they cost — and only
            // now, because a header is cheap and an 8 MiB body is not.
            if !self.requested.insert(att.id.clone()) {
                continue;
            }
            match file {
                // A path form is refused for a read-only device by the node, so it is refused
                // here first rather than spending the round trip to be told.
                Some(path) => {
                    images.request_file(pane, path, role);
                }
                None => images.request(pane, att),
            }
        }
        for (att, _) in &seen {
            let Some(offer) = images.offer(pane, &att.id) else {
                continue;
            };
            let ready = offer.ready && offer.inline;
            let note = aside(&offer);
            let changed = match ready {
                true => self.inline.insert(att.id.clone()),
                false => self.inline.remove(&att.id),
            };
            let renoted = match note {
                Some(note) => self.notes.insert(att.id.clone(), note.clone()) != Some(note),
                None => self.notes.remove(&att.id).is_some(),
            };
            if changed || renoted {
                self.touch();
            }
        }
        marks
    }
}

/// What sits above the oldest turn held. The three states are distinct on the wire, and a client
/// that folds them together offers a `convo.load` it can never send.
fn edge(cursor: Option<&str>, more: bool, theme: &Theme) -> Line<'static> {
    let dim = Style::default().fg(theme.mute);
    match (cursor, more) {
        (Some(_), true) => Line::styled("  ⟨ pgup for earlier turns ⟩".to_string(), dim),
        (None, true) => Line::styled("  ⟨ earlier turns are not pageable ⟩".to_string(), dim),
        _ => Line::styled("  ⟨ the start of this transcript ⟩".to_string(), dim),
    }
}

/// Everything a block needs to be laid out that is not the block. It travels as one value
/// because every arm of [`lay_block`] wants a different half of it.
pub(super) struct Laying<'a> {
    pub width: u16,
    pub theme: &'a Theme,
    pub inline: &'a HashSet<String>,
    pub notes: &'a HashMap<String, String>,
    pub writes: bool,
    /// The newest launched conversation this transcript holds, which is the one the key opens and
    /// so the only card that may say so.
    pub newest_launch: Option<&'a str>,
}

impl Laying<'_> {
    pub fn bare(width: u16, theme: &Theme) -> Laying<'_> {
        static NONE: OnceLock<(HashSet<String>, HashMap<String, String>)> = OnceLock::new();
        let (inline, notes) = NONE.get_or_init(Default::default);
        Laying {
            width,
            theme,
            inline,
            notes,
            writes: false,
            newest_launch: None,
        }
    }
}

fn lay_turn(turn: &Turn, at: &Laying<'_>, open: &HashSet<String>, out: &mut Vec<Piece>) {
    let theme = at.theme;
    let summary = turn.summary();
    // **Not "you".** The queue belongs to the pane, so a prompt standing in it may have been
    // typed at the desk by somebody else entirely.
    let (who, colour) = match (summary, turn.queued(), turn.person()) {
        (true, _, _) => ("compacted", theme.dim),
        (_, true, _) => ("queued", theme.working),
        (_, _, true) => ("you", theme.accent_hi),
        _ => ("agent", theme.done),
    };
    out.push(Piece::Line(Line::default()));
    let mut head = vec![Span::styled(
        format!("  {who}"),
        Style::default().fg(colour).add_modifier(Modifier::BOLD),
    )];
    if let Some(stamp) = turn.at.as_deref().and_then(when) {
        head.push(Span::styled(
            format!("  {stamp}"),
            Style::default().fg(theme.mute),
        ));
    }
    out.push(Piece::Line(Line::from(head)));
    if summary && !open.contains(&turn.id) {
        out.push(Piece::Line(Line::styled(
            format!("  ⟨ {} lines · → to open ⟩", turn.lines()),
            Style::default().fg(theme.mute),
        )));
        return;
    }
    // What the card above a result claims the whole of it came to, which is the only way a block
    // that was clipped on the node can say so.
    let mut produced = None;
    for block in &turn.blocks {
        if let Block::Tool { lines, .. } = block {
            produced = *lines;
        }
        lay_block(block, at, produced, out);
    }
    if summary {
        out.push(Piece::Line(Line::styled(
            "  ⟨ ← to put it away ⟩".to_string(),
            Style::default().fg(theme.mute),
        )));
    }
    if turn.id == LIVE {
        // The wording may still change under the reader, so it is marked rather than drawn as a
        // recorded turn. Kampr's own conversation view puts a caret and *still writing*.
        out.push(Piece::Line(Line::from(vec![
            Span::styled(
                "  ▏".to_string(),
                Style::default().fg(theme.working).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " still writing".to_string(),
                Style::default().fg(theme.working).add_modifier(Modifier::ITALIC),
            ),
        ])));
    }
}

pub(super) fn lay_block(block: &Block, at: &Laying<'_>, produced: Option<u32>, out: &mut Vec<Piece>) {
    let (width, theme) = (at.width, at.theme);
    match block {
        Block::Md { text, att: None } => markdown(text, width, theme, out),
        Block::Md { text, att: Some(att) } => {
            let header = att.header();
            let marker = Line::from(vec![
                Span::styled(
                    format!("  {}", marker(text, &header)),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    at.notes.get(&header.id).cloned().unwrap_or_default(),
                    Style::default().fg(theme.mute),
                ),
            ]);
            // `kind` is an open string and a client that does not recognise one treats it as a
            // file rather than dropping the block, so a later `video` needs no client release.
            out.push(Piece::Image {
                att: header,
                marker,
                file: None,
            });
            if at.inline.contains(&att.id) {
                for _ in 1..IMAGE_ROWS {
                    out.push(Piece::Line(Line::default()));
                }
            }
        }
        Block::Code { lang, text, role } => {
            let result = role.as_deref() == Some(OUTPUT);
            let head = match (result, lang) {
                (true, _) => OUTPUT.to_string(),
                (false, Some(lang)) => lang.clone(),
                (false, None) => "code".to_string(),
            };
            let rule = (width as usize).saturating_sub(head.chars().count() + 4);
            out.push(Piece::Code {
                head: Line::styled(
                    format!("  {head} {}", "─".repeat(rule)),
                    Style::default().fg(theme.mute),
                ),
                text: text.clone(),
            });
            let mut shown = 0;
            for line in text.lines() {
                shown += 1;
                out.push(Piece::Line(Line::styled(
                    format!("  {}", clip(line, width.saturating_sub(2) as usize)),
                    Style::default().fg(theme.text).bg(theme.surface),
                )));
            }
            // The node clips a result to the head of it and leaves the card counting all of it, so
            // a total above what arrived is the rest still sitting on the host.
            if let Some(total) = produced.filter(|total| result && *total as usize > shown) {
                out.push(Piece::Line(Line::styled(
                    format!("  ⟨ showing the first {shown} of {total} lines ⟩"),
                    Style::default().fg(theme.mute),
                )));
            }
        }
        Block::Tool {
            name,
            summary,
            lines,
            state,
        } => {
            let (mark, colour) = match state.as_str() {
                "running" => ("◐", theme.working),
                "error" => ("✗", theme.blocked),
                _ => ("✓", theme.done),
            };
            let mut spans = vec![
                Span::styled(format!("  {mark} "), Style::default().fg(colour)),
                Span::styled(
                    name.clone(),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(summary) = summary {
                spans.push(Span::styled(
                    format!(" · {summary}"),
                    Style::default().fg(theme.dim),
                ));
            }
            if let Some(lines) = lines {
                spans.push(Span::styled(
                    format!(" · {lines} lines"),
                    Style::default().fg(theme.mute),
                ));
            }
            spans.push(Span::styled(format!(" · {state}"), Style::default().fg(colour)));
            out.push(Piece::Line(Line::from(spans)));
            // A picture a tool call named by path, for one whose record has been rewritten out
            // from under its id. **Only a device that may send input may ask for one**, so the
            // affordance is gated on the live role rather than on the greeting's.
            if let Some(path) = at.writes.then(|| summary.as_deref().and_then(picture)).flatten() {
                let id = crate::image::file_id(path);
                let marker = Line::from(vec![
                    Span::styled(format!("  [{path}]"), Style::default().fg(theme.accent)),
                    Span::styled(
                        at.notes.get(&id).cloned().unwrap_or_default(),
                        Style::default().fg(theme.mute),
                    ),
                ]);
                let inline = at.inline.contains(&id);
                out.push(Piece::Image {
                    att: Attachment {
                        id,
                        kind: "image".into(),
                        mime: None,
                        bytes: None,
                        name: std::path::Path::new(path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(str::to_string),
                    },
                    marker,
                    file: Some(path.to_string()),
                });
                if inline {
                    for _ in 1..IMAGE_ROWS {
                        out.push(Piece::Line(Line::default()));
                    }
                }
            }
        }
        Block::Diff { path, text } => diff(path.as_deref(), text, width, theme, out),
        // Offered, not inlined. The hint rides on the newest, which is the one the key opens.
        Block::Sub { id, kind, title } => {
            let mut spans = vec![
                Span::styled("  ⇲ ".to_string(), Style::default().fg(theme.accent)),
                Span::styled(
                    headline(kind.as_deref(), title.as_deref()),
                    Style::default().fg(theme.text),
                ),
            ];
            if at.newest_launch == Some(id.as_str()) {
                spans.push(Span::styled(
                    "  ⟨ ^b O to open ⟩".to_string(),
                    Style::default().fg(theme.mute),
                ));
            }
            out.push(Piece::Line(Line::from(spans)));
        }
        Block::Unknown => {}
    }
}

/// What the offer says beside the marker. The `413` is worth saying out loud because the picture
/// is there and is simply bigger than the ceiling; the single `404` is not, because an id names a
/// record in a transcript and stops resolving when the transcript is rewritten.
fn aside(offer: &crate::image::Offer<'_>) -> Option<String> {
    if offer.too_large {
        return Some(" · too large to fetch".to_string());
    }
    if offer.inline {
        return None;
    }
    match (offer.ready, offer.bytes) {
        (true, Some(bytes)) => Some(format!(" · {} · click to save", size(bytes))),
        (true, None) => Some(" · click to save".to_string()),
        (false, _) => None,
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.0} kB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
    }
}

/// An absolute path with an image extension, out of a tool call's own words. Anything looser
/// would ask the node for whatever a summary happened to contain.
fn picture(summary: &str) -> Option<&str> {
    summary
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | ','))
        .find(|word| word.starts_with('/') && crate::image::image_mime(word).is_some())
}

/// The `text` beside an `att` is the marker an installed client already renders — `[image · png]`
/// — so it is what goes on the screen when the bytes are not here.
fn marker(text: &str, att: &Attachment) -> String {
    match text.trim().is_empty() {
        false => text.trim().to_string(),
        true => {
            let name = att.name.clone().unwrap_or_else(|| att.kind.clone()).to_string();
            match att.mime.as_deref() {
                Some(mime) => format!("[{name} · {mime}]"),
                None => format!("[{name}]"),
            }
        }
    }
}

/// Claude rebuilds a unified diff from `structuredPatch`; Codex sends its `*** Begin Patch`
/// envelope verbatim; `agy` sends hunk headers and no `---`/`+++` at all. All three share the
/// `+`/`-` line prefixes, so the prefixes are the classifier and a header is never assumed.
fn diff(path: Option<&str>, text: &str, width: u16, theme: &Theme, out: &mut Vec<Piece>) {
    let lines: Vec<&str> = text.lines().collect();
    let added = lines
        .iter()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .count();
    let removed = lines
        .iter()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .count();
    let mut head = vec![Span::styled(
        "  diff".to_string(),
        Style::default().fg(theme.mute).add_modifier(Modifier::BOLD),
    )];
    if let Some(path) = path {
        head.push(Span::styled(format!(" · {path}"), Style::default().fg(theme.dim)));
    }
    head.push(Span::styled(
        format!("  +{added} -{removed}"),
        Style::default().fg(theme.done),
    ));
    out.push(Piece::Line(Line::from(head)));
    for line in lines {
        let ink = match () {
            _ if line.starts_with("***") || line.starts_with("+++") || line.starts_with("---") => theme.mute,
            _ if line.starts_with("@@") => theme.accent,
            _ if line.starts_with('+') => theme.done,
            _ if line.starts_with('-') => theme.blocked,
            _ => theme.dim,
        };
        out.push(Piece::Line(Line::styled(
            format!("  {}", clip(line, width.saturating_sub(2) as usize)),
            Style::default().fg(ink).bg(theme.surface),
        )));
    }
}

/// The strip, and where each option's key chip landed so a click can answer it.
#[derive(Default)]
struct Strip {
    lines: Vec<Line<'static>>,
    chips: Vec<(u16, u16)>,
    /// Which of [`Self::lines`] the chips were laid on. A row rather than a constant because the
    /// desk line stands above the prompt and moves it.
    chip_row: u16,
}

/// A line the operator left at the pane's own keyboard. Dim rather than alarming: it is context for
/// the reply about to be appended to it, not a fault.
fn waiting(line: &str, theme: &Theme, width: u16) -> Line<'static> {
    // At least one character of the line survives however narrow the pane is: a bare ellipsis
    // says a line is waiting and refuses to say anything about it.
    let room = (width.saturating_sub(16) as usize).max(2);
    let shown: String = match line.chars().count() > room {
        true => line.chars().take(room - 1).chain(['…']).collect(),
        false => line.to_string(),
    };
    Line::from(vec![
        Span::styled(
            " at the desk ".to_string(),
            Style::default().fg(theme.on_accent).bg(theme.working),
        ),
        Span::styled(format!(" {shown}"), Style::default().fg(theme.dim)),
    ])
}

/// The chips land on the second line, which is what [`Strip::chip_row`] carries out to the caller.
fn prompt(pending: &Pending, theme: &Theme, width: u16) -> Strip {
    let Some(question) = pending.question.as_deref() else {
        return Strip::default();
    };
    // The dialog's own title in front of the question, where it draws one. Two lines is the whole
    // budget here — a row this strip takes is a row the pane never gets (#373, #374) — so the
    // header rides on the question's line rather than earning one, and the per-option descriptions
    // the Compose client draws have nowhere to go at all.
    let asked = match pending.header.as_deref() {
        Some(header) => format!("{header} · {question}"),
        None => question.to_string(),
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            " ⚑ ".to_string(),
            Style::default()
                .fg(theme.blocked)
                .bg(theme.blocked_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            clip(&asked, width.saturating_sub(4) as usize),
            Style::default()
                .fg(theme.text)
                .bg(theme.blocked_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    // Only the keys the node offered. It decides whether a submit key follows, per harness —
    // claude selects on the bare digit, codex needs an Enter (#43) — so nothing is synthesised.
    let mut spans = vec![Span::styled(
        " ".to_string(),
        Style::default().bg(theme.blocked_bg),
    )];
    let mut chips = Vec::new();
    let mut x = 1u16;
    for option in &pending.options {
        let key = format!(" {} ", option.key);
        let chip = key.chars().count() as u16;
        chips.push((x, chip));
        x += chip;
        spans.push(Span::styled(
            key,
            Style::default()
                .fg(theme.on_accent)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        // A tick is drawn rather than left implicit: on a question that takes several answers the
        // node strips the checkbox out of the label to publish `chosen`, and a reader who cannot
        // see which are ticked cannot tell what pressing again would do.
        let label = match (pending.multi, option.chosen) {
            (true, true) => format!(" ☑ {}  ", option.label),
            (true, false) => format!(" ☐ {}  ", option.label),
            _ => format!(" {}  ", option.label),
        };
        x += label.chars().count() as u16;
        spans.push(Span::styled(
            label,
            Style::default().fg(theme.text).bg(theme.blocked_bg),
        ));
    }
    if pending.options.is_empty() {
        spans.push(Span::styled(
            "no keys were offered".to_string(),
            Style::default().fg(theme.dim).bg(theme.blocked_bg),
        ));
    }
    // **The one thing this client must not let a chip imply.** A press here ticks a box and does
    // not answer, and there is no commit affordance on this surface — the sequence that commits is
    // right-arrow then Enter (#421), which the operator sends by opening the pane.
    if pending.multi {
        spans.push(Span::styled(
            "· ticks only, submit in the pane".to_string(),
            Style::default().fg(theme.mute).bg(theme.blocked_bg),
        ));
    }
    lines.push(Line::from(spans));
    Strip {
        lines,
        chips,
        chip_row: 1,
    }
}

pub(super) fn clip(text: &str, width: usize) -> String {
    if Span::raw(text).width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if Span::raw(out.as_str()).width() + 1 >= width {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

pub(super) fn pad(text: &str, width: usize, align: Alignment) -> String {
    let text = clip(text, width);
    let slack = width.saturating_sub(Span::raw(text.as_str()).width());
    match align {
        Alignment::Right => format!("{}{text}", " ".repeat(slack)),
        Alignment::Center => {
            let left = slack / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(slack - left))
        }
        _ => format!("{text}{}", " ".repeat(slack)),
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    fn option(key: &str, label: &str, chosen: bool) -> kampr_client::PendingOption {
        kampr_client::PendingOption {
            key: key.into(),
            label: label.into(),
            chosen,
            ..kampr_client::PendingOption::default()
        }
    }

    fn said(pending: &Pending) -> Vec<String> {
        prompt(pending, &crate::theme::PHOSPHOR, 120)
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    /// Two lines is the whole budget, so the dialog's own title rides on the question's line
    /// rather than earning one of its own.
    #[test]
    fn a_dialogs_title_reaches_a_terminal_without_costing_the_pane_a_row() {
        let strip = prompt(
            &Pending {
                pane: "p".into(),
                question: Some("Which indentation do you prefer?".into()),
                header: Some("Indentation".into()),
                options: vec![option("1", "Tabs", false)],
                ..Pending::default()
            },
            &crate::theme::PHOSPHOR,
            120,
        );
        assert_eq!(strip.lines.len(), 2, "the strip grew a row");
        assert!(
            strip.lines[0]
                .spans
                .iter()
                .any(|s| s.content.contains("Indentation · Which indentation")),
            "{:?}",
            said(&Pending::default()),
        );
    }

    /// **The chip must not read as an answer when it is a tick.** This surface has no commit
    /// affordance — the sequence is right-arrow then Enter (#421) — so it says where to send.
    #[test]
    fn a_question_that_takes_several_answers_says_a_press_is_only_a_tick() {
        let pending = Pending {
            pane: "p".into(),
            question: Some("Which test suites should I run?".into()),
            multi: true,
            options: vec![option("1", "unit", true), option("2", "integration", false)],
            ..Pending::default()
        };
        let lines = said(&pending);
        assert!(lines[1].contains("☑ unit"), "{lines:?}");
        assert!(lines[1].contains("☐ integration"), "{lines:?}");
        assert!(lines[1].contains("ticks only, submit in the pane"), "{lines:?}");
    }

    /// And a single-answer question is untouched: no boxes, no warning, because a press there
    /// really is the answer.
    #[test]
    fn a_single_answer_question_is_drawn_exactly_as_it_was() {
        let lines = said(&Pending {
            pane: "p".into(),
            question: Some("Do you want to make this edit?".into()),
            options: vec![option("1", "Yes", false), option("2", "No", false)],
            ..Pending::default()
        });
        assert!(
            lines[1].contains(" Yes  ") && lines[1].contains(" No  "),
            "{lines:?}"
        );
        assert!(
            !lines[1].contains('☐') && !lines[1].contains("ticks only"),
            "{lines:?}"
        );
    }
}
