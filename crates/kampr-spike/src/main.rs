//! Fidelity check for the observe -> emulator -> cell-grid pipeline.
//!
//! Reconstructs a pane's grid from the frame stream alone, then compares it against herdr's own
//! `pane.read visible`. Any mismatch is an emulator bug, since both describe the same screen.

use anyhow::{Context, Result, bail};
use kampr_herdr::{Herdr, Observer, StreamEvent};
use kampr_term::Emulator;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let herdr = Herdr::discover()?;
    let pane_id = std::env::args().nth(1);

    let snap = herdr.snapshot().await.context("session.snapshot")?;
    let pane = match &pane_id {
        Some(id) => snap.pane(id).context("pane not found")?,
        None => snap.panes.first().context("no panes in session")?,
    };
    let (cols, rows) = snap.geometry(&pane.pane_id).context("pane has no layout rect")?;
    println!(
        "pane {} — native {}x{} — agent {:?} — scrollback safe: {}",
        pane.pane_id,
        cols,
        rows,
        pane.agent,
        pane.scrollback_is_safe_to_read()
    );

    let mut obs = Observer::spawn("herdr", herdr.socket(), &pane.pane_id, cols, rows)?;
    let mut term = Emulator::new(cols as u16, rows as u16);

    let mut frames = 0usize;
    let mut bytes = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, obs.events.recv()).await {
            Ok(Some(StreamEvent::Frame { full, bytes: b, .. })) => {
                if full {
                    term.reset();
                }
                bytes += b.len();
                frames += 1;
                term.feed(&b);
            }
            Ok(Some(StreamEvent::Closed { reason })) => bail!("stream closed: {reason}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    obs.shutdown().await;

    let mine = render(&term);
    let theirs: kampr_herdr::model::Read = herdr
        .call(
            "pane.read",
            serde_json::json!({
                "pane_id": pane.pane_id, "source": "visible",
                "lines": rows, "format": "text"
            }),
        )
        .await
        .map(|r: kampr_herdr::model::ReadReply| r.read)?;

    let a: Vec<&str> = mine.lines().map(str::trim_end).collect();
    let b: Vec<&str> = theirs.text.lines().map(str::trim_end).collect();
    let n = a.len().max(b.len());
    let mut same = 0;
    let mut first_bad = None;
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(""), b.get(i).copied().unwrap_or(""));
        if x == y {
            same += 1;
        } else if first_bad.is_none() {
            first_bad = Some((i, x.to_string(), y.to_string()));
        }
    }

    println!("frames applied: {frames}  ({bytes} bytes)");
    println!(
        "grid {}x{}  rows matching herdr's own read: {same}/{n}",
        term.grid().cols(),
        term.grid().rows()
    );
    let (cc, cr, cv) = term.cursor();
    println!(
        "cursor: col {cc} row {cr} visible {cv}   hyperlinks interned: {}",
        term.grid().links.len()
    );
    match first_bad {
        None => println!("\nPERFECT MATCH"),
        Some((i, x, y)) => {
            println!("\nfirst mismatch at row {i}:\n  kampr : {x:?}\n  herdr : {y:?}");
        }
    }
    Ok(())
}

fn render(term: &Emulator) -> String {
    let g = term.grid();
    let mut out = String::new();
    for r in 0..g.rows() {
        for c in g.row(r) {
            out.push(c.ch);
        }
        out.push('\n');
    }
    out
}
