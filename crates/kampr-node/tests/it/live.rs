//! End-to-end against a real Herdr.
//!
//! Every test here runs in a throwaway named session created and destroyed by the test itself.
//! `default` is never touched. When `herdr` is not on PATH the suite reports a skip rather than a
//! failure, so it stays honest on a machine that has no herd.

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

static SESSIONS_STARTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

struct Session {
    name: String,
    socket: PathBuf,
}

impl Session {
    async fn start(tag: &str) -> Option<Self> {
        which("herdr")?;
        // The tag does not make this unique — two tests already pass `identity` — and two tests in
        // this binary run at once. Sharing a name is sharing one herdr server, and the first of
        // them to finish stops it in `Drop`, out from under whichever is still using it.
        let seq = SESSIONS_STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("kampr-it-{tag}-{}-{seq}", std::process::id());
        assert_ne!(name, "default");
        let socket = herdr_home().join("sessions").join(&name).join("herdr.sock");
        std::process::Command::new("herdr")
            .args(["server", "--session", &name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..100 {
            if socket.exists() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                return Some(Self { name, socket });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    fn herdr(&self) -> kampr_herdr::Herdr {
        kampr_herdr::Herdr::new(&self.socket)
    }

    /// Stops the server without deleting the session, so it can be started again on the same
    /// socket — a herdr outage rather than a herd that went away.
    async fn stop(&self) {
        let _ = self.herdr().call::<Value>("server.stop", json!({})).await;
        for _ in 0..50 {
            if !self.socket.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The rung a reboot lands on. A `server.stop` unlinks the socket; a **SIGKILL** does not, so
    /// the file is left on disk with nothing listening on it and herdr goes on reporting
    /// `running: false` because it decides that by connecting rather than by `stat` (#427).
    ///
    /// The pid is looked up rather than kept: `herdr server` daemonises, so the process this
    /// harness spawned exits immediately and the one holding the socket is a fork further on.
    async fn kill(&self) {
        let pid = server_pid(&self.name).expect("the herdr server's own pid");
        let killed = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
        assert!(killed.is_ok_and(|s| s.success()), "could not SIGKILL herdr {pid}");
        for _ in 0..100 {
            if server_pid(&self.name).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(server_pid(&self.name).is_none(), "herdr {pid} outlived a SIGKILL");
        assert!(
            self.herdr().snapshot().await.is_err(),
            "something is still answering on the socket"
        );
        // **The one assertion that keeps this from being a second copy of the clean-stop test.**
        // A `server.stop` unlinks the socket *before* the process goes, so waiting for the process
        // and then finding the file is the whole of the difference between the two outages — and
        // asserting it the other way round, on the socket the moment herdr stops answering, holds
        // nothing: a clean stop passes that too, because the unlink has not landed yet.
        assert!(
            self.socket.exists(),
            "the socket went with the server, so this was a clean stop after all"
        );
    }

    // The server is stopped over its own socket in `Drop`, not by reaping a child: it outlives
    // this handle by design, exactly as the one `start` spawns does.
    #[allow(clippy::zombie_processes)]
    async fn respawn(&self) {
        std::process::Command::new("herdr")
            .args(["server", "--session", &self.name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("herdr server");
        for _ in 0..100 {
            if self.socket.exists() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("herdr never came back on {}", self.socket.display());
    }

    async fn call(&self, method: &str, params: Value) -> Value {
        self.herdr()
            .call::<Value>(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(dir) = self.socket.parent() {
            forget_session(dir);
        }
    }
}

/// A session a test made, removed whether that test passes, fails or panics. Teardown at the end
/// of a body is skipped by both a panic and an abort, and an aborted run once left thirty-seven
/// of these — which is not tidiness but correctness, because a node serves every session it can
/// find (#97), so one left behind is in the next test's herd.
struct CreatedSession {
    dir: PathBuf,
}

impl CreatedSession {
    fn named(name: &str) -> Self {
        Self {
            dir: herdr_home().join("sessions").join(name),
        }
    }

    fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for CreatedSession {
    fn drop(&mut self) {
        forget_session(&self.dir);
        assert!(
            std::thread::panicking() || !self.dir.exists(),
            "left {:?} behind",
            self.dir
        );
    }
}

/// Never leave a herdr behind. `server.stop` over the session's own socket first — a test that
/// panicked before its `session.stop` still has one running, and removing the directory under it
/// leaves the herdr — then the socket, then the directory. A stopped session keeps its directory
/// until something removes it (#242), and a herdr asked to stop still owns that directory for a
/// moment: one removal races it, which is what leaves a throwaway session listed forever. So wait
/// for the socket to go, then keep removing until the directory stays gone.
pub(crate) fn forget_session(dir: &Path) {
    if !dir.exists() {
        return;
    }
    let socket = dir.join("herdr.sock");
    if socket.exists() {
        // `block_on` panics inside a runtime, and every caller of this is inside one.
        std::thread::spawn({
            let socket = socket.clone();
            move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("teardown runtime");
                runtime.block_on(async {
                    let _ = kampr_herdr::Herdr::new(&socket)
                        .call::<Value>("server.stop", json!({}))
                        .await;
                });
            }
        })
        .join()
        .ok();
    }
    for _ in 0..100 {
        if !socket.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for _ in 0..25 {
        let _ = std::fs::remove_dir_all(dir);
        std::thread::sleep(Duration::from_millis(200));
        if !dir.exists() {
            break;
        }
    }
}

/// The pid of the `herdr server` on `name`, read out of `/proc` because the process that holds
/// the socket is not the one this harness spawned.
fn server_pid(name: &str) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let argv: Vec<&str> = std::str::from_utf8(&raw)
            .unwrap_or_default()
            .split('\0')
            .filter(|a| !a.is_empty())
            .collect();
        if argv.contains(&"server") && argv.contains(&name) {
            return Some(pid);
        }
    }
    None
}

fn which(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

/// Leaves the session holding **only** the workspace this harness asked for.
///
/// herdr's headless server creates a startup workspace of its own (#452) — one login shell in `$HOME`,
/// logged as `created startup workspace cwd=<home>`, before it accepts a single call — so a
/// throwaway session is two panes and not one. That second pane is nobody's: its working
/// directory is the operator's home rather than the `/tmp` this harness names, and its workspace
/// carries herdr's own `~` label rather than `kampr`. Every test that asks the harness for "the
/// pane" means the one it created, and `pane_id` takes the first the herd offers, so the startup
/// pane silently answers for it — a conversation looked for under `~/.claude/projects/-tmp` in a
/// pane whose cwd is `$HOME`, and a name reported as `~ · bash` where the workspace label is the
/// assertion.
///
/// Waited out rather than assumed: `workspace.close` is answered `ok` from the API thread and the
/// snapshot catches up after it, and the node builds its first herd off whatever the snapshot
/// says at the moment it starts.
async fn the_only_workspace(session: &Session, keep: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut standing = Vec::new();
    // Two clear readings, not one. herdr creates its startup workspace ~50 ms *after* it starts
    // listening, so on a loaded box it can appear after the first look — and a single clear
    // reading would then be of a session that is about to gain a pane nobody asked for.
    let mut clear = 0;
    while tokio::time::Instant::now() < deadline {
        // Never `Session::call` here, which panics on a socket that answered slowly. This runs
        // while every harness in the suite is starting at once, and a herdr that took longer than
        // its timeout to answer must cost a retry rather than the test.
        let Ok(snapshot) = session.herdr().call::<Value>("session.snapshot", json!({})).await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        standing = snapshot["snapshot"]["workspaces"]
            .as_array()
            .map(|ws| {
                ws.iter()
                    .filter_map(|w| w["workspace_id"].as_str())
                    .filter(|id| *id != keep)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if standing.is_empty() {
            clear += 1;
            if clear == 2 {
                return;
            }
        } else {
            clear = 0;
            for id in &standing {
                let _ = session
                    .herdr()
                    .call::<Value>("workspace.close", json!({ "workspace_id": id }))
                    .await;
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("herdr would not give up {standing:?}, the workspaces this harness did not create");
}

/// Waits for the harness's pane to be a shell at a prompt and nothing else.
///
/// **A pane is not a fixture until its `.bashrc` has finished.** For the first second and a half
/// of one, the foreground process group carries whatever the operator's profile spawns beside the
/// shell — `node`, `dirname`, `tail`, `head`, `atuin` on this machine (probe #342) — and ble.sh is
/// still to re-render the line it accepted, which lands *after* whatever a test drew and wipes it
/// ([#446](#)). Both are why a test that types into this pane in its first seconds is measuring
/// the profile: a name read then is `kampr · node` and stays it, and a screen painted then can be
/// gone by the time the node reads it.
///
/// The condition is herdr's own: one foreground process, and it is the shell itself. Nothing here
/// depends on what that shell is called, which `/bin/sh` being a different binary on half these
/// machines has cost this suite before.
///
/// A give-up threshold on the environment rather than an assertion: every test that cares has its
/// own budget, and a machine so loaded that a shell has not reached its prompt in a minute will
/// say so in the test that needed it.
async fn a_pane_at_its_shell(session: &Session, pane: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        let Ok(info) = session
            .herdr()
            .call::<Value>("pane.process_info", json!({ "pane_id": pane }))
            .await
        else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let shell = info["process_info"]["shell_pid"].as_u64();
        let foreground = info["process_info"]["foreground_processes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if shell.is_some() && foreground.len() == 1 && foreground[0]["pid"].as_u64() == shell {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn herdr_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").expect("HOME")).join(".config"))
        .join("herdr")
}

struct Harness {
    _session: Session,
    _state: tempfile::TempDir,
    node: Arc<Node>,
    origin: String,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Harness {
    async fn start_with(tag: &str, tweak: impl FnOnce(&mut Config)) -> Option<Self> {
        let session = Session::start(tag).await?;
        let created = session
            .call("workspace.create", json!({ "label": "kampr", "cwd": "/tmp" }))
            .await;
        the_only_workspace(&session, created["workspace"]["workspace_id"].as_str()?).await;
        a_pane_at_its_shell(&session, created["root_pane"]["pane_id"].as_str()?).await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
        let port = listener.local_addr().ok()?.port();
        let state = tempfile::tempdir().ok()?;
        let config_dir = state.path().join("config");

        let mut config = Config::bootstrap("testnode");
        // Nothing in this suite reaches the internet: the release check is the one thing in a
        // node that would, and a test that phoned GitHub would be one with a rate limit.
        config.update.check = false;
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = session.socket.display().to_string();
        // Serve only this test's own session. The node discovers every herdr running on the
        // machine by default, and another test's throwaway herd is not this test's herd.
        config.herdr.sessions = Some(Vec::new());
        config.auth.audit = true;
        config.limits.client_queue = 32;
        tweak(&mut config);
        config.save(&config_dir).ok()?;

        let origin = config.origin();
        let node = Node::start(config, state.path()).await.ok()?;
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        // The first herd model is built by a background task; a watch with no panes is not ready.
        for _ in 0..50 {
            if node.herd().panes.iter().any(|p| p.node_id == node.node_id()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Some(Self {
            _session: session,
            _state: state,
            node,
            origin,
            server,
        })
    }

    async fn token(&self, role: Role) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(role, kampr_auth::Delivery::Console)
            .await
            .unwrap();
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.unwrap());
        }
        let body = json!({ "code": pairing.code, "device_name": "integration" });
        let response = post(&format!("{}/auth/pair", self.origin), &body).await;
        response["token"].as_str().expect("a token").to_string()
    }

    async fn connect(&self, token: &str) -> Socket {
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("websocket upgrade");
        socket
    }

    /// Whether the herd says this pane has a transcript, once the sweep has had time to say so.
    async fn claims_a_conversation(&self, pane: &str) -> bool {
        let mut claimed = false;
        for _ in 0..24 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Some(entry) = self.node.herd().pane(pane) {
                claimed |= serde_json::to_value(entry).unwrap()["has_conversation"] == true;
            }
        }
        claimed
    }

    /// The pane of this harness's session rooted at `cwd`, once herdr has reported it.
    async fn pane_with_cwd(&self, cwd: &str) -> Option<String> {
        for _ in 0..100 {
            let found = self
                .node
                .herd()
                .panes
                .iter()
                .find(|p| p.node_id == self.node.node_id() && p.cwd.as_deref() == Some(cwd))
                .map(|p| p.id.clone());
            if found.is_some() {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    /// A pane of *this harness's own* session. The node discovers every named herdr session on
    /// the machine, so the herd carries other tests' panes — and another test's throwaway session
    /// is not what a test means by "the pane".
    fn pane_id(&self) -> String {
        self.node
            .herd()
            .panes
            .iter()
            .find(|p| p.node_id == self.node.node_id())
            .expect("a pane on this harness's own session")
            .id
            .clone()
    }

    /// Whether the node has noticed that its own herdr is gone. A test that stops herdr and acts
    /// immediately is racing the poll loop, and proves whatever the race decided.
    async fn offline(&self, seconds: u64) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        while tokio::time::Instant::now() < deadline {
            let herd = self.node.herd();
            if herd
                .nodes
                .iter()
                .any(|n| n.id == self.node.node_id() && !n.online)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        false
    }
}

async fn post(url: &str, body: &Value) -> Value {
    post_as(url, body, None).await.1
}

/// `host` and `origin` are what a DNS-rebinding attacker controls: a domain they own, pointed at
/// this node's address, so the browser sends both as their own.
async fn post_from(url: &str, body: &Value, host: &str, origin: Option<&str>) -> String {
    let (real_host, port, path) = split(url);
    let payload = body.to_string();
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         {origin}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    raw_with_status(&real_host, port, &request).await.0
}

async fn head_of(url: &str, body: &Value) -> String {
    let (host, port, path) = split(url);
    let payload = body.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect((host.as_str(), port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    text.split_once("\r\n\r\n")
        .map_or(text.clone(), |(h, _)| h.to_string())
}

async fn with_cookie(method: &str, url: &str, token: &str, origin: Option<&str>) -> String {
    let (host, port, path) = split(url);
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nCookie: kampr_session={token}\r\n\
         {origin}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    raw_with_status(&host, port, &request).await.0
}

async fn post_as(url: &str, body: &Value, forwarded_for: Option<&str>) -> (String, Value) {
    let (host, port, path) = split(url);
    let payload = body.to_string();
    let forwarded = forwarded_for.map_or(String::new(), |f| format!("X-Forwarded-For: {f}\r\n"));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         {forwarded}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let (status, body) = raw_with_status(&host, port, &request).await;
    (status, Value::from(body))
}

async fn get(url: &str, token: Option<&str>) -> (String, Value) {
    let (host, port, path) = split(url);
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\n{auth}Connection: close\r\n\r\n");
    let (status, body) = raw_with_status(&host, port, &request).await;
    (status, Value::from(body))
}

/// The whole response head, for the one question a JSON body cannot answer: what a browser is
/// told to do with a file it already has.
async fn response_head(url: &str, extra: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (host, port, path) = split(url);
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\n{extra}Connection: close\r\n\r\n");
    let mut stream = TcpStream::connect((host.as_str(), port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    text.split_once("\r\n\r\n")
        .map(|(head, _)| head.to_string())
        .unwrap_or(text)
}

fn split(url: &str) -> (String, u16, String) {
    let rest = url.trim_start_matches("http://");
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
    (host.to_string(), port.parse().unwrap(), format!("/{path}"))
}

async fn raw_with_status(host: &str, port: u16, request: &str) -> (String, Json) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect((host, port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or_default().to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    (status, Json(body.trim().to_string()))
}

struct Json(String);

impl From<Json> for Value {
    fn from(json: Json) -> Self {
        serde_json::from_str(&json.0).unwrap_or(Value::Null)
    }
}

/// Drains until the peer closes. `true` means the node hung up inside the window.
async fn closed(socket: &mut Socket, seconds: u64) -> bool {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, tungstenite::Message::Close(_)) {
                return;
            }
        }
    })
    .await
    .is_ok()
}

async fn recv(socket: &mut Socket, timeout: Duration) -> Option<Value> {
    let message = tokio::time::timeout(timeout, socket.next()).await.ok()??.ok()?;
    match message {
        tungstenite::Message::Text(text) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

async fn until(socket: &mut Socket, tag: &str, seconds: u64) -> Value {
    until_seen(socket, tag, seconds).await.0
}

/// [`until`], plus every `t` that arrived before it — for the claims that are about what the node
/// did *not* send.
async fn until_seen(socket: &mut Socket, tag: &str, seconds: u64) -> (Value, Vec<String>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == tag {
            return (message, seen);
        }
        seen.push(message["t"].as_str().unwrap_or("?").to_string());
    }
    panic!("never saw {tag}; saw {seen:?}");
}

/// Like [`until`], but for a message about one pane: the node serves every herdr session on the
/// machine, so another test's session going away arrives here as an unrelated `error`.
async fn until_pane(socket: &mut Socket, tag: &str, pane: &str, seconds: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == tag && message["pane"] == pane {
            return message;
        }
        seen.push(message["t"].as_str().unwrap_or("?").to_string());
    }
    panic!("never saw {tag} for {pane}; saw {seen:?}");
}

/// A rebuild the caller did not cause reaches the client as a `herd.patch` too — a title
/// settling, a cwd arriving — and that patch carries no `added`. Waiting for the tag alone takes
/// whichever came first and reads `added.panes` off a patch that never had one.
async fn until_panes_added(socket: &mut Socket, seconds: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "herd.patch"
            && message["added"]["panes"]
                .as_array()
                .is_some_and(|panes| !panes.is_empty())
        {
            return message;
        }
        seen.push(message["t"].as_str().unwrap_or("?").to_string());
    }
    panic!("no herd.patch added a pane; saw {seen:?}");
}

/// The pane's `agent_status` as the herd puts it on the wire, waited for rather than sampled: the
/// full herd arrives once and everything after it is a patch, so both carry panes, and a rebuild
/// the node has not run yet is not a status it refused.
async fn until_status(socket: &mut Socket, pane: &str, want: &str, seconds: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut saw = String::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        let found = ["panes"]
            .iter()
            .map(|key| &message[key])
            .chain([&message["changed"]["panes"], &message["added"]["panes"]])
            .filter_map(|panes| panes.as_array())
            .flat_map(|panes| panes.iter())
            .find(|p| p["id"] == pane)
            .and_then(|entry| entry["agent_status"].as_str())
            .map(str::to_string);
        let Some(entry) = found else {
            continue;
        };
        saw = entry;
        if saw == want {
            return;
        }
    }
    panic!("{pane} never reached {want} on the wire; it settled at {saw:?}");
}

async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(tungstenite::Message::text(value.to_string()))
        .await
        .expect("send");
}

macro_rules! harness {
    ($tag:expr) => {
        harness!($tag, |_| {})
    };
    ($tag:expr, $tweak:expr) => {
        match Harness::start_with($tag, $tweak).await {
            Some(h) => h,
            None => {
                eprintln!("skipping: no herdr on PATH");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_device_drives_a_pane_end_to_end() {
    let h = harness!("drive");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;

    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(hello["protocol"], 1);
    assert_eq!(hello["role"], "full");
    assert_eq!(hello["caps"]["manage"], true);
    assert_eq!(hello["node_name"], "testnode");
    assert_eq!(hello["security"]["tier"], 0, "a loopback IP origin is Tier 0");
    assert_eq!(hello["security"]["passkeys"], false);

    let herd = until(&mut socket, "herd", 10).await;
    let pane = herd["panes"][0]["id"].as_str().expect("a pane").to_string();
    assert!(pane.starts_with(hello["node_id"].as_str().unwrap()));
    assert!(herd["panes"][0]["rows"].as_u64().unwrap() > 0);
    // `cols` is absent until a wrap has proved the PTY width. The layout rect is not it: headless,
    // a pane whose rect reads 47 is really 93 wide (probe #68).
    assert!(herd["panes"][0]["cols"].as_u64().is_none_or(|cols| cols > 0));
    // A brand-new workspace's pane really is nearly empty (probe #212), and empty is not a fault:
    // `detail` is absent for every pane a node can actually stream.
    assert!(
        herd["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["detail"].is_null()),
        "a healthy node marked a pane unstreamable: {herd}"
    );

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let reset = until(&mut socket, "grid.reset", 15).await;
    assert_eq!(reset["pane"], pane.as_str());
    assert_eq!(
        reset["rows_data"].as_array().unwrap().len(),
        reset["rows"].as_u64().unwrap() as usize
    );

    let marker = "kampr-echo-marker";
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": format!("echo {marker}\n") }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut echoed = false;
    while tokio::time::Instant::now() < deadline && !echoed {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        let text = message.to_string();
        echoed = text.contains(marker);
    }
    assert!(echoed, "the pane never echoed {marker} back over the wire");

    send(&mut socket, json!({ "t": "ping", "n": 7 })).await;
    assert_eq!(until(&mut socket, "pong", 10).await["n"], 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_readonly_device_is_refused_writes_and_still_sees_the_pane() {
    let h = harness!("ro");
    let token = h.token(Role::Readonly).await;
    let mut socket = h.connect(&token).await;

    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(hello["role"], "readonly");
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let reset = until(&mut socket, "grid.reset", 15).await;
    assert_eq!(
        reset["pane"],
        pane.as_str(),
        "readonly still receives every frame"
    );

    for write in [
        json!({ "t": "input", "pane": pane, "text": "whoami\n" }),
        json!({ "t": "answer", "pane": pane, "key": "1" }),
        json!({ "t": "manage", "op": "workspace.create", "label": "nope" }),
    ] {
        send(&mut socket, write).await;
        let error = until(&mut socket, "error", 10).await;
        assert_eq!(error["code"], "not_writer");
    }

    // And the stream is still live afterwards.
    send(&mut socket, json!({ "t": "ping", "n": 3 })).await;
    assert_eq!(until(&mut socket, "pong", 10).await["n"], 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn manage_ops_reshape_the_herd_and_come_back_as_a_patch() {
    let h = harness!("manage");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let herd = until(&mut socket, "herd", 10).await;
    let before = herd["panes"].as_array().unwrap().len();
    let pane = herd["panes"][0]["id"].as_str().unwrap().to_string();

    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.split", "at": pane, "direction": "right", "ratio": 0.5 }),
    )
    .await;
    let ack = until(&mut socket, "managed", 15).await;
    assert_eq!(ack["ok"], true, "{ack}");
    assert!(ack["id"].as_str().unwrap().contains(":p"), "{ack}");

    until_panes_added(&mut socket, 20).await;
    assert!(h.node.herd().panes.len() > before);

    send(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": pane, "label": "build" }),
    )
    .await;
    assert_eq!(until(&mut socket, "managed", 15).await["ok"], true);

    send(&mut socket, json!({ "t": "manage", "op": "nonsense.op" })).await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(refusal["code"], "unsupported");
}

#[tokio::test(flavor = "multi_thread")]
async fn capability_discovery_comes_from_the_host_not_a_list() {
    let h = harness!("caps");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    send(&mut socket, json!({ "t": "caps" })).await;
    let caps = until(&mut socket, "caps", 15).await;
    let kinds: Vec<&str> = caps["agent_kinds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|k| k.as_str())
        .collect();
    assert!(kinds.contains(&"claude"), "{kinds:?}");
    assert!(kinds.len() > 3, "the host reported {} kinds", kinds.len());

    let sessions: Vec<&str> = caps["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(sessions.contains(&h._session.name.as_str()), "{sessions:?}");

    // A session this node does not serve must not be advertised as one a client can open a pane
    // on — that promise is what made the capability a lie.
    let served: Vec<&str> = caps["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["served"] == true)
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert_eq!(
        served,
        [h._session.name.as_str()],
        "this node serves only its own session: {caps}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_message_is_ignored_and_a_bad_one_is_refused() {
    let h = harness!("unknown");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    // Unknown `t` and unknown fields are how a v1 client survives a v1.1 node.
    send(&mut socket, json!({ "t": "telepathy", "wish": "coffee" })).await;
    send(&mut socket, json!({ "t": "ping", "n": 1, "unknown_field": true })).await;
    assert_eq!(until(&mut socket, "pong", 10).await["n"], 1);

    send(&mut socket, json!({ "t": "input", "pane": h.pane_id() })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "bad_request");

    send(
        &mut socket,
        json!({ "t": "input", "pane": h.pane_id(), "b64": "//8=" }),
    )
    .await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(refusal["code"], "bad_request");
    assert!(refusal["message"].as_str().unwrap().contains("UTF-8"));

    send(&mut socket, json!({ "t": "watch", "pane": "01JNOTOURS/w9:p9" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "unknown_pane");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_http_surface_refuses_what_it_should() {
    let h = harness!("http");
    let (status, body) = get(&format!("{}/api/node", h.origin), None).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["security"]["tier"], 0);
    assert_eq!(body["security"]["passkeys"], false);
    assert_eq!(body["security"]["unlocks"][0], "passkeys");

    let (status, _) = get(&format!("{}/api/devices", h.origin), None).await;
    assert!(status.contains("401"), "{status}");

    let (status, _) = get(&format!("{}/api/devices", h.origin), Some("kmp_bogus")).await;
    assert!(status.contains("401"), "{status}");

    let token = h.token(Role::Full).await;
    let (status, body) = get(&format!("{}/api/devices", h.origin), Some(&token)).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);

    // A passkey ceremony on an IP origin is refused with a reason, not a broken ceremony.
    let refusal = post(
        &format!("{}/auth/webauthn/authenticate/start", h.origin),
        &json!({}),
    )
    .await;
    assert!(
        refusal["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("registrable domain"),
        "{refusal}"
    );
}

/// The same-origin gate used to build its allowlist from the request's own `Host`, so a
/// rebinding attacker satisfied it with two headers they wrote themselves.
#[tokio::test(flavor = "multi_thread")]
async fn an_origin_the_attacker_chose_never_satisfies_the_same_origin_gate() {
    let h = harness!("origin");
    let pairing = h
        .node
        .auth
        .create_pairing(Role::Full, kampr_auth::Delivery::Authenticated)
        .await
        .unwrap();
    let body = json!({ "code": pairing.code, "device_name": "rebound" });

    let status = post_from(
        &format!("{}/auth/pair", h.origin),
        &body,
        "kampr.rebind.example",
        Some("http://kampr.rebind.example"),
    )
    .await;
    assert!(
        status.contains("403"),
        "Origin == Host is the attacker's own claim, not a same-origin proof: {status}"
    );

    // And the ordinary path still works, so the gate is a gate rather than a wall.
    let good = post(&format!("{}/auth/pair", h.origin), &body).await;
    assert!(good["token"].is_string(), "{good}");
}

/// A cookie is the one credential a browser attaches by itself, and the origin gate exempts every
/// `GET` outside `/ws` — so as long as a cookie authenticates anything, `GET /api/devices` is one
/// deployment decision away from being a cross-origin read of the whole inventory. Nothing sets
/// this cookie and no client sends one, so the honest fix is that it is not a credential at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_cookie_is_not_a_credential() {
    let h = harness!("cookie");
    let token = h.token(Role::Full).await;
    h.token(Role::Readonly).await;
    let victim_id = h
        .node
        .auth
        .devices()
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.role == Role::Readonly)
        .unwrap()
        .id;

    let revoke = format!("{}/api/devices/{victim_id}/revoke", h.origin);
    assert!(
        with_cookie("POST", &revoke, &token, None).await.contains("401"),
        "an ambient credential must not be a credential"
    );
    assert!(
        with_cookie("POST", &revoke, &token, Some(&h.origin))
            .await
            .contains("401"),
        "a same-origin cookie is still a cookie"
    );

    // The un-gated GET is the one that mattered: the device inventory, read from another origin.
    let devices = format!("{}/api/devices", h.origin);
    assert!(
        with_cookie("GET", &devices, &token, Some("https://evil.example"))
            .await
            .contains("401"),
        "a cross-origin page must not be able to read the device inventory"
    );

    // And the header the shipped client actually sends still works.
    assert!(
        get(&devices, Some(&token)).await.0.contains("200"),
        "the bearer path is the one the client uses and must be untouched"
    );
}

/// Reconnection only. That a *live* socket dies is a different claim, and it has its own test —
/// this one used to be named as though it covered both.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_token_cannot_reconnect_and_a_wrong_code_never_gets_one() {
    let h = harness!("revoke");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node.auth.revoke(&device.id, &device).await.unwrap();

    let url = h.origin.replacen("http", "ws", 1) + "/ws";
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("kampr.token.{token}").parse().unwrap(),
    );
    assert!(
        tokio_tungstenite::connect_async(request).await.is_err(),
        "a revoked token must not reconnect"
    );

    let refused = post(
        &format!("{}/auth/pair", h.origin),
        &json!({ "code": "ZZZZ-ZZZZ", "device_name": "attacker" }),
    )
    .await;
    assert!(refused["token"].is_null(), "{refused}");

    // The audit log records the write actions, at 0600.
    let audit = Config::audit_path(h._state.path());
    let text = std::fs::read_to_string(&audit).unwrap();
    assert!(text.lines().any(|l| l.contains("\"pairing.redeemed\"")), "{text}");
    assert!(text.lines().any(|l| l.contains("\"device.revoked\"")), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&audit).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

/// The kill switch has to reach the socket that is already open. Revoking a device and then
/// checking that a *second* connection is refused proves nothing about the one holding the pane.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_hangs_up_the_socket_it_is_already_using() {
    let h = harness!("revoke-live");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node.auth.revoke(&device.id, &device).await.unwrap();

    assert!(
        closed(&mut socket, 15).await,
        "a revoked device kept its live socket"
    );
}

/// Same defect from the other two directions: the handshake snapshot of the role, and the
/// handshake snapshot of the expiry. Both have to be re-read on the live socket.
#[tokio::test(flavor = "multi_thread")]
async fn a_demotion_and_an_expiry_both_land_on_the_open_socket() {
    let h = harness!("demote-live");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node
        .auth
        .set_role(&device.id, Role::Readonly, &device)
        .await
        .unwrap();
    send(&mut socket, json!({ "t": "input", "pane": pane, "text": "x" })).await;
    assert_eq!(
        until(&mut socket, "error", 10).await["code"],
        "not_writer",
        "a demoted device kept writing against its handshake role"
    );

    // Straight to the database, so nothing in-process is notified — this is the path a Tier 0
    // token passing its expiry mid-session takes.
    h.node
        .auth
        .store()
        .extend_device(&device.id, Some(kampr_auth::now() - 1))
        .await
        .unwrap();
    assert!(
        closed(&mut socket, 15).await,
        "an expired device kept its live socket"
    );
}

/// Enforcement without an announcement is half a fix: a demoted device keeps every write
/// affordance drawn and finds out by being refused. The change has to reach the client on the
/// socket it is already holding — in both directions, because a promotion is the same problem in
/// reverse — and `hello` has to stay the *first* message on a connection rather than quietly
/// becoming one a node re-sends.
#[tokio::test(flavor = "multi_thread")]
async fn a_demotion_and_a_promotion_are_both_announced_on_the_open_socket() {
    let h = harness!("role-frame");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    assert_eq!(until(&mut socket, "hello", 10).await["role"], "full");

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node
        .auth
        .set_role(&device.id, Role::Readonly, &device)
        .await
        .unwrap();
    let (demoted, before) = until_seen(&mut socket, "role", 10).await;
    assert_eq!(demoted["role"], "readonly", "{demoted}");
    assert!(
        !before.contains(&"hello".to_string()),
        "a role change re-sent `hello`, which the protocol defines as the first message on a \
         connection: {before:?}"
    );

    h.node
        .auth
        .set_role(&device.id, Role::Full, &device)
        .await
        .unwrap();
    let (promoted, between) = until_seen(&mut socket, "role", 10).await;
    assert_eq!(promoted["role"], "full", "{promoted}");
    assert!(!between.contains(&"hello".to_string()), "{between:?}");

    // And the promotion is real, not just announced.
    send(
        &mut socket,
        json!({ "t": "input", "pane": h.pane_id(), "text": "" }),
    )
    .await;
    send(&mut socket, json!({ "t": "ping", "n": 1 })).await;
    let (_, seen) = until_seen(&mut socket, "pong", 10).await;
    assert!(
        !seen.contains(&"error".to_string()),
        "a promoted device was still refused: {seen:?}"
    );
}

/// Probe #125. A read-only device probing what it can reach is precisely the thing an audit log
/// exists to notice, and it left no trace at all — not on the socket, and not over HTTP. A buggy
/// client retrying in a loop must not be able to write the log full with the answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_write_is_audited_and_a_retry_loop_does_not_flood_the_log() {
    let h = harness!("refused");
    let pane = h.pane_id();
    let viewer = h.token(Role::Readonly).await;
    let mut socket = h.connect(&viewer).await;
    until(&mut socket, "hello", 10).await;

    send(&mut socket, json!({ "t": "input", "pane": pane, "text": "x" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    send(&mut socket, json!({ "t": "answer", "pane": pane, "key": "1" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.close", "at": pane, "rid": "r1" }),
    )
    .await;
    assert_eq!(until(&mut socket, "managed", 10).await["ok"], false);

    // The same gate, the same silence, on the other surface: the device inventory is one a
    // half-trusted device is refused, and that refusal was unlogged too.
    assert!(
        get(&format!("{}/api/devices", h.origin), Some(&viewer))
            .await
            .0
            .contains("403")
    );

    for _ in 0..40 {
        send(&mut socket, json!({ "t": "input", "pane": pane, "text": "x" })).await;
    }
    send(&mut socket, json!({ "t": "ping", "n": 7 })).await;
    until(&mut socket, "pong", 15).await;

    let text = std::fs::read_to_string(Config::audit_path(h._state.path())).unwrap();
    let refusals: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["action"] == "refused")
        .collect();
    let verb =
        |name: &str| -> Vec<&Value> { refusals.iter().filter(|e| e["detail"]["verb"] == name).collect() };
    for name in ["input", "answer", "manage", "api.devices"] {
        assert!(
            !verb(name).is_empty(),
            "a refused {name} left no audit line at all; the log holds {text}"
        );
    }
    let first = verb("input")[0];
    assert_eq!(first["role"], "readonly", "{first}");
    assert_eq!(
        first["pane"],
        pane.as_str(),
        "a refusal that does not say on what: {first}"
    );
    assert_eq!(first["detail"]["code"], "not_writer", "{first}");
    assert!(
        first["device"].is_string() && first["device_name"] == "integration",
        "{first}"
    );
    assert_eq!(verb("manage")[0]["detail"]["op"], "pane.close");

    // 41 refused inputs. Every one of them on its own line is a log a loop can fill; one line for
    // all of them is a count nobody can read off the log.
    let lines = verb("input").len();
    assert!(
        (2..=8).contains(&lines),
        "41 refusals of one verb wrote {lines} lines"
    );
    let highest = verb("input")
        .iter()
        .filter_map(|e| e["detail"]["attempt"].as_u64())
        .max()
        .unwrap_or(0);
    assert!(
        highest >= 16,
        "the suppressed refusals were never counted; highest attempt was {highest}"
    );
}

/// `X-Forwarded-For` grows left to right: the client's own value sits at the head and each proxy
/// appends what it actually saw. Reading the head hands a rotating header a fresh rate-limit
/// bucket per request — which is exactly what the limiter exists to stop.
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_forwarded_for_buys_no_fresh_rate_limit_bucket() {
    let h = harness!("xff", |c: &mut Config| c.server.trust_proxy = true);
    let mut refused = 0;
    for n in 0..12u8 {
        let (status, _) = post_as(
            &format!("{}/auth/pair", h.origin),
            &json!({ "code": "ZZZZ-ZZZZ", "device_name": "attacker" }),
            Some(&format!("10.0.0.{n}, 203.0.113.9")),
        )
        .await;
        if status.contains("429") {
            refused += 1;
        }
    }
    assert!(
        refused > 0,
        "the limiter must key on what the trusted proxy saw, not on what the client claimed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_http_surface_leaks_neither_paths_nor_the_device_inventory() {
    let h = harness!("leaks");
    let full = h.token(Role::Full).await;
    let readonly = h.token(Role::Readonly).await;

    let (status, body) = get(&format!("{}/api/devices", h.origin), Some(&readonly)).await;
    assert!(
        status.contains("403"),
        "a read-only device must not read the device inventory: {status} {body}"
    );
    assert!(
        get(&format!("{}/api/devices", h.origin), Some(&full))
            .await
            .0
            .contains("200")
    );

    // A token in a response body must never be cached by anything between here and the phone.
    let pairing = h
        .node
        .auth
        .create_pairing(Role::Full, kampr_auth::Delivery::Authenticated)
        .await
        .unwrap();
    let headers = head_of(
        &format!("{}/auth/pair", h.origin),
        &json!({ "code": pairing.code, "device_name": "cached" }),
    )
    .await;
    assert!(
        headers.to_lowercase().contains("cache-control: no-store"),
        "{headers}"
    );
    assert!(
        !headers.contains("connect-src 'self' ws:"),
        "a bare ws: scheme matches any host and is a clean exfiltration channel: {headers}"
    );
}

/// A read-only device receives every frame of every pane and is refused every write — but `prefs`
/// and `caps` were not writes as far as the dispatcher was concerned. One wrote unbounded rows
/// keyed on an arbitrary pane id; the other shelled out to `herdr session list` per message.
#[tokio::test(flavor = "multi_thread")]
async fn prefs_and_caps_are_bounded_rather_than_an_amplifier() {
    let h = harness!("bounds");
    let token = h.token(Role::Readonly).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    // The greeting now carries this device's stored preferences, so the answer to a *write* is
    // the second `prefs` frame on the socket rather than the first.
    until(&mut socket, "prefs", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "zoom": 1.6 } }),
    )
    .await;
    let stored = until(&mut socket, "prefs", 10).await;
    assert_eq!(stored["panes"][&pane]["zoom"], 1.6, "a viewer keeps its own zoom");

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": "01JNOTAPANE/w9:p9", "prefs": { "zoom": 2.0 } }),
    )
    .await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(
        refusal["code"], "unknown_pane",
        "an arbitrary pane id is a row nobody asked for: {refusal}"
    );

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "junk": "x".repeat(64 * 1024) } }),
    )
    .await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(refusal["code"], "bad_request", "{refusal}");

    // Repeated `caps` must not be one `herdr session list` process per message.
    let before = h.node.caps_spawns();
    for _ in 0..8 {
        send(&mut socket, json!({ "t": "caps" })).await;
        until(&mut socket, "caps", 10).await;
    }
    assert!(
        h.node.caps_spawns() - before <= 1,
        "caps shelled out {} times for 8 messages",
        h.node.caps_spawns() - before
    );
}

/// A read-only device that watches every pane exfiltrates every terminal on the host, and used to
/// leave one `session.opened` line behind. And a `manage` entry that omits `cwd`, `args`, `path`
/// and `branch` records that something ran, not what.
#[tokio::test(flavor = "multi_thread")]
async fn the_audit_records_what_was_read_and_what_actually_ran() {
    let h = harness!("audit");
    let pane = h.pane_id();

    let viewer = h.token(Role::Readonly).await;
    let mut watching = h.connect(&viewer).await;
    until(&mut watching, "hello", 10).await;
    send(
        &mut watching,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut watching, "grid.reset", 15).await;
    send(&mut watching, json!({ "t": "unwatch", "pane": pane })).await;

    let writer = h.token(Role::Full).await;
    let mut driving = h.connect(&writer).await;
    until(&mut driving, "hello", 10).await;
    send(
        &mut driving,
        json!({ "t": "manage", "op": "pane.split", "at": pane, "direction": "right", "cwd": "/tmp" }),
    )
    .await;
    until(&mut driving, "managed", 15).await;

    // The `keys` form of `input` is herdr's key grammar, and probe #7 says that grammar includes
    // single characters — so a client that types a password one key at a time types it, and the
    // log is what an operator hands to somebody else during an investigation.
    send(
        &mut driving,
        json!({ "t": "input", "pane": pane, "keys": ["h", "u", "n", "t", "e", "r", "2"] }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let text = std::fs::read_to_string(Config::audit_path(h._state.path())).unwrap();
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let of = |action: &str| -> Value {
        entries
            .iter()
            .find(|e| e["action"] == action)
            .unwrap_or_else(|| panic!("no {action} entry in {text}"))
            .clone()
    };

    let watch = of("watch");
    assert_eq!(watch["pane"], pane.as_str());
    assert_eq!(watch["role"], "readonly");
    assert_eq!(
        watch["detail"]["scrollback"], true,
        "history is the whole terminal"
    );
    assert_eq!(of("unwatch")["pane"], pane.as_str());

    let managed = of("manage");
    assert_eq!(managed["detail"]["op"], "pane.split");
    assert_eq!(managed["detail"]["cwd"], "/tmp", "{managed}");
    assert_eq!(managed["detail"]["direction"], "right", "{managed}");

    let typed = of("input");
    assert_eq!(typed["pane"], pane.as_str());
    assert_eq!(
        typed["detail"],
        json!({ "keys": 7 }),
        "the log records how much was typed, never what: {typed}",
    );
    assert!(
        !text.contains("hunter2"),
        "typed text reached the audit log in the clear",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_pane_delivers_its_history_as_absolute_rows() {
    let h = harness!("history");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "seq 1 400\n" }),
    )
    .await;

    let history = until(&mut socket, "scrollback", 25).await;
    assert_eq!(history["pane"], pane.as_str());
    let rows = history["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "{history}");
    // Absolute ring indices, ascending — `row` is a u32 here rather than a viewport row.
    let indices: Vec<u64> = rows.iter().map(|r| r["row"].as_u64().unwrap()).collect();
    assert!(
        indices.windows(2).all(|w| w[0] < w[1]),
        "history rows are ordered"
    );
    assert_eq!(indices[0], history["from_top"].as_u64().unwrap());
    assert!(history["total_rows"].as_u64().unwrap() >= rows.len() as u64);
    assert!(history["complete"].is_boolean() && history["capped"].is_boolean());

    let text: String = rows
        .iter()
        .flat_map(|r| r["runs"].as_array().cloned().unwrap_or_default())
        .filter_map(|run| run["x"].as_str().map(str::to_string))
        .collect();
    assert!(text.contains("100"), "history did not carry the pane's output");
}

/// The columns a row's runs actually cover, `None` where a column is the right half of the
/// double-width glyph before it. This is the client's own reading of `w` (probe #210) and the
/// only honest way to ask whether a frame puts a wide glyph where a terminal does.
fn columns(runs: &Value) -> Vec<Option<char>> {
    let mut out = Vec::new();
    for run in runs.as_array().cloned().unwrap_or_default() {
        let w = run["w"].as_u64().unwrap_or(1);
        for ch in run["x"].as_str().unwrap_or("").chars() {
            out.push(Some(ch));
            for _ in 1..w {
                out.push(None);
            }
        }
    }
    out
}

/// The row as the client would build it: one entry per column, `None` where the column is the
/// right half of a wide cell, and each cell's `m` entry glued back onto its base.
fn clusters(runs: &Value) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for run in runs.as_array().cloned().unwrap_or_default() {
        let w = run["w"].as_u64().unwrap_or(1);
        let marks = run["m"].as_array().cloned().unwrap_or_default();
        for (i, ch) in run["x"].as_str().unwrap_or("").chars().enumerate() {
            let mut cell = ch.to_string();
            cell.push_str(marks.get(i).and_then(Value::as_str).unwrap_or(""));
            out.push(Some(cell));
            for _ in 1..w {
                out.push(None);
            }
        }
    }
    out
}

fn cluster_text(cols: &[Option<String>]) -> String {
    cols.iter()
        .flatten()
        .cloned()
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn text_of(cols: &[Option<char>]) -> String {
    cols.iter().flatten().collect::<String>().trim_end().to_string()
}

/// Probe #210, end to end against a real herdr rather than against an idea of one: herdr spends
/// two columns on a double-width glyph and addresses the next glyph at col+2, so a frame that
/// spends one leaves a blank behind every wide character for good.
#[tokio::test(flavor = "multi_thread")]
async fn a_wide_glyph_reaches_the_client_in_the_two_columns_herdr_gave_it() {
    let h = harness!("wide");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "printf '%s\\n' 'AB\u{65e5}\u{672c}\u{8a9e}CD' 'XY\u{1f680}ZW'\n" }),
    )
    .await;

    let mut grid: std::collections::HashMap<u64, Vec<Option<char>>> = std::collections::HashMap::new();
    let mut cjk = None;
    let mut emoji = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline && (cjk.is_none() || emoji.is_none()) {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        let rows = match message["t"].as_str() {
            Some("grid.reset") => {
                grid.clear();
                message["rows_data"].clone()
            }
            Some("grid.patch") => message["rows"].clone(),
            _ => continue,
        };
        for row in rows.as_array().cloned().unwrap_or_default() {
            let cols = columns(&row["runs"]);
            grid.insert(row["row"].as_u64().unwrap_or(0), cols);
        }
        cjk = grid
            .values()
            .find(|c| text_of(c) == "AB\u{65e5}\u{672c}\u{8a9e}CD")
            .cloned();
        emoji = grid.values().find(|c| text_of(c) == "XY\u{1f680}ZW").cloned();
    }

    let cjk = cjk.expect("no row came back reading AB\u{65e5}\u{672c}\u{8a9e}CD");
    assert_eq!(cjk[2], Some('\u{65e5}'));
    assert_eq!(
        cjk[3], None,
        "column 3 is the other half of \u{65e5}, not a blank"
    );
    assert_eq!(cjk[4], Some('\u{672c}'), "herdr addresses this glyph at column 5");
    assert_eq!(cjk[6], Some('\u{8a9e}'), "and this one at column 7");
    assert_eq!(cjk[8], Some('C'), "and the text after them at column 9");

    let emoji = emoji.expect("no row came back reading XY\u{1f680}ZW");
    assert_eq!(
        emoji[2],
        Some('\u{1f680}'),
        "one astral glyph, not two surrogate halves"
    );
    assert_eq!(emoji[3], None);
    assert_eq!(emoji[4], Some('Z'));
}

/// Probe #223, end to end against a real herdr rather than against an idea of one: herdr's cell is
/// a grapheme, it keeps the marks on the base and addresses the next glyph at base + the cluster's
/// width — so an emulator that drops the mark loses it for good, because herdr never repaints a
/// cell it believes already matches.
#[tokio::test(flavor = "multi_thread")]
async fn a_combining_mark_reaches_the_client_riding_on_its_base() {
    let h = harness!("marks");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;
    let script = "printf '%b\\n' 'e\\u0301f'                   'ZZ\\U0001F468\\u200d\\U0001F469\\u200d\\U0001F467XY'                   'QQ\\U0001F1EC\\U0001F1E7XY'\n";
    send(&mut socket, json!({ "t": "input", "pane": pane, "text": script })).await;

    let accent = "e\u{301}";
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let flag = "\u{1F1EC}\u{1F1E7}";
    let mut grid: std::collections::HashMap<u64, Vec<Option<String>>> = std::collections::HashMap::new();
    let (mut marked, mut joined, mut flagged) = (None, None, None);
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline && (marked.is_none() || joined.is_none() || flagged.is_none())
    {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        let rows = match message["t"].as_str() {
            Some("grid.reset") => {
                grid.clear();
                message["rows_data"].clone()
            }
            Some("grid.patch") => message["rows"].clone(),
            _ => continue,
        };
        for row in rows.as_array().cloned().unwrap_or_default() {
            grid.insert(row["row"].as_u64().unwrap_or(0), clusters(&row["runs"]));
        }
        let want = |t: &str| grid.values().find(|c| cluster_text(c) == t).cloned();
        marked = want(&format!("{accent}f"));
        joined = want(&format!("ZZ{family}XY"));
        flagged = want(&format!("QQ{flag}XY"));
    }

    let marked = marked.expect("no row came back reading e\u{301}f");
    assert_eq!(
        marked[0].as_deref(),
        Some(accent),
        "the accent rode in on its base"
    );
    assert_eq!(marked[1].as_deref(), Some("f"), "and f is still in column 1");

    let joined = joined.expect("no row came back reading ZZ<family>XY");
    assert_eq!(joined[2].as_deref(), Some(family), "one cell, not three emoji");
    assert_eq!(joined[3], None, "column 3 is the family's other half");
    assert_eq!(joined[4].as_deref(), Some("X"), "herdr addresses X at column 5");

    let flagged = flagged.expect("no row came back reading QQ<flag>XY");
    assert_eq!(
        flagged[2].as_deref(),
        Some(flag),
        "a flag is one cell of two columns"
    );
    assert_eq!(flagged[3], None);
    assert_eq!(flagged[4].as_deref(), Some("X"));
}

#[tokio::test(flavor = "multi_thread")]
async fn resync_repaints_every_watched_pane_and_unwatch_stops_one() {
    let h = harness!("resync");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    send(&mut socket, json!({ "t": "resync" })).await;
    until(&mut socket, "herd", 10).await;
    let repaint = until(&mut socket, "grid.reset", 15).await;
    assert_eq!(repaint["pane"], pane.as_str());

    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    // A frame the node had already sent when it read the `unwatch` is late, not a leak — dispatch
    // is sequential, so a pong sent after it is the client's proof that the unwatch has happened
    // and that anything arriving from here on was chosen after the pane was dropped. Without this
    // barrier the resync's own repaint, published a moment before, is counted as the leak.
    send(&mut socket, json!({ "t": "ping", "n": 8 })).await;
    until(&mut socket, "pong", 10).await;

    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "echo after-unwatch\n" }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    send(&mut socket, json!({ "t": "ping", "n": 9 })).await;

    let mut grids = 0;
    loop {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            break;
        };
        match message["t"].as_str() {
            Some("pong") => break,
            Some("grid.patch" | "grid.reset") => grids += 1,
            _ => {}
        }
    }
    assert_eq!(grids, 0, "an unwatched pane must stop streaming");
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_preferences_are_stored_per_device() {
    let h = harness!("prefs");
    let mut first = h.connect(&h.token(Role::Full).await).await;
    let mut second = h.connect(&h.token(Role::Full).await).await;
    until(&mut first, "hello", 10).await;
    until(&mut second, "hello", 10).await;
    for socket in [&mut first, &mut second] {
        assert_eq!(
            until(socket, "prefs", 10).await["panes"],
            json!({}),
            "the greeting's prefs, before either device has stored any"
        );
    }
    let pane = h.pane_id();

    send(
        &mut first,
        json!({ "t": "prefs", "pane": pane, "prefs": { "zoom": 1.75, "follow": true } }),
    )
    .await;
    let saved = until(&mut first, "prefs", 10).await;
    assert_eq!(saved["panes"][&pane]["zoom"], 1.75);

    send(&mut second, json!({ "t": "prefs" })).await;
    let other = until(&mut second, "prefs", 10).await;
    assert_eq!(
        other["panes"],
        json!({}),
        "a second device does not inherit the first device's zoom"
    );
}

/// The `pending` path end to end.
///
/// Claude publishes nothing about a pending request until after it is answered (probe #42), so the
/// question is read off the screen and `source` says `"screen"`. This drives that with herdr's own
/// `pane.report_agent`: a real pane, a real blocked agent status, a real prompt on the screen.
#[tokio::test(flavor = "multi_thread")]
async fn a_blocked_agent_pane_publishes_the_question_from_the_screen() {
    let h = harness!("pending");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    // Put a permission prompt on the screen, the way a harness would — **including the blank line
    // above the question**, which every real dialog has (#407) and which is what makes this test
    // independent of the machine it runs on. Without it the row above the question is the shell's
    // echo of this very command, and whether `question_above` joins that echo onto the question
    // turns on how wide the ambient `PS1` is (#406): at CI's 27 columns the echo does not wrap, so
    // it is one row, it is joined, and the assertion below reads back the whole command line; at
    // this desk's 35 it wraps to a two-character stub that `prose` happens to reject. Three of the
    // four prompt widths measured are red, and the one that passes does so by one character.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane,
                "text": "printf '\\nDo you want to make this edit?\\n\\n 1. Yes\\n 2. No\\n'\n" }),
    )
    .await;
    // **On the screen, not on the clock, and on both the screens the node reads.** A pane reported
    // blocked before its dialog has finished painting is a pane the node reads with no question on
    // it, and it retries a bounded twelve times over six seconds before it stops asking (#421's
    // `PENDING_ATTEMPTS`) — after which nothing re-arms it, because the status does not move
    // again. 1.2 s covered the paint on an idle box and not under sixteen spinners.
    assert!(
        drawn(&h, &local, "2. No", 60).await,
        "the dialog never reached the screen",
    );

    report(&h._session, &local, "blocked").await;
    until_herd(&h, &pane, "blocked").await;

    let pending = until_pane(&mut socket, "pending", &pane, 25).await;
    assert_eq!(pending["pane"], pane.as_str());
    assert_eq!(
        pending["source"], "screen",
        "probe #42: the transcript is not the source"
    );
    assert_eq!(pending["question"], "Do you want to make this edit?");
    let options = pending["options"].as_array().unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0]["key"], "1");
    assert_eq!(options[0]["label"], "Yes");

    // And the press lands. Nothing covered the happy path of `answer` at all — only the two
    // refusals — while the report that found it was a chip that pressed and delivered nothing
    // (#414). The key goes on the end of a line the shell is already holding, so what arrives is
    // unambiguous: a bare `1` is a character the dialog above it already put on the screen.
    // `claude` takes no submit key (#413), so the line stays unexecuted where it can be read.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "echo kampr-answer-" }),
    )
    .await;
    // The line has to be on the pane before the key goes on the end of it, or what comes back is
    // the two in the other order and the assertion below reads a screen that never held the word.
    assert!(
        drawn(&h, &local, "echo kampr-answer-", 60).await,
        "the line the answer key lands on never reached the pane",
    );
    send(&mut socket, json!({ "t": "answer", "pane": pane, "key": "1" })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut landed = false;
    while tokio::time::Instant::now() < deadline && !landed {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        landed = message.to_string().contains("kampr-answer-1");
    }
    assert!(landed, "the key an answer carries never reached the pane's pty");

    // Leaving the blocked state clears the strip — the only way a client can know to drop it.
    report(&h._session, &local, "idle").await;
    let cleared = until_pane(&mut socket, "pending", &pane, 25).await;
    assert!(cleared["question"].is_null(), "{cleared}");
    assert!(cleared["options"].as_array().unwrap().is_empty());
}

/// The desk half of W9, against a real herdr: the name Kampr computes reaches herdr's own
/// metadata table and comes back on `pane.get` as `title` (probe #294).
///
/// **The read-back is the assertion, not the ack.** `pane.report_metadata` answers `ok` to a
/// report it silently dropped (probe #295), so a test that watched the call succeed would go green
/// against a reporter that never landed a single name.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_told_to_report_names_puts_one_on_the_pane_at_the_desk() {
    let h = harness!("naming-on", |config: &mut Config| {
        config.naming.report_to_herdr = true;
    });
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    // The workspace this harness creates is labelled `kampr`, and the harness leaves its pane at a
    // shell with no job in it — so the command section drops and the name is what is left. That
    // the name *follows* a job rather than latching on the first one read is
    // `kampr-core`'s `a_panes_command_is_re_read_on_every_sweep_because_a_job_starting_has_no_cue`,
    // which can move a process under the provider on demand where this cannot.
    let mut title = Value::Null;
    let mut saw: Vec<String> = Vec::new();
    for _ in 0..60 {
        title = h._session.call("pane.get", json!({ "pane_id": local })).await["pane"]["title"].clone();
        if title == "kampr · bash" {
            break;
        }
        if let Some(said) = title.as_str()
            && saw.last().map(String::as_str) != Some(said)
        {
            saw.push(said.to_string());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        title, "kampr · bash",
        "pane.get said {title}; the names before it were {saw:?}"
    );

    // The same name goes in as a *token*, which is the only field herdr will sort its agents
    // sidebar on: the sortable builtins are `agent` and `status`, and `title` is not one of them.
    let pane = h._session.call("pane.get", json!({ "pane_id": local })).await;
    assert_eq!(
        pane["pane"]["tokens"][kampr_core::reporter::TOKEN],
        "kampr · bash",
        "pane.get said {pane}"
    );
}

/// The node half of the sidebar sort, end to end and through the config that gates it: two
/// settings on, a real herdr, and the desk put back on the way out.
///
/// **The clear is the half nothing can assert here**, because herdr will not say what view it is
/// holding — there is no `agent.view.get` and `agent.list` is untouched by the view (probe #296).
/// What this proves is that the whole path reaches a real herdr without being refused; that the
/// clear is *sent*, and sent exactly once and never by a node that set nothing, is
/// `kampr-core`'s `reporting.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_told_to_sort_a_desk_sorts_it_and_puts_it_back_on_the_way_out() {
    let h = harness!("desk-sort", |config: &mut Config| {
        config.naming.report_to_herdr = true;
        config.naming.sort_desk_agents = true;
    });
    assert!(
        h.node.config.naming.desk_agents().is_some(),
        "both settings on is the only combination that sorts anything"
    );
    // Long enough for the sweep that sets the view; a refusal would be a `warn!` and nothing else,
    // so the assertion that matters is the clear below going through on a herdr that took the set.
    tokio::time::sleep(Duration::from_secs(2)).await;
    h.node.restore_desks().await;

    let again = h
        ._session
        .herdr()
        .clear_agent_view()
        .await
        .expect("herdr answers a clear");
    assert!(!again.active);
}

/// The same node with the setting left alone. Looking at somebody's herd writes nothing into it,
/// which is ADR 0002's invariant and the whole reason the other test needs a flag.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_nobody_asked_leaves_the_desk_exactly_as_it_found_it() {
    let h = harness!("naming-off");
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    // Long enough for several sweeps of a node that was going to report.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let got = h._session.call("pane.get", json!({ "pane_id": local })).await;
    assert!(got["pane"]["title"].is_null(), "{got}");
}

/// A forged `X-Forwarded-For` must not hand an attacker a fresh rate-limit bucket per guess. The
/// header is believed only when `trust_proxy` is set, and it is never inferred.
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_forwarded_header_does_not_buy_a_fresh_rate_limit_bucket() {
    let h = harness!("proxy");
    assert!(!h.node.config.server.trust_proxy, "never inferred");

    let mut limited = false;
    for n in 0..24 {
        let (status, _) = post_as(
            &format!("{}/auth/pair", h.origin),
            &json!({ "code": "ZZZZ-ZZZZ", "device_name": "attacker" }),
            Some(&format!("10.0.0.{n}")),
        )
        .await;
        if status.contains("429") {
            limited = true;
            break;
        }
    }
    assert!(
        limited,
        "rotating X-Forwarded-For escaped the limiter, so the header was trusted"
    );
}

/// Own-TLS is the alternative to a reverse proxy: the node terminates it itself. It is only half
/// of what moves a node off Tier 0 — a certificate for a *hostname* is the other half — so this
/// proves the transport works and that HTTPS alone still buys no passkeys.
#[tokio::test(flavor = "multi_thread")]
async fn the_node_can_terminate_tls_itself() {
    let Some(session) = Session::start("tls").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    session
        .call("workspace.create", json!({ "label": "tls", "cwd": "/tmp" }))
        .await;

    let state = tempfile::tempdir().unwrap();
    let certified = rcgen::generate_simple_self_signed(vec!["kampr.test".into()]).unwrap();
    let cert = state.path().join("cert.pem");
    let key = state.path().join("key.pem");
    std::fs::write(&cert, certified.cert.pem()).unwrap();
    std::fs::write(&key, certified.signing_key.serialize_pem()).unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut config = Config::bootstrap("tlsnode");
    // Nothing in this suite reaches the internet: the release check is the one thing in a
    // node that would, and a test that phoned GitHub would be one with a rate limit.
    config.update.check = false;
    config.server.bind = format!("127.0.0.1:{port}");
    config.server.origin = format!("https://kampr.test:{port}");
    config.server.tls.enabled = true;
    config.server.tls.cert = cert.display().to_string();
    config.server.tls.key = key.display().to_string();
    config.herdr.socket = session.socket.display().to_string();
    config.save(&state.path().join("config")).unwrap();

    let node = Node::start(config, state.path()).await.unwrap();
    // A hostname *with* a certificate is Tier 1 — this is the rung Tier 0 cannot reach.
    assert_eq!(node.auth.tier().tier, 1);
    assert!(node.auth.tier().passkeys);
    assert_eq!(node.auth.tier().rp_id.as_deref(), Some("kampr.test"));

    let server = tokio::spawn(http::serve(node.clone()));
    tokio::time::sleep(Duration::from_millis(600)).await;

    let body = tls_get(port, "kampr.test", "/api/node").await;
    assert!(
        body.to_lowercase().contains("strict-transport-security"),
        "a node that has TLS should say so: {body}"
    );
    assert!(body.contains("\"tier\":1"), "{body}");
    assert!(body.contains("\"passkeys\":true"), "{body}");
    assert!(body.contains("tlsnode"), "{body}");
    server.abort();
}

/// A minimal HTTPS GET that pins the node's own self-signed certificate.
async fn tls_get(port: u16, host: &str, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[derive(Debug)]
    struct TrustTheNode;
    impl rustls::client::danger::ServerCertVerifier for TrustTheNode {
        fn verify_server_cert(
            &self,
            _e: &rustls::pki_types::CertificateDer<'_>,
            _i: &[rustls::pki_types::CertificateDer<'_>],
            _n: &rustls::pki_types::ServerName<'_>,
            _o: &[u8],
            _t: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _m: &[u8],
            _c: &rustls::pki_types::CertificateDer<'_>,
            _d: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _m: &[u8],
            _c: &rustls::pki_types::CertificateDer<'_>,
            _d: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TrustTheNode))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string()).unwrap();
    let mut tls = connector
        .connect(server_name, stream)
        .await
        .expect("tls handshake");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = tls.read_to_end(&mut response).await;
    String::from_utf8_lossy(&response).to_string()
}

// ---------------------------------------------------------------------------
// Binding before herdr, and making an outage visible.
// ---------------------------------------------------------------------------

/// Serves on a listener the caller already owns and hands back the origin.
async fn serve_config(
    config: Config,
    state: &tempfile::TempDir,
) -> (Arc<Node>, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = config;
    config.server.bind = format!("127.0.0.1:{port}");
    config.server.origin = format!("http://127.0.0.1:{port}");
    config.save(&state.path().join("config")).unwrap();
    let origin = config.origin();
    let node = Node::start(config, state.path())
        .await
        .expect("the node must start");
    let server = tokio::spawn({
        let app = http::router(node.clone());
        async move {
            let _ = http::serve_on(listener, app).await;
        }
    });
    (node, origin, server)
}

async fn pair(node: &Arc<Node>, origin: &str) -> String {
    let pairing = node
        .auth
        .create_pairing(Role::Full, kampr_auth::Delivery::Console)
        .await
        .unwrap();
    if !pairing.armed {
        assert!(node.auth.arm_pairing(&pairing.code).await.unwrap());
    }
    let body = json!({ "code": pairing.code, "device_name": "integration" });
    post(&format!("{origin}/auth/pair"), &body).await["token"]
        .as_str()
        .expect("a token")
        .to_string()
}

async fn open(origin: &str, token: &str) -> Socket {
    let url = origin.replacen("http", "ws", 1) + "/ws";
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("kampr.token.{token}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .expect("upgrade")
        .0
}

/// The whole point of task 1: with no herdr at all the node still binds, still serves, and still
/// says *why* the herd is empty. It used to spend 30 s refusing connections and then exit.
#[tokio::test(flavor = "multi_thread")]
async fn the_port_is_bound_before_herdr_is_needed() {
    let state = tempfile::tempdir().unwrap();
    let mut config = Config::bootstrap("lonely");
    // Nothing in this suite reaches the internet: the release check is the one thing in a
    // node that would, and a test that phoned GitHub would be one with a rate limit.
    config.update.check = false;
    config.herdr.socket = state.path().join("nothing-here.sock").display().to_string();
    // Only the configured session, which does not exist. Otherwise this node discovers every
    // other test's throwaway herd and the herd is not empty for reasons that are not the point.
    config.herdr.sessions = Some(Vec::new());

    let started = tokio::time::Instant::now();
    let (node, origin, server) = serve_config(config, &state).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "Node::start must not wait on herdr, took {:?}",
        started.elapsed()
    );

    let (status, body) = get(&format!("{origin}/api/node"), None).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["node_name"], "lonely");

    let token = pair(&node, &origin).await;
    let mut socket = open(&origin, &token).await;
    until(&mut socket, "hello", 10).await;
    let herd = until(&mut socket, "herd", 10).await;
    assert_eq!(herd["panes"].as_array().unwrap().len(), 0, "an empty herd");
    assert_eq!(herd["nodes"][0]["online"], false, "and it says it is offline");
    assert!(
        herd["nodes"][0]["detail"]
            .as_str()
            .is_some_and(|d| d.contains("nothing-here.sock")),
        "the node must say why: {}",
        herd["nodes"][0]
    );
    assert!(
        herd["nodes"][0]["herdr_version"].is_null(),
        "no version was ever learned"
    );
    server.abort();
}

/// Probe #70: stopping herdr under a live watcher produced no error and left `online` true. Both
/// documented codes now fire, and the herd comes back on its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_herdr_outage_reaches_the_client_and_recovers() {
    let h = harness!("outage");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    h._session.stop().await;

    let mut saw_unavailable = false;
    let mut saw_node_offline = false;
    let mut saw_offline_node = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && !(saw_unavailable && saw_node_offline && saw_offline_node)
    {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        match message["t"].as_str() {
            // **Both name the node they are about.** Without that a client has nothing to decide
            // with and shows every outage as a modal strip over whatever screen is open — so a
            // node the operator is not looking at interrupts a pane on a different one, on a
            // phone. The node cannot know which pane is on screen; naming itself is what lets the
            // client know whether this is the thing in the operator's hands or the herd.
            Some("error") => match message["code"].as_str() {
                Some("herdr_unavailable") => {
                    saw_unavailable = true;
                    assert!(
                        message["node"].as_str().is_some_and(|n| !n.is_empty()),
                        "herdr_unavailable named no node: {message}"
                    );
                }
                Some("node_offline") => {
                    saw_node_offline = true;
                    assert!(
                        message["node"].as_str().is_some_and(|n| !n.is_empty()),
                        "node_offline named no node: {message}"
                    );
                }
                _ => {}
            },
            Some("herd.patch") => {
                saw_offline_node |= message["changed"]["nodes"]
                    .as_array()
                    .is_some_and(|nodes| nodes.iter().any(|n| n["online"] == false));
            }
            _ => {}
        }
    }
    assert!(saw_unavailable, "an outage must produce herdr_unavailable");
    assert!(saw_node_offline, "and node_offline");
    assert!(saw_offline_node, "and flip herd.nodes[].online");

    h._session.respawn().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut back = false;
    while tokio::time::Instant::now() < deadline && !back {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        back = match message["t"].as_str() {
            Some("herd") => message["nodes"][0]["online"] == true,
            Some("herd.patch") => message["changed"]["nodes"]
                .as_array()
                .is_some_and(|nodes| nodes.iter().any(|n| n["online"] == true)),
            _ => false,
        };
    }
    assert!(back, "the node must come back online on its own");
}

/// How the herdr went away, which is the whole difference between the two tests below.
enum Outage {
    /// `server.stop`, which unlinks the socket on its way out.
    Clean,
    /// SIGKILL, which leaves the socket file on disk with nothing listening on it (#427).
    Killed,
}

/// The node is up, herdr is not, and the operator taps New. Probe #324 says one spawn spelling
/// starts either kind of server and #325 says racing it is harmless, so the op starts the herdr it
/// needs rather than refusing — and waits for an answered call, which per #326 is the only thing
/// that means the herdr it just started can serve the op.
async fn a_manage_op_wakes_the_herdr(h: &Harness, outage: Outage, label: &str) {
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let node = h.node.node_id().to_string();

    match outage {
        Outage::Clean => h._session.stop().await,
        Outage::Killed => h._session.kill().await,
    }
    assert!(
        h.offline(20).await,
        "the node never noticed its herdr had gone, so this proves nothing"
    );

    let created = ok(
        &mut socket,
        json!({ "t": "manage", "op": "workspace.create", "node": node,
                "label": label, "cwd": "/tmp" }),
        30,
    )
    .await;

    // The ack is not the claim. Herdr is answering again, and it is holding the workspace the op
    // said it made — a `managed{ok}` for a workspace nothing has is exactly the shape of #233.
    let snapshot = h._session.herdr().snapshot().await.expect("herdr answers again");
    assert!(
        snapshot
            .workspaces
            .iter()
            .any(|w| w.label.as_deref() == Some(label)),
        "the woken herdr does not have the workspace the op acked: {created}"
    );
}

/// The rarely-visited host, stopped the way a person stops one.
#[tokio::test(flavor = "multi_thread")]
async fn a_manage_op_on_a_node_whose_herdr_is_stopped_starts_it_rather_than_refusing() {
    let h = harness!("wake");
    a_manage_op_wakes_the_herdr(&h, Outage::Clean, "woken").await;
}

/// **The rung an actual reboot lands on**, and the one the test above cannot reach: its `stop()`
/// waits for the socket to disappear, so it only ever exercises the clean path. A machine that
/// went down hard comes back with `herdr.sock` still on disk and nothing behind it — probe #427
/// measured that `herdr session list --json` still reports `running: false` there, because herdr
/// decides that **by connecting, not by `stat`**, and that a detached server takes the stale path
/// over in 54 ms. So a stale socket is not a herdr, and `wake()` must not read it as one.
#[tokio::test(flavor = "multi_thread")]
async fn a_manage_op_over_a_stale_herdr_socket_starts_it_rather_than_refusing() {
    let h = harness!("stale");
    a_manage_op_wakes_the_herdr(&h, Outage::Killed, "revived").await;
}

/// Kampr must not start a herdr nobody asked it to. Watching, polling and reconnecting are not
/// requests; only a manage op is.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_left_alone_never_starts_the_herdr_it_is_missing() {
    let h = harness!("no-wake");
    h._session.stop().await;
    assert!(h.offline(20).await, "the node never noticed the outage");

    tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(
        !h._session.socket.exists(),
        "the node started herdr on its own, with nobody asking for anything"
    );
}

// ---------------------------------------------------------------------------
// Every session on the host.
// ---------------------------------------------------------------------------

/// A named session is a separate herdr server, so it is a separate node — not an invisible one.
#[tokio::test(flavor = "multi_thread")]
async fn every_herdr_session_on_the_host_is_its_own_node() {
    let Some(first) = Session::start("multi-a").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    let Some(second) = Session::start("multi-b").await else {
        return;
    };
    first
        .call("workspace.create", json!({ "label": "one", "cwd": "/tmp" }))
        .await;
    second
        .call("workspace.create", json!({ "label": "two", "cwd": "/tmp" }))
        .await;

    let state = tempfile::tempdir().unwrap();
    let mut config = Config::bootstrap("multinode");
    // Nothing in this suite reaches the internet: the release check is the one thing in a
    // node that would, and a test that phoned GitHub would be one with a rate limit.
    config.update.check = false;
    config.herdr.socket = first.socket.display().to_string();
    config.herdr.sessions = Some(vec![second.name.clone()]);
    let (node, origin, server) = serve_config(config, &state).await;

    let base = node.config.node_id.clone();
    let extra = format!("{base}.{}", second.name);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        let herd = node.herd();
        if herd.nodes.len() == 2
            && herd.panes.iter().any(|p| p.node_id == extra)
            && herd.panes.iter().any(|p| p.node_id == base)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let token = pair(&node, &origin).await;
    let mut socket = open(&origin, &token).await;
    until(&mut socket, "hello", 10).await;
    let mut herd = until(&mut socket, "herd", 10).await;
    let both = |herd: &Value| {
        [base.as_str(), extra.as_str()].iter().all(|id| {
            herd["nodes"]
                .as_array()
                .is_some_and(|n| n.iter().any(|n| n["id"] == *id))
                && herd["panes"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|p| p["node_id"] == *id))
        })
    };
    for _ in 0..60 {
        if both(&herd) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        send(&mut socket, json!({ "t": "resync" })).await;
        herd = until(&mut socket, "herd", 10).await;
    }

    let ids: Vec<String> = herd["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.contains(&base),
        "the configured session keeps the bare node id: {ids:?}"
    );
    assert!(
        ids.contains(&extra),
        "the second session is its own node: {ids:?}"
    );

    let panes: Vec<String> = herd["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        panes.iter().any(|p| p.starts_with(&format!("{base}/"))),
        "the first session's panes: {panes:?}"
    );
    assert!(
        panes.iter().any(|p| p.starts_with(&format!("{extra}/"))),
        "the second session's panes: {panes:?}"
    );

    // Ids from two sessions must be distinguishable, and a pane must be driven on its own herdr.
    let target = panes
        .iter()
        .find(|p| p.starts_with(&format!("{extra}/")))
        .unwrap()
        .clone();
    send(&mut socket, json!({ "t": "watch", "pane": target })).await;
    let reset = until(&mut socket, "grid.reset", 20).await;
    assert_eq!(reset["pane"], target.as_str());

    // Dropping a session takes its node down without taking the herd with it.
    second.stop().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut fell_over = false;
    while tokio::time::Instant::now() < deadline && !fell_over {
        fell_over = node.herd().nodes.iter().any(|n| n.id == extra && !n.online);
    }
    assert!(fell_over, "the second session going away is one node offline");
    assert!(
        node.herd().nodes.iter().any(|n| n.id == base && n.online),
        "and never the first"
    );
    server.abort();
}

// ---------------------------------------------------------------------------
// Observing at the PTY's real width.
// ---------------------------------------------------------------------------

/// Probe #68/#84. In a headless session the PTY does not follow the layout rect, so observing at
/// the rect crops every row. The width is derived from what `pane.read visible` renders instead.
#[tokio::test(flavor = "multi_thread")]
async fn a_split_pane_is_observed_at_the_pty_width_not_the_rect() {
    let h = harness!("ptywidth");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    // A line far wider than either rect, so the PTY's own wrap width is on the screen.
    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": "clear; printf '%.0s#' $(seq 1 400); echo\n" }),
        )
        .await;
    let pty = filled_width(&h._session, &local).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    await_grid_at(&mut socket, &pane, pty).await;

    let rect_before = rect_width(&h._session, &local).await;
    h._session
        .call(
            "pane.split",
            json!({ "target_pane_id": local, "direction": "right" }),
        )
        .await;
    let rect_after = a_halved_rect(&h._session, &local, rect_before).await;
    assert!(
        rect_after < rect_before,
        "the split must have halved the rect: {rect_before} -> {rect_after}"
    );
    assert_eq!(
        widest_rendered_row(&h._session, &local).await,
        pty,
        "probe #68: the PTY did not follow the rect"
    );

    // The node must follow the PTY, not the rect it was just handed.
    await_grid_at(&mut socket, &pane, pty).await;
}

/// The rect after a split, waited for. A split is a request to herdr and the layout it produces
/// arrives on herdr's own clock; three seconds covered that on an idle box, and the assertion
/// that follows is about what the *PTY* did once the rect moved, which is nothing at all until it
/// has. The give-up returns the last rect it saw so the assertion still names the failure.
async fn a_halved_rect(session: &Session, pane: &str, was: u16) -> u16 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut rect = was;
    while tokio::time::Instant::now() < deadline {
        rect = rect_width(session, pane).await;
        if rect < was {
            return rect;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    rect
}

/// Probe #211. The width prober can prove a width nothing ever wrapped at, and the proof used to
/// be sticky. A screen of double-width glyphs is the sharpest case: every row is the PTY's width
/// in columns and half of it in characters, so counting characters called the pane half as wide
/// as it is, and the stream was restarted at that width and stayed there.
///
/// What is asserted is the contract rather than a number — the node must never stream narrower
/// than the PTY. A wrap made by a wide glyph cannot say whether the last column was used or was
/// too narrow to hold one, so a screen with nothing but wide glyphs on it is allowed to read one
/// column wide; it is never allowed to read narrow.
#[tokio::test(flavor = "multi_thread")]
async fn a_screen_of_wide_glyphs_is_not_streamed_at_half_the_pty_width() {
    let h = harness!("cjkwidth");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": "clear; printf '%.0s#' $(seq 1 400); echo\n" }),
        )
        .await;
    let pty = filled_width(&h._session, &local).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    await_grid_at(&mut socket, &pane, pty).await;

    // The phases #211 was driven through, ending on the one that has no ASCII wrap left on it.
    for phase in [
        "clear; for i in $(seq 1 60); do printf '\\r'; printf '=%.0s' $(seq 1 $i); done; echo\n",
        "clear; seq 1 3000\n",
        "clear; printf '%.0s\u{65e5}' $(seq 1 200); echo\n",
    ] {
        h._session
            .call("pane.send_text", json!({ "pane_id": local, "text": phase }))
            .await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // Four width polls with nothing but wide glyphs on the screen, and then a fresh watch, which
    // repaints from the pane as it is now (probe #209) and so carries the width the node has
    // settled on rather than the one it started at.
    let narrow = drain_resets(&mut socket, &pane, Duration::from_secs(12)).await;
    assert!(
        narrow.iter().all(|&cols| cols >= pty),
        "the node streamed a {pty}-column pane at {narrow:?}"
    );

    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let after = until_pane(&mut socket, "grid.reset", &pane, 30).await["cols"]
        .as_u64()
        .unwrap() as u16;
    assert_eq!(
        after, pty,
        "a pane of wide glyphs came back at {after} columns against a PTY of {pty}"
    );
}

/// The residual of [#218](#), measured out (probe #220): a screen of nothing but wide glyphs
/// reads *identically* on a grid of `2n` and one of `2n + 1` — the same rows and the same logical
/// line, byte for byte at 92 columns and at 93 — because half a glyph will not sit in the last
/// column. So the break cannot settle its own width, and an even-width pane was streamed a column
/// wide for as long as the CJK stayed on it. It does not have to settle it: the ASCII this pane
/// wrapped a moment earlier did, and a break the standing proof agrees with confirms that width
/// rather than widening it.
#[tokio::test(flavor = "multi_thread")]
async fn an_even_width_pane_of_wide_glyphs_keeps_the_width_its_wrap_proved() {
    let h = harness!("cjkeven");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    control_cols(&h._session, &local, 92).await;

    // A prompt short enough to leave the bottom of the screen alone. An interactive prompt comes
    // back from `recent_unwrapped` glued to the blanks around it as one logical line no set of
    // rows rebuilds, and the walk stops at the bottom-most line it cannot rebuild — so the whole
    // reading measures nothing and neither rule is exercised.
    for line in ["exec /bin/sh\n", "PS1='$ '\n", "clear\n"] {
        typed(&h._session, &local, line).await;
    }
    for _ in 0..4 {
        typed(&h._session, &local, "printf '%.0s#' $(seq 1 400); echo\n").await;
    }
    let pty = filled_width(&h._session, &local).await;
    assert_eq!(pty, 92, "the controller left the PTY at an even width");

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    await_grid_at(&mut socket, &pane, pty).await;

    // Enough wide-glyph lines to push every ASCII row out of the read window: one unambiguous
    // break left in it settles the width on its own and proves nothing about the ambiguous ones.
    typed(&h._session, &local, "clear\n").await;
    for _ in 0..8 {
        typed(&h._session, &local, "printf '%.0s\u{65e5}' $(seq 1 200); echo\n").await;
    }
    tokio::time::sleep(Duration::from_secs(6)).await;

    let widths = drain_resets(&mut socket, &pane, Duration::from_secs(12)).await;
    assert!(
        widths.iter().all(|&cols| cols == pty),
        "the node moved a {pty}-column pane to {widths:?}"
    );

    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let after = until_pane(&mut socket, "grid.reset", &pane, 30).await["cols"]
        .as_u64()
        .unwrap() as u16;
    assert_eq!(
        after, pty,
        "a pane of wide glyphs came back at {after} columns against a PTY of {pty}"
    );
}

/// The one op that reshapes a pane knows the width it asked for, and until it said so the node
/// went on streaming the pane at the width it had *inferred* — a measurement only a wrap can
/// repeat (probe #84). A full-screen agent draws no wrap, so nothing re-proved the new width and
/// every client went on laying out the old one until the agent printed a message long enough to
/// wrap. That is the report: "match this view works, but it doesn't horizontally update until a
/// message goes through, so typing a prompt ends up typing off the visible cols."
///
/// **The screen is cleared before the resize on purpose.** Herdr reflows what is already on the
/// pane when the PTY moves, so a pane with a wrapped line still on it re-proves its own width on
/// the very next poll and would pass this test with the defect restored. A cleared prompt is what
/// the width walk sees on an agent's screen: nothing to measure, and a stale proof standing.
#[tokio::test(flavor = "multi_thread")]
async fn a_resized_pane_is_streamed_at_the_width_it_was_given_before_anything_wraps_again() {
    let h = harness!("sizedcols");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    // **No shell on the pane at all.** The probe line used to be typed at the operator's own login
    // prompt, and that shell loads ble.sh, which re-renders the accepted command line on its own
    // schedule — under load it repaints over whatever was drawn, and a `PS1` set in one burst with
    // the `exec` in front of it is one multi-line buffer the editor is still holding rather than
    // two commands it ran. A painter draws the probe and takes it away again with nothing else
    // ever touching the screen, which is also a *closer* reading of the case this test is about:
    // an agent's pane with nothing on it to measure.
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 15).await;
    a_painter_on_the_pane(&h, &mut socket, &pane, &local).await;

    // 400 `#` cannot fit on one row of any plausible pane, so what reaches the screen is a wrap.
    paint_screen(&mut socket, &pane, &"#".repeat(400)).await;
    let before = filled_width(&h._session, &local).await;
    a_pane_the_node_streams_at(&h, &pane, before).await;

    // Now take the wrap away, and wait for it to be gone rather than allowing a fixed moment for
    // it: herdr reflows what is on the pane when the PTY moves, so a wrap still standing when the
    // resize lands re-proves the new width on the next poll and this test passes with the defect
    // restored. What is left is a prompt no set of rows rebuilds and no line that spans two of
    // them, which is exactly the reading that measures nothing.
    paint_screen(&mut socket, &pane, "").await;
    a_screen_with_no_wrap_on_it(&h._session, &local).await;

    let wider = before + 24;
    let taller = viewport_rows(&h._session, &local).await + 1;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": pane, "cols": wider, "rows": taller }),
    )
    .await;

    // The ack and the grid are collected in one pass: the width the node adopts is pushed the
    // moment the resize lands, so it can arrive either side of the ack.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut ack = None;
    let mut widths = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "managed" && message["op"] == "pane.size" {
            ack = Some(message);
            continue;
        }
        if message["t"] == "grid.reset" && message["pane"] == pane {
            widths.push(message["cols"].as_u64().unwrap_or(0) as u16);
            if widths.last() == Some(&wider) {
                break;
            }
        }
    }
    let ack = ack.expect("no managed ack for pane.size");
    assert_eq!(ack["ok"], json!(true), "{ack}");

    // **The environment half, and it is asked separately so a failure names the right half.**
    // Columns are reported by nothing anywhere (#221) and rows are the half herdr answers
    // honestly (#84), so the rows landing is the only evidence the claim took at all — it is what
    // `size_pane` itself decides `kept` by, and the wire does not carry that. Without this, a
    // resize herdr never applied read as the node refusing to stream a width it was never given.
    let landed = viewport_rows(&h._session, &local).await;
    assert_eq!(
        landed, taller,
        "herdr never took the resize: the pane is {landed} rows, not {taller}, so there is no \
         width for the node to have adopted"
    );

    // The clause the whole test turns on: nothing on this pane has wrapped since the resize, so
    // the only thing that could have carried the new width is the resize itself.
    let widest = widest_rendered_row(&h._session, &local).await;
    assert!(
        widest < before,
        "the pane printed a {widest}-column row, which re-proves its own width and makes this \
         test worthless"
    );
    assert!(
        widths.contains(&wider),
        "a pane resized to {wider} columns was still streamed at {widths:?}"
    );
}

/// Waits until the node has settled on `cols` for this pane, read off the herd it publishes.
///
/// **Not a `grid.reset`.** A reset is what a stream re-opening produces, so waiting for one is
/// waiting for the node to have *guessed wrong first*: the first observe is sized from the layout
/// rect and a proof that disagrees restarts it. A pane whose rect already equals its PTY — which
/// is every pane herdr spawns at its layout width rather than at the headless 120 and then
/// reflows (#452) — is guessed right, streams one reset before anything is painted on it, and never
/// produces another. The claim this stands in for is the baseline the resize below moves off:
/// the width the node is streaming at, which the herd carries.
/// **The release nobody sent.**
///
/// A matched hold is claimed by a terminal view and let go when that view closes — but a closed
/// laptop, a crashed tab and a dropped link never send the closing half. So the hold is owned by
/// the websocket rather than by anything the client remembers to do, and this drops the socket
/// without a word to prove it: no `release`, no close frame, just a client that stops being there.
///
/// The wrap is not decoration. A width is proved by a wrap and by nothing else (#84, #221), so a
/// pane that has never wrapped has no geometry honest enough to put back and the node deliberately
/// records none — this is what makes the restore possible at all. See
/// [ADR 0013](../../../../docs/adr/0013-a-standing-intent-to-match-the-view.md).
#[tokio::test(flavor = "multi_thread")]
async fn a_matched_pane_is_put_back_when_the_socket_holding_it_stops_answering() {
    let h = harness!("matchdrop");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 15).await;
    a_painter_on_the_pane(&h, &mut socket, &pane, &local).await;
    paint_screen(&mut socket, &pane, &"#".repeat(400)).await;
    let found_cols = filled_width(&h._session, &local).await;
    a_pane_the_node_streams_at(&h, &pane, found_cols).await;
    let found_rows = viewport_rows(&h._session, &local).await;

    let wider = found_cols + 24;
    let taller = (found_rows + 3).max(30);
    let ack = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": pane,
                "cols": wider, "rows": taller, "mode": "match" }),
        30,
    )
    .await;
    assert_eq!(ack["held"], json!(true), "{ack}");
    assert_eq!(ack["matched"], json!(true), "{ack}");
    // The ack says what it will put back, or says nothing — never a rect, which is fiction (#68).
    assert_eq!(
        (&ack["found_cols"], &ack["found_rows"]),
        (&json!(found_cols), &json!(found_rows)),
        "the ack does not say what the release will put back: {ack}",
    );
    assert!(
        rows_settle_at(&h._session, &local, taller, 20).await,
        "the match never landed, so there is nothing for the release to undo; the pane is {} rows",
        viewport_rows(&h._session, &local).await,
    );

