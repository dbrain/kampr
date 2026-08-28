use crate::provider::{AgentStatus, PaneInfo};
use crate::registry::PaneUpdate;
use crate::scrollback::ScrollbackDoc;
use kampr_journal::{Page, Turn};
use kampr_term::{Cell, CellAttrs, Color, RowDiff};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PROTOCOL: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    #[serde(flatten)]
    pub attrs: CellAttrs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Styles {
    pub from: u32,
    pub styles: Vec<Style>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub s: u32,
    pub x: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub l: Option<u32>,
    /// Columns per character in `x`, 1 or 2. A client has no Unicode width table and must not
    /// need one: this is how it learns that the column after a double-width glyph belongs to that
    /// glyph and not to the next one (probe #210). Omitted when 1, so `x` stays exactly one code
    /// point per cell and every path that reads it — copy, find, link detection — reads it
    /// unchanged rather than stripping a sentinel out of it.
    #[serde(default = "narrow", skip_serializing_if = "is_narrow")]
    pub w: u8,
    /// The zero-width code points each cell of this run is wearing — combining marks, ZWJ,
    /// variation selectors — by position, empty where a cell wears none and truncated after the
    /// last one (probe #223). It rides beside `x` rather than in it so that `x` stays exactly one
    /// code point per cell: a row is still `sum(codepoints(x) * w)` columns wide, and a client
    /// that has never heard of this field draws the bases it already drew instead of a row shifted
    /// one column per accent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub m: Vec<String>,
}

fn coarse(style: Style) -> Style {
    Style {
        fg: quantise(style.fg),
        bg: quantise(style.bg),
        attrs: style.attrs,
    }
}

fn quantise(colour: Color) -> Color {
    match colour {
        Color::Rgb(r, g, b) => {
            let mask = !0u8 << (8 - COARSE_BITS);
            Color::Rgb(r & mask, g & mask, b & mask)
        }
        other => other,
    }
}

fn narrow() -> u8 {
    1
}

