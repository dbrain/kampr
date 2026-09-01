//! What a far end may make this process allocate by claiming a number.
//!
//! `grid.reset` carries the geometry as two `u16`s and the shadow used to believe them: a ~100
//! byte frame claiming 65535x65535 asks for **159 GiB** in one `vec!`, which is an OOM rather
//! than an error. The message-size ceilings do not bound it, because the allocation is derived
//! from the claim rather than from the bytes.
//!
//! Two far ends can make that claim, and today the second one is new: an enrolled peer sending a
//! hub a `grid.reset`, and — since `kampr-client` — any node sending the operator's own terminal
//! one.

use kampr_core::wire::{Cursor, Style, Styles};
use kampr_mesh::shadow::{MAX_COLS, MAX_GRID_CELLS, MAX_LINKS, MAX_STYLES, Shadow, StyleTable};

#[test]
fn a_geometry_a_far_end_merely_claimed_does_not_become_an_allocation() {
    let mut shadow = Shadow::default();
    shadow.reset(
        u16::MAX,
        u16::MAX,
        &[],
        Cursor::default(),
        Vec::new(),
        &StyleTable::default(),
    );
    let (cols, rows) = shadow.geometry();
    assert!(cols <= MAX_COLS, "cols clamped, got {cols}");
    let cells = cols as usize * rows as usize;
    assert!(
        cells <= MAX_GRID_CELLS,
        "a claimed 65535x65535 is {cells} cells, over the {MAX_GRID_CELLS} budget"
    );
    assert_eq!(
        shadow.rows().len(),
        rows as usize,
        "the grid matches the geometry it reports"
    );
}

#[test]
fn an_ordinary_pane_is_not_clamped() {
    let mut shadow = Shadow::default();
    shadow.reset(
        292,
        72,
        &[],
        Cursor::default(),
        Vec::new(),
        &StyleTable::default(),
    );
    assert_eq!(
        shadow.geometry(),
        (292, 72),
        "the widest pane ever measured here is nowhere near the budget"
    );
}

/// A pane's link table is *appended to* across messages, and the shadow believed every append. A
/// peer sending `grid.patch` with a fresh handful of hyperlinks, forever, grows one table on the
/// hub per pane it holds — no single message is large, and nothing ever evicts.
#[test]
fn a_link_table_a_peer_grows_across_messages_stops_at_a_ceiling() {
    let mut shadow = Shadow::default();
    let styles = StyleTable::default();
    shadow.reset(80, 24, &[], Cursor::default(), Vec::new(), &styles);
    for batch in 0..64 {
        let links: Vec<String> = (0..1024)
            .map(|n| format!("https://kampr.dev/{batch}/{n}"))
            .collect();
        shadow.patch(&[], Cursor::default(), links, &styles);
    }
    assert!(
        shadow.links().len() <= MAX_LINKS,
        "a peer grew the hub's link table to {}",
        shadow.links().len(),
    );
}

/// The same shape one field over. `absorb` refuses a batch that does not continue the table,
/// because `from` is an allocation the far end asks for — and then appends whatever follows it,
/// message after message, with nothing counting the total.
#[test]
fn a_style_table_a_peer_appends_to_forever_stops_at_a_ceiling() {
    let mut styles = StyleTable::default();
    let mut refused = false;
    for batch in 0..64u32 {
        let from = 1 + batch * 1024;
        refused |= !styles.absorb(&Styles {
            from,
            styles: vec![Style::default(); 1024],
        });
    }
    assert!(refused, "a peer appended past the ceiling and was told nothing");
    assert!(
        styles.len() <= MAX_STYLES,
        "a peer grew the hub's style table to {}",
        styles.len(),
    );
}
