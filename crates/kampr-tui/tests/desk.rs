//! The shell against this machine's own node, rendered headless.
//!
//! `#[ignore]`d: it needs a node answering on this host, it is **read-mostly** — it watches and
//! draws and never types into anybody's pane — and it prints the frame rather than asserting a
//! shape a real herd has no reason to hold still for.
//!
//! `cargo test -p kampr-tui --test desk -- --ignored --nocapture`

use kampr_client::{Client, Event};
use kampr_tui::app::{App, Options};
use kampr_tui::image::Images;
use kampr_tui::render::fit::{self, Chrome, Need};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::Arc;
use std::time::Duration;

/// A terminal that answers its size and refuses every resize request — which is what ghostty
/// 1.3.1 and kitty 0.48.2 do (#291), and the only honest stand-in when there is no tty at all.
struct Headless(u16, u16);

impl fit::Display for Headless {
    fn cells(&mut self) -> Option<(u16, u16)> {
        Some((self.0, self.1))
    }
    fn host(&mut self) -> Option<String> {
        Some("headless (no tty)".into())
    }
    fn largest(&mut self) -> Option<(u16, u16)> {
        Some((320, 90))
    }
    fn request(&mut self, _cols: u16, _rows: u16) {}
    fn settle(&mut self, _was: (u16, u16)) -> Option<(u16, u16)> {
        None
    }
}

#[tokio::test]
#[ignore = "needs a kampr node on this machine"]
async fn the_shell_draws_this_machines_own_herd() {
    let config = kampr_node::config::default_config_dir();
    let session = match kampr_client::resolve(&config, None).await {
        Ok(session) => session,
        Err(e) => panic!("no herd to open — this test needs a node on this machine: {e}"),
    };
    println!("session: {}", session.describe());
    let client = Arc::new(Client::start(session));
    let mut events = client.events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(Event::Prefs { greeting: true })) => break,
            Ok(Ok(Event::Disconnected { reason })) => panic!("the node dropped this client: {reason}"),
            Ok(Ok(_)) => {}
            _ => panic!("the greeting never finished"),
        }
    }

    let (width, height) = (120u16, 34u16);
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.adopt_prefs();
    app.refocus();
    if let Ok(pane) = std::env::var("KAMPR_PANE") {
        app.clicked(kampr_tui::mouse::Click::Focus(pane));
    }
    app.sync_watches();
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if let Ok(Event::Grid { .. }) = events.recv().await {
                break;
            }
        }
    })
    .await;

    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a backend");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");

    if let Some(pane) = app.focused().map(str::to_string) {
        let need = {
            let state = client.state();
            let cols = state.herd.pane(&pane).and_then(|e| e.cols);
            let held = state.pane(&pane).map(|p| p.geometry());
            match (cols, held) {
                (Some(cols), Some((_, rows))) => Some(Need { cols, rows }),
                (None, Some((cols, rows))) if cols > 0 => Some(Need { cols, rows }),
                _ => None,
            }
        };
        if let Some(need) = need {
            app.fit(&mut Headless(width, height), need, Chrome { cols: 34, rows: 5 });
            terminal.draw(|frame| app.draw(frame)).expect("a frame");
        }
    }

    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        println!("{line}");
    }
    println!("\nladder: {:?}", app.rung().map(|r| r.report()));
}
