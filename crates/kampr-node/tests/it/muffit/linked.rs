//! A terminal running muffit's TUI is that session's pane (docs/18): the daemon lists the TUI's
//! pid among the session's clients, so the terminal gets the conversation and the session has no
//! pane of its own beside it.

use super::fake::{Fake, TOKEN, session, text};
use super::{Client, Harness, NODE, eventually, pane};
use serde_json::{Value, json};
use std::path::Path;

const ONE_PIXEL: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap()
        .trim()
        .to_string()
}

fn tui(pid: u32) -> Value {
    json!([{ "kind": "tui", "pid": pid, "host": hostname(), "observing": false }])
}

impl Client {
    async fn manage(&mut self, op: Value) -> Value {
        let mut op = op;
        op["t"] = json!("manage");
        self.send(op).await;
        self.frame("the ack", |f| f["t"] == "managed").await
    }

    /// A terminal standing in for the TUI: it records its pid, then everything typed into it.
    async fn terminal(&mut self, dir: &Path) -> (String, u32) {
        let script = format!(
            "echo $$ > {0}/pid; stty raw -echo; exec cat > {0}/typed",
            dir.display()
        );
        let ack = self
            .manage(json!({ "op": "native.create", "node": NODE, "args": ["sh", "-c", script] }))
            .await;
        assert_eq!(ack["ok"], json!(true), "{ack}");
        let id = ack["id"].as_str().unwrap().to_string();
        let pid = eventually("the terminal's pid", || {
            std::fs::read_to_string(dir.join("pid"))
                .ok()
                .and_then(|p| p.trim().parse().ok())
        })
        .await;
        (id, pid)
    }
}

fn typed(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("typed")).unwrap_or_default()
}

