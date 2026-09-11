use crate::provider::RawScrollback;
use kampr_term::{Emulator, RowDiff, column_bound};

/// A memory bound, not a display one. At roughly 200 bytes of raw ANSI per row this is ~4 MB for a
/// pane that has genuinely produced this much, and most panes never approach it.
///
/// **No client may impose a cap of its own, and the client mirrors this one.** A surface that
/// stopped short of what the node holds would hide rows the operator can still be shown, which is
/// what this rule has always forbidden. But `send_history` re-bases every delta onto the client's
/// end, so a ring that trimmed here is invisible from there — a client whose depth only ever grew
/// ended up the sole copy of rows nothing could re-serve, since there is no `scrollback.load` and
/// [#51](#) says there cannot be one. `SCROLLBACK_MAX_ROWS` in `client/shared`'s `PaneState.kt` is
/// this number, and the two move together.
pub const DEFAULT_MAX_ROWS: usize = 20_000;

/// The other half of that bound, because a row has no length. A pane that writes one enormous line
/// per row fills the ring with 20 000 of them, and rows of 144 KB — which a hostile pane produces
/// at 80 columns, wearing marks — is 2.8 GB of `String` before anything is laid out on a grid.
///
/// Twice the ~4 MB a full ring of ordinary rows holds, so no pane that has genuinely produced this
/// much loses a row to it, and the document it becomes still fits the 16 MiB a mesh peer will
/// carry (`MAX_MESH_MESSAGE_BYTES`). It is the same ceiling the journal puts on one attachment.
const MAX_RING_BYTES: usize = 8 * 1024 * 1024;

/// The cells one document may be laid out on, which is the bound the other two do not give.
///
/// **Width and depth multiply.** `lay_out` sizes one grid by the widest row the ring holds, so a
/// single row of 65 535 columns beside twenty thousand ordinary ones is 1.3 billion cells — 52 GB
/// at 40 bytes each, in one allocation, which is `handle_alloc_error` and an abort rather than a
/// panic anything upstream could catch. Neither a row cap nor a byte cap sees it: those twenty
/// thousand rows are 200 KB.
///
/// A full 20 000-row ring of a 93-column pane is 1.9 M cells, so this is twice the deepest document
/// a real pane produces and a 200-column pane still reaches full depth on it. Past it the ring
/// trims from the top, which is what it already does for the other two bounds and what `capped`
/// already tells the client about.
const MAX_GRID_CELLS: usize = 4 * 1024 * 1024;

/// A row and the columns it would be laid out on, measured once when the read that brought it in
/// is parsed: [`ScrollbackRing::trim`] needs the widest row it is keeping to know how large a grid
/// the document it holds would ask for, and [`lay_out`] needs the same number again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    text: String,
    cols: u16,
}

impl Row {
    fn new(text: String) -> Self {
        let cols = column_bound(&text);
        Self { text, cols }
    }
}

#[derive(Debug, Clone)]
pub struct ScrollbackDoc {
    /// Absolute index of the first delivered row, counted from the top of the node's ring.
    pub from_top: u32,
    pub rows: Vec<RowDiff>,
    /// How many rows the ring holds, not the index it ends at: it spans
    /// `from_top .. from_top + total_rows`.
    pub total_rows: u32,
    pub complete: bool,
    /// True when history above `from_top` existed and is unreachable — herdr's read cap, a gap
    /// between reads, or the ring's own bound.
    pub capped: bool,
    /// Which run of rows this document belongs to. **A reader holding a document of an older era
    /// holds rows that are not this one's ancestors**, however adjacent the indices look, and must
    /// throw them away rather than append.
    ///
    /// It exists because the indices cannot say it. A ring that is discarded and filled again
    /// advances `base` past everything it dropped, so the refill lands exactly where a tail would
    /// land — and a harness taking the alternate screen and giving it back is that discard and
    /// that refill, twice per session, with the *same rows* arriving each time (probe #498). Every
    /// consumer downstream read it as the pane having produced its whole shell era over again: the
    /// phone client carried a parked reader up by one ring per delivery, into the era it had just
    /// been handed.
    ///
    /// Growth never moves it, and neither does trimming — a trimmed row keeps the index it had.
    pub era: u32,
}