    // No goodbye. The socket simply stops.
    drop(socket);

    assert!(
        rows_settle_at(&h._session, &local, found_rows, 45).await,
        "a pane held at the size of a window nobody is looking at any more was left there; it is \
         {} rows and it was found at {found_rows}",
        viewport_rows(&h._session, &local).await,
    );
}

/// The other half of the rule, and the half that answers #298: a release puts the pane back
/// **only** if the pane is still the shape this hold made it. Something else deliberately moving
/// it — the operator through the panel, another client, herdr — is the last word, and a viewer
/// closing a window it had open must not undo that behind them.
#[tokio::test(flavor = "multi_thread")]
async fn a_matched_pane_something_else_resized_is_left_where_that_resize_left_it() {
    let h = harness!("matchkeep");
    let token = h.token(Role::Full).await;
    let mut viewer = h.connect(&token).await;
    until(&mut viewer, "hello", 10).await;
    until(&mut viewer, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut viewer, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut viewer, "grid.reset", &pane, 15).await;
    a_painter_on_the_pane(&h, &mut viewer, &pane, &local).await;
    paint_screen(&mut viewer, &pane, &"#".repeat(400)).await;
    let found_cols = filled_width(&h._session, &local).await;
    a_pane_the_node_streams_at(&h, &pane, found_cols).await;
    let found_rows = viewport_rows(&h._session, &local).await;

    let matched_rows = (found_rows + 3).max(30);
    ok(
        &mut viewer,
        json!({ "t": "manage", "op": "pane.size", "at": pane,
                "cols": found_cols + 24, "rows": matched_rows, "mode": "match" }),
        30,
    )
    .await;

    // The panel, from somewhere else, asking for a size by name. It supersedes the hold, and the
    // geometry it left is the one the pane keeps.
    let mut panel = h.connect(&token).await;
    until(&mut panel, "hello", 10).await;
    let deliberate = matched_rows + 7;
    let sized = ok(
        &mut panel,
        json!({ "t": "manage", "op": "pane.size", "at": pane,
                "cols": found_cols + 8, "rows": deliberate }),
        30,
    )
    .await;
    assert_eq!(
        sized["kept"],
        json!(true),
        "the panel's resize did not land: {sized}"
    );

    drop(viewer);

    assert!(
        rows_stay_at(&h._session, &local, deliberate, 20).await,
        "a viewer closing a window undid a resize somebody asked for by name; the pane is {} rows",
        viewport_rows(&h._session, &local).await,
    );
}

