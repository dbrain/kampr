//! The source against a stand-in child (`support/fake-claude.py`) under a real keeper's pipes.
//! No real `claude` runs, so no turn is spent and nothing reaches Anthropic (docs/21 guard 11);
//! what each scenario reads is what the child was really sent and what became of the child.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kampr_claude::config::{Billing, Overage};
use kampr_claude::{ClaudeSource, Config, Herd};
use kampr_core::conversation::facts::{Fix, RunState};
use kampr_core::conversation::{Command, Ending, NewSession, Reply, Source, Update, When};
use kampr_keep::{PipeDir, PipeRead, SeqReader, SessionDir};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const FAKE: &str = include_str!("support/fake-claude.py");

fn wait_for(what: &str, secs: u64, mut found: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !found() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

/// `real` run from an environment as bare as a service manager's, with `<dir>/home` for a home,
/// so no scenario reads the operator's own profile.
fn bare_shell(dir: &std::path::Path, real: &str) -> String {
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let shell = dir.join(format!("login-{}", real.rsplit('/').next().unwrap_or(real)));
    if !shell.exists() {
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nHOME='{}' PATH=/usr/bin:/bin exec {real} \"$@\"\n",
                home.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    shell.display().to_string()
}

fn plan_account() -> Value {
    json!({"email": "o@x.dev", "subscriptionType": "Claude Max", "apiProvider": "firstParty"})
}

fn initialize(account: Value) -> Value {
    json!({"account": account, "commands": [], "models": [], "fast_mode_state": "off", "current_permission_mode": "default"})
}

struct Rig {
    dir: tempfile::TempDir,
    root: tempfile::TempDir,
    config: Config,
    source: ClaudeSource,
}

impl Rig {
    fn start(billing: Billing, overage: Overage, script: Value) -> Self {
        Self::start_with(billing, overage, script, |dir, _| bare_shell(dir, "/bin/sh"))
    }

    fn start_with(
        billing: Billing,
        overage: Overage,
        script: Value,
        shell: impl FnOnce(&std::path::Path, &mut Config) -> String,
    ) -> Self {
        let dir = tempfile::tempdir().expect("a scratch dir");
        let root = tempfile::tempdir().expect("a keeper root");
        let fake = dir.path().join("fake-claude.py");
        std::fs::write(&fake, FAKE).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("script.json"), script.to_string()).unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        let mut config = Config {
            billing,
            overage,
            config_dir: dir.path().join("claude").display().to_string(),
            binary: fake.display().to_string(),
            ..Config::default()
        };
        let shell = shell(dir.path(), &mut config);
        let source = ClaudeSource::new(
            config.clone(),
            Arc::new(Herd::at(root.path()).0),
            root.path().to_path_buf(),
            kampr_testkit::keeper(),
            shell,
        );
        Rig {
            dir,
            root,
            config,
            source,
        }
    }

    fn source_for(config: &Config, root: &tempfile::TempDir, herd: Arc<Herd>) -> ClaudeSource {
        ClaudeSource::new(
            config.clone(),
            herd,
            root.path().to_path_buf(),
            kampr_testkit::keeper(),
            bare_shell(
                std::path::Path::new(&config.config_dir)
                    .parent()
                    .expect("the rig's dir"),
                "/bin/sh",
            ),
        )
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn create(&self, prompt: Option<&str>) -> String {
        block_on(self.source.create(NewSession {
            cwd: Some(self.work()),
            prompt: prompt.map(str::to_string),
        }))
        .expect("the session to start")
    }

    /// The lines of a file the child keeps, parsed where they are JSON.
    fn lines(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn stdin(&self) -> Vec<Value> {
        self.lines("stdin.log")
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn words(&self) -> Vec<Value> {
        self.stdin().into_iter().filter(|l| l["type"] == "user").collect()
    }

    fn session(&self, id: &str) -> SessionDir {
        SessionDir::at(self.root.path().join(id))
    }

    /// Everything on one side of the child's pipes so far, as the keeper logged it.
    fn logged(&self, id: &str, side: PipeDir) -> Vec<String> {
        let mut reader = SeqReader::at(self.session(id).path(), 0).expect("the log");
        let mut lines = Vec::new();
        while let Ok(PipeRead::Entry(entry)) = reader.read() {
            if entry.dir == side {
                lines.push(entry.line);
            }
        }
        lines
    }

    fn out(&self, id: &str) -> Vec<String> {
        self.logged(id, PipeDir::Out)
    }

    /// What was written to the child, whether or not it lived to read it.
    fn sent(&self, id: &str) -> Vec<Value> {
        self.logged(id, PipeDir::In)
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn exited(&self, id: &str) -> bool {
        self.session(id).exit().is_some()
    }
}

/// A conversation running on its own thread, as the node runs one.
struct Watch {
    updates: Arc<Mutex<Vec<Update>>>,
    commands: Option<mpsc::Sender<Command>>,
    finished: Arc<AtomicBool>,
}

impl Watch {
    fn open(source: &ClaudeSource, id: &str) -> Self {
        let convo = block_on(source.open(&format!("c:{id}"))).expect("the conversation to open");
        let (tx, rx) = mpsc::channel(32);
        let updates = Arc::new(Mutex::new(Vec::new()));
        let finished = Arc::new(AtomicBool::new(false));
        let (keep, done) = (updates.clone(), finished.clone());
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(convo.run(
                rx,
                Box::new(move |u| {
                    keep.lock().unwrap().push(u);
                    true
                }),
            ));
            done.store(true, Ordering::SeqCst);
        });
        Watch {
            updates,
            commands: Some(tx),
            finished,
        }
    }

    fn send(&self, command: Command) {
        self.commands
            .as_ref()
            .unwrap()
            .blocking_send(command)
            .expect("the conversation to take it");
    }

    fn say(&self, text: &str) {
        self.send(Command::Input {
            text: text.into(),
            msg_id: "m1".into(),
            images: vec![],
            files: vec![],
            when: When::Idle,
        });
    }

    fn updates(&self) -> Vec<Update> {
        self.updates.lock().unwrap().clone()
    }

    fn has(&self, pred: impl Fn(&Update) -> bool) -> bool {
        self.updates().iter().any(pred)
    }

    fn leave(&mut self) {
        self.commands = None;
        wait_for("the conversation to end when its pane left", 10, || {
            self.finished.load(Ordering::SeqCst)
        });
    }
}

fn billing_trouble(w: &Watch) -> bool {
    w.has(|u| matches!(u, Update::Trouble(Some(t)) if t.fix == Some(Fix::Billing)))
}

fn text_of(turn: &kampr_journal::Turn) -> String {
    turn.blocks
        .iter()
        .filter_map(|b| match b {
            kampr_journal::Block::Md { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn init_line(key: &str) -> Value {
    json!({"type":"system","subtype":"init","session_id":"s","apiKeySource":key,"model":"m","permissionMode":"default"})
}

fn result_line() -> Value {
    json!({"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.01,"result":"done"})
}

fn can_use_bash() -> Value {
    json!({"type":"control_request","request_id":"tool1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"permission_suggestions":[]}})
}

// The scenarios share a process environment (the keeper inherits the parent's), so they run in
// one test, in order, and a failure names its scenario.
#[test]
fn the_source_against_a_stand_in_child() {
    unsafe {
        std::env::set_var("KAMPR_KEEP_LAUNCH", "detached");
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-api03-in-the-parent");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "token-in-the-parent");
        std::env::set_var("ANTHROPIC_BASE_URL", "http://stray.invalid");
        std::env::set_var("CLAUDE_CODE_USE_BEDROCK", "1");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "aws-in-the-parent");
    }
    a_created_session_is_named_prompted_scrubbed_and_listed_with_its_history();
    a_headless_child_starts_with_the_login_environment_less_what_pays();
    a_claude_only_bashrc_finds_starts_with_bashrcs_path();
    a_key_beside_the_login_ends_the_session_before_a_word_is_sent();
    a_turn_paid_by_a_key_is_stopped_for_real();
    an_overage_is_stopped_for_real();
    an_answer_reaches_the_child_in_a_shape_it_accepts();
    a_node_that_restarts_comes_back_to_the_standing_request();
    a_session_that_has_ended_opens_ended();
    a_launch_under_test_with_no_mock_is_refused();
    a_conversation_is_handed_over_once_and_nothing_resumes_it_meanwhile();
    an_opening_prompt_outlives_a_pane_that_left_before_sending_it();
}

fn a_conversation_is_handed_over_once_and_nothing_resumes_it_meanwhile() {
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
    );
    let id = rig.create(None);
    let pane = format!("c:{id}");
    wait_for("the child to start", 20, || rig.session(&id).keeper().is_some());
    let handed = block_on(rig.source.hand_over(&pane)).expect("the first hand-over");
    let project = rig.dir.path().join("claude/projects/-handed");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(format!("{}.jsonl", handed.claim.id())),
        json!({"type":"user","cwd": rig.work().display().to_string(),"message":{"content":"x"}}).to_string()
            + "\n",
    )
    .unwrap();

    let again = block_on(rig.source.hand_over(&pane));
    assert!(again.is_err(), "a hand-over in flight was started again");
    let resumed = block_on(rig.source.resume(handed.claim.id(), None, None));
    assert!(
        resumed.is_err(),
        "resumed headless while a terminal was being started on it: {:?}",
        resumed.ok()
    );
    drop(handed);
    let after = block_on(rig.source.hand_over(&pane));
    assert!(
        after.is_err(),
        "a conversation already handed over was handed over again"
    );
}

fn an_opening_prompt_outlives_a_pane_that_left_before_sending_it() {
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
    );
    let id = rig.create(Some("kept for later"));
    drop(block_on(rig.source.open(&format!("c:{id}"))).expect("the first pane to open"));
    let mut watch = Watch::open(&rig.source, &id);
    wait_for("the opening prompt to reach the child", 20, || {
        !rig.words().is_empty()
    });
    watch.leave();
    let mut again = Watch::open(&rig.source, &id);
    std::thread::sleep(Duration::from_secs(1));
    again.leave();
    let words: Vec<_> = rig
        .words()
        .iter()
        .map(|w| w["message"]["content"].clone())
        .collect();
    assert_eq!(words, [json!("kept for later")], "sent once, and only once");
}

fn a_created_session_is_named_prompted_scrubbed_and_listed_with_its_history() {
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
    );
    let id = rig.create(Some("hi there"));
    assert!(rig.source_herd_lists(&id), "the herd lists the new session");

    let mut watch = Watch::open(&rig.source, &id);
    wait_for("the opening prompt to reach the child", 20, || {
        !rig.words().is_empty()
    });
    assert_eq!(rig.words()[0]["message"]["content"], "hi there");
    let initialized = rig
        .stdin()
        .iter()
        .position(|l| l["request"]["subtype"] == "initialize")
        .unwrap();
    let first_word = rig.stdin().iter().position(|l| l["type"] == "user").unwrap();
    assert!(
        initialized < first_word,
        "a word went out before the account was asked about"
    );

    // How it was started.
    let argv: Vec<String> = serde_json::from_str(&rig.lines("argv.log")[0]).unwrap();
    let at = argv
        .iter()
        .position(|a| a == "--session-id")
        .expect("a named session");
    let uuid = &argv[at + 1];
    assert_eq!(uuid.len(), 36, "{uuid}");
    assert!(!argv.iter().any(|a| a == "--bare"));
    let env: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(rig.dir.path().join("env.json")).unwrap()).unwrap();
    for name in env.keys() {
        let leaked = ["ANTHROPIC_", "CLAUDE_CODE_USE_"]
            .iter()
            .any(|p| name.starts_with(p));
        assert!(!leaked, "{name} reached the child of a plan session");
    }
    assert!(env.contains_key("HOME") && env.contains_key("PATH"));
    assert_eq!(env["CLAUDE_CONFIG_DIR"], rig.config.config_dir);

    // The history is the transcript the child wrote under the id that was chosen for it.
    watch.send(Command::Older { before: None });
    wait_for("the transcript to be paged", 10, || {
        watch.has(|u| matches!(u, Update::Older(_)))
    });
    let page = watch
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Older(p) => Some(p),
            _ => None,
        })
        .unwrap();
    assert!(
        page.turns.iter().any(|t| text_of(t).contains("fake answer")),
        "{page:?}"
    );
    watch.send(Command::Find {
        query: "hi there".into(),
        cap: 5,
    });
    wait_for("the transcript to be searched", 10, || {
        watch.has(|u| matches!(u, Update::Found { found, .. } if found.total == 1))
    });

    // While it runs it is not offered for resuming: two writers would fork the transcript.
    let live = block_on(rig.source.resumable(Some(&rig.work())));
    assert!(live.iter().all(|s| &s.id != uuid), "{live:?}");

    block_on(rig.source.end(&format!("c:{id}"), Ending::End)).expect("end");
    wait_for("the child to exit", 10, || rig.exited(&id));
    assert!(!rig.source_herd_lists(&id));
    watch.leave();

    // Once it has ended it can be resumed by that id, from this working directory.
    let stored = block_on(rig.source.resumable(Some(&rig.work())));
    let found = stored
        .iter()
        .find(|s| &s.id == uuid)
        .unwrap_or_else(|| panic!("{stored:?}"));
    assert_eq!(found.title.as_deref(), Some("hi there"));
}

/// A node run as a service has the service manager's `PATH` and none of the operator's profile, so
/// the child is given what the operator's login shell builds, as a native session's terminal is,
/// less what would decide who pays.
fn a_headless_child_starts_with_the_login_environment_less_what_pays() {
    let rig = Rig::start_with(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
        |dir, _| {
            let shell = bare_shell(dir, "/bin/sh");
            let bin = dir.join("profile-bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(
                dir.join("home").join(".profile"),
                format!(
                    "echo welcome back\nexport PATH='{}':\"$PATH\"\nexport FOO_FROM_PROFILE=1\n\
                     export ANTHROPIC_API_KEY=leak\nexport CLAUDE_CODE_USE_VERTEX=1\n",
                    bin.display()
                ),
            )
            .unwrap();
            shell
        },
    );
    rig.create(None);
    let env_json = rig.dir.path().join("env.json");
    wait_for("the child to start", 20, || env_json.exists());
    let env: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(env_json).unwrap()).unwrap();

    let bin = rig.dir.path().join("profile-bin").display().to_string();
    assert!(env["PATH"].split(':').any(|d| d == bin), "{:?}", env["PATH"]);
    assert_eq!(env.get("FOO_FROM_PROFILE").map(String::as_str), Some("1"));
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_BEDROCK",
    ] {
        assert!(
            !env.contains_key(key),
            "{key} reached the child of a plan session"
        );
    }
    assert_eq!(
        env.get("AWS_SECRET_ACCESS_KEY").map(String::as_str),
        Some("aws-in-the-parent")
    );
    assert_eq!(env["CLAUDE_CONFIG_DIR"], rig.config.config_dir);
    let here = std::env::current_dir().unwrap().display().to_string();
    assert_ne!(
        env.get("PWD"),
        Some(&here),
        "the capturing shell's own directory crossed"
    );
}

/// #419: `~/.local/bin`, where Claude installs itself, is added by `.bashrc` on half this herd,
/// and only an interactive shell reads it. The binary is found there and the child is given it.
fn a_claude_only_bashrc_finds_starts_with_bashrcs_path() {
    let rig = Rig::start_with(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
        |dir, config| {
            let shell = bare_shell(dir, "/bin/bash");
            let bin = dir.join("bashrc-bin");
            std::fs::create_dir_all(&bin).unwrap();
            let claude = bin.join("kampr-bashrc-claude");
            std::fs::write(&claude, format!("#!/bin/sh\nexec '{}' \"$@\"\n", config.binary)).unwrap();
            std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            std::fs::write(
                dir.join("home").join(".bashrc"),
                format!("export PATH='{}':\"$PATH\"\n", bin.display()),
            )
            .unwrap();
            config.binary = "kampr-bashrc-claude".into();
            shell
        },
    );
    rig.create(None);
    let env_json = rig.dir.path().join("env.json");
    wait_for("the child to start", 20, || env_json.exists());
    let env: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(env_json).unwrap()).unwrap();
    let bin = rig.dir.path().join("bashrc-bin").display().to_string();
    assert!(env["PATH"].split(':').any(|d| d == bin), "{:?}", env["PATH"]);
}

