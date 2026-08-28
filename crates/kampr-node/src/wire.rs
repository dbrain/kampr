use crate::outbox::{Frame, Outbox};
use kampr_core::registry::PaneUpdate;
use kampr_core::scrollback::ScrollbackDoc;
use kampr_core::wire::{Encoder, ErrorCode, ServerMsg};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// One connection's view of the protocol: its own style table, its own queue.
///
/// The style table is per connection because ids are only promised to be stable for the life of
/// a connection — sharing one across sockets would mean either replaying the whole table to every
/// joiner or handing out ids a client was never told about.
pub struct Wire {
    outbox: Arc<Outbox>,
    encoder: Mutex<Encoder>,
}

impl Wire {
    pub fn new(outbox: Arc<Outbox>) -> Self {
        Self {
            outbox,
            encoder: Mutex::new(Encoder::new()),
        }
    }

    pub fn outbox(&self) -> &Arc<Outbox> {
        &self.outbox
    }

    pub fn send(&self, msg: &ServerMsg) -> bool {
        match serde_json::to_string(msg) {
            Ok(json) => self.outbox.push(Frame::plain(json)),
            Err(e) => {
                tracing::error!(error = %e, "a server message would not serialise");
                true
            }
        }
    }

    pub fn send_json(&self, value: &Value) -> bool {
        self.outbox.push(Frame::plain(value.to_string()))
    }

    /// Encodes and enqueues under one lock, so the `styles` message that introduces a pen always
    /// precedes the runs that reference it — on this connection and in this order.
    pub fn send_update(&self, pane: &str, update: &PaneUpdate) -> bool {
        let mut encoder = self.encoder.lock().unwrap();
        self.emit(pane, encoder.encode(pane, update))
    }

    pub fn send_scrollback(&self, pane: &str, doc: &ScrollbackDoc) -> bool {
        let mut encoder = self.encoder.lock().unwrap();
        self.emit(pane, encoder.encode_scrollback(pane, doc))
    }

    /// Only `grid.reset` and `grid.patch` are marked as the pane's, because those are the only
    /// frames a purge may drop: a reset makes every patch it replaced irrelevant.
    ///
    /// `styles` is excluded so a purge cannot drop the table entry a surviving run depends on,
    /// and `scrollback` because history is append-only and the pump's cursor only moves forward —
    /// a dropped one is a hole in the client's ring that the following reset does not repair,
    /// since a reset carries the viewport and nothing above it.
    fn emit(&self, pane: &str, messages: Vec<ServerMsg>) -> bool {
        for message in messages {
            let Ok(json) = serde_json::to_string(&message) else {
                continue;
            };
            let frame = match message {
                ServerMsg::GridReset { .. } | ServerMsg::GridPatch { .. } => Frame::grid(pane, json),
                _ => Frame::plain(json),
            };
            if !self.outbox.push(frame) {
                return false;
            }
        }
        true
    }

    pub fn error(&self, code: ErrorCode, message: &str, pane: Option<&str>) -> bool {
        self.send(&ServerMsg::Error {
            code,
            message: message.to_string(),
            pane: pane.map(str::to_string),
            node: None,
        })
    }