/// **Newest holder wins, and the earlier one does not fight back or take the later one with it.**
///
/// *"you might have something open on your phone but also on your desktop, if you switch between
/// the two and both are set to 'match view' that's kind of expected."* So the second viewer's claim
/// displaces the first's, the first's release lands on nothing when it eventually happens, and the
/// geometry the *pane* was found at — not the first viewer's — is what the last one out puts back.
#[tokio::test(flavor = "multi_thread")]
async fn the_newest_viewer_wins_the_pane_and_the_last_one_out_puts_the_pane_itself_back() {
    let h = harness!("matchtwo");
    let token = h.token(Role::Full).await;
    let mut desk = h.connect(&token).await;
    until(&mut desk, "hello", 10).await;
    until(&mut desk, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut desk, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut desk, "grid.reset", &pane, 15).await;
    a_painter_on_the_pane(&h, &mut desk, &pane, &local).await;
    paint_screen(&mut desk, &pane, &"#".repeat(400)).await;
    let found_cols = filled_width(&h._session, &local).await;
    a_pane_the_node_streams_at(&h, &pane, found_cols).await;
    let found_rows = viewport_rows(&h._session, &local).await;

    let desk_rows = (found_rows + 3).max(30);
    ok(
        &mut desk,
        json!({ "t": "manage", "op": "pane.size", "at": pane,
                "cols": found_cols + 24, "rows": desk_rows, "mode": "match" }),
        30,
    )
    .await;
    assert!(
        rows_settle_at(&h._session, &local, desk_rows, 20).await,
        "the first match never landed"
    );

    // A second window on the same pane, and it is the one being looked at now.
    let mut second = h.connect(&token).await;
    until(&mut second, "hello", 10).await;
    let second_rows = desk_rows + 6;
    let took = ok(
        &mut second,
        json!({ "t": "manage", "op": "pane.size", "at": pane,
                "cols": found_cols + 40, "rows": second_rows, "mode": "match" }),
        30,
    )
    .await;
    // It carries the *pane's* own geometry forward rather than re-reading one the first viewer set.
    assert_eq!(
        (&took["found_cols"], &took["found_rows"]),
        (&json!(found_cols), &json!(found_rows)),
        "the handover took the first viewer's size for the pane's own: {took}",
    );
    assert!(
        rows_settle_at(&h._session, &local, second_rows, 20).await,
        "the handover never landed"
    );

    // The first window goes. Its hold was displaced, so it has nothing to let go of — and letting
    // go of the hold that replaced it would take the pane out from under the window in front.
    drop(desk);
    assert!(
        rows_stay_at(&h._session, &local, second_rows, 15).await,
        "a displaced viewer closing took the newer viewer's hold with it; the pane is {} rows",
        viewport_rows(&h._session, &local).await,
    );

    // And the last one out puts back what the pane was, not what either viewer made it.
    drop(second);
    assert!(
        rows_settle_at(&h._session, &local, found_rows, 45).await,
        "the pane was left at a viewer's size; it is {} rows and it was found at {found_rows}",
        viewport_rows(&h._session, &local).await,
    );
}

