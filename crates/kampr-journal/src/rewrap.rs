//! A composer's rows read back into the text that wraps into them.
//!
//! Claude's box is `wrap-ansi` with `hard` on and `trim` off, `columns` four short of the pane
//! (#662): the space a row broke at stays on the end of that row, or opens the next one when the
//! row was already full; a word longer than a row is cut at the last column, and starts on a row
//! of its own only when that saves a cut. So how a row ends depends on how the one before it was
//! filled and on how long a word still to come turns out to be, and no rule about one break at a
//! time reads it. A reading is tried instead: the rows joined one way, wrapped again, and kept
//! only when that draws the rows on the screen.

/// Wraps tried before a box is left to the guess from its widths.
const BUDGET: usize = 256;

struct Row {
    text: Vec<char>,
    /// Opens a line the operator started.
    first: bool,
    /// Opens with the space the row above had no room for, which the box does not draw.
    lead: bool,
}

impl Row {
    fn drawn(&self) -> &[char] {
        let from = usize::from(self.lead);
        let to = self
            .text
            .iter()
            .rposition(|c| *c != ' ')
            .map_or(from, |at| at + 1);
        &self.text[from.min(to)..to]
    }
}

fn wrap(text: &[char], columns: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let open = |rows: &mut Vec<Row>, first: bool, lead: bool| {
        rows.push(Row {
            text: Vec::new(),
            first,
            lead,
        })
    };
    for line in text.split(|c| *c == '\n') {
        open(&mut rows, true, false);
        for (index, word) in line.split(|c| *c == ' ').enumerate() {
            let mut filled = rows.last().map_or(0, |row| row.text.len());
            if index != 0 {
                if filled >= columns {
                    open(&mut rows, false, true);
                    filled = 0;
                }
                rows.last_mut().expect("a row").text.push(' ');
                filled += 1;
            }
            if word.len() > columns {
                let here = 1 + (word.len() - (columns - filled) - 1) / columns;
                let below = (word.len() - 1) / columns;
                if below < here {
                    open(&mut rows, false, false);
                }
                for c in word {
                    if rows.last().map_or(0, |row| row.text.len()) >= columns {
                        open(&mut rows, false, false);
                    }
                    rows.last_mut().expect("a row").text.push(*c);
                }
                continue;
            }
            if filled + word.len() > columns && filled > 0 && !word.is_empty() {
                open(&mut rows, false, false);
            }
            rows.last_mut().expect("a row").text.extend(word);
        }
    }
    rows
}

fn trimmed(row: &[char]) -> &[char] {
    let to = row.iter().rposition(|c| *c != ' ').map_or(0, |at| at + 1);
    &row[..to]
}

struct Search<'a> {
    drawn: &'a [Vec<char>],
    columns: usize,
    spent: usize,
}

impl Search<'_> {
    /// The rows a longer word cannot move are the ones before the row above the last word's
    /// start: a word that grows past a row's width may yet be started a row earlier or later.
    fn settled(&self, text: &[char], rows: &[Row]) -> bool {
        let word = text
            .iter()
            .rposition(|c| *c == ' ' || *c == '\n')
            .map_or(0, |at| at + 1);
        let mut before = 0;
        let mut holds = rows.len();
        for (index, row) in rows.iter().enumerate() {
            before += row.text.len() + usize::from(index > 0 && row.first);
            if before > word {
                holds = index;
                break;
            }
        }
        (0..holds.saturating_sub(1)).all(|at| {
            self.drawn
                .get(at)
                .is_some_and(|drawn| trimmed(drawn) == rows[at].drawn())
        })
    }

    fn whole(&self, rows: &[Row]) -> bool {
        rows.len() == self.drawn.len()
            && rows
                .iter()
                .zip(self.drawn)
                .all(|(row, drawn)| row.drawn() == trimmed(drawn))
    }

    fn join(&mut self, text: &mut Vec<char>, next: usize) -> Option<Vec<Row>> {
        if self.spent >= BUDGET {
            return None;
        }
        self.spent += 1;
        let rows = wrap(text, self.columns);
        if next == self.drawn.len() {
            return self.whole(&rows).then_some(rows);
        }
        if !self.settled(text, &rows) {
            return None;
        }
        let above = trimmed(&self.drawn[next - 1]);
        let below = &self.drawn[next];
        let tail = above.iter().rev().take_while(|c| **c != ' ').count();
        let head = below.iter().take_while(|c| **c != ' ').count();
        // A filled row whose last word and the next row's first make one too long for any row
        // is a cut word far more often than two that happened to end at the edge.
        let cut = above.len() + 1 >= self.columns && tail + head > self.columns;
        let order: [&[char]; 3] = match cut {
            true => [&[], &[' '], &['\n']],
            false => [&[' '], &[], &['\n']],
        };
        let length = text.len();
        for between in order {
            text.extend(between);
            text.extend(below);
            if let Some(rows) = self.join(text, next + 1) {
                return Some(rows);
            }
            text.truncate(length);
        }
        None
    }
}

/// The text that wraps into `drawn` at `columns`, and where in it the caret at `caret` (a row of
/// `drawn` and a column of that row) sits, in characters. `None` when no joining of the rows wraps
/// back into them, which is a box this model does not describe.
pub(crate) fn unwrap(drawn: &[String], columns: usize, caret: (usize, usize)) -> Option<(String, usize)> {
    if columns == 0 || drawn.is_empty() {
        return None;
    }
    let drawn: Vec<Vec<char>> = drawn.iter().map(|row| row.chars().collect()).collect();
    let mut text = drawn[0].clone();
    let mut search = Search {
        drawn: &drawn,
        columns,
        spent: 0,
    };
    let rows = search.join(&mut text, 1)?;
    let (at, col) = caret;
    let row = rows.get(at)?;
    let before: usize = rows[..at].iter().map(|row| row.text.len()).sum();
    let lines = rows[..=at].iter().skip(1).filter(|row| row.first).count();
    let within = (col + usize::from(row.lead)).min(row.text.len());
    Some((text.into_iter().collect(), before + lines + within))
}
