//! The emulator applies every frame herdr sends for every watched pane, so its steady state is
//! held to allocating nothing per cell. A refactor that puts a `String` or a `Vec` on that path
//! fails here rather than in a profile months later.

use kampr_term::Emulator;
use kampr_testkit::alloc::{Counting, count};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const COLS: u16 = 150;
const ROWS: u16 = 40;

/// What herdr's serialiser sends (ARCHITECTURE.md §4.3): absolute addressing, SGR, text.
fn frame(tick: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for row in 1..=ROWS {
        out.extend_from_slice(format!("\x1b[{row};1H\x1b[0;1;38;5;{}m", 16 + tick).as_bytes());
        let glyph = b'a' + (tick + row as u8) % 26;
        out.extend(std::iter::repeat_n(glyph, COLS as usize));
    }
    out.extend_from_slice(b"\x1b[0m\x1b[1;1H");
    out
}

fn warm() -> Emulator {
    let mut emulator = Emulator::new(COLS, ROWS);
    emulator.feed(&frame(0));
    emulator.take_dirty();
    emulator
}

#[test]
fn applying_a_frame_allocates_nothing() {
    let mut emulator = warm();
    let frames: Vec<Vec<u8>> = (1..=3).map(frame).collect();
    for bytes in &frames {
        assert_eq!(count(|| emulator.feed(bytes)), 0, "a full {COLS}x{ROWS} repaint");
    }
}

/// `take_dirty` hands each changed row out as an owned copy, so it allocates once per dirty row
/// plus the list's own doublings — per row, never per cell (measured: 45 for 40 rows). The bound
/// is the current design, not a target: a row-recycling `take_dirty` would bring it to zero.
#[test]
fn collecting_a_frames_dirty_rows_allocates_per_row_not_per_cell() {
    let mut emulator = warm();
    emulator.feed(&frame(1));
    let mut rows = 0;
    let allocations = count(|| rows = emulator.take_dirty().len());
    assert_eq!(rows, ROWS as usize);
    let doublings = rows.next_power_of_two().trailing_zeros() as usize;
    assert!(
        allocations <= rows + doublings,
        "{allocations} allocations for {rows} dirty rows"
    );
}
