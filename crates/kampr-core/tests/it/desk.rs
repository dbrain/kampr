//! A herdr pane running `kampr attach`: left out of the herd, and told the session's agent, status
//! and name (docs/17 §4).
//!
//! The fake holds `pane.report_agent` to what #576 and #583 measured: a per-source `seq` dropped
//! silently when stale, a `resume_argv` that starts with a path refused `invalid_resume_argv` with
//! nothing applied, 0.9.1 accepting the field and keeping nothing, and a restart that brings the
//! pane back under its id with no report on it.

use kampr_core::desk::{Desk, DeskHosts, DeskReport, Hosting};
use kampr_core::provider::{AgentStatus, Input, Provider};
use kampr_core::{HerdrConfig, HerdrProvider};
use kampr_herdr::Herdr;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

const SHELL: &str = "w1:p1";
const HOST: &str = "w1:p2";
const NATIVE: &str = "n:01knative";

#[derive(Default, Clone)]
struct Shown {
    agent: Option<String>,
    state: Option<String>,
    title: Option<String>,
    resume: Option<Vec<String>>,
    seq: HashMap<&'static str, u64>,
}

struct FakeHerdr {
    version: String,
    panes: Mutex<HashMap<String, Shown>>,
    calls: Mutex<HashMap<String, usize>>,
    _dir: tempfile::TempDir,
    socket: PathBuf,
}

impl FakeHerdr {
    fn start(version: &str) -> Arc<Self> {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let fake = Arc::new(Self {
            version: version.to_string(),
            panes: Mutex::new([SHELL, HOST].map(|p| (p.to_string(), Shown::default())).into()),
            calls: Mutex::default(),
            _dir: dir,
            socket,
        });
        tokio::spawn({
            let fake = fake.clone();
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(fake.clone().serve(stream));
                }
            }
        });
        fake
    }

    fn count(&self, method: &str) -> usize {
        self.calls.lock().unwrap().get(method).copied().unwrap_or(0)
    }

    fn shown(&self, pane: &str) -> Shown {
        self.panes.lock().unwrap().get(pane).cloned().unwrap_or_default()
    }

    /// The pane is restored under its id with nothing reported on it, and the `seq` herdr
    /// remembered is gone with the process (#583).
    fn restart(&self) {
        for shown in self.panes.lock().unwrap().values_mut() {
            *shown = Shown::default();
        }
    }

    fn stale(&self, pane: &str, kind: &'static str, seq: u64) -> bool {
        let mut panes = self.panes.lock().unwrap();
        let shown = panes.entry(pane.to_string()).or_default();
        if shown.seq.get(kind).is_some_and(|last| seq <= *last) {
            return true;
        }
        shown.seq.insert(kind, seq);
        false
    }

    fn report_agent(&self, params: &Value) -> Value {
        let pane = params["pane_id"].as_str().unwrap_or_default().to_string();
        let resume: Option<Vec<String>> = params["resume_argv"].as_array().map(|a| {
            a.iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect()
        });
        if resume
            .as_ref()
            .and_then(|r| r.first())
            .is_some_and(|first| first.contains('/'))
        {
            return json!({ "error": { "code": "invalid_resume_argv",
                "message": "resume_argv must start with a plain command name, not a path" } });
        }
        if self.stale(&pane, "agent", params["seq"].as_u64().unwrap_or(0)) {
            return json!({ "result": { "type": "ok" } });
        }
        let resumes = self.version.as_str() >= "0.9.2";
        let mut panes = self.panes.lock().unwrap();
        let shown = panes.entry(pane).or_default();
        shown.agent = params["agent"].as_str().map(str::to_string);
        shown.state = params["state"].as_str().map(str::to_string);
        if resumes {
            shown.resume = resume;
        }
        json!({ "result": { "type": "ok" } })
    }

    fn report_metadata(&self, params: &Value) -> Value {
        let pane = params["pane_id"].as_str().unwrap_or_default().to_string();
        if !self.stale(&pane, "metadata", params["seq"].as_u64().unwrap_or(0)) {
            self.panes.lock().unwrap().entry(pane).or_default().title =
                params["title"].as_str().map(str::to_string);
        }
        json!({ "result": { "type": "ok" } })
    }

    fn pane(&self, id: &str) -> Value {
        let shown = self.shown(id);
        json!({
            "pane_id": id,
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": "/tmp",
            "label": null,
            "title": shown.title,
            "agent": shown.agent,
            "agent_status": shown.state.as_deref().unwrap_or("unknown"),
            "agent_session": null,
            "scroll": { "offset_from_bottom": 0, "max_offset_from_bottom": 0, "viewport_rows": 40 },
        })
    }

    fn snapshot(&self) -> Value {
        json!({
            "version": self.version,
            "protocol": 22,
            "focused_pane_id": SHELL,
            "workspaces": [{ "workspace_id": "w1", "number": 1, "label": "kampr" }],
            "tabs": [{ "tab_id": "w1:t1", "workspace_id": "w1", "label": "1" }],
            "panes": [self.pane(SHELL), self.pane(HOST)],
            "layouts": [{
                "tab_id": "w1:t1",
                "area": { "x": 0, "y": 0, "width": 120, "height": 40 },
                "panes": [
                    { "pane_id": SHELL, "rect": { "x": 0, "y": 0, "width": 60, "height": 40 } },
                    { "pane_id": HOST, "rect": { "x": 60, "y": 0, "width": 60, "height": 40 } },
                ],
            }],
        })
    }

    async fn serve(self: Arc<Self>, stream: UnixStream) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let method = request["method"].as_str().unwrap_or_default().to_string();
        *self.calls.lock().unwrap().entry(method.clone()).or_default() += 1;
        let params = &request["params"];
        let mut stream = reader.into_inner();
        let reply = match method.as_str() {
            "events.subscribe" => {
                let ack = json!({ "id": "kampr-events", "result": { "type": "subscription_started" } });
                let _ = write_line(&mut stream, &ack).await;
                std::future::pending::<()>().await;
                return;
            }
            "session.snapshot" => json!({ "result": { "snapshot": self.snapshot() } }),
            "pane.process_info" => json!({ "result": { "process_info": {
                "pane_id": params["pane_id"], "shell_pid": 4242, "foreground_process_group_id": 4242,
                "foreground_processes": [{ "pid": 4242, "name": "bash", "argv": ["bash"], "cmdline": "bash" }],
            } } }),
            "pane.get" => {
                json!({ "result": { "pane": self.pane(params["pane_id"].as_str().unwrap_or_default()) } })
            }
            "pane.report_agent" => self.report_agent(params),
            "pane.report_metadata" => self.report_metadata(params),
            "pane.read" => json!({ "result": { "read": { "text": "", "truncated": false } } }),
            other => json!({ "result": { "ok": other } }),
        };
        let mut reply = reply;
        reply["id"] = json!("kampr");
        let _ = write_line(&mut stream, &reply).await;
    }
}