fn a_key_beside_the_login_ends_the_session_before_a_word_is_sent() {
    let key =
        json!({"tokenSource": "claude.ai", "apiKeySource": "ANTHROPIC_API_KEY", "apiProvider": "firstParty"});
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(key)}),
    );
    let id = rig.create(Some("never sent"));
    let watch = Watch::open(&rig.source, &id);
    wait_for("the account check to refuse", 20, || billing_trouble(&watch));
    wait_for("the child to be hung up on", 10, || rig.exited(&id));
    assert!(rig.words().is_empty(), "a word reached a session paid by a key");
    assert!(watch.has(|u| matches!(u, Update::State(RunState::Ended))));
}

fn a_turn_paid_by_a_key_is_stopped_for_real() {
    let turn = json!([init_line("ANTHROPIC_API_KEY"), {"sleep": 2.0}, result_line()]);
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account()), "turn": turn}),
    );
    let id = rig.create(None);
    let watch = Watch::open(&rig.source, &id);
    wait_for("the account to be allowed", 20, || {
        watch.has(|u| matches!(u, Update::Session(s) if s.billing.provider.is_some()))
    });
    watch.say("go");
    wait_for("the guard to see the key", 20, || billing_trouble(&watch));
    wait_for("the child to be hung up on", 5, || rig.exited(&id));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        !rig.out(&id).iter().any(|l| l.contains("\"type\":\"result\"")),
        "the turn ran on after the guard said it stopped"
    );
    assert!(
        rig.sent(&id)
            .iter()
            .any(|l| l["request"]["subtype"] == "interrupt"),
        "the turn was hung up on without being interrupted first"
    );
}

