use kampr_core::registry::PaneUpdate;
use kampr_core::scrollback::ScrollbackDoc;
use kampr_core::wire::{Cursor, RowRuns, Run, Style, Styles};
use kampr_term::{Cell, RowDiff};
use std::sync::Arc;

/// The widest grid this process will allocate for, whatever a far end claims.
///
/// `grid.reset` carries geometry as two `u16`s, and a shadow that believes them turns a ~100 byte
/// frame claiming `65535x65535` into a **159 GiB** allocation — an OOM, not an error. No message
/// ceiling bounds it, because the allocation is derived from the claim rather than from the bytes.
/// `StyleTable::absorb` already refuses the same shape for the same reason; the grid did not.
///
/// The numbers are generous against every pane ever measured here: the widest was 292 columns
/// (#265) and a headless PTY is 93 (#68), so 4096 columns is an order of magnitude of headroom and
/// the cell budget is ~36x the largest real grid. A far end that wants more than this is not a
/// terminal.
pub const MAX_COLS: u16 = 4096;
pub const MAX_ROWS: u16 = 4096;
pub const MAX_GRID_CELLS: usize = 1 << 20;

/// Pens and hyperlinks one link may make this process hold, **per pane**.
///
/// Both tables are appended to across messages and nothing ever evicts from either, so neither is
/// bounded by the message ceiling: a far end that sends a small `styles` or a small `grid.patch`
/// often enough grows them without limit. The clamp above bounds one claim; these bound a habit.
///
/// Neither number can be reached by anything speaking this protocol honestly, because each is the
/// ceiling the *sending* side already holds itself to: `kampr-core`'s `Encoder::MAX_STYLES` stops
/// minting pens at 4096 per connection and degrades to the nearest it already sent, and
/// `kampr-term`'s own `MAX_LINKS` stops interning at 4096 *distinct* URIs — past a screenful of
/// entirely distinct hyperlinks on a 93x40 pane. A far end past either is not one of ours.
pub const MAX_STYLES: usize = 4096;
pub const MAX_LINKS: usize = 4096;

/// Rows lose before columns do: a width is what every stored row was wrapped at, so cropping it
/// makes the rows lie, while cropping the row count only shows less of a screen that is already
/// clipped to a viewport.
fn budget(cols: u16, rows: u16) -> (u16, u16) {
    let cols = cols.min(MAX_COLS);
    let rows = rows.min(MAX_ROWS);
    if cols == 0 {
        return (cols, rows);
    }
    let affordable = (MAX_GRID_CELLS / cols as usize).min(u16::MAX as usize) as u16;
    (cols, rows.min(affordable))
}

/// A link's style table, rebuilt from the `styles` messages the far end sends.
///
/// Ids are only promised to be stable for the life of a connection, so this is per link and is
/// thrown away with it — and it is *not* forwarded to clients, who each get ids minted by their
/// own encoder. Decoding here and re-encoding there is what lets the WAN hop stay compressed
/// while every client keeps a table it was actually told about.
#[derive(Debug)]
pub struct StyleTable {
    entries: Vec<Style>,
}

impl Default for StyleTable {
    fn default() -> Self {
        Self {
            entries: vec![Style::default()],
        }
    }
}

impl StyleTable {
    /// `from` is where the far end says this batch starts, so the table is resized to it before
    /// the batch is appended — id 0 stays the default pen whatever arrives.
    ///
    /// A batch that starts *past* the end of this table is refused, because ids are minted
    /// append-only by one encoder per link and there is no honest way to skip: the number would
    /// only ever be the peer's own count of what it has already sent. Resizing to it instead
    /// would let `{"t":"styles","from":4294967295,"styles":[]}` — a message of forty bytes, well
    /// under any size ceiling — ask the hub for four billion entries in one allocation.
    #[must_use]
    pub fn absorb(&mut self, message: &Styles) -> bool {
        if message.from as usize > self.entries.len() {
            return false;
        }
        if message.from as usize + message.styles.len() > MAX_STYLES {
            return false;
        }
        self.entries
            .resize((message.from as usize).max(1), Style::default());
        self.entries.extend(message.styles.iter().copied());
        true
    }

