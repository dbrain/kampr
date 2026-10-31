//! A native pane watched at a width of the watcher's own, re-laid by the node (docs/17 §3).

use super::*;

const LONG: usize = 150;

fn script(after: &str) -> String {
    format!(
        "for i in $(seq 1 30); do printf 'L%02d' $i; printf '%097d\\n' 0; done; \
         printf 'W'; printf '%0{}d\\n' 0; {after}",
        LONG - 1
    )
}

async fn reflowed(client: &mut Client, id: &str, cols: u16) {
    client
        .send(json!({ "t": "watch", "pane": id, "scrollback": true, "reflow": { "cols": cols } }))
        .await;
}

async fn reset_where(client: &mut Client, id: &str, want: impl Fn(&Value) -> bool) -> Value {
    let id = id.to_string();
    client
        .until("a grid.reset as wanted", |c| {
            c.resets.get(&id).filter(|r| want(r)).cloned()
        })
        .await
}

fn cursor_of(client: &Client, id: &str) -> (u64, u64) {
    let cursor = &client.grids[id].0;
    (cursor["row"].as_u64().unwrap(), cursor["col"].as_u64().unwrap())
}

#[tokio::test]
async fn a_native_shell_watched_narrower_gets_its_lines_and_history_re_laid_and_the_pty_untouched() {
    let h = Harness::start().await;
    let mut narrow = h.client().await;
    let mut wide = h.client().await;
    let line = script("printf 'P%0129d' 0; exec cat");
    let id = narrow
        .create(&["sh", "-c", &line], json!({ "cols": 120, "rows": 24 }))
        .await;

    reflowed(&mut narrow, &id, 40).await;
    wide.watch(&id, false).await;
    let reset = reset_where(&mut narrow, &id, |r| r["reflow"] == "on").await;
    assert_eq!(
        (reset["cols"].clone(), reset["rows"].clone()),
        (json!(40), json!(24)),
        "{reset}"
    );
    let screen = narrow.shows(&id, "P0").await;
    let tail = [
        format!("W{}", "0".repeat(39)),
        "0".repeat(40),
        "0".repeat(40),
        "0".repeat(LONG - 120),
        format!("P{}", "0".repeat(39)),
        "0".repeat(40),
        "0".repeat(40),
        "0".repeat(10),
    ];
    let at = screen
        .iter()
        .position(|r| r.starts_with('W'))
        .expect("the long line");
    assert_eq!(screen[at..at + 8], tail, "{screen:#?}");
    assert!(screen.iter().all(|r| r.chars().count() <= 40), "{screen:#?}");
    assert_eq!(cursor_of(&narrow, &id), ((at + 7) as u64, 10), "{screen:#?}");

    narrow
        .until("history re-laid at 40", |c| {
            let rows = c.history.get(&id)?;
            let first = rows.iter().position(|r| r.starts_with("L01"))?;
            (rows.get(first..first + 3)
                == Some(&[format!("L01{}", "0".repeat(37)), "0".repeat(40), "0".repeat(20)][..]))
            .then_some(())
        })
        .await;

    let plain = reset_where(&mut wide, &id, |_| true).await;
    assert_eq!(plain["cols"], json!(120), "{plain}");
    assert!(plain.get("reflow").is_none(), "{plain}");
    let screen = wide.shows(&id, "P0").await;
    assert!(
        screen.iter().any(|r| *r == format!("W{}", "0".repeat(119))),
        "{screen:#?}"
    );

    let pane = narrow.pane(&id, |_| true).await;
    assert_eq!(
        (pane["cols"].clone(), pane["rows"].clone()),
        (json!(120), json!(24)),
        "{pane}"
    );
    assert!(
        pane.get("size_holder").is_none() || pane["size_holder"] == "spawn",
        "{pane}"
    );
    assert_eq!(
        session_of(&h.root(), &id)
            .size()
            .map(|s| (s.cols, s.rows))
            .unwrap_or((120, 24)),
        (120, 24)
    );
}

#[tokio::test]
async fn a_native_reflow_falls_back_to_the_pane_grid_while_the_alternate_screen_is_up() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let line = script("printf '\\033[?1049hFULLSCREEN'; read x; printf '\\033[?1049l$ '; exec cat");
    let id = client
        .create(&["sh", "-c", &line], json!({ "cols": 120, "rows": 24 }))
        .await;
    reflowed(&mut client, &id, 40).await;
    let reset = reset_where(&mut client, &id, |r| r["reflow"] == "alternate").await;
    assert_eq!(reset["cols"], json!(120), "{reset}");
    client.shows(&id, "FULLSCREEN").await;

    client.type_text(&id, "\r").await;
    let reset = reset_where(&mut client, &id, |r| r["reflow"] == "on").await;
    assert_eq!(reset["cols"], json!(40), "{reset}");
    client.shows(&id, "$").await;
}

#[tokio::test]
async fn a_native_claude_is_never_re_laid() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::start().await;
    let claude = h.work().join("claude");
    std::fs::write(
        &claude,
        format!("#!/bin/sh\nprintf '%0{LONG}d\\n' 0\nwhile :; do sleep 1; done\n"),
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut client = h.client().await;
    let id = client
        .create(
            &[&claude.display().to_string()],
            json!({ "cols": 120, "rows": 24 }),
        )
        .await;
    client.pane(&id, |p| p["agent"] == "claude").await;
    reflowed(&mut client, &id, 40).await;
    let reset = reset_where(&mut client, &id, |r| r.get("reflow").is_some()).await;
    assert_eq!(
        (reset["reflow"].clone(), reset["cols"].clone()),
        (json!("harness"), json!(120)),
        "{reset}"
    );
}

/// A harness typed into a reflowed shell takes the view back to the pane's own grid as soon as
/// the session sees the job start, not when a cached answer about the foreground runs out: the
/// echo of the line that starts it is read first, and that look finds the shell still there.
#[tokio::test]
async fn a_harness_started_by_input_switches_the_reflowed_view_at_once() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::start().await;
    let claude = h.work().join("claude");
    std::fs::write(&claude, "#!/bin/sh\nwhile :; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut client = h.client().await;
    let id = client
        .create(&["sh", "-i"], json!({ "cols": 120, "rows": 24 }))
        .await;
    reflowed(&mut client, &id, 40).await;
    reset_where(&mut client, &id, |r| r["reflow"] == "on").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    client.type_text(&id, &format!("{}\r", claude.display())).await;
    let typed = std::time::Instant::now();
    reset_where(&mut client, &id, |r| r["reflow"] == "harness").await;
    let took = typed.elapsed();
    assert!(
        took < Duration::from_millis(400),
        "the view switched {took:?} after the harness was typed"
    );
}
