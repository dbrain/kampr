use anyhow::{Result, anyhow, bail};
use kampr_herdr::{Controller, Herdr};
use kampr_node::Config;
use kampr_node::manage::{MIN_COLS, MIN_ROWS, checked_size};
use std::path::Path;

/// `kampr resize` — the same escape hatch the phone has, for the case it is most often needed in:
/// an agent spun a herdr up headless, the pane was born at whatever size that shell was, and you
/// are already on the box.
///
/// Worth having over a bare `herdr terminal session control` for two reasons, and they are the two
/// that took the longest to learn. A controller that stops holds the PTY for ever (#20), so the
/// release is timed and ends in a kill rather than a hope. And the size is *measured* afterwards
/// rather than assumed: on a pane with a desk attached the geometry is handed straight back on
/// release (#19), so a command that printed "resized" either way would be lying half the time.
pub async fn run(config: &Path, cols: u32, rows: u32, pane: Option<String>) -> Result<()> {
    checked_size(cols, rows).map_err(|e| {
        anyhow!("{e}\n  a pane keeps the size it is given, so {MIN_COLS}x{MIN_ROWS} is the floor")
    })?;

    let config = Config::load(config)?;
    let herdr = Herdr::discover()?;
    let socket = herdr.socket().to_path_buf();

    let pane = match pane {
        Some(id) => id,
        None => sole_pane(&herdr).await?,
    };

    let before = viewport_rows(&herdr, &pane).await;
    println!("claiming {pane} at {cols}x{rows} …");
    let controller = Controller::claim(&config.herdr.binary, &socket, &pane, cols, rows).await?;
    controller.release().await?;

    match viewport_rows(&herdr, &pane).await {
        Some(measured) if measured == u64::from(rows) => {
            println!("  {pane} is {cols}x{rows}, and stayed there after the claim was released");
        }
        Some(measured) => {
            println!(
                "  {pane} came back at {measured} rows, not {rows}.\n  \
                 Something is attached to this pane at a desk, and a desk restores its own \
                 geometry the moment a controller lets go. Resize it from there, or hold the \
                 claim from the app's zoom panel."
            );
        }
        None => {
            println!(
                "  {pane} was resized, but herdr would not say what it is now — nothing on its \
                 socket API reports a column count, and the rows did not come back either."
            );
        }
    }
    if let (Some(was), true) = (before, true) {
        println!("  it was {was} rows before this ran");
    }
    Ok(())
}

/// The pane to act on when the operator named none. One is unambiguous; several is a question only
/// they can answer, so it lists them rather than guessing.
async fn sole_pane(herdr: &Herdr) -> Result<String> {
    let snapshot = herdr.snapshot().await?;
    let panes: Vec<String> = snapshot.panes.iter().map(|pane| pane.pane_id.clone()).collect();
    match panes.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!("this herdr has no panes to resize"),
        many => bail!(
            "this herdr has {} panes — name one:\n  {}",
            many.len(),
            many.join("\n  ")
        ),
    }
}

async fn viewport_rows(herdr: &Herdr, pane: &str) -> Option<u64> {
    let reply: kampr_herdr::model::PaneReply = herdr
        .call("pane.get", serde_json::json!({ "pane_id": pane }))
        .await
        .ok()?;
    reply.pane.scroll.map(|s| s.viewport_rows)
}