/// Waits for a pane to arrive at `want` rows. The release is a controller exiting and then a
/// second claim-and-release putting the size back, so it is seconds rather than instant.
async fn rows_settle_at(session: &Session, pane: &str, want: u16, seconds: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        if viewport_rows(session, pane).await == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

/// The opposite question, and it has to be asked over time rather than once: a restore that fires
/// late would pass a single reading taken before it.
async fn rows_stay_at(session: &Session, pane: &str, want: u16, seconds: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        if viewport_rows(session, pane).await != want {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    true
}

async fn a_pane_the_node_streams_at(h: &Harness, pane: &str, cols: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    let mut seen = None;
    while tokio::time::Instant::now() < deadline {
        seen = h.node.herd().pane(pane).and_then(|entry| entry.cols);
        if seen == Some(cols) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the node never settled on {cols} columns for this pane; it is streaming {seen:?}");
}

/// No `#` left anywhere on the screen, which is this pane's only wrapped line gone.
async fn a_screen_with_no_wrap_on_it(session: &Session, pane: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut rows = Vec::new();
    while tokio::time::Instant::now() < deadline {
        rows = hash_rows(session, pane).await;
        if rows.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the probe line is still on the screen at {rows:?}, so the wrap was never taken away");
}

async fn typed(session: &Session, pane: &str, text: &str) {
    session
        .call("pane.send_text", json!({ "pane_id": pane, "text": text }))
        .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
}

/// Resizes a pane's PTY to an even width by claiming it with a controller and letting the
/// controller go: the PTY stays where the controller left it and the layout rect never moves
/// (probe #219), which is the only way to reach a width nothing else in this suite produces.
async fn control_cols(session: &Session, pane: &str, cols: u16) {
    let rows = session.call("session.snapshot", json!({})).await["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["scroll"]["viewport_rows"].as_u64())
        .unwrap_or(40);
    let mut controller = std::process::Command::new("herdr")
        .args(["--session", &session.name, "terminal", "session", "control", pane])
        .args(["--cols", &cols.to_string(), "--rows", &rows.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("herdr terminal session control");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let _ = controller.kill();
    let _ = controller.wait();
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// Every width a `grid.reset` carried for this pane over `window`.
async fn drain_resets(socket: &mut Socket, pane: &str, window: Duration) -> Vec<u16> {
    let deadline = tokio::time::Instant::now() + window;
    let mut widths = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "grid.reset" && message["pane"] == pane {
            widths.push(message["cols"].as_u64().unwrap_or(0) as u16);
        }
    }
    widths
}

/// Waits for a `grid.reset` on this pane whose grid is `pty` wide **and** whose probe line is
/// uncropped. The first reading can miss — a probe that loses its socket call falls back to the
/// rect and the next poll corrects it — so what is asserted is that the node *arrives* at the
/// PTY's width, not that it never guesses.
///
/// **Both, in the wait.** Every caller used to assert the width on whatever frame came back
/// uncropped, and a rect-width guess carrying a row the pane had already wrapped satisfies the
/// second half without the first: a 93-column pane streamed at 94 for one poll took the test down
/// with `left: 94, right: 93`. Waiting for the frame that has both is the claim the docstring
/// already made.
async fn await_grid_at(socket: &mut Socket, pane: &str, pty: u16) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "grid.reset" || message["pane"] != pane {
            continue;
        }
        seen.push(message["cols"].as_u64().unwrap_or(0));
        if longest_hash_run(&message) == pty && message["cols"].as_u64() == Some(pty as u64) {
            return message;
        }
    }
    panic!("the grid never came back at {pty} columns uncropped; saw widths {seen:?}");
}

/// The width the `#` probe line wrapped at — **proved by the wrap**, and not by a widest row that
/// stopped growing.
///
/// A half-painted first row is also the widest row on the screen, and "the same width twice in a
/// row" is exactly what a shell stalled mid-write produces. Under sixteen spinners two reads
/// 500 ms apart caught the same half-drawn row and a 93-column pane was measured at **75**; every
/// assertion after that was against a width nothing on the machine had, and the test failed
/// saying the grid never came back at 75 columns — a number it had invented.
///
/// 400 `#` wrapped at `w` leave *at least two* rows of exactly `w`, the last row being the
/// remainder, and no partial paint can produce a second full row at a width the PTY did not wrap
/// at. So the condition is the evidence rather than a guess about how long a write takes.
async fn filled_width(session: &Session, pane: &str) -> u16 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut rows = Vec::new();
    while tokio::time::Instant::now() < deadline {
        rows = hash_rows(session, pane).await;
        let widest = rows.iter().copied().max().unwrap_or(0);
        if widest > 60 && rows.iter().filter(|&&run| run == widest).count() >= 2 {
            return widest;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the probe line never wrapped; its rows of `#` were {rows:?}");
}

/// How many `#` are on each row of the visible screen, blank rows left out. The probe line is the
/// only thing these panes draw one with, so a row's run is its share of the wrap.
async fn hash_rows(session: &Session, pane: &str) -> Vec<u16> {
    visible(session, pane)
        .await
        .lines()
        .map(|row| row.chars().filter(|c| *c == '#').count() as u16)
        .filter(|run| *run > 0)
        .collect()
}

async fn widest_rendered_row(session: &Session, pane: &str) -> u16 {
    visible(session, pane)
        .await
        .lines()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(0)
}

async fn visible(session: &Session, pane: &str) -> String {
    session
        .call(
            "pane.read",
            json!({ "pane_id": pane, "source": "visible", "format": "text" }),
        )
        .await["read"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

async fn rect_width(session: &Session, pane: &str) -> u16 {
    let layout = session.call("pane.layout", json!({ "pane_id": pane })).await;
    layout["layout"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["rect"]["width"].as_u64())
        .unwrap() as u16
}

/// The longest unbroken run of `#` in a grid message — the probe line, as the client would see it.
fn longest_hash_run(message: &Value) -> u16 {
    let rows = message["rows_data"]
        .as_array()
        .or_else(|| message["rows"].as_array());
    rows.map(|rows| {
        rows.iter()
            .map(|row| {
                row["runs"]
                    .as_array()
                    .map(|runs| {
                        runs.iter()
                            .filter_map(|r| r["x"].as_str())
                            .collect::<String>()
                            .matches('#')
                            .count() as u16
                    })
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0)
    })
    .unwrap_or(0)
}

/// A Claude-shaped transcript, written the way the harness writes one: JSON Lines, a `tool_use`
/// whose result lands in a later record.
///
/// **Stamped from the clock, not from a date somebody typed.** A transcript belongs to the
/// harness process running in the pane, and a node will not serve one whose last word was written
/// before that process existed — so a fixture frozen at a past date describes a conversation no
/// live pane could have had, and would pass only for as long as nobody looked.
fn claude_transcript(cwd: &str, filler: usize) -> (String, String) {
    let opened = time::OffsetDateTime::now_utc().replace_nanosecond(0).unwrap();
    let at = |seconds: i64| {
        (opened + time::Duration::seconds(seconds))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    };
    let mut lines = Vec::new();
    for n in 0..filler {
        lines.push(json!({
            "type": "user", "uuid": format!("u{n}"), "cwd": cwd,
            "timestamp": at(n as i64),
            "message": { "content": format!("filler {n}") }
        }));
    }
    lines.push(json!({
        "type": "assistant", "uuid": "a-md", "cwd": cwd,
        "timestamp": at(filler as i64 + 1),
        "message": { "content": [
            { "type": "text",
              "text": "Six, and they are…\n\n| Key | Accepted |\n|---|---|\n| `Up` | yes |\n" }
        ] }
    }));
    lines.push(json!({
        "type": "assistant", "uuid": "a-tool", "cwd": cwd,
        "timestamp": at(filler as i64 + 2),
        "message": { "content": [
            { "type": "tool_use", "id": "tu1", "name": "Bash",
              "input": { "command": "herdr pane list --json", "description": "probe key grammar" } }
        ] }
    }));
    let settle = json!({
        "type": "user", "uuid": "u-result", "cwd": cwd,
        "timestamp": at(filler as i64 + 3),
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "tu1", "content": "one\ntwo\nthree\n" }
        ] }
    });
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    (body, format!("{settle}\n"))
}

/// Makes the pane's own shell the harness: a copy of `bash` under the harness's name, `exec`ed
/// in place, so that herdr's `pane.process_info` reports a foreground process named after the
/// agent the pane claims to be running. A node serves no conversation to a pane with no harness
/// in it — herdr detects one by scraping the screen and can be wrong — and `pane.report_agent`
/// alone is exactly that wrongness. A shell rather than a `sleep`, because these tests go on to
/// paint the harness's screen through it. Answers with its pid.
async fn become_harness(session: &Session, local: &str, dir: &Path, agent: &str) -> u32 {
    let binary = dir.join(agent);
    if !binary.exists() {
        std::fs::copy(which("bash").expect("bash on PATH"), &binary).unwrap();
    }
    session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": format!("exec {}\n", binary.display()) }),
        )
        .await;
    for _ in 0..100 {
        let info = session
            .call("pane.process_info", json!({ "pane_id": local }))
            .await;
        let found = info["process_info"]["foreground_processes"]
            .as_array()
            .and_then(|ps| ps.iter().find(|p| p["name"] == agent))
            .and_then(|p| p["pid"].as_u64());
        if let Some(pid) = found {
            return pid as u32;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("herdr never reported a {agent} process in the pane");
}

/// The `timestamp` a record carries, read back out of the transcript that was written. The claim
/// is that the wire passes the harness's own stamp through, so the transcript has to be what the
/// expectation is taken from — a literal repeated in the test only ever agrees with itself.
fn stamp_of(body: &str, uuid: &str) -> String {
    body.lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["uuid"] == uuid)
        .expect("the record")["timestamp"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The conversation path end to end: a real node, a real WebSocket, and the bytes checked against
/// what `client/shared/.../Codec.kt` reads — `pane` / `cursor` / `more` / `turns`, each turn
/// `id` / `role` / `at` / `blocks`, each block tagged `b`.
///
/// Herdr 0.8.2 never populates `pane.agent_session` — it detects a harness by scraping the screen
/// — so the transcript is found from the pane's own working directory, which is the path that runs
/// against a real `claude`.
#[tokio::test(flavor = "multi_thread")]
async fn a_watched_agent_pane_streams_its_conversation() {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("9f1c0b2e-0000-4000-8000-000000000042.jsonl");

    let home_path = home.path().display().to_string();
    let h = harness!("convo", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;

    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(
        hello["caps"]["conversation"], true,
        "a node with a claude adapter serves conversations: {hello}"
    );
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    // Herdr's own agent report is what makes this an agent pane, exactly as detection would —
    // and a real process named after the harness is what makes the report true. The transcript is
    // written after both, because a harness cannot have written one before it started.
    become_harness(&h._session, &local, home.path(), "claude").await;
    // Nothing is reported into a pane herdr has not published about yet: a report inside the
    // post-label `unknown` hold is overtaken by herdr's own first screen publish (#405), and every
    // status this test then reports would be racing it.
    herdr_has_scraped(&h._session, &local).await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    let (body, settle) = claude_transcript(cwd, 45);
    std::fs::write(&transcript, &body).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;

    let convo = until(&mut socket, "convo", 25).await;
    assert_eq!(convo["pane"], pane.as_str());
    assert_eq!(convo["more"], true, "45 filler turns are more than one page");
    let cursor = convo["cursor"].as_str().expect("an opaque cursor").to_string();
    let turns = convo["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 40, "a page is bounded: {}", turns.len());
    assert_eq!(
        turns.first().unwrap()["id"],
        cursor,
        "the cursor is the oldest turn in the page"
    );

    let markdown = turns
        .iter()
        .find(|t| t["id"] == "a-md")
        .expect("the assistant turn is in the newest page");
    assert_eq!(markdown["role"], "assistant");
    assert_eq!(markdown["at"], stamp_of(&body, "a-md"));
    let block = &markdown["blocks"][0];
    assert_eq!(block["b"], "md");
    assert!(
        block["text"].as_str().unwrap().contains("| Key | Accepted |"),
        "markdown is passed through verbatim so a table stays a table: {block}"
    );

    let running = turns.iter().find(|t| t["id"] == "a-tool").unwrap();
    assert_eq!(running["blocks"][0]["b"], "tool");
    assert_eq!(running["blocks"][0]["name"], "Bash");
    assert_eq!(running["blocks"][0]["summary"], "probe key grammar");
    assert_eq!(running["blocks"][0]["state"], "running");
    assert_eq!(running["blocks"][1]["b"], "code");
    assert_eq!(running["blocks"][1]["lang"], "bash");

    // The tool settles. It must come back under the same id, replacing the turn rather than
    // arriving as a second one — appending renders every tool twice.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(&mut file, settle.as_bytes()).unwrap();
    drop(file);

    let revision = until(&mut socket, "convo.turn", 20).await;
    assert_eq!(revision["pane"], pane.as_str());
    let revised = revision["turns"].as_array().unwrap();
    assert_eq!(revised.len(), 1, "only what changed: {revision}");
    assert_eq!(revised[0]["id"], "a-tool", "matched by id, not appended");
    assert_eq!(revised[0]["blocks"][0]["state"], "done");
    assert_eq!(revised[0]["blocks"][0]["lines"], 3);

    // And the client can page backwards through the opaque cursor it was given.
    send(
        &mut socket,
        json!({ "t": "convo.load", "pane": pane, "before": cursor }),
    )
    .await;
    let older = until(&mut socket, "convo", 15).await;
    let older_turns = older["turns"].as_array().unwrap();
    assert_eq!(older_turns.len(), 7, "the remainder of the transcript: {older}");
    assert_eq!(older["more"], false);
    assert_eq!(older_turns.first().unwrap()["id"], "u0");
}

/// The reported defect, end to end: a prompt sent from a phone while the agent is working is in
/// the transcript the moment the harness queues it and on the pane's screen the whole time, and
/// the conversation showed nothing until the agent got round to taking it — because `convo.facets`
/// was collected once, when the transcript was opened, and never again.
#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_queued_while_the_agent_is_working_reaches_the_client_before_the_agent_takes_it() {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("9f1c0b2e-0000-4000-8000-000000000043.jsonl");

    let home_path = home.path().display().to_string();
    let h = harness!("facets", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    become_harness(&h._session, &local, home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "working" }),
        )
        .await;
    let (body, _) = claude_transcript(cwd, 2);
    std::fs::write(&transcript, &body).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    until_pane(&mut socket, "convo", &pane, 25).await;
    let opening = until_pane(&mut socket, "convo.facets", &pane, 20).await;
    assert_eq!(
        opening["facets"],
        json!({}),
        "nothing is queued yet, and a harness with nothing to say says nothing: {opening}"
    );

    // The operator sends a prompt from a phone mid-turn. The harness records the enqueue and will
    // not touch it again until it takes it.
    let queued = json!({
        "type": "queue-operation", "operation": "enqueue",
        "timestamp": "2026-08-28T02:10:59.658Z", "content": "and copy the config across"
    });
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(&mut file, format!("{queued}\n").as_bytes()).unwrap();
    drop(file);

    let moved = until_pane(&mut socket, "convo.facets", &pane, 20).await;
    assert_eq!(
        moved["facets"]["queued"][0]["text"], "and copy the config across",
        "the prompt is waiting on the pane and the client was never told: {moved}"
    );
    assert_eq!(moved["facets"]["queued"][0]["at"], "2026-08-28T02:10:59.658Z");

    // And the harness takes it. `dequeue` carries a null `content` (#320), so the fold has to take
    // the head by position — a client left holding the prompt would draw one nobody is waiting on.
    let taken = json!({ "type": "queue-operation", "operation": "dequeue", "content": null });
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(&mut file, format!("{taken}\n").as_bytes()).unwrap();
    drop(file);

    let emptied = until_pane(&mut socket, "convo.facets", &pane, 20).await;
    assert_eq!(
        emptied["facets"],
        json!({}),
        "the queue emptying is a change like any other: {emptied}"
    );
}

/// The operator, on 0.1.49: *"sometimes claude leaves shells open forever and 'working' can mean
/// nothing but 'a shell was left running'"*. End to end, on the shape measured in #418: a
/// background command whose `tool_result` arrives **at launch** and is therefore not an ending, and
/// the `<task-notification>` that is.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_left_running_in_the_background_is_named_on_the_wire_until_it_reports_back() {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("9f1c0b2e-0000-4000-8000-000000000047.jsonl");

    let home_path = home.path().display().to_string();
    let h = harness!("running", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    become_harness(&h._session, &local, home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "working" }),
        )
        .await;
    let (body, _) = claude_transcript(cwd, 2);
    std::fs::write(&transcript, &body).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    until_pane(&mut socket, "convo", &pane, 25).await;
    let opening = until_pane(&mut socket, "convo.facets", &pane, 20).await;
    assert_eq!(
        opening["facets"],
        json!({}),
        "nothing has been launched yet: {opening}"
    );

    let append = |records: &[serde_json::Value]| {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        for record in records {
            std::io::Write::write_all(&mut file, format!("{record}\n").as_bytes()).unwrap();
        }
    };

    append(&[
        json!({
            "type": "assistant", "uuid": "cccccccc-0000-4000-8000-000000000001",
            "timestamp": "2026-09-01T23:46:16.370Z",
            "message": { "role": "assistant", "content": [{
                "type": "tool_use", "id": "toolu_bg", "name": "Bash",
                "input": { "command": "cargo build --release", "description": "the release build",
                           "run_in_background": true }
            }]}
        }),
        // Measured at 300-400 ms after the call, and carrying nothing but the task id: this is the
        // harness saying "started". Treating it as an ending is the defect.
        json!({
            "type": "user", "uuid": "cccccccc-0000-4000-8000-000000000002",
            "timestamp": "2026-09-01T23:46:16.720Z",
            "toolUseResult": { "stdout": "", "stderr": "", "backgroundTaskId": "brelease" },
            "message": { "role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "toolu_bg", "content": "ok"
            }]}
        }),
    ]);

    let launched = until_pane(&mut socket, "convo.facets", &pane, 20).await;
    let running = &launched["facets"]["running"];
    assert_eq!(
        running[0]["kind"], "shell",
        "the launch acknowledgement ended it: {launched}"
    );
    assert_eq!(running[0]["title"], "the release build");
    assert_eq!(running[0]["name"], "Bash");
    assert_eq!(
        running[0]["since"], "2026-09-01T23:46:16.370Z",
        "the stopwatch runs from the call, not from whenever a client asked",
    );
    assert!(running[1].is_null(), "one launch, one entry: {launched}");

    append(&[json!({
        "type": "queue-operation", "operation": "enqueue",
        "timestamp": "2026-09-01T23:50:16.000Z",
        "content": "<task-notification>\n<task-id>brelease</task-id>\n                    <tool-use-id>toolu_bg</tool-use-id>\n<status>completed</status>\n                    <summary>Background command \"the release build\" completed (exit code 0)</summary>\n                    </task-notification>"
    })]);

    let over = until_pane(&mut socket, "convo.facets", &pane, 20).await;
    assert_eq!(
        over["facets"],
        json!({}),
        "the notification is the one thing that ends a background run: {over}"
    );
}

/// `convo.load` used to answer `unsupported`. It is implemented now, so a pane with no transcript
/// is `not_found` — and a shell pane is never watched with a conversation in the first place.
#[tokio::test(flavor = "multi_thread")]
async fn a_shell_pane_has_no_conversation_to_page() {
    let h = harness!("convo-none");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    send(&mut socket, json!({ "t": "convo.load", "pane": pane })).await;
    let refusal = until_pane(&mut socket, "error", &pane, 15).await;
    assert_eq!(refusal["code"], "not_found", "{refusal}");
}

/// One `managed` ack, matched by op: the node serves every session on the machine, so an ack for
/// somebody else's op is not this op's answer.
async fn managed(socket: &mut Socket, op: &str, seconds: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "managed" && message["op"] == op {
            return message;
        }
        seen.push(format!("{}/{}", message["t"], message["op"]));
    }
    panic!("never saw managed for {op}; saw {seen:?}");
}

async fn ok(socket: &mut Socket, request: Value, seconds: u64) -> Value {
    let op = request["op"].as_str().expect("an op").to_string();
    send(socket, request).await;
    let ack = managed(socket, &op, seconds).await;
    assert_eq!(ack["ok"], true, "{op}: {ack}");
    ack
}

/// Waits for a `herd.patch` that adds a pane under `workspace`, which is what proves the node —
/// not the client — put it on screen.
async fn patch_adding(socket: &mut Socket, workspace: &str, seconds: u64) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] != "herd.patch" {
            continue;
        }
        for pane in message["added"]["panes"].as_array().unwrap_or(&Vec::new()) {
            if pane["workspace_id"] == workspace {
                return pane["id"].as_str().expect("a pane id").to_string();
            }
        }
    }
    panic!("no herd.patch added a pane under {workspace}");
}

/// Every op the Kotlin client can build, in the JSON it builds, against a real herd. The shapes
/// come from `tests/fixtures/manage-ops.json`, which `client/shared`'s `ManageWireTest` asserts
/// the client emits byte for byte — so neither side gets to agree with itself.
#[tokio::test(flavor = "multi_thread")]
async fn every_client_op_lands_on_a_real_herd() {
    let h = harness!("clientops");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let herd = until(&mut socket, "herd", 10).await;

    // The ids a client addresses its containers by have to be on the wire in the first place.
    let seed = herd["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["node_id"] == node.as_str())
        .expect("a pane of this harness's session")
        .clone();
    assert!(seed["workspace_id"].as_str().is_some(), "{seed}");
    assert!(seed["tab_id"].as_str().is_some(), "{seed}");

    let repo = tempfile::tempdir().expect("a repo");
    let repo_path = repo.path().display().to_string();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "root",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(repo.path())
                .status()
                .expect("git")
                .success()
        );
    }

    // workspace.create, with the env map the client's old `Map<String, String?>` could not send.
    let created = ok(
        &mut socket,
        json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "kampr-probe",
                "cwd": repo_path, "env": { "KAMPR_PROBE": "1" } }),
        20,
    )
    .await;
    let workspace = created["id"].as_str().expect("a workspace id").to_string();
    let first_pane = patch_adding(&mut socket, &workspace, 25).await;

    // tab.create takes a workspace, and the node derives one from a pane id too.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "tab.create", "at": workspace, "label": "tests", "cwd": repo_path }),
        20,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "tab.create", "at": first_pane, "label": "derived" }),
        20,
    )
    .await;

    // pane.split, with the float the old type could not send either.
    let split = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.split", "at": first_pane, "direction": "right", "ratio": 0.35 }),
        20,
    )
    .await;
    let second_pane = split["id"].as_str().expect("a pane id").to_string();
    assert_ne!(second_pane, first_pane);

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.zoom", "at": first_pane, "mode": "on" }),
        15,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.zoom", "at": first_pane, "mode": "off" }),
        15,
    )
    .await;

    // The one op that reshapes a pane, driven for real: claimed, resized, released. This session
    // is headless, so #219 says the size stays — and the reply says what it *measured* rather than
    // echoing what was asked for, because on an attached pane the desk takes it straight back
    // (#19) and a reply that assumed otherwise would be a plausible-looking success.
    let sized = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane, "cols": 100, "rows": 30 }),
        30,
    )
    .await;
    assert_eq!(sized["ok"], json!(true), "{sized}");
    // **`ok` is not the answer.** The op ran; whether the pane is now 30 rows is a separate
    // question, and `size_pane` measures it — so the ack carries the measurement or it is telling
    // this client `true` about a resize that did not happen (#233's shape on the one op ADR 0012
    // permits). This session is headless, so #219 says the size stays and `kept` is `true`; on an
    // attached pane the same field is how a client learns the desk took it back inside a second.
    assert_eq!(
        sized["kept"],
        json!(true),
        "the ack does not say whether the resize stuck: {sized}"
    );
    assert_eq!(
        sized["measured_rows"],
        json!(30),
        "the ack does not say what the pane actually measured: {sized}"
    );

    // The floor, against a real node: a size no shell is usable at is refused rather than made
    // permanent. This is the guard the whole feature is shaped around.
    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane, "cols": 40, "rows": 12 }),
    )
    .await;
    let refused = managed(&mut socket, "pane.size", 15).await;
    assert_eq!(refused["ok"], json!(false), "40x12 was allowed: {refused}");
    assert_eq!(refused["code"], json!("bad_request"), "{refused}");

    // Holding and letting go. The hold is what makes a size survive on a pane a desk is attached
    // to; the release is the ordinary end of one, and the node's own deadline is the backstop.
    let held = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane,
                "cols": 100, "rows": 30, "mode": "hold" }),
        30,
    )
    .await;
    assert_eq!(
        held["held"],
        json!(true),
        "a hold that does not say it is holding: {held}"
    );
    let released = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane, "mode": "release" }),
        15,
    )
    .await;
    // A release that let go of nothing and one that let go of a live controller are different
    // answers, and only this field tells them apart.
    assert_eq!(
        released["was_held"],
        json!(true),
        "the release does not say there was a hold to let go of: {released}"
    );
    assert_eq!(released["held"], json!(false), "{released}");

    // Only a pane's label is nullable, and null is what clears it.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": first_pane, "label": "build" }),
        15,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": first_pane, "label": null }),
        15,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": workspace, "label": "kampr-probe-2" }),
        15,
    )
    .await;
    send(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": workspace, "label": null }),
    )
    .await;
    let refused = managed(&mut socket, "rename", 15).await;
    assert_eq!(
        refused["ok"], false,
        "a workspace has no label to clear: {refused}"
    );
    assert_eq!(refused["code"], "bad_request");

    for at in [&first_pane, &workspace] {
        ok(&mut socket, json!({ "t": "manage", "op": "focus", "at": at }), 15).await;
    }

    // agent.start, with the array of args the old type could not send.
    let kinds = {
        send(&mut socket, json!({ "t": "caps" })).await;
        let caps = until(&mut socket, "caps", 20).await;
        caps["agent_kinds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect::<Vec<_>>()
    };
    assert!(kinds.contains(&"claude".to_string()), "{kinds:?}");
    // The only assertion in this suite that needs a *second* binary on the host. A runner without
    // `claude` cannot exercise herdr's agent detection at all, and pretending otherwise would be a
    // green tick for something never run.
    let has_claude = std::process::Command::new("claude")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !has_claude {
        eprintln!(
            "!!!! agent.start unexercised: no `claude` on PATH, so herdr's agent detection and the \
             herd's `agent` field are untested on this host !!!!"
        );
    }
    // A pane herdr has only just created is not yet "an available shell", so this is the one op
    // that has to wait for the thing it acts on rather than for its own answer.
    let mut started = json!(null);
    if has_claude {
        for _ in 0..20 {
            send(
                &mut socket,
                json!({ "t": "manage", "op": "agent.start", "at": second_pane, "kind": "claude",
                    "name": "probe", "args": [] }),
            )
            .await;
            started = managed(&mut socket, "agent.start", 30).await;
            if started["ok"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert_eq!(started["ok"], true, "{started}");
        // The agent is real: the node sees the harness on the pane and says so in the herd.
        //
        // **Two waits, because two unrelated things are being waited for**, and folding them into
        // one 20 s budget is what made this test load-sensitive. The first is the host booting a
        // real `claude` and herdr noticing the process: 0.42–0.65 s on an idle machine here, and
        // 9.5–10.2 s with these cores twelve times oversubscribed, because a Node.js binary
        // starting under contention is neither fast nor bounded and nothing in this repository
        // makes it faster. The second — the only half this test is about — is the node carrying
        // herdr's answer into its own herd, and that one is *flat* under the same load: 0.92–1.06 s
        // loaded against well under a second idle, because `pane.agent_detected` is a subscribed
        // topology event and a missed one costs the 500 ms resubscribe floor rather than the 30 s
        // sweep. The old failure said the herd never saw the agent when what had happened was that
        // `claude` had not finished starting.
        //
        // The first wait's four minutes are a give-up threshold and not a measurement, which is
        // what makes them honest: nothing is being timed there, and when they run out the message
        // says the host never started the agent rather than blaming the node. Four rather than two
        // because two was not enough once in five full-suite runs under sixteen spinners.
        let local = second_pane.rsplit('/').next().unwrap().to_string();
        herdr_pane(&h._session, &local, 240, "labelled an agent on", |p| {
            p["agent"] == "claude"
        })
        .await;
        let mut agent = None;
        for _ in 0..120 {
            agent = h
                .node
                .herd()
                .panes
                .iter()
                .find(|p| p.id == second_pane)
                .and_then(|p| p.agent.clone());
            if agent.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(125)).await;
        }
        assert_eq!(
            agent.as_deref(),
            Some("claude"),
            "herdr had labelled the pane and the node's herd never carried it",
        );
    }

    // worktree.create / worktree.open — Herdr's own git support, straight through.
    let worktree = ok(
        &mut socket,
        json!({ "t": "manage", "op": "worktree.create", "node": node, "cwd": repo_path,
                "branch": "kampr/probe", "base": "main", "label": "probe-tree" }),
        30,
    )
    .await;
    assert!(worktree["ok"] == true, "{worktree}");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "worktree.open", "node": node, "path": repo_path,
                "label": "probe-open" }),
        30,
    )
    .await;

    // A pane can be closed on its own, before the layout round-trip renumbers the tab.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "close", "at": second_pane }),
        20,
    )
    .await;

    // layout.export round-trips into layout.apply unchanged.
    let tab = h
        .node
        .herd()
        .panes
        .iter()
        .find(|p| p.id == first_pane)
        .and_then(|p| p.tab_id.clone())
        .expect("the pane's tab id");
    let exported = ok(
        &mut socket,
        json!({ "t": "manage", "op": "layout.export", "at": tab }),
        20,
    )
    .await;
    assert!(exported["layout"]["root"]["type"].is_string(), "{exported}");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "layout.apply", "at": tab, "layout": exported["layout"] }),
        20,
    )
    .await;

    // A named session is a whole separate herdr server. Created here, stopped here, gone here.
    let session = format!("kampr-probe-{}", std::process::id());
    let created = CreatedSession::named(&session);
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    for _ in 0..100 {
        if created.dir().join("herdr.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        created.dir().join("herdr.sock").exists(),
        "no socket at {:?}",
        created.dir()
    );
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;

    // Closing the workspace takes its panes with it, and the client hears about it as a patch.
    let doomed: Vec<String> = h
        .node
        .herd()
        .panes
        .iter()
        .filter(|p| p.workspace_id.as_deref() == Some(workspace.as_str()))
        .map(|p| p.id.clone())
        .collect();
    assert!(!doomed.is_empty(), "the probe workspace had no panes to lose");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "close", "at": workspace }),
        20,
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut gone = false;
    while tokio::time::Instant::now() < deadline && !gone {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "herd.patch" {
            gone = message["removed_ids"]
                .as_array()
                .is_some_and(|ids| doomed.iter().any(|d| ids.iter().any(|id| id == &json!(d))));
        }
    }
    assert!(gone, "closing the workspace never arrived as a herd.patch");

    // The two book ops, on a node that has a real herd attached. They reach herdr for nothing —
    // the book is this node's own database — and driving them here is what proves that: they are
    // answered on a socket whose every other op went to a herdr server, and they are the only two
    // this test never routes.
    let kept = ok(
        &mut socket,
        json!({ "t": "manage", "op": "fleet.save", "args": ["kampr", "update"],
                "label": "update everything" }),
        15,
    )
    .await;
    let entry = kept["id"].as_str().expect("the entry it kept").to_string();
    let book = until(&mut socket, "fleet.book", 15).await;
    assert_eq!(book["saved"][0]["args"], json!(["kampr", "update"]));
    assert_eq!(book["saved"][0]["label"], "update everything");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "fleet.drop", "entry": entry }),
        15,
    )
    .await;
    assert_eq!(until(&mut socket, "fleet.book", 15).await["saved"], json!([]));

    // If the client learns a new op, this test has to have driven it against a real herd before
    // it counts as working.
    let fixture: Value = serde_json::from_str(include_str!("../fixtures/manage-ops.json")).unwrap();
    let expected: std::collections::BTreeSet<&str> = fixture
        .as_object()
        .unwrap()
        .values()
        .filter_map(|v| v["op"].as_str())
        .collect();
    let exercised: std::collections::BTreeSet<&str> = [
        "workspace.create",
        "tab.create",
        "pane.split",
        "pane.zoom",
        "pane.size",
        "rename",
        "close",
        "focus",
        "agent.start",
        "worktree.create",
        "worktree.open",
        "layout.export",
        "layout.apply",
        "session.create",
        "session.stop",
        "fleet.save",
        "fleet.drop",
    ]
    .into_iter()
    .collect();
    assert!(
        expected.difference(&exercised).next().is_none(),
        "no live coverage for {:?}",
        expected.difference(&exercised).collect::<Vec<_>>(),
    );
}

/// A read-only device is refused every one of them by the same gate, which is why the client
/// hides the affordances rather than offering a tap into a refusal.
#[tokio::test(flavor = "multi_thread")]
async fn a_readonly_device_is_refused_every_manage_op() {
    let h = harness!("nomanage");
    let token = h.token(Role::Readonly).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    for op in [
        json!({ "t": "manage", "op": "workspace.create", "node": h.node.node_id() }),
        json!({ "t": "manage", "op": "pane.split", "at": pane, "direction": "right", "ratio": 0.5 }),
        json!({ "t": "manage", "op": "rename", "at": pane, "label": null }),
        json!({ "t": "manage", "op": "close", "at": pane }),
        // The one that reshapes a pane for everybody, and so the one it matters most to refuse.
        // Both forms: the resize itself, and the hold that would keep a desk's screen wrong.
        json!({ "t": "manage", "op": "pane.size", "at": pane, "cols": 200, "rows": 50 }),
        json!({ "t": "manage", "op": "pane.size", "at": pane, "cols": 200, "rows": 50, "mode": "hold" }),
    ] {
        send(&mut socket, op).await;
        assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    }
}

/// The New sheet builds a `workspace.create` with **no `env` key at all** whenever the operator
/// typed no environment variables, which is almost always. Herdr 0.8.2 refuses `"env": null`
/// with `invalid type: null, expected a map`, so a node that serialises `Option::None` into the
/// params fails every one of those — the button has never worked.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_with_no_environment_is_created() {
    let h = harness!("bareenv");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;

    // The three shapes a client can produce: no key (the New sheet's own), an empty map, and a
    // populated one. Only the last was ever exercised.
    for (case, request) in [
        (
            "no env key",
            json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "bare" }),
        ),
        (
            "an empty env map",
            json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "empty",
                    "env": {} }),
        ),
        (
            "a populated env map",
            json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "full",
                    "env": { "KAMPR_LIVE": "1" } }),
        ),
    ] {
        send(&mut socket, request).await;
        let ack = managed(&mut socket, "workspace.create", 25).await;
        assert_eq!(ack["ok"], true, "{case}: {ack}");
        let workspace = ack["id"].as_str().expect("a workspace id").to_string();
        patch_adding(&mut socket, &workspace, 25).await;
        ok(
            &mut socket,
            json!({ "t": "manage", "op": "close", "at": workspace }),
            20,
        )
        .await;
    }
}

