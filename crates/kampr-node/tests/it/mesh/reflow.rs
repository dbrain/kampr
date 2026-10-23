//! A peer's native pane watched through the hub at a width of the watcher's own (docs/17 §3).

use super::*;
use crate::native::Client;

fn script() -> String {
    "for i in $(seq 1 30); do printf 'L%02d' $i; printf '%097d\\n' 0; done; \
     printf 'W'; printf '%0149d\\n' 0; printf 'P%0129d' 0; exec cat"
        .to_string()
}

async fn reset_where(client: &mut Client, id: &str, want: impl Fn(&Value) -> bool) -> Value {
    let id = id.to_string();
    client
        .until("a grid.reset as wanted", |c| {
            c.resets.get(&id).filter(|r| want(r)).cloned()
        })
        .await
}

async fn found(client: &mut Client, id: &str, query: &str) -> Value {
    client
        .send(json!({ "t": "find", "pane": id, "query": query, "backward": true }))
        .await;
    let found = client.reply("find").await;
    assert_eq!(found["pane"], id, "{found}");
    found
}

fn reflowed(id: &str, cols: u16) -> Value {
    json!({ "t": "watch", "pane": id, "scrollback": true, "reflow": { "cols": cols } })
}

/// The view is the peer's: its rows, its history and its search reach the phone through the hub
/// exactly as they reach a client of the peer itself, and a viewer beside it keeps the pane's grid.
#[tokio::test(flavor = "multi_thread")]
async fn a_peers_native_shell_watched_narrower_through_the_hub_is_re_laid_by_the_peer() {
    let mesh = NativeMesh::start().await;
    let mut narrow = mesh.hub.client().await;
    narrow
        .send(json!({
            "t": "manage", "op": "native.create", "node": mesh.peer_node_id,
            "args": ["sh", "-c", script()], "cols": 120, "rows": 24,
        }))
        .await;
    let ack = narrow.reply("managed").await;
    assert_eq!(ack["ok"], true, "{ack}");
    let id = ack["id"].as_str().expect("a pane id").to_string();
    narrow.pane(&id, |_| true).await;

    narrow.send(reflowed(&id, 40)).await;
    let reset = reset_where(&mut narrow, &id, |r| r["reflow"] == "on").await;
    assert_eq!(
        (reset["cols"].clone(), reset["rows"].clone()),
        (json!(40), json!(24)),
        "{reset}"
    );
    let screen = narrow.shows(&id, "P0").await;
    assert!(screen.iter().all(|r| r.chars().count() <= 40), "{screen:#?}");
    assert!(screen.contains(&format!("P{}", "0".repeat(39))), "{screen:#?}");
    narrow
        .until("history re-laid at 40", |c| {
            let rows = c.history.get(&id)?;
            let first = rows.iter().position(|r| r.starts_with("L01"))?;
            (rows.get(first..first + 3)
                == Some(&[format!("L01{}", "0".repeat(37)), "0".repeat(40), "0".repeat(20)][..]))
            .then_some(())
        })
        .await;

    let mut also = mesh.hub.client().await;
    also.send(reflowed(&id, 40)).await;
    let shared = reset_where(&mut also, &id, |r| r["reflow"] == "on").await;
    assert_eq!(shared["cols"], json!(40), "{shared}");
    also.shows(&id, "P0").await;

    let mut wide = mesh.hub.client().await;
    wide.send(json!({ "t": "watch", "pane": id, "scrollback": true }))
        .await;
    let plain = reset_where(&mut wide, &id, |_| true).await;
    assert_eq!(plain["cols"], json!(120), "{plain}");
    assert!(plain.get("reflow").is_none(), "{plain}");
    let screen = wide.shows(&id, "P0").await;
    assert!(screen.contains(&format!("W{}", "0".repeat(119))), "{screen:#?}");

    let mut direct = mesh.peer.client().await;
    direct.send(reflowed(&id, 40)).await;
    reset_where(&mut direct, &id, |r| r["reflow"] == "on").await;
    let at_peer = found(&mut direct, &id, "L01").await;
    let through_hub = found(&mut narrow, &id, "L01").await;
    let of_the_pane = found(&mut wide, &id, "L01").await;
    assert_eq!(at_peer["total"], json!(1), "{at_peer}");
    assert_eq!(
        through_hub["matches"], at_peer["matches"],
        "{through_hub} vs {at_peer}"
    );
    assert_ne!(
        of_the_pane["matches"][0]["from_bottom"], at_peer["matches"][0]["from_bottom"],
        "the pane's own grid and the view count rows alike, so this proves nothing: {of_the_pane}"
    );

    mesh.stop();
}