fn an_overage_is_stopped_for_real() {
    let overage = json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed","isUsingOverage":true,"unifiedWindows":{"five_hour":{"utilization":1.1,"resetsAt":1795800000}}}});
    let turn = json!([init_line("none"), overage, {"sleep": 2.0}, result_line()]);
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account()), "turn": turn}),
    );
    let id = rig.create(None);
    let watch = Watch::open(&rig.source, &id);
    wait_for("the account to be allowed", 20, || {
        watch.has(|u| matches!(u, Update::Session(s) if s.billing.provider.is_some()))
    });
    watch.say("go");
    wait_for("the guard to see the overage", 20, || billing_trouble(&watch));
    wait_for("the child to be hung up on", 5, || rig.exited(&id));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        !rig.out(&id).iter().any(|l| l.contains("\"type\":\"result\"")),
        "the turn ran on onto credits"
    );
}

fn an_answer_reaches_the_child_in_a_shape_it_accepts() {
    let turn = json!([init_line("none"), can_use_bash(), {"wait": "answer"}, result_line()]);
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account()), "turn": turn}),
    );
    let id = rig.create(None);
    let watch = Watch::open(&rig.source, &id);
    wait_for("the account to be allowed", 20, || {
        watch.has(|u| matches!(u, Update::Session(s) if s.billing.provider.is_some()))
    });
    watch.say("go");
    wait_for("the ask", 20, || {
        watch.has(|u| matches!(u, Update::Asks(a) if a.iter().any(|a| a.id.as_deref() == Some("tool1"))))
    });
    watch.send(Command::Answer {
        ask: "tool1".into(),
        reply: Reply::Allow {
            input: None,
            remember: false,
        },
    });
    wait_for("the child to judge the answer", 10, || {
        !rig.lines("answers.log").is_empty()
    });
    assert_eq!(rig.lines("answers.log"), ["ok"]);
    wait_for("the turn to finish", 10, || {
        watch.has(|u| matches!(u, Update::Cost(_)))
    });
}

