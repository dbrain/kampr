//! Names, from the pane's foreground job to the wire and back into herdr's own metadata table.
//!
//! The fake here is not a stub that says yes. It implements what probe #295 *measured*
//! `pane.report_metadata` to do — a record per source, last writer wins across them, a per-source
//! monotonic `seq`, and a stale report dropped **silently while still answering `ok`** — because a
//! fake that acknowledges everything would let a reporter that trusts `ok` pass every test here.
//!
//! `agent.view.*` is the same fake held to the same standard, from the probe that measured it:
//! `set` answers `{active, source, label}` and **never says what it sorted on**, `clear` takes no
//! source and wipes whatever is active whoever set it, and there is no `agent.view.get` at all.

use kampr_core::agent_view::{DeskAgents, View};
use kampr_core::naming::Template;
use kampr_core::provider::Provider;
use kampr_core::reporter::{Reported, Reporter};
use kampr_core::{HerdrConfig, HerdrProvider};
use kampr_herdr::Herdr;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Default)]
struct Metadata {
    /// The record each source holds for a pane, and the source whose write was most recent.
    records: HashMap<(String, String), String>,
    showing: HashMap<String, String>,
    seq: HashMap<(String, String), u64>,
    /// Tokens **merge** across sources into one map per pane rather than arbitrating like `title`
    /// does, and they read back on `pane.get`.
    tokens: HashMap<String, HashMap<String, String>>,
}

/// The whole of what herdr holds for an agents view — and the whole of what a client can never
/// read back out of it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentView {
    source: String,
    sort: Value,
    label: Option<String>,
}

struct FakeHerdr {
    calls: Mutex<HashMap<String, usize>>,
    /// What herdr's screen-scrape calls the pane, which is what gates harness resolution today.
    agent: Mutex<Option<String>>,
    metadata: Mutex<Metadata>,
    /// `foreground_processes`, exactly as herdr shapes them, plus the two pids that say whether
    /// the shell has a job at all (probe #297).
    processes: Mutex<Value>,
    view: Mutex<Option<AgentView>>,
    views_set: Mutex<Vec<AgentView>>,
    _dir: tempfile::TempDir,
    socket: std::path::PathBuf,
}

