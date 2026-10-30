//! Native sessions over the wire: a node that keeps terminals itself, with no herdr anywhere.
//!
//! Every session here is a real keeper (`kampr-keep`, launched detached) under a root of the test's
//! own, so nothing reaches the operator's sessions and every keeper is killed on the way out.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

mod reflow;

const NODE: &str = "01JNATIVENODE";
const DEVICE: &str = "native";
const SOON: Duration = Duration::from_secs(15);

struct Harness {
    home: Option<tempfile::TempDir>,
    node: Arc<Node>,
    origin: String,
    port: u16,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.shutdown();
        self.server.abort();
        if let Some(home) = &self.home {
            kill_keepers(&home.path().join("native"));
        }
    }
}

fn kill_keepers(root: &Path) {
    let fleets = std::fs::read_dir(root.join("fleet"))
        .into_iter()
        .flatten()
        .flatten();
    let roots = std::iter::once(root.to_path_buf()).chain(fleets.map(|e| e.path()));
    for session in roots.flat_map(|r| kampr_keep::list(&r).unwrap_or_default()) {
        if let Some(keeper) = session.keeper() {
            unsafe {
                libc::kill(-(keeper.child as i32), libc::SIGKILL);
                libc::kill(keeper.keeper as i32, libc::SIGKILL);
            }
        }
    }
}

impl Harness {
    async fn start() -> Self {
        Self::start_in(Self::home()).await
    }

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("kn")
            .tempdir_in("/tmp")
            .expect("a home")
    }

    async fn start_in(home: tempfile::TempDir) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        Self::on(home, listener, port).await
    }

    /// A new node on the same state and the same native root, with every keeper left running.
    async fn restart(mut self) -> Self {
        let home = self.home.take().expect("a home");
        let port = self.port;
        drop(self);
        let listener = loop {
            match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => break listener,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        Self::on(home, listener, port).await
    }

    async fn on(home: tempfile::TempDir, listener: tokio::net::TcpListener, port: u16) -> Self {
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("native");

        config.muffit.enabled = Some(false);
        config.update.check = false;
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        config.fleet.path = "/usr/bin:/bin".into();
        config.native.root = home.path().join("native").display().to_string();
        config.native.keeper = kampr_testkit::keeper().display().to_string();
        config.native.launch = "detached".into();
        config.journals.home = home.path().display().to_string();
        config.node_id = NODE.into();
        config.save(&config_dir).expect("a config");

        let origin = config.origin();
        let node = Node::start(config, &state_dir).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        Self {
            home: Some(home),
            node,
            origin,
            port,
            server,
        }
    }

    fn root(&self) -> PathBuf {
        self.home.as_ref().expect("a home").path().join("native")
    }

    fn work(&self) -> PathBuf {
        let dir = self.home.as_ref().expect("a home").path().join("work");
        std::fs::create_dir_all(&dir).expect("a work dir");
        dir
    }

    async fn client(&self) -> Client {
        let pairing = self
            .node
            .auth
            .create_pairing(Role::Full, kampr_auth::Delivery::Console)
            .await
            .expect("a pairing");
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.expect("armed"));
        }
        let body = json!({ "code": pairing.code, "device_name": DEVICE }).to_string();
        let token = post(&self.origin, "/auth/pair", &body).await["token"]
            .as_str()
            .expect("a token")
            .to_string();
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("a websocket");
        Client::over(socket, token)
    }
}

pub(crate) struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    token: String,
    nodes: HashMap<String, Value>,
    panes: HashMap<String, Value>,
    grids: HashMap<String, (Value, BTreeMap<u64, Value>)>,
    pub(crate) resets: HashMap<String, Value>,
    pub(crate) history: HashMap<String, Vec<String>>,
    others: Vec<Value>,
}

impl Client {
    pub(crate) fn over(socket: WebSocketStream<MaybeTlsStream<TcpStream>>, token: String) -> Self {
        Self {
            socket,
            token,
            nodes: HashMap::new(),
            panes: HashMap::new(),
            grids: HashMap::new(),
            resets: HashMap::new(),
            history: HashMap::new(),
            others: Vec::new(),
        }
    }

    pub(crate) async fn send(&mut self, value: Value) {
        self.socket
            .send(tungstenite::Message::Text(value.to_string().into()))
            .await
            .expect("a send");
    }

    async fn pump(&mut self, within: Duration) -> bool {
        let Ok(Some(Ok(tungstenite::Message::Text(text)))) =
            tokio::time::timeout(within, self.socket.next()).await
        else {
            return false;
        };
        let value: Value = serde_json::from_str(&text).expect("a frame");
        let pane = value["pane"].as_str().unwrap_or_default().to_string();
        match value["t"].as_str().unwrap_or_default() {
            "herd" => {
                self.panes.clear();
                for n in value["nodes"].as_array().into_iter().flatten() {
                    self.nodes.insert(n["id"].as_str().unwrap().into(), n.clone());
                }
                for p in value["panes"].as_array().into_iter().flatten() {
                    self.panes.insert(p["id"].as_str().unwrap().into(), p.clone());
                }
            }
            "herd.patch" => {
                for list in ["added", "changed"] {
                    for n in value[list]["nodes"].as_array().into_iter().flatten() {
                        self.nodes.insert(n["id"].as_str().unwrap().into(), n.clone());
                    }
                    for p in value[list]["panes"].as_array().into_iter().flatten() {
                        self.panes.insert(p["id"].as_str().unwrap().into(), p.clone());
                    }
                }
                for id in value["removed_ids"].as_array().into_iter().flatten() {
                    self.panes.remove(id.as_str().unwrap());
                }
            }
            "grid.reset" => {
                let rows = value["rows_data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| (r["row"].as_u64().unwrap(), r["runs"].clone()))
                    .collect();
                self.grids.insert(pane.clone(), (value["cursor"].clone(), rows));
                self.resets.insert(pane, value);
            }
            "grid.patch" => {
                let grid = self.grids.entry(pane).or_default();
                grid.0 = value["cursor"].clone();
                for r in value["rows"].as_array().into_iter().flatten() {
                    grid.1.insert(r["row"].as_u64().unwrap(), r["runs"].clone());
                }
            }
            "scrollback" => {
                let rows = self.history.entry(pane).or_default();
                for r in value["rows"].as_array().into_iter().flatten() {
                    rows.push(text_of(&r["runs"]));
                }
            }
            _ => self.others.push(value),
        }
        true
    }

    pub(crate) async fn until<T>(&mut self, what: &str, mut found: impl FnMut(&mut Self) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            if let Some(t) = found(self) {
                return t;
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!left.is_zero(), "{what} never happened; herd: {:?}", self.panes);
            self.pump(left.min(Duration::from_millis(200))).await;
        }
    }