    /// A fault that belongs to a whole node rather than to one pane. See [`ServerMsg::Error`]'s
    /// `node` for why it has to be said and why the client is what decides how loudly.
    pub fn node_error(&self, code: ErrorCode, message: &str, node: &str) -> bool {
        self.send(&ServerMsg::Error {
            code,
            message: message.to_string(),
            pane: None,
            node: Some(node.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::scrollback::ScrollbackDoc;
    use kampr_core::wire::Cursor;
    use kampr_term::{Cell, RowDiff};

    fn reset() -> PaneUpdate {
        PaneUpdate::Reset {
            cols: 3,
            rows: 1,
            rows_data: Arc::new(vec![RowDiff {
                row: 0,
                cells: vec![Cell {
                    ch: 'h',
                    ..Cell::default()
                }],
            }]),
            cursor: Cursor {
                col: 1,
                row: 0,
                visible: true,
            },
            links: Arc::new(vec![]),
        }
    }

    /// A cursor that moved on its own — no cell changed, and the client's own caret is wrong
    /// until it is told. The node forwards it verbatim; anything here that treated an empty
    /// `rows` as nothing to say would silently undo that.
    #[tokio::test]
    async fn a_patch_that_only_moves_the_cursor_still_reaches_the_client() {
        let outbox = Arc::new(Outbox::new(16));
        let wire = Wire::new(outbox.clone());
        wire.send_update("n/w1:p1", &reset());
        drain(&outbox);

        wire.send_update(
            "n/w1:p1",
            &PaneUpdate::Patch {
                rows: Arc::new(Vec::new()),
                cursor: Cursor {
                    col: 2,
                    row: 0,
                    visible: true,
                },
                new_links: Arc::new(Vec::new()),
            },
        );
        let sent = drain(&outbox);
        assert_eq!(sent.len(), 1, "an empty patch is a message, not nothing");
        assert_eq!(sent[0]["t"], "grid.patch");
        assert_eq!(sent[0]["rows"], serde_json::json!([]));
        assert_eq!(sent[0]["cursor"]["col"], 2);
    }

    fn drain(outbox: &Outbox) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(frame) = futures_util::FutureExt::now_or_never(outbox.next())
            .flatten()
            .ok_or(())
        {
            out.push(serde_json::from_str(&frame.json).unwrap());
        }
        out
    }

    #[tokio::test]
    async fn a_grid_reset_is_purgeable_and_a_styles_message_is_not() {
        let outbox = Arc::new(Outbox::new(16));
        let wire = Wire::new(outbox.clone());
        wire.send_update("n/w1:p1", &reset());
        assert_eq!(
            outbox.purge_pane("n/w1:p1"),
            1,
            "only the grid frame is droppable"
        );
        assert_eq!(
            outbox.depth(),
            0,
            "no styles frame was emitted for the default pen"
        );
    }

    #[tokio::test]
    async fn styles_precede_the_runs_that_use_them() {
        let outbox = Arc::new(Outbox::new(16));
        let wire = Wire::new(outbox.clone());
        let mut coloured = reset();
        if let PaneUpdate::Reset { rows_data, .. } = &mut coloured {
            *rows_data = Arc::new(vec![RowDiff {
                row: 0,
                cells: vec![Cell {
                    ch: 'x',
                    fg: kampr_term::Color::Indexed(9),
                    ..Cell::default()
                }],
            }]);
        }
        wire.send_update("n/w1:p1", &coloured);
        let frames = drain(&outbox);
        assert_eq!(frames[0]["t"], "styles");
        assert_eq!(frames[1]["t"], "grid.reset");
        assert_eq!(frames[1]["rows_data"][0]["runs"][0]["s"], frames[0]["from"]);
    }

    /// Probe #210: a double-width glyph spends two columns, so the run that carries it says so.
    /// Without `w` the client can only know by recomputing character widths, and it has no
    /// Unicode width table to do it with.
    #[tokio::test]
    async fn a_double_width_run_declares_its_column_span() {
        let outbox = Arc::new(Outbox::new(16));
        let wire = Wire::new(outbox.clone());
        let mut term = kampr_term::Emulator::new(20, 1);
        term.feed("AB\u{65e5}\u{672c}\u{8a9e}CD".as_bytes());
        let mut wide = reset();
        if let PaneUpdate::Reset { rows_data, cols, .. } = &mut wide {
            *cols = 20;
            *rows_data = Arc::new(vec![RowDiff {
                row: 0,
                cells: term.grid().row(0).to_vec(),
            }]);
        }
        wire.send_update("n/w1:p1", &wide);
        let frames = drain(&outbox);
        let runs = &frames[0]["rows_data"][0]["runs"];
        assert_eq!(runs[0]["x"], "AB");
        assert_eq!(runs[0].get("w"), None, "a narrow run omits the field");
        assert_eq!(runs[1]["x"], "\u{65e5}\u{672c}\u{8a9e}");
        assert_eq!(runs[1]["w"], 2);
        assert_eq!(runs[2]["x"], "CD");
        let span: u64 = runs
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["x"].as_str().unwrap().chars().count() as u64 * r["w"].as_u64().unwrap_or(1))
            .sum();
        assert_eq!(span, 10, "the runs cover the columns the glyphs actually occupy");
    }

    /// Probe #223: the marks ride in `m`, positionally, so `x` is still one code point per cell
    /// and the row's width is still countable from it.
    #[tokio::test]
    async fn a_marked_run_carries_its_marks_beside_the_text_not_inside_it() {
        let outbox = Arc::new(Outbox::new(16));
        let wire = Wire::new(outbox.clone());
        let mut term = kampr_term::Emulator::new(20, 1);
        term.feed("e\u{301}f\u{301}g".as_bytes());
        let mut marked = reset();
        if let PaneUpdate::Reset { rows_data, cols, .. } = &mut marked {
            *cols = 20;
            *rows_data = Arc::new(vec![RowDiff {
                row: 0,
                cells: term.grid().row(0).to_vec(),
            }]);
        }
        wire.send_update("n/w1:p1", &marked);
        let frames = drain(&outbox);
        let run = &frames[0]["rows_data"][0]["runs"][0];
        assert_eq!(run["x"], "efg", "x is the bases, one code point per column");
        assert_eq!(run["m"], serde_json::json!(["\u{301}", "\u{301}"]));
        assert_eq!(run.get("w"), None);
    }

    /// History is append-only and the pump's cursor only moves forward, so a dropped
    /// `scrollback` is a hole nothing repairs — the `grid.reset` that replaces a purged patch
    /// queue carries the viewport and nothing above it.
    #[tokio::test]
    async fn a_purge_drops_patches_but_never_history() {
        let outbox = Arc::new(Outbox::new(16));
        let wire = Wire::new(outbox.clone());
        wire.send_scrollback(
            "n/w1:p1",
            &ScrollbackDoc {
                from_top: 0,
                rows: vec![RowDiff {
                    row: 0,
                    cells: vec![Cell {
                        ch: 'h',
                        ..Cell::default()
                    }],
                }],
                total_rows: 1,
                complete: true,
                capped: false,
            },
        );
        wire.send_update("n/w1:p1", &reset());

        assert_eq!(outbox.purge_pane("n/w1:p1"), 1, "the grid frame goes");
        let survivors = drain(&outbox);
        assert_eq!(survivors.len(), 1);
        assert_eq!(survivors[0]["t"], "scrollback", "history survives a purge");
    }

    #[tokio::test]
    async fn an_error_carries_the_documented_spelling_of_its_code() {
        let outbox = Arc::new(Outbox::new(4));
        let wire = Wire::new(outbox.clone());
        wire.error(ErrorCode::Unsupported, "this node does not do that", None);
        let frames = drain(&outbox);
        assert_eq!(frames[0]["t"], "error");
        assert_eq!(frames[0]["code"], "unsupported");
        assert!(frames[0]["pane"].is_null());
    }
}