impl FakeHerdr {
    fn start() -> Arc<Self> {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let fake = Arc::new(Self {
            calls: Mutex::default(),
            agent: Mutex::default(),
            metadata: Mutex::default(),
            processes: Mutex::new(idle_shell()),
            view: Mutex::default(),
            views_set: Mutex::default(),
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

    fn herdr(&self) -> Herdr {
        Herdr::new(&self.socket)
    }

    fn count(&self, method: &str) -> usize {
        self.calls.lock().unwrap().get(method).copied().unwrap_or(0)
    }

    fn running(&self, processes: Value) {
        *self.processes.lock().unwrap() = processes;
    }

    fn scraped(&self, agent: &str) {
        *self.agent.lock().unwrap() = Some(agent.to_string());
    }

    fn snapshot(&self) -> Value {
        let mut snapshot = snapshot();
        snapshot["panes"][0]["agent"] = match self.agent.lock().unwrap().clone() {
            Some(agent) => json!(agent),
            None => Value::Null,
        };
        snapshot
    }

    fn showing(&self, pane: &str) -> Option<String> {
        self.metadata.lock().unwrap().showing.get(pane).cloned()
    }

    fn token(&self, pane: &str, token: &str) -> Option<String> {
        self.metadata
            .lock()
            .unwrap()
            .tokens
            .get(pane)
            .and_then(|t| t.get(token))
            .cloned()
    }

    fn view(&self) -> Option<AgentView> {
        self.view.lock().unwrap().clone()
    }

    fn views_set(&self) -> Vec<AgentView> {
        self.views_set.lock().unwrap().clone()
    }

    /// Somebody at that desk sorted their own sidebar, which `agent.view.clear` does not respect.
    fn view_of_their_own(&self) {
        *self.view.lock().unwrap() = Some(AgentView {
            source: "theirs".into(),
            sort: Value::Null,
            label: Some("grouped".into()),
        });
    }

    fn set_view(&self, params: &Value) -> Value {
        let view = AgentView {
            source: params["source"].as_str().unwrap_or_default().to_string(),
            sort: params["sort"].clone(),
            label: params["label"].as_str().map(str::to_string),
        };
        self.views_set.lock().unwrap().push(view.clone());
        let reply = json!({
            "type": "agent_view",
            "active": true,
            "source": view.source,
            "label": view.label,
        });
        *self.view.lock().unwrap() = Some(view);
        reply
    }

    /// No source, and no respect for the one that set the view: whatever is active goes.
    fn clear_view(&self) -> Value {
        *self.view.lock().unwrap() = None;
        json!({ "type": "agent_view", "active": false })
    }

    /// Another reporter got there first, at a `seq` this one cannot beat.
    fn already_reported(&self, pane: &str, source: &str, title: &str, seq: u64) {
        let mut metadata = self.metadata.lock().unwrap();
        metadata
            .records
            .insert((pane.into(), source.into()), title.into());
        metadata.seq.insert((pane.into(), source.into()), seq);
        metadata.showing.insert(pane.into(), title.into());
    }

    fn report(&self, params: &Value) -> Value {
        let pane = params["pane_id"].as_str().unwrap_or_default().to_string();
        let source = params["source"].as_str().unwrap_or_default().to_string();
        let title = params["title"].as_str().unwrap_or_default().to_string();
        let mut metadata = self.metadata.lock().unwrap();
        let key = (pane.clone(), source.clone());
        if let Some(seq) = params["seq"].as_u64() {
            // Probe #295: a stale seq changes nothing and is still answered `ok`.
            if metadata.seq.get(&key).is_some_and(|last| seq <= *last) {
                return json!({ "type": "ok" });
            }
            metadata.seq.insert(key.clone(), seq);
        }
        if let Some(tokens) = params["tokens"].as_object() {
            let merged = metadata.tokens.entry(pane.clone()).or_default();
            for (name, value) in tokens {
                merged.insert(name.clone(), value.as_str().unwrap_or_default().to_string());
            }
        }
        metadata.records.insert(key, title.clone());
        metadata.showing.insert(pane, title);
        json!({ "type": "ok" })
    }

    fn pane(&self, params: &Value) -> Value {
        let pane_id = params["pane_id"].as_str().unwrap_or_default();
        let mut pane = self.snapshot()["panes"][0].clone();
        pane["title"] = match self.showing(pane_id) {
            Some(title) => json!(title),
            None => Value::Null,
        };
        json!({ "pane": pane })
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
        let mut stream = reader.into_inner();
        if method == "events.subscribe" {
            let ack = json!({ "id": "kampr-events", "result": { "type": "subscription_started" } });
            let _ = write_line(&mut stream, &ack).await;
            // Held open, so the provider's topology loop parks here rather than reconnecting.
            std::future::pending::<()>().await;
            return;
        }
        let result = match method.as_str() {
            "session.snapshot" => json!({ "snapshot": self.snapshot() }),
            "pane.process_info" => json!({ "process_info": self.processes.lock().unwrap().clone() }),
            "pane.get" => self.pane(&request["params"]),
            "pane.report_metadata" => self.report(&request["params"]),
            "agent.view.set" => self.set_view(&request["params"]),
            "agent.view.clear" => self.clear_view(),
            "pane.read" => json!({ "read": { "text": "", "truncated": false } }),
            other => json!({ "ok": other }),
        };
        let _ = write_line(&mut stream, &json!({ "id": "kampr", "result": result })).await;
    }
}

async fn write_line(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    stream.write_all(value.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

fn idle_shell() -> Value {
    json!({
        "pane_id": "w1:p1",
        "shell_pid": 4242,
        "foreground_process_group_id": 4242,
        "foreground_processes": [{ "pid": 4242, "name": "bash", "argv": ["bash"], "cmdline": "bash" }],
    })
}

/// A plain shell with a job in it, which is what herdr answers off a machine with no ble.sh.
fn running(name: &str, cmdline: &str) -> Value {
    json!({
        "pane_id": "w1:p1",
        "shell_pid": 4242,
        "foreground_process_group_id": 5150,
        "foreground_processes": [
            { "pid": 5150, "name": name, "argv": [name], "cmdline": cmdline },
        ],
    })
}

/// Probe #297: ble.sh keeps the job in the shell's own process group, so herdr names `bash` while
/// `cargo test` is running. The shell pid here is nobody, so the procfs walk that answers this
/// case for real has nothing to find and the pane degrades — which is what a host with no procfs
/// does too.
fn ble_sh_running() -> Value {
    json!({
        "pane_id": "w1:p1",
        "shell_pid": 4242,
        "foreground_process_group_id": 4242,
        "foreground_processes": [{ "pid": 4242, "name": "bash", "argv": ["bash"], "cmdline": "bash" }],
    })
}

fn snapshot() -> Value {
    json!({
        "version": "0.8.2",
        "protocol": 20,
        "focused_pane_id": "w1:p1",
        "workspaces": [{ "workspace_id": "w1", "number": 1, "label": "kampr" }],
        "tabs": [{ "tab_id": "w1:t1", "workspace_id": "w1", "label": "1" }],
        "panes": [{
            "pane_id": "w1:p1",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": "/home/dbrain/dev/kampr",
            "label": null,
            "agent": null,
            "agent_status": "unknown",
            "agent_session": null,
            "scroll": { "offset_from_bottom": 0, "max_offset_from_bottom": 12, "viewport_rows": 40 },
        }],
        "layouts": [{
            "tab_id": "w1:t1",
            "area": { "x": 0, "y": 0, "width": 94, "height": 40 },
            "panes": [{ "pane_id": "w1:p1", "rect": { "x": 0, "y": 0, "width": 94, "height": 40 } }],
        }],
    })
}

fn config() -> HerdrConfig {
    HerdrConfig {
        sweep: Duration::from_secs(3600),
        sweep_watched: Duration::from_secs(3600),
        settle: Duration::from_millis(5),
        ..HerdrConfig::default()
    }
}

async fn only_pane(provider: &HerdrProvider) -> kampr_core::PaneInfo {
    provider
        .refresh()
        .await
        .expect("the fake answers session.snapshot");
    provider
        .list_panes()
        .await
        .expect("panes")
        .into_iter()
        .next()
        .expect("the fake serves one pane")
}

#[tokio::test]
async fn a_pane_running_a_job_carries_it_and_a_pane_at_its_prompt_carries_nothing() {
    let fake = FakeHerdr::start();
    // This one is about what the whole line says, so it asks for the whole line.
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            send_argv: true,
            ..config()
        },
    );

    let quiet = only_pane(&provider).await;
    assert_eq!(quiet.cmd, None, "an idle shell is not a job");
    assert_eq!(quiet.argv, None);

    fake.running(running("cargo", "cargo test"));
    let busy = only_pane(&provider).await;
    assert_eq!(busy.cmd.as_deref(), Some("cargo"));
    assert_eq!(busy.argv.as_deref(), Some("cargo test"));

    fake.running(ble_sh_running());
    let hidden = only_pane(&provider).await;
    assert_eq!(
        hidden.cmd, None,
        "herdr claims nothing and pid 4242 is nobody, so nothing may be claimed"
    );
    assert_eq!(
        Template::default().render(&kampr_core::naming::Fields::from_info(&hidden)),
        "kampr · bash",
        "and the name degrades to the shell rather than naming a job that is not there"
    );
}

/// **A command line is a credential-bearing string, and a `readonly` device is handed every one
/// of them without watching a pane.**
///
/// `mysql -p<password>`, `curl -H "Authorization: …"`, `ssh -o ProxyCommand=…`. The screen a
/// readonly device can already stream is not the same disclosure: the herd model arrives at
/// `hello` and is patched forever with no `watch` at all, and an alt-screen or cleared pane shows
/// nothing on screen while `argv` names the job for its whole life. So the full line is off by
/// default and `cmd` — the process name, which is all the naming complaint ever needed — is not.
///
/// The default template names the job with `{argv|cmd|agent|'bash'}`, so this costs no client
/// release and no template change: the slot falls through to `cmd` exactly as it already falls
/// through to the agent under ble.sh (#297).
#[tokio::test]
async fn the_whole_command_line_is_not_handed_to_every_device_that_pairs() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    fake.running(running("mysql", "mysql -h db -u root -phunter2"));

    let pane = only_pane(&provider).await;
    assert_eq!(
        pane.cmd.as_deref(),
        Some("mysql"),
        "the process name is what tells six panes in one directory apart"
    );
    assert_eq!(
        pane.argv, None,
        "the arguments are where the secrets are, and nothing asked for them"
    );
    assert_eq!(
        Template::default().render(&kampr_core::naming::Fields::from_info(&pane)),
        "kampr · mysql",
        "the name still says what the pane is doing"
    );

    let telling = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            send_argv: true,
            ..config()
        },
    );
    let pane = only_pane(&telling).await;
    assert_eq!(
        pane.argv.as_deref(),
        Some("mysql -h db -u root -phunter2"),
        "an operator who turned it on gets it"
    );
}

