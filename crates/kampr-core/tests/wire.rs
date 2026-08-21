use kampr_core::provider::{AgentStatus, PaneInfo};
use kampr_core::wire::{Cursor, Encoder, HerdDelta, PaneEntry, ServerMsg};
use kampr_term::{Cell, CellAttrs, Color, RowDiff};

fn cell(ch: char, fg: Color) -> Cell {
    Cell {
        ch,
        fg,
        ..Cell::default()
    }
}

fn row(n: u32, cells: Vec<Cell>) -> RowDiff {
    RowDiff { row: n, cells }
}

#[test]
fn identical_neighbours_collapse_into_one_run() {
    let mut enc = Encoder::new();
    let cells: Vec<Cell> = "hello".chars().map(|c| cell(c, Color::Default)).collect();
    let rows = enc.rows(&[row(0, cells)]);
    assert_eq!(rows[0].runs.len(), 1);
    assert_eq!(rows[0].runs[0].x, "hello");
    assert_eq!(rows[0].runs[0].s, 0, "the default pen is always style 0");
}

#[test]
fn a_style_change_starts_a_new_run() {
    let mut enc = Encoder::new();
    let mut cells: Vec<Cell> = "ab".chars().map(|c| cell(c, Color::Default)).collect();
    cells.push(cell('c', Color::Indexed(196)));
    cells.push(cell('d', Color::Indexed(196)));
    let rows = enc.rows(&[row(3, cells)]);
    assert_eq!(rows[0].row, 3);
    let runs = &rows[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].s, runs[0].x.as_str()), (0, "ab"));
    assert_eq!((runs[1].s, runs[1].x.as_str()), (1, "cd"));
}

#[test]
fn styles_are_interned_once_per_connection_and_only_new_ones_are_sent() {
    let mut enc = Encoder::new();
    let red = cell('x', Color::Indexed(1));
    enc.rows(&[row(0, vec![red])]);
    let first = enc.take_styles().expect("style 1 is new");
    assert_eq!(first.from, 1);
    assert_eq!(first.styles.len(), 1);
    assert_eq!(first.styles[0].fg, Color::Indexed(1));

    enc.rows(&[row(1, vec![red])]);
    assert!(
        enc.take_styles().is_none(),
        "an already-sent style is never re-sent"
    );

    enc.rows(&[row(2, vec![cell('y', Color::Indexed(2))])]);
    let second = enc.take_styles().expect("style 2 is new");
    assert_eq!(second.from, 2);
    assert_eq!(second.styles.len(), 1);
}

#[test]
fn trailing_default_cells_are_dropped() {
    let mut enc = Encoder::new();
    let mut cells: Vec<Cell> = "hi".chars().map(|c| cell(c, Color::Default)).collect();
    cells.extend(std::iter::repeat_n(Cell::default(), 70));
    let rows = enc.rows(&[row(0, cells)]);
    assert_eq!(rows[0].runs.len(), 1);
    assert_eq!(rows[0].runs[0].x, "hi");
}

#[test]
fn a_trailing_run_with_a_background_survives_the_trim() {
    let mut enc = Encoder::new();
    let mut cells = vec![cell('a', Color::Default)];
    cells.extend(std::iter::repeat_n(
        Cell {
            ch: ' ',
            bg: Color::Indexed(4),
            ..Cell::default()
        },
        3,
    ));
    let rows = enc.rows(&[row(0, cells)]);
    assert_eq!(rows[0].runs.len(), 2, "a coloured background is not blank");
    assert_eq!(rows[0].runs[1].x, "   ");
}

#[test]
fn hyperlink_ids_split_runs_and_ride_along() {
    let mut enc = Encoder::new();
    let mut cells: Vec<Cell> = "go".chars().map(|c| cell(c, Color::Default)).collect();
    cells[0].link = Some(0);
    cells[1].link = Some(0);
    cells.push(Cell {
        ch: '!',
        ..Cell::default()
    });
    let rows = enc.rows(&[row(0, cells)]);
    assert_eq!(rows[0].runs.len(), 2);
    assert_eq!(rows[0].runs[0].l, Some(0));
    assert_eq!(rows[0].runs[1].l, None);
}

#[test]
fn attributes_are_part_of_the_style_key() {
    let mut enc = Encoder::new();
    let plain = Cell::default();
    let bold = Cell {
        attrs: CellAttrs {
            bold: true,
            ..CellAttrs::default()
        },
        ..Cell::default()
    };
    enc.rows(&[row(0, vec![plain, bold])]);
    let s = enc.take_styles().unwrap();
    assert_eq!(s.styles.len(), 1);
    assert!(s.styles[0].attrs.bold);
}

