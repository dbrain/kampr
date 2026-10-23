use kampr_core::provider::Hit;
use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

#[derive(Clone)]
pub(crate) struct Row {
    pub styled: String,
    pub plain: String,
    pub wrapped: bool,
    pub cols: u16,
}

/// A copy of the primary screen's scrollback, numbered from the first row that ever entered it.
/// alacritty only shows its history through the active grid, so while the alternate screen is up
/// the primary's is unreachable there, and it keeps only a short window of it besides (`screen.rs`).
pub(crate) struct History {
    rows: VecDeque<Row>,
    first: u64,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            rows: VecDeque::new(),
            first: 0,
            cap,
        }
    }

    pub fn resume(cap: usize, first: u64, rows: impl IntoIterator<Item = Row>) -> Self {
        let mut history = Self {
            rows: VecDeque::new(),
            first,
            cap,
        };
        for row in rows {
            history.push(row);
        }
        history
    }

    pub fn first(&self) -> u64 {
        self.first
    }

    pub fn end(&self) -> u64 {
        self.first + self.rows.len() as u64
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn row(&self, abs: u64) -> Option<&Row> {
        abs.checked_sub(self.first)
            .and_then(|i| self.rows.get(i as usize))
    }

    pub fn iter_from(&self, abs: u64) -> impl Iterator<Item = &Row> {
        self.rows.iter().skip(abs.saturating_sub(self.first) as usize)
    }

    pub fn push(&mut self, row: Row) {
        if self.cap == 0 {
            self.first += 1;
            return;
        }
        if self.rows.len() == self.cap {
            self.rows.pop_front();
            self.first += 1;
        }
        self.rows.push_back(row);
    }

    /// Rows a taller screen took back.
    pub fn drop_newest(&mut self, n: usize) {
        self.rows.truncate(self.rows.len().saturating_sub(n));
    }

    /// Erased rows keep their numbers spent, so a number is never handed out twice.
    pub fn clear(&mut self) {
        self.first = self.end();
        self.rows.clear();
    }
}

pub(crate) struct Line<'a> {
    pub plain: &'a str,
    pub wrapped: bool,
    pub cols: u16,
}

/// Matches on logical lines, so a word a soft wrap split is still found, and in smart case: a
/// query with no capital letter matches either case.
pub(crate) fn find(
    lines: &[Line<'_>],
    query: &str,
    backward: bool,
    from: Option<u32>,
) -> (Vec<Hit>, Option<u32>) {
    let fold = !query.chars().any(char::is_uppercase);
    let norm = |c: char| {
        if fold {
            c.to_lowercase().next().unwrap_or(c)
        } else {
            c
        }
    };
    let needle: Vec<char> = query.chars().map(norm).collect();
    let bottom = lines.len().saturating_sub(1);
    let mut hits = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let mut chars: Vec<(char, usize, u16)> = Vec::new();
        loop {
            let line = &lines[i];
            let mut col: u16 = 0;
            for c in line.plain.chars() {
                let w = c.width().unwrap_or(0).min(2) as u16;
                let at = if w == 0 { col.saturating_sub(1) } else { col };
                chars.push((norm(c), i, at));
                col = col.saturating_add(w);
            }
            if !line.wrapped || i + 1 >= lines.len() {
                break;
            }
            while col < line.cols {
                chars.push((' ', i, col));
                col += 1;
            }
            i += 1;
        }
        i += 1;
        if needle.is_empty() {
            continue;
        }
        let mut k = 0;
        while k + needle.len() <= chars.len() {
            if chars[k..k + needle.len()]
                .iter()
                .map(|c| c.0)
                .eq(needle.iter().copied())
            {
                let (_, row, col) = chars[k];
                let (_, end_row, end_col) = chars[k + needle.len() - 1];
                hits.push(Hit {
                    from_bottom: (bottom - row) as u32,
                    col,
                    end_from_bottom: (bottom - end_row) as u32,
                    end_col,
                    text: lines[row].plain.to_string(),
                });
                k += needle.len();
            } else {
                k += 1;
            }
        }
    }
    let start = from.unwrap_or(0);
    let current = if hits.is_empty() {
        None
    } else if backward {
        hits.iter()
            .rposition(|h| h.from_bottom > start)
            .or(Some(hits.len() - 1))
    } else {
        hits.iter().position(|h| h.from_bottom <= start).or(Some(0))
    };
    (hits, current.map(|c| c as u32))
}
