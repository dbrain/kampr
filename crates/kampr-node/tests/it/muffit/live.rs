//! Against the real daemon on this machine, read-only: it lists, reads transcripts and, for a
//! session that is already live, attaches; it never sends input, answers, creates or stops
//! anything. `MUFFIT_LIVE=1` runs it.

use super::{Harness, NODE};
use serde_json::{Value, json};

const DAEMON: &str = "http://127.0.0.1:7891";

fn token() -> String {
    let home = std::env::var("HOME").expect("a home");
    std::fs::read_to_string(format!("{home}/.local/share/muffit/server.token"))
        .expect("the daemon's token")
        .trim()
        .to_string()
}

fn live() -> bool {
    let on = std::env::var("MUFFIT_LIVE").as_deref() == Ok("1");
    if !on {
        eprintln!("skipped: MUFFIT_LIVE=1 runs it against the daemon at {DAEMON}");
    }
    on
}

fn sane(turns: &[Value], what: &str) {
    for turn in turns {
        assert!(
            matches!(turn["role"].as_str(), Some("user" | "assistant")),
            "{what}: {turn}"
        );
        assert!(
            turn["id"].as_str().is_some_and(|id| id.starts_with('m')),
            "{what}: {turn}"
        );
        let blocks = turn["blocks"].as_array().unwrap();
        assert!(!blocks.is_empty(), "{what}: {turn}");
        for block in blocks {
            assert!(
                matches!(block["b"].as_str(), Some("md" | "tool" | "code" | "diff" | "sub")),
                "{what}: {block}"
            );
        }
    }
}

#[tokio::test]
async fn the_real_daemon_s_stored_transcripts_map_to_turns() {
    if !live() {
        return;
    }
    let token = token();
    let all: Value = reqwest::Client::new()
        .get(format!("{DAEMON}/sessions?all=true"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("the daemon")
        .json()
        .await
        .unwrap();
    let client = kampr_muffit::Client::new(DAEMON, &token).unwrap();
    let mut mapped = 0;
    for row in all["sessions"].as_array().expect("sessions").iter().take(20) {
        let id = row["id"].as_str().unwrap();
        let page = client
            .transcript(id, None, None, 500)
            .await
            .expect("a transcript");
        let turns = kampr_muffit::mapper::turns(&page.transcript, &Default::default());
        let turns = serde_json::to_value(&turns).unwrap();
        sane(turns.as_array().unwrap(), id);
        let entries = page
            .transcript
            .iter()
            .filter(|e| e.role == "user" || e.role == "assistant")
            .count();
        eprintln!(
            "{id}: {} entries, {entries} spoken, {} turns",
            page.transcript.len(),
            turns.as_array().unwrap().len()
        );
        mapped += 1;
    }
    assert!(mapped > 0, "the daemon has no stored sessions to read");
}

#[tokio::test]
async fn the_real_daemon_s_live_sessions_are_panes_with_readable_conversations() {
    if !live() {
        return;
    }
    let token = token();
    let client = kampr_muffit::Client::new(DAEMON, &token).unwrap();
    if client.sessions().await.expect("the daemon").is_empty() {
        eprintln!("skipped: no session is live on the daemon right now");
        return;
    }
    let h = Harness::beside(DAEMON, &token).await;
    let mut client = h.client().await;
    let prefix = format!("{NODE}/m:");
    let first = client
        .until("a muffit pane", |c| {
            c.panes
                .values()
                .find(|p| p["id"].as_str().unwrap().starts_with(&prefix))
                .cloned()
        })
        .await;
    assert_eq!(first["provider"], "muffit", "{first}");
    assert_eq!(first["has_conversation"], json!(true), "{first}");
    let id = first["id"].as_str().unwrap().to_string();
    client.watch(&id).await;
    let page = client.convo(&id).await;
    sane(page["turns"].as_array().expect("turns"), &id);
    let facets = client.frame("facets", |f: &Value| f["t"] == "convo.facets").await;
    eprintln!(
        "{id}: {} turns, facets {}",
        page["turns"].as_array().unwrap().len(),
        facets["facets"]
    );
}

#[tokio::test]
async fn the_real_daemon_s_finished_conversation_is_read_through_the_node_without_restoring_it() {
    if !live() {
        return;
    }
    let token = token();
    let daemon = kampr_muffit::Client::new(DAEMON, &token).unwrap();
    let live_now: Vec<String> = daemon
        .sessions()
        .await
        .expect("the daemon")
        .into_iter()
        .map(|s| s.id)
        .collect();
    let h = Harness::beside(DAEMON, &token).await;
    let mut client = h.client().await;
    let prefix = format!("{NODE}/m:");
    let stored = client
        .until("a finished conversation", |c| {
            c.panes
                .values()
                .find(|p| {
                    let id = p["id"].as_str().unwrap();
                    id.starts_with(&prefix) && !live_now.iter().any(|l| id.ends_with(&format!("m:{l}")))
                })
                .cloned()
        })
        .await;
    assert_eq!(stored["agent_status"], "idle", "{stored}");
    let id = stored["id"].as_str().unwrap().to_string();
    let session = stored["session_id"].as_str().unwrap().to_string();
    client.watch(&id).await;
    let page = client.convo(&id).await;
    sane(page["turns"].as_array().expect("turns"), &id);
    assert!(!page["turns"].as_array().unwrap().is_empty(), "{page}");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let live_after = daemon.sessions().await.expect("the daemon");
    assert!(
        !live_after.iter().any(|s| s.id == session),
        "watching {session} restored it: the node attached"
    );
    eprintln!(
        "{id}: {} turns read without attaching",
        page["turns"].as_array().unwrap().len()
    );
}
