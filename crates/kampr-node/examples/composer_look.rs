//! The grid Kampr's own emulator builds for one pane, and what the composer reader makes of it,
//! printed as one JSON line per `look` read on stdin. Driven by `research/probe/mirror/*.py`.
//!
//! Usage: composer_look <herdr socket> <pane> <cols> <rows> <agent>

use kampr_herdr::{Observer, StreamEvent};
use kampr_journal::{Caret, ComposerReader};
use kampr_term::Emulator;
use std::io::BufRead;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (socket, pane, cols, rows, agent) =
        (&args[1], &args[2], args[3].parse()?, args[4].parse()?, &args[5]);
    let reader: Option<ComposerReader> = match agent.as_str() {
        "claude" => Some(kampr_journal::claude::composer),
        "codex" => Some(kampr_journal::codex::composer),
        "agy" => Some(kampr_journal::agy::composer),
        "omp" => Some(kampr_journal::omp::composer),
        _ => None,
    };
    let mut obs = Observer::spawn("herdr", std::path::Path::new(socket), pane, cols, rows)?;
    let term = Arc::new(Mutex::new(Emulator::new(cols as u16, rows as u16)));
    let fed = term.clone();
    tokio::spawn(async move {
        while let Some(event) = obs.events.recv().await {
            if let StreamEvent::Frame { full, bytes, .. } = event {
                let mut term = fed.lock().unwrap();
                if full {
                    term.reset();
                }
                term.feed(&bytes);
            }
        }
    });
    let stdin = std::io::stdin();
    for _ in stdin.lock().lines() {
        let term = term.lock().unwrap();
        let grid = term.grid();
        let lines: Vec<String> = (0..grid.rows()).map(|r| grid.row_text(r)).collect();
        let (col, row, _) = term.cursor();
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let caret = Caret { col, row };
        let read = reader.and_then(|r| r(&borrowed, caret, kampr_core::registry::frame(grid, caret)));
        println!(
            "{}",
            serde_json::json!({
                "rows": lines,
                "caret": [col, row],
                "read": read.as_ref().map(|c| &c.text),
                "at": read.as_ref().map(|c| c.caret),
            })
        );
    }
    Ok(())
}
