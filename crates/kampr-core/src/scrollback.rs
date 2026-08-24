use crate::provider::RawScrollback;
use kampr_term::{Emulator, RowDiff, column_bound};

/// A memory bound, not a display one. Clients must not impose a row cap of their own: a terminal
/// surface fills the viewport and scrolls as far back as the node actually holds. At roughly 200
/// bytes of raw ANSI per row this is ~4 MB for a pane that has genuinely produced this much, and
/// most panes never approach it.
pub const DEFAULT_MAX_ROWS: usize = 20_000;

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
    rows: Vec<String>,
    /// The width a wrap has actually proved, or nothing. An estimate must never sit here: the
    /// ring restarts when this moves, and the rect moves on every pane the width probe reaches.
    cols: Option<u16>,
    /// Absolute index of `rows[0]`. Only ever increases.
    base: u32,
    capped: bool,
    max_rows: usize,
    rendered: Option<Vec<RowDiff>>,
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
            capped: false,
            max_rows: max_rows.max(1),
            rendered: None,
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
        self.rendered = None;
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
            let dropped = self.restart(incoming);
            return Ingest::Rewrapped { dropped };
        }
        if self.rows.is_empty() {
            self.rows = incoming;
            self.capped |= raw.truncated;
            self.trim();
            return Ingest::Fresh {
                rows: self.rows.len(),
            };
        }
        match overlap(&self.rows, &incoming) {
            0 => Ingest::Gap {
                dropped: self.restart(incoming),
            },
            k => {
                let added = incoming.len() - k;
                self.rows.extend_from_slice(&incoming[k..]);
                self.trim();
                Ingest::Stitched { added }
            }
        }
    }

    /// Held rather than rebuilt: a client polls this every three seconds per pane, and laying out
    /// twenty thousand rows of ANSI is tens of milliseconds of a tokio worker with no `.await` in
    /// it to yield at. Every path that moves a row drops the cache.
    pub fn render(&mut self) -> ScrollbackDoc {
        // A depth, not a highest index: the ring spans `from_top .. from_top + total_rows`.
        let total_rows = self.rows.len() as u32;
        let rows = match self.rendered.as_ref() {
            Some(rows) => rows.clone(),
            None => {
                let rows = lay_out(&self.rows, self.base);
                self.rendered = Some(rows.clone());
                rows
            }
        };
        ScrollbackDoc {
            from_top: self.base,
            rows,
            total_rows,
            complete: self.base == 0,
            capped: self.capped,
        }
    }

    /// The newest read shares nothing with what we hold. Splicing them would fabricate adjacency
    /// between two unrelated stretches of history, so the old rows go and the ring says it is
    /// capped from here.
    fn restart(&mut self, incoming: Vec<String>) -> usize {
        let dropped = self.rows.len();
        self.base += dropped as u32;
        self.rows = incoming;
        self.capped = true;
        self.trim();
        dropped
    }

    fn trim(&mut self) {
        if self.rows.len() <= self.max_rows {
            return;
        }
        let excess = self.rows.len() - self.max_rows;
        self.rows.drain(..excess);
        self.base += excess as u32;
        self.capped = true;
    }
}

/// **The grid is sized by the widest row it is about to be handed, never by the pane's width.**
/// A row wider than the grid wraps onto a second line, pushes the document past the grid's height
/// and `Grid::scroll_up` drops rows off the *top* — while `from_top`, `total_rows` and every row
/// index still describe the original span. That is a silent discard of exactly the kind ADR 0004
/// exists to make loud, and the label goes too narrow routinely (probe #68).
fn lay_out(rows: &[String], base: u32) -> Vec<RowDiff> {
    if rows.is_empty() {
        return Vec::new();
    }
    let cols = rows.iter().map(|r| column_bound(r)).max().unwrap_or(1).max(1);
    let mut term = Emulator::new(cols, rows.len().min(u16::MAX as usize) as u16);
    // herdr separates rows with LF alone, which moves down without returning the carriage.
    term.feed(rows.join("\r\n").as_bytes());
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
fn history_rows(raw: &RawScrollback) -> Vec<String> {
    let mut lines: Vec<&str> = raw.text.split('\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let keep = lines.len().saturating_sub(raw.viewport_rows as usize);
    lines[..keep].iter().map(|l| l.to_string()).collect()
}

/// Longest suffix of `held` that is also a prefix of `incoming`.
fn overlap(held: &[String], incoming: &[String]) -> usize {
    let max = held.len().min(incoming.len());
    (1..=max)
        .rev()
        .find(|k| held[held.len() - k..] == incoming[..*k])
        .unwrap_or(0)
}