/// How many rows either side of a join have to agree before it is made. Enough that repeated
/// content cannot satisfy it by accident, short enough to survive a ring that trimmed one row.
const ANCHOR: usize = 8;

/// A hole between what the ring holds and what the next read starts at, and the range to fetch to
/// close it.
///
/// `fetch` reaches back over the ring's own tail by `anchor` rows. Those rows are what prove the
/// fetched history is *this* pane's history and not a renumbered ring's (#520); they are compared,
/// not appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    pub hole: (u32, u32),
    pub fetch: (u32, u32),
    pub anchor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingest {
    Fresh {
        rows: usize,
    },
    Stitched {
        added: usize,
    },
    /// No overlap with what we hold: output outran the poll, so the two reads cannot be joined.
    Gap {
        dropped: usize,
    },
    /// The pane changed width, so every stored row was re-wrapped underneath us.
    Rewrapped {
        dropped: usize,
    },
}

/// History accumulated across reads.
///
/// `pane.read recent` returns at most the newest 1000 rows and takes no offset (probe #51), so a
/// single read can never reach past the cap. Successive reads overlap while the node is watching,
/// and the overlap is what lets the ring grow deeper than any one read.
#[derive(Debug, Clone)]
pub struct ScrollbackRing {
    rows: Vec<Row>,
    /// The width a wrap has actually proved, or nothing. An estimate must never sit here: the
    /// ring restarts when this moves, and the rect moves on every pane the width probe reaches.
    cols: Option<u16>,
    /// Absolute index of `rows[0]`. Only ever increases.
    base: u32,
    /// Where the ring's **last** row sits in the *provider's* history, and `None` while no read has
    /// said. Distinct from [`Self::base`], which counts in the ring's own space and never moves
    /// backwards: this one follows herdr's numbering, which a reflow or a retention trim renumbers
    /// wholesale (probe #520). It is only ever compared against a `first_row` from the same read
    /// generation, and every path that stops holding what it held replaces it.
    end_abs: Option<u32>,
    /// See [`ScrollbackDoc::era`]. Bumped by every path that stops holding what it held.
    era: u32,
    capped: bool,
    max_rows: usize,
    rendered: Option<Vec<RowDiff>>,
    /// The width [`rendered`](Self::rendered) was laid out at. A row arriving wider than every row
    /// before it changes the grid a full lay-out would produce, so the cache stops being the thing
    /// a full lay-out would have built and has to go.
    rendered_cols: u16,
}

impl Default for ScrollbackRing {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ROWS)
    }
}

