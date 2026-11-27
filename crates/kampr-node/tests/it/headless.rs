//! A headless Claude pane's composer send and ask-dock answer reach the child, not a terminal
//! there is none behind (docs/21 Phase 5b). The child is kampr-claude's stand-in
//! (`tests/support/fake-claude.py`), so no turn is spent and nothing reaches Anthropic, and what
//! it records is what the node really sent it.

use crate::native::{Client, Harness};
use kampr_core::conversation::{Ending, NewSession, Source};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

const FAKE: &str = include_str!("../../../kampr-claude/tests/support/fake-claude.py");

async fn eventually(what: &str, mut found: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !found() {
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn recorded(dir: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(name))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn words(dir: &Path) -> Vec<Value> {
    recorded(dir, "stdin.log")
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|l| l["type"] == "user")
        .collect()
}

fn errors(client: &Client) -> Vec<Value> {
    client
        .others
        .iter()
        .filter(|m| m["t"] == "error")
        .cloned()
        .collect()
}

// Each test points the process-wide keeper variables at its own root.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Headless {
    _turn: tokio::sync::MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    _keep_root: tempfile::TempDir,
    _h: Harness,
    source: std::sync::Arc<kampr_claude::ClaudeSource>,
    session_id: String,
    pane: String,
    client: Client,
}

impl Headless {
    async fn start(turn: Value) -> Self {
        let turn_lock = ONE_AT_A_TIME.lock().await;
        let dir = tempfile::tempdir().expect("a scratch dir");
        let keep_root = tempfile::tempdir().expect("a throwaway keeper root");
        let fake = dir.path().join("fake-claude.py");
        std::fs::write(&fake, FAKE).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let account =
            json!({"email": "o@x.dev", "subscriptionType": "Claude Max", "apiProvider": "firstParty"});
        std::fs::write(
            dir.path().join("script.json"),
            json!({"initialize": {"account": account, "commands": [], "models": []}, "turn": turn})
                .to_string(),
        )
        .unwrap();
        for (k, v) in [
            ("KAMPR_KEEP_LAUNCH", "detached"),
            (
                "KAMPR_KEEP_EXE",
                kampr_testkit::keeper().display().to_string().as_str(),
            ),
            ("KAMPR_KEEP_ROOT", keep_root.path().display().to_string().as_str()),
        ] {
            unsafe { std::env::set_var(k, v) };
        }
        let config_dir = dir.path().join("claude");
        let h = Harness::start_with(Harness::home(), |config| {
            config.claude.binary = fake.display().to_string();
            config.claude.config_dir = config_dir.display().to_string();
        })
        .await;

        let source = h
            .node
            .sessions
            .claude_source()
            .expect("the node's headless Claude source");
        let session_id = source
            .create(NewSession {
                cwd: Some(dir.path().to_path_buf()),
                prompt: None,
            })
            .await
            .expect("the headless session to start");
        let pane = format!("{}/c:{session_id}", h.node.node_id());
        eventually("the herd lists the headless pane", || {
            h.node.herd().pane(&pane).is_some()
        })
        .await;

        let client = h.client().await;
        Self {
            _turn: turn_lock,
            dir,
            _keep_root: keep_root,
            _h: h,
            source,
            session_id,
            pane,
            client,
        }
    }

    async fn open(&mut self) -> Value {
        self.client
            .send(json!({ "t": "watch", "pane": self.pane, "conversation": true }))
            .await;
        self.client
            .until("the conversation to open", |c| {
                c.others.iter().find(|m| m["t"] == "convo.abilities").cloned()
            })
            .await
    }

    async fn say(&mut self, text: &str) {
        self.client
            .send(json!({ "t": "input", "pane": self.pane, "text": text }))
            .await;
        let deadline = Instant::now() + Duration::from_secs(30);
        while words(self.dir.path()).is_empty() {
            assert!(
                Instant::now() < deadline,
                "the composer send never reached the child: {:?}",
                errors(&self.client)
            );
            self.client.pump(Duration::from_millis(100)).await;
        }
    }