/// Nothing in herdr's snapshot moves when a pane starts a build — same workspace, same tab, same
/// cwd, same geometry — so a herd derived from the snapshot alone would carry a name that never
/// changed. The command has to move the revision itself or the whole feature is a value nobody
/// re-reads.
#[tokio::test]
async fn a_pane_that_starts_a_job_moves_the_herd_that_nothing_else_would_have_moved() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    only_pane(&provider).await;
    let mut topology = provider.topology();
    topology.mark_unchanged();

    fake.running(running("cargo", "cargo test"));
    only_pane(&provider).await;
    assert!(topology.has_changed().expect("the provider is alive"));
}

/// A pipeline reports every member (probe #297), and a name that showed only the first of them
/// would call `sleep 9 | cat` a `sleep`.
#[tokio::test]
async fn a_pipeline_is_named_the_way_the_shell_wrote_it() {
    let fake = FakeHerdr::start();
    fake.running(json!({
        "pane_id": "w1:p1",
        "shell_pid": 4242,
        "foreground_process_group_id": 5150,
        "foreground_processes": [
            { "pid": 5150, "name": "sleep", "argv": ["sleep", "9"], "cmdline": "sleep 9" },
            { "pid": 5151, "name": "cat", "argv": ["cat"], "cmdline": "cat" },
        ],
    }));
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            send_argv: true,
            ..config()
        },
    );
    let pane = only_pane(&provider).await;
    assert_eq!(pane.argv.as_deref(), Some("sleep 9 | cat"));
    assert_eq!(pane.cmd.as_deref(), Some("sleep"));
}

