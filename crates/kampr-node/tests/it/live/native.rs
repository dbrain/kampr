//! A native session shown on a real herdr desk (docs/17 §4): one pane in the herd, an agent at the
//! desk, and a session that outlives its herdr pane and herdr itself.

use super::{Harness, Socket, recv, send, until};
use kampr_auth::Role;
use kampr_core::wire::PaneEntry;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SOON: Duration = Duration::from_secs(30);

/// Every keeper under the root is killed on the way out, pass or fail: a detached keeper outlives
/// the test that started it.
struct Root(tempfile::TempDir);

impl Root {
    fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        for session in kampr_keep::list(self.path()).unwrap_or_default() {
            if let Some(keeper) = session.keeper() {
                unsafe {
                    libc::kill(-(keeper.child as i32), libc::SIGKILL);
                    libc::kill(keeper.keeper as i32, libc::SIGKILL);
                }
            }
        }
    }
}

struct Desk {
    h: Harness,
    root: Root,
    socket: Socket,
}

impl Desk {
    async fn start(tag: &str) -> Self {
        let root = Root(
            tempfile::Builder::new()
                .prefix("kd")
                .tempdir_in("/tmp")
                .expect("a native root"),
        );
        let native: PathBuf = root.path().to_path_buf();
        let h = Harness::start_with(tag, move |config| {
            config.native.root = native.display().to_string();
            config.native.keeper = kampr_testkit::keeper().display().to_string();
            config.native.attach = kampr_testkit::kampr().display().to_string();
            config.native.launch = "detached".into();
        })
        .await;
        let token = h.token(Role::Full).await;
        let mut socket = h.connect(&token).await;
        until(&mut socket, "hello", 10).await;
        until(&mut socket, "herd", 10).await;
        Self { h, root, socket }
    }

    fn herdr(&self) -> kampr_herdr::Herdr {
        self.h._session.herdr()
    }

