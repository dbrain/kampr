//! A headless Claude pane's composer send and ask-dock answer reach the conversation, not a
//! terminal there is none behind (docs/21 Phase 5b-1). The node routes the pane's input and
//! answer to the conversation; a send that bailed to `write_pane` would come back as a wire
//! error, because a headless pane has no terminal to write to. No test reaches the real API:
//! the session runs under a keeper against the mock, and `KAMPR_KEEP_EXE` points the node at the
//! keeper this build made.

use crate::live::Harness;
use crate::native::Client;
use kampr_auth::Role;
use kampr_core::conversation::{NewSession, Source};
use serde_json::json;
use std::process::{Child, Command as StdCommand};
use std::time::{Duration, Instant};

const CLAUDE: &str = "/home/dbrain/.local/bin/claude";
const MOCK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../research/probe/claude-stream-json/mock.py"
);

struct Mock {
    port: u16,
    child: Child,
    _dir: tempfile::TempDir,
}

impl Mock {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("a mock control dir");
        let port = free_port();
        let child = StdCommand::new("python3")
            .args([MOCK, &port.to_string(), &dir.path().display().to_string()])
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
        Mock {
            port,
            child,
            _dir: dir,
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
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

async fn eventually(what: &str, mut found: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if found() {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The wire frames that are refusals: a composer send or answer that bailed to a terminal there
/// is none behind comes back as one of these.
fn errors(client: &Client) -> Vec<serde_json::Value> {
    client
        .others
        .iter()
        .filter(|m| m["t"].as_str() == Some("error"))
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_headless_panes_composer_send_and_answer_reach_the_conversation_not_a_terminal() {
    let mock = Mock::start();
    let keep = tempfile::tempdir().expect("a throwaway CLAUDE_CONFIG_DIR");
    let keep_root = tempfile::tempdir().expect("a throwaway keeper root");
    for (k, v) in [
        ("KAMPR_TEST", "1"),
        ("KAMPR_KEEP_LAUNCH", "detached"),
        ("ANTHROPIC_BASE_URL", mock.url().as_str()),
        ("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"),
        (
            "KAMPR_KEEP_EXE",
            kampr_testkit::keeper().display().to_string().as_str(),
        ),
        ("KAMPR_KEEP_ROOT", keep_root.path().display().to_string().as_str()),
    ] {
        unsafe { std::env::set_var(k, v) };
    }
    let _h = Harness::start_with("headless-convo", |config| {
        config.claude.binary = CLAUDE.into();
        config.claude.config_dir = keep.path().display().to_string();
        config.claude.mode = "default".into();
    })
    .await;

    // Create a headless Claude pane through the node's own source: the herd lists it.
    let source = _h
        .node
        .sessions
        .claude_source()
        .expect("the node's headless Claude source");
    let session_id = source
        .create(NewSession {
            cwd: Some(keep.path().to_path_buf()),
            prompt: None,
        })
        .await
        .expect("the headless session to start");
    let pane = format!("{}/c:{session_id}", _h.node.node_id());
    eventually("the herd lists the headless pane", || {
        _h.node.herd().pane(&pane).is_some()
    })
    .await;

    let token = _h.token(Role::Full).await;
    let socket = _h.connect(&token).await;
    let mut client = Client::over(socket, token);

    // Watching a headless pane opens its conversation (the watch handler routes it straight to
    // watch_conversation); the abilities frame is the proof it opened.
    client
        .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
        .await;
    client
        .until("the conversation to open", |c| {
            c.others
                .iter()
                .find(|m| m["t"].as_str() == Some("convo.abilities"))
                .cloned()
        })
        .await;

    // The composer send: routed to the conversation, so it does not bail to a terminal (no
    // wire error). A send that went to `write_pane` would come back as one.
    client
        .send(json!({ "t": "input", "pane": pane, "text": "say hello" }))
        .await;
    let settle = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < settle {
        client.pump(Duration::from_millis(100)).await;
    }
    assert!(
        errors(&client).is_empty(),
        "the composer send bailed to a terminal there is none behind: {:?}",
        errors(&client)
    );

    // The ask-dock answer: routed to the conversation the same way, so it too does not bail to
    // a terminal.
    client
        .send(json!({
            "t": "answer", "pane": pane, "key": "1",
            "reply": { "allow": { "input": null, "remember": false } }
        }))
        .await;
    let settle = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < settle {
        client.pump(Duration::from_millis(100)).await;
    }
    assert!(
        errors(&client).is_empty(),
        "the ask-dock answer bailed to a terminal there is none behind: {:?}",
        errors(&client)
    );

    // Close the socket, then end the conversation: the keeper kills the `claude -p` process, so
    // the node's teardown does not wait on a conversation task that is still reading the pipe-log.
    drop(client);
    let _ = source
        .end(&format!("c:{session_id}"), kampr_core::conversation::Ending::End)
        .await;

    for k in [
        "KAMPR_TEST",
        "KAMPR_KEEP_LAUNCH",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "KAMPR_KEEP_EXE",
        "KAMPR_KEEP_ROOT",
    ] {
        unsafe { std::env::remove_var(k) };
    }
}

/// The headless pane carries the seam's full control surface (docs/21 Phase 5b-2): the
/// abilities frame declares what the pane draws, and each declared word reaches the `claude -p`
/// child — the interrupt, the context reads, the completions, the queue and the files.
#[tokio::test(flavor = "multi_thread")]
async fn a_headless_pane_carries_the_seams_full_control_surface() {
    let mock = Mock::start();
    let keep = tempfile::tempdir().expect("a throwaway CLAUDE_CONFIG_DIR");
    let keep_root = tempfile::tempdir().expect("a throwaway keeper root");
    for (k, v) in [
        ("KAMPR_TEST", "1"),
        ("KAMPR_KEEP_LAUNCH", "detached"),
        ("ANTHROPIC_BASE_URL", mock.url().as_str()),
        ("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"),
        (
            "KAMPR_KEEP_EXE",
            kampr_testkit::keeper().display().to_string().as_str(),
        ),
        ("KAMPR_KEEP_ROOT", keep_root.path().display().to_string().as_str()),
    ] {
        unsafe { std::env::set_var(k, v) };
    }
    // Under the api policy the key the mock expects may cross to the child; under the plan
    // default it is scrubbed and the binary has nothing to start with.
    let _h = Harness::start_with("headless-controls", |config| {
        config.claude.binary = CLAUDE.into();
        config.claude.config_dir = keep.path().display().to_string();
        config.claude.mode = "default".into();
        config.claude.billing = kampr_claude::config::Billing::Api;
    })
    .await;

    let source = _h
        .node
        .sessions
        .claude_source()
        .expect("the node's headless Claude source");
    let session_id = source
        .create(NewSession {
            cwd: Some(keep.path().to_path_buf()),
            prompt: None,
        })
        .await
        .expect("the headless session to start");
    let pane = format!("{}/c:{session_id}", _h.node.node_id());
    eventually("the herd lists the headless pane", || {
        _h.node.herd().pane(&pane).is_some()
    })
    .await;

    let token = _h.token(Role::Full).await;
    let socket = _h.connect(&token).await;
    let mut client = Client::over(socket, token);

    client
        .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
        .await;
    let abilities = client
        .until("the conversation to open", |c| {
            c.others
                .iter()
                .find(|m| m["t"].as_str() == Some("convo.abilities"))
                .cloned()
        })
        .await;
    // What the pane draws is what the source declared: the stop control, the completions, the
    // queue words and the context bar.
    let declared = &abilities["abilities"];
    {
        let word = "interrupt";
        assert!(
            declared["steer"]
                .as_array()
                .is_some_and(|s| s.iter().any(|w| w == word)),
            "steer declares {word}: {declared}"
        );
    }
    for word in [
        "slash",
        "mention",
        "queuenext",
        "sendnow",
        "unqueue",
        "files",
        "images",
    ] {
        assert!(
            declared["input"]
                .as_array()
                .is_some_and(|s| s.iter().any(|w| w == word)),
            "input declares {word}: {declared}"
        );
    }
    assert!(
        declared["facts"]
            .as_array()
            .is_some_and(|s| s.iter().any(|w| w == "context")),
        "facts declare context: {declared}"
    );

    // The remaining control words (interrupt, context, queue, files) are verified by the
    // kampr-claude lib tests against the mock: each sends its control request to the child's
    // stdin and the lib test asserts the line appears in the pipe log. The wire-level proof
    // here is that the source declares them, so the pane draws the controls.
    // Close the socket, then end the conversation: the keeper kills the `claude -p` process, so
    // the node's teardown does not wait on a conversation task that is still reading the pipe-log.
    drop(client);
    let _ = source
        .end(&format!("c:{session_id}"), kampr_core::conversation::Ending::End)
        .await;

    for k in [
        "KAMPR_TEST",
        "KAMPR_KEEP_LAUNCH",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_API_KEY",
        "KAMPR_KEEP_EXE",
        "KAMPR_KEEP_ROOT",
    ] {
        unsafe { std::env::remove_var(k) };
    }
}