async fn write_line(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    stream.write_all(value.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

struct FakeDesk {
    hosting: Mutex<Vec<Hosting>>,
    report: Mutex<DeskReport>,
    changes: watch::Sender<u64>,
}

impl FakeDesk {
    fn new(socket: &Path) -> Arc<Self> {
        Arc::new(Self {
            hosting: Mutex::new(vec![hosting(socket, 7001)]),
            report: Mutex::new(DeskReport {
                agent: "claude".into(),
                status: AgentStatus::Working,
                title: "review".into(),
                resume_argv: vec![
                    "env".into(),
                    "/opt/kampr".into(),
                    "attach".into(),
                    "01knative".into(),
                ],
            }),
            changes: watch::channel(0).0,
        })
    }

    fn set(&self, f: impl FnOnce(&mut DeskReport)) {
        f(&mut self.report.lock().unwrap());
        self.changes.send_modify(|n| *n += 1);
    }

    fn host(&self, hosting: Vec<Hosting>) {
        *self.hosting.lock().unwrap() = hosting;
        self.changes.send_modify(|n| *n += 1);
    }
}

impl Desk for FakeDesk {
    fn hosting(&self) -> Vec<Hosting> {
        self.hosting.lock().unwrap().clone()
    }

    fn report(&self, native_pane: &str) -> Option<DeskReport> {
        (native_pane == NATIVE).then(|| self.report.lock().unwrap().clone())
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
}

fn hosting(socket: &Path, pid: u32) -> Hosting {
    Hosting {
        socket: socket.to_path_buf(),
        herdr_pane: HOST.into(),
        host_pid: pid,
        native_pane: NATIVE.into(),
    }
}

fn provider(fake: &FakeHerdr, desk: &Arc<FakeDesk>) -> HerdrProvider {
    HerdrProvider::spawn(
        Herdr::new(&fake.socket),
        HerdrConfig {
            sweep: Duration::from_secs(3600),
            sweep_watched: Duration::from_secs(3600),
            settle: Duration::from_millis(5),
            desk: Some(DeskHosts(desk.clone() as Arc<dyn Desk>)),
            ..HerdrConfig::default()
        },
    )
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..200 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} never happened");
}

async fn listed(provider: &HerdrProvider) -> Vec<String> {
    provider.refresh().await.expect("the fake answers");
    let mut ids: Vec<String> = provider
        .list_panes()
        .await
        .expect("panes")
        .into_iter()
        .map(|p| p.pane_id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn a_herdr_pane_showing_a_native_session_is_not_a_pane_of_its_own() {
    let fake = FakeHerdr::start("0.9.5");
    let link_dir = tempfile::tempdir().expect("tempdir");
    let link = link_dir.path().join("linked.sock");
    std::os::unix::fs::symlink(&fake.socket, &link).expect("symlink");
    let elsewhere = link_dir.path().join("another-herdr.sock");
    for (socket, hidden) in [(&fake.socket, true), (&link, true), (&elsewhere, false)] {
        let desk = FakeDesk::new(socket);
        let provider = provider(&fake, &desk);
        let expected: Vec<String> = match hidden {
            true => vec![SHELL.into()],
            false => vec![SHELL.into(), HOST.into()],
        };
        assert_eq!(
            listed(&provider).await,
            expected,
            "host socket {}",
            socket.display()
        );
        assert_eq!(provider.hosting(HOST).is_some(), hidden);
        if hidden {
            assert!(
                provider.watch_pane(HOST).await.is_err(),
                "streamed as a pane of its own"
            );
            assert!(
                provider
                    .write_pane(HOST, Input::Bytes(b"x".to_vec()))
                    .await
                    .is_err(),
                "typed into as a pane of its own"
            );
            assert!(provider.read_scrollback(HOST).await.is_err());
        }
    }
}

#[tokio::test]
async fn the_host_pane_is_told_the_sessions_agent_status_and_name() {
    for (version, resumes) in [("0.9.5", true), ("0.9.2", true), ("0.9.1", false)] {
        let fake = FakeHerdr::start(version);
        let desk = FakeDesk::new(&fake.socket);
        let provider = provider(&fake, &desk);
        listed(&provider).await;
        let shown = fake.shown(HOST);
        assert_eq!(shown.agent.as_deref(), Some("claude"), "{version}");
        assert_eq!(shown.state.as_deref(), Some("working"), "{version}");
        assert_eq!(shown.title.as_deref(), Some("review"), "{version}");
        assert_eq!(
            shown.resume.is_some(),
            resumes,
            "{version}: resume_argv is what re-hosts the session after a restart (#583)"
        );
        assert_eq!(fake.shown(SHELL).agent, None, "a pane nobody hosts is left alone");
    }
}

#[tokio::test]
async fn a_status_change_reaches_the_desk_and_an_unchanged_one_is_not_resent() {
    let fake = FakeHerdr::start("0.9.5");
    let desk = FakeDesk::new(&fake.socket);
    let provider = provider(&fake, &desk);
    listed(&provider).await;
    let sent = fake.count("pane.report_agent");
    listed(&provider).await;
    assert_eq!(
        fake.count("pane.report_agent"),
        sent,
        "nothing moved, nothing to say"
    );

    desk.set(|r| r.status = AgentStatus::Blocked);
    until("the desk shows blocked", || {
        fake.shown(HOST).state.as_deref() == Some("blocked")
    })
    .await;
    desk.set(|r| r.title = "renamed".into());
    until("the desk shows the new name", || {
        fake.shown(HOST).title.as_deref() == Some("renamed")
    })
    .await;
    desk.set(|r| r.status = AgentStatus::Done);
    until("done is told as idle", || {
        fake.shown(HOST).state.as_deref() == Some("idle")
    })
    .await;
}

#[tokio::test]
async fn a_herdr_restart_brings_the_pane_back_bare_and_the_new_host_reports_it_again() {
    let fake = FakeHerdr::start("0.9.5");
    let desk = FakeDesk::new(&fake.socket);
    let provider = provider(&fake, &desk);
    listed(&provider).await;
    assert_eq!(fake.shown(HOST).agent.as_deref(), Some("claude"));

    fake.restart();
    desk.host(vec![hosting(&fake.socket, 7002)]);
    until("the restored pane is reported again", || {
        fake.shown(HOST).agent.as_deref() == Some("claude")
    })
    .await;
    assert!(fake.shown(HOST).resume.is_some());
}