/// ADR 0002's invariant, and the whole reason this half is a setting: looking at somebody's herd
/// must not write into it.
#[tokio::test]
async fn a_node_nobody_asked_writes_no_name_into_anybodys_herdr() {
    let fake = FakeHerdr::start();
    fake.running(running("cargo", "cargo test"));
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    assert_eq!(
        config().report_names,
        None,
        "the shipped default is not to report at all"
    );
    for _ in 0..3 {
        only_pane(&provider).await;
    }
    assert_eq!(fake.count("pane.report_metadata"), 0);
    assert_eq!(fake.showing("w1:p1"), None);
}

#[tokio::test]
async fn the_name_kampr_computes_reaches_the_pane_border_at_the_desk() {
    let fake = FakeHerdr::start();
    fake.running(running("cargo", "cargo test"));
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            report_names: Some(Template::default()),
            ..config()
        },
    );
    only_pane(&provider).await;
    assert_eq!(fake.showing("w1:p1").as_deref(), Some("kampr · cargo"));

    // A name that has not moved is not re-sent: herdr is a shared table and every write is a
    // chance to stamp on somebody else's record.
    let after_first = fake.count("pane.report_metadata");
    only_pane(&provider).await;
    assert_eq!(fake.count("pane.report_metadata"), after_first);

    fake.running(running("vim", "vim src/naming.rs"));
    only_pane(&provider).await;
    assert_eq!(
        fake.showing("w1:p1").as_deref(),
        Some("kampr · vim"),
        "the process name, because the arguments are off by default"
    );
}

/// Probe #295. herdr answers `ok` to a report it dropped, so a reporter that reads the ack is a
/// reporter that believes a name it never set.
#[tokio::test]
async fn a_report_herdr_answered_ok_and_silently_dropped_is_not_reported_as_applied() {
    let fake = FakeHerdr::start();
    fake.already_reported("w1:p1", kampr_core::reporter::SOURCE, "somebody else's", u64::MAX);
    let reporter = Reporter::new();
    let outcome = reporter
        .report(&fake.herdr(), "w1:p1", "kampr (cargo test) · bash")
        .await
        .expect("the call itself succeeds — that is the point");
    assert_eq!(outcome, Reported::NotApplied);
    assert_eq!(fake.showing("w1:p1").as_deref(), Some("somebody else's"));
}