    pub fn get(&self, id: u32) -> Style {
        self.entries.get(id as usize).copied().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Runs are contiguous from column 0 and trailing default cells are omitted, so the row is padded
/// back out to the pane's width here — a client is entitled to a full row either way.
pub fn decode_row(runs: &[Run], styles: &StyleTable, cols: u16) -> Vec<Cell> {
    let mut row = Vec::with_capacity(cols as usize);
    for run in runs {
        let style = styles.get(run.s);
        let width = if run.w >= 2 { 2 } else { 1 };
        for (i, ch) in run.x.chars().enumerate() {
            let cell = Cell {
                ch,
                fg: style.fg,
                bg: style.bg,
                attrs: style.attrs,
                link: run.l,
                marks: run
                    .m
                    .get(i)
                    .filter(|m| !m.is_empty())
                    .map(|m| Arc::new(m.clone())),
            };
            let tail = (width == 2).then(|| cell.tail());
            row.push(cell);
            row.extend(tail);
        }
    }
    row.resize(cols as usize, Cell::default());
    row
}

/// The hub's copy of a remote pane's grid.
///
/// The peer runs the one emulator; this is only its last published output. It exists so that a
/// second client joining costs no round trip, and so that a client the hub has to purge can be
/// reset from memory rather than by asking the far end for a repaint it already sent.
#[derive(Debug, Default)]
pub struct Shadow {
    cols: u16,
    rows: u16,
    grid: Vec<Vec<Cell>>,
    cursor: Cursor,
    links: Vec<String>,
    ready: bool,
}

impl Shadow {
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn geometry(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// The grid itself, borrowed. A renderer draws from this every frame, and [`Self::full`]
    /// clones the whole thing — which is right for re-serving a joining watcher and wrong for a
    /// draw path where text shaping is already the entire cost of a frame (#58–#62).
    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.grid
    }

    /// The pane's link table, which a run's `l` indexes into. Borrowed for the same reason
    /// [`Self::rows`] is.
    pub fn links(&self) -> &[String] {
        &self.links
    }

    pub fn link(&self, id: u32) -> Option<&str> {
        self.links.get(id as usize).map(String::as_str)
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn reset(
        &mut self,
        cols: u16,
        rows: u16,
        rows_data: &[RowRuns],
        cursor: Cursor,
        links: Vec<String>,
        styles: &StyleTable,
    ) -> PaneUpdate {
        let (cols, rows) = budget(cols, rows);
        self.cols = cols;
        self.rows = rows;
        self.grid = vec![vec![Cell::default(); cols as usize]; rows as usize];
        self.cursor = cursor;
        self.links = links;
        self.links.truncate(MAX_LINKS);
        self.ready = true;
        for row in rows_data {
            self.write(row, styles);
        }
        self.full()
    }

    pub fn patch(
        &mut self,
        rows: &[RowRuns],
        cursor: Cursor,
        new_links: Vec<String>,
        styles: &StyleTable,
    ) -> Option<PaneUpdate> {
        if !self.ready {
            return None;
        }
        self.cursor = cursor;
        // Truncated rather than refused: a link id past the table resolves to no URI, which costs
        // an OSC 8 target its Open strip, and closing a peer's whole link over a hyperlink would
        // cost every pane on it.
        self.links.extend(new_links.iter().cloned());
        self.links.truncate(MAX_LINKS);
        let changed: Vec<RowDiff> = rows
            .iter()
            .filter_map(|row| {
                self.write(row, styles);
                (row.row < self.rows as u32).then(|| RowDiff {
                    row: row.row,
                    cells: self.grid[row.row as usize].clone(),
                })
            })
            .collect();
        Some(PaneUpdate::Patch {
            rows: Arc::new(changed),
            cursor,
            new_links: Arc::new(new_links),
        })
    }

    /// Everything a joining watcher needs in one message, which is the same thing a purged one
    /// needs: the protocol has exactly one repair and this is it.
    pub fn full(&self) -> PaneUpdate {
        PaneUpdate::Reset {
            cols: self.cols,
            rows: self.rows,
            rows_data: Arc::new(
                self.grid
                    .iter()
                    .enumerate()
                    .map(|(row, cells)| RowDiff {
                        row: row as u32,
                        cells: cells.clone(),
                    })
                    .collect(),
            ),
            cursor: self.cursor,
            links: Arc::new(self.links.clone()),
        }
    }

    fn write(&mut self, row: &RowRuns, styles: &StyleTable) {
        if row.row >= self.rows as u32 {
            return;
        }
        self.grid[row.row as usize] = decode_row(&row.runs, styles, self.cols);
    }
}

/// A remote pane's history, stitched from the deltas the peer sends.
///
/// History is append-only and keyed on absolute row index, so a hole in it is not repairable by
/// any later message — which is why a backpressure purge may never drop one. The peer already
/// advanced `from_top` past anything it lost, so a delta that does not touch what is held is a
/// gap the hub reports rather than papers over.
#[derive(Debug, Default)]
pub struct History {
    from_top: u32,
    rows: Vec<RowDiff>,
    complete: bool,
    capped: bool,
    /// The peer's own era, carried through unchanged — the hub renumbers nothing. See
    /// [`ScrollbackDoc::era`]: it is the only thing that tells a refill from a tail, because the
    /// two land on the same index.
    era: u32,
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Answers whether what is now held replaces what was, rather than continuing it — which the
    /// caller has to relay as a document of its own, since a delta "since the old end" cannot say
    /// it.
    pub fn absorb(&mut self, doc: &ScrollbackDoc) -> bool {
        self.complete = doc.complete;
        self.capped |= doc.capped;
        let restart = doc.era != self.era
            || doc.from_top < self.from_top
            || doc.from_top > self.from_top + self.rows.len() as u32;
        self.era = doc.era;
        if restart {
            self.from_top = doc.from_top;
            self.rows.clear();
            self.capped |= doc.from_top > 0;
        }
        for row in &doc.rows {
            let Some(index) = row.row.checked_sub(self.from_top).map(|i| i as usize) else {
                continue;
            };
            let row = RowDiff {
                row: row.row,
                cells: row.cells.clone(),
            };
            match index.cmp(&self.rows.len()) {
                std::cmp::Ordering::Less => self.rows[index] = row,
                std::cmp::Ordering::Equal => self.rows.push(row),
                std::cmp::Ordering::Greater => {}
            }
        }
        restart
    }

    pub fn doc(&self) -> ScrollbackDoc {
        ScrollbackDoc {
            from_top: self.from_top,
            rows: self.rows.clone(),
            total_rows: self.rows.len() as u32,
            complete: self.complete,
            capped: self.capped,
            era: self.era,
        }
    }

    /// Rows at or above `sent`, as a document a watcher can append by index.
    pub fn since(&self, sent: u32) -> Option<ScrollbackDoc> {
        let end = self.from_top + self.rows.len() as u32;
        if end <= sent && sent >= self.from_top {
            return None;
        }
        let start = sent.max(self.from_top);
        let rows: Vec<RowDiff> = self.rows.iter().filter(|r| r.row >= start).cloned().collect();
        if rows.is_empty() {
            return None;
        }
        Some(ScrollbackDoc {
            from_top: start,
            total_rows: rows.len() as u32,
            rows,
            complete: self.complete,
            capped: self.capped,
            era: self.era,
        })
    }

    pub fn end(&self) -> u32 {
        self.from_top + self.rows.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_term::Color;

    fn run(s: u32, text: &str) -> Run {
        Run {
            s,
            x: text.into(),
            l: None,
            w: 1,
            m: Vec::new(),
        }
    }

    fn table() -> StyleTable {
        let mut styles = StyleTable::default();
        assert!(styles.absorb(&Styles {
            from: 1,
            styles: vec![Style {
                fg: Color::Indexed(9),
                ..Style::default()
            }],
        }));
        styles
    }

    fn text(update: &PaneUpdate) -> Vec<String> {
        update
            .rows()
            .iter()
            .map(|r| r.cells.iter().map(|c| c.ch).collect::<String>())
            .collect()
    }

    #[test]
    fn a_style_table_keeps_the_default_pen_at_zero() {
        let styles = table();
        assert_eq!(styles.get(0), Style::default());
        assert_eq!(styles.get(1).fg, Color::Indexed(9));
        assert_eq!(styles.get(99), Style::default(), "an id we were never told about");
    }

    #[test]
    fn a_second_styles_message_appends_rather_than_replacing() {
        let mut styles = table();
        assert!(styles.absorb(&Styles {
            from: 2,
            styles: vec![Style {
                fg: Color::Indexed(4),
                ..Style::default()
            }],
        }));
        assert_eq!(styles.get(1).fg, Color::Indexed(9));
        assert_eq!(styles.get(2).fg, Color::Indexed(4));
        assert_eq!(styles.len(), 3);
    }

    #[test]
    fn a_reset_rebuilds_the_grid_and_pads_short_rows() {
        let mut shadow = Shadow::default();
        let update = shadow.reset(
            5,
            2,
            &[RowRuns {
                row: 0,
                runs: vec![run(0, "hi")],
            }],
            Cursor {
                col: 2,
                row: 0,
                visible: true,
            },
            vec!["https://herdr.dev".into()],
            &table(),
        );
        assert_eq!(text(&update), ["hi   ", "     "]);
        assert!(shadow.is_ready());
        match update {
            PaneUpdate::Reset {
                cols, rows, links, ..
            } => {
                assert_eq!((cols, rows), (5, 2));
                assert_eq!(links.len(), 1);
            }
            other => panic!("expected a reset, got {other:?}"),
        }
    }

    #[test]
    fn a_patch_lands_on_the_shadow_so_a_later_joiner_sees_it() {
        let styles = table();
        let mut shadow = Shadow::default();
        shadow.reset(
            5,
            2,
            &[RowRuns {
                row: 0,
                runs: vec![run(0, "hi")],
            }],
            Cursor::default(),
            vec![],
            &styles,
        );
        let patch = shadow
            .patch(
                &[RowRuns {
                    row: 1,
                    runs: vec![run(1, "done")],
                }],
                Cursor {
                    col: 4,
                    row: 1,
                    visible: true,
                },
                vec!["https://kampr.dev".into()],
                &styles,
            )
            .unwrap();
        assert_eq!(text(&patch), ["done "]);
        assert_eq!(text(&shadow.full()), ["hi   ", "done "]);
        let PaneUpdate::Reset { links, .. } = shadow.full() else {
            panic!("full is a reset");
        };
        assert_eq!(links.as_ref(), &["https://kampr.dev".to_string()]);
        assert_eq!(shadow.full().rows()[1].cells[0].fg, Color::Indexed(9));
    }

    /// A hub re-encodes what it decodes, so anything `decode_row` throws away is thrown away for
    /// every client behind the hub. It used to spend one cell per character however wide the run
    /// said the character was, which column-shifted every wide glyph on a relayed pane, and it had
    /// nowhere to put a mark.
    #[test]
    fn a_relayed_row_keeps_its_columns_and_its_marks() {
        let mut shadow = Shadow::default();
        let update = shadow.reset(
            8,
            1,
            &[RowRuns {
                row: 0,
                runs: vec![
                    Run {
                        s: 0,
                        x: "e".into(),
                        l: None,
                        w: 1,
                        m: vec!["\u{301}".into()],
                    },
                    Run {
                        s: 0,
                        x: "\u{65E5}".into(),
                        l: None,
                        w: 2,
                        m: Vec::new(),
                    },
                    run(0, "f"),
                ],
            }],
            Cursor::default(),
            vec![],
            &table(),
        );
        let cells = &update.rows()[0].cells;
        assert_eq!(cells[0].cluster(), "e\u{301}");
        assert_eq!(cells[1].ch, '\u{65E5}');
        assert!(cells[2].is_tail(), "the wide glyph keeps its second column");
        assert_eq!(cells[3].ch, 'f', "and f stays in column 3");
    }

    #[test]
    fn a_patch_before_any_reset_is_dropped_rather_than_guessed_at() {
        let mut shadow = Shadow::default();
        assert!(
            shadow
                .patch(&[], Cursor::default(), vec![], &StyleTable::default())
                .is_none()
        );
    }

    fn history_doc(from_top: u32, rows: &[(u32, &str)], capped: bool) -> ScrollbackDoc {
        ScrollbackDoc {
            from_top,
            rows: rows
                .iter()
                .map(|(row, text)| RowDiff {
                    row: *row,
                    cells: text
                        .chars()
                        .map(|ch| Cell {
                            ch,
                            ..Cell::default()
                        })
                        .collect(),
                })
                .collect(),
            total_rows: rows.len() as u32,
            complete: true,
            capped,
            era: 0,
        }
    }

    #[test]
    fn history_stitches_deltas_by_absolute_index() {
        let mut history = History::default();
        history.absorb(&history_doc(0, &[(0, "one"), (1, "two")], false));
        history.absorb(&history_doc(2, &[(2, "three")], false));
        let doc = history.doc();
        assert_eq!(doc.from_top, 0);
        assert_eq!(doc.total_rows, 3);
        assert_eq!(history.end(), 3);
        let since = history.since(2).unwrap();
        assert_eq!(since.from_top, 2);
        assert_eq!(since.rows.len(), 1);
        assert!(history.since(3).is_none(), "nothing new to send");
    }

    /// The refill a harness's exit produces lands **exactly** on what the hub already holds
    /// (probe #498) — the peer advanced its base past every row it dropped — so the indices say
    /// "tail" and the rows are the ones the hub is already holding. Only the era says otherwise,
    /// and the hub has to relay the whole document rather than a delta past its old end, since a
    /// delta cannot say "and throw the rest away".
    #[test]
    fn a_new_era_replaces_what_the_hub_held_even_where_the_indices_look_adjacent() {
        let mut history = History::default();
        history.absorb(&history_doc(0, &[(0, "shell-1"), (1, "shell-2")], false));
        let mut refill = history_doc(2, &[(2, "shell-1"), (3, "shell-2")], true);
        refill.era = 2;

        assert!(history.absorb(&refill), "a new era replaces what was held");
        let doc = history.doc();
        assert_eq!(doc.from_top, 2, "the era before it is gone, not underneath it");
        assert_eq!(doc.total_rows, 2, "the hub holds one ring, not two");
        assert_eq!(
            doc.era, 2,
            "and it carries the peer's own era on to its own watchers"
        );
    }

    #[test]
    fn a_tail_in_the_same_era_is_still_stitched_on() {
        let mut history = History::default();
        assert!(!history.absorb(&history_doc(0, &[(0, "one")], false)));
        assert!(!history.absorb(&history_doc(1, &[(1, "two")], false)));
        assert_eq!(history.doc().total_rows, 2);
    }

    #[test]
    fn a_ring_that_restarted_replaces_what_the_hub_held_and_says_it_was_capped() {
        let mut history = History::default();
        history.absorb(&history_doc(0, &[(0, "one")], false));
        history.absorb(&history_doc(900, &[(900, "later")], true));
        let doc = history.doc();
        assert_eq!(doc.from_top, 900, "the hub never invents the rows in between");
        assert_eq!(doc.total_rows, 1);
        assert!(doc.capped);
    }
}