impl ScrollbackRing {
    pub fn new(max_rows: usize) -> Self {
        Self {
            rows: Vec::new(),
            cols: None,
            base: 0,
            end_abs: None,
            era: 0,
            capped: false,
            max_rows: max_rows.max(1),
            rendered: None,
            rendered_cols: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn capped(&self) -> bool {
        self.capped
    }

    /// Absolute index of the ring's first row. Read beside [`Self::len`] and [`Self::capped`] as
    /// the whole of what a rendered document is: a ring whose three of them have not moved
    /// renders identically, so that triple is what tells a watcher there is something new to
    /// send without laying twenty thousand rows out to find out.
    pub fn base(&self) -> u32 {
        self.base
    }

    /// The absolute rows this read has left out, when it has left any out.
    ///
    /// Asked *before* [`Self::ingest`], so a caller can fetch them and hand them back. `None` when
    /// the read continues what is held, when either side has no position, or when the pane
    /// re-wrapped underneath — a reflow renumbers herdr's whole row space (#520), so the rows a
    /// stale `end_abs` names are not the rows it meant.
    pub fn gap_before(&self, raw: &RawScrollback) -> Option<Gap> {
        let start = raw.first_row?;
        let end = self.end_abs?;
        if self.rows.is_empty() {
            return None;
        }
        if matches!((raw.cols, self.cols), (Some(now), Some(was)) if now != was) {
            return None;
        }
        if start <= end.saturating_add(1) {
            return None;
        }
        // The fetch reaches back over the ring's own tail, because rows that merely sit next to
        // what is held prove nothing: it is the overlap that says these are the same history.
        let anchor = ANCHOR.min(self.rows.len()) as u32;
        Some(Gap {
            hole: (end.saturating_add(1), start.saturating_sub(1)),
            fetch: (
                end.saturating_add(1).saturating_sub(anchor),
                start.saturating_sub(1),
            ),
            anchor: anchor as usize,
        })
    }

    /// Fills a gap with rows fetched by position, **only if they demonstrably continue what is
    /// held**, and answers whether they did.
    ///
    /// The check is the whole of the safety. herdr's absolute row numbers are positions in its
    /// current ring rather than identities: a retention trim renumbers them wholesale and reports
    /// nothing about how far (#520), so a `missing` fetched against a ring that moved underneath
    /// would splice unrelated history into the operator's scrollback and call it theirs. Measured
    /// firing: with herdr trimmed to 663 rows under a burst, its reachable top was `B-005339`
    /// while the ring's tail was `A-000861`, and the anchor refused (#523).
    ///
    /// A refusal is not a failure — the caller falls back to the honest discard, which is what it
    /// did before any of this existed.
    pub fn splice(&mut self, fetched: Vec<String>, raw: &RawScrollback) -> bool {
        let Some(gap) = self.gap_before(raw) else {
            return false;
        };
        let wanted = (gap.fetch.1 as usize)
            .checked_sub(gap.fetch.0 as usize)
            .and_then(|n| n.checked_add(1));
        if wanted != Some(fetched.len()) {
            return false;
        }
        let n = gap.anchor;
        if n == 0 || n > fetched.len() {
            return false;
        }
        let missing = fetched;
        // The fetched rows are plain and the held rows carry their attributes, so they are compared
        // on what they have in common: the characters, trimmed of the `\r` a CRLF read leaves on
        // every row (#521).
        let held_tail = &self.rows[self.rows.len() - n..];
        if !held_tail
            .iter()
            .zip(missing.iter().take(n))
            .all(|(a, b)| plain(&a.text) == plain(b))
        {
            return false;
        }
        self.rendered = None;
        self.rows.extend(
            missing
                .into_iter()
                .skip(n)
                .map(|t| Row::new(format!("{}\r", plain(&t)))),
        );
        self.end_abs = Some(gap.hole.1);
        self.trim();
        true
    }

    pub fn ingest(&mut self, raw: &RawScrollback) -> Ingest {
        let incoming = history_rows(raw);
        // A read that comes back as the live viewport and nothing else is not history that
        // disagrees with what is held — it is no news about history at all. A full-screen program
        // has the pane and herdr has no ring to offer for as long as it does (#244). The ring is
        // the node's own accumulation and outlives that; treating the silence as a gap threw the
        // operator's whole scrollback away and rebased the ring, and a rebase is indistinguishable
        // from growth to every consumer downstream, so each of them dropped its copy too.
        if incoming.is_empty() && !self.rows.is_empty() {
            return Ingest::Stitched { added: 0 };
        }
        // A width change re-wraps every stored row, so nothing older can be trusted to line up.
        // The ring adopts the new width *before* restarting on it (probe #112): a restart that
        // kept the old one would find every later read disagreeing with it too, and throw the
        // whole ring away on every read for as long as the pane stayed that width.
        //
        // **A width arriving where there was none is not a change.** The first reads of a
        // freshly-watched pane land before anything has measured it (probe #68), and a ring that
        // restarted when the label resolved from nothing to the PTY's own width would flush the
        // operator's history on every split pane they opened.
        let rewrapped =
            matches!((raw.cols, self.cols), (Some(now), Some(was)) if now != was) && !self.rows.is_empty();
        if let Some(cols) = raw.cols {
            self.cols = Some(cols);
        }
        if rewrapped {
            // The position goes with the width. A reflow renumbers herdr's whole row space (#520),
            // so an `end_abs` carried across one would be compared against numbers that no longer
            // mean what it meant — which is the mis-splice this position exists to prevent.
            let dropped = self.restart_at(incoming, raw.first_row);
            return Ingest::Rewrapped { dropped };
        }
        if self.rows.is_empty() {
            // **A ring that has already dropped rows is not being filled for the first time.**
            // `rows` is empty here for one of two reasons and they are opposites: a pane that has
            // never scrolled, where the rows now arriving really did just leave the live grid; or
            // a ring a harness superseded, where they did not — they are the era from before it,
            // handed back untouched when it gave the screen up (#244, #438). `base` is what tells
            // the two apart, because nothing but a discard puts it above zero with nothing held.
            if self.base > 0 {
                self.era += 1;
            }
            self.end_abs = raw
                .first_row
                .map(|s| s.saturating_add(incoming.len().saturating_sub(1) as u32));
            self.rendered = None;
            self.rows = incoming;
            self.capped |= raw.truncated;
            self.trim();
            return Ingest::Fresh {
                rows: self.rows.len(),
            };
        }
        // **A position, when the provider can give one, rather than a suffix match.** `overlap`
        // finds the longest suffix of what is held that prefixes what arrived, and on output that
        // repeats itself the longest such run is not the true one — a burst that pushed the window
        // clean past everything held spliced at an offset nothing chose and the document came out
        // `complete: true` over the hole (probe #522). A read that knows where it starts settles
        // this arithmetically, and the only thing it can be wrong about is a provider that lied.
        if let (Some(start), Some(end)) = (raw.first_row, self.end_abs) {
            return match start > end.saturating_add(1) {
                true => Ingest::Gap {
                    dropped: self.restart_at(incoming, raw.first_row),
                },
                false => {
                    // How much of the incoming window this ring already holds. Saturating because
                    // a window that starts *below* the ring's own base is one the ring outgrew,
                    // which is history it keeps rather than rows it re-adds.
                    let already =
                        usize::try_from(end.saturating_add(1).saturating_sub(start)).unwrap_or(usize::MAX);
                    let added = incoming.len().saturating_sub(already);
                    if added > 0 {
                        self.rows.extend_from_slice(&incoming[incoming.len() - added..]);
                        self.end_abs = Some(end.saturating_add(added as u32));
                        self.extend_rendered(added);
                        self.trim();
                    }
                    Ingest::Stitched { added }
                }
            };
        }
        match overlap(&self.rows, &incoming) {
            0 => Ingest::Gap {
                dropped: self.restart_at(incoming, raw.first_row),
            },
            k => {
                let added = incoming.len() - k;
                self.rows.extend_from_slice(&incoming[k..]);
                self.end_abs = self.end_abs.map(|e| e.saturating_add(added as u32));
                self.extend_rendered(added);
                self.trim();
                Ingest::Stitched { added }
            }
        }
    }

    /// The width a full lay-out would use: the widest row held, never the pane's own.
    ///
    /// It is read over the whole ring rather than over the rows being laid out, so that laying
    /// out a tail produces exactly the rows laying out everything would have produced. See
    /// [`lay_out`] for what a grid too narrow for its widest row does.
    fn layout_cols(&self) -> u16 {
        self.rows.iter().map(|r| r.cols).max().unwrap_or(1).max(1)
    }

    fn ensure_rendered(&mut self) {
        if self.rendered.is_some() {
            return;
        }
        let cols = self.layout_cols();
        self.rendered = Some(lay_out(&self.rows, self.base, cols));
        self.rendered_cols = cols;
    }

    /// Lay out the rows just appended and join them onto what is already laid out.
    ///
    /// **This is what lets history keep up with the grid.** A full lay-out is linear in the ring's
    /// whole depth — measured at 2 ms per thousand rows, so 55 ms at the 20 000-row bound — and
    /// paying it on every read is what forced the poll and the socket's floor to be slow enough
    /// that the rows a reader was looking at could be seconds behind the pane (probe #529). An
    /// append costs its own rows and nothing else.
    ///
    /// **It is sound because herdr's rows do not lean on each other.** `pane.read source=recent
    /// format=ansi` re-emits every row's colour on the row itself and closes it again at the end —
    /// a colour set once and spanning three lines comes back as three independently styled rows,
    /// and so does an unclosed bold (probe #530). So a row laid out on its own is the row it would
    /// have been laid out as in company, and the only thing that can invalidate that is the grid
    /// getting wider underneath it.
    fn extend_rendered(&mut self, added: usize) {
        if added == 0 || self.rendered.is_none() {
            return;
        }
        let cols = self.layout_cols();
        if cols != self.rendered_cols {
            self.rendered = None;
            return;
        }
        let from = self.rows.len() - added;
        let laid = lay_out(&self.rows[from..], self.base + from as u32, cols);
        if let Some(cached) = self.rendered.as_mut() {
            cached.extend(laid);
        }
    }

    /// Held rather than rebuilt: laying out twenty thousand rows of ANSI is tens of milliseconds
    /// of a tokio worker with no `.await` in it to yield at. Every path that moves a row already
    /// held drops the cache; an append extends it.
    pub fn render(&mut self) -> ScrollbackDoc {
        self.render_from(None)
    }

    /// The tail a reader has not been sent, or the whole ring when what they hold is not this
    /// ring's to add to.
    ///
    /// `sent_era` is the half that cannot be inferred: a ring that was discarded and filled again
    /// advances past everything it dropped, so a refill lands exactly where a tail would land and
    /// the indices alone cannot tell the two apart (see [`ScrollbackDoc::era`]).
    pub fn render_since(&mut self, sent_rows: u32, sent_era: u32) -> ScrollbackDoc {
        let from = (self.era == sent_era).then_some(sent_rows);
        self.render_from(from)
    }

    fn render_from(&mut self, from: Option<u32>) -> ScrollbackDoc {
        self.ensure_rendered();
        // A depth, not a highest index: the ring spans `from_top .. from_top + total_rows`.
        let end = self.base + self.rows.len() as u32;
        let from_top = from.unwrap_or(self.base).clamp(self.base, end);
        let cached = self.rendered.as_deref().unwrap_or_default();
        let at = cached.partition_point(|r| r.row < from_top);
        ScrollbackDoc {
            from_top,
            rows: cached[at..].to_vec(),
            total_rows: end - from_top,
            complete: self.base == 0,
            capped: self.capped,
            era: self.era,
        }
    }

    /// A harness took the pane's screen, so there is no ring behind it and will not be one until
    /// the harness exits — and everything held is from whatever ran *before* it.
    ///
    /// Claude Code does not clear the scrollback. It sets `\e[?1049h` and takes the **alternate
    /// screen**, measured straight off its own pty with no `\e[3J` anywhere, which is probe #244:
    /// herdr's ring goes away for as long as a full-screen program holds the pane and comes back
    /// untouched on exit. The conversation never enters that ring at all — a real session driven
    /// to two full replies kept `max_offset_from_bottom` at 0 throughout, answered every read with
    /// exactly the viewport, and gave the shell era back on exit with not one conversation row in
    /// it. So the rows this drops are the last thing the pane did before the harness started, and
    /// serving a `git log` one screen above a Claude conversation is what made an operator stop
    /// believing the surface.
    ///
    /// **Not the same thing as a read that came back short.** [`Self::ingest`] must go on treating
    /// that as no news (#244 from the other side: a pager holds the screen for a moment and gives
    /// it back, and the ring outlives it). The difference is *whose* screen it is, which only the
    /// provider can answer — see `Provider::harness_owns_the_screen`.
    ///
    /// Runs on every poll for the harness's whole life, so it is a no-op once the ring is empty:
    /// a base that kept advancing would rebase the client's copy every three seconds on a pane
    /// where nothing had happened at all.
    pub fn superseded(&mut self) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        let dropped = self.rows.len();
        self.rendered = None;
        self.base += dropped as u32;
        self.era += 1;
        self.rows.clear();
        // Not "there is no history": there is, and this node cannot reach it (#233).
        self.capped = true;
        dropped
    }

    /// The newest read shares nothing with what we hold. Splicing them would fabricate adjacency
    /// between two unrelated stretches of history, so the old rows go and the ring says it is
    /// capped from here.
    /// Starts again on `incoming`, remembering where it sits in the provider's history so the
    /// *next* read can be joined by position rather than by a suffix match.
    fn restart_at(&mut self, incoming: Vec<Row>, first_row: Option<u32>) -> usize {
        self.rendered = None;
        let dropped = self.rows.len();
        self.base += dropped as u32;
        self.era += 1;
        self.end_abs = first_row.map(|s| s.saturating_add(incoming.len().saturating_sub(1) as u32));
        self.rows = incoming;
        self.capped = true;
        self.trim();
        dropped
    }

    /// How deep the ring can stay: rows, bytes, and the cells the two of them would be laid out
    /// on together. Walked from the newest row back, because the newest is the one row that is
    /// never dropped — it is what the pane is doing now, and a ring that answered a 9 MB row with
    /// nothing would be a pane that had gone blank.
    fn trim(&mut self) {
        let mut kept = 0usize;
        let mut bytes = 0usize;
        let mut cols = 0u16;
        for row in self.rows.iter().rev() {
            let widest = cols.max(row.cols);
            let deeper = kept + 1;
            let over = deeper > self.max_rows
                || bytes + row.text.len() > MAX_RING_BYTES
                || widest as usize * deeper > MAX_GRID_CELLS;
            if kept > 0 && over {
                break;
            }
            kept = deeper;
            bytes += row.text.len();
            cols = widest;
        }
        let excess = self.rows.len() - kept;
        if excess == 0 {
            return;
        }
        self.rows.drain(..excess);
        self.base += excess as u32;
        // The same rows off the front of the cache. A trimmed row keeps the absolute index it
        // had, so what is left still lines up with what is held.
        if let Some(cached) = self.rendered.as_mut() {
            cached.drain(..excess.min(cached.len()));
        }
        self.capped = true;
    }
}

/// **The grid is sized by the widest row it is about to be handed, never by the pane's width.**
/// A row wider than the grid wraps onto a second line, pushes the document past the grid's height
/// and `Grid::scroll_up` drops rows off the *top* — while `from_top`, `total_rows` and every row
/// index still describe the original span. That is a silent discard of exactly the kind ADR 0004
/// exists to make loud, and the label goes too narrow routinely (probe #68).
fn lay_out(rows: &[Row], base: u32, cols: u16) -> Vec<RowDiff> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut term = Emulator::new(cols, rows.len().min(u16::MAX as usize) as u16);
    // herdr separates rows with LF alone, which moves down without returning the carriage.
    let joined: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    term.feed(joined.join("\r\n").as_bytes());
    let grid = term.grid();
    (0..grid.rows())
        .map(|r| RowDiff {
            row: base + r as u32,
            cells: grid.row(r).to_vec(),
        })
        .collect()
}

/// The rows of a read that are history: `recent` hands back the live viewport too, and that
/// already travels as the grid.
fn history_rows(raw: &RawScrollback) -> Vec<Row> {
    let mut lines: Vec<&str> = raw.text.split('\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let keep = lines.len().saturating_sub(raw.viewport_rows as usize);
    lines[..keep].iter().map(|l| Row::new(l.to_string())).collect()
}

/// A row reduced to what a fetched row and a held row can both be compared on: its characters,
/// with SGR gone and the trailing `\r` a CRLF read leaves behind (#521) trimmed off.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Every escape this can meet is a CSI or an OSC, and both end on a byte this skips to.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() || c == '\u{7}' || c == '\u{5c}' {
                break;
            }
        }
    }
    out.trim_end_matches(['\r', ' ']).to_string()
}

/// Longest suffix of `held` that is also a prefix of `incoming`.
fn overlap(held: &[Row], incoming: &[Row]) -> usize {
    let max = held.len().min(incoming.len());
    (1..=max)
        .rev()
        .find(|k| held[held.len() - k..] == incoming[..*k])
        .unwrap_or(0)
}
