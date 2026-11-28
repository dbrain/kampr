//! Starting a headless Claude conversation, resuming a stored one, and handing one between a
//! terminal and no terminal (docs/21 Phase 5). The headless child is kampr-claude's stand-in
//! (`fake-claude.py`), so no turn is spent and what it records is what the node really sent it.

use super::{Client, Harness, NODE, fake_claude, session_of};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const FAKE: &str = include_str!("../../../../kampr-claude/tests/support/fake-claude.py");

// Harness::start_with takes a plain fn, and each test is its own process under nextest.
static HOME: OnceLock<PathBuf> = OnceLock::new();
static TWEAK: OnceLock<fn(&mut kampr_node::Config)> = OnceLock::new();

struct Headless {
    h: Harness,
    home: PathBuf,
}

impl Drop for Headless {
    fn drop(&mut self) {
        kampr_testkit::stop_keepers(&self.h.root());
    }
}

impl Headless {
    async fn start() -> Self {
        Self::start_with(|_| {}).await
    }

    async fn start_with(tweak: fn(&mut kampr_node::Config)) -> Self {
        let dir = Harness::home();
        let home = dir.path().to_path_buf();
        HOME.set(home.clone()).expect("one fixture per test process");
        TWEAK.set(tweak).expect("one fixture per test process");
        let fake = home.join("fake");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::write(fake.join("fake-claude.py"), FAKE).unwrap();
        std::fs::set_permissions(
            fake.join("fake-claude.py"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        script(
            &fake,
            json!([
                init(), {"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0,"result":"done"}]),
        );
        // The headless child's environment is read through the login shell, and this one reads
        // the fixture's profile rather than the operator's.
        let shell = home.join("login-sh");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nHOME='{}' PATH=/usr/bin:/bin exec /bin/sh \"$@\"\n",
                home.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        for (k, v) in [
            ("SHELL", shell.display().to_string()),
            ("KAMPR_KEEP_LAUNCH", "detached".to_string()),
            ("KAMPR_KEEP_EXE", kampr_testkit::keeper().display().to_string()),
            ("KAMPR_KEEP_ROOT", home.join("claude-keep").display().to_string()),
        ] {
            unsafe { std::env::set_var(k, v) };
        }
        let h = Harness::start_with(dir, |config| {
            let home = HOME.get().expect("a home");
            config.claude.binary = home.join("fake").join("fake-claude.py").display().to_string();
            config.claude.config_dir = home.join(".claude").display().to_string();
            (TWEAK.get().expect("a tweak"))(config);
        })
        .await;
        Self { h, home }
    }

    fn fake(&self) -> PathBuf {
        self.home.join("fake")
    }

    fn launches(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.fake().join("argv.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn words(&self) -> Vec<String> {
        std::fs::read_to_string(self.fake().join("stdin.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|l| l["type"] == "user")
            .map(|l| l["message"]["content"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    async fn create(&self, client: &mut Client, prompt: Option<&str>) -> String {
        let mut op =
            json!({ "op": "claude.create", "node": NODE, "cwd": self.h.work().display().to_string() });
        if let Some(prompt) = prompt {
            op["prompt"] = json!(prompt);
        }
        let ack = client.manage(op).await;
        assert_eq!(ack["ok"], json!(true), "{ack}");
        let pane = ack["id"].as_str().expect("the new pane").to_string();
        assert!(pane.starts_with(&format!("{NODE}/c:")), "{pane}");
        client.pane(&pane, |p| p["agent"] == "claude").await;
        pane
    }

    async fn converse(&self, client: &mut Client, pane: &str) {
        client
            .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
            .await;
        client
            .until("the conversation to open", |c| {
                c.others.iter().find(|m| m["t"] == "convo.abilities").cloned()
            })
            .await;
    }
}

fn init() -> Value {
    json!({"type":"system","subtype":"init","session_id":"s","apiKeySource":"none","model":"m"})
}

/// What the fake plays for each prompt.
fn script(fake: &std::path::Path, turn: Value) {
    let account = json!({"email": "o@x.dev", "subscriptionType": "Claude Max", "apiProvider": "firstParty"});
    std::fs::write(
        fake.join("script.json"),
        json!({"initialize": {"account": account, "commands": [], "models": []}, "turn": turn, "linger": 1})
            .to_string(),
    )
    .unwrap();
}

/// A hand-over waits out a turn, so a test of one at rest waits for the turn's result to reach the
/// pane: the state the pane opened on, then the one the result set.
async fn turn_over(client: &mut Client) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while client.others.iter().filter(|m| m["t"] == "convo.state").count() < 2 {
        assert!(
            Instant::now() < deadline,
            "the turn never ended: {:?}",
            client.others
        );
        client.pump(Duration::from_millis(100)).await;
    }
}

async fn pumped(client: &mut Client, what: &str, mut found: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !found() {
        assert!(
            Instant::now() < deadline,
            "{what} never happened: {:?}",
            client.others
        );
        client.pump(Duration::from_millis(100)).await;
    }
}

fn resumes(argv: &[String], id: &str) -> bool {
    argv.windows(2).any(|w| w[0] == "-r" && w[1] == id)
}

fn claude_id_of(argv: &[String]) -> String {
    argv.windows(2)
        .find(|w| w[0] == "--session-id")
        .map(|w| w[1].clone())
        .expect("a new conversation is named")
}

#[tokio::test]
async fn a_new_conversation_runs_its_first_prompt_and_moves_to_a_terminal() {
    let f = Headless::start().await;
    let mut client = f.h.client().await;
    let node = client.until("the node", |c| c.nodes.get(NODE).cloned()).await;
    assert_eq!(node["claude"], json!(true), "{node}");

    let pane = f.create(&mut client, Some("first words")).await;
    f.converse(&mut client, &pane).await;
    pumped(&mut client, "the first prompt reaching the child", || {
        f.words() == ["first words"]
    })
    .await;
    let claude = claude_id_of(&f.launches()[0]);
    turn_over(&mut client).await;

    let ack = client
        .manage(json!({ "op": "claude.to_terminal", "at": pane }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let terminal = ack["id"].as_str().expect("the terminal pane").to_string();
    assert!(terminal.starts_with(&format!("{NODE}/n:")), "{terminal}");
    pumped(&mut client, "the terminal's claude starting", || {
        f.launches()
            .iter()
            .any(|argv| resumes(argv, &claude) && !argv.iter().any(|a| a == "-p"))
    })
    .await;
    let launched = f.launches();
    let exited = launched.iter().position(|argv| argv == &["exited"]);
    let terminal_started = launched
        .iter()
        .position(|argv| resumes(argv, &claude) && !argv.iter().any(|a| a == "-p"));
    assert!(
        exited.is_some() && exited < terminal_started,
        "the terminal took the transcript while the headless child still held it: {launched:?}"
    );
    let spec = session_of(&f.h.root(), &terminal)
        .spec()
        .expect("the terminal session");
    assert!(resumes(&spec.argv, &claude), "{:?}", spec.argv);
    assert_eq!(
        spec.cwd.map(PathBuf::from),
        Some(f.h.work()),
        "the terminal opens where the conversation ran"
    );
    client.gone(&pane).await;
}

fn terminal_launches(f: &Headless, claude: &str) -> usize {
    f.launches()
        .iter()
        .filter(|argv| resumes(argv, claude) && !argv.iter().any(|a| a == "-p"))
        .count()
}

async fn quiet(client: &mut Client, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        client.pump(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_conversation_handed_to_a_terminal_is_not_handed_over_again() {
    let f = Headless::start().await;
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, None).await;
    pumped(&mut client, "the child starting", || !f.launches().is_empty()).await;
    let claude = claude_id_of(&f.launches()[0]);

    let first = client
        .manage(json!({ "op": "claude.to_terminal", "at": pane }))
        .await;
    assert_eq!(first["ok"], json!(true), "{first}");
    let second = client
        .manage(json!({ "op": "claude.to_terminal", "at": pane }))
        .await;
    assert_eq!(second["ok"], json!(false), "{second}");
    pumped(&mut client, "the terminal's claude starting", || {
        terminal_launches(&f, &claude) > 0
    })
    .await;
    quiet(&mut client, 1500).await;
    assert_eq!(
        terminal_launches(&f, &claude),
        1,
        "two writers on one transcript fork it: {:?}",
        f.launches()
    );
}

// Neither side running is the one outcome a hand-over must not leave.
#[tokio::test]
async fn a_terminal_that_cannot_start_gives_the_conversation_back_to_a_headless_child() {
    let f = Headless::start_with(|config| {
        let keeper = HOME.get().expect("a home").join("native-keeper");
        std::os::unix::fs::symlink(kampr_testkit::keeper(), &keeper).unwrap();
        config.native.keeper = keeper.display().to_string();
    })
    .await;
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, Some("first words")).await;
    f.converse(&mut client, &pane).await;
    pumped(&mut client, "the first prompt reaching the child", || {
        f.words() == ["first words"]
    })
    .await;
    let claude = claude_id_of(&f.launches()[0]);
    turn_over(&mut client).await;

    std::fs::remove_file(f.home.join("native-keeper")).unwrap();
    let ack = client
        .manage(json!({ "op": "claude.to_terminal", "at": pane }))
        .await;

    assert_eq!(ack["ok"], json!(false), "{ack}");
    let message = ack["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("starting the keeper") && message.contains("resumed without a terminal as c:"),
        "the failure and the rollback are both said: {ack}"
    );
    pumped(&mut client, "the conversation resumed headless", || {
        f.launches()
            .iter()
            .any(|argv| resumes(argv, &claude) && argv.iter().any(|a| a == "-p"))
    })
    .await;
    client
        .until("a headless pane on the conversation", |c| {
            c.panes
                .keys()
                .find(|id| id.starts_with(&format!("{NODE}/c:")) && **id != pane)
                .cloned()
        })
        .await;
    assert_eq!(terminal_launches(&f, &claude), 0);
}

#[tokio::test]
async fn a_terminal_claude_is_left_running_when_no_headless_claude_can_start() {
    let f = Headless::start().await;
    let id = "66666666-6666-4666-8666-666666666666";
    let claude = fake_claude(&f.h.work(), "idle");
    let mut client = f.h.client().await;
    let terminal = client
        .create(
            &[&claude.display().to_string(), &f.home.display().to_string(), id],
            json!({ "cwd": f.h.work().display().to_string() }),
        )
        .await;
    client
        .pane(&terminal, |p| {
            p["converses"] == json!(true) && p["agent_status"] == "idle"
        })
        .await;
    std::fs::rename(f.fake().join("fake-claude.py"), f.fake().join("moved.py")).unwrap();

    let ack = client
        .manage(json!({ "op": "claude.from_terminal", "at": terminal }))
        .await;
    assert_eq!(ack["ok"], json!(false), "{ack}");
    assert!(
        ack["message"]
            .as_str()
            .unwrap_or_default()
            .contains("fake-claude.py"),
        "{ack}"
    );
    quiet(&mut client, 1000).await;
    assert!(
        client.panes.contains_key(&terminal),
        "the terminal Claude was stopped with nothing to continue it"
    );
}

async fn terminal_claude(f: &Headless, client: &mut Client, claude: &std::path::Path, id: &str) -> String {
    let terminal = client
        .create(
            &[&claude.display().to_string(), &f.home.display().to_string(), id],
            json!({ "cwd": f.h.work().display().to_string() }),
        )
        .await;
    client
        .pane(&terminal, |p| {
            p["agent"] == "claude" && p["converses"] == json!(true)
        })
        .await;
    terminal
}

async fn resumed_headless(f: &Headless, client: &mut Client, ack: &Value, terminal: &str, id: &str) {
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let pane = ack["id"].as_str().expect("the headless pane").to_string();
    assert!(pane.starts_with(&format!("{NODE}/c:")), "{pane}");
    client.gone(terminal).await;
    pumped(client, "the headless child resuming it", || {
        f.launches()
            .iter()
            .any(|argv| resumes(argv, id) && argv.iter().any(|a| a == "-p"))
    })
    .await;
}

#[tokio::test]
async fn a_terminal_claude_at_rest_continues_without_its_terminal_unasked() {
    let f = Headless::start().await;
    let id = "44444444-4444-4444-8444-444444444444";
    let claude = fake_claude(&f.h.work(), "idle");
    let mut client = f.h.client().await;
    let terminal = terminal_claude(&f, &mut client, &claude, id).await;
    let ack = client
        .manage(json!({ "op": "claude.from_terminal", "at": terminal }))
        .await;
    resumed_headless(&f, &mut client, &ack, &terminal, id).await;
}

// A Claude opened and not yet spoken to has no transcript to resume, and nothing to lose either:
// it continues as a new conversation where it was opened.
#[tokio::test]
async fn a_terminal_claude_not_yet_spoken_to_continues_as_a_new_conversation() {
    let f = Headless::start().await;
    let id = "47474747-4747-4747-8747-474747474747";
    let claude = fake_claude(&f.h.work(), "idle");
    let unspoken: String = std::fs::read_to_string(&claude)
        .unwrap()
        .lines()
        .filter(|l| !l.contains("$project/$id.jsonl"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&claude, unspoken).unwrap();
    let mut client = f.h.client().await;
    let terminal = terminal_claude(&f, &mut client, &claude, id).await;

    let ack = client
        .manage(json!({ "op": "claude.from_terminal", "at": terminal }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let pane = ack["id"].as_str().expect("the headless pane").to_string();
    assert!(pane.starts_with(&format!("{NODE}/c:")), "{pane}");
    client.gone(&terminal).await;
    pumped(&mut client, "a new headless child starting", || {
        f.launches()
            .iter()
            .any(|argv| argv.iter().any(|a| a == "--session-id") && !resumes(argv, id))
    })
    .await;
    client
        .pane(&pane, |p| p["cwd"] == json!(f.h.work().display().to_string()))
        .await;
}

/// A Claude mid-turn is refused with what would stop, and once the operator says stop it, Escape
/// reaches its terminal before it is hung up on. This one goes idle on Escape, as Claude does.
#[tokio::test]
async fn a_busy_terminal_claude_moves_only_when_told_to_stop_and_hears_escape_first() {
    let f = Headless::start().await;
    let id = "45454545-4545-4545-8545-454545454545";
    let claude = fake_claude(&f.h.work(), "busy");
    let script = std::fs::read_to_string(&claude).unwrap().replace(
        "while :; do sleep 1; done",
        r#"marker="$home/.claude/sessions/$$.json"
stty raw -echo
while :; do
  c=$(dd bs=1 count=1 2>/dev/null)
  if [ "$c" = "$(printf '\033')" ]; then
    echo escape >> "$home/escaped"
    s=$(sed 's/"busy"/"idle"/' "$marker"); printf '%s' "$s" > "$marker"
  fi
done"#,
    );
    std::fs::write(&claude, script).unwrap();
    let mut client = f.h.client().await;
    let terminal = terminal_claude(&f, &mut client, &claude, id).await;

    let busy = client
        .manage(json!({ "op": "claude.from_terminal", "at": terminal }))
        .await;
    assert_eq!(busy["ok"], json!(false), "{busy}");
    assert_eq!(busy["code"], "busy", "{busy}");
    assert!(
        busy["message"]
            .as_str()
            .unwrap_or_default()
            .contains("the turn in progress"),
        "{busy}"
    );
    assert!(client.panes.contains_key(&terminal), "a refusal stopped nothing");
    assert!(!f.home.join("escaped").exists(), "a refusal interrupted the turn");

    let ack = client
        .manage(json!({ "op": "claude.from_terminal", "at": terminal, "interrupt": true }))
        .await;
    assert!(
        f.home.join("escaped").exists(),
        "Claude was hung up on without being interrupted first: {ack}"
    );
    resumed_headless(&f, &mut client, &ack, &terminal, id).await;
}

/// The turn and the permission it waits on are what a move would stop; told to, the node
/// interrupts the turn before the terminal takes the conversation.
#[tokio::test]
async fn a_busy_conversation_moves_to_a_terminal_only_when_told_to_stop_its_turn() {
    let f = Headless::start().await;
    let ask = json!({"type":"control_request","request_id":"perm-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"t1"}});
    script(&f.fake(), json!([init(), ask, {"wait": "answer"}]));
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, Some("first words")).await;
    f.converse(&mut client, &pane).await;
    pumped(&mut client, "the first prompt reaching the child", || {
        f.words() == ["first words"]
    })
    .await;
    let claude = claude_id_of(&f.launches()[0]);

    let deadline = Instant::now() + Duration::from_secs(30);
    let busy = loop {
        let busy = client
            .manage(json!({ "op": "claude.to_terminal", "at": pane }))
            .await;
        assert_eq!(busy["ok"], json!(false), "{busy}");
        assert_eq!(busy["code"], "busy", "{busy}");
        if busy["message"].as_str().unwrap_or_default().contains("Bash") || Instant::now() > deadline {
            break busy;
        }
        client.pump(Duration::from_millis(100)).await;
    };
    let said = busy["message"].as_str().unwrap_or_default();
    assert!(
        said.contains("the turn in progress") && said.contains("the permission it is asking for (Bash)"),
        "{busy}"
    );
    assert!(
        !f.launches().iter().any(|argv| argv == &["exited"]),
        "a refusal stopped the child"
    );
    assert_eq!(terminal_launches(&f, &claude), 0);

    let ack = client
        .manage(json!({ "op": "claude.to_terminal", "at": pane, "interrupt": true }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let interrupted = std::fs::read_to_string(f.fake().join("stdin.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .any(|l| l["type"] == "control_request" && l["request"]["subtype"] == "interrupt");
    assert!(interrupted, "the turn was hung up on, not interrupted");
    pumped(&mut client, "the terminal's claude starting", || {
        terminal_launches(&f, &claude) == 1
    })
    .await;
    client.gone(&pane).await;
}

#[tokio::test]
async fn a_stored_conversation_is_listed_for_its_directory_and_resumed_once() {
    let f = Headless::start().await;
    let id = "33333333-3333-4333-8333-333333333333";
    let cwd = f.h.work().display().to_string();
    let project = f.home.join(".claude/projects/-stored");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(format!("{id}.jsonl")),
        json!({"type":"user","cwd": cwd,"message":{"content":"fix the build"}}).to_string() + "\n",
    )
    .unwrap();
    let mut client = f.h.client().await;

    let listed = client
        .manage(json!({ "op": "claude.resumable", "node": NODE, "cwd": cwd }))
        .await;
    assert_eq!(listed["ok"], json!(true), "{listed}");
    assert_eq!(
        listed["resumable"],
        json!([{ "id": id, "cwd": cwd, "title": "fix the build" }]),
        "{listed}"
    );
    let elsewhere = client
        .manage(json!({ "op": "claude.resumable", "node": NODE, "cwd": "/nowhere" }))
        .await;
    assert_eq!(elsewhere["resumable"], json!([]), "{elsewhere}");

    let ack = client
        .manage(json!({ "op": "claude.resume", "node": NODE, "id": id }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let pane = ack["id"].as_str().expect("the resumed pane").to_string();
    assert!(pane.starts_with(&format!("{NODE}/c:")), "{pane}");
    client.pane(&pane, |p| p["cwd"] == json!(cwd)).await;
    pumped(&mut client, "the child resuming it", || {
        f.launches().iter().any(|argv| resumes(argv, id))
    })
    .await;

    let twice = client
        .manage(json!({ "op": "claude.resume", "node": NODE, "id": id }))
        .await;
    assert_eq!(
        twice["ok"],
        json!(false),
        "a second writer forks the transcript: {twice}"
    );

    let ended = client.manage(json!({ "op": "claude.end", "at": pane })).await;
    assert_eq!(ended["ok"], json!(true), "{ended}");
    client.gone(&pane).await;
}

// `close` is the verb every client already sends a pane it is done with.
#[tokio::test]
async fn closing_a_headless_pane_ends_its_conversation() {
    let f = Headless::start().await;
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, None).await;
    pumped(&mut client, "the child starting", || !f.launches().is_empty()).await;

    let closed = client.manage(json!({ "op": "close", "at": pane })).await;
    assert_eq!(closed["ok"], json!(true), "{closed}");
    client.gone(&pane).await;
}

// A node lists every live pipe-mode session under its root as a headless Claude, so a root it was
// not configured with puts another node's conversations, or the operator's, in its herd.
#[tokio::test]
async fn a_headless_conversation_is_kept_under_the_nodes_own_session_root() {
    let f = Headless::start().await;
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, None).await;
    let keeper = pane.rsplit_once("/c:").expect("a headless pane").1;
    assert!(
        f.h.root().join(keeper).exists(),
        "not under the configured root {}",
        f.h.root().display()
    );
    assert!(
        !f.home.join("claude-keep").join(keeper).exists(),
        "kept under the process-wide default root"
    );
}

#[tokio::test]
async fn keys_typed_at_a_headless_pane_are_a_line_and_never_prompt_text() {
    let f = Headless::start().await;
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, None).await;
    f.converse(&mut client, &pane).await;
    for keys in [
        json!(["Enter"]),
        json!(["Up", "Left"]),
        json!(["h", "i", "Enter"]),
    ] {
        client
            .send(json!({ "t": "input", "pane": pane, "keys": keys }))
            .await;
    }
    pumped(&mut client, "the line reaching the child", || {
        !f.words().is_empty()
    })
    .await;
    let quiet = Instant::now() + Duration::from_millis(500);
    while Instant::now() < quiet {
        client.pump(Duration::from_millis(100)).await;
    }
    assert_eq!(f.words(), ["hi"]);
}

#[tokio::test]
async fn a_node_with_headless_claude_off_offers_none_and_refuses_its_ops() {
    let f = Headless::start_with(|config| config.claude.headless = false).await;
    let mut client = f.h.client().await;
    let node = client.until("the node", |c| c.nodes.get(NODE).cloned()).await;
    assert_ne!(node["claude"], json!(true), "{node}");
    let ack = client
        .manage(json!({ "op": "claude.create", "node": NODE, "cwd": f.h.work().display().to_string() }))
        .await;
    assert_eq!(ack["ok"], json!(false), "{ack}");
    assert!(
        ack["message"]
            .as_str()
            .unwrap_or_default()
            .contains("headless = false"),
        "{ack}"
    );
    assert!(f.launches().is_empty());
}

// On a remote host the node's PATH is a service's, without ~/.local/bin, and only the login shell
// finds `claude`; a hand-over that cannot find it must leave the headless child running.
#[tokio::test]
async fn a_claude_only_the_login_shell_finds_starts_and_resumes_and_a_lost_one_hands_over_nothing() {
    let f = Headless::start_with(|config| {
        let home = HOME.get().expect("a home");
        std::fs::write(
            home.join(".profile"),
            format!("PATH=\"{}:$PATH\"; export PATH\n", home.join("fake").display()),
        )
        .unwrap();
        unsafe {
            std::env::set_var("HOME", home);
        }
        config.claude.binary = "fake-claude.py".into();
    })
    .await;
    let mut client = f.h.client().await;
    let pane = f.create(&mut client, None).await;
    pumped(&mut client, "the child starting", || !f.launches().is_empty()).await;

    let id = "55555555-5555-4555-8555-555555555555";
    let project = f.home.join(".claude/projects/-stored");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(format!("{id}.jsonl")),
        json!({"type":"user","cwd": f.h.work().display().to_string(),"message":{"content":"x"}}).to_string()
            + "\n",
    )
    .unwrap();
    let resumed = client
        .manage(json!({ "op": "claude.resume", "node": NODE, "id": id }))
        .await;
    assert_eq!(resumed["ok"], json!(true), "{resumed}");
    pumped(&mut client, "the child resuming it", || {
        f.launches().iter().any(|argv| resumes(argv, id))
    })
    .await;

    std::fs::rename(f.fake().join("fake-claude.py"), f.fake().join("moved.py")).unwrap();
    let handed = client
        .manage(json!({ "op": "claude.to_terminal", "at": pane }))
        .await;
    assert_eq!(handed["ok"], json!(false), "{handed}");
    assert!(
        handed["message"]
            .as_str()
            .unwrap_or_default()
            .contains("fake-claude.py"),
        "{handed}"
    );
    let created = client
        .manage(json!({ "op": "claude.create", "node": NODE, "cwd": f.h.work().display().to_string() }))
        .await;
    assert_eq!(created["ok"], json!(false), "{created}");
    let quiet = Instant::now() + Duration::from_millis(500);
    while Instant::now() < quiet {
        client.pump(Duration::from_millis(100)).await;
    }
    assert!(
        client.panes.contains_key(&pane),
        "the refused hand-over stopped the headless pane"
    );
    assert!(
        !f.launches().iter().any(|argv| argv == &["exited"]),
        "a headless child was stopped: {:?}",
        f.launches()
    );
}