/// `docs/04-wire-protocol.md`, first rule: unknown `t` values are ignored, not errors. A node
/// that answers `bad_request` breaks every v1 client the moment a v1.1 client shares a build.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tag_draws_no_answer_at_all() {
    let h = harness!("futuret");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    send(&mut socket, json!({ "t": "future.thing", "whatever": 1 })).await;
    send(&mut socket, json!({ "t": "ping", "n": 11 })).await;
    // The pong is the fence: anything the node had to say about `future.thing` is ahead of it.
    let mut answers = Vec::new();
    loop {
        let message = recv(&mut socket, Duration::from_secs(5))
            .await
            .expect("the node answered the ping");
        if message["t"] == "pong" {
            assert_eq!(message["n"], 11);
            break;
        }
        if message["t"] == "error" {
            answers.push(message);
        }
    }
    assert!(
        answers.is_empty(),
        "an unknown `t` must be ignored, not refused: {answers:?}"
    );

    // Ignoring the unknown must not cost the refusal of a *known* verb sent wrong.
    send(&mut socket, json!({ "t": "watch" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "bad_request");
}

/// Per-device preferences are the mechanism behind remembered zoom, and neither half worked: a
/// fresh socket was never told what it had stored, and a one-key write replaced the blob.
#[tokio::test(flavor = "multi_thread")]
async fn prefs_are_restored_at_hello_and_merged_on_a_partial_write() {
    let h = harness!("prefsmerge");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let empty = until(&mut socket, "prefs", 10).await;
    assert_eq!(empty["panes"], json!({}), "a device with nothing stored: {empty}");
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "view": "conversation" } }),
    )
    .await;
    until(&mut socket, "prefs", 10).await;
    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "zoom": "2" } }),
    )
    .await;
    let merged = until(&mut socket, "prefs", 10).await;
    assert_eq!(
        merged["panes"][&pane],
        json!({ "view": "conversation", "zoom": "2" }),
        "a one-key write must merge, not replace: {merged}"
    );

    // A null clears one key and leaves the rest, which is the only way a client can ever undo a
    // preference once writes merge.
    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "view": null } }),
    )
    .await;
    let cleared = until(&mut socket, "prefs", 10).await;
    assert_eq!(cleared["panes"][&pane], json!({ "zoom": "2" }), "{cleared}");

    // And a new socket is told, unasked. Nothing restores otherwise: the client has no other
    // source for the zoom it left the pane at.
    let mut second = h.connect(&token).await;
    let restored = until(&mut second, "prefs", 10).await;
    assert_eq!(restored["panes"][&pane], json!({ "zoom": "2" }), "{restored}");
}

/// The protocol says a refused op is acknowledged too, and the client's New sheet clears its
/// in-flight state on the ack alone — so a `manage` that draws only an `error` leaves the sheet
/// spinning forever. `rid` has to survive both refusal paths as well.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_manage_is_still_acknowledged() {
    let h = harness!("refusedack");
    let readonly = h.token(Role::Readonly).await;
    let mut socket = h.connect(&readonly).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "manage", "op": "close", "at": pane, "rid": "r1" }),
    )
    .await;
    let ack = managed(&mut socket, "close", 10).await;
    assert_eq!(ack["ok"], false, "{ack}");
    assert_eq!(ack["code"], "not_writer", "{ack}");
    assert_eq!(ack["rid"], "r1", "the ack echoes the caller's token: {ack}");
    assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");

    // The peer-routing refusal is the other path that built an ack by hand and dropped the `rid`.
    let full = h.token(Role::Full).await;
    let mut writer = h.connect(&full).await;
    until(&mut writer, "hello", 10).await;
    send(
        &mut writer,
        json!({ "t": "manage", "op": "layout.export", "at": "01JNOTANODE/w9:t1", "rid": "r2" }),
    )
    .await;
    let refused = managed(&mut writer, "layout.export", 10).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["code"], "unknown_pane", "{refused}");
    assert_eq!(refused["rid"], "r2", "{refused}");
}

/// Field 5 of `/proc/<pid>/stat`, read past the comm field so a process named `a) b` cannot
/// shift the count.
#[cfg(target_os = "linux")]
fn process_group(pid: u32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(") ")?.1;
    after_comm.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn herdr_pid_for(session: &str) -> Option<u32> {
    let wanted = format!("--session\0{session}\0");
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let cmdline = std::fs::read_to_string(entry.path().join("cmdline")).ok()?;
        cmdline.contains(&wanted).then_some(pid)
    })
}

/// A herdr session the node created has to outlive the node. It does not while it is a member of
/// the node's process group: `systemctl restart` and a Ctrl-C on a foreground `kampr serve` both
/// signal the group, and take every agent in every node-created session with them.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_created_session_leaves_the_nodes_process_group() {
    let h = harness!("detach");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let session = format!("kampr-detach-{}", std::process::id());
    let created = CreatedSession::named(&session);
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    for _ in 0..100 {
        if created.dir().join("herdr.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let spawned = herdr_pid_for(&session).expect("the created herdr is running");
    let ours = process_group(std::process::id()).expect("our own process group");
    let theirs = process_group(spawned).expect("the created herdr's process group");

    let stopped = ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    // N5: a session op answers with a session, so its `id` must not be dressed up as a pane id a
    // client would then try to watch.
    assert_eq!(stopped["id"], session.as_str(), "{stopped}");

    assert_ne!(
        theirs, ours,
        "a created session shares the node's process group, so one signal kills both"
    );
    assert_eq!(theirs, spawned as i32, "a detached session leads its own group");
}

/// Reported from a phone, twice: "creating a new session — session doesn't open when done" and
/// "closing a session — session doesn't close when done". Both ops did exactly what they said.
/// What was wrong was the answer to the question the client asks next: the New sheet's session
/// list is `caps.sessions`, and `caps` was cached for ten seconds *and* acked before the host
/// agreed with it — `server.stop` answers in under a millisecond while `session list` goes on
/// reporting `running: true` for up to 300 ms (#241). So the one refresh that could have shown
/// the operator anything was handed the session set from before the op, by two mechanisms at once.
#[tokio::test(flavor = "multi_thread")]
async fn caps_answers_a_session_op_with_the_session_set_that_op_produced() {
    let h = harness!("capsfresh");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let session = format!("kampr-capsfresh-{}", std::process::id());
    let _created = CreatedSession::named(&session);

    // Warmed the way a client warms it — on `hello`, long before the operator types a name — so
    // what follows is the answer they would really have been given.
    send(&mut socket, json!({ "t": "caps" })).await;
    let before = until(&mut socket, "caps", 10).await;
    assert_eq!(session_state(&before, &session), None, "{before}");

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    send(&mut socket, json!({ "t": "caps" })).await;
    let created = until(&mut socket, "caps", 10).await;
    assert_eq!(
        session_state(&created, &session),
        Some(true),
        "the session the operator just made is missing from the refresh their client sends on its ack: {created}"
    );

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    send(&mut socket, json!({ "t": "caps" })).await;
    let stopped = until(&mut socket, "caps", 10).await;
    assert_eq!(
        session_state(&stopped, &session),
        Some(false),
        "the session is still advertised as running after its own stop was acknowledged: {stopped}"
    );
}

/// The settle that makes a session op's ack honest is a wait of up to five seconds, and it used
/// to run on the dispatch loop. Dispatch is sequential, so a `server.stop` that goes on being
/// listed as running for another 300 ms (#241) was 300 ms in which this socket answered nothing
/// at all — no input to any other pane, no watch, no caps — because somebody stopped a session.
#[tokio::test(flavor = "multi_thread")]
async fn a_settling_session_op_does_not_stop_the_socket_answering_anything_else() {
    let h = harness!("settling");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let session = format!("kampr-settling-{}", std::process::id());
    let _created = CreatedSession::named(&session);
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
    )
    .await;
    send(&mut socket, json!({ "t": "ping", "n": 41 })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut answered = false;
    let ack = loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never saw the stop acknowledged"
        );
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "pong" && message["n"] == 41 {
            answered = true;
        }
        if message["t"] == "managed" && message["op"] == "session.stop" {
            break message;
        }
    };
    assert!(
        answered,
        "the socket answered nothing until the stop had settled: {ack}"
    );
    assert_eq!(ack["ok"], true, "{ack}");
    assert_eq!(ack["id"], session.as_str(), "{ack}");
}

/// `session.create` used to answer on the spawn alone, which made it a `managed{ok}` for a
/// session that may not exist — the shape of #233, a failure wearing a plausible success. Driven
/// with a herdr that starts, says nothing and leaves: the operator has to be told.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_named_session_that_never_starts_is_refused_rather_than_acknowledged() {
    let bin = tempfile::tempdir().expect("a bin dir");
    let shim = bin.path().join("herdr");
    std::fs::write(&shim, "#!/bin/sh\nexit 0\n").expect("a shim");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let h = harness!("nosession", |c: &mut Config| {
        c.herdr.binary = shim.display().to_string();
    });
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": "kampr-never" }),
    )
    .await;
    let ack = managed(&mut socket, "session.create", 30).await;
    assert_eq!(ack["ok"], false, "{ack}");
    assert_eq!(ack["code"], "herdr_unavailable", "{ack}");
    let said = ack["message"].as_str().unwrap_or_default();
    assert!(said.contains("kampr-never"), "the refusal has to name it: {said}");
}

/// `Some(running)` when `caps` lists the session at all — a stopped one stays listed for ever
/// as `running: false` and is only forgotten when its directory goes (#242), so "gone" and
/// "stopped" are two different answers and the sheet draws them differently.
fn session_state(caps: &Value, name: &str) -> Option<bool> {
    caps["sessions"].as_array()?.iter().find(|s| s["name"] == name)?["running"].as_bool()
}

/// **The two halves of the question, and why a pane needs both.**
///
/// `has_conversation` promised a transcript and delivered `not_found`: it was derived from the
/// pane's *harness*, so a `claude` started a minute ago advertised a conversation nothing could
/// load. Deriving it from the file on disk fixed that and cost the other half — for the whole gap
/// between a session opening and its first prompt landing, a client had no way to *offer* the
/// conversation at all, so a fresh agent could only be talked to through its grid.
///
/// So the entry carries both. `converses` is the adapter half — this node could serve a
/// conversation for this harness — and it is what a client offers the view on. `has_conversation`
/// stays the transcript half, and it is what says whether `convo.load` will answer with anything.
#[tokio::test(flavor = "multi_thread")]
async fn a_freshly_started_agent_offers_its_conversation_before_a_transcript_exists() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".claude/projects")).unwrap();
    let home_path = home.path().display().to_string();
    let h = harness!("nojournal", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(hello["caps"]["conversation"], true, "{hello}");

    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    become_harness(&h._session, &local, home.path(), "claude").await;
    // Nothing is reported into a pane herdr has not published about yet: a report inside the
    // post-label `unknown` hold is overtaken by herdr's own first screen publish (#405), and every
    // status this test then reports would be racing it.
    herdr_has_scraped(&h._session, &local).await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    // Give the herd every chance to make the claim before it is disproved.
    let mut claimed = false;
    let mut offered = false;
    for _ in 0..24 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(entry) = h.node.herd().pane(&pane) {
            let entry = serde_json::to_value(entry).unwrap();
            claimed |= entry["has_conversation"] == true;
            offered |= entry["converses"] == true;
        }
    }
    assert!(
        !claimed,
        "the pane advertised a conversation that convo.load answers not_found for"
    );
    assert!(
        offered,
        "a claude with an adapter and no transcript yet is still a pane a client may open the \
         conversation on — this is the whole gap between a session starting and its first prompt"
    );

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    send(&mut socket, json!({ "t": "convo.load", "pane": pane })).await;
    assert_eq!(
        until_pane(&mut socket, "error", &pane, 15).await["code"],
        "not_found"
    );

    // And it flips once the transcript is on disk, which is what makes it a derivation rather
    // than a refusal.
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let (body, _) = claude_transcript("/tmp", 2);
    std::fs::write(project.join("9f1c0b2e-0000-4000-8000-000000000043.jsonl"), body).unwrap();
    let mut announced = false;
    for _ in 0..60 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "a transcript on disk never reached the herd");
}

/// In a headless session the PTY does not follow the layout rect — a pane whose rect says 47 is
/// really 93 wide — so reporting the rect as `cols` is reporting a number no row was ever
/// wrapped at. The client prints it to the operator in three places.
#[tokio::test(flavor = "multi_thread")]
async fn an_unmeasured_pane_reports_no_width_rather_than_its_rect() {
    let h = harness!("colslie");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    let rect_before = h._session.call("session.snapshot", json!({})).await["snapshot"]["layouts"][0]["panes"]
        [0]["rect"]["width"]
        .as_u64()
        .expect("a layout rect");
    h._session
        .call(
            "pane.split",
            json!({ "target_pane_id": local, "direction": "right", "focus": false }),
        )
        .await;

    let mut narrowed = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let snapshot = h._session.call("session.snapshot", json!({})).await;
        let rect = snapshot["snapshot"]["layouts"][0]["panes"][0]["rect"]["width"]
            .as_u64()
            .unwrap_or(rect_before);
        if rect < rect_before {
            narrowed = Some(rect);
            break;
        }
    }
    let rect = narrowed.expect("the split never narrowed the rect");

    let mut entry = json!(null);
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(pane) = h.node.herd().pane(&pane) {
            entry = serde_json::to_value(pane).unwrap();
            if entry["rows"].as_u64().is_some() {
                break;
            }
        }
    }
    assert_ne!(
        entry["cols"].as_u64(),
        Some(rect),
        "the herd reported the layout rect as a measured width: {entry}"
    );
    assert!(
        entry["cols"].is_null(),
        "an unmeasured pane has no width to report: {entry}"
    );
    // Rows are knowable without measuring — herdr reports the PTY's own viewport — so they stay.
    let viewport = h._session.call("session.snapshot", json!({})).await["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == local.as_str())
        .and_then(|p| p["scroll"]["viewport_rows"].as_u64())
        .expect("herdr reports the viewport rows");
    assert_eq!(entry["rows"].as_u64(), Some(viewport), "{entry}");
}

/// Probe #112. The pane's measured width moves once — the layout rect is not the PTY width until
/// something wraps and proves it (probes #68/#69) — and the ring restarts on it. What must not
/// happen is the restart repeating: a quiet pane whose width has settled has nothing new to say,
/// and a ring that threw itself away on every read would re-send its whole history for ever.
#[tokio::test(flavor = "multi_thread")]
async fn a_settled_pane_stops_restarting_its_ring() {
    let h = harness!("rewrap");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    // Short rows first, so the ring is keyed on the layout rect. Only then a line longer than the
    // pane, which is the one thing that proves the PTY's real width — and the rect is a column
    // wider than the PTY even unsplit (probe #69), so proving it *moves* the ring's width.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "seq 1 200\n" }),
    )
    .await;
    until_pane(&mut socket, "scrollback", &pane, 25).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "printf '#%.0s' $(seq 1 400); echo\n" }),
    )
    .await;
    let first = until_pane(&mut socket, "scrollback", &pane, 25).await;

    // Let the pane drain and the width prober settle before measuring.
    let mut settled = first["from_top"].as_u64().expect("from_top");
    let settle = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < settle {
        if let Some(m) = recv(&mut socket, Duration::from_secs(1)).await
            && m["t"] == "scrollback"
            && m["pane"] == pane.as_str()
            && let Some(from_top) = m["from_top"].as_u64()
        {
            settled = from_top;
        }
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    while tokio::time::Instant::now() < deadline {
        let Some(m) = recv(&mut socket, Duration::from_secs(1)).await else {
            continue;
        };
        if m["t"] != "scrollback" || m["pane"] != pane.as_str() {
            continue;
        }
        assert_eq!(
            m["from_top"].as_u64(),
            Some(settled),
            "the pane is idle, so its ring must not have restarted; it holds {} rows",
            m["total_rows"]
        );
    }
}

/// The count the herd reports for one pane, from a `herd` or a `herd.patch`. `Some(None)` is the
/// field being absent, which is what one viewer looks like on the wire.
fn watchers_in(message: &Value, pane: &str) -> Option<Option<u64>> {
    let lists = [
        &message["panes"],
        &message["changed"]["panes"],
        &message["added"]["panes"],
    ];
    lists
        .iter()
        .filter_map(|l| l.as_array())
        .flatten()
        .find(|p| p["id"] == pane)
        .map(|p| p["watchers"].as_u64())
}

async fn until_watchers(socket: &mut Socket, pane: &str, want: Option<u64>, seconds: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(1)).await else {
            continue;
        };
        if let Some(found) = watchers_in(&message, pane) {
            if found == want {
                return;
            }
            seen.push(found);
        }
    }
    panic!("never saw watchers={want:?} for {pane}; saw {seen:?}");
}

/// Two people can type into one pane and neither could tell. The herd says so now — and it says
/// nothing at all while a pane has one viewer, so the field is a signal rather than noise.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_viewer_is_announced_on_the_pane_they_share() {
    let h = harness!("watchers");
    let token = h.token(Role::Full).await;
    let mut first = h.connect(&token).await;
    until(&mut first, "hello", 10).await;
    let pane = h.pane_id();

    let herd = until(&mut first, "herd", 10).await;
    assert_eq!(
        watchers_in(&herd, &pane),
        Some(None),
        "an unwatched pane says nothing about viewers"
    );

    send(&mut first, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut first, "grid.reset", &pane, 15).await;

    let mut second = h.connect(&token).await;
    until(&mut second, "hello", 10).await;
    send(&mut second, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut second, "grid.reset", &pane, 15).await;
    until_watchers(&mut first, &pane, Some(2), 10).await;

    send(&mut second, json!({ "t": "unwatch", "pane": pane })).await;
    until_watchers(&mut first, &pane, None, 10).await;
}

/// The subscription list, offered to a real herdr.
///
/// All-or-nothing twice over (probes #54, #76): one name herdr will not accept refuses the whole
/// list, and the node then has no events at all and falls back on a sweep measured in tens of
/// seconds with nothing in the log to say why. `pane.output_changed` is exactly such a name —
/// herdr emits it and refuses to subscribe you to it — so no schema check can stand in for this.
#[tokio::test(flavor = "multi_thread")]
async fn herdr_accepts_every_event_the_node_subscribes_to() {
    let Some(session) = Session::start("subs").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    session
        .call("workspace.create", json!({ "label": "kampr", "cwd": "/tmp" }))
        .await;
    let snapshot = session.call("session.snapshot", json!({})).await;
    let pane = snapshot["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .expect("a pane")
        .to_string();

    for agents in [vec![], vec![pane]] {
        let subs = kampr_core::herdr_provider::subscriptions(&agents);
        session
            .herdr()
            .subscribe(&subs)
            .await
            .unwrap_or_else(|e| panic!("herdr refused the {}-entry list: {e}", subs.len()));
    }
}

/// Change-to-client latency, which is the price the slow sweep is not allowed to charge.
///
/// The reconciliation sweep is tens of seconds now, so nothing but the event subscription can
/// carry this: a workspace opened at the desk has to be on the phone in a fraction of a second, or
/// the herd is only as current as its slowest timer.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_opened_at_the_desk_reaches_a_client_at_once() {
    let h = harness!("evtlatency");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let before: Vec<String> = h.node.herd().panes.iter().map(|p| p.id.clone()).collect();

    let at = tokio::time::Instant::now();
    h._session
        .call("workspace.create", json!({ "label": "second", "cwd": "/tmp" }))
        .await;

    while at.elapsed() < Duration::from_secs(2) {
        let Some(message) = recv(&mut socket, Duration::from_millis(250)).await else {
            continue;
        };
        let fresh = [&message["panes"], &message["added"]["panes"]]
            .iter()
            .filter_map(|l| l.as_array())
            .flatten()
            .filter_map(|p| p["id"].as_str())
            .any(|id| !before.iter().any(|seen| seen == id));
        if fresh {
            eprintln!("a new pane reached the client in {:?}", at.elapsed());
            return;
        }
    }
    panic!("a new workspace took longer than two seconds to reach a connected client");
}

/// A pane whose herdr says `claude`, whose working directory a transcript claims, and whose screen
/// is painted the way Claude 2.1.239 paints one.
///
/// Nothing here is a stub: a real herdr runs the pane, the node's own emulator holds the grid, the
/// journal follows a file on disk, and the frames are read off a websocket.
struct LiveTurn {
    home: tempfile::TempDir,
    work: tempfile::TempDir,
    transcript: PathBuf,
}

impl LiveTurn {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home");
        let work = tempfile::tempdir().expect("work");
        let cwd = work.path().canonicalize().expect("cwd");
        let slug = cwd.display().to_string().replace('/', "-");
        let project = home.path().join(".claude/projects").join(slug);
        std::fs::create_dir_all(&project).expect("project dir");
        let transcript = project.join("11111111-2222-3333-4444-555555555555.jsonl");
        std::fs::write(&transcript, String::new()).expect("transcript");
        Self {
            home,
            work,
            transcript,
        }
    }

    /// The first record, written **after** the pane exists.
    ///
    /// A harness cannot have written a transcript before the terminal it runs in was opened, and
    /// a node will not serve one that claims to have: the transcripts in a working directory
    /// outnumber the sessions a pane has had, and the ones that predate its harness belong to
    /// somebody else. So the fixture has to happen in the order the real thing happens in.
    fn open(&self) {
        self.append(&json!({
            "type": "user",
            "uuid": "u-1",
            "cwd": self.cwd(),
            "message": { "content": "explain the parser" },
        }));
    }

    fn append(&self, record: &Value) {
        let mut line = record.to_string();
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .expect("open transcript");
        use std::io::Write;
        file.write_all(line.as_bytes()).expect("append");
    }

    fn cwd(&self) -> String {
        self.work.path().canonicalize().unwrap().display().to_string()
    }
}

/// Paints literal screen fragments, one shell command, with a pause between them — so a block is
/// seen to *grow*, and only one echoed command line ever sits above it.
fn paint(parts: &[&[&str]]) -> String {
    let mut command = String::new();
    for part in parts {
        if !command.is_empty() {
            command.push_str("sleep 1; ");
        }
        let quoted: Vec<String> = part
            .iter()
            .map(|l| format!("'{}'", l.replace('\'', "'\\''")))
            .collect();
        command.push_str(&format!("printf '%s\\n' {}; ", quoted.join(" ")));
    }
    command.push('\n');
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_in_progress_streams_off_the_screen_and_yields_to_the_record() {
    let fixture = LiveTurn::new();
    let home = fixture.home.path().to_path_buf();
    let h = harness!("liveturn", |config: &mut Config| {
        config.journals.home = home.display().to_string();
    });
    // A second workspace, because the harness's own is rooted at /tmp and a transcript is found
    // through the pane's working directory.
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd() }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd()).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();
    fixture.open();

    // Herdr detects a harness by scraping the screen, so a test that wants a `claude` pane at
    // `working` says so through the same API a plugin would — over a pane that really is running
    // a process by that name, because a node serves no conversation to one that is not.
    become_harness(&h._session, &local, fixture.home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "source": "kampr-test", "agent": "claude", "state": "working" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 20).await;
    assert_eq!(
        opening["turns"].as_array().map(Vec::len),
        Some(1),
        "the transcript opens with the operator's own turn and nothing else"
    );

    // The message arrives in two paints, because a block earns its preview by growing.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": paint(&[
            &[
                "● The parser is a state machine over the byte stream, and every escape",
                "  sequence is one walk through it.",
            ],
            &[
                "  Printable text takes the short path and lands in a cell; a control byte",
                "  takes the long one.",
            ],
        ]) }),
    )
    .await;

    let live = until_live(&mut socket, &pane, 20).await.expect("a live turn");
    let text = live["blocks"][0]["text"].as_str().expect("md").to_string();
    assert_eq!(live["id"], "live");
    assert_eq!(live["role"], "assistant");
    assert!(
        text.starts_with("The parser is a state machine over the byte stream"),
        "the marker is layout and the wrap indent is stripped: {text:?}"
    );
    assert!(
        !text.contains('●') && !text.contains("printf"),
        "neither the harness's glyph nor the shell that painted it: {text:?}"
    );

    // The record lands, unwrapped and in markdown, exactly as Claude writes one.
    fixture.append(&json!({
        "type": "assistant",
        "uuid": "a-1",
        "message": { "content": [ { "type": "text", "text":
            "The **parser** is a state machine over the byte stream, and every escape sequence is one walk through it.\n\nPrintable text takes the short path and lands in a cell; a control byte takes the long one." } ] },
    }));

    let mut authoritative = false;
    let mut withdrawn = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && !(authoritative && withdrawn) {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] != "convo.turn" || message["pane"] != pane.as_str() {
            continue;
        }
        // **The whole frame first.** The claim is about what happens *once the record is on the
        // wire*, and the two can share a frame — a client that applied a non-empty preview
        // alongside the record would draw the message twice, so within a frame the rule still
        // holds. What the rule does not forbid is the preview being published one more time in
        // the window between the record reaching the disk and the node reading it: the pump polls
        // the screen on its own clock, that frame is a truthful reading, and asserting on it made
        // this test fail two runs in twenty saying the opposite of what its own message says.
        authoritative |= message["turns"]
            .as_array()
            .is_some_and(|turns| turns.iter().any(|turn| turn["id"] == "a-1"));
        for turn in message["turns"].as_array().unwrap_or(&Vec::new()) {
            if turn["id"] == "live" {
                withdrawn = turn["blocks"].as_array().is_none_or(Vec::is_empty);
                assert!(
                    withdrawn || !authoritative,
                    "once the record is on the wire the preview may only be withdrawn: {turn}"
                );
            }
        }
    }
    assert!(authoritative, "the transcript record never arrived");
    assert!(
        withdrawn,
        "the preview was never withdrawn, so the client renders the message twice"
    );

    // And nothing is streamed for a client that did not ask for the conversation.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 20).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": paint(&[
            &[
                "● A second message nobody asked to see, growing",
                "  across two paints so it would qualify.",
            ],
            &["  And here is the second paint."],
        ]) }),
    )
    .await;
    assert!(
        until_live(&mut socket, &pane, 6).await.is_none(),
        "a pane watched without `conversation` runs no pump and reads no screen"
    );
}

/// The next `convo.turn` carrying the reserved live id, or `None` if none arrives in time.
async fn until_live(socket: &mut Socket, pane: &str, seconds: u64) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] != "convo.turn" || message["pane"] != pane {
            continue;
        }
        for turn in message["turns"].as_array().unwrap_or(&Vec::new()) {
            if turn["id"] == "live" && !turn["blocks"].as_array().is_none_or(Vec::is_empty) {
                return Some(turn.clone());
            }
        }
    }
    None
}

async fn report(session: &Session, pane: &str, state: &str) {
    session
        .call(
            "pane.report_agent",
            json!({ "pane_id": pane, "agent": "claude", "source": "kampr-test", "state": state }),
        )
        .await;
}