    pub(crate) async fn reply(&mut self, tag: &str) -> Value {
        self.until(tag, |c| {
            let at = c.others.iter().position(|v| v["t"] == tag)?;
            Some(c.others.remove(at))
        })
        .await
    }

    async fn manage(&mut self, op: Value) -> Value {
        let mut op = op;
        op["t"] = json!("manage");
        self.send(op).await;
        self.reply("managed").await
    }

    async fn create(&mut self, args: &[&str], extra: Value) -> String {
        let mut op = json!({ "op": "native.create", "node": NODE, "args": args });
        for (k, v) in extra.as_object().into_iter().flatten() {
            op[k] = v.clone();
        }
        let ack = self.manage(op).await;
        assert_eq!(ack["ok"], json!(true), "{ack}");
        let id = ack["id"].as_str().expect("an id").to_string();
        assert!(id.starts_with(&format!("{NODE}/n:")), "{id}");
        self.pane(&id, |_| true).await;
        id
    }

    pub(crate) async fn pane(&mut self, id: &str, want: impl Fn(&Value) -> bool) -> Value {
        self.until(&format!("{id} as wanted"), |c| {
            c.panes.get(id).filter(|p| want(p)).cloned()
        })
        .await
    }

    async fn watch(&mut self, id: &str, scrollback: bool) {
        self.send(json!({ "t": "watch", "pane": id, "scrollback": scrollback }))
            .await;
    }

    async fn gone(&mut self, id: &str) {
        self.until(&format!("{id} gone from the herd"), |c| {
            (!c.panes.contains_key(id)).then_some(())
        })
        .await;
    }

    async fn type_text(&mut self, id: &str, text: &str) {
        self.send(json!({ "t": "input", "pane": id, "text": text })).await;
    }

    fn screen(&self, id: &str) -> Vec<String> {
        self.grids
            .get(id)
            .map(|(_, rows)| rows.values().map(text_of).collect())
            .unwrap_or_default()
    }

    pub(crate) async fn shows(&mut self, id: &str, needle: &str) -> Vec<String> {
        let id = id.to_string();
        self.until(&format!("{needle:?} on screen"), |c| {
            let screen = c.screen(&id);
            screen.iter().any(|r| r.contains(needle)).then_some(screen)
        })
        .await
    }
}

fn text_of(runs: &Value) -> String {
    runs.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["x"].as_str())
        .collect::<String>()
        .trim_end()
        .to_string()
}

async fn post(origin: &str, path: &str, body: &str) -> Value {
    let head = format!(
        "POST {path} HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    http(origin, &head, body).await
}

async fn get(origin: &str, path: &str, token: &str) -> Value {
    http(
        origin,
        &format!("GET {path} HTTP/1.1\r\nAuthorization: Bearer {token}\r\n"),
        "",
    )
    .await
}

async fn http(origin: &str, head: &str, body: &str) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = origin.trim_start_matches("http://");
    let (host, port) = rest.split_once(':').unwrap_or((rest, "80"));
    let request = format!("{head}Host: {host}:{port}\r\nConnection: close\r\n\r\n{body}");
    let mut stream = TcpStream::connect((host, port.parse::<u16>().unwrap()))
        .await
        .expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    serde_json::from_str(body.trim()).unwrap_or(Value::Null)
}

/// The herd lets a session go once its spec is deleted, which is before the rest of its directory.
fn removed(path: &Path) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while path.exists() {
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn session_of(root: &Path, id: &str) -> kampr_keep::SessionDir {
    let local = id.rsplit_once("/n:").expect("a native id").1;
    kampr_keep::SessionDir::at(root.join(local))
}

fn log_end(session: &kampr_keep::SessionDir) -> u64 {
    kampr_keep::SizedReader::at(session.path(), 0)
        .and_then(|r| r.end())
        .unwrap_or(0)
}

#[tokio::test]
async fn a_session_made_over_the_wire_is_a_pane_that_echoes_what_is_typed() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["cat"], json!({ "label": "echo" })).await;

    let pane = client.pane(&id, |p| p["cmd"] == "cat").await;
    assert_eq!(pane["provider"], "native", "{pane}");
    assert_eq!(pane["session_id"], id.rsplit_once("/n:").unwrap().1, "{pane}");
    assert_eq!(pane["detached"], json!(true), "{pane}");
    assert_eq!(pane["label"], "echo", "{pane}");
    assert_eq!(
        (pane["cols"].clone(), pane["rows"].clone()),
        (json!(80), json!(24)),
        "{pane}"
    );
    assert!(pane.get("exited").is_none(), "{pane}");
    assert!(pane.get("on_desk").is_none(), "{pane}");

    client.watch(&id, false).await;
    client.type_text(&id, "hello\r").await;
    let screen = client.shows(&id, "hello").await;
    let screen = match screen.iter().filter(|r| *r == "hello").count() {
        2 => screen,
        _ => {
            client
                .until("the tty's echo and cat's copy", |c| {
                    let s = c.screen(&id);
                    (s.iter().filter(|r| *r == "hello").count() == 2).then_some(s)
                })
                .await
        }
    };
    assert_eq!(&screen[..2], &["hello", "hello"]);

    client
        .send(json!({ "t": "input", "pane": id, "keys": ["h", "i", "Enter"] }))
        .await;
    client
        .until("keys turned into bytes", |c| {
            (c.screen(&id).iter().filter(|r| *r == "hi").count() == 2).then_some(())
        })
        .await;
}