    async fn asked(&mut self, id: &str, want: impl Fn(&Value) -> bool) -> Value {
        self.client
            .until(id, |c| {
                c.others
                    .iter()
                    .rev()
                    .find(|m| m["t"] == "pending" && m["id"] == id && want(m))
                    .cloned()
            })
            .await
    }

    async fn send(&mut self, mut frame: Value) {
        frame["pane"] = json!(self.pane);
        self.client.send(frame).await;
    }

    /// The body the source wrote back for this request, once it has.
    async fn responded(&mut self, request: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let responses: Vec<Value> = recorded(self.dir.path(), "stdin.log")
                .iter()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter(|l| l["type"] == "control_response")
                .collect();
            let judged = recorded(self.dir.path(), "answers.log").len() >= responses.len();
            let found = responses.iter().find(|l| l["response"]["request_id"] == request);
            if let Some(line) = found.filter(|_| judged) {
                return line["response"]["response"].clone();
            }
            assert!(
                Instant::now() < deadline,
                "{request} was never answered: {:?}",
                errors(&self.client)
            );
            self.client.pump(Duration::from_millis(100)).await;
        }
    }

    async fn end(self) {
        drop(self.client);
        let _ = self
            .source
            .end(&format!("c:{}", self.session_id), Ending::End)
            .await;
        for k in ["KAMPR_KEEP_LAUNCH", "KAMPR_KEEP_EXE", "KAMPR_KEEP_ROOT"] {
            unsafe { std::env::remove_var(k) };
        }
    }
}

fn init() -> Value {
    json!({"type":"system","subtype":"init","session_id":"s","apiKeySource":"none","model":"m"})
}