/// Probe #295 again, from the consequence rather than the mechanism. A report that did not land
/// must not be sent again on the next sweep and the one after: two sources overwriting each other
/// for ever is a worse outcome than a name that did not take.
#[tokio::test]
async fn a_pane_kampr_did_not_win_is_not_stamped_on_again_every_sweep() {
    let fake = FakeHerdr::start();
    fake.already_reported("w1:p1", kampr_core::reporter::SOURCE, "theirs", u64::MAX);
    fake.running(running("cargo", "cargo test"));
    let herdr = fake.herdr();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    let reporter = Reporter::new();
    let template = Template::default();

    let panes = vec![only_pane(&provider).await];
    reporter.sweep(&herdr, &template, &panes).await;
    assert_eq!(fake.count("pane.report_metadata"), 1);
    assert_eq!(fake.showing("w1:p1").as_deref(), Some("theirs"));

    reporter.sweep(&herdr, &template, &panes).await;
    reporter.sweep(&herdr, &template, &panes).await;
    assert_eq!(
        fake.count("pane.report_metadata"),
        1,
        "the name has not changed, so there is nothing new to say"
    );
}

/// herdr remembers the last `seq` a source sent for as long as the pane lives; a restarted node
/// does not. A counter that starts at one is a whole herd of names silently dropped and every one
/// of them answered `ok`.
#[tokio::test]
async fn a_node_that_restarts_does_not_lose_every_pane_to_its_own_stale_seq() {
    let fake = FakeHerdr::start();
    let before = Reporter::new();
    before
        .report(&fake.herdr(), "w1:p1", "kampr · bash")
        .await
        .expect("reports");

    let after_restart = Reporter::new();
    assert_eq!(
        after_restart
            .report(&fake.herdr(), "w1:p1", "kampr (cargo test) · bash")
            .await
            .expect("reports"),
        Reported::Applied
    );
    assert_eq!(
        fake.showing("w1:p1").as_deref(),
        Some("kampr (cargo test) · bash")
    );
}

/// The name has to reach herdr as a **token**, not only as a title, or there is nothing for
/// [`kampr_core::agent_view`] to sort on: herdr's sortable builtins are `agent` and `status` and
/// nothing else, and `title` is not among them.
#[tokio::test]
async fn the_name_reaches_herdr_as_the_token_a_sidebar_can_be_sorted_on() {
    let fake = FakeHerdr::start();
    fake.running(running("cargo", "cargo test"));
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            report_names: Some(Template::default()),
            ..config()
        },
    );
    only_pane(&provider).await;
    assert_eq!(
        fake.token("w1:p1", kampr_core::reporter::TOKEN).as_deref(),
        Some("kampr · cargo")
    );
}

/// ADR 0002 for the sidebar rather than the pane border. A node nobody asked writes no view, and
/// — because `agent.view.clear` carries no source and takes down whatever is active — it must not
/// clear one either.
#[tokio::test]
async fn a_node_nobody_asked_leaves_the_desks_own_agent_order_alone() {
    let fake = FakeHerdr::start();
    fake.view_of_their_own();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    assert_eq!(
        config().desk_agents,
        None,
        "the shipped default is to leave the desk alone"
    );
    for _ in 0..3 {
        only_pane(&provider).await;
    }
    provider.restore_desk().await;
    assert_eq!(fake.count("agent.view.set"), 0);
    assert_eq!(
        fake.count("agent.view.clear"),
        0,
        "clearing is unscoped, so a node that set nothing must clear nothing"
    );
    assert_eq!(
        fake.view().map(|v| v.source),
        Some("theirs".into()),
        "and the view they set for themselves is still theirs"
    );
}

#[tokio::test]
async fn the_desk_is_sorted_by_kamprs_name_and_is_not_told_twice() {
    let fake = FakeHerdr::start();
    fake.running(running("cargo", "cargo test"));
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            report_names: Some(Template::default()),
            desk_agents: Some(View::by_name()),
            ..config()
        },
    );
    for _ in 0..3 {
        only_pane(&provider).await;
    }
    assert_eq!(
        fake.views_set().len(),
        1,
        "the view has not moved, so there is nothing to say again"
    );
    let sent = fake.view().expect("a view is active");
    assert_eq!(sent.source, kampr_core::reporter::SOURCE);
    assert_eq!(sent.label.as_deref(), Some(kampr_core::agent_view::LABEL));
    assert_eq!(
        sent.sort,
        json!([{ "field": { "token": kampr_core::reporter::TOKEN }, "order": "asc" }])
    );
}