fn a_node_that_restarts_comes_back_to_the_standing_request() {
    let turn = json!([init_line("none"), can_use_bash(), {"wait": "answer"}, result_line()]);
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account()), "turn": turn}),
    );
    let id = rig.create(None);
    let mut watch = Watch::open(&rig.source, &id);
    wait_for("the account to be allowed", 20, || {
        watch.has(|u| matches!(u, Update::Session(s) if s.billing.provider.is_some()))
    });
    watch.say("go");
    wait_for("the ask", 20, || {
        watch.has(|u| matches!(u, Update::Asks(a) if !a.is_empty()))
    });
    watch.leave();

    // The node goes away and comes back: its herd is rebuilt from the keeper's directory.
    let (herd, _) = Herd::at(rig.root.path());
    assert_eq!(
        herd.sessions(),
        std::slice::from_ref(&id),
        "a live session is a pane again after a restart"
    );
    let again = Rig::source_for(&rig.config, &rig.root, Arc::new(herd));
    let watch = Watch::open(&again, &id);
    wait_for("the same ask", 20, || {
        watch.has(|u| matches!(u, Update::Asks(a) if a.iter().any(|a| a.id.as_deref() == Some("tool1"))))
    });
    watch.send(Command::Answer {
        ask: "tool1".into(),
        reply: Reply::Deny { message: None },
    });
    wait_for("the child to judge the answer", 10, || {
        !rig.lines("answers.log").is_empty()
    });
    assert_eq!(rig.lines("answers.log"), ["ok"]);
    wait_for("the turn to finish", 10, || {
        watch.has(|u| matches!(u, Update::Cost(_)))
    });

    let initializes = rig
        .stdin()
        .iter()
        .filter(|l| l["request"]["subtype"] == "initialize")
        .count();
    assert_eq!(
        initializes, 1,
        "the reopened conversation initialised the session again"
    );
    assert_eq!(rig.words().len(), 1);
    block_on(again.end(&format!("c:{id}"), Ending::End)).unwrap();
}