async fn scripted_pane(hub: &Running, caps: Value) -> (Scripted, Home, &'static str) {
    let pane = "01JVIEW/n:s1";
    let home = Home::new();
    let mut peer = Scripted::join(hub, &home, "01JVIEW", "laptop").await;
    peer.say(json!({ "t": "hello", "node_id": "01JVIEW", "caps": caps }))
        .await;
    peer.advertise(&[("01JVIEW", "laptop")], &[pane]).await;
    mesh_settles(hub, 10, |peers| peers.herd().panes.iter().any(|p| p.id == pane)).await;
    (peer, home, pane)
}

fn view_reset(pane: &str, text: &str, label: Option<&str>) -> Value {
    let mut reset = json!({
        "t": "grid.reset", "pane": pane, "cols": 8, "rows": 2,
        "rows_data": [{ "row": 0, "runs": [{ "s": 0, "x": text }] }],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": [],
    });
    if let Some(label) = label {
        reset["reflow"] = json!(label);
    }
    reset
}

/// Two phones at one width are one stream on the link; a phone without the flag is another; and
/// everything the peer says about the view reaches the phone as the pane it asked about.
#[tokio::test(flavor = "multi_thread")]
async fn hub_watchers_of_one_width_share_one_view_and_its_verbs_reach_it() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let (mut peer, _home, pane) =
        scripted_pane(&hub, json!({ "reflow": true, "find": true, "convo.find": true })).await;
    let view = format!("{pane}~40");

    let mut first = hub.client().await;
    first.send(reflowed(pane, 40)).await;
    let asked = peer.next_but_ping().await;
    assert_eq!(
        (asked["t"].clone(), asked["pane"].clone()),
        (json!("watch"), json!(view)),
        "{asked}"
    );
    let mut second = hub.client().await;
    second.send(reflowed(pane, 40)).await;
    let mut wide = hub.client().await;
    wide.send(json!({ "t": "watch", "pane": pane })).await;
    let asked = peer.next_but_ping().await;
    assert_eq!(
        asked["pane"], pane,
        "a second phone at the width asked the peer again: {asked}"
    );

    peer.say(view_reset(&view, "narrow", Some("on"))).await;
    peer.say(view_reset(pane, "wide", None)).await;
    for client in [&mut first, &mut second] {
        let reset = reset_where(client, pane, |r| r["reflow"] == "on").await;
        assert_eq!(reset["rows_data"][0]["runs"][0]["x"], "narrow", "{reset}");
    }
    let plain = reset_where(&mut wide, pane, |_| true).await;
    assert_eq!(plain["rows_data"][0]["runs"][0]["x"], "wide", "{plain}");
    assert!(plain.get("reflow").is_none(), "{plain}");

    peer.say(view_reset(&view, "pager", Some("alternate"))).await;
    let reset = reset_where(&mut first, pane, |r| r["reflow"] == "alternate").await;
    assert_eq!(reset["rows_data"][0]["runs"][0]["x"], "pager", "{reset}");

    for (verb, extra) in [
        ("find", json!({ "query": "x" })),
        ("convo.load", json!({})),
        ("convo.find", json!({ "query": "x" })),
    ] {
        let mut ask = json!({ "t": verb, "pane": pane });
        for (k, v) in extra.as_object().unwrap() {
            ask[k] = v.clone();
        }
        first.send(ask).await;
        let relayed = peer.next_but_ping().await;
        assert_eq!(
            (relayed["t"].clone(), relayed["pane"].clone()),
            (json!(verb), json!(view)),
            "{relayed}"
        );
    }
    peer.say(json!({ "t": "find", "pane": view, "query": "x", "matches": [], "total": 0 }))
        .await;
    let answer = first.reply("find").await;
    assert_eq!(answer["pane"], pane, "{answer}");

    wide.send(json!({ "t": "find", "pane": pane, "query": "y" }))
        .await;
    let relayed = peer.next_but_ping().await;
    assert_eq!(
        relayed["pane"], pane,
        "a plain watch's search went to the view: {relayed}"
    );

    hub.stop();
}

/// A peer whose build has no reflow is never sent a view it would answer with `unknown_pane` or
/// silence: it is asked for the pane, and the phone is told the grid is the pane's own.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_too_old_to_reflow_is_watched_as_the_pane_and_said_to_be_unsupported() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let (mut peer, _home, pane) = scripted_pane(&hub, json!({ "find": true })).await;

    let mut client = hub.client().await;
    client.send(reflowed(pane, 40)).await;
    let asked = peer.next_but_ping().await;
    assert_eq!(
        (asked["t"].clone(), asked["pane"].clone()),
        (json!("watch"), json!(pane)),
        "{asked}"
    );
    peer.say(view_reset(pane, "wide", None)).await;
    let reset = reset_where(&mut client, pane, |_| true).await;
    assert_eq!(reset["reflow"], "unsupported", "{reset}");
    assert_eq!(reset["rows_data"][0]["runs"][0]["x"], "wide", "{reset}");

    client
        .send(json!({ "t": "find", "pane": pane, "query": "x" }))
        .await;
    let relayed = peer.next_but_ping().await;
    assert_eq!(relayed["pane"], pane, "{relayed}");

    hub.stop();
}