    async fn call(&self, method: &str, params: Value) -> Value {
        self.herdr()
            .call::<Value>(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    async fn manage(&mut self, op: Value) -> Value {
        let mut op = op;
        op["t"] = json!("manage");
        let name = op["op"].clone();
        send(&mut self.socket, op).await;
        let deadline = tokio::time::Instant::now() + SOON;
        while tokio::time::Instant::now() < deadline {
            let Some(message) = recv(&mut self.socket, Duration::from_secs(2)).await else {
                continue;
            };
            if message["t"] == "managed" && message["op"] == name {
                return message;
            }
        }
        panic!("no answer to a manage op");
    }

    /// A shell with no rc of the operator's, so nothing it prints races what a test types.
    async fn create(&mut self) -> String {
        let beside = self.h.pane_id();
        let ack = self
            .manage(json!({ "op": "native.create", "desk": true, "at": beside,
                            "args": ["bash", "--norc", "--noprofile"], "label": "deskrun" }))
            .await;
        assert_eq!(ack["ok"], true, "{ack}");
        let id = ack["id"].as_str().expect("the session's pane id").to_string();
        assert!(id.starts_with(&format!("{}/n:", self.h.node.node_id())), "{id}");
        id
    }

    fn entry(&self, id: &str) -> Option<PaneEntry> {
        self.h.node.herd().pane(id).cloned()
    }

    fn local(&self, herdr_pane: &str) -> String {
        format!("{}/{herdr_pane}", self.h.node.node_id())
    }

    async fn on_desk(&self, id: &str, not: Option<&str>) -> String {
        let deadline = tokio::time::Instant::now() + SOON;
        while tokio::time::Instant::now() < deadline {
            if let Some(desk) = self.entry(id).and_then(|e| e.on_desk)
                && Some(desk.as_str()) != not
            {
                return desk;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("{id} never showed on the desk: {:?}", self.entry(id));
    }

    async fn reported(&self, herdr_pane: &str) -> Value {
        let deadline = tokio::time::Instant::now() + SOON;
        let mut pane = Value::Null;
        while tokio::time::Instant::now() < deadline {
            if let Ok(got) = self
                .herdr()
                .call::<Value>("pane.get", json!({ "pane_id": herdr_pane }))
                .await
            {
                pane = got["pane"].clone();
                if pane["agent"] == "kampr" && pane["title"] == "deskrun" {
                    return pane;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("herdr never showed the session's report on {herdr_pane}: {pane}");
    }

    async fn foreground(&self, herdr_pane: &str) -> Vec<String> {
        self.herdr()
            .call::<Value>("pane.process_info", json!({ "pane_id": herdr_pane }))
            .await
            .map(|info| {
                info["process_info"]["foreground_processes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn screen_shows(&self, herdr_pane: &str, marker: &str) -> bool {
        let deadline = tokio::time::Instant::now() + SOON;
        while tokio::time::Instant::now() < deadline {
            if let Ok(read) = self
                .herdr()
                .call::<Value>(
                    "pane.read",
                    json!({ "pane_id": herdr_pane, "source": "recent", "format": "text", "lines": 200 }),
                )
                .await
                && read["read"]["text"].as_str().is_some_and(|t| t.contains(marker))
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    async fn detached(&self, id: &str) {
        let deadline = tokio::time::Instant::now() + SOON;
        while tokio::time::Instant::now() < deadline {
            if let Some(entry) = self.entry(id)
                && entry.detached == Some(true)
                && entry.on_desk.is_none()
            {
                assert!(
                    entry.exited.is_none(),
                    "the session died with its desk pane: {entry:?}"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("{id} never came off the desk: {:?}", self.entry(id));
    }

    async fn status(&self, herdr_pane: &str, want: &str) {
        let deadline = tokio::time::Instant::now() + SOON;
        let mut pane = Value::Null;
        while tokio::time::Instant::now() < deadline {
            pane = self.call("pane.get", json!({ "pane_id": herdr_pane })).await["pane"].clone();
            if pane["agent_status"] == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("herdr never showed {want} on {herdr_pane}: {pane}");
    }

    async fn close(&mut self, id: &str) {
        let ack = self.manage(json!({ "op": "native.close", "at": id })).await;
        assert_eq!(ack["ok"], true, "{ack}");
    }

    fn alive(&self) -> bool {
        kampr_keep::list(self.root.path())
            .unwrap_or_default()
            .iter()
            .all(|s| s.alive())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_desk_shows_one_pane_that_herdr_draws_as_an_agent_and_typing_reaches_both() {
    let mut d = Desk::start("native-desk").await;
    let id = d.create().await;
    let desk = d.on_desk(&id, None).await;

    let ours: Vec<String> =
        d.h.node
            .herd()
            .panes
            .iter()
            .filter(|p| p.node_id == d.h.node.node_id())
            .map(|p| p.id.clone())
            .collect();
    assert!(
        !ours.contains(&d.local(&desk)),
        "the host pane is in the herd as well: {ours:?}"
    );
    assert_eq!(
        ours.len(),
        2,
        "the shell the harness made, and the session: {ours:?}"
    );

    assert!(d.foreground(&desk).await.contains(&"kampr".to_string()));
    d.reported(&desk).await;
    d.status(&desk, "idle").await;
    let agents = d.call("agent.list", json!({})).await;
    assert!(
        agents["agents"]
            .as_array()
            .is_some_and(|a| a.iter().any(|a| a["pane_id"] == desk.as_str())),
        "{agents}"
    );

    send(
        &mut d.socket,
        json!({ "t": "input", "pane": id, "text": "echo DESK-$((6*7))-PASS\r" }),
    )
    .await;
    assert!(
        d.screen_shows(&desk, "DESK-42-PASS").await,
        "typing from the phone never reached herdr's screen through the attach"
    );

    let refused = d
        .manage(json!({ "op": "rename", "at": d.local(&desk), "label": "mine" }))
        .await;
    assert_eq!(
        (&refused["ok"], &refused["code"]),
        (&json!(false), &json!("bad_request")),
        "the host pane was addressed as a pane: {refused}"
    );

    let focused = d.manage(json!({ "op": "focus", "at": id })).await;
    assert_eq!(focused["ok"], true, "{focused}");
    let snapshot = d.call("session.snapshot", json!({})).await;
    assert_eq!(
        snapshot["snapshot"]["focused_pane_id"],
        desk.as_str(),
        "{snapshot}"
    );

    d.close(&id).await;
}

/// A host's own + has no pane to stand beside, and splitting whatever the desk has focused would
/// reshape somebody's layout for a session they did not ask to see there. It gets a workspace.
#[tokio::test(flavor = "multi_thread")]
async fn native_desk_with_nothing_to_stand_beside_opens_a_workspace_of_its_own() {
    let mut d = Desk::start("native-own-ws").await;
    let before = d.call("session.snapshot", json!({})).await["snapshot"].clone();
    let ack = d
        .manage(
            json!({ "op": "native.create", "desk": true, "node": d.h.node.node_id(),
                        "args": ["bash", "--norc", "--noprofile"], "label": "deskrun" }),
        )
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    let id = ack["id"].as_str().expect("the session's pane id").to_string();
    let desk = d.on_desk(&id, None).await;
    d.reported(&desk).await;

    let after = d.call("session.snapshot", json!({})).await["snapshot"].clone();
    let workspace = |snapshot: &Value, pane: &str| {
        snapshot["panes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["pane_id"] == pane)
            .map(|p| p["workspace_id"].clone())
    };
    let mine = workspace(&after, &desk).expect("the desk pane's workspace");
    assert!(
        before["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|w| w["workspace_id"] != mine),
        "{after}"
    );
    let count = |snapshot: &Value, key: &str| snapshot[key].as_array().map_or(0, Vec::len);
    assert_eq!(
        count(&after, "workspaces"),
        count(&before, "workspaces") + 1,
        "{after}"
    );
    assert_eq!(count(&after, "panes"), count(&before, "panes") + 1, "{after}");
    assert_eq!(
        after["focused_pane_id"], before["focused_pane_id"],
        "making a session moved the desk's focus"
    );

    d.close(&id).await;
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let now = d.call("session.snapshot", json!({})).await["snapshot"].clone();
        if count(&now, "workspaces") == count(&before, "workspaces") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the session's workspace outlived it: {now}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_desk_closing_the_herdr_pane_detaches_the_session_and_native_desk_hosts_it_again() {
    let mut d = Desk::start("native-detach").await;
    let id = d.create().await;
    let desk = d.on_desk(&id, None).await;

    d.call("pane.close", json!({ "pane_id": desk })).await;
    d.detached(&id).await;
    assert!(d.alive(), "closing the desk pane took the session with it");

    let ack = d.manage(json!({ "op": "native.desk", "at": id })).await;
    assert_eq!(ack["ok"], true, "{ack}");
    let again = d.on_desk(&id, None).await;
    d.reported(&again).await;
    assert!(!d.h.node.herd().panes.iter().any(|p| p.id == d.local(&again)));

    d.close(&id).await;
    let deadline = tokio::time::Instant::now() + SOON;
    while d.entry(&id).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the closed session never left the herd"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(kampr_keep::list(d.root.path()).unwrap_or_default().is_empty());
}

/// herdr takes `kampr attach` down whichever way it stops (#579) and runs `resume_argv` into the
/// restored pane (#583); the report does not come back with it, so the node sends it again.
#[tokio::test(flavor = "multi_thread")]
async fn native_desk_survives_herdr_stopping_and_being_killed_and_is_hosted_again_by_resume_argv() {
    let mut d = Desk::start("native-resume").await;
    let id = d.create().await;
    let desk = d.on_desk(&id, None).await;
    d.reported(&desk).await;
    send(
        &mut d.socket,
        json!({ "t": "input", "pane": id, "text": "echo BEFORE-RESTART\r" }),
    )
    .await;
    assert!(d.screen_shows(&desk, "BEFORE-RESTART").await);

    for kill in [false, true] {
        match kill {
            false => d.h._session.stop().await,
            true => d.h._session.kill().await,
        }
        d.detached(&id).await;
        assert!(
            d.alive(),
            "herdr going down (kill: {kill}) took the session with it"
        );
        d.h._session.respawn().await;
        let back = d.on_desk(&id, None).await;
        assert_eq!(back, desk, "herdr restored the pane under another id");
        d.reported(&back).await;
        assert!(
            d.screen_shows(&back, "BEFORE-RESTART").await,
            "the replay did not rebuild herdr's history (kill: {kill})"
        );
    }

    d.close(&id).await;
}