fn done() -> Value {
    json!({"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0,"result":"done"})
}

#[tokio::test(flavor = "multi_thread")]
async fn a_headless_panes_composer_send_and_answer_reach_the_child() {
    let turn = json!([
        init(),
        {"type":"control_request","request_id":"tool1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"permission_suggestions":[]}},
        {"wait": "answer"},
        done(),
    ]);
    let mut s = Headless::start(turn).await;
    let abilities = s.open().await;
    for word in ["slash", "queuenext", "sendnow", "unqueue", "files", "images"] {
        assert!(
            abilities["abilities"]["input"]
                .as_array()
                .is_some_and(|s| s.iter().any(|w| w == word)),
            "input declares {word}: {abilities}"
        );
    }

    // Under the default plan policy: the account check allows it, and the word reaches the child.
    s.say("say hello").await;
    assert_eq!(words(s.dir.path())[0]["message"]["content"], "say hello");

    let asked = s.asked("tool1", |_| true).await;
    assert_eq!(asked["kind"], "permission", "{asked}");
    s.send(json!({
        "t": "answer", "key": "1", "id": "tool1",
        "reply": { "allow": { "input": null, "remember": false } }
    }))
    .await;
    s.responded("tool1").await;
    assert_eq!(
        recorded(s.dir.path(), "answers.log"),
        ["ok"],
        "the child rejected the answer's shape"
    );
    assert!(errors(&s.client).is_empty(), "{:?}", errors(&s.client));
    s.end().await;
}

fn allow(remember: bool) -> Value {
    json!({ "allow": { "input": null, "remember": remember } })
}

/// Every frame here is one the web client sends: a press carries the permission-shaped reply its
/// key would mean on a permission (keys 1 to 3), words carry none, and a form carries its values.
#[tokio::test(flavor = "multi_thread")]
async fn a_headless_question_plan_and_form_are_answered_with_what_the_clients_send() {
    let questions = json!([
        {"question": "Which fruit?", "header": "Fruit", "multiSelect": false,
         "options": [{"label": "Apple", "description": "red"}, {"label": "Pear"}]},
        {"question": "Which suites?", "header": "Tests", "multiSelect": true,
         "options": [{"label": "unit"}, {"label": "live"}, {"label": "e2e"}]},
        {"question": "Anything else?", "header": "More", "multiSelect": false,
         "options": [{"label": "No"}]},
    ]);
    let schema = json!({
        "type": "object",
        "properties": {
            "count": {"type": "integer", "description": "how many"},
            "colour": {"type": "string", "title": "Colour", "enum": ["red", "teal"]},
        },
        "required": ["colour"],
    });
    let turn = json!([
        init(),
        {"type":"control_request","request_id":"ask1","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions": questions}}},
        {"wait": "answer"},
        {"type":"control_request","request_id":"plan1","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","input":{"plan":"# Split the router"}}},
        {"wait": "answer"},
        {"type":"control_request","request_id":"form1","request":{"subtype":"elicitation","mcp_server_name":"probe","message":"Pick a colour","mode":"form","requested_schema": schema,"property_order":["colour","count"]}},
        {"wait": "answer"},
        done(),
    ]);
    let mut s = Headless::start(turn).await;
    s.open().await;
    s.say("go").await;

    let fruit = s.asked("ask1#0", |_| true).await;
    assert_eq!(fruit["kind"], "question", "{fruit}");
    let labels: Vec<&str> = fruit["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Apple", "Pear", "Type something"], "{fruit}");
    assert_eq!(fruit["options"][2]["text"], true, "{fruit}");
    s.send(json!({ "t": "answer", "key": "2", "id": "ask1#0", "reply": allow(true) }))
        .await;

    let suites = s.asked("ask1#1", |_| true).await;
    assert_eq!(suites["multi"], true, "{suites}");
    s.send(json!({ "t": "answer", "key": "1", "id": "ask1#1", "reply": allow(false) }))
        .await;
    s.send(json!({ "t": "answer", "key": "3", "id": "ask1#1", "reply": { "deny": { "message": null } } }))
        .await;
    s.asked("ask1#1", |p| {
        p["options"][0]["chosen"] == true && p["options"][2]["chosen"] == true
    })
    .await;
    s.send(json!({ "t": "answer.submit", "id": "ask1#1" })).await;

    s.asked("ask1#2", |_| true).await;
    s.send(json!({ "t": "answer", "key": "2", "id": "ask1#2", "text": "ship it" }))
        .await;

    let chosen = s.responded("ask1").await;
    assert_eq!(chosen["behavior"], "allow", "{chosen}");
    assert_eq!(
        chosen["updatedInput"]["answers"],
        json!({ "Which fruit?": "Pear", "Which suites?": "unit, e2e", "Anything else?": "ship it" }),
        "{chosen}"
    );

    let plan = s.asked("plan1", |_| true).await;
    assert_eq!(plan["kind"], "plan", "{plan}");
    let revise = plan["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["text"] == true)
        .unwrap_or_else(|| panic!("an option that takes what to change: {plan}"))
        .clone();
    s.send(json!({ "t": "answer", "key": revise["key"], "id": "plan1", "text": "smaller steps" }))
        .await;
    let revised = s.responded("plan1").await;
    assert_eq!(
        (&revised["behavior"], &revised["message"]),
        (&json!("deny"), &json!("smaller steps")),
        "{revised}"
    );

    let form = s.asked("form1", |_| true).await;
    assert_eq!(form["kind"], "form", "{form}");
    assert_eq!(
        form["fields"],
        json!([
            {"name": "colour", "title": "Colour", "type": "string", "required": true, "choices": ["red", "teal"]},
            {"name": "count", "detail": "how many", "type": "integer"},
        ]),
        "{form}"
    );
    s.send(json!({
        "t": "answer", "key": "1", "id": "form1",
        "reply": { "form": { "values": { "colour": "teal", "count": 3 } } }
    }))
    .await;
    let filled = s.responded("form1").await;
    assert_eq!(
        filled,
        json!({ "action": "accept", "content": { "colour": "teal", "count": 3 } })
    );

    assert!(
        recorded(s.dir.path(), "answers.log").iter().all(|l| l == "ok"),
        "{:?}",
        recorded(s.dir.path(), "answers.log")
    );
    assert!(errors(&s.client).is_empty(), "{:?}", errors(&s.client));
    s.end().await;
}