/// A shell is never what a pane is running, as on a herdr pane (`procfs.rs`): a client reads a
/// `cmd` as "something other than the shell is listening", and scrolls it with cursor keys that a
/// shell's line editor takes as its history.
#[tokio::test]
async fn a_shell_at_its_prompt_names_no_job_and_the_job_it_runs_is_named() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["sh", "-i"], json!({})).await;
    client.watch(&id, false).await;
    client.shows(&id, "$").await;
    client.pane(&id, |p| p["cmd"].is_null()).await;

    client.type_text(&id, "cat\r").await;
    client.pane(&id, |p| p["cmd"] == "cat").await;
    client.type_text(&id, "\x04").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    eprintln!("{}", processes_of(&session_of(&h.root(), &id)));
    client.pane(&id, |p| p["cmd"].is_null()).await;
}

fn processes_of(session: &kampr_keep::SessionDir) -> String {
    use kampr_proc::Table;
    let table = kampr_proc::here();
    let Some(child) = session.keeper().map(|k| k.child) else {
        return "no keeper".into();
    };
    let mut seen = vec![child];
    let mut at = 0;
    let mut out = String::new();
    while let Some(&pid) = seen.get(at) {
        out += &format!(
            "{pid}: comm={:?} argv={:?} tpgid={:?} tty={:?} children={:?}\n",
            table.comm(pid),
            table.argv(pid),
            table.tpgid(pid),
            table.tty(pid),
            table.children(pid)
        );
        seen.extend(table.children(pid));
        at += 1;
    }
    out
}

#[tokio::test]
async fn a_size_asked_for_reaches_the_pty_and_a_matched_one_is_put_back() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["sh"], json!({})).await;
    client.watch(&id, false).await;

    let sized = client
        .manage(json!({ "op": "pane.size", "at": id, "cols": 100, "rows": 30 }))
        .await;
    assert_eq!(sized["ok"], json!(true), "{sized}");
    assert_eq!(sized["kept"], json!(true), "{sized}");
    let pane = client.pane(&id, |p| p["cols"] == 100 && p["rows"] == 30).await;
    assert_eq!(pane["size_holder"], format!("phone:{DEVICE}"), "{pane}");
    client.type_text(&id, "stty size\r").await;
    client.shows(&id, "30 100").await;

    for (mode, cols, rows) in [("match", 120, 40), ("grow", 130, 20)] {
        let held = client
            .manage(json!({ "op": "pane.size", "at": id, "cols": cols, "rows": rows, "mode": mode }))
            .await;
        assert_eq!(held["ok"], json!(true), "{mode}: {held}");
        let lease = held["lease"]
            .as_u64()
            .unwrap_or_else(|| panic!("{mode}: no lease in {held}"));
        let (want_cols, want_rows) = match mode {
            "grow" => (130, 30),
            _ => (cols, rows),
        };
        client
            .pane(&id, |p| p["cols"] == want_cols && p["rows"] == want_rows)
            .await;
        let released = client
            .manage(json!({ "op": "pane.size", "at": id, "mode": "release", "lease": lease }))
            .await;
        assert_eq!(released["ok"], json!(true), "{mode}: {released}");
        client.pane(&id, |p| p["cols"] == 100 && p["rows"] == 30).await;
    }
    client.type_text(&id, "clear; stty size\r").await;
    client
        .until("the size put back", |c| {
            let s = c.screen(&id);
            (s.iter().any(|r| r == "30 100") && !s.iter().any(|r| r.contains("40 120"))).then_some(())
        })
        .await;
}

/// A desk's attach that started under a view's hold leaves the size alone, so the size the view
/// found is the spawn's. Letting go puts the session at the attach's terminal, which is still
/// showing it, rather than at a size nobody is looking at.
#[tokio::test]
async fn a_released_match_goes_to_the_desk_still_showing_the_session() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["sh"], json!({})).await;
    client.watch(&id, false).await;
    let held = client
        .manage(json!({ "op": "pane.size", "at": id, "cols": 150, "rows": 45, "mode": "match" }))
        .await;
    assert_eq!(held["ok"], json!(true), "{held}");
    client.pane(&id, |p| p["cols"] == 150 && p["rows"] == 45).await;

    let desk = kampr_keep::register_host(
        &session_of(&h.root(), &id),
        kampr_keep::Host {
            pid: std::process::id(),
            herdr_pane: None,
            herdr_socket: None,
            cols: 119,
            rows: 40,
            answers_from: None,
        },
    )
    .expect("a desk");
    let released = client
        .manage(json!({ "op": "pane.size", "at": id, "mode": "release", "lease": held["lease"] }))
        .await;
    assert_eq!(released["ok"], json!(true), "{released}");
    let put = client.pane(&id, |p| p["cols"] == 119 && p["rows"] == 40).await;
    assert_eq!(
        put["size_holder"],
        format!("attach:{}", std::process::id()),
        "{put}"
    );
    drop(desk);
}

#[tokio::test]
async fn history_and_search_reach_rows_long_gone_from_the_screen() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client
        .create(&["sh", "-c", "echo deepmark; seq 1 5000; exec cat"], json!({}))
        .await;
    client.watch(&id, true).await;
    client.shows(&id, "5000").await;

    client
        .until("history past the screen", |c| {
            let rows = c.history.get(&id)?;
            (rows.iter().any(|r| r == "4500") && rows.iter().any(|r| r == "4977")).then_some(())
        })
        .await;
    let pane = client
        .pane(&id, |p| p["scrollback_rows"].as_u64() > Some(4900))
        .await;
    assert!(pane["scrollback_rows"].as_u64() <= Some(5001), "{pane}");

    client
        .send(json!({ "t": "find", "pane": id, "query": "deepmark" }))
        .await;
    let found = client.reply("find").await;
    let hits = found["matches"].as_array().expect("matches").clone();
    assert_eq!(found["total"], json!(1), "{found}");
    assert!(hits[0]["from_bottom"].as_u64() > Some(4990), "{found}");
    assert_eq!(hits[0]["text"], "deepmark", "{found}");

    client
        .send(json!({ "t": "find", "pane": id, "query": "7" }))
        .await;
    let many = client.reply("find").await;
    assert!(many["total"].as_u64() > Some(1000), "{many}");
    assert_eq!(many["matches"].as_array().map(Vec::len), Some(32), "{many}");
}