/// The setting turned off and a clean shutdown are the same thing to the desk, and they are the
/// same path here. A sort that outlives the node that set it cannot be cleared by anything, since
/// herdr will not say what view it is holding.
#[tokio::test]
async fn a_desk_kampr_sorted_gets_its_own_order_back() {
    let fake = FakeHerdr::start();
    let herdr = fake.herdr();
    let desk = DeskAgents::new();
    let view = View::by_name();

    desk.sweep(&herdr, Some(&view)).await;
    assert!(fake.view().is_some());

    desk.restore(&herdr).await;
    assert_eq!(fake.view(), None);
    assert_eq!(fake.count("agent.view.clear"), 1);

    desk.restore(&herdr).await;
    assert_eq!(
        fake.count("agent.view.clear"),
        1,
        "and a desk already put back is not put back again every sweep"
    );
}

#[tokio::test]
async fn a_node_shutting_down_puts_the_desk_back() {
    let fake = FakeHerdr::start();
    fake.running(running("cargo", "cargo test"));
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            report_names: Some(Template::default()),
            desk_agents: Some(View::by_name()),
            ..config()
        },
    );
    only_pane(&provider).await;
    assert!(fake.view().is_some());

    provider.restore_desk().await;
    assert_eq!(fake.view(), None);
}

// ---------------------------------------------------------------------------------------------
// W8/W3: what the node can see about a pane by looking at its own machine.
//
// These drive the **real** procfs of the process running them. A synthetic tree would prove the
// parser and nothing about the kernel, and the two facts that matter here — that a shell's job is
// reachable through `/proc/<pid>/task/*/children`, and that a job which has exited stays in that
// file as a zombie — are facts about the kernel.

/// A real process tree, rooted at a process standing in for the pane's shell.
///
/// The root `exec`s so that it never reaps: a child killed under it stays a zombie in its
/// `children` file, which is the staleness these tests exist to catch.
struct Tree(std::process::Child);

impl Tree {
    fn spawn(script: &str) -> Self {
        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{script} exec sleep 900"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("this machine can run sh");
        let tree = Self(child);
        tree.settle();
        tree
    }

    fn shell(&self) -> u32 {
        self.0.id()
    }

    /// Waits for the script's own children to exist, because a walk run before `sh` has forked
    /// would measure nothing and pass for the wrong reason.
    fn settle(&self) {
        for _ in 0..200 {
            if !children_of(self.shell()).is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the tree never started");
    }

    fn children(&self) -> Vec<u32> {
        children_of(self.shell())
    }

    /// herdr's answer on a machine that sources ble.sh: the shell, however busy the pane is
    /// (probe #297).
    fn as_ble_sh_reports_it(&self) -> Value {
        json!({
            "pane_id": "w1:p1",
            "shell_pid": self.shell(),
            "foreground_process_group_id": self.shell(),
            "foreground_processes": [
                { "pid": self.shell(), "name": "bash", "argv": ["bash"], "cmdline": "bash" },
            ],
        })
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // Depth-first, because a wrapper shell's own job is a grandchild and an orphan left
        // holding this test binary's stdout keeps `cargo test` from ever finishing.
        fn reap(pid: u32) {
            for child in children_of(pid) {
                reap(child);
            }
            kill(pid);
        }
        for child in children_of(self.shell()) {
            reap(child);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn children_of(pid: u32) -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return pids;
    };
    for task in tasks.flatten() {
        if let Ok(text) = std::fs::read_to_string(task.path().join("children")) {
            pids.extend(
                text.split_ascii_whitespace()
                    .filter_map(|p| p.parse::<u32>().ok()),
            );
        }
    }
    pids
}

fn kill(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status();
}

fn procfs_readable() -> bool {
    std::path::Path::new("/proc/self/task").is_dir()
}

fn walking_config() -> HerdrConfig {
    HerdrConfig {
        send_argv: true,
        ..config()
    }
}

/// **The defect W8 names.** `ProcessInfo::command` answers `None` whenever the foreground process
/// group is the shell's, which is what ble.sh does to every interactive shell on the operator's
/// machine (probe #297) — so `cmd` and `argv` are blank on precisely the panes worth naming, and
/// the sidebar says `kampr · bash` for a pane running a build. The node is on that machine as
/// that user, and `/proc/<shell>/task/*/children` names the job outright.
#[tokio::test]
async fn a_job_ble_sh_hides_from_herdr_is_still_what_the_sidebar_says_the_pane_is_running() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sleep 300 &");
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    assert_eq!(
        pane.cmd.as_deref(),
        Some("sleep"),
        "herdr answered the shell; the machine itself answers the job"
    );
    assert_eq!(pane.argv.as_deref(), Some("sleep 300"));
    assert_eq!(
        Template::default().render(&kampr_core::naming::Fields::from_info(&pane)),
        "kampr · sleep 300",
    );
}

/// **The failure to guard is staleness, not absence.** A child that has exited stays in its
/// parent's `children` file until it is reaped, so a walk that trusts the file names a job that is
/// gone — the sidebar saying `kampr · sleep 300` for a pane sitting at its prompt. Naming nothing
/// is the honest answer and the template already renders it.
#[tokio::test]
async fn a_job_that_has_already_exited_is_not_named_as_though_it_were_still_running() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sleep 300 &");
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());
    assert_eq!(only_pane(&provider).await.cmd.as_deref(), Some("sleep"));

    let job = *tree.children().first().expect("the tree has a job");
    kill(job);
    for _ in 0..200 {
        if std::fs::read(format!("/proc/{job}/cmdline")).is_ok_and(|line| line.is_empty()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        tree.children().contains(&job),
        "the whole point: the kernel still lists the dead child, so the file cannot be trusted"
    );

    let pane = only_pane(&provider).await;
    assert_eq!(pane.cmd, None, "a job that exited is not a job");
    assert_eq!(pane.argv, None);
    assert_eq!(
        Template::default().render(&kampr_core::naming::Fields::from_info(&pane)),
        "kampr · bash",
        "and the pane falls back to the shell rather than keeping a name that has expired"
    );
}

