use alacritty_terminal::grid::Row;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::cell::{Cell as ACell, Flags};
use alacritty_terminal::vte::ansi::Color as AColor;
use kampr_term::{Cell, CellAttrs, Color};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write as _;
use std::sync::Arc;
use unicode_segmentation::GraphemeCursor;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// What `kampr-term` will hold once it has been sent this row: alacritty's cells, except where the
/// two disagree about a glyph's width or about where a grapheme ends. Those cells become U+FFFD,
/// because the alternative is a consumer whose columns drift from that cell to the row's end.
pub(crate) fn row_cells(row: &Row<ACell>, cols: u16, links: &mut Links) -> Vec<Cell> {
    let mut out: Vec<Cell> = Vec::with_capacity(cols as usize);
    let mut col = 0;
    while col < cols {
        let c = &row[Column(col as usize)];
        let flags = c.flags;
        let mut cell = Cell {
            ch: ' ',
            fg: colour(c.fg),
            bg: colour(c.bg),
            attrs: CellAttrs {
                bold: flags.contains(Flags::BOLD),
                dim: flags.contains(Flags::DIM),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                reverse: flags.contains(Flags::INVERSE),
                strike: flags.contains(Flags::STRIKEOUT),
                hidden: flags.contains(Flags::HIDDEN),
                ..CellAttrs::default()
            },
            link: c.hyperlink().map(|h| links.id(h.uri())),
            marks: None,
        };
        let wide = flags.contains(Flags::WIDE_CHAR)
            && col + 1 < cols
            && row[Column(col as usize + 1)]
                .flags
                .contains(Flags::WIDE_CHAR_SPACER);
        // alacritty keeps a `\t` in the cell a tab started from, for copying.
        let blank = c.c == ' ' || c.c == '\t';
        // A wide glyph whose second column was erased or overwritten is gone in xterm and in
        // `kampr-term` alike; alacritty keeps drawing it over the column it lost.
        let orphan = flags.contains(Flags::WIDE_CHAR) && !wide;
        if orphan
            || flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            || (blank && c.zerowidth().is_none())
        {
            out.push(cell);
            col += 1;
            continue;
        }
        let mut cluster = String::with_capacity(4);
        cluster.push(if blank { ' ' } else { c.c });
        cluster.extend(c.zerowidth().unwrap_or_default());
        let mut chars = cluster.chars();
        cell.ch = chars.next().unwrap_or(' ');
        let rest = chars.as_str();
        if !rest.is_empty() {
            cell.marks = Some(Arc::new(rest.to_string()));
        }
        let mut width = if wide { 2 } else { 1 };
        let ascii = cluster.len() == 1 && out.last().is_none_or(|c| c.ch.is_ascii() && c.marks.is_none());
        let term = if ascii { width } else { term_width(&cluster) };
        // alacritty spends one column on a text character that a variation selector turns into a
        // two-column emoji (`❤️`), where `kampr-term`, herdr and the program spend two. A plain
        // blank after it is what the second column would have shown anyway.
        if term == 2 && width == 1 && col + 1 < cols && spare(&row[Column(col as usize + 1)], c) {
            width = 2;
        }
        if !ascii && (term != width || joins(&out, &cluster)) {
            cell.ch = '\u{fffd}';
            cell.marks = None;
            out.push(cell.clone());
            if width == 2 {
                cell.ch = ' ';
                out.push(cell);
            }
        } else if width == 2 {
            let tail = cell.tail();
            out.push(cell);
            out.push(tail);
        } else {
            out.push(cell);
        }
        col += width;
    }
    out
}

fn spare(next: &ACell, lead: &ACell) -> bool {
    next.c == ' '
        && next.zerowidth().is_none()
        && next.hyperlink().is_none()
        && next.bg == lead.bg
        && next.flags.difference(Flags::WRAPLINE).is_empty()
        && !lead.flags.contains(Flags::INVERSE)
}

pub(crate) fn wrapped(row: &Row<ACell>) -> bool {
    row.last().is_some_and(|c| c.flags.contains(Flags::WRAPLINE))
}

/// A link's id within one grid is a hash of its URI, so equal links compare equal across frames
/// without a table that only ever grows.
#[derive(Default)]
pub(crate) struct Links(HashMap<u32, Arc<str>>);