#[tokio::test]
async fn a_session_closed_on_purpose_leaves_the_herd_once_it_ends() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["cat", "/dev/stdin"], json!({})).await;
    let pane = client.pane(&id, |p| p["cmd"] == "cat").await;
    assert!(
        pane.get("argv").is_none(),
        "a command line is off the wire by default: {pane}"
    );
    assert!(pane["label"].is_null(), "and so is a label made of one: {pane}");

    let refused = client.manage(json!({ "op": "native.forget", "at": id })).await;
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap_or_default()
            .contains("still running"),
        "{refused}"
    );

    let closed = client.manage(json!({ "op": "native.close", "at": id })).await;
    assert_eq!(closed["ok"], json!(true), "{closed}");
    client.gone(&id).await;
    assert!(removed(session_of(&h.root(), &id).path()));
}

/// A program that finished cleanly is gone, as a herdr pane is when its shell exits. One that
/// failed is kept, so its exit status can be read, until somebody forgets it.
#[tokio::test]
async fn a_clean_exit_leaves_the_herd_and_a_failed_one_stays_until_forgotten() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let clean = client.create(&["sh", "-c", "read x; exit 0"], json!({})).await;
    let failed = client.create(&["sh", "-c", "read x; exit 3"], json!({})).await;
    for id in [&clean, &failed] {
        client.pane(id, |p| p["cmd"].is_null()).await;
        client.type_text(id, "\r").await;
    }
    client.gone(&clean).await;
    assert!(removed(session_of(&h.root(), &clean).path()));

    let pane = client.pane(&failed, |p| p.get("exited").is_some()).await;
    assert_eq!(pane["exited"], json!({ "code": 3 }), "{pane}");
    assert_ne!(pane["agent_status"], "working", "{pane}");
    let forgot = client
        .manage(json!({ "op": "native.forget", "at": failed }))
        .await;
    assert_eq!(forgot["ok"], json!(true), "{forgot}");
    client.gone(&failed).await;
    assert!(removed(session_of(&h.root(), &failed).path()));
}

#[tokio::test]
async fn a_session_that_ended_cleanly_while_no_node_ran_is_gone_when_one_starts() {
    let home = Harness::home();
    let root = home.path().join("native");
    let ended = |code: i32| {
        let spec = kampr_keep::Spec::new(vec!["sh".into(), "-c".into(), format!("exit {code}")]);
        let session = kampr_keep::spawn(
            &root,
            spec,
            kampr_keep::Launch::Detached,
            &kampr_testkit::keeper(),
        )
        .expect("a keeper");
        let deadline = std::time::Instant::now() + SOON;
        while session.exit().is_none() {
            assert!(std::time::Instant::now() < deadline, "exit {code} never ended");
            std::thread::sleep(Duration::from_millis(50));
        }
        format!("{NODE}/n:{}", session.id())
    };
    let (clean, failed) = (ended(0), ended(3));

    let h = Harness::start_in(home).await;
    let mut client = h.client().await;
    client.pane(&failed, |p| p.get("exited").is_some()).await;
    assert!(!client.panes.contains_key(&clean), "{:?}", client.panes.keys());
    assert!(removed(session_of(&h.root(), &clean).path()));
}

#[tokio::test]
async fn output_is_working_and_its_end_is_done_until_somebody_types() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client
        .create(
            &[
                "sh",
                "-c",
                "sleep 1; i=0; while [ $i -lt 30 ]; do echo $i; i=$((i+1)); sleep 0.1; done; exec cat",
            ],
            json!({}),
        )
        .await;
    client.pane(&id, |p| p["agent_status"] == "working").await;
    client.pane(&id, |p| p["agent_status"] == "done").await;
    client.type_text(&id, "x").await;
    client.pane(&id, |p| p["agent_status"] == "idle").await;
}

/// A shell redraws its prompt for every new size, and two screens of different sizes taking turns
/// at a session resize it every time either is looked at: an idle shell read as working, then
/// done, over and over.
#[tokio::test]
async fn a_redraw_for_a_new_size_is_not_work() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client
        .create(
            &[
                "sh",
                "-c",
                "trap 'printf \"\\r$ REDRAWN \"' WINCH; sleep 1; printf '$ '; while :; do sleep 0.05; done",
            ],
            json!({}),
        )
        .await;
    client.watch(&id, false).await;
    client.pane(&id, |p| p["agent_status"] == "done").await;
    client.type_text(&id, "x").await;
    client.pane(&id, |p| p["agent_status"] == "idle").await;
    for (cols, rows) in [(100, 30), (90, 28), (110, 32)] {
        let ack = client
            .manage(json!({ "op": "pane.size", "at": id, "cols": cols, "rows": rows }))
            .await;
        assert_eq!(ack["ok"], json!(true), "{ack}");
        let quiet = tokio::time::Instant::now() + Duration::from_millis(700);
        while let left = quiet.saturating_duration_since(tokio::time::Instant::now())
            && !left.is_zero()
        {
            client.pump(left).await;
            let status = &client.panes[&id]["agent_status"];
            assert_eq!(status, "idle", "resized to {cols}x{rows}");
        }
    }
    assert!(
        client.screen(&id).iter().any(|r| r.contains("REDRAWN")),
        "the shell never redrew: {:?}",
        client.screen(&id)
    );
}