fn is_narrow(w: &u8) -> bool {
    *w == 1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowRuns {
    pub row: u32,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Caps {
    pub push: bool,
    pub scrollback: bool,
    pub conversation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Full,
    Readonly,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hello {
    pub protocol: u32,
    pub node_id: String,
    pub node_name: String,
    pub build: String,
    pub role: Role,
    pub caps: Caps,
}

/// Deserialisable because a hub reads one off a peer's own `herd` message and re-publishes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeEntry {
    pub id: String,
    pub name: String,
    /// `local` for a herdr session this process serves, `peer` for one reached over a mesh link.
    pub kind: String,
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_version: Option<String>,
    /// The kampr build behind this node. Additive, and the whole of the version-skew story: two
    /// nodes in one herd may be running different releases, and a client can only say so if each
    /// node names its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    /// The release that supersedes [`Self::build`], named — absent when this node is current,
    /// when it has not managed to ask, and when its operator has turned the check off. All three
    /// are "nothing to say", and a client that renders the field renders nothing for all three.
    ///
    /// **A version, not a flag.** The mesh question is which machines are stale, and a boolean
    /// answers it without saying what they are stale against.
    ///
    /// Filled in by the node it describes, never judged by a hub: only that node knows what it is
    /// actually running, and only that node's own config can say whether it may ask GitHub at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    /// Why a node is offline, in the words the operator would see in the log. Additive: a v1
    /// client that does not know the field ignores it, and one that does can say *why* the herd
    /// is empty instead of showing an empty herd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneEntry {
    pub id: String,
    pub node_id: String,
    /// Node-qualified, exactly like `id`, so either can be used as a `manage` op's `at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub tab: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
    /// Absent until measured: the layout rect is not the PTY width in a headless session, and a
    /// client shows the operator this number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    pub rows: u16,
    #[serde(default)]
    pub scrollback_rows: u32,
    #[serde(default)]
    pub has_conversation: bool,
    /// How many viewers this node is streaming the pane to, when it is more than one — so a client
    /// can say the pane is **open** somewhere else. It says nothing about typing: a viewer is
    /// somebody who has the pane on screen, not somebody at the keys. Absent for nought and for
    /// one, which is the overwhelmingly common case and reads as "just me".
    ///
    /// **A floor, never a headcount.** It counts *this node's* watchers, so a pane relayed through
    /// a hub carries the peer's own number and the whole hub is one viewer in it however many
    /// clients sit behind it. Under-counting is the only direction this may fail in: a phone that
    /// claims company when there is none has told a lie about a terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watchers: Option<u32>,
    /// Node-stamped; herdr's snapshot carries no timestamp, so whoever assembles the herd message
    /// owns this clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Why this pane has no picture, in the operator's words — the pane-level twin of
    /// [`NodeEntry::detail`], and absent for exactly the same reason: nothing is wrong.
    ///
    /// A node reaches herdr two ways, over a socket for the model and over a spawned binary for
    /// the screens, and it can have exactly one of them working. A node in that state serves a
    /// correct herd and streams nothing, which is a blank grid and a flashing cursor for ever —
    /// so the half that is broken has to be *said*, on the pane the operator is looking at.
    ///
    /// Additive: a client that has never heard of the field is a client that behaves as it does
    /// today. It clears itself, because the supervisor behind it retries for ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The foreground job in this pane: `cmd` is the process name, `argv` the whole command line
    /// with its arguments, and a pipeline joins its members with ` | `.
    ///
    /// **Both are absent far more often than a schema reading suggests, and that is not a fault.**
    /// A pane at its prompt is running its shell and has no job to name; and on a machine that
    /// sources ble.sh — every interactive shell on the operator's own (probe #297) — the job runs
    /// inside the shell's own process group, so herdr reports the shell however busy the pane is.
    /// A client renders what it has and drops the section when it has nothing, which is what
    /// [`crate::naming`]'s `[…]` exists for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<String>,
    /// What the harness in this pane calls the session it is having.
    ///
    /// **Generated, and therefore beneath anything the operator set by hand.** `label` is herdr's
    /// and is what somebody typed; this is what the harness derived for itself, and the naming
    /// template takes the first of the two that resolves. A pane with a name of its own is
    /// otherwise described by its working directory, which is the same word for every pane in a
    /// repository.
    ///
    /// Cheap by construction: it is read from the session marker the pane's own harness writes
    /// (#311), which is opened by pid. The richer titles a transcript carries — `ai-title`, and a
    /// `custom-title.json` the operator typed — cost a whole-transcript read and ride the
    /// conversation instead, where that read is already being paid for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl PaneEntry {
    pub fn new(node_id: &str, p: &PaneInfo, has_conversation: bool) -> Self {
        Self {
            id: format!("{node_id}/{}", p.pane_id),
            node_id: node_id.to_string(),
            workspace_id: p.workspace_id.as_ref().map(|w| format!("{node_id}/{w}")),
            tab_id: p.tab_id.as_ref().map(|t| format!("{node_id}/{t}")),
            workspace: p.workspace.clone(),
            tab: p.tab.clone(),
            cwd: p.cwd.clone(),
            label: p.label.clone(),
            agent: p.agent.clone(),
            agent_status: p.agent_status,
            cols: p.cols,
            rows: p.rows,
            scrollback_rows: p.scrollback_rows,
            has_conversation,
            watchers: None,
            updated_at: None,
            detail: p.detail.clone(),
            cmd: p.cmd.clone(),
            argv: p.argv.clone(),
            title: None,
        }
    }

    /// The one place the "omitted below two" rule lives, so the node cannot spell it one way and
    /// the wire document another.
    pub fn with_watchers(mut self, watchers: usize) -> Self {
        self.watchers = (watchers > 1).then_some(watchers as u32);
        self
    }

    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.title = title;
        self
    }
}

/// `herd.patch` carries the same shape as `herd` under `added` and `changed`, so a node going
/// offline and a pane changing travel the same way.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HerdDelta {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneEntry>,
}

