//! The billing guards, end to end against the mock: a real `claude -p` under a keeper's pipes,
//! pointed at the mock endpoint, so no test reaches Anthropic.

use std::path::PathBuf;
use std::process::{Child, Command as StdCommand};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kampr_claude::config::Billing;
use kampr_claude::{ClaudeSource, Config};
use kampr_core::conversation::facts::Fix;
use kampr_core::conversation::{Command, Ending, NewSession, Source, Update, When};
use tokio::sync::mpsc;

const CLAUDE: &str = "/home/dbrain/.local/bin/claude";
const MOCK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../research/probe/claude-stream-json/mock.py"
);

struct Mock {
    port: u16,
    dir: PathBuf,
    child: Child,
}

impl Mock {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("a mock control dir").path().to_path_buf();
        let port = free_port();
        let child = StdCommand::new("python3")
            .args([MOCK, &port.to_string(), &dir.display().to_string()])
            .spawn()
            .expect("the mock endpoint");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "the mock never came up");
            std::thread::sleep(Duration::from_millis(50));
        }
        Mock { port, dir, child }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn mode(&self, mode: &str) {
        std::fs::write(self.dir.join("mode"), mode).expect("the mock's mode");
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .unwrap()
        .port()
}

struct Harness {
    mock: Mock,
    config_dir: PathBuf,
    _keep: tempfile::TempDir,
    source: ClaudeSource,
}

impl Harness {
    fn start(billing: Billing) -> Self {
        let mock = Mock::start();
        let keep = tempfile::tempdir().expect("a throwaway CLAUDE_CONFIG_DIR");
        let config_dir = keep.path().to_path_buf();
        set_env(&[
            ("KAMPR_TEST", "1"),
            ("KAMPR_KEEP_LAUNCH", "detached"),
            ("ANTHROPIC_BASE_URL", &mock.url()),
            ("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"),
        ]);
        let (herd, _rx) = kampr_claude::Herd::new();
        let config = Config {
            billing,
            config_dir: config_dir.display().to_string(),
            binary: CLAUDE.into(),
            ..Config::default()
        };
        let source = ClaudeSource::new(
            config,
            Arc::new(herd),
            kampr_keep::root().expect("the keeper root"),
            kampr_testkit::keeper(),
        );
        Harness {
            mock,
            config_dir,
            _keep: keep,
            source,
        }
    }

    fn create(&self) -> String {
        block_on(self.source.create(NewSession {
            cwd: Some(self.config_dir.clone()),
            prompt: None,
        }))
        .expect("the headless session to start")
    }

    /// Open the conversation for `pane`, send `prompt`, and return the `Update`s until a turn ends.
    /// The conversation runs on its own thread and runtime: its pipe-log I/O blocks, and a
    /// long-lived `run` is not joined — the keeper kill is the cleanup, and the process exit
    /// reaps the thread.
    fn turn(&self, pane: &str, prompt: &str) -> Vec<Update> {
        let convo = block_on(self.source.open(pane)).expect("the conversation to open");
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(16);
        let updates = Arc::new(Mutex::new(Vec::new()));
        let keep = updates.clone();
        let emit = Box::new(move |u: Update| {
            keep.lock().unwrap().push(u);
            true
        });
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the conversation");
            rt.block_on(convo.run(cmd_rx, emit));
        });
        std::thread::sleep(Duration::from_secs(3));
        cmd_tx
            .blocking_send(Command::Input {
                text: prompt.into(),
                msg_id: "m1".into(),
                images: vec![],
                files: vec![],
                when: When::Next,
            })
            .expect("the prompt to be queued");
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let done = updates.lock().unwrap().iter().any(|u| {
                matches!(u, Update::Cost(_)) || matches!(u, Update::Trouble(Some(_)))
            });
            if done {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the turn never ended: {:?}",
                updates.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        drop(cmd_tx);
        updates.lock().unwrap().clone()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        clear_env();
    }
}

fn set_env(pairs: &[(&str, &str)]) {
    for (k, v) in pairs {
        unsafe { std::env::set_var(k, v) };
    }
}

fn clear_env() {
    for k in ["KAMPR_TEST", "KAMPR_KEEP_LAUNCH", "ANTHROPIC_BASE_URL", "ANTHROPIC_API_KEY"] {
        unsafe { std::env::remove_var(k) };
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    // Multi-thread: the conversation's run does blocking pipe-log I/O in a task, and a
    // current-thread runtime would let one blocking read stall the whole test.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

fn billing_trouble<'a>(updates: &'a [Update]) -> Option<&'a String> {
    updates.iter().find_map(|u| match u {
        Update::Trouble(Some(t)) if t.fix == Some(Fix::Billing) => Some(&t.text),
        _ => None,
    })
}

// The e2e scenarios run in one test, in order, because they share the process environment.
#[test]
fn the_billing_guards_against_the_mock() {
    // The happy path: a prompt, an answer and a cost, with no billing trouble.
    {
        let h = Harness::start(Billing::Any);
        let pane = h.create();
        let updates = h.turn(&format!("c:{pane}"), "say hello");
        assert!(
            updates.iter().any(|u| matches!(u, Update::Live(l) if !l.text.is_empty())),
            "no live text: {updates:?}"
        );
        assert!(updates.iter().any(|u| matches!(u, Update::Cost(_))), "no cost: {updates:?}");
        assert!(billing_trouble(&updates).is_none(), "an unexpected billing trouble: {updates:?}");
        let _ = block_on(h.source.end(&format!("c:{pane}"), Ending::End));
    }

    // The error path: the mock returns a 401, and the turn is a trouble, not an answer.
    {
        let h = Harness::start(Billing::Any);
        h.mock.mode("error 401");
        let pane = h.create();
        let updates = h.turn(&format!("c:{pane}"), "say hello");
        assert!(
            updates.iter().any(|u| matches!(u, Update::Trouble(Some(t)) if t.kind == "api")),
            "no api trouble: {updates:?}"
        );
        let _ = block_on(h.source.end(&format!("c:{pane}"), Ending::End));
    }

    // Guard 3: under plan, a session that would be paid by a key is refused before a word is sent.
    {
        let h = Harness::start(Billing::Plan);
        let pane = h.create();
        let updates = h.turn(&format!("c:{pane}"), "say hello");
        assert!(
            billing_trouble(&updates).is_some(),
            "a plan session paid by a key must be refused: {updates:?}"
        );
        let _ = block_on(h.source.end(&format!("c:{pane}"), Ending::End));
    }

    // Guard 11: under KAMPR_TEST=1 with no mock address, the launcher refuses to start.
    {
        set_env(&[("KAMPR_TEST", "1"), ("KAMPR_KEEP_LAUNCH", "detached")]);
        let config_dir = tempfile::tempdir().expect("a config dir").path().to_path_buf();
        let (herd, _rx) = kampr_claude::Herd::new();
        let config = Config {
            billing: Billing::Any,
            config_dir: config_dir.display().to_string(),
            binary: CLAUDE.into(),
            ..Config::default()
        };
        let source = ClaudeSource::new(
            config,
            Arc::new(herd),
            kampr_keep::root().expect("the keeper root"),
            kampr_testkit::keeper(),
        );
        let result = block_on(source.create(NewSession {
            cwd: Some(config_dir.clone()),
            prompt: None,
        }));
        assert!(result.is_err(), "the launcher must refuse under test without a mock");
        clear_env();
    }
}