/// **Answered live, never from the replay, and never over a host.** The query in the log was asked
/// of whoever was there when it was written; a node restarting and answering it again types the
/// answer into the program that is reading the terminal today, which here is a `cat` that would
/// write it straight back into the log.
#[tokio::test]
async fn a_query_is_answered_once_by_the_node_and_not_at_all_while_a_host_answers() {
    let script = "stty -echo -icanon min 0 time 20; while [ ! -e go ]; do sleep 0.05; done; \
                  printf '\\033[c'; r=$(dd bs=1 count=9 2>/dev/null); \
                  printf 'GOT[%s]\\n' \"$(printf %s \"$r\" | tr '\\033' E)\"; stty min 1 time 0; exec cat";
    let mut h = Harness::start().await;
    for hosted in [false, true] {
        let work = h.work().join(if hosted { "hosted" } else { "bare" });
        std::fs::create_dir_all(&work).unwrap();
        let mut client = h.client().await;
        let id = client
            .create(
                &["sh", "-c", script],
                json!({ "cwd": work.display().to_string() }),
            )
            .await;
        let session = session_of(&h.root(), &id);
        let _host = hosted.then(|| {
            kampr_keep::register_host(
                &session,
                kampr_keep::Host {
                    pid: std::process::id(),
                    herdr_pane: Some("w1:p3".into()),
                    herdr_socket: None,
                    cols: 80,
                    rows: 24,
                    answers_from: None,
                },
            )
            .expect("a host")
        });
        if hosted {
            let pane = client.pane(&id, |p| p["detached"] == json!(false)).await;
            assert_eq!(pane["on_desk"], "w1:p3", "{pane}");
        }
        client.watch(&id, false).await;
        std::fs::write(work.join("go"), b"").unwrap();
        let want = if hosted { "GOT[]" } else { "GOT[E[?62;22c]" };
        let screen = client.shows(&id, "GOT[").await;
        assert!(screen.iter().any(|r| r == want), "hosted {hosted}: {screen:?}");

        if !hosted {
            drop(client);
            let before = log_end(&session);
            h = h.restart().await;
            let mut client = h.client().await;
            client.pane(&id, |_| true).await;
            client.watch(&id, false).await;
            client.shows(&id, "GOT[").await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert_eq!(
                log_end(&session),
                before,
                "the restarted node answered a replayed query"
            );
        }
    }
}

/// A `claude` that negotiates `OSC 7501` as 2.1.308 does (#600) and then reports each state as the
/// test lets it, while its marker says `waiting` throughout: every state but `blocked` shows that
/// the harness's own channel, not the marker, is the status.
fn osc7501_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("claude");
    let script = r#"#!/bin/sh
home=$1
start=$(cut -d' ' -f22 /proc/$$/stat 2>/dev/null || LC_ALL=C TZ=UTC ps -o lstart= -p $$ | sed 's/^ *//; s/ *$//')
mkdir -p "$home/.claude/sessions"
printf '{"pid":%s,"sessionId":"77777777-7777-4777-8777-777777777777","cwd":"%s","procStart":"%s","version":"2.1.308","kind":"interactive","entrypoint":"cli","status":"waiting"}' $$ "$(pwd -P)" "$start" > "$home/.claude/sessions/$$.json"
stty -echo -icanon min 0 time 30
while [ ! -e go ]; do sleep 0.05; done
printf '\033]7501;?\007\033[c'
r=$(dd bs=1 count=19 2>/dev/null)
printf 'GOT[%s]\n' "$(printf %s "$r" | tr '\033' E)"
for s in working blocked error done; do
  while [ ! -e "$s" ]; do sleep 0.05; done
  case $s in
    blocked) extra=':kind=permission:msg=YXBwcm92ZSBCYXNoOiB0b3VjaCB4' ;;
    error) extra=':msg=QVBJIEVycm9yOiA0MDA=' ;;
    *) extra= ;;
  esac
  printf '\033]7501;state=%s:app=claude-code%s\007%s\n' "$s" "$extra" "$s"
done
while [ ! -e end ]; do sleep 0.05; done
stty min 1 time 0
exec cat
"#;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// **Claude's own channel is the status.** The node answers `OSC 7501;?` in stream order, so ahead
/// of DA1's answer (#600); each state maps; and a restarted node rebuilds the last one from the log
/// without answering the query again.
#[tokio::test]
async fn a_harness_reporting_over_osc_7501_is_the_status_and_a_restart_keeps_it() {
    let mut h = Harness::start().await;
    let home = h.home.as_ref().unwrap().path().to_path_buf();
    let work = h.work();
    let claude = osc7501_claude(&work);
    let mut client = h.client().await;
    let id = client
        .create(
            &[&claude.display().to_string(), &home.display().to_string()],
            json!({ "cwd": work.display().to_string() }),
        )
        .await;
    client.pane(&id, |p| p["agent"] == "claude").await;
    client.watch(&id, false).await;
    std::fs::write(work.join("go"), b"").unwrap();
    let screen = client.shows(&id, "GOT[").await;
    assert!(
        screen.iter().any(|r| r == "GOT[E]7501;?E\\E[?62;22c]"),
        "{screen:?}"
    );
    for (step, want) in [
        ("working", "working"),
        ("blocked", "blocked"),
        ("error", "idle"),
        ("done", "done"),
    ] {
        std::fs::write(work.join(step), b"").unwrap();
        client.shows(&id, step).await;
        let pane = client.pane(&id, |p| p["agent_status"] == want).await;
        assert_eq!(pane["agent"], "claude", "{step}: {pane}");
    }

    std::fs::write(work.join("end"), b"").unwrap();
    client.pane(&id, |p| p["cmd"] == "cat").await;
    drop(client);
    let session = session_of(&h.root(), &id);
    let before = log_end(&session);
    h = h.restart().await;
    let mut client = h.client().await;
    client.pane(&id, |p| p["agent_status"] == "done").await;
    client.watch(&id, false).await;
    client.shows(&id, "GOT[").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        log_end(&session),
        before,
        "the restarted node answered a replayed query"
    );
    client.pane(&id, |p| p["agent_status"] == "done").await;
    client.type_text(&id, "x").await;
    client.pane(&id, |p| p["agent_status"] == "idle").await;

    drop(client);
    h = h.restart().await;
    let mut client = h.client().await;
    client.pane(&id, |p| p["cmd"] == "cat").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pane = client.pane(&id, |p| p["cmd"] == "cat").await;
    assert_eq!(
        pane["agent_status"], "idle",
        "a done already read came back after a restart: {pane}"
    );
}