/// Herdr's own answer for the pane, waited for — and the point of waiting is that it is the state
/// every assertion below is *against*. A wire that says `done` proves nothing if herdr never
/// synthesised one, and `done` is synthesised only on the `working`→`idle` edge (#357), so a
/// second report that overtakes the first arms nothing at all.
async fn herdr_says(session: &Session, pane: &str, want: &str) {
    let mut saw = String::new();
    for _ in 0..100 {
        saw = session.call("pane.get", json!({ "pane_id": pane })).await["pane"]["agent_status"]
            .as_str()
            .expect("an agent_status")
            .to_string();
        if saw == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("herdr never said {want} about {pane}; it says {saw:?}");
}

/// Herdr's *own* first answer about the pane, waited for before anything is reported into it —
/// which is what makes `done` armable at all.
///
/// A label attaches ~0.2 s after the process appears and herdr then holds `unknown` for 3.33 s
/// before its idle fallback publishes (#360). A `working` report inside that window publishes
/// `working` — the report outranks the fallback (#358), and `pane.get` reads `working` for as long
/// as you care to poll — but it arms `done` for only ~0.4 s: the first screen publish lands *after*
/// the report and takes the pane's transition history with it, so the `idle` report that follows is
/// no longer the `working`→`idle` edge `done` is synthesised from, and no later report re-arms it.
/// Past that first publish the arming is durable, measured out to 10 s (#405). Locally the two
/// reports are 2 ms apart and land inside the window; a loaded runner is where the 0.4 s is spent.
///
/// This is a read, so it moves nothing (#357), and it waits on `agent` too: a status that settled
/// before herdr saw the process is the fallback for a pane with no agent, not this pane's.
async fn herdr_has_scraped(session: &Session, pane: &str) {
    herdr_pane(session, pane, 30, "scraped an agent out of", |p| {
        p["agent"] == "claude" && p["agent_status"] == "idle"
    })
    .await;
}

/// Herdr's own answer about a pane, polled until it is the one asked for.
///
/// Every caller is waiting on something herdr does on its own clock rather than on anything this
/// suite drives, so the failure message carries what herdr last said — a pane that never got a
/// label and a pane whose label is the wrong one are different faults and used to read alike.
async fn herdr_pane(
    session: &Session,
    pane: &str,
    seconds: u64,
    what: &str,
    ready: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut saw = Value::Null;
    while tokio::time::Instant::now() < deadline {
        saw = session.call("pane.get", json!({ "pane_id": pane })).await["pane"].clone();
        if ready(&saw) {
            return saw;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("herdr never {what} {pane}; it says {saw}");
}

/// The pane's status as the herd holds it *now*, rather than as some frame once carried it.
///
/// The socket is the honest level for "the client is told", and its sibling above uses it. It is
/// the wrong level for a claim about which of two sources won, because a patch built before the
/// state was armed says `working` for reasons that have nothing to do with `done`. The model is
/// the newest answer there is, and it is the value the encoder serialises.
async fn until_herd(h: &Harness, pane: &str, want: &str) {
    let mut saw = String::new();
    for _ in 0..150 {
        saw = h
            .node
            .herd()
            .pane(pane)
            .map(|entry| serde_json::to_value(entry).unwrap()["agent_status"].to_string())
            .unwrap_or_default();
        if saw.trim_matches('"') == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the herd never called {pane} {want}; it says {saw}");
}

/// A rebuild, asked for by a read.
///
/// A marker changing on disk signals nothing — no herdr event, no provider revision — and the
/// sweep behind it is `HERD_RECONCILE`, thirty seconds. The watcher count is the one thing a
/// client can move that rebuilds the model on demand, and watching leaves herdr's own answer
/// exactly where it was: every read does (#357).
async fn pump(socket: &mut Socket, pane: &str) {
    send(socket, json!({ "t": "watch", "pane": pane })).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    send(socket, json!({ "t": "unwatch", "pane": pane })).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// One `claude`-shaped harness in a pane, and the session file it would have written.
///
/// The process is real: a copy of `sleep` under the name the pane's harness has, run in the pane
/// so that herdr's own `pane.process_info` reports it and the node reads its pid from there. What
/// is written by hand is only the *contents* of `~/.claude/sessions/<pid>.json`, whose shape is
/// checked against a verbatim capture of `claude` 2.1.239 in
/// `kampr-journal/tests/fixtures/identity`.
struct Harnessed {
    home: PathBuf,
    project: PathBuf,
    cwd: String,
    binary: PathBuf,
}

impl Harnessed {
    fn new(home: &Path, work: &Path) -> Self {
        let cwd = work.canonicalize().unwrap().display().to_string();
        let project = home.join(".claude/projects").join(cwd.replace('/', "-"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(home.join(".claude/sessions")).unwrap();
        let binary = home.join("claude");
        std::fs::copy(which("sleep").expect("sleep on PATH"), &binary).unwrap();
        Self {
            home: home.to_path_buf(),
            project,
            cwd,
            binary,
        }
    }

    /// Runs the harness in the pane and waits for herdr to report it, answering with its pid.
    async fn start(&self, session: &Session, local: &str) -> u32 {
        session
            .call(
                "pane.send_text",
                json!({ "pane_id": local, "text": format!("{} 600\n", self.binary.display()) }),
            )
            .await;
        for _ in 0..100 {
            let info = session
                .call("pane.process_info", json!({ "pane_id": local }))
                .await;
            let found = info["process_info"]["foreground_processes"]
                .as_array()
                .and_then(|ps| ps.iter().find(|p| p["name"] == "claude"))
                .and_then(|p| p["pid"].as_u64());
            if let Some(pid) = found {
                return pid as u32;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("herdr never reported a claude process in the pane");
    }

    async fn stop(&self, session: &Session, local: &str, pid: u32) {
        std::fs::remove_file(self.session_file(pid)).unwrap();
        session
            .call("pane.send_keys", json!({ "pane_id": local, "keys": ["ctrl+c"] }))
            .await;
        for _ in 0..100 {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn session_file(&self, pid: u32) -> PathBuf {
        self.home.join(".claude/sessions").join(format!("{pid}.json"))
    }

    /// What the harness publishes about itself, with the pid and `procStart` of the process that
    /// is really running — so a node that checks them against `/proc` is checking real values.
    fn announce(&self, pid: u32, id: &str) {
        self.announce_status(pid, id, "idle");
    }

    fn announce_status(&self, pid: u32, id: &str, status: &str) {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("/proc stat");
        let start = stat[stat.rfind(") ").unwrap() + 2..]
            .split_whitespace()
            .nth(19)
            .expect("field 22")
            .to_string();
        let record = json!({
            "pid": pid, "sessionId": id, "cwd": self.cwd, "procStart": start,
            "version": "2.1.239", "kind": "interactive", "entrypoint": "cli", "status": status,
        });
        std::fs::write(self.session_file(pid), record.to_string()).unwrap();
    }

    /// A further turn on a transcript that is already open, which is what the follow tick sends
    /// as a `convo.turn` rather than as a page.
    fn append(&self, id: &str, uuid: &str, text: &str) {
        let at = time::OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let record = json!({
            "type": "assistant", "uuid": uuid, "cwd": self.cwd, "timestamp": at,
            "message": { "content": [ { "type": "text", "text": text } ] },
        });
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.project.join(format!("{id}.jsonl")))
            .unwrap();
        std::io::Write::write_all(&mut file, format!("{record}\n").as_bytes()).unwrap();
    }

    /// A one-turn transcript for a session, stamped `offset` seconds from now — so a test can put
    /// another run's transcript *ahead* of the one the pane is really on.
    fn transcript(&self, id: &str, text: &str, offset: i64) {
        let at = (time::OffsetDateTime::now_utc() + time::Duration::seconds(offset))
            .replace_nanosecond(0)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let record = json!({
            "type": "assistant", "uuid": format!("{id}-1"), "cwd": self.cwd, "timestamp": at,
            "message": { "content": [ { "type": "text", "text": text } ] },
        });
        std::fs::write(self.project.join(format!("{id}.jsonl")), format!("{record}\n")).unwrap();
    }
}

/// The operator's two reports, end to end: *"i opened claude on a terminal … that had never
/// opened claude and its showing me the most recent session"*, and *"closed claude -> opened
/// again fresh session -> conversation panel showing old and not updating to new at all"*.
///
/// Both are one defect — a transcript resolved from the pane's *directory* — and both are driven
/// here against a real herdr, with a real process in the pane and the pid read out of herdr's own
/// `pane.process_info`. The decoy transcript is deliberately the newest thing in the directory,
/// which is what every previous rule went by.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_shows_the_session_its_own_process_is_on_and_moves_when_it_restarts() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("identity", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let first = fixture.start(&h._session, &local).await;
    fixture.announce(first, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    // Somebody else working in the same directory while this pane sat idle, and the newest
    // transcript in it by half a minute — which is what every rule before this one went by.
    fixture.transcript("99999999-9999-4999-8999-999999999999", "SOMEBODY ELSE", -30);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;

    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let turns = opening["turns"].as_array().expect("turns").clone();
    assert_eq!(
        turns.len(),
        1,
        "the pane's own session, not the directory's newest: {opening}"
    );
    assert_eq!(turns[0]["blocks"][0]["text"], "FIRST SESSION");
    let stale: Vec<String> = turns
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // Quit the agent and start a fresh one in the same pane, in the same directory. Nothing about
    // the pane changes except the process — herdr goes on reporting `claude` throughout, because
    // its detection is a screen scrape and this test is what a stale one looks like.
    fixture.stop(&h._session, &local, first).await;
    // And somebody else works in the directory in the gap — after the pane's *shell* started, so
    // only the harness's own start time excludes it, and there is no harness.
    fixture.transcript("99999999-9999-4999-8999-999999999998", "SOMEBODY ELSE AGAIN", 0);
    assert!(
        !h.claims_a_conversation(&pane).await,
        "a pane with nothing running in it advertised the directory's newest transcript"
    );
    let second = fixture.start(&h._session, &local).await;
    assert_ne!(second, first);
    fixture.announce(second, "22222222-2222-4222-8222-222222222222");
    fixture.transcript("22222222-2222-4222-8222-222222222222", "SECOND SESSION", 0);

    // The new conversation arrives, and the old one is taken off the client first — a page merges
    // by id, so without the withdrawal the new turns land *above* the old ones and the panel
    // reads as though it never updated.
    let stale = moved_to(&mut socket, &pane, &stale, "SECOND SESSION").await;

    // `/clear` is the same move without a new process: claude rewrites `sessionId` in place in
    // `~/.claude/sessions/<pid>.json` and opens a new transcript under the same pid, so nothing
    // about the pane changes at all (#259). `/compact` does neither and never moves a pane.
    fixture.announce(second, "33333333-3333-4333-8333-333333333333");
    fixture.transcript("33333333-3333-4333-8333-333333333333", "AFTER CLEAR", 0);
    moved_to(&mut socket, &pane, &stale, "AFTER CLEAR").await;
}

/// The same defect one screen away: the transcript moves while nobody is watching the pane.
///
/// **The withdrawal lives with the pump and the conversation lives with the client, and the two
/// have different lifetimes.** A pump is created by `watch` and aborted by `unwatch` — which is
/// what leaving a pane's screen does — while the client's turns are kept for the life of the app.
/// So a `/clear` between leaving a pane and coming back is served by a pump that has never shown
/// this client anything, withdraws nothing, and sends a page whose ids match none of what is on
/// the screen: the new conversation lands *above* the old one and the panel reads as though it
/// never updated.
#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_that_moved_while_the_pane_was_unwatched_is_still_taken_off_the_client() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("unwatched", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let turns = opening["turns"].as_array().expect("turns").clone();
    assert_eq!(turns[0]["blocks"][0]["text"], "FIRST SESSION", "{opening}");
    let mut stale: Vec<String> = turns
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // The conversation goes on after its first page, and a turn that arrived as a revision is on
    // the client exactly as firmly as one that arrived in the page.
    fixture.append(
        "11111111-1111-4111-8111-111111111111",
        "first-session-2",
        "STILL THE FIRST SESSION",
    );
    let grown = until_pane(&mut socket, "convo.turn", &pane, 25).await;
    assert_eq!(grown["turns"][0]["id"], "first-session-2", "{grown}");
    stale.push("first-session-2".to_string());

    // The operator leaves the pane's screen. `AppState` unwatches, and the client keeps every
    // turn it was ever sent — `paneStates` is never pruned.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    // And `/clear`s in the terminal while the phone is on some other screen.
    fixture.announce(pid, "22222222-2222-4222-8222-222222222222");
    fixture.transcript("22222222-2222-4222-8222-222222222222", "AFTER CLEAR", 0);

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    moved_to(&mut socket, &pane, &stale, "AFTER CLEAR").await;
}

/// Herdr decides a pane's agent status by scraping its screen (#75), and a pane blocked on a
/// prompt looks exactly like a pane that has finished — so `blocked` is a state the scrape
/// structurally cannot reach, and it answers `idle` for both.
///
/// The harness writes down what it is actually doing, in a file this node already opens once per
/// pane per rebuild for the title. Asserted against herdr saying something *else*, on purpose: a
/// test that only checked the status arrived would pass with the override removed.
#[tokio::test(flavor = "multi_thread")]
async fn a_harness_that_says_it_is_waiting_outranks_the_screen_that_says_it_is_working() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("waiting", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce_status(pid, "33333333-3333-4333-8333-333333333333", "waiting");
    report(&h._session, &local, "working").await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until_status(&mut socket, &pane, "blocked", 30).await;
}

/// The operator's report: *"when an agent is done done — wasm desktop at least doesn't show
/// anything, just goes grey."*
///
/// Herdr's `done` is a pane that finished `working`→`idle` while **unfocused** (#357) — the
/// operator's unread flag — and a finished Claude session writes `status: "idle"` into its own
/// marker at that same moment. Both are true; `done` is the one that also says nobody has looked.
/// The harness outranking the screen turned that into a demotion, and `Idle` renders grey with no
/// status mark at all, which is the "doesn't show anything".
///
/// The mutation that must fail: take the `done` arm off `settled_status` in `state.rs` and this
/// hangs on `idle` for the full thirty seconds.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_that_finished_unwatched_stays_done_when_its_harness_says_idle() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("done", |c: &mut Config| c.journals.home = home_path);
    // `workspace.create` does not take the focus, so the harness's own `kampr` workspace keeps it
    // and this pane is the unfocused one `done` needs. Nothing here focuses anything: `pane`,
    // `tab` and `workspace` focus all destroy the marker under test.
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce_status(pid, "44444444-4444-4444-8444-444444444444", "idle");
    herdr_has_scraped(&h._session, &local).await;
    report(&h._session, &local, "working").await;
    herdr_says(&h._session, &local, "working").await;
    report(&h._session, &local, "idle").await;
    herdr_says(&h._session, &local, "done").await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until_status(&mut socket, &pane, "done", 30).await;
}

/// The three the harness still outranks, on one pane in sequence: `done` is the exception it may
/// not correct, not the end of the override.
///
/// Every step moves the herd off the answer the step before it left, and every step's herdr state
/// is confirmed with herdr *before* the assertion — a wire that says `working` proves nothing
/// about `done` if herdr never synthesised one. The mutation that must fail: delete the override
/// from `build_model` and the pane never leaves what herdr scraped — `done` at the second step
/// and the third, `working` at the fourth.
#[tokio::test(flavor = "multi_thread")]
async fn a_harness_still_outranks_the_screen_for_every_word_that_is_not_done() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("outranks", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();
    let pid = fixture.start(&h._session, &local).await;
    let session = "55555555-5555-4555-8555-555555555555";

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    // Nothing written down yet, so the herd carries herdr's own answer and nothing else — and
    // herdr's own answer is what the step after this one needs to have landed before it reports.
    herdr_has_scraped(&h._session, &local).await;
    until_herd(&h, &pane, "idle").await;

    report(&h._session, &local, "working").await;
    herdr_says(&h._session, &local, "working").await;
    report(&h._session, &local, "idle").await;
    herdr_says(&h._session, &local, "done").await;
    until_herd(&h, &pane, "done").await;

    // A pane that has started again is not one waiting to be read, whatever herdr synthesised.
    fixture.announce_status(pid, session, "busy");
    pump(&mut socket, &pane).await;
    until_herd(&h, &pane, "working").await;

    // And `waiting` is the word the scrape structurally cannot reach (#360), over `done` as over
    // anything else herdr can publish.
    fixture.announce_status(pid, session, "waiting");
    pump(&mut socket, &pane).await;
    until_herd(&h, &pane, "blocked").await;

    // The half of the override that has nothing to do with `done`: a screen herdr reads as
    // working, against a harness that says it has stopped.
    report(&h._session, &local, "working").await;
    herdr_says(&h._session, &local, "working").await;
    fixture.announce_status(pid, session, "idle");
    pump(&mut socket, &pane).await;
    until_herd(&h, &pane, "idle").await;
}

/// The same move, with the operator *looking at* the pane.
///
/// A `/clear` or a restart points the marker at a session whose transcript does not exist
/// until a first message is sent — 0.1 s after a `/clear` and 2 min 42 s from launch,
/// measured ([#259](docs/03-probe-log.md), #311). For that whole window this node has no
/// conversation to send, and it used to send nothing at all: the previous session's turns
/// stayed on the screen and took no new ones, which is the panel "showing old and not
/// updating to new at all" seen from the node's end.
///
/// The mutation that must fail: take the retirement off the move and this hangs, holding a
/// conversation the pane has already left.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_has_written_nothing_takes_the_previous_conversation_off_the_client() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("unwritten", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let turns = opening["turns"].as_array().expect("turns").clone();
    assert_eq!(turns[0]["blocks"][0]["text"], "FIRST SESSION", "{opening}");
    let stale: Vec<String> = turns
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // The agent opens a session of its own and has not said anything on it yet. Nothing
    // else about the pane moves — same pid, same `procStart`, same directory (#259).
    fixture.announce(pid, "22222222-2222-4222-8222-222222222222");

    retired(&mut socket, &pane, &stale).await;
}

/// The same window reached the way an operator actually reaches it: **quit the agent and run it
/// again in the same terminal**, rather than `/clear`ing it in place.
///
/// The two look identical on the wire and are not. A `/clear` moves the announcement straight
/// from one session to the next under one pid, so the node sees two names that disagree. A
/// restart goes through a third state — the process is gone, nothing announces, and the pane
/// names no session at all — and the rule that withdrew a conversation only when the *previous
/// tick* named a different one saw `A -> none` and then `none -> B`, disagreed with neither, and
/// left the first session's turns on the screen. The reader then had the panel from the run
/// before, and the ask for older turns that a stale cursor still offered was answered
/// `not_found`, which is the reported "no conversation open for this pane".
///
/// No transcript is written for the second session on purpose: that is the whole gap (#311), and
/// a test that writes one is passed by [`deliver`]'s withdrawal instead of by the rule under test.
///
/// The mutation that must fail: compare the announcement against the previous handle rather than
/// against the last session the pane was seen on, and this hangs.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_quit_and_run_again_takes_the_previous_conversation_off_the_client() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("rerun", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let first = fixture.start(&h._session, &local).await;
    fixture.announce(first, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let turns = opening["turns"].as_array().expect("turns").clone();
    assert_eq!(turns[0]["blocks"][0]["text"], "FIRST SESSION", "{opening}");
    let stale: Vec<String> = turns
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // Ctrl-C, then `claude` again at the same prompt. Herdr goes on calling the pane `claude`
    // across both, because its detection is a screen scrape.
    fixture.stop(&h._session, &local, first).await;
    let second = fixture.start(&h._session, &local).await;
    assert_ne!(second, first);
    fixture.announce(second, "22222222-2222-4222-8222-222222222222");

    retired(&mut socket, &pane, &stale).await;
}

/// The other half of the same gap, and the one that actually happens: the transcript **does not
/// move** while the pane is unwatched, it just grows.
///
/// A page merges by id, and the merge an installed phone performs files what it does not recognise
/// at the *top* — a rule written for `convo.load`, which pages backwards. Re-opening the
/// transcript the client is already holding pages forwards, so the turns it is missing are the
/// newest ones and every one of them lands above a conversation from hours earlier, on a view
/// pinned to the bottom. That is a message that was never dropped and never seen, and never
/// revisited either, because the node then records it as delivered.
///
/// Phones on older releases cannot be fixed from the client side, so the node does not send them a
/// page it knows they will misfile. Asserted through the *old* merge on purpose: a test that only
/// checks the turn arrived passes with the defect restored.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_written_while_the_pane_was_unwatched_lands_below_the_conversation_not_above_it() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("regrown", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "WHICH KEYS?", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let mut screen: Vec<String> = opening["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(!screen.is_empty(), "{opening}");

    // The operator leaves the pane's screen. `AppState` unwatches; the client keeps every turn it
    // was ever sent, because `paneStates` is never pruned.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;

    // The agent answers while nobody is on that screen. Same transcript, same session, one turn
    // longer — this is the ordinary case, not `/clear`.
    fixture.append(
        "11111111-1111-4111-8111-111111111111",
        "the-answer",
        "HERE IS THE ANSWER",
    );

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["pane"] != pane {
            continue;
        }
        let tag = message["t"].as_str().unwrap_or("");
        if tag != "convo" && tag != "convo.turn" {
            continue;
        }
        merge_as_an_older_client_would(&mut screen, &message);
        if screen.iter().any(|id| id == "the-answer") {
            assert_eq!(
                screen.last().map(String::as_str),
                Some("the-answer"),
                "the answer was filed above the conversation instead of below it: {screen:?} from {message}",
            );
            return;
        }
    }
    panic!("the answer never reached the client at all");
}

/// The merge performed by every client already installed on a phone, from
/// `docs/04-wire-protocol.md`: a page replaces known ids in place and **prepends** the rest unless
/// it says `fresh`, and a `convo.turn` replaces known ids in place and appends the rest.
fn merge_as_an_older_client_would(screen: &mut Vec<String>, message: &Value) {
    let turns = message["turns"].as_array().cloned().unwrap_or_default();
    let ids: Vec<String> = turns
        .iter()
        .filter_map(|t| t["id"].as_str())
        .map(str::to_string)
        .collect();
    if message["t"] == "convo" && message["fresh"] == json!(true) {
        screen.clear();
    }
    let unknown: Vec<String> = ids.iter().filter(|id| !screen.contains(id)).cloned().collect();
    if message["t"] == "convo" {
        screen.splice(0..0, unknown);
    } else {
        screen.extend(unknown);
    }
}

/// A client that **freezes rather than closes** is reaped, and one that is merely quiet is not.
///
/// Every path where the node learns the client is gone already tears down cleanly — `unwatch`, a
/// clean close, an abrupt RST, the pane closing (#284). The hole is the peer whose kernel is alive
/// and ACKing while the application never reads again: a phone frozen in the background, a
/// suspended laptop, a NAT that dropped the flow. Its socket sits in TCP zero-window persist,
/// which resets the probe counter on every probe, so nothing below ever errors, the writer never
/// breaks, and `outbox.close()` never happens. Measured: the watch was still held after 25
/// minutes, costing herdr exactly what a watched pane costs and holding one of 64 socket permits
/// for ever. The mesh link has had this guard all along; the link a phone uses had none.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_stops_answering_is_dropped_and_a_quiet_one_is_not() {
    let h = harness!("keepalive", |c: &mut Config| c.limits.client_keepalive_secs = 1);
    let deadline = Duration::from_secs(3);
    let token = h.token(Role::Full).await;

    // The control, and it has to come first: a client that answers is *quiet* on this wire too —
    // it sends nothing at all once it has greeted — so a reaper keyed on inbound traffic rather
    // than on the pong would take every idle phone with it, and this test would be the only thing
    // between that and a release. Polling the socket is the whole of what an awake client does:
    // `tungstenite` answers the node's pings from inside `next`, and never touching it is what
    // "frozen" means below.
    let mut awake = h.connect(&token).await;
    until(&mut awake, "hello", 10).await;
    assert!(
        !closed_within(&mut awake, deadline * 3).await,
        "an idle client that still answers its pings was dropped"
    );
    send(&mut awake, json!({ "t": "resync" })).await;
    until(&mut awake, "herd", 10).await;

    // And now one that goes away without saying so. Nothing is read from this socket at all, so
    // every ping the node sends goes unanswered.
    let mut frozen = h.connect(&token).await;
    until(&mut frozen, "hello", 10).await;
    tokio::time::sleep(deadline * 3).await;
    assert!(
        closed_within(&mut frozen, Duration::from_secs(20)).await,
        "a client that stopped answering was still being served past its deadline"
    );
}

/// The expensive case end to end: a frozen client **holding a watch on a producing pane**, which
/// is the one that costs herdr what a live watcher costs (#284) — a frozen client on a quiet pane
/// costs nothing but a socket permit.
///
/// **What this does not prove.** The intended guard here is the deadline on `out.send`: once the
/// socket's buffers fill, the send pends inside the `select!` and the keepalive arm is never polled
/// again, so the ping cannot fire at all. Disabling the ping shows this test still failing, so the
/// send never stalls for a whole deadline at this volume — `pump_pane` purges a congested pane's
/// frames rather than queueing them, which keeps the writer from feeding the socket fast enough to
/// fill it. The state is real regardless: #284 captured `Send-Q 2636553 notsent rwnd_limited:100%`
/// on a socket in exactly this condition. So the send deadline stands on that measurement and on a
/// reading of the `select!`, and **not** on this test, which the ping is what passes.
#[tokio::test(flavor = "multi_thread")]
async fn a_frozen_client_watching_a_producing_pane_is_dropped() {
    let h = harness!("keepalive-write", |c: &mut Config| c
        .limits
        .client_keepalive_secs = 1);
    let token = h.token(Role::Full).await;
    let pane = h.pane_id();

    let mut frozen = h.connect(&token).await;
    until(&mut frozen, "hello", 10).await;
    send(
        &mut frozen,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until_pane(&mut frozen, "grid.reset", &pane, 25).await;

    // From here the socket is never read again, and the pane is told to fill it. The wait has to
    // be a bare sleep: `closed_within` *reads*, and a read is `tungstenite` answering the node's
    // pings from inside `next` — which is the client waking up, not the client being frozen.
    send(
        &mut frozen,
        json!({ "t": "input", "pane": pane, "text": "seq 1 200000\n" }),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(15)).await;

    assert!(
        closed_within(&mut frozen, Duration::from_secs(30)).await,
        "a client that stopped reading was written to for ever instead of being dropped"
    );
}

/// Whether the node closed this socket, as distinct from it having nothing to say. [`recv`] folds
/// a timeout, a close and an undecodable frame into the same `None`, and the difference is the
/// whole claim here.
async fn closed_within(socket: &mut Socket, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), socket.next()).await {
            Err(_) => continue,
            Ok(None) | Ok(Some(Err(_))) => return true,
            Ok(Some(Ok(tungstenite::Message::Close(_)))) => return true,
            Ok(Some(Ok(_))) => continue,
        }
    }
    false
}

/// The half a withdrawal cannot reach: a client that comes back on a **different socket**.
///
/// The node's record of what a client is holding lives with that client's session, and a
/// reconnecting phone gets a new one — `KamprConnection` re-watches every pane it holds and
/// `paneStates` survives the drop, so the turns are still on the screen and the node has no way to
/// name them. Server-side memory cannot close this either: the node itself restarts, and a client
/// that reconnects to a restarted node is in exactly the same position. So the page says so, and
/// the client drops what it holds for the pane before applying it. Additive: a build that ignores
/// the field behaves as it does today.
///
/// The negative is the other half of the claim. `convo.load` answers with older slices of the
/// *same* transcript and those must merge, or paging backwards would throw away the page above.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_a_reconnecting_client_could_not_have_been_told_about_replaces_what_it_holds() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("reconnect", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    assert_eq!(opening["turns"][0]["blocks"][0]["text"], "FIRST SESSION");
    assert_eq!(
        opening["fresh"], true,
        "a first page is a conversation starting, and the node cannot know what this socket \
         arrived holding: {opening}"
    );

    let cursor = opening["cursor"].as_str().expect("a cursor").to_string();
    send(
        &mut socket,
        json!({ "t": "convo.load", "pane": pane, "before": cursor }),
    )
    .await;
    let older = until_pane(&mut socket, "convo", &pane, 15).await;
    assert!(
        older["fresh"].is_null(),
        "paging backwards through one transcript must merge, or the page above it goes: {older}"
    );

    // The phone goes to sleep, `/clear` happens in the terminal, and the phone comes back on a
    // socket this node has never seen.
    drop(socket);
    fixture.announce(pid, "22222222-2222-4222-8222-222222222222");
    fixture.transcript("22222222-2222-4222-8222-222222222222", "AFTER CLEAR", 0);

    let mut back = h.connect(&token).await;
    until(&mut back, "hello", 10).await;
    send(
        &mut back,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let after = until_pane(&mut back, "convo", &pane, 25).await;
    assert_eq!(after["turns"][0]["blocks"][0]["text"], "AFTER CLEAR", "{after}");
    assert_eq!(
        after["fresh"], true,
        "the old conversation is still on this client and nothing has told it to let go: {after}"
    );
}

/// Waits for the client to be told to let go of every turn it holds. Unlike [`moved_to`]
/// there is no conversation to move *to*: the session the pane is on has written nothing,
/// and an empty screen is the honest answer for as long as that lasts.
async fn retired(socket: &mut Socket, pane: &str, stale: &[String]) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["pane"] != pane || message["t"] != "convo.turn" {
            continue;
        }
        let gone: Vec<String> = message["turns"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|t| t["blocks"].as_array().is_none_or(Vec::is_empty))
            .filter_map(|t| t["id"].as_str())
            .map(str::to_string)
            .collect();
        if stale.iter().all(|id| gone.contains(id)) {
            return;
        }
    }
    panic!("the previous conversation was never taken off the client");
}

/// Waits for the pane to move to the conversation whose only turn reads `text`, and answers with
/// the ids the client is left holding. Fails unless the previous conversation was withdrawn on
/// the way, because a page that merges leaves the old one underneath the new.
async fn moved_to(socket: &mut Socket, pane: &str, stale: &[String], text: &str) -> Vec<String> {
    let mut withdrawn = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["pane"] != pane {
            continue;
        }
        let turns = message["turns"].as_array().cloned().unwrap_or_default();
        if message["t"] == "convo.turn" {
            let retired: Vec<&str> = turns
                .iter()
                .filter(|t| t["blocks"].as_array().is_none_or(Vec::is_empty))
                .filter_map(|t| t["id"].as_str())
                .collect();
            withdrawn |= stale.iter().all(|id| retired.contains(&id.as_str()));
        }
        if message["t"] == "convo" {
            let texts: Vec<&str> = turns
                .iter()
                .filter_map(|t| t["blocks"][0]["text"].as_str())
                .collect();
            assert_eq!(texts, [text], "{message}");
            assert!(
                withdrawn,
                "the previous conversation was never taken off the client"
            );
            return turns
                .iter()
                .map(|t| t["id"].as_str().unwrap().to_string())
                .collect();
        }
    }
    panic!("the pane never moved to the conversation reading {text:?}");
}

/// A node with nothing to do rebuilds its herd because something changed, not because it can.
///
/// Release discovery is declined by dropping the sender, and a `watch` whose sender is gone
/// answers `changed()` immediately and for ever — so the rebuild loop selected on a channel that
/// was always ready and went round as fast as it could, pinging every herdr it serves on every
/// pass. Nothing in the model moved, so nothing on the wire ever showed it; what it cost was a
/// core, and everything else on the runtime being starved of one.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_with_release_discovery_off_rebuilds_its_herd_only_when_something_changes() {
    let h = harness!("quiet");
    let mut herd = h.node.subscribe_herd();
    herd.borrow_and_update();
    // Nothing is watched, nothing is connected and nothing moves in this window, and the
    // reconcile sweep behind it is thirty seconds — so the honest number of rebuilds is none.
    let mut rebuilds = 0u32;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::timeout_at(deadline, herd.changed()).await.is_ok() {
        rebuilds += 1;
    }
    assert!(
        rebuilds <= 8,
        "the herd was rebuilt {rebuilds} times in two seconds with nothing to rebuild it for"
    );
}

/// Reported from a phone: a workspace made from the New sheet comes up with nothing on it, typing
/// into it puts nothing on the screen, and there is no shell anywhere in sight.
///
/// Everything `a_paired_device_drives_a_pane_end_to_end` proves, against a pane that did not exist
/// when the herd was first sent — which is the only difference, and the whole report.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_created_from_the_client_is_a_pane_you_can_type_into() {
    let h = harness!("newws");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "workspace.create", "node": node,
                "label": "typed", "cwd": "/tmp" }),
    )
    .await;
    let ack = managed(&mut socket, "workspace.create", 25).await;
    assert_eq!(ack["ok"], true, "{ack}");
    let workspace = ack["id"].as_str().expect("a workspace id").to_string();
    let pane = patch_adding(&mut socket, &workspace, 30).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let reset = until_pane(&mut socket, "grid.reset", &pane, 25).await;
    assert!(
        reset["rows"].as_u64().unwrap_or(0) > 0,
        "the new pane arrived with no grid at all: {reset}"
    );
    // A shell that has printed its prompt and nothing else puts every non-blank row at the very
    // top of a 40-row grid, with the caret on row 0 — the shape the phone reported as blank.
    assert_eq!(reset["cursor"]["row"], 0, "{reset}");

    let marker = "kampr-new-workspace-marker";
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": format!("echo {marker}\n") }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut echoed = false;
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline && !echoed {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] == "error" {
            seen.push(message.to_string());
        }
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        echoed = message["pane"] == pane.as_str() && message.to_string().contains(marker);
    }
    assert!(
        echoed,
        "a workspace the client made never echoed what was typed into it; errors: {seen:?}"
    );
}

/// Reported from a phone: a pane that produced a burst of output showed a few lines from the top
/// of the screen and never the bottom, and reopening it showed the same truncated screen.
///
/// The layout rect's height is not the PTY's — probe #205: a `down` split halves the rect and
/// leaves the PTY at its old size — and `observe --rows` *crops* to the rows it is handed rather
/// than scrolling to the bottom (probe #206). Sizing the stream from the rect therefore serves
/// every client the top of the pane and nothing else, permanently: there is no later frame that
/// repairs it, because the bottom of the pane was never in the stream to begin with.
#[tokio::test(flavor = "multi_thread")]
async fn a_split_pane_is_observed_at_the_pty_height_not_the_rect() {
    let h = harness!("ptyheight");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    h._session
        .call(
            "pane.split",
            json!({ "target_pane_id": local, "direction": "down" }),
        )
        .await;

    let pty = viewport_rows(&h._session, &local).await;
    let rect = a_halved_rect_height(&h._session, &local, pty).await;
    assert!(
        rect < pty,
        "the split must have halved the rect: rect {rect}, PTY {pty}"
    );

    // Content past the rect, so the bottom of the pane is only reachable at the PTY's height —
    // and drawn by a painter rather than typed at the operator's prompt. ble.sh re-renders the
    // command line it accepted *after* the command's own output and can wipe it outright
    // ([#446](#)); a screenful of filler ending in a marker on the last row is exactly the shape
    // that redraw scrolls away, and the marker never comes back.
    let marker = "kampr-bottom-marker";
    a_painter_on_the_pane(&h, &mut socket, &pane, &local).await;
    let filler: String = (1..=pty - 5).map(|i| format!("filler-{i}\\n")).collect();
    paint_screen(&mut socket, &pane, &format!("{filler}{marker}")).await;
    // Waited for rather than slept over: the claim below is that the *repaint* carries the bottom
    // of the pane, and a repaint of a screen the filler has not finished reaching carries it
    // honestly and says nothing. Two seconds covered that on an idle box and lost three of ten
    // full-suite runs under sixteen spinners. Herdr's screen and not the node's, because nothing
    // is watching this pane yet — the node has no emulator for it until the `watch` below, and it
    // starts that one from whatever herdr is holding.
    assert!(
        drawn_on_herdr(&h._session, &local, marker, 60).await,
        "the pane never printed {marker}"
    );

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    let mut heights = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "grid.reset" || message["pane"] != pane.as_str() {
            continue;
        }
        heights.push(message["rows"].as_u64().unwrap_or(0));
        if message["rows"].as_u64() == Some(pty as u64) && message.to_string().contains(marker) {
            return;
        }
    }
    panic!(
        "the pane was never streamed at its PTY's {pty} rows with the bottom of the screen on it; \
         the grid came back at {heights:?} rows, and the rect claims {rect}"
    );
}

