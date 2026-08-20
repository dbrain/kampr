//! End-to-end check against a live herdr session.
//!
//!   herdr --session kpcore                              # somewhere
//!   HERDR_SESSION=kpcore cargo run -p kampr-core --example live
//!
//! Proves the four things brief A is judged on: two watchers on one emulator, an observer restart,
//! a native-geometry change, and 400+ rows of coloured scrollback.

use anyhow::{Context, Result};
use kampr_core::herdr_provider::connect_with_retry;
use kampr_core::provider::Input;
use kampr_core::registry::{PaneRegistry, PaneUpdate, Watcher};
use kampr_core::wire::{Encoder, ServerMsg};
use kampr_core::{HerdrConfig, Provider};
use kampr_herdr::Herdr;
use std::sync::Arc;
use std::time::Duration;

const LINES: usize = 600;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kampr_core=debug")
        .init();
    let herdr = Herdr::discover()?;
    let provider = Arc::new(connect_with_retry(herdr, HerdrConfig::default()).await?);
    let node_id = "01LIVE";

    let panes = provider.list_panes().await?;
    println!("== herd ==");
    for p in &panes {
        println!(
            "  {}  {}x{}  agent {:?}  scrollback {}",
            p.pane_id, p.cols, p.rows, p.agent, p.scrollback_rows
        );
    }
    let target = std::env::args()
        .nth(1)
        .or_else(|| {
            panes
                .iter()
                .find(|p| p.agent.is_none())
                .map(|p| p.pane_id.clone())
        })
        .context("no shell pane to drive")?;
    println!("target: {target}\n");

    let registry = PaneRegistry::new(provider.clone());
    let herd_log: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    tokio::spawn({
        let (registry, herd_log) = (registry.clone(), herd_log.clone());
        let mut topology = registry.topology();
        async move {
            while topology.changed().await.is_ok() {
                let Ok(panes) = registry.list_panes().await else {
                    continue;
                };
                let line = panes
                    .iter()
                    .map(|p| format!("{} {}x{} sb{}", p.pane_id, p.cols, p.rows, p.scrollback_rows))
                    .collect::<Vec<_>>()
                    .join("  ");
                herd_log.lock().unwrap().push(line);
            }
        }
    });

    println!("== 1. two watchers, one emulator ==");
    let mut a = registry.watch(&target).await?;
    let mut b = registry.watch(&target).await?;
    println!("  watcher_count = {}", registry.watcher_count(&target));
    println!("  observe children = {}", observe_children(&target));

    registry.write(&target, Input::Bytes(b"clear\n".to_vec())).await?;
    drain(&mut a, Duration::from_millis(1200)).await;
    drain(&mut b, Duration::from_millis(200)).await;

    registry
        .write(
            &target,
            Input::Bytes(
                format!(
                    "for i in $(seq 1 {LINES}); do printf '\\033[38;5;%dmSB-%04d\\033[0m coloured scrollback row\\n' $((i % 214 + 16)) $i; done\n"
                )
                .into_bytes(),
            ),
        )
        .await?;
    let ua = drain(&mut a, Duration::from_secs(3)).await;
    let ub = drain(&mut b, Duration::from_secs(1)).await;
    println!("  A saw {} updates, B saw {} updates", ua.len(), ub.len());
    println!("  observe children still = {}", observe_children(&target));

    println!("\n== 2. a late watcher is caught up from the shared emulator ==");
    let c = registry.watch(&target).await?;
    let joined = c.initial();
    println!("  watcher_count = {}", registry.watcher_count(&target));
    println!("  observe children = {}", observe_children(&target));
    println!("  joiner's grid  = {:?}", joined.geometry());
    let disturbed = drain(&mut a, Duration::from_millis(400)).await;
    println!(
        "  updates pushed at existing watchers by the join: {}",
        disturbed.len()
    );
    for line in rows_text(joined).iter().rev().take(3).rev() {
        println!("    | {line}");
    }

    println!("\n== 3. wire encoding for one connection ==");
    let mut enc = Encoder::new();
    let msgs = enc.encode(&format!("{node_id}/{target}"), joined);
    let mut style_count = 0;
    for m in &msgs {
        let json = serde_json::to_string(m)?;
        if let ServerMsg::Styles(s) = m {
            style_count = s.styles.len();
        }
        println!("  {} bytes  {}", json.len(), &json[..json.len().min(72)]);
    }
    let per_cell = serde_json::to_string(joined.rows())?.len();
    println!("  interned styles: {style_count};  per-cell JSON would be {per_cell} bytes");

    println!("\n== 4. scrollback ==");
    let doc = registry
        .scrollback(&target)
        .await?
        .context("scrollback was refused")?;
    let text = rows_text_diffs(&doc.rows);
    let markers = (1..=LINES)
        .filter(|i| text.iter().any(|l| l.contains(&format!("SB-{i:04}"))))
        .count();
    let coloured = doc
        .rows
        .iter()
        .filter(|r| r.cells.iter().any(|c| c.fg != kampr_term::Color::Default))
        .count();
    println!(
        "  rows {} (from_top {} of total {}, complete {})",
        doc.rows.len(),
        doc.from_top,
        doc.total_rows,
        doc.complete
    );
    let on_screen = {
        let live = registry.watch(&target).await?;
        rows_text(live.initial())
    };
    let union = (1..=LINES)
        .filter(|i| {
            let m = format!("SB-{i:04}");
            text.iter().any(|l| l.contains(&m)) || on_screen.iter().any(|l| l.contains(&m))
        })
        .count();
    println!("  markers in the ring: {markers}/{LINES};  rows carrying colour: {coloured}");
    println!("  markers in ring + live grid: {union}/{LINES}");
    println!(
        "  first row index {:?}, last {:?}",
        doc.rows.first().map(|r| r.row),
        doc.rows.last().map(|r| r.row)
    );
    let sb = enc.encode_scrollback(&format!("{node_id}/{target}"), &doc);
    println!(
        "  scrollback message: {} bytes",
        serde_json::to_string(sb.last().unwrap())?.len()
    );

    println!("\n== 5. observer restart ==");
    let before = observe_children(&target);
    kill_observers(&target);
    let restart = wait_for_reset(&mut a, Duration::from_secs(15)).await;
    println!("  killed {before} observe child(ren)");
    match &restart {
        Some(u) => println!("  A got a fresh grid.reset at {:?}", u.geometry()),
        None => println!("  NO RESET — restart supervision failed"),
    }
    println!("  observe children = {}", observe_children(&target));
    let _ = drain(&mut b, Duration::from_millis(500)).await;

    println!("\n== 6. native geometry change (resize the desk now) ==");
    let start = restart.as_ref().and_then(PaneUpdate::geometry);
    let mut seen = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while tokio::time::Instant::now() < deadline {
        let Some(u) = wait_for_reset(&mut a, Duration::from_secs(5)).await else {
            continue;
        };
        println!("  ... reset at {:?}", u.geometry());
        if u.geometry() != start {
            seen = u.geometry();
            break;
        }
    }
    match seen {
        Some(g) => println!("  grid.reset at the new native geometry {g:?} (was {start:?})"),
        None => println!("  no geometry change observed"),
    }

    println!("\n== 7. herdr restart (stop and start the session now) ==");
    if !wait_until(Duration::from_secs(90), || async {
        provider.refresh().await.is_err()
    })
    .await
    {
        println!("  herdr never went away — skip");
    }
    println!("  socket gone; observe children = {}", observe_children(&target));
    if !wait_until(Duration::from_secs(180), || async {
        provider.refresh().await.is_ok()
    })
    .await
    {
        println!("  herdr never came back — skip");
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("  socket back; observe children = {}", observe_children(&target));
    let _ = drain(&mut b, Duration::from_secs(2)).await;
    registry
        .write(&target, Input::Bytes(b"echo RECOVERED-OK\n".to_vec()))
        .await?;
    let after = drain(&mut b, Duration::from_secs(4)).await;
    let alive = after
        .iter()
        .any(|u| rows_text(u).iter().any(|l| l.contains("RECOVERED-OK")));
    println!("  the same watcher is live again after the socket died: {alive}");
    println!("  watcher_count still {}", registry.watcher_count(&target));

    println!("\n== 8. teardown ==");
    drop(a);
    drop(c);
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!(
        "  one watcher left; observe children = {}",
        observe_children(&target)
    );
    drop(b);
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("  watcher_count = {}", registry.watcher_count(&target));
    println!("  observe children = {}", observe_children(&target));

    println!("\n== 9. herd model, driven by the topology signal ==");
    let log = herd_log.lock().unwrap();
    println!("  {} herd changes observed", log.len());
    for line in log.iter().rev().take(4).rev() {
        println!("    {line}");
    }
    Ok(())
}

async fn wait_until<F, Fut>(budget: Duration, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    false
}

async fn drain(w: &mut Watcher, budget: Duration) -> Vec<PaneUpdate> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + budget;
    while let Ok(Ok(u)) = tokio::time::timeout_at(deadline, w.recv()).await {
        out.push(u);
    }
    out
}

async fn wait_for_reset(w: &mut Watcher, budget: Duration) -> Option<PaneUpdate> {
    let deadline = tokio::time::Instant::now() + budget;
    while let Ok(Ok(u)) = tokio::time::timeout_at(deadline, w.recv()).await {
        if u.is_reset() {
            return Some(u);
        }
    }
    None
}

fn rows_text(u: &PaneUpdate) -> Vec<String> {
    rows_text_diffs(u.rows())
}

fn rows_text_diffs(rows: &[kampr_term::RowDiff]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn observe_children(pane: &str) -> usize {
    let out = std::process::Command::new("pgrep")
        .args(["-f", &format!("terminal session observe {pane}")])
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0)
}

fn kill_observers(pane: &str) {
    let _ = std::process::Command::new("pkill")
        .args(["-f", &format!("terminal session observe {pane}")])
        .status();
}