/// Keys typed by name reach a program that pushed kitty keyboard flags in the encoding it asked
/// for, so Esc is no longer the start of an escape and shift+Enter is not Enter (#594).
#[tokio::test]
async fn named_keys_are_encoded_for_the_kitty_flags_the_program_pushed() {
    let script = "stty -echo raw; printf '\\033[>1uREADY\\r\\n'; \
                  r=$(dd bs=1 count=19 2>/dev/null | od -An -tx1 | tr -d ' \\n'); \
                  printf 'GOT[%s]\\r\\n' \"$r\"; exec cat";
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["sh", "-c", script], json!({})).await;
    client.watch(&id, false).await;
    client.shows(&id, "READY").await;
    client
        .send(json!({ "t": "input", "pane": id, "keys": ["Escape", "shift+Enter", "ctrl+c"] }))
        .await;
    let screen = client.shows(&id, "GOT[").await;
    assert!(
        screen
            .iter()
            .any(|r| r == "GOT[1b5b3237751b5b31333b32751b5b39393b3575]"),
        "{screen:?}"
    );
}

/// **The same screen, cell for cell and style for style**, from a node that never saw it drawn.
#[tokio::test]
async fn a_node_restart_rebuilds_the_same_screen_from_the_log() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client
        .create(
            &[
                "sh",
                "-c",
                "printf 'one\\ntwo\\n\\033[1;31mred\\033[0m \\033[44mblue\\033[0m\\n'; seq 1 40; printf 'end\\033[3;5H'; exec cat",
            ],
            json!({}),
        )
        .await;
    client.watch(&id, false).await;
    client.shows(&id, "end").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    while client.pump(Duration::from_millis(100)).await {}
    let before = client.grids.get(&id).cloned().expect("a grid");
    drop(client);

    let h = h.restart().await;
    let mut client = h.client().await;
    client.pane(&id, |_| true).await;
    client.watch(&id, false).await;
    client.shows(&id, "end").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    while client.pump(Duration::from_millis(100)).await {}
    let after = client.grids.get(&id).cloned().expect("a grid");
    assert_eq!(after, before);
}

#[tokio::test]
async fn native_ops_refuse_what_they_cannot_do_and_never_ask_herdr() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client.create(&["cat"], json!({})).await;
    for (op, needle) in [
        (
            json!({ "op": "native.create", "node": NODE, "cols": 100 }),
            "both",
        ),
        (json!({ "op": "focus", "at": id }), "not on the desk"),
        (
            json!({ "op": "native.close", "at": format!("{NODE}/n:nosuchsession") }),
            "not a pane",
        ),
    ] {
        let ack = client.manage(op.clone()).await;
        assert_eq!(ack["ok"], json!(false), "{op}: {ack}");
        assert!(
            ack["message"].as_str().unwrap_or_default().contains(needle),
            "{op}: {ack}"
        );
        assert!(
            !ack["message"].as_str().unwrap_or_default().contains("herdr"),
            "{op} reached for herdr: {ack}"
        );
    }

    let sessions = kampr_keep::list(&h.root()).expect("the root").len();
    for op in [
        json!({ "op": "native.create", "node": NODE, "desk": true, "at": format!("{NODE}/w1") }),
        json!({ "op": "native.desk", "at": id }),
    ] {
        let ack = client.manage(op.clone()).await;
        assert_eq!(
            (&ack["ok"], &ack["code"]),
            (&json!(false), &json!("herdr_unavailable")),
            "{op}: {ack}"
        );
    }
    assert_eq!(
        kampr_keep::list(&h.root()).expect("the root").len(),
        sessions,
        "a session meant for a desk that could not be reached was made anyway"
    );

    let renamed = client
        .manage(json!({ "op": "rename", "at": id, "label": "renamed" }))
        .await;
    assert_eq!(renamed["ok"], json!(true), "{renamed}");
    client.pane(&id, |p| p["label"] == "renamed").await;
    let closed = client.manage(json!({ "op": "close", "at": id })).await;
    assert_eq!(closed["ok"], json!(true), "{closed}");
    client.gone(&id).await;
}

/// The harness is read off the PTY's foreground process, and a dialog on an agent's screen is a
/// session waiting on somebody once its output has stopped.
#[tokio::test]
async fn an_agent_is_known_by_its_process_and_its_dialog_is_blocked() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::start().await;
    let claude = h.work().join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\nprintf 'Allow this command?\\n\\n1) Yes\\n2) No\\n'\nwhile :; do sleep 1; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut client = h.client().await;
    let id = client.create(&[&claude.display().to_string()], json!({})).await;
    let pane = client.pane(&id, |p| p["agent_status"] == "blocked").await;
    assert_eq!(pane["agent"], "claude", "{pane}");
}