/// A pane genuinely sitting at its prompt has to go on reporting nothing.
///
/// **The shell here is `zsh` and that is the whole design of the fixture.** The decision is made on
/// pids — `foreground_process_group_id == shell_pid` is what says there is no job (#297) — so the
/// name is free, and naming it `bash` would make a walk that reported the shell as its own job
/// render `kampr · bash`, which is byte-identical to the right answer under the default template.
/// Any shell but the one the template falls back to keeps the rendered name discriminating.
#[tokio::test]
async fn a_pane_sitting_at_its_prompt_still_names_no_job_at_all() {
    if !procfs_readable() {
        return;
    }
    let idle = std::process::Command::new("sleep")
        .arg("900")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("this machine can run sleep");
    let fake = FakeHerdr::start();
    fake.running(json!({
        "pane_id": "w1:p1",
        "shell_pid": idle.id(),
        "foreground_process_group_id": idle.id(),
        "foreground_processes": [
            { "pid": idle.id(), "name": "zsh", "argv": ["zsh"], "cmdline": "zsh" },
        ],
    }));
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    assert_eq!(pane.cmd, None, "there is nothing under this shell to name");
    assert_eq!(
        Template::default().render(&kampr_core::naming::Fields::from_info(&pane)),
        "kampr · bash",
        "the fall-back literal, and never the shell the walk was looking at",
    );
    kill(idle.id());
    let _ = { idle }.wait();
}

/// herdr stays the source of truth where it has one. The walk is a fallback for the case herdr's
/// process-group check gives up on, not a second opinion about a pane herdr has already answered
/// for — and a pane whose job is a wrapper would otherwise be renamed after whatever the wrapper
/// spawned.
#[tokio::test]
async fn herdrs_own_answer_wins_over_the_walk_wherever_herdr_has_one() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sleep 300 &");
    let fake = FakeHerdr::start();
    fake.running(json!({
        "pane_id": "w1:p1",
        "shell_pid": tree.shell(),
        "foreground_process_group_id": 5150,
        "foreground_processes": [
            { "pid": 5150, "name": "cargo", "argv": ["cargo"], "cmdline": "cargo test" },
        ],
    }));
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    assert_eq!(pane.cmd.as_deref(), Some("cargo"));
    assert_eq!(pane.argv.as_deref(), Some("cargo test"));
}

/// A script is a shell with a job under it, and stopping at the shell would name the pane `bash`
/// — a shell name, which is the one answer the template cannot render.
#[tokio::test]
async fn a_job_run_through_a_wrapper_shell_is_named_after_the_job_and_not_the_wrapper() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sh -c 'sleep 300; true' &");
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    assert_eq!(pane.cmd.as_deref(), Some("sleep"));
}

/// A pipeline names every member (probe #297), and under ble.sh every member is a child of the
/// shell. Naming only the first of them would call `sleep 300 | sleep 301` a `sleep`.
#[tokio::test]
async fn a_pipeline_hidden_by_ble_sh_is_named_the_way_the_shell_wrote_it() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sleep 300 & sleep 301 &");
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    assert_eq!(pane.argv.as_deref(), Some("sleep 300 | sleep 301"));
}