/// The rect once the split has halved it, waited for on herdr's own clock. Its floor is the PTY's
/// height, which the rect is only below once the layout has actually moved.
async fn a_halved_rect_height(session: &Session, pane: &str, pty: u16) -> u16 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut rect = pty;
    while tokio::time::Instant::now() < deadline {
        rect = rect_height(session, pane).await;
        if rect < pty {
            return rect;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    rect
}

async fn rect_height(session: &Session, pane: &str) -> u16 {
    let layout = session.call("pane.layout", json!({ "pane_id": pane })).await;
    layout["layout"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["rect"]["height"].as_u64())
        .unwrap() as u16
}

/// Herdr's own `scroll.viewport_rows` — the PTY's height, which is what the program in the pane
/// is actually writing to, and the half of a pane's geometry herdr answers honestly (#84). It is
/// therefore the only evidence available that a `pane.size` landed at all, columns being reported
/// by nothing anywhere (#221).
async fn viewport_rows(session: &Session, pane: &str) -> u16 {
    let snapshot = session.call("session.snapshot", json!({})).await;
    snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["scroll"]["viewport_rows"].as_u64())
        .unwrap() as u16
}

/// The operator's own suggested repair — "on opening a session we could ask for the freshest
/// data". A second `watch` must repaint from the pane's *current* screen rather than attaching
/// silently to a stream already in flight, or a pane that moved while nobody was looking stays
/// wrong until it happens to move again.
///
/// A second viewer holds the pane open throughout, so the registry entry survives the `unwatch`
/// and the reopening client is answered from the emulator every viewer shares — the case where
/// there is no fresh `observe` to repaint it by accident.
#[tokio::test(flavor = "multi_thread")]
async fn reopening_a_pane_repaints_it_from_the_screen_as_it_is_now() {
    let h = harness!("reopen");
    let mut held = h.connect(&h.token(Role::Full).await).await;
    let mut socket = h.connect(&h.token(Role::Full).await).await;
    for s in [&mut held, &mut socket] {
        until(s, "hello", 10).await;
        until(s, "herd", 10).await;
    }
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut held, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut held, "grid.reset", &pane, 20).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 20).await;
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;

    // Everything from here happens while this client is not looking — and it has to have actually
    // happened before the client comes back, or the repaint it gets is honest about a screen the
    // marker had not reached yet and every frame that carries it afterwards is a `grid.patch`
    // this test is not looking for. A fixed two seconds covered that on an idle box and not on a
    // contended one, where it went red on a run with nothing else on the machine. Waited for on
    // both the screens the node reads: a reopening client is repainted from the node's own
    // emulator, so the marker has to have reached that one too.
    let marker = "kampr-while-away-marker";
    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": format!("echo {marker}\n") }),
        )
        .await;
    assert!(
        drawn(&h, &local, marker, 60).await,
        "the pane never printed {marker} while the client was away"
    );

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut resets = 0;
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "grid.reset" || message["pane"] != pane.as_str() {
            continue;
        }
        resets += 1;
        if message.to_string().contains(marker) {
            return;
        }
    }
    panic!(
        "reopening the pane never repainted it as it is now: {resets} grid.reset frames, none of \
         them carrying what the pane printed while this client was away"
    );
}

/// Herdr's cell boundary is UAX #29's and its column count is not `unicode-width`'s: a conjoining
/// jamo block is **one** cell of two columns however many lead jamo it stacks, and an unpaired
/// regional indicator gets the two columns a whole flag gets. Both are only visible through
/// scrollback — `pane.read` hands the node the raw code points and the node lays the row out
/// itself, where the live grid gets herdr's own cursor addressing to lean on.
#[tokio::test(flavor = "multi_thread")]
async fn a_jamo_block_and_a_lone_flag_half_arrive_in_the_columns_herdr_spends() {
    let h = harness!("jamo");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    // Only scrollback shows the node laying a row out on its own, and a pane has no scrollback
    // until it has scrolled — so the three lines are pushed off the screen on purpose.
    let script =
        "printf '%b\\n' 'AB\\u1100\\u1100CD' 'AB\\U0001F1EBCD' 'AB\\u1100\\u1161\\u11a8CD'; seq 1 200\n";
    send(&mut socket, json!({ "t": "input", "pane": pane, "text": script })).await;

    let wanted = [
        ("two lead jamo", "\u{1100}\u{1100}"),
        ("an unpaired regional indicator", "\u{1F1EB}"),
        ("a jamo syllable block", "\u{1100}\u{1161}\u{11A8}"),
    ];
    let mut found: Vec<Option<Vec<Option<String>>>> = vec![None; wanted.len()];
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline && found.iter().any(Option::is_none) {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "scrollback" {
            continue;
        }
        for row in message["rows"].as_array().cloned().unwrap_or_default() {
            let cols = clusters(&row["runs"]);
            let text = cluster_text(&cols);
            for (i, (_, cluster)) in wanted.iter().enumerate() {
                if text == format!("AB{cluster}CD") {
                    found[i] = Some(cols.clone());
                }
            }
        }
    }

    for (i, (name, cluster)) in wanted.iter().enumerate() {
        let cols = found[i]
            .clone()
            .unwrap_or_else(|| panic!("no row came back reading AB{cluster}CD ({name})"));
        assert_eq!(cols[1].as_deref(), Some("B"), "{name}: the base row is intact");
        assert_eq!(
            cols[2].as_deref(),
            Some(*cluster),
            "{name} is one cell carrying every code point herdr kept"
        );
        assert_eq!(cols[3], None, "{name}: column 3 is the cluster's other half");
        assert_eq!(
            cols[4].as_deref(),
            Some("C"),
            "{name}: C is in column 4, two columns past the cluster"
        );
    }
}

/// Probe #231. The interlock used to refuse history to any pane with a detected agent, on an
/// inherited hazard that was never measured: that reading above the viewport there harvests
/// through the agent's own mouse-scroll interface and moves the operator's screen. It does not —
/// a live `codex` and a live `claude`, both herdr-detected and both holding a ring, answer
/// `lines: 5000` in 1 ms with the whole ring and the viewport where it was. So the pane that is
/// the entire point of the product gets its history like any other.
#[tokio::test(flavor = "multi_thread")]
async fn a_detected_agent_pane_delivers_its_history_too() {
    let h = harness!("agenthistory");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "seq 1 400\n" }),
    )
    .await;

    let history = until(&mut socket, "scrollback", 25).await;
    let text: String = history["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .flat_map(|r| r["runs"].as_array().cloned().unwrap_or_default())
        .filter_map(|run| run["x"].as_str().map(str::to_string))
        .collect();
    assert!(text.contains("100"), "an agent pane's ring is a ring: {history}");
}

/// The reported defect, and the half of it that made it survive for months: a node whose PATH has
/// no herdr serves a correct herd, accepts input and answers every health check, while every pane
/// shows a blank grid and a flashing cursor for ever. The grid was a promise the node made before
/// it knew it could keep it, and the failure to keep it went to a journal that nobody holding a
/// phone can read.
///
/// This is the same machine `transfer.rs` builds, minus the part that hid it: that harness points
/// the socket at nothing too, so its supervisor parks waiting for geometry and never reaches the
/// spawn at all. A node has two ways to reach herdr and can have exactly one of them working.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_that_cannot_run_herdr_says_so_instead_of_promising_a_grid() {
    let nowhere = std::env::temp_dir().join(format!("kampr-no-herdr-{}", std::process::id()));
    let h = harness!("noobserve", |c: &mut Config| {
        c.herdr.binary = nowhere.display().to_string();
    });
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let herd = until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    assert_eq!(
        herd["nodes"][0]["online"], true,
        "the socket is fine and the herd is right — that is what made this invisible: {herd}"
    );

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut refusal: Option<Value> = None;
    let mut detail: Option<String> = None;
    let mut promised = false;
    while tokio::time::Instant::now() < deadline && (refusal.is_none() || detail.is_none()) {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        match message["t"].as_str() {
            Some("grid.reset") if message["pane"] == pane.as_str() => promised = true,
            Some("error") if message["pane"] == pane.as_str() => refusal = Some(message),
            Some("herd") | Some("herd.patch") => detail = detail.or(pane_detail(&message, &pane)),
            _ => {}
        }
    }

    let refusal = refusal.expect("a node that cannot stream a pane has to say so to the client");
    assert_eq!(refusal["code"], "stream_unavailable", "{refusal}");
    let said = refusal["message"].as_str().unwrap_or_default();
    eprintln!("what the operator reads on the phone:\n{said}");
    assert!(
        said.contains("herdr"),
        "the message has to name what is wrong: {said}"
    );
    assert!(
        said.contains("PATH") || said.contains("herdr.binary"),
        "the operator is holding a phone and cannot read a journal, so it has to name the fix: {said}"
    );
    assert!(
        !promised,
        "the node sent a grid it had no stream for; the geometry is a promise it cannot keep"
    );
    let detail = detail.expect("the herd has to carry the state, not just announce the event");
    assert!(detail.contains("herdr"), "{detail}");
}

/// Retrying for ever is right; saying nothing while retrying for ever is not — and neither is
/// leaving the notice up once the pane can paint again. A binary appearing is the recovery this
/// supervisor already retries for, so the whole story has to clear itself.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_recovers_on_its_own_once_herdr_can_be_run() {
    let Some(real) = which("herdr") else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    let bin = tempfile::tempdir().expect("a bin dir");
    let shim = bin.path().join("herdr");
    let h = harness!("recover", |c: &mut Config| {
        c.herdr.binary = shim.display().to_string();
    });
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let refusal = until_pane(&mut socket, "error", &pane, 30).await;
    assert_eq!(refusal["code"], "stream_unavailable", "{refusal}");

    std::fs::write(&shim, format!("#!/bin/sh\nexec {} \"$@\"\n", real.display())).expect("a shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    // Both halves in one pass: the herd clears at the spawn and the grid arrives at the first
    // frame after it, so waiting for one and then the other misses whichever came first.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut painted = false;
    let mut cleared = false;
    while tokio::time::Instant::now() < deadline && !(painted && cleared) {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        match message["t"].as_str() {
            Some("grid.reset") if message["pane"] == pane.as_str() => {
                assert!(message["rows"].as_u64().unwrap_or(0) > 0, "{message}");
                painted = true;
            }
            Some("herd") | Some("herd.patch") if names(&message, &pane) => {
                cleared = pane_detail(&message, &pane).is_none();
            }
            _ => {}
        }
    }
    assert!(painted, "the pane never painted once herdr could be run");
    assert!(cleared, "the notice outlived the fault it was about");
}

/// Whether a herd message says anything about this pane at all, so "no detail" is read off an
/// entry that is present rather than off one that simply is not in the patch.
fn names(message: &Value, pane: &str) -> bool {
    pane_entries(message).any(|p| p["id"] == pane)
}

fn pane_detail(message: &Value, pane: &str) -> Option<String> {
    pane_entries(message)
        .find(|p| p["id"] == pane)
        .and_then(|p| p["detail"].as_str().map(str::to_string))
}

fn pane_entries(message: &Value) -> impl Iterator<Item = &Value> {
    ["panes", "added", "changed"]
        .into_iter()
        .flat_map(move |key| match message[key].as_array() {
            Some(panes) => panes.iter().collect::<Vec<_>>(),
            None => message[key]["panes"]
                .as_array()
                .map_or_else(Vec::new, |p| p.iter().collect()),
        })
}

/// A named session joins the herd as its own node, and it did so only on the next discovery poll
/// — up to fifteen seconds after the operator watched their own create succeed, with the herd
/// screen showing nothing in between (#240). The op is the one thing that *knows* the session set
/// changed, so it reconciles rather than leaving the loop to find out for itself.
///
/// Four seconds is not an arbitrary bound. `discover` polls on a `tokio::time::interval`, whose
/// first tick fires the moment the node starts, so its ticks land at node start + 0, +15, +30.
/// Both ops here land two to four seconds in, which is the middle of that window — without the
/// reconcile there is no tick anywhere near this deadline, and with it the herd moves in
/// milliseconds.
#[tokio::test(flavor = "multi_thread")]
async fn a_created_session_is_in_the_herd_by_the_time_its_ack_arrives() {
    let session = format!("kampr-herdjoin-{}", std::process::id());
    // The harness serves only its own session, for the reason its comment gives. This one has to
    // serve the session it is about to make as well — and nothing else, so a throwaway herd
    // belonging to another test running beside it is still not this test's herd (#97).
    let allowed = session.clone();
    let h = harness!("herdjoin", |c: &mut Config| {
        c.herdr.sessions = Some(vec![allowed.clone()]);
    });
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let joined = format!("{node}.{session}");
    let _created = CreatedSession::named(&session);
    // Reports what it actually saw when it gives up. A bare `false` here made a real intermittent
    // unattributable: the herd and herdr's own list can disagree, and which of them is wrong is
    // the whole question.
    let serving = async |want: bool| -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        loop {
            if h.node.herd().nodes.iter().any(|n| n.id == joined) == want {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let herd: Vec<String> = h.node.herd().nodes.iter().map(|n| n.id.clone()).collect();
                let listed = match kampr_node::caps::sessions(&h.node.config.herdr.binary).await {
                    Ok(found) => found
                        .iter()
                        .map(|s| format!("{}={}", s.name, s.running))
                        .collect::<Vec<_>>()
                        .join(" "),
                    Err(e) => format!("<the list could not be read: {e}>"),
                };
                return Err(format!(
                    "wanted {joined} present={want}; herd has [{}]; herdr lists [{listed}]",
                    herd.join(" ")
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if let Err(saw) = serving(false).await {
        panic!("the session is in the herd before it was made — {saw}");
    }

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    if let Err(saw) = serving(true).await {
        panic!("the session was acknowledged and the herd waited for the poll to hear about it — {saw}");
    }

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    if let Err(saw) = serving(false).await {
        panic!("the session was stopped and is still an online node — {saw}");
    }
}

/// Probe #272: keypress-to-glyph, socket in to socket out, against a real herdr.
///
/// The hop this measures is everything the node owns — websocket read, `pane.send_text` over the
/// herdr socket, the PTY, herdr's own render and its `observe` frame, the `vte` emulator, the diff
/// and the wire encode — but not the browser's frame-drained input queue and not the LAN. It is
/// the number that did not exist: #22 measured herdr alone through `session control`, and #257
/// measured a frame's round trip under an attachment's load, but nothing measured a keystroke
/// going all the way through a node and coming back as a glyph.
///
/// One character at a time and no newline: a shell that runs a command answers on its own
/// schedule, and this is asking what the echo costs.
///
/// **It is `bash`'s echo, and on a machine with `ble.sh` that is nearly all of the number.** The
/// same keystrokes into a pane running `cat` come back in **1.2 ms** and into `bash --norc` in
/// 1.2 ms, against 26.5 ms into an interactive `bash` with the operator's rc (#273) — so what this
/// records is a ceiling on the whole path, not a reading of the node's share, and the node's share
/// is the part that is genuinely below the instrument.
///
/// **So it is measured twice, and only the second one is asserted on.** #273's split is the whole
/// reason: one budget over both halves is a budget over the operator's `.bashrc`, and that is what
/// went red five times out of five under sixteen spinners while the node was doing nothing wrong.
/// Measured here, p50 / max:
///
/// | | idle | 16 spinners | whole suite + 16 spinners |
/// |---|---|---|---|
/// | the pane's own shell on the path | 8.5 ms / 109 ms | 39 ms / 284 ms | **73 ms / 1800 ms** |
/// | no shell on the path | 2.4 ms / 141 ms | 9.0 ms / 156 ms | **16.9 ms / 162 ms** |
///
/// The shell's half is unbounded — a second and three quarters at its worst — and it is not this
/// suite's to bound. The node's half moves by seven over the same range and its tail stays inside
/// 162 ms, which is what a budget can be derived from. The first arm keeps its liveness: every one
/// of its keystrokes still has to come back inside ten seconds or the test says which one did not.
#[tokio::test(flavor = "multi_thread")]
async fn a_keystroke_comes_back_as_a_glyph_and_the_round_trip_is_recorded() {
    let h = harness!("latency");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 15).await;

    // A prompt that has not finished painting keeps sending frames of its own, and they would be
    // counted as the answer to the first keystroke.
    drain(&mut socket, Duration::from_millis(600)).await;

    // The whole path, with whatever shell the operator's machine puts in a pane. Recorded, and
    // guarded only by the ten-second deadline inside every round: this is #272's number and there
    // is nothing here that can make a `.bashrc` faster.
    let shell = keypress_to_glyph(&mut socket, &pane, 48).await;
    eprintln!(
        "keypress-to-glyph, the pane's own shell on the path: {}",
        quantiles(&shell)
    );

    // The same path with the shell taken off it, which is the half this node owns.
    //
    // `stty sane` first, and it is not tidiness: ble.sh leaves an idle pane's tty with ECHO
    // already off (#333) and `exec` keeps the termios, so `cat` inherits a tty that echoes
    // nothing and the first keystroke never comes back at all. Waited for on herdr's own answer
    // about the pane's foreground process rather than slept over.
    let local = pane.rsplit('/').next().unwrap().to_string();
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "stty sane; exec cat\n" }),
    )
    .await;
    let mut bare_pane = false;
    for _ in 0..600 {
        let info = h
            ._session
            .call("pane.process_info", json!({ "pane_id": local }))
            .await;
        bare_pane = info["process_info"]["foreground_processes"]
            .as_array()
            .is_some_and(|ps| ps.iter().any(|p| p["name"] == "cat"));
        if bare_pane {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        bare_pane,
        "the pane never got a shell-free process on it to measure"
    );
    drain(&mut socket, Duration::from_millis(600)).await;
    let bare = keypress_to_glyph(&mut socket, &pane, 48).await;
    eprintln!("keypress-to-glyph, no shell on the path: {}", quantiles(&bare));

    // **The floor, not the median, and from the second arm only.**
    //
    // A median is the statistic contention moves: it went 4.6 ms to 63.4 ms between two runs of
    // this suite with nothing else on the box, because a share of the readings park on a ~110 ms
    // plateau that is the *test's own* wakeup latency under a parallel suite and not anything the
    // node did. The floor cannot be moved that way — a busy machine can only ever add — while a
    // regression that puts constant latency in the path shifts every reading including the
    // fastest. Both directions are measured: the floor of this arm is 1.7–2.4 ms idle, 6.0–6.8 ms
    // under sixteen spinners and 6.6–8.0 ms with the whole suite as well, and with a deliberate
    // 150 ms parked in the node's own `input` it is 152.6 ms. A hundred is twelve times the worst
    // floor and a third of the mutation.
    //
    // What the floor cannot see is a regression that is slow only sometimes; the whole
    // distribution is printed above for exactly that, and #272 is where it is read.
    assert!(
        bare[0] < 100.0,
        "the fastest of {} keystrokes still took {:.1} ms to come back as a glyph with no shell \
         in the way, so the node's own share of the path has grown",
        bare.len(),
        bare[0]
    );
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.0) * q).round() as usize]
}

fn quantiles(sorted: &[f64]) -> String {
    format!(
        "over {} readings: min {:.1} ms  p50 {:.1} ms  p90 {:.1} ms  max {:.1} ms",
        sorted.len(),
        sorted[0],
        percentile(sorted, 0.50),
        percentile(sorted, 0.90),
        sorted[sorted.len() - 1],
    )
}

/// Keypress-to-glyph readings for one pane, sorted, in milliseconds.
async fn keypress_to_glyph(socket: &mut Socket, pane: &str, rounds: u32) -> Vec<f64> {
    let mut readings = Vec::new();
    for round in 0..rounds {
        // Every keystroke is a character the screen does not already hold, so the frame that
        // carries it cannot be a repaint of something older.
        let ch = char::from(b'a' + (round % 26) as u8);
        let at = std::time::Instant::now();
        send(
            socket,
            json!({ "t": "input", "pane": pane, "text": ch.to_string() }),
        )
        .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut took = None;
        while tokio::time::Instant::now() < deadline && took.is_none() {
            let Some(message) = recv(socket, Duration::from_secs(2)).await else {
                continue;
            };
            if message["pane"] != pane {
                continue;
            }
            if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
                continue;
            }
            if message.to_string().contains(ch) {
                took = Some(at.elapsed());
            }
        }
        let took = took.unwrap_or_else(|| panic!("round {round}: {ch} never came back as a glyph"));
        readings.push(took.as_secs_f64() * 1000.0);

        // Take the line back so the next keystroke is landing on a clean prompt, and wait an
        // uneven amount before the next one. A fixed cadence puts every keystroke at the same
        // phase of whatever else in the path is periodic and the readings all land together: the
        // first run of this used a flat 120 ms and measured a p50 of 98 ms, against 31 ms with the
        // jitter (#272). That was read as a ~100 ms tick in herdr and it is not one — herdr's only
        // periodicity is a 16 ms floor between frames and it delivers a write in ~1 ms (#274) —
        // so the aliasing is in the pane's own shell. The jitter stays because the number moves
        // by a factor of three without it.
        //
        // **The last sentence is the one this suite has since contradicted, and it has not been
        // reprobed.** The plateau is at ~110 ms rather than ~100 ms, and it shows up on the arm
        // with *no shell on the pane at all* — p90 104–117 ms in every loaded run of the suite,
        // against a p50 of 13–17 ms and a floor of 7 ms. Whatever it is, a shell is not required
        // to produce it, and it tracks how oversubscribed the box is rather than what the pane is
        // running: it is most likely this test's own wakeup latency and not the node's. That is a
        // guess and it is why nothing here asserts on a quantile it can reach.
        // **`Backspace`, because `BSpace` is not a key herdr has.** herdr 0.8.2 answers it
        // `invalid_key: unsupported key BSpace` and delivers no byte, so for as long as this said
        // `BSpace` the line was never taken back and every reading after the first landed on a
        // prompt one character longer than the last. Probe #7's grammar has `Backspace` and `BS`;
        // both send `0x7f` and both erase. It also cost a refused herdr call per round, and a
        // refused call is still a call — worst case a whole 100 ms poll (#445).
        send(
            socket,
            json!({ "t": "input", "pane": pane, "keys": ["Backspace"] }),
        )
        .await;
        let jitter = 50 + (round as u64 * 37) % 190;
        drain(socket, Duration::from_millis(jitter)).await;
    }
    readings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    readings
}

async fn drain(socket: &mut Socket, quiet: Duration) {
    while recv(socket, quiet).await.is_some() {}
}

/// Every parameter in `agent.view.set` is a guess unless a real herdr accepts it, and this is the
/// one call in the naming path with **no read-back at all** — there is no `agent.view.get`, and
/// `agent.list` is untouched by the view (probe #296), so a schema that drifted would go on
/// answering an error nothing looks at. The reply is the whole of what herdr will say back:
/// `active`, the `source`, and the `label` that replaced the sort-mode word in its header.
#[tokio::test(flavor = "multi_thread")]
async fn herdr_accepts_the_agents_view_the_node_sets_and_gives_the_desk_back_on_clear() {
    let Some(session) = Session::start("agentview").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    let view = kampr_core::agent_view::View::by_name();
    let set = session
        .herdr()
        .set_agent_view(&view.source, &view.token, view.order, &view.label)
        .await
        .unwrap_or_else(|e| panic!("herdr refused the view the node sets: {e}"));
    assert_eq!(
        set,
        kampr_herdr::AgentView {
            active: true,
            source: Some(view.source.clone()),
            label: Some(view.label.clone()),
        }
    );

    let cleared = session
        .herdr()
        .clear_agent_view()
        .await
        .expect("herdr clears the view");
    assert!(!cleared.active, "the desk has its own order back");
}

/// **A paste is bytes that become a path.** An agent reached over ssh reads a local file
/// perfectly well and chokes on a terminal's image-paste protocol, so nothing here speaks one:
/// the node writes what it is given beside its own state and types in where it put it.
///
/// The pane is a real herdr pane and the text really is typed, because the half that could
/// silently not happen is the typing — a file written into a directory nobody looks at is exactly
/// the shape of a feature that reports success and does nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_paste_lands_on_the_node_and_its_path_is_typed_into_the_pane() {
    let h = harness!("paste");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    // A PNG's magic bytes, so the extension is decided by the body and can be seen to have been.
    let body = b"\x89PNG\r\n\x1a\n\x00kampr-paste-body";
    send(
        &mut socket,
        json!({
            "t": "paste",
            "pane": pane,
            "b64": base64::engine::general_purpose::STANDARD.encode(body),
            "name": "shot",
        }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut typed: Option<String> = None;
    while tokio::time::Instant::now() < deadline && typed.is_none() {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        let painted = message.to_string();
        if let Some(at) = painted.find("shot-") {
            typed = Some(painted[at..].chars().take(64).collect());
        }
    }

    let typed = typed.expect("the path was never typed into the pane");
    assert!(
        typed.contains(".png"),
        "the extension came from somewhere other than the bytes: {typed}"
    );

    let dir = h.node.state_dir.join("pastes");
    let written: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("no paste directory at {dir:?}: {e}"))
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(written.len(), 1, "expected one pasted file in {dir:?}");
    assert_eq!(
        std::fs::read(&written[0]).expect("the pasted file"),
        body,
        "the bytes on disk are not the bytes that were sent"
    );
}

/// **A screenshot is not a few kilobytes, and the socket has to be able to carry one.**
///
/// Every other client message is small — a `prefs` blob is capped at 2 KiB — so the socket's
/// message ceiling was set at 1 MiB to keep a stranger from handing the JSON parser something
/// enormous. `paste` is the one message that is legitimately large: the client refuses at 8 MiB
/// and says so on screen, and everything between the two ceilings was accepted by the client,
/// encoded, sent, and then killed the connection. The client had already said "sent", because
/// there is no acknowledgement for a paste — so an attachment simply never arrived and nothing
/// anywhere said why.
///
/// 2 MiB is a phone photograph, and above the old ceiling by enough that no rounding reaches it.
#[tokio::test(flavor = "multi_thread")]
async fn a_paste_the_size_of_a_photograph_crosses_the_socket_rather_than_closing_it() {
    let h = harness!("paste-big");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    let mut body = b"\x89PNG\r\n\x1a\n".to_vec();
    body.resize(2 * 1024 * 1024, b'k');
    send(
        &mut socket,
        json!({
            "t": "paste",
            "pane": pane,
            "b64": base64::engine::general_purpose::STANDARD.encode(&body),
            "name": "photo",
        }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut typed: Option<String> = None;
    while tokio::time::Instant::now() < deadline && typed.is_none() {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        let painted = message.to_string();
        if let Some(at) = painted.find("photo-") {
            typed = Some(painted[at..].chars().take(64).collect());
        }
    }
    typed.expect("a 2 MiB paste never reached the pane");

    let dir = h.node.state_dir.join("pastes");
    let written: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("no paste directory at {dir:?}: {e}"))
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(written.len(), 1, "expected one pasted file in {dir:?}");
    assert_eq!(
        std::fs::read(&written[0]).expect("the pasted file").len(),
        body.len(),
        "the bytes on disk are not the bytes that were sent"
    );
}

/// **A conversation must not be torn down because the agent went from busy to idle.**
///
/// The pump decides a conversation has moved by comparing what identifies the pane's session, and
/// answers a difference by taking the open transcript off the client and paging a fresh one. So
/// anything that merely *describes* a session and changes while it runs — the harness's own
/// `status`, flipping every turn — must stay out of that comparison, or every turn ends by
/// replacing the reader's conversation with its newest page. That is #314's defect wearing a
/// different hat, and it was briefly reintroduced by carrying the session marker in the wrong
/// struct.
///
/// **What this does and does not prove.** The pane here runs a shell, so the marker resolves to
/// nothing and a status field could not move even if one were back in [`Identity`] — this pins the
/// invariant and would catch a field that churns on *any* pane, but only a real agent pane could
/// catch one that churns solely on an agent's. The comment on `Identity` is what carries the rest.
#[tokio::test(flavor = "multi_thread")]
async fn what_the_pump_compares_holds_still_while_the_agent_works() {
    let h = harness!("identity");
    let pane = h.pane_id();
    let (_, local) = h.node.resolve(&pane).expect("a local pane");
    let (session, _) = h.node.resolve(&pane).expect("a local pane");
    let journals = h.node.journals();

    let first = kampr_node::convo::identity(&journals, &session.provider, &local);
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            kampr_node::convo::identity(&journals, &session.provider, &local),
            first,
            "what the pump compares moved on its own, which re-pages the conversation"
        );
    }
}

/// The reported surprise, end to end: `input` is herdr's `pane.send_text` and it **appends** to
/// whatever is already on the pane's line, so a sentence begun at the desk and a reply sent from a
/// phone submit as one run-on line — and nothing on the phone ever showed the first half. The desk
/// line is that first half, lifted off the same grid the client is already streaming.
///
/// The pane runs a shell rather than a real Claude, and paints Claude's own composer into it with
/// a `printf` that leaves no newline: the caret then rests past the marker exactly as it does in
/// the captures under `kampr-journal/tests/fixtures/composer`, which is the thing being read.
#[tokio::test(flavor = "multi_thread")]
async fn what_the_operator_left_in_the_panes_own_composer_reaches_the_phone_before_it_is_added_to() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("9f1c0b2e-0000-4000-8000-000000000044.jsonl");

    let home_path = home.path().display().to_string();
    let h = harness!("desk", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    become_harness(&h._session, &local, home.path(), "claude").await;
    // Nothing is reported into a pane herdr has not published about yet: a report inside the
    // post-label `unknown` hold is overtaken by herdr's own first screen publish (#405), and every
    // status this test then reports would be racing it.
    herdr_has_scraped(&h._session, &local).await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    let (body, _) = claude_transcript("/tmp", 2);
    std::fs::write(&transcript, &body).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    until_pane(&mut socket, "convo", &pane, 25).await;

    // U+276F, then the non-breaking space Claude 2.1.250 separates its composer with, then the
    // half-sentence — written as raw bytes because `pane.send_text` carries them intact (#9), and
    // with no newline after them so the caret rests past the marker the way a person's would. The
    // `sleep` is what keeps it there: a shell that returns paints its own prompt onto the same
    // row, and the row would then no longer be the one the operator left.
    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local,
                "text": "clear; printf '\u{276f}\u{a0}push the branch when'; sleep 60\n" }),
        )
        .await;

    let seen = until_pane(&mut socket, "convo.composer", &pane, 25).await;
    assert_eq!(
        seen["text"], "push the branch when",
        "the half-sentence at the desk did not reach the phone: {seen}",
    );
    assert_eq!(
        seen["clear"], "\u{3}",
        "the keystroke measured to clear Claude's composer did not ride with it: {seen}",
    );

    // And it comes down again when the desk empties the box, or nothing on the phone would ever
    // stop claiming a line that is no longer there.
    h._session
        .call("pane.send_text", json!({ "pane_id": local, "text": "\u{3}" }))
        .await;
    h._session
        .call("pane.send_text", json!({ "pane_id": local, "text": "clear\n" }))
        .await;
    let gone = until_pane(&mut socket, "convo.composer", &pane, 25).await;
    assert!(
        gone["text"].is_null(),
        "an emptied composer left the strip standing: {gone}",
    );
}

impl Harnessed {
    /// The marker as the harness really writes it: `~/.claude/sessions/<pid>.json` carries the
    /// name it derived at session open, `kampr-44`, with `nameSource: "derived"` (#311).
    fn announce_named(&self, pid: u32, id: &str, name: &str, source: &str) {
        self.announce(pid, id);
        let path = self.session_file(pid);
        let mut record: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        record["name"] = json!(name);
        record["nameSource"] = json!(source);
        std::fs::write(path, record.to_string()).unwrap();
    }

    /// A record kind the transcript carries and the marker never does. Claude rewrites `ai-title`
    /// as the session goes, so a later one replaces an earlier one.
    fn titled(&self, id: &str, title: &str) {
        let record = json!({ "type": "ai-title", "aiTitle": title, "sessionId": id });
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.project.join(format!("{id}.jsonl")))
            .unwrap();
        std::io::Write::write_all(&mut file, format!("{record}\n").as_bytes()).unwrap();
    }
}