/// A `claude` that publishes itself the way 2.1.239 does: `~/.claude/sessions/<pid>.json` with its
/// own `procStart`, and a transcript under the project directory its working directory names.
fn fake_claude(dir: &Path, status: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("claude");
    let script = format!(
        r#"#!/bin/sh
home=$1; id=$2
cwd=$(pwd -P)
start=$(cut -d' ' -f22 /proc/$$/stat 2>/dev/null || LC_ALL=C TZ=UTC ps -o lstart= -p $$ | sed 's/^ *//; s/ *$//')
project="$home/.claude/projects/$(printf %s "$cwd" | tr / -)"
mkdir -p "$project" "$home/.claude/sessions"
now=$(date -u +%Y-%m-%dT%H:%M:%S.999Z)
printf '{{"type":"user","uuid":"%s-1","cwd":"%s","timestamp":"%s","message":{{"role":"user","content":"hello"}}}}\n' "$id" "$cwd" "$now" > "$project/$id.jsonl"
printf '{{"type":"assistant","uuid":"%s-2","cwd":"%s","timestamp":"%s","message":{{"content":[{{"type":"text","text":"NATIVE TURN"}}]}}}}\n' "$id" "$cwd" "$now" >> "$project/$id.jsonl"
printf '{{"type":"ai-title","aiTitle":"the native rewrite","sessionId":"%s"}}\n' "$id" >> "$project/$id.jsonl"
printf '{{"pid":%s,"sessionId":"%s","cwd":"%s","procStart":"%s","version":"2.1.239","kind":"interactive","entrypoint":"cli","status":"{status}"}}' $$ "$id" "$cwd" "$start" > "$home/.claude/sessions/$$.json"
printf 'claude is up\n'
while :; do sleep 1; done
"#
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// **The harness is two shells down**, as it is whenever a wrapper script starts it: the session's
/// own child is `sh`, and only the process at the bottom wrote the marker. What the marker says (`waiting`)
/// is nowhere on the screen, so a blocked pane proves the marker was found by that pid.
#[tokio::test]
async fn a_claude_under_a_shell_has_its_conversation_title_and_marker_served() {
    let h = Harness::start().await;
    let home = h.home.as_ref().unwrap().path().to_path_buf();
    let claude = fake_claude(&h.work(), "waiting");
    let id = "66666666-6666-4666-8666-666666666666";
    let line = format!(
        "sh -c '{} {} {id}; :'; exec cat",
        claude.display(),
        home.display()
    );
    let mut client = h.client().await;
    let pane = client
        .create(
            &["sh", "-c", &line],
            json!({ "cwd": h.work().display().to_string() }),
        )
        .await;

    let entry = client
        .pane(&pane, |p| {
            p["agent"] == "claude"
                && p["has_conversation"] == json!(true)
                && p["title"] == "the native rewrite"
        })
        .await;
    let entry = match entry["agent_status"] == "blocked" {
        true => entry,
        false => client.pane(&pane, |p| p["agent_status"] == "blocked").await,
    };
    assert_eq!(entry["converses"], json!(true), "{entry}");

    // Somebody else in the same directory, and newer: only the pid tells the two apart.
    let cwd = h.work().canonicalize().unwrap().display().to_string();
    let decoy = json!({
        "type": "assistant", "uuid": "decoy-1", "cwd": cwd,
        "timestamp": (time::OffsetDateTime::now_utc() + time::Duration::seconds(30))
            .format(&time::format_description::well_known::Rfc3339).unwrap(),
        "message": { "content": [ { "type": "text", "text": "SOMEBODY ELSE" } ] },
    });
    std::fs::write(
        home.join(".claude/projects")
            .join(cwd.replace('/', "-"))
            .join("99999999-9999-4999-8999-999999999999.jsonl"),
        format!("{decoy}\n"),
    )
    .unwrap();

    client
        .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
        .await;
    let convo = client.reply("convo").await;
    let text = convo["turns"].to_string();
    assert!(
        text.contains("NATIVE TURN") && !text.contains("SOMEBODY ELSE"),
        "{convo}"
    );

    // A process's start instant is re-derived from `uptime` on every look, so it never reads the
    // same twice; comparing it tore the conversation down and re-paged it on every tick, ten
    // times a second for as long as anybody watched.
    let quiet = tokio::time::Instant::now() + Duration::from_millis(1500);
    while let left = quiet.saturating_duration_since(tokio::time::Instant::now())
        && !left.is_zero()
    {
        client.pump(left).await;
    }
    let again: Vec<_> = client
        .others
        .iter()
        .filter(|v| v["t"] == "convo" || v["t"] == "convo.turn")
        .collect();
    assert!(
        again.is_empty(),
        "the conversation was sent again with nothing changed: {again:?}"
    );
}

/// **The moves are counted from the native screen, not asked of herdr.** A cursor dialog read as no
/// dialog at all gets the bare digit Claude used to be sent, and a commit with no question standing
/// is refused; either way the keys the harness reads are not these four.
#[tokio::test]
async fn a_dialog_on_a_native_claude_is_answered_with_the_moves_its_cursor_needs() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::start().await;
    let claude = h.work().join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\ncat \"$1\"\nstty raw -echo\nk=$(head -c 4 | od -An -tx1 | tr -s ' ' | sed 's/ *$//')\nstty sane\n\
         printf '\\nKEYS[%s]\\n' \"$k\"\nexec sleep 600\n",
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let dialogs = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dialogs");
    let mut client = h.client().await;
    for (fixture, frame, want) in [
        (
            "claude-trust",
            json!({ "t": "answer", "key": "2" }),
            "KEYS[ 1b 5b 42 0d]",
        ),
        (
            "claude-multi",
            json!({ "t": "answer.submit" }),
            "KEYS[ 1b 5b 43 0d]",
        ),
    ] {
        let screen = dialogs.join(format!("{fixture}.txt")).display().to_string();
        let id = client
            .create(
                &[&claude.display().to_string(), &screen],
                json!({ "cols": 120, "rows": 40 }),
            )
            .await;
        client
            .pane(&id, |p| p["agent"] == "claude" && p["agent_status"] == "blocked")
            .await;
        client.watch(&id, false).await;
        client
            .until(&format!("{fixture} published as a question"), |c| {
                c.others
                    .iter()
                    .find(|v| v["t"] == "pending" && v["pane"] == json!(id) && !v["question"].is_null())
                    .map(|_| ())
            })
            .await;
        let warm = get(&h.origin, &format!("/api/warm?pane={id}"), &client.token).await;
        assert!(
            warm["pending"]["question"].is_string(),
            "{fixture}: a tapped notification opens onto no question: {warm}"
        );
        let mut frame = frame;
        frame["pane"] = json!(id);
        client.send(frame).await;
        let screen = client.shows(&id, "KEYS[").await;
        assert!(
            screen.iter().any(|r| r == want),
            "{fixture}: {screen:?}; {:?}",
            client.others
        );
    }
}

/// **No herdr anywhere is a first-class node.** `online` stays herdr's own answer, and the node says
/// beside it that it serves native sessions, so nothing about it reads as offline: no error frame
/// names it, and a session made, typed into, sized and hung up never meets a herdr code.
#[tokio::test]
async fn a_node_with_no_herdr_is_serving_and_every_native_failure_says_what_failed() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let node = client
        .until("the node in the herd", |c| c.nodes.get(NODE).cloned())
        .await;
    assert_eq!(
        (&node["online"], &node["reachable"], &node["native"]),
        (&json!(false), &json!(true), &json!(true)),
        "{node}"
    );
    assert!(node["herdr_version"].is_null(), "{node}");

    let id = client.create(&["sh", "-c", "cat; exit 1"], json!({})).await;
    client.watch(&id, false).await;
    client.type_text(&id, "typed\r").await;
    client
        .until("the echo", |c| {
            (c.screen(&id).iter().filter(|r| *r == "typed").count() == 2).then_some(())
        })
        .await;
    let sized = client
        .manage(json!({ "op": "pane.size", "at": id, "cols": 90, "rows": 28 }))
        .await;
    assert_eq!(sized["ok"], json!(true), "{sized}");
    client.pane(&id, |p| p["cols"] == 90 && p["rows"] == 28).await;

    client.type_text(&id, "\u{4}").await;
    client.pane(&id, |p| p.get("exited").is_some()).await;
    client.type_text(&id, "too late\r").await;
    let refused = client
        .until("the refused write", |c| {
            c.others
                .iter()
                .find(|v| v["t"] == "error" && v["pane"] == json!(id))
                .cloned()
        })
        .await;
    assert_eq!(refused["code"], "session_unavailable", "{refused}");
    assert!(
        refused["message"].as_str().unwrap_or_default().contains("ended"),
        "{refused}"
    );

    while client.pump(Duration::from_millis(300)).await {}
    let errors: Vec<&Value> = client
        .others
        .iter()
        .filter(|v| v["t"] == "error" && v["pane"] != json!(id))
        .collect();
    assert!(
        errors.is_empty(),
        "a node serving native sessions was reported down: {errors:?}"
    );
}