fn a_session_that_has_ended_opens_ended() {
    let rig = Rig::start(
        Billing::Plan,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
    );
    let id = rig.create(None);
    let first = Watch::open(&rig.source, &id);
    wait_for("the account to be allowed", 20, || {
        first.has(|u| matches!(u, Update::Session(s) if s.billing.provider.is_some()))
    });
    block_on(rig.source.end(&format!("c:{id}"), Ending::End)).unwrap();
    wait_for("the child to exit", 10, || rig.exited(&id));

    let watch = Watch::open(&rig.source, &id);
    wait_for("a dead child to be said to have ended", 10, || {
        watch.has(|u| matches!(u, Update::State(RunState::Ended)))
    });
    wait_for("the conversation to stop waiting on it", 10, || {
        watch.finished.load(Ordering::SeqCst)
    });
}

// Guard 11: a launch under KAMPR_TEST=1 names the mock it is pointed at, or does not start.
fn a_launch_under_test_with_no_mock_is_refused() {
    let rig = Rig::start(
        Billing::Any,
        Overage::Refuse,
        json!({"initialize": initialize(plan_account())}),
    );
    unsafe {
        std::env::set_var("KAMPR_TEST", "1");
        std::env::remove_var("ANTHROPIC_BASE_URL");
    }
    let refused = block_on(rig.source.create(NewSession {
        cwd: Some(rig.work()),
        prompt: None,
    }));
    unsafe { std::env::remove_var("KAMPR_TEST") };
    assert!(
        refused.is_err(),
        "the launcher started a session under test with no mock"
    );
}

impl Rig {
    fn source_herd_lists(&self, id: &str) -> bool {
        self.source.herd().sessions().iter().any(|s| s == id)
    }
}