impl HerdDelta {
    pub fn panes(panes: Vec<PaneEntry>) -> Self {
        Self {
            nodes: Vec::new(),
            panes,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.panes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingOption {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingSource {
    Transcript,
    Screen,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t")]
pub enum ServerMsg {
    #[serde(rename = "hello")]
    Hello(Hello),
    /// A mid-connection role change. `hello` is defined as the first message on a connection and
    /// stays that way, so a demotion or a promotion travels on its own frame rather than as a
    /// second greeting.
    #[serde(rename = "role")]
    RoleChanged { role: Role },
    #[serde(rename = "herd")]
    Herd {
        nodes: Vec<NodeEntry>,
        panes: Vec<PaneEntry>,
    },
    #[serde(rename = "herd.patch")]
    HerdPatch {
        #[serde(skip_serializing_if = "HerdDelta::is_empty")]
        added: HerdDelta,
        #[serde(skip_serializing_if = "HerdDelta::is_empty")]
        changed: HerdDelta,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        removed_ids: Vec<String>,
    },
    #[serde(rename = "styles")]
    Styles(Styles),
    #[serde(rename = "grid.reset")]
    GridReset {
        pane: String,
        cols: u16,
        rows: u16,
        rows_data: Vec<RowRuns>,
        cursor: Cursor,
        links: Vec<String>,
    },
    #[serde(rename = "grid.patch")]
    GridPatch {
        pane: String,
        rows: Vec<RowRuns>,
        cursor: Cursor,
        /// Absent unless the pane's link table grew; the protocol document only shows `links` on
        /// `grid.reset`, but a hyperlink can first appear inside a patch.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        links: Vec<String>,
    },
    #[serde(rename = "scrollback")]
    Scrollback {
        pane: String,
        from_top: u32,
        rows: Vec<RowRuns>,
        total_rows: u32,
        complete: bool,
        capped: bool,
    },
    #[serde(rename = "convo")]
    Convo {
        pane: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
        more: bool,
        /// **This page is the pane's whole conversation, and anything else held for the pane
        /// belongs to a transcript that has been left.**
        ///
        /// A page merges by id, which is what lets `convo.load` prepend older slices of the same
        /// transcript — and what leaves a stale conversation underneath a new one when the pane
        /// has moved. The node takes the stale turns off by name where it knows them, but a
        /// client that reconnects arrives on a socket carrying no history and the node itself
        /// restarts, so there are cases where nothing on this end can name them. Additive: a
        /// client that has never heard of the field merges, exactly as it does today.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        fresh: bool,
        /// Which launched conversation this page is of, absent for the pane's own. A client that
        /// has never heard of the field only ever receives pages without it, because a page with
        /// one is only ever an answer to a verb such a client cannot send.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sub: Option<String>,
        turns: Vec<Turn>,
    },
    /// Turns added *or revised*. A tool turn is replaced by id when its result lands, so a client
    /// that appends renders every tool twice.
    #[serde(rename = "convo.turn")]
    ConvoTurn {
        pane: String,
        /// Which launched conversation these turns belong to, absent for the pane's own.
        ///
        /// A subagent's transcript grows while it runs, so a reader who opened one wants it to
        /// keep arriving rather than to close and re-open it — but a turn of a *launched*
        /// conversation must never be appended to the pane's own, which would put a subagent's
        /// words in the parent's voice. Absent is what every turn carried before this, so a client
        /// that has never heard of the field only ever receives the turns it already understood.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sub: Option<String>,
        turns: Vec<Turn>,
    },
    /// `question: null` is how a prompt clears, so it is serialised as null rather than omitted.
    #[serde(rename = "pending")]
    Pending {
        pane: String,
        question: Option<String>,
        options: Vec<PendingOption>,
        source: PendingSource,
    },
    /// What the harness wrote down about the *session* rather than about any one turn — its own
    /// title for the conversation, how long each turn took, what is queued behind the one running,
    /// which mode it is in, where it was compacted.
    ///
    /// **Sent once, when a conversation opens.** Collecting it is a whole-transcript read, measured
    /// at 154 ms for 29.4 MB, so it is not something a poll may do — and none of it changes often
    /// enough to be worth re-reading at the follow rate. Every field is optional and a harness with
    /// nothing to say sends an empty object, so a client draws nothing for what it does not get.
    #[serde(rename = "convo.facets")]
    ConvoFacets {
        pane: String,
        facets: kampr_journal::Facets,
    },
    #[serde(rename = "error")]
    Error {
        code: ErrorCode,
        message: String,
        pane: Option<String>,
        /// Which node this is about, when it is about a node rather than a pane.
        ///
        /// **The client is the only half that can tell a fault from an interruption.** A node
        /// going unreachable used to be sent with no subject at all, so every client showed it
        /// the only way it could — a strip over whatever screen was open — and a node the
        /// operator was not looking at interrupted a pane on a different one. The node cannot
        /// know which pane is on screen and must not guess; naming itself is what lets the half
        /// that does know decide whether to say it loudly or leave it to the herd screen.
        ///
        /// Additive: absent is what every error carried before this, and a client that has never
        /// heard of the field behaves exactly as it does today.
        #[serde(skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
    #[serde(rename = "pong")]
    Pong { n: u64 },
}

impl ServerMsg {
    pub fn convo(pane: &str, page: Page, fresh: bool) -> Self {
        Self::Convo {
            pane: pane.to_string(),
            cursor: page.cursor,
            more: page.more,
            fresh,
            sub: None,
            turns: page.turns,
        }
    }

    /// A page of a conversation the pane's agent launched. Always `fresh`: it is not a slice of
    /// the pane's own transcript and has no turn in common with it, so there is nothing for a
    /// client to merge it into.
    pub fn convo_sub(pane: &str, sub: &str, page: Page) -> Self {
        Self::Convo {
            pane: pane.to_string(),
            cursor: page.cursor,
            more: page.more,
            fresh: true,
            sub: Some(sub.to_string()),
            turns: page.turns,
        }
    }
}

/// Every code an `error` frame can carry. One vocabulary rather than two: a node that spelled its
/// codes as strings beside a typed enum nothing constructed had already drifted — the enum was
/// missing three codes production emitted and carried one it never did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotWriter,
    UnknownPane,
    NodeOffline,
    HerdrUnavailable,
    BadRequest,
    NotFound,
    Revoked,
    /// `manage` only: an op this node has no verb for. Not on the v1 list, because v1 had no
    /// `manage`.
    Unsupported,
    /// The herd is reachable and this pane's *screen* is not. Distinct from `herdr_unavailable`,
    /// which is the socket being down and the whole herd with it: here the node is answering, the
    /// pane list is right, and only the frames are missing.
    StreamUnavailable,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "t")]
pub enum ClientMsg {
    #[serde(rename = "watch")]
    Watch {
        pane: String,
        #[serde(default)]
        scrollback: bool,
        #[serde(default)]
        conversation: bool,
    },
    #[serde(rename = "unwatch")]
    Unwatch { pane: String },
    #[serde(rename = "input")]
    Input {
        pane: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        b64: Option<String>,
        #[serde(default)]
        keys: Option<Vec<String>>,
    },
    /// Bytes for the pane to work on, written to a file on the pane's own node and typed in as
    /// the path to it.
    ///
    /// **The path is the point.** An agent reached over ssh reads a local file perfectly well; it
    /// is the terminal's own image-paste protocol that dies, so nothing here tries to speak one.
    /// `name` is a hint at the stem only — the node owns the directory and derives the extension
    /// from the bytes, because an extension the sender chose is an extension the sender chose.
    #[serde(rename = "paste")]
    Paste {
        pane: String,
        b64: String,
        #[serde(default)]
        name: Option<String>,
    },
    #[serde(rename = "answer")]
    Answer { pane: String, key: String },
    #[serde(rename = "convo.load")]
    ConvoLoad { pane: String, before: Option<String> },
    /// A page of a conversation this pane's agent *launched*, named by the handle a `sub` block
    /// carried. Its own verb rather than a field on `convo.load`, so that a node which has never
    /// heard of it ignores the frame exactly as it ignores any other unknown `t` — and so the
    /// page it answers with cannot be mistaken for the pane's own.
    #[serde(rename = "convo.sub")]
    ConvoSub {
        pane: String,
        id: String,
        #[serde(default)]
        before: Option<String>,
    },
    #[serde(rename = "resync")]
    Resync,
    #[serde(rename = "ping")]
    Ping { n: u64 },
}

/// How many pens one connection may be told about.
///
/// The wire says ids are stable for the life of a connection, so nothing can ever be evicted from
/// the table — which makes it an unbounded per-connection allocation, and a 24-bit gradient on a
/// 93x40 pane mints up to 3700 pens *per frame*. Real content uses hundreds, so this is far above
/// anything a terminal produces on purpose and still a ceiling.
const MAX_STYLES: usize = 4096;

/// How many bits of each RGB channel survive the fallback lookup. Three is 512 buckets a channel,
/// which puts a gradient's neighbours in the same bucket — the whole point of the degradation.
const COARSE_BITS: u8 = 3;

/// Per-connection style interning. Ids are stable for the life of a connection, and style 0 is
/// always the default pen, so a client can render before the first `styles` message arrives.
#[derive(Debug, Default)]
pub struct Encoder {
    entries: Vec<Style>,
    index: HashMap<Style, u32>,
    sent: u32,
    /// Built once the table fills, and never after: the entries it indexes cannot change again.
    nearest: HashMap<Style, u32>,
}

impl Encoder {
    pub fn new() -> Self {
        let default = Style::default();
        Self {
            entries: vec![default],
            index: HashMap::from([(default, 0)]),
            sent: 1,
            nearest: HashMap::new(),
        }
    }

    fn intern(&mut self, style: Style) -> u32 {
        if let Some(id) = self.index.get(&style) {
            return *id;
        }
        if self.entries.len() >= MAX_STYLES {
            return self.nearest_to(style);
        }
        let id = self.entries.len() as u32;
        self.entries.push(style);
        self.index.insert(style, id);
        id
    }

    /// The closest pen the client already holds. Additive and invisible to an old client: the
    /// wire still only ever names an id it was told about, and a gradient past the ceiling
    /// degrades in colour rather than in correctness.
    fn nearest_to(&mut self, style: Style) -> u32 {
        if self.nearest.is_empty() {
            for (id, held) in self.entries.iter().enumerate() {
                self.nearest.entry(coarse(*held)).or_insert(id as u32);
            }
        }
        self.nearest.get(&coarse(style)).copied().unwrap_or(0)
    }

    pub fn take_styles(&mut self) -> Option<Styles> {
        if self.sent as usize == self.entries.len() {
            return None;
        }
        let from = self.sent;
        self.sent = self.entries.len() as u32;
        Some(Styles {
            from,
            styles: self.entries[from as usize..].to_vec(),
        })
    }

    pub fn rows(&mut self, diffs: &[RowDiff]) -> Vec<RowRuns> {
        diffs
            .iter()
            .map(|d| RowRuns {
                row: d.row,
                runs: self.runs(&d.cells),
            })
            .collect()
    }

    fn runs(&mut self, cells: &[Cell]) -> Vec<Run> {
        let blank = Cell::default();
        let end = cells.iter().rposition(|c| *c != blank).map_or(0, |i| i + 1);
        let mut runs: Vec<Run> = Vec::new();
        let mut at = 0usize;
        for (i, cell) in cells[..end].iter().enumerate() {
            // The right half of a double-width glyph is carried by its lead's `w`, not by a
            // character of its own.
            if cell.is_tail() {
                continue;
            }
            let w = if cells.get(i + 1).is_some_and(Cell::is_tail) {
                2
            } else {
                1
            };
            let s = self.intern(Style {
                fg: cell.fg,
                bg: cell.bg,
                attrs: cell.attrs,
            });
            match runs.last_mut() {
                Some(r) if r.s == s && r.l == cell.link && r.w == w => {
                    r.x.push(cell.ch);
                    at += 1;
                }
                _ => {
                    runs.push(Run {
                        s,
                        x: cell.ch.to_string(),
                        l: cell.link,
                        w,
                        m: Vec::new(),
                    });
                    at = 0;
                }
            }
            let marks = cell.marks();
            if !marks.is_empty() {
                let run = runs.last_mut().expect("just pushed or matched");
                run.m.resize(at, String::new());
                run.m.push(marks.to_string());
            }
        }
        runs
    }

    /// Emits the `styles` message first when the update introduced new pens, so a client never
    /// sees a run referencing a style id it has not been told about.
    pub fn encode(&mut self, pane: &str, update: &PaneUpdate) -> Vec<ServerMsg> {
        let rows = self.rows(update.rows());
        let mut out = Vec::with_capacity(2);
        if let Some(s) = self.take_styles() {
            out.push(ServerMsg::Styles(s));
        }
        out.push(match update {
            PaneUpdate::Reset {
                cols,
                rows: n,
                cursor,
                links,
                ..
            } => ServerMsg::GridReset {
                pane: pane.to_string(),
                cols: *cols,
                rows: *n,
                rows_data: rows,
                cursor: *cursor,
                links: links.as_ref().clone(),
            },
            PaneUpdate::Patch {
                cursor, new_links, ..
            } => ServerMsg::GridPatch {
                pane: pane.to_string(),
                rows,
                cursor: *cursor,
                links: new_links.as_ref().clone(),
            },
        });
        out
    }

    pub fn encode_scrollback(&mut self, pane: &str, doc: &ScrollbackDoc) -> Vec<ServerMsg> {
        let rows = self.rows(&doc.rows);
        let mut out = Vec::with_capacity(2);
        if let Some(s) = self.take_styles() {
            out.push(ServerMsg::Styles(s));
        }
        out.push(ServerMsg::Scrollback {
            pane: pane.to_string(),
            from_top: doc.from_top,
            rows,
            total_rows: doc.total_rows,
            complete: doc.complete,
            capped: doc.capped,
        });
        out
    }
}