#[tokio::test]
async fn the_terminal_running_the_tui_is_the_session_s_pane() {
    let fake = Fake::start(vec![]).await;
    fake.set_transcript(
        "t1",
        vec![text(1, "user", "hello"), text(2, "assistant", "[mock] hello")],
    );
    let h = Harness::start(&fake).await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = h.client().await;
    let (terminal, pid) = client.terminal(dir.path()).await;

    let mut row = session("t1", "waiting_user", Some("from the tui"));
    row["clients"] = tui(pid);
    fake.insert(row);
    let linked = client.pane(&terminal, |p| p["agent"] == "muffit").await;
    for (field, want) in [
        ("converses", json!(true)),
        ("has_conversation", json!(true)),
        ("title", json!("from the tui")),
        ("agent_status", json!("done")),
        ("provider", json!("native")),
    ] {
        assert_eq!(linked[field], want, "{field}: {linked}");
    }
    let settle = tokio::time::Instant::now() + std::time::Duration::from_millis(300);
    while tokio::time::Instant::now() < settle {
        client.pump(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        !client.panes.contains_key(&pane("t1")),
        "the session has a pane beside its terminal: {:?}",
        client.panes.keys()
    );

    client.watch(&terminal).await;
    let page = client.convo(&terminal).await;
    assert_eq!(super::ids(&page), ["m1", "m2"], "{page}");
    eventually("the node watches the session", || {
        fake.seen()
            .into_iter()
            .find(|s| s.path == "/sessions/t1/attach")
            .map(|s| assert_eq!(s.query.as_deref(), Some("observe=true")))
    })
    .await;

    let prompt = json!({ "action": "run `make`", "id": 4, "tool": "bash" });
    fake.update("t1", |row| row["permission"] = prompt.clone());
    fake.status(prompt, json!([]));
    client
        .frame("the prompt", |f| {
            f["t"] == "pending" && f["pane"] == terminal && f["question"] == "run `make`"
        })
        .await;
    client
        .send(json!({ "t": "answer", "pane": terminal, "key": "1" }))
        .await;
    let answered = eventually("an answer over rest", || {
        fake.posts("/sessions/t1/permission").into_iter().next()
    })
    .await;
    assert_eq!(answered, json!({ "id": 4, "answer": "allow" }));

    client.input(&terminal, "typed at the desk").await;
    client
        .send(json!({ "t": "paste", "pane": terminal, "b64": ONE_PIXEL, "name": "shot" }))
        .await;
    let got = eventually("the paste in the terminal", || {
        let got = typed(dir.path());
        got.contains("\u{1b}[201~").then_some(got)
    })
    .await;
    let (before, pasted) = got.split_once("\u{1b}[200~").expect("a bracketed paste");
    assert_eq!(before, "typed at the desk", "{got:?}");
    let path = pasted.strip_suffix("\u{1b}[201~").expect("closed");
    assert!(Path::new(path).is_file(), "{got:?}");
    assert!(path.ends_with(".png"), "{got:?}");
    assert!(
        fake.attach_frames("input").is_empty() && fake.posts("/sessions/t1/input").is_empty(),
        "typing goes to the TUI, never around it"
    );

    fake.update("t1", |row| row["clients"] = json!([]));
    client.pane(&pane("t1"), |_| true).await;
    client.pane(&terminal, |p| p["agent"] != "muffit").await;
}

#[tokio::test]
async fn a_tui_elsewhere_or_in_no_terminal_here_links_nothing() {
    let fake = Fake::start(vec![]).await;
    let h = Harness::start(&fake).await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = h.client().await;
    let (terminal, pid) = client.terminal(dir.path()).await;

    let mut elsewhere = session("t1", "running", Some("on the laptop"));
    elsewhere["clients"] =
        json!([{ "kind": "tui", "pid": pid, "host": "not-this-host", "observing": false }]);
    let mut watched = session("t2", "running", Some("watched"));
    watched["clients"] = json!([{ "kind": "kampr", "pid": pid, "observing": true }]);
    fake.insert(elsewhere);
    fake.insert(watched);
    client.pane(&pane("t1"), |_| true).await;
    client.pane(&pane("t2"), |_| true).await;
    let mine = client.pane(&terminal, |_| true).await;
    assert_ne!(mine["agent"], "muffit", "{mine}");
}

#[tokio::test]
async fn a_picture_pasted_into_a_background_session_goes_with_the_next_message() {
    let fake = Fake::start(vec![session("s1", "waiting_user", None)]).await;
    fake.set_transcript("s1", vec![]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;

    client
        .send(json!({ "t": "paste", "pane": id, "b64": ONE_PIXEL, "name": "shot" }))
        .await;
    client.input(&id, "what is this?").await;
    client.input(&id, "\r").await;
    let input = eventually("the message", || fake.attach_frames("input").into_iter().next()).await;
    assert_eq!(input["text"], "what is this?", "{input}");
    assert_eq!(
        input["images"],
        json!([{ "mime": "image/png", "data": ONE_PIXEL }]),
        "{input}"
    );
    let hello = &fake.attach_frames("hello")[0];
    assert_eq!(hello["client"], json!({ "kind": "kampr" }), "{hello}");
}

#[tokio::test]
async fn a_new_session_opens_in_a_terminal_running_the_tui() {
    let fake = Fake::start(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("muffit");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" > {0}/args\necho $$ > {0}/pid\nexec sleep 600\n",
            dir.path().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let h = Harness::beside_with(&fake.url, TOKEN, |config| {
        config.muffit.tui = script.display().to_string();
    })
    .await;
    let mut client = h.client().await;
    let work = dir.path().display().to_string();
    let ack = client
        .manage(
            json!({ "op": "muffit.create", "node": NODE, "cwd": work, "prompt": "fix it", "terminal": true }),
        )
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    assert_eq!(
        fake.posts("/sessions"),
        [json!({ "cwd": work, "prompt": "fix it" })]
    );
    let terminal = ack["id"].as_str().unwrap().to_string();
    assert!(terminal.starts_with(&format!("{NODE}/n:")), "{ack}");
    let args = eventually("the tui started", || {
        std::fs::read_to_string(dir.path().join("args")).ok()
    })
    .await;
    assert_eq!(args.trim(), "--resume new1");

    let pid: u32 = eventually("its pid", || {
        std::fs::read_to_string(dir.path().join("pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    })
    .await;
    fake.update("new1", |row| row["clients"] = tui(pid));
    client.pane(&terminal, |p| p["agent"] == "muffit").await;
    assert!(
        !client.panes.contains_key(&pane("new1")),
        "{:?}",
        client.panes.keys()
    );
}