/// **W3.** `ProcessInfo::harness` matches by *name* over what herdr reported, and under ble.sh
/// herdr reports only `bash` — so a pane herdr has screen-scraped as a `claude` pane resolves to
/// no harness at all, which reads as `Absent` and refuses the conversation outright. The pid is
/// under the shell where it always was.
#[tokio::test]
async fn a_harness_ble_sh_hides_from_herdr_is_still_the_process_the_pane_is_having_its_session_in() {
    if !procfs_readable() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let harness = dir.path().join("claude");
    std::fs::copy("/usr/bin/sleep", &harness).expect("this machine has sleep");
    let tree = Tree::spawn(&format!("{} 300 &", harness.display()));
    let fake = FakeHerdr::start();
    fake.scraped("claude");
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    let pid = *tree.children().first().expect("the tree has a job");
    assert_eq!(
        pane.agent_harness.process().map(|p| p.pid),
        Some(pid),
        "the harness is the process under the shell, whatever herdr could see of it"
    );
}

/// **W3's seam.** Which session a pane is having is answered by intersecting the pane's pids with
/// the harness's own pid-keyed marker directory, and that intersection needs *every* foreground
/// pid rather than the one that happened to match a name. It has to be there for a pane herdr has
/// not called an agent at all, because that is the gap: the marker exists from the moment the
/// agent opens, minutes before herdr's screen-scrape or any transcript.
#[tokio::test]
async fn every_pid_in_a_pane_is_offered_up_even_where_herdr_has_not_called_it_an_agent() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sleep 300 &");
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());
    let pane = only_pane(&provider).await;
    assert_eq!(pane.agent, None, "herdr has scraped nothing out of this pane");

    let pids: Vec<u32> = provider
        .pane_processes("w1:p1")
        .into_iter()
        .map(|p| p.pid)
        .collect();
    let job = *tree.children().first().expect("the tree has a job");
    assert!(
        pids.contains(&job),
        "the job under the shell is a candidate: {pids:?}"
    );
    assert!(
        pids.contains(&tree.shell()),
        "and so is the shell herdr named: {pids:?}"
    );
}

/// The set is re-walked, never held: a pid that has gone must not go on being offered to a
/// marker directory that could hand back the session of whoever inherits it.
#[tokio::test]
async fn a_pid_that_has_gone_stops_being_offered_on_the_very_next_sweep() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn("sleep 300 &");
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());
    only_pane(&provider).await;
    let job = *tree.children().first().expect("the tree has a job");
    assert!(provider.pane_processes("w1:p1").iter().any(|p| p.pid == job));

    kill(job);
    for _ in 0..200 {
        if std::fs::read(format!("/proc/{job}/cmdline")).is_ok_and(|line| line.is_empty()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    only_pane(&provider).await;
    assert!(
        !provider.pane_processes("w1:p1").iter().any(|p| p.pid == job),
        "the walk is re-run on the sweep and the dead child is dropped"
    );
}

/// A host with no procfs degrades to exactly what it did before any of this, rather than refusing
/// to serve the pane: the shell pid names nothing, and the pane keeps herdr's answer.
#[tokio::test]
async fn a_pane_whose_shell_cannot_be_looked_up_is_served_exactly_as_it_was_before() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    assert_eq!(pane.cmd, None, "pid 4242 is nobody on this machine");
    assert_eq!(
        Template::default().render(&kampr_core::naming::Fields::from_info(&pane)),
        "kampr · bash",
    );
}

/// The walk reaches command lines herdr's process-group check used to hide, and a harness
/// launched with a brief in its arguments has a command line kilobytes long — which the default
/// template renders straight into the pane's name, sends to every device in a herd patch, and
/// writes back into herdr's own pane title.
#[tokio::test]
async fn a_command_line_long_enough_to_be_a_document_does_not_become_the_panes_name() {
    if !procfs_readable() {
        return;
    }
    let tree = Tree::spawn(&format!("sleep{} &", " 300".repeat(80)));
    let fake = FakeHerdr::start();
    fake.running(tree.as_ble_sh_reports_it());
    let provider = HerdrProvider::spawn(fake.herdr(), walking_config());

    let pane = only_pane(&provider).await;
    let argv = pane.argv.expect("the job is named");
    assert!(
        argv.chars().count() < 300,
        "a name is a name, and this one is {} characters",
        argv.chars().count()
    );
    assert!(argv.starts_with("sleep 300 300"), "and it still says what it is");
}