/// A fleet run is a native session with a cohort (docs/17 §2): the node that started it can go
/// away and the next one serves the same run, still asking, still on screen, still answerable.
#[tokio::test]
async fn a_fleet_run_survives_a_node_restart_with_its_screen_and_its_answers() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let ack = client
        .manage(json!({
            "op": "fleet.run", "node": NODE, "cohort": "upgrade",
            "command": "printf 'before the restart\\nProceed? [Y/n] '; read a; echo \"got $a\"; read b",
        }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let id = ack["id"].as_str().expect("an id").to_string();
    assert!(id.starts_with(&format!("{NODE}/fleet:")), "{id}");
    let waiting = |p: &Value| p["fleet"]["state"] == "waiting";
    client.pane(&id, waiting).await;
    drop(client);

    let h = h.restart().await;
    let mut client = h.client().await;
    let pane = client.pane(&id, waiting).await;
    assert_eq!(pane["fleet"]["cohort"], "upgrade", "{pane}");
    assert_eq!(pane["fleet"]["question"]["prompt"], "Proceed? [Y/n]", "{pane}");
    assert!(
        pane.get("provider").is_none(),
        "a fleet run's record changed shape: {pane}"
    );
    client.watch(&id, false).await;
    client.shows(&id, "before the restart").await;
    client.type_text(&id, "y\r").await;
    client.shows(&id, "got y").await;
}

/// What `kampr update` does to a keeper's binary: a new file renamed over the old path.
#[tokio::test]
async fn a_node_starting_after_the_keeper_binary_was_replaced_re_execs_its_keepers_into_it() {
    let h = Harness::start().await;
    let bin = h.work().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin dir");
    let exe = bin.join("kampr-keep");
    let install = || {
        std::fs::copy(kampr_testkit::keeper(), bin.join("staged")).expect("copy");
        std::fs::rename(bin.join("staged"), &exe).expect("rename");
    };
    install();
    let mut spec = kampr_keep::Spec::new(vec!["sh".into(), "-c".into(), "exec cat".into()]);
    spec.env.retain(|(k, _)| !k.starts_with("KAMPR_KEEP"));
    let session = kampr_keep::spawn(&h.root(), spec, kampr_keep::Launch::Detached, &exe).expect("a session");
    let first = session.keeper().expect("keeper.json");
    install();
    assert!(kampr_keep::stale(&session));

    let _h = h.restart().await;
    let deadline = std::time::Instant::now() + SOON;
    while session.keeper().is_none_or(|k| k.how != kampr_keep::How::ReExec) {
        assert!(
            std::time::Instant::now() < deadline,
            "never upgraded: {:?}",
            session.keeper()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let upgraded = session.keeper().expect("keeper.json");
    assert_eq!((upgraded.keeper, upgraded.child), (first.keeper, first.child));
    assert!(!kampr_keep::stale(&session));
}

/// A native session splits the way a herdr pane does, by starting another beside it, and the two
/// carry their places in one tree for every client to lay out; the tree closes up when one goes.
#[tokio::test]
async fn a_split_native_session_starts_another_beside_it_in_one_tree() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let first = client.create(&["cat", "/dev/stdin"], json!({})).await;

    let split = client
        .manage(
            json!({ "op": "pane.split", "at": first, "direction": "down", "args": ["cat", "/dev/stdin"] }),
        )
        .await;
    assert_eq!(split["ok"], json!(true), "{split}");
    let second = split["id"].as_str().expect("the new session").to_string();
    let tile = |pane: &Value| pane["tile"].clone();
    let top = client.pane(&first, |p| !p["tile"].is_null()).await;
    let bottom = client.pane(&second, |p| !p["tile"].is_null()).await;
    assert_eq!(tile(&top)["group"], tile(&bottom)["group"], "{top}\n{bottom}");
    assert_eq!(tile(&top)["path"], json!([{ "split": "down", "ratio": 500 }]));
    assert_eq!(
        tile(&bottom)["path"],
        json!([{ "split": "down", "ratio": 500, "second": true }])
    );

    let closed = client.manage(json!({ "op": "native.close", "at": second })).await;
    assert_eq!(closed["ok"], json!(true), "{closed}");
    client.gone(&second).await;
    client.pane(&first, |p| p["tile"].is_null()).await;
}