#[test]
fn grid_patch_serialises_to_the_documented_shape() {
    let mut enc = Encoder::new();
    let cells: Vec<Cell> = "ok".chars().map(|c| cell(c, Color::Default)).collect();
    let rows = enc.rows(&[row(9, cells)]);
    let msg = ServerMsg::GridPatch {
        pane: "01J/w3:p2".into(),
        rows,
        cursor: Cursor {
            col: 5,
            row: 10,
            visible: true,
        },
        links: Vec::new(),
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["t"], "grid.patch");
    assert_eq!(v["pane"], "01J/w3:p2");
    assert_eq!(v["rows"][0]["row"], 9);
    assert_eq!(v["rows"][0]["runs"][0]["s"], 0);
    assert_eq!(v["rows"][0]["runs"][0]["x"], "ok");
    assert!(
        v["rows"][0]["runs"][0].get("l").is_none(),
        "absent link id is omitted"
    );
    assert_eq!(v["cursor"]["col"], 5);
    assert!(v.get("links").is_none(), "an unchanged link table is omitted");
}

#[test]
fn styles_message_matches_the_documented_shape() {
    let mut enc = Encoder::new();
    enc.rows(&[row(
        0,
        vec![Cell {
            ch: 'x',
            fg: Color::Rgb(255, 120, 0),
            attrs: CellAttrs {
                bold: true,
                underline: true,
                ..CellAttrs::default()
            },
            ..Cell::default()
        }],
    )]);
    let v = serde_json::to_value(ServerMsg::Styles(enc.take_styles().unwrap())).unwrap();
    assert_eq!(v["t"], "styles");
    assert_eq!(v["from"], 1);
    assert_eq!(v["styles"][0]["fg"]["k"], "r");
    assert_eq!(v["styles"][0]["fg"]["v"], serde_json::json!([255, 120, 0]));
    assert_eq!(v["styles"][0]["bg"]["k"], "d");
    assert_eq!(v["styles"][0]["bold"], true);
    assert!(
        v["styles"][0].get("italic").is_none(),
        "false attributes are omitted"
    );
}

#[test]
fn run_length_encoding_is_far_smaller_than_per_cell_json() {
    let mut enc = Encoder::new();
    let diffs: Vec<RowDiff> = (0..30)
        .map(|r| {
            let cells: Vec<Cell> = (0..74)
                .map(|c| cell(if c % 9 == 0 { 'x' } else { 'a' }, Color::Indexed((c / 9) as u8)))
                .collect();
            row(r, cells)
        })
        .collect();
    let per_cell = serde_json::to_string(&diffs).unwrap().len();
    let rows = enc.rows(&diffs);
    let runs = serde_json::to_string(&rows).unwrap().len();
    assert!(runs * 10 < per_cell, "run encoding {runs} vs per-cell {per_cell}");
}

#[test]
fn an_absolute_ring_index_beyond_sixteen_bits_survives_the_wire() {
    let mut enc = Encoder::new();
    let rows = enc.rows(&[RowDiff {
        row: 200_000,
        cells: vec![cell('x', Color::Default)],
    }]);
    let v = serde_json::to_value(ServerMsg::Scrollback {
        pane: "01J/w3:p2".into(),
        from_top: 200_000,
        rows,
        total_rows: 200_001,
        complete: false,
        capped: true,
    })
    .unwrap();
    assert_eq!(v["rows"][0]["row"], 200_000);
    assert_eq!(v["capped"], true);
    assert_eq!(v["complete"], false);
}

#[test]
fn a_herd_patch_carries_the_same_shape_as_herd() {
    let pane = PaneEntry::new(
        "01J",
        &PaneInfo {
            pane_id: "w3:p2".into(),
            cols: Some(74),
            rows: 30,
            agent_status: AgentStatus::Blocked,
            ..PaneInfo::default()
        },
        false,
    );
    let v = serde_json::to_value(ServerMsg::HerdPatch {
        added: HerdDelta::default(),
        changed: HerdDelta::panes(vec![pane]),
        removed_ids: vec!["01J/w3:p9".into()],
    })
    .unwrap();
    assert_eq!(v["t"], "herd.patch");
    assert!(
        v.get("added").is_none(),
        "an empty delta is omitted, not sent as []"
    );
    assert_eq!(v["changed"]["panes"][0]["id"], "01J/w3:p2");
    assert_eq!(v["changed"]["panes"][0]["agent_status"], "blocked");
    assert_eq!(v["removed_ids"][0], "01J/w3:p9");
}

/// Two people can type into one pane with no sign of each other. `watchers` is how a client says
/// so — and it is absent for nobody and for one, so the common case costs nothing on the wire and
/// a client can read "absent" as "just me".
#[test]
fn a_shared_pane_carries_a_watcher_count_and_a_solitary_one_does_not() {
    let info = PaneInfo {
        pane_id: "w3:p2".into(),
        rows: 30,
        ..PaneInfo::default()
    };
    let entry =
        |watchers| serde_json::to_value(PaneEntry::new("01J", &info, false).with_watchers(watchers)).unwrap();
    assert!(entry(0).get("watchers").is_none(), "{}", entry(0));
    assert!(entry(1).get("watchers").is_none(), "{}", entry(1));
    assert_eq!(entry(2)["watchers"], 2);
    assert_eq!(entry(7)["watchers"], 7);
}