/// The operator's report: two Claude panes in one workspace render identically, because the only
/// name the herd path had was one the harness made up for itself — the cwd basename and two hex
/// characters — and that one is dropped (#311) rather than shown.
///
/// The good name is in the transcript and always was. This drives the whole path: a real herdr, a
/// real process in the pane, the marker the harness writes by pid, and an `ai-title` record on the
/// transcript the pane's own process is on.
///
/// The mutation that must fail: take the transcript out of the title and this pane comes back
/// with no title at all, because `kampr-44` is refused and nothing else is in hand.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_is_called_what_the_transcript_calls_it_rather_than_what_the_harness_made_up() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("titled", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let id = "44444444-4444-4444-8444-444444444444";
    let pid = fixture.start(&h._session, &local).await;
    fixture.announce_named(pid, id, "kampr-44", "derived");
    fixture.transcript(id, "A TURN", -60);
    fixture.titled(id, "the width inference rewrite");
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    assert_eq!(
        pane_title(&mut socket, &pane).await.as_deref(),
        Some("the width inference rewrite"),
        "the harness derived `kampr-44` for itself and the transcript holds the real name"
    );
}

/// The precedence the conversation surface publishes, end to end on the herd path: a title the
/// operator typed beside the session outranks the one the harness generated, and neither is
/// displaced by the name it derived.
#[tokio::test(flavor = "multi_thread")]
async fn a_title_the_operator_typed_outranks_the_one_the_harness_generated() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("typed", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let id = "55555555-5555-4555-8555-555555555555";
    let pid = fixture.start(&h._session, &local).await;
    fixture.announce_named(pid, id, "kampr-55", "derived");
    fixture.transcript(id, "A TURN", -60);
    fixture.titled(id, "Inferring a pane's width");
    let tree = fixture.project.join(id);
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(
        tree.join("custom-title.json"),
        json!({ "customTitle": "the release" }).to_string(),
    )
    .unwrap();
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    assert_eq!(
        pane_title(&mut socket, &pane).await.as_deref(),
        Some("the release")
    );
}

/// The full herd arrives once and everything after it is a patch, so a pane's title can land in
/// any of the three.
async fn pane_title(socket: &mut Socket, pane: &str) -> Option<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut saw = None;
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        let found = [
            &message["panes"],
            &message["changed"]["panes"],
            &message["added"]["panes"],
        ]
        .into_iter()
        .filter_map(|panes| panes.as_array())
        .flat_map(|panes| panes.iter())
        .find(|p| p["id"] == pane);
        let Some(entry) = found else {
            continue;
        };
        saw = entry["title"].as_str().map(str::to_string);
        if saw.is_some() {
            return saw;
        }
    }
    saw
}

/// Every transcript a node opened, and the pane it opened it for.
///
/// `pump_convo` records this because a pane served the *wrong* conversation used to leave no trace
/// at all; it is also the only honest answer to "was the transcript found and parsed again", which
/// a stopwatch can only approximate. Installed once for the whole binary and read by one test —
/// every other test emits into it and never asks.
static OPENED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn opens(pane: &str) -> usize {
    OPENED.lock().unwrap().iter().filter(|p| *p == pane).count()
}

fn recording_opens() {
    use tracing_subscriber::layer::SubscriberExt;
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Opened));
    });
}

struct Opened;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Opened {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != "kampr_node::convo" {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.message.as_deref() == Some("conversation opened")
            && let Some(pane) = fields.pane
        {
            OPENED.lock().unwrap().push(pane);
        }
    }
}

#[derive(Default)]
struct Fields {
    message: Option<String>,
    pane: Option<String>,
}

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let slot = match field.name() {
            "message" => &mut self.message,
            "pane" => &mut self.pane,
            _ => return,
        };
        *slot = Some(format!("{value:?}"));
    }
}

/// Reopening a pane's conversation must not cost what opening it cost.
///
/// The grid beside it is warm — the registry holds a pane's stream across a re-watch by design
/// (#252) — and the conversation was not: `watch` built a journal, `resolve` found the file,
/// the first `poll` parsed the whole of it and the fold read it again, every single time. Measured
/// on a 30.7 MB transcript, four rounds against one pane: **1.99 s to the first conversation
/// message and 0.86 s more for the facets, and every re-watch cost the same 1.9 s** (#409). That
/// is what a reader saw as a conversation that had gone out of date and would not come back.
///
/// **The count of opens rather than a stopwatch.** A ratio was the first instrument here and it
/// was a proxy: the cold side is a parse and shrinks with the machine, the warm side is a socket
/// round trip and a scheduler and does not, so the two do not scale together. This machine
/// measures 16x unloaded and 43x with its cores taken away; a GitHub runner measured 3.80x against
/// a bar of 4x, having *worked* — 507 ms where the parse cost 1.93 s. What the test claims is that
/// the transcript was not opened again, and the node says so itself.
#[tokio::test(flavor = "multi_thread")]
async fn reopening_a_conversation_serves_the_one_it_already_had_rather_than_parsing_it_again() {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let session = "9f1c0b2e-0000-4000-8000-0000000003f4";
    let transcript = project.join(format!("{session}.jsonl"));

    recording_opens();
    let home_path = home.path().display().to_string();
    let h = harness!("warm", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    let _ = until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    let pid = become_harness(&h._session, &local, home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    // The marker a real Claude writes, so this measures the rung a real pane is resolved on: the
    // pid names the session exactly, and the handle alone answers whether the transcript moved.
    // Without it the pane falls back to the working directory, where every re-open has to re-derive
    // the transcript and the figures below are a different claim (#412).
    write_session_marker(home.path(), pid, session);
    // Big enough that finding and parsing it is the whole cost, which is the case the report came
    // from: a session that had been running for days.
    let (body, _) = claude_transcript(cwd, 90_000);
    // A prompt waiting on the harness, so the fold has something to say. Without it both rounds
    // publish `{}` and the claim about what a warm fold hands a reader who has just arrived is
    // untested — a fold answers the *difference*, and a warm one has usually not moved.
    let queued = json!({
        "type": "queue-operation", "operation": "enqueue",
        "timestamp": "2026-08-28T02:10:59.658Z", "content": "and copy the config across"
    });
    std::fs::write(&transcript, format!("{body}{queued}\n")).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    let cold = watch_a_conversation(&mut socket, &pane).await;
    assert_eq!(
        opens(&pane),
        1,
        "opening it is what the reopen is measured against"
    );
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let warm = watch_a_conversation(&mut socket, &pane).await;

    assert!(
        warm.turns > 0,
        "the re-watch served no conversation at all, which is worse than serving it slowly",
    );
    assert_eq!(
        opens(&pane),
        1,
        "the transcript was found and parsed again for the reader who came back",
    );
    // A fold answers the *difference* since the last read, and a fold kept warm has usually not
    // moved — so a client that has just arrived would be sent `{}` and draw nothing, which is the
    // shape of the panel that showed a queued prompt to whoever asked first and to nobody after.
    assert_eq!(
        warm.facets, cold.facets,
        "the reader who came back was sent different facets from the one who opened it",
    );

    // And the socket itself going away is the other half of it: a phone in a pocket drops the
    // connection, and the session that held the pane goes with it. Warmth is the node's, not the
    // session's, so what the phone reconnects onto is still the conversation it left.
    drop(socket);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let mut second = h.connect(&token).await;
    let _ = until(&mut second, "hello", 10).await;
    let reconnected = watch_a_conversation(&mut second, &pane).await;
    assert!(
        reconnected.turns > 0,
        "the reconnecting reader was served nothing"
    );
    assert_eq!(
        opens(&pane),
        1,
        "a dropped socket threw the transcript away and the phone paid for it again",
    );
}

struct Served {
    turns: usize,
    facets: Value,
}

/// Watches a pane's conversation and answers what was served: the turns, and the facets that go
/// with them. Both are read off the transcript, and a client has neither until both have arrived.
/// `convo` or `convo.turn`, because a client already holding this transcript is sent the revision
/// rather than a page that would merge above what is on its screen.
async fn watch_a_conversation(socket: &mut Socket, pane: &str) -> Served {
    send(
        socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    let mut turns = 0;
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        let tag = message["t"].as_str().unwrap_or("?").to_string();
        if message["pane"] != pane {
            continue;
        }
        if tag == "convo" || tag == "convo.turn" {
            turns = message["turns"].as_array().map(Vec::len).unwrap_or(0);
        }
        if tag == "convo.facets" && turns > 0 {
            return Served {
                turns,
                facets: message["facets"].clone(),
            };
        }
        seen.push(tag);
    }
    panic!("the node never served the conversation; saw {seen:?}");
}

/// A pane whose transcript moved while nobody was watching must not be handed the one it had.
///
/// Keeping the parse and the fold across a re-watch (#409) is what makes coming back to a pane
/// instant, and it is also a way to be confidently wrong: the pump that inherits them has not
/// asked which transcript this pane is on *now*. Every case a harness announces is caught by the
/// handle — `/clear` mints a new session id and rewrites the pane's marker in place before the
/// next prompt is submitted (#393) — but a harness that announces nothing is resolved by working
/// directory, and there the handle is identical either side of a new transcript. That is the
/// conversation showing one session while the terminal beside it shows another, which is the one
/// thing worse than showing it slowly.
#[tokio::test(flavor = "multi_thread")]
async fn a_transcript_that_moved_while_nobody_watched_is_not_the_one_the_reader_comes_back_to() {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();

    let home_path = home.path().display().to_string();
    let h = harness!("moved", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    let _ = until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    become_harness(&h._session, &local, home.path(), "claude").await;
    // Nothing is reported into a pane herdr has not published about yet: a report inside the
    // post-label `unknown` hold is overtaken by herdr's own first screen publish (#405), and every
    // status this test then reports would be racing it.
    herdr_has_scraped(&h._session, &local).await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    let before = project.join("9f1c0b2e-0000-4000-8000-00000000ab01.jsonl");
    std::fs::write(&before, one_turn(cwd, "before", "the session the reader left")).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    assert_eq!(
        opening["turns"][0]["id"], "before",
        "the reader never saw the session they are about to leave: {opening}"
    );

    // Away, and the pane starts a new session under the same working directory and the same
    // harness — which is what `/clear` leaves behind on a harness that publishes no marker.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let after = project.join("9f1c0b2e-0000-4000-8000-00000000ab02.jsonl");
    std::fs::write(&after, one_turn(cwd, "after", "the session the pane is on now")).unwrap();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    let (drawn, retired) = until_conversation_ids(&mut socket, &pane, 25).await;
    assert!(
        drawn.contains(&"after".to_string()),
        "the reader came back to the conversation the pane had left: drew {drawn:?}",
    );
    assert!(
        !drawn.contains(&"before".to_string()),
        "both sessions were drawn at once: {drawn:?}",
    );
    // A page merges by id, and the ids of another session's transcript match nothing — so the one
    // on the screen has to be taken off before the new one lands, or they stack.
    assert!(
        retired.contains(&"before".to_string()),
        "the session the pane had left was never taken off the client: retired {retired:?}",
    );
}

/// **Stamped to the millisecond, because that is what claude writes** (#285). A record rounded
/// down to its second is one the node reads as written *before* the pane's harness whenever the
/// two fall in the same second — the directory bound compares it against a process start read at
/// nanoseconds — and the transcript is then refused for the life of that process. A fixture that
/// drops the fraction is not the harness's output, and this one was failing about one run in three
/// on the clock alone.
fn one_turn(cwd: &str, uuid: &str, text: &str) -> String {
    let now = time::OffsetDateTime::now_utc();
    let at = now
        .replace_nanosecond(now.millisecond() as u32 * 1_000_000)
        .unwrap()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    format!(
        "{}\n",
        json!({
            "type": "user", "uuid": uuid, "cwd": cwd, "timestamp": at,
            "message": { "content": text }
        })
    )
}

/// What the node draws on this pane and what it takes back off it, told apart the way a client
/// tells them apart: a turn carrying no blocks is a retirement and is drawn as nothing.
async fn until_conversation_ids(socket: &mut Socket, pane: &str, seconds: u64) -> (Vec<String>, Vec<String>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let (mut drawn, mut retired) = (Vec::new(), Vec::new());
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["pane"] != pane || (message["t"] != "convo" && message["t"] != "convo.turn") {
            continue;
        }
        for turn in message["turns"].as_array().into_iter().flatten() {
            let Some(id) = turn["id"].as_str().map(str::to_string) else {
                continue;
            };
            match turn["blocks"].as_array().is_some_and(|b| !b.is_empty()) {
                true => drawn.push(id),
                false => retired.push(id),
            }
        }
        if !drawn.is_empty() {
            return (drawn, retired);
        }
    }
    (drawn, retired)
}

/// **A pane that is asking the operator something must not blank the message it is asking about.**
///
/// Claude publishes nothing about a pending request until after it is answered (probe #42), so
/// while a pane is blocked the transcript is frozen and the screen is the only source there is.
/// The pump withdrew the live preview the moment the pane stopped `working` — so an agent that
/// wrote a message and then asked about it made that message *disappear* from the conversation,
/// while the terminal beside it went on showing both. The operator is then being asked to approve
/// something the conversation has stopped describing (#410).
///
/// `blocked` is not `idle`. A turn the operator interrupts really does leave half-written text on
/// the screen for ever and is right to be withdrawn; a turn waiting on an answer is the harness
/// having stopped on purpose, with the message still on the screen under it.
#[tokio::test(flavor = "multi_thread")]
async fn the_message_a_pane_is_asking_about_stays_on_the_conversation_while_it_asks() {
    let (h, mut socket, pane, local, _home) = an_agent_pane_with_a_transcript("asking").await;

    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "working" }),
        )
        .await;
    // The agent writes, a paint at a time. A preview is a block the node has watched *move*, so a
    // message that arrives all at once is not one.
    for said in [
        "I am about to remove",
        "I am about to remove the old migration file,",
        "I am about to remove the old migration file, which cannot be undone.",
    ] {
        repaint_harness_screen(
            &h,
            &mut socket,
            &pane,
            &format!("\u{25cf} {said}\\n\\n\u{276f} \\n"),
            said,
        )
        .await;
    }
    let (writing, _) = live_preview(
        &mut socket,
        &pane,
        60,
        Some("I am about to remove the old migration file, which cannot be undone."),
    )
    .await;
    assert_eq!(
        writing.as_deref(),
        Some("I am about to remove the old migration file, which cannot be undone."),
        "the conversation never showed the message the pane was writing, so this proves nothing",
    );

    // And then it asks. The message is still on the screen, above the dialog.
    repaint_harness_screen(
        &h,
        &mut socket,
        &pane,
        "\u{25cf} I am about to remove the old migration file, which cannot be undone.\\n\\n\
         Do you want to make this edit?\\n\\n 1. Yes\\n 2. No\\n\\n\u{276f} \\n",
        "Do you want to make this edit?",
    )
    .await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "blocked" }),
        )
        .await;
    // **The window is opened on the edge, not on the report.** Withdrawing is what the pump does
    // when a pane stops being live, so a watch that expires before the herd has even carried
    // `blocked` to the pump proves nothing and passes — which is the worse of the two failures a
    // load-sensitive negative assertion can have. Three seconds past the edge is fifteen turns of
    // the 200 ms live poll, and the withdrawal this guards against is published on the first.
    until_herd(&h, &pane, "blocked").await;
    let (_, retired) = live_preview(&mut socket, &pane, 3, None).await;
    assert!(
        !retired,
        "the pane asked a question and the conversation took away the message it was asking about",
    );
}

/// The same gap reached from the other side, which is the one a phone reaches it from: a push says
/// an agent is blocked, and the pane is opened *already* asking. Nothing has been watched moving,
/// the transcript is frozen, and the conversation opens on a question with no message above it.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_opened_while_it_is_already_asking_still_says_what_it_is_asking_about() {
    let (h, mut socket, pane, local, _home) = an_agent_pane_with_a_transcript("asked").await;

    repaint_harness_screen(
        &h,
        &mut socket,
        &pane,
        "\u{25cf} I am about to remove the old migration file, which cannot be undone.\\n\\n\
         Do you want to make this edit?\\n\\n 1. Yes\\n 2. No\\n\\n\u{276f} \\n",
        "Do you want to make this edit?",
    )
    .await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "blocked" }),
        )
        .await;

    let (shown, _) = live_preview(
        &mut socket,
        &pane,
        60,
        Some("I am about to remove the old migration file, which cannot be undone."),
    )
    .await;
    assert_eq!(
        shown.as_deref(),
        Some("I am about to remove the old migration file, which cannot be undone."),
        "the conversation opened on a question with no message above it",
    );
}

/// A watched agent pane with a transcript open on it, which is where both of the above start.
async fn an_agent_pane_with_a_transcript(
    tag: &'static str,
) -> (Harness, Socket, String, String, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join(format!("9f1c0b2e-0000-4000-8000-0000000{:05x}.jsonl", tag.len()));

    let home_path = home.path().display().to_string();
    let h = match Harness::start_with(tag, |c: &mut Config| c.journals.home = home_path).await {
        Some(h) => h,
        None => panic!("no herdr on PATH"),
    };
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    let _ = until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    become_harness(&h._session, &local, home.path(), "claude").await;
    // Nothing is reported into a pane herdr has not published about yet: a report inside the
    // post-label `unknown` hold is overtaken by herdr's own first screen publish (#405), and every
    // status this test then reports would be racing it.
    herdr_has_scraped(&h._session, &local).await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    let (body, _) = claude_transcript(cwd, 3);
    std::fs::write(&transcript, &body).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;
    let _ = until_pane(&mut socket, "convo", &pane, 25).await;
    (h, socket, pane, local, home)
}

/// The defect this was written for: `pending` is published on a blocked *edge* and latches on its
/// first successful read, so a dialog whose **checkboxes move under the operator** was drawn from
/// its first reading for as long as it stood. Every press on a multiple-answer question is a tick
/// (#421), and nothing else on the wire ever says which are ticked.
///
/// **Nothing interactive is left on the pane, and that is the whole of why this used to flake.**
/// The dialog was painted with `clear; printf` through the operator's own login shell, which
/// loads ble.sh. ble.sh re-renders the accepted command line on its own schedule, and under load
/// that re-render lands *after* the `printf`: the screen was left carrying the echoed command and
/// a fresh prompt, with the dialog gone and never coming back. Measured under sixteen spinners,
/// four of six full-suite runs died here, and the failing screen read back as
/// `clear; printf '%b' ' ☐ Test suites\n\n…'` over three wrapped rows and a prompt — no dialog
/// at all. So `drawn` saw the dialog, the report went in, and by the time the node read the pane
/// there was nothing on it to publish; it spent its twelve retries (`PENDING_ATTEMPTS`, six
/// seconds) on a wiped screen and correctly stopped asking. The node was right and the fixture was
/// fighting a line editor — [#442](#)'s lesson one layer up.
///
/// A painter with no echo, no prompt and no line editor is also the *truer* fixture: a real dialog
/// has the harness's own chrome above it and never a shell echo ([#407](#)), which is the entire
/// subject of [#406](#).
#[tokio::test(flavor = "multi_thread")]
async fn the_ticks_on_a_question_that_takes_several_answers_move_as_they_are_pressed() {
    let h = harness!("multipending");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 15).await;
    a_painter_on_the_pane(&h, &mut socket, &pane, &local).await;

    // The blank row above the question is what makes the reading independent of the machine
    // (#406, #407); with the shell gone there is nothing above the header at all.
    let dialog = |unit: &str, browser: &str| {
        let box_glyph = '\u{2610}';
        format!(
            " {box_glyph} Test suites\\n\\nWhich test suites should I run?\\n\\n             1. [{unit}] unit\\n  Run the unit test suite.\\n             2. [ ] integration\\n  Run the integration test suite.\\n             3. [{browser}] browser\\n  Run the browser test suite."
        )
    };

    // **Waited for on the screen, not on the clock**, and on *both* the screens the node reads.
    // This is the environment half: one write down a pty and back out of herdr, which is 5 ms
    // here and ten seconds at forty-eight spinners.
    paint_screen(&mut socket, &pane, &dialog(" ", " ")).await;
    assert!(
        drawn(&h, &local, "Run the browser test suite.", 60).await,
        "the dialog never reached the screen",
    );
    report(&h._session, &local, "blocked").await;
    // And the second environment half: herdr taking the report and the node's herd carrying it.
    // The product only has an edge to publish on once this has happened, so a budget that spans
    // it is a budget on herdr's detection and on the node's poll (#440).
    until_herd(&h, &pane, "blocked").await;

    let asked = until_pane(&mut socket, "pending", &pane, 20).await;
    assert_eq!(
        asked["multi"], true,
        "the checkboxes say what kind of question this is: {asked}"
    );
    assert_eq!(asked["header"], "Test suites");
    assert_eq!(asked["question"], "Which test suites should I run?");
    assert_eq!(
        asked["options"][0]["label"], "unit",
        "the checkbox is stripped out of the label"
    );
    assert_eq!(
        asked["options"][0]["detail"], "Run the unit test suite.",
        "the description is the whole point of this: {asked}",
    );
    assert_eq!(
        asked["options"][0]["chosen"],
        Value::Null,
        "nothing is ticked yet"
    );

    // The operator presses 1 and 3 — at the desk, or from a phone. Either way a press is a *tick*
    // (#421), the screen moves, and nothing but a re-read can say so.
    paint_screen(&mut socket, &pane, &dialog("\u{2714}", "\u{2714}")).await;
    assert!(
        drawn(&h, &local, "[\u{2714}] browser", 60).await,
        "the ticks never reached the screen"
    );

    // Not merely "the next `pending`": the erase and the redraw are one write, but the node polls
    // herdr on its own clock and may still read the screen between them. What has to arrive is the
    // frame carrying the ticks.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut moved = Value::Null;
    while tokio::time::Instant::now() < deadline {
        let Some(frame) = recv(&mut socket, Duration::from_secs(1)).await else {
            continue;
        };
        if frame["t"] != "pending" || frame["pane"] != pane {
            continue;
        }
        if frame["options"][0]["chosen"] == true {
            moved = frame;
            break;
        }
    }
    assert_ne!(
        moved,
        Value::Null,
        "the ticks froze at the first reading and never moved"
    );
    assert_eq!(moved["multi"], true, "{moved}");
    assert_eq!(moved["options"][2]["chosen"], true, "{moved}");
    assert_eq!(
        moved["options"][1]["chosen"],
        Value::Null,
        "an untouched option was reported as ticked: {moved}",
    );

    // **And a dialog nobody is touching costs nothing.** Re-reading for as long as one stands is
    // only affordable because the frame is sent when the *reading* moved, not when the tick fired
    // — without that this is two frames a second, per watched blocked pane, for as long as the
    // operator is thinking about the question.
    tokio::time::sleep(Duration::from_secs(2)).await;
    while recv(&mut socket, Duration::from_millis(200)).await.is_some() {}
    let quiet = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < quiet {
        if let Some(message) = recv(&mut socket, Duration::from_millis(500)).await {
            assert_ne!(
                message["t"], "pending",
                "the screen has not moved and the node published anyway: {message}",
            );
        }
    }
}

/// Takes the operator's shell off the pane and leaves a painter on it: one `read`, one `printf`,
/// no echo, no prompt and no line editor. Everything the pane shows from here is what a test put
/// there.
///
/// **Waited for on what the painter can do rather than on what it is called.** `/bin/sh` is a
/// symlink to a different binary on half the machines this runs on, so a process name is not the
/// condition; decoding `\145` into an `e` is, and only `printf %b` does it. A shell still holding
/// the pane echoes the probe verbatim and tries to run it, which draws neither the erase nor the
/// word.
///
/// The 60 s is a give-up threshold on the environment, not a budget on anything the node does.
async fn a_painter_on_the_pane(h: &Harness, socket: &mut Socket, pane: &str, local: &str) {
    send(
        socket,
        json!({ "t": "input", "pane": pane,
                "text": "stty -echo; exec /bin/sh -c 'while IFS= read -r row; do printf \"%b\" \"$row\"; done'\n" }),
    )
    .await;
    for _ in 0..30 {
        paint_screen(socket, pane, "paint\\145r-ready").await;
        // herdr's screen alone, because a pane nobody is watching has no other one: the node
        // opens an emulator for a pane when a client watches it, and two of the three callers
        // here are setting a pane up *before* anything does.
        if drawn_on_herdr(&h._session, local, "painter-ready", 2).await {
            return;
        }
    }
    panic!("the pane never got a painter on it");
}

/// One line to the painter: erase, then the body, in **one** write.
///
/// The erase is part of the same `printf` on purpose. Two writes leave the screen genuinely blank
/// between them, and a node that reads it there reads a pane with no dialog on it — which is a
/// truthful reading of a screen no harness ever draws, and it costs the read that mattered.
async fn paint_screen(socket: &mut Socket, pane: &str, body: &str) {
    let erase = "\\033[H\\033[2J\\033[3J";
    send(
        socket,
        json!({ "t": "input", "pane": pane, "text": format!("{erase}{body}\n") }),
    )
    .await;
}

/// Whether a screen a test painted has landed on **herdr's** screen, which is the only one there
/// is until something watches the pane.
async fn drawn_on_herdr(session: &Session, pane: &str, want: &str, seconds: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        if visible(session, pane).await.contains(want) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Whether a screen a test painted has landed — on **both** of the screens the node reads.
///
/// A sleep is not enough: herdr reports `processing input...` while a long line is still arriving,
/// and a node asked to read one mid-paste reads whatever is there. Nor is it enough on a contended
/// machine, where the shell running the `printf` competes for the same cores as the test — a fixed
/// 900 ms covered none of three paints on a box with twelve times more runnable work than cores.
///
/// Both, because the node has two screens and its two surfaces read different ones: `pending`
/// re-reads herdr over the socket, and the conversation's live preview reads the node's own
/// emulator, fed by a stream that arrives on its own schedule. Waiting for one and asserting on
/// the other is the same fixed-duration bet with extra steps.
async fn drawn(h: &Harness, local: &str, want: &str, seconds: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        let seen: Value = h
            ._session
            .call(
                "pane.read",
                json!({ "pane_id": local, "source": "visible", "format": "text", "strip_ansi": true }),
            )
            .await;
        let on_herdr = seen["read"]["text"]
            .as_str()
            .is_some_and(|text| text.contains(want));
        let on_node = h
            .node
            .primary()
            .registry
            .screen(local)
            .is_some_and(|screen| screen.rows.join("\n").contains(want));
        if on_herdr && on_node {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// The same screen for a test watching the *conversation*, painted two ways the one above is not.
///
/// **The erase is inside the `printf`.** Two commands leave the screen genuinely blank between
/// them, and the node's live preview polls every 200 ms: on a contended machine it reads that
/// blank, `Watch::stop`s on it, and the preview it was following is withdrawn and cannot come
/// back — the block it would have resumed from is the one `stop` just forgot. That withdrawal is
/// the pump behaving correctly about a screen no real harness ever draws, and it was the fixture
/// drawing it. One `printf` is one write.
///
/// **And it is waited for rather than slept over.** The fixed sleep above covered none of three
/// paints on a box with twelve times more runnable work than cores, and a preview of a message
/// nothing was ever seen writing is no preview at all.
///
/// Neither change is portable back to the sibling, which is why there are two: a `pending` test's
/// command line is part of *its* fixture — the shell echoes it, it is long enough to wrap into the
/// rows the dialog detector reads, and how it wraps turns on the ambient prompt width (#406, #407).
/// Lengthening that echo with escapes cost `the_ticks_on_a_question…` its `header`/`question`
/// split in four loaded runs out of six where `clear` cost it none.
async fn repaint_harness_screen(h: &Harness, socket: &mut Socket, pane: &str, body: &str, drawn_when: &str) {
    let erase = "\\033[H\\033[2J\\033[3J";
    send(
        socket,
        json!({ "t": "input", "pane": pane, "text": format!("printf '%b' '{erase}{body}'\n") }),
    )
    .await;
    let local = pane.rsplit('/').next().unwrap();
    assert!(
        drawn(h, local, drawn_when, 60).await,
        "the pane never painted {drawn_when:?}",
    );
}

/// What the live preview is showing on this pane, and whether it was ever taken away.
///
/// `wanted` is what turns this from a sample into a wait. A claim that the conversation *shows*
/// something is proved the instant it arrives, so watching a fixed window past that only makes the
/// test slower — and, on a loaded machine, watching a window too short for the paints to have
/// landed at all made it report that nothing was ever shown. A claim that nothing was *withdrawn*
/// has no such moment and still watches the whole window; its callers earn the window by waiting
/// for the edge the withdrawal would happen on before they open it.
async fn live_preview(
    socket: &mut Socket,
    pane: &str,
    seconds: u64,
    wanted: Option<&str>,
) -> (Option<String>, bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let (mut shown, mut retired) = (None, false);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(1)).await else {
            continue;
        };
        if message["pane"] != pane || message["t"] != "convo.turn" {
            continue;
        }
        for turn in message["turns"].as_array().into_iter().flatten() {
            if turn["id"] != "live" {
                continue;
            }
            match turn["blocks"].as_array().is_some_and(|b| !b.is_empty()) {
                true => shown = Some(turn["blocks"][0]["text"].as_str().unwrap_or("?").to_string()),
                // Sticky. A preview withdrawn and re-published a poll later is a message that
                // blinked out of the conversation, and a reader watching it happen learns not to
                // trust the surface — which is the whole complaint.
                false => retired = true,
            }
        }
        if wanted.is_some() && shown.as_deref() == wanted {
            return (shown, retired);
        }
    }
    (shown, retired)
}

/// `~/.claude/sessions/<pid>.json`, the map a harness publishes from its own process to its own
/// session. `procStart` is left out on purpose: a marker that records none is owned by whatever
/// pid it is named after, which is what a test wants and what `PaneProcess::owns` already says.
fn write_session_marker(home: &Path, pid: u32, session: &str) {
    let sessions = home.join(".claude/sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join(format!("{pid}.json")),
        json!({ "sessionId": session, "cwd": "/tmp", "status": "idle" }).to_string(),
    )
    .unwrap();
}

/// **The bundle is re-checked on every load and re-sent only when it changed.** #157 fixed the
/// other end of this — a stable name served `immutable` pinned a returning browser to a build the
/// node no longer had — and the fix was `no-store`, which is *never keep this at all*. The four
/// terminal faces are 1.01 MB each since the emoji were cut in (#417), so that was +2.7 MB on
/// every visit rather than on the first, on the surface the operator reads from a phone.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_that_already_has_the_bundle_is_not_sent_it_again() {
    // **Asked of the bundle, not of the shell.** A build with no bundle staged into it — which is
    // every CI job but `single-binary` — answers `index.html` with the *placeholder*, and that is a
    // page the node generates rather than a file it holds: it has no content to hash and it is
    // `no-store` for the same reason the shell is not, because it stops being the right answer the
    // moment a bundle appears. Skipping on a 404 was checking for a reply this path never gives.
    if !kampr_node::assets::has_bundle() {
        eprintln!("skipping: no client bundle staged into this build");
        return;
    }
    let h = harness!("assets");
    let head = response_head(&format!("{}/index.html", h.origin), "").await;
    assert!(head.contains("200"), "{head}");
    assert!(
        head.to_lowercase().contains("cache-control: no-cache"),
        "a stable name must be revalidated, never kept without asking: {head}",
    );
    let tag = head
        .lines()
        .find_map(|line| {
            line.strip_prefix("etag: ")
                .or_else(|| line.strip_prefix("ETag: "))
        })
        .expect("an entity tag")
        .trim()
        .to_string();

    let again = response_head(
        &format!("{}/index.html", h.origin),
        &format!("If-None-Match: {tag}\r\n"),
    )
    .await;
    assert!(
        again.contains("304"),
        "a browser holding the current file was sent the whole thing again: {again}",
    );

    let stale = response_head(
        &format!("{}/index.html", h.origin),
        "If-None-Match: \"0123456789abcdef\"\r\n",
    )
    .await;
    assert!(
        stale.contains("200"),
        "a browser holding an old file was told it was still current: {stale}",
    );
}
