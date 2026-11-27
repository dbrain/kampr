//! The rows above a full-screen program are the shell session from before it, whatever its name.

use super::*;

fn held(c: &Client, id: &str) -> (u64, bool) {
    let total = c.docs.get(id).and_then(|d| d["total_rows"].as_u64());
    let rows = c.history.get(id).map_or(&[][..], Vec::as_slice);
    (total.unwrap_or(0), rows.iter().any(|r| r == "250"))
}

/// The operator, on muffit's TUI in a native pane: the shell output from before it stayed drawn
/// above it. A program nobody named holds the alternate screen as a harness does, so the era before
/// it goes while it runs and comes back when it gives the screen up. `dd` is the job while it
/// waits, as muffit is: on no harness list.
#[tokio::test]
async fn a_program_on_the_alternate_screen_supersedes_the_shell_era_whatever_its_name() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let script = "seq 1 300; stty -echo raw; dd bs=1 count=1 2>/dev/null; \
                  printf '\\033[?1049h\\033[?1000h\\033[?1006hTUI\\r\\n'; dd bs=1 count=1 2>/dev/null; \
                  printf '\\033[?1049l'; exec cat";
    let id = client.create(&["sh", "-c", script], json!({})).await;
    client.watch(&id, true).await;
    client
        .until("the shell era in history", |c| (held(c, &id).1).then_some(()))
        .await;
    let era = client.docs[&id]["era"].clone();

    client.type_text(&id, "x").await;
    client.shows(&id, "TUI").await;
    client
        .until("the shell era superseded", |c| {
            (c.docs[&id]["era"] != era && held(c, &id) == (0, false)).then_some(())
        })
        .await;

    client.type_text(&id, "x").await;
    client
        .until("the shell era given back", |c| (held(c, &id).1).then_some(()))
        .await;
}