impl Links {
    fn id(&mut self, uri: &str) -> u32 {
        let mut hasher = DefaultHasher::new();
        uri.hash(&mut hasher);
        let id = hasher.finish() as u32;
        self.0.entry(id).or_insert_with(|| uri.into());
        id
    }

    fn uri(&self, id: u32) -> Option<&str> {
        self.0.get(&id).map(|u| &**u)
    }
}

/// `kampr-term`'s own width for a cluster (#226, #227): a lone regional indicator spends two.
pub(crate) fn term_width(cluster: &str) -> u16 {
    if cluster.starts_with(|c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)) {
        return 2;
    }
    cluster.width().min(2) as u16
}

/// Whether `kampr-term` would fold this cluster into the one before it (#225), which alacritty
/// never does for a code point that has a width of its own.
fn joins(before: &[Cell], cluster: &str) -> bool {
    let Some(lead) = before.iter().rev().find(|c| !c.is_tail()) else {
        return false;
    };
    let mut joined = lead.cluster();
    let at = joined.len();
    joined.push_str(cluster);
    !GraphemeCursor::new(at, joined.len(), true)
        .is_boundary(&joined, 0)
        .unwrap_or(true)
}

fn colour(c: AColor) -> Color {
    match c {
        AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(i) => Color::Indexed(i),
        AColor::Named(n) if (n as usize) < 16 => Color::Indexed(n as u8),
        AColor::Named(_) => Color::Default,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Pen {
    fg: Color,
    bg: Color,
    attrs: CellAttrs,
}

impl Pen {
    fn of(cell: &Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            attrs: cell.attrs,
        }
    }

    fn plain() -> Self {
        Self::of(&Cell::default())
    }
}

/// Every escape it writes is one `kampr-term` parses: absolute addressing, SGR, `K`, `J` and OSC 8.
/// A glyph is always placed by an address rather than by a wrap, so nothing depends on autowrap.
/// Every frame ends outside any link, so the next one starts from a consumer with none open.
struct Writer<'a> {
    out: Vec<u8>,
    pen: Option<Pen>,
    at: Option<(u16, u16)>,
    link: Option<u32>,
    links: &'a Links,
}

impl<'a> Writer<'a> {
    fn new(links: &'a Links) -> Self {
        Self {
            out: Vec::new(),
            pen: None,
            at: None,
            link: None,
            links,
        }
    }

    fn link(&mut self, link: Option<u32>) {
        if self.link == link {
            return;
        }
        let uri = link.and_then(|id| self.links.uri(id)).unwrap_or_default();
        let _ = write!(self.out, "\x1b]8;;{uri}\x1b\\");
        self.link = link;
    }

    fn goto(&mut self, row: u16, col: u16) {
        if self.at != Some((row, col)) {
            let _ = write!(self.out, "\x1b[{};{}H", row + 1, col + 1);
            self.at = Some((row, col));
        }
    }

    fn pen(&mut self, pen: Pen) {
        if self.pen == Some(pen) {
            return;
        }
        self.out.extend_from_slice(b"\x1b[0");
        let a = pen.attrs;
        for (on, code) in [
            (a.bold, "1"),
            (a.dim, "2"),
            (a.italic, "3"),
            (a.underline, "4"),
            (a.blink, "5"),
            (a.reverse, "7"),
            (a.hidden, "8"),
            (a.strike, "9"),
        ] {
            if on {
                self.out.push(b';');
                self.out.extend_from_slice(code.as_bytes());
            }
        }
        sgr_colour(&mut self.out, 38, pen.fg);
        sgr_colour(&mut self.out, 48, pen.bg);
        self.out.push(b'm');
        self.pen = Some(pen);
    }

    fn put(&mut self, row: u16, col: u16, cell: &Cell, width: u16) {
        self.goto(row, col);
        self.pen(Pen::of(cell));
        self.link(cell.link);
        let mut text = String::with_capacity(4);
        cell.push_cluster(&mut text);
        self.out.extend_from_slice(text.as_bytes());
        self.at = Some((row, col + width));
    }

    fn erase_from(&mut self, row: u16, col: u16) {
        self.goto(row, col);
        self.pen(Pen::plain());
        self.link(None);
        self.out.extend_from_slice(b"\x1b[K");
    }

    fn cursor(&mut self, cursor: Cursor, cols: u16) {
        self.link(None);
        self.at = None;
        self.goto(cursor.row, cursor.col.min(cols.saturating_sub(1)));
        if cursor.visible {
            self.out.extend_from_slice(b"\x1b[?25h");
        }
    }
}

fn sgr_colour(out: &mut Vec<u8>, base: u8, c: Color) {
    let _ = match c {
        Color::Default => Ok(()),
        Color::Indexed(i) => write!(out, ";{base};5;{i}"),
        Color::Rgb(r, g, b) => write!(out, ";{base};2;{r};{g};{b}"),
    };
}

fn width_at(row: &[Cell], col: usize) -> u16 {
    if row.get(col + 1).is_some_and(Cell::is_tail) {
        2
    } else {
        1
    }
}

fn content_end(row: &[Cell]) -> usize {
    let blank = Cell::default();
    row.iter().rposition(|c| *c != blank).map_or(0, |i| i + 1)
}

/// How many unchanged columns are rewritten rather than skipped with an address, which costs more.
const SKIP: usize = 4;

pub(crate) fn full(grid: &[Cell], links: &Links, cols: u16, cursor: Cursor) -> Vec<u8> {
    let mut w = Writer::new(links);
    w.out.extend_from_slice(b"\x1b[?25l\x1b[0m\x1b[H\x1b[2J");
    w.pen = Some(Pen::plain());
    let blank = Cell::default();
    for (r, row) in grid.chunks(cols.max(1) as usize).enumerate() {
        let r = r as u16;
        let end = content_end(row);
        let mut c = 0;
        while c < end {
            if row[c] == blank {
                let run = row[c..end].iter().take_while(|x| **x == blank).count();
                if run >= SKIP || w.at != Some((r, c as u16)) {
                    c += run;
                    continue;
                }
            }
            let width = width_at(row, c);
            w.put(r, c as u16, &row[c], width);
            c += width as usize;
        }
    }
    w.cursor(cursor, cols);
    w.out
}

/// Diffs against what was last sent rather than against a consumer's real grid, so it is only
/// sound when left-to-right order holds: overwriting half a wide glyph blanks its other half in
/// `kampr-term`, and that half always differs too, so it is written after.
pub(crate) fn diff(
    old: &[Cell],
    new: &[Cell],
    links: &Links,
    cols: u16,
    was: Cursor,
    now: Cursor,
) -> Option<Vec<u8>> {
    if old == new && was == now {
        return None;
    }
    let mut w = Writer::new(links);
    w.out.extend_from_slice(b"\x1b[?25l");
    let n = cols.max(1) as usize;
    for (r, (old, new)) in old.chunks(n).zip(new.chunks(n)).enumerate() {
        if old == new {
            continue;
        }
        let r = r as u16;
        let end = content_end(new);
        let mut c = 0;
        while c < end {
            let width = width_at(new, c);
            let changed = new[c..c + width as usize] != old[c..c + width as usize];
            let near =
                w.at.is_some_and(|(row, col)| row == r && (col as usize) < c && c - col as usize <= SKIP);
            if changed && near {
                let (_, mut from) = w.at.unwrap_or_default();
                while (from as usize) < c {
                    let wf = width_at(new, from as usize);
                    w.put(r, from, &new[from as usize], wf);
                    from += wf;
                }
            }
            if changed {
                w.put(r, c as u16, &new[c], width);
            }
            c += width as usize;
        }
        if old[end..] != new[end..] {
            w.erase_from(r, end as u16);
        }
    }
    w.cursor(now, cols);
    Some(w.out)
}

/// One history row as `RawScrollback.text` carries it: SGR and glyphs only, starting and ending on
/// the default pen, because the ring lays rows out one after another on a single emulator.
pub(crate) fn styled_row(row: &[Cell], links: &Links) -> String {
    let mut w = Writer::new(links);
    w.pen = Some(Pen::plain());
    let end = content_end(row);
    let mut c = 0;
    while c < end {
        let width = width_at(row, c);
        w.pen(Pen::of(&row[c]));
        w.link(row[c].link);
        let mut text = String::new();
        row[c].push_cluster(&mut text);
        w.out.extend_from_slice(text.as_bytes());
        c += width as usize;
    }
    w.pen(Pen::plain());
    w.link(None);
    String::from_utf8(w.out).unwrap_or_default()
}

pub(crate) fn plain_row(row: &[Cell]) -> String {
    let mut text = String::with_capacity(row.len());
    for cell in row.iter().filter(|c| !c.is_tail()) {
        cell.push_cluster(&mut text);
    }
    text.trim_end().to_string()
}
