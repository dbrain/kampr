//! Claude's own hooks reaching the node through `kampr hook` (docs/12 W11): the permission card,
//! its decisions, the live turn and the acknowledgement, and installing the hooks themselves.

use super::*;
use tokio::time::Instant;

const SESSION: &str = "abababab-abab-4bab-8bab-abababababab";

/// A `claude` that publishes its marker and transcript, and runs `kampr hook` for every event
/// the test drops into its box — so each event reaches the node from a process inside the pane,
/// which is what the node routes by. `status` rewrites its marker; `paint` prints to its screen;
/// `exit` ends it; the `--settings` it was started with, wherever in its argv, is left in
/// `settings` before `up` is. A held hook ignores the hangup a closed pane sends its process group, so what
/// lets it go is the node.
fn hooked_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("claude");
    let script = r#"#!/bin/sh
settings=; n=0
while [ $# -gt 0 ]; do
  if [ "$1" = --settings ]; then settings=$2; shift 2; continue; fi
  n=$((n+1)); eval "a$n=\$1"; shift
done
home=$a1; id=$a2; kampr=$a3; sock=$a4; box=$a5
cwd=$(pwd -P)
start=$(cut -d' ' -f22 /proc/$$/stat 2>/dev/null || LC_ALL=C TZ=UTC ps -o lstart= -p $$ | sed 's/^ *//; s/ *$//')
project="$home/.claude/projects/$(printf %s "$cwd" | tr / -)"
mkdir -p "$project" "$home/.claude/sessions" "$box"
now=$(date -u +%Y-%m-%dT%H:%M:%S.999Z)
printf '{"type":"user","uuid":"%s-1","cwd":"%s","timestamp":"%s","message":{"role":"user","content":"hello"}}\n' "$id" "$cwd" "$now" > "$project/$id.jsonl"
printf '{"type":"assistant","uuid":"%s-2","cwd":"%s","timestamp":"%s","message":{"content":[{"type":"text","text":"NATIVE TURN"}]}}\n' "$id" "$cwd" "$now" >> "$project/$id.jsonl"
mark() {
  printf '{"pid":%s,"sessionId":"%s","cwd":"%s","procStart":"%s","version":"2.1.311","kind":"interactive","entrypoint":"cli","status":"%s"}' $$ "$id" "$cwd" "$start" "$1" > "$home/.claude/sessions/$$.json"
}
[ -n "$settings" ] && printf %s "$settings" > "$box/settings"
: > "$box/up"
mark idle
printf 'claude is up\n'
while :; do
  for req in "$box"/*.json; do
    [ -e "$req" ] || continue
    name=${req%.json}
    mv "$req" "$name.event"
    ( trap '' HUP; "$kampr" hook --socket "$sock" < "$name.event" > "$name.out" 2> "$name.err" & echo $! > "$name.pid"; wait $!; echo $? > "$name.code" ) &
  done
  if [ -e "$box/exit" ]; then rm -f "$box/exit"; exit 0; fi
  if [ -e "$box/status" ]; then mv "$box/status" "$box/status.taken"; mark "$(cat "$box/status.taken")"; fi
  for paint in "$box"/*.paint; do
    [ -e "$paint" ] || continue
    cat "$paint"; rm -f "$paint"
  done
  sleep 0.05
done
"#;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Hooked {
    h: Harness,
    client: Client,
    pane: String,
    inbox: PathBuf,
    cwd: String,
    fired: usize,
    painted: std::sync::atomic::AtomicUsize,
}

struct Ran {
    out: String,
    err: String,
    code: String,
}

impl Hooked {
    async fn start() -> Self {
        Self::then(None).await
    }

    async fn then(after: Option<fn(&Path) -> PathBuf>) -> Self {
        let h = Harness::start().await;
        let after = after.map(|write| write(&h.work()));
        let mut client = h.client().await;
        let home = h.home.as_ref().unwrap().path().to_path_buf();
        let inbox = home.join("box");
        let pane = open_pane(&h, &mut client, &inbox, after.as_deref()).await;
        let cwd = h.work().canonicalize().unwrap().display().to_string();
        Self {
            h,
            client,
            pane,
            inbox,
            cwd,
            fired: 0,
            painted: Default::default(),
        }
    }

    async fn another(&mut self) -> (String, PathBuf) {
        let inbox = self.home().join(format!("box{}", self.fired + 100));
        let pane = open_pane(&self.h, &mut self.client, &inbox, None).await;
        (pane, inbox)
    }

    async fn watch(&mut self, conversation: bool) {
        self.client
            .send(json!({ "t": "watch", "pane": self.pane, "conversation": conversation }))
            .await;
        if conversation {
            self.client.reply("convo").await;
        }
    }

    fn fire(&mut self, event: Value) -> String {
        let inbox = self.inbox.clone();
        self.fire_in(&inbox, event)
    }

    fn fire_in(&mut self, inbox: &Path, event: Value) -> String {
        let mut event = event;
        event["session_id"] = json!(SESSION);
        event["cwd"] = json!(self.cwd);
        event["transcript_path"] = json!("/dev/null");
        self.fired += 1;
        let name = format!("e{:03}", self.fired);
        std::fs::create_dir_all(inbox).unwrap();
        let staged = inbox.join(format!("{name}.staged"));
        std::fs::write(&staged, event.to_string()).unwrap();
        std::fs::rename(&staged, inbox.join(format!("{name}.json"))).unwrap();
        name
    }

    async fn ran(&mut self, name: &str) -> Ran {
        let inbox = self.inbox.clone();
        self.ran_in(&inbox, name).await
    }

    async fn ran_in(&mut self, inbox: &Path, name: &str) -> Ran {
        let code = inbox.join(format!("{name}.code"));
        self.client
            .until(&format!("{name} to exit"), |_| {
                std::fs::read_to_string(&code).ok().filter(|c| c.ends_with('\n'))
            })
            .await;
        let read = |ext: &str| std::fs::read_to_string(inbox.join(format!("{name}.{ext}"))).unwrap();
        Ran {
            out: read("out"),
            err: read("err"),
            code: read("code").trim().to_string(),
        }
    }

    fn exited(&self, name: &str) -> bool {
        self.inbox.join(format!("{name}.code")).exists()
    }

    fn pid(&self, name: &str) -> i32 {
        std::fs::read_to_string(self.inbox.join(format!("{name}.pid")))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    // Each is renamed into place, and each paint has a name of its own: a `cat` and an `rm` are a
    // process each, 100-250 ms apiece on a Mac, and a file rewritten in between was read from the
    // previous one's length and then deleted unshown.
    fn status(&self, status: &str) {
        self.drop_in("status", status);
    }

    fn paint(&self, text: &str) {
        let n = self.painted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.drop_in(&format!("{n:04}.paint"), text);
    }

    fn drop_in(&self, name: &str, text: &str) {
        let staged = self.inbox.join(format!("{name}.staged"));
        std::fs::write(&staged, text).unwrap();
        std::fs::rename(&staged, self.inbox.join(name)).unwrap();
    }

    async fn question(&mut self) -> Value {
        let pane = self.pane.clone();
        self.client
            .until("a question", |c| {
                let at = c
                    .others
                    .iter()
                    .position(|v| v["t"] == "pending" && v["pane"] == pane && !v["question"].is_null())?;
                Some(c.others.remove(at))
            })
            .await
    }

    async fn cleared(&mut self) {
        let pane = self.pane.clone();
        self.client
            .until("the question cleared", |c| {
                let at = c
                    .others
                    .iter()
                    .position(|v| v["t"] == "pending" && v["pane"] == pane && v["question"].is_null())?;
                Some(c.others.remove(at))
            })
            .await;
    }

    async fn answer(&mut self, key: &str) {
        self.client
            .send(json!({ "t": "answer", "pane": self.pane, "key": key }))
            .await;
    }

    async fn answer_with(&mut self, key: &str, text: &str) {
        self.client
            .send(json!({ "t": "answer", "pane": self.pane, "key": key, "text": text }))
            .await;
    }

    async fn refused(&mut self, words: &str) {
        let refused = self.client.reply("error").await;
        assert_eq!(refused["code"], "bad_request", "{refused}");
        assert!(
            refused["message"].as_str().is_some_and(|m| m.contains(words)),
            "{refused}"
        );
    }

    async fn settle(&mut self, quiet: Duration) {
        let until = Instant::now() + quiet;
        while let left = until.saturating_duration_since(Instant::now())
            && !left.is_zero()
        {
            self.client.pump(left).await;
        }
    }

    async fn manage(&mut self, op: Value) -> Value {
        self.client.manage(op).await
    }

    async fn hooks_frame(&mut self) -> Value {
        self.client.reply("claude.hooks").await
    }

    fn home(&self) -> PathBuf {
        self.h.home.as_ref().unwrap().path().to_path_buf()
    }
}

async fn open_pane(h: &Harness, client: &mut Client, inbox: &Path, after: Option<&Path>) -> String {
    let home = h.home.as_ref().unwrap().path().to_path_buf();
    let work = h.work();
    let claude = hooked_claude(&work);
    let args = [
        claude.display().to_string(),
        home.display().to_string(),
        SESSION.to_string(),
        kampr_testkit::kampr().display().to_string(),
        socket(&home).display().to_string(),
        inbox.display().to_string(),
    ];
    // Through `env`, so the node does not see a `claude` of its own launching and pass it the
    // hooks in `--settings`: what these panes are offered is the settings files' alone.
    let argv: Vec<String> = match after {
        None => std::iter::once("env".to_string()).chain(args).collect(),
        Some(after) => {
            let quoted: Vec<String> = args.iter().map(|a| format!("'{a}'")).collect();
            let line = format!(
                "{}; exec '{}' '{}' {SESSION}",
                quoted.join(" "),
                after.display(),
                home.display()
            );
            vec!["sh".into(), "-c".into(), line]
        }
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let pane = client
        .create(&argv, json!({ "cwd": work.display().to_string() }))
        .await;
    client
        .pane(&pane, |p| {
            p["agent"] == "claude" && p["has_conversation"] == json!(true)
        })
        .await;
    pane
}

fn socket(home: &Path) -> PathBuf {
    home.join("hooks.sock")
}

fn bash_request(suggestions: Option<Value>) -> Value {
    let mut event = json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": { "command": "echo probe > probe.txt", "description": "Write a probe file" },
        "tool_use_id": "toolu_bash",
    });
    if let Some(suggestions) = suggestions {
        event["permission_suggestions"] = suggestions;
    }
    event
}

fn suggestions() -> Value {
    json!([
        { "type": "addRules", "rules": [ { "toolName": "Bash", "ruleContent": "echo:*" } ],
          "behavior": "allow", "destination": "localSettings" },
        { "type": "setMode", "mode": "acceptEdits", "destination": "session" },
    ])
}

fn decided(decision: Value) -> Value {
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
}

fn reply(ran: &Ran) -> Value {
    assert_eq!(ran.code, "0", "{}", ran.err);
    serde_json::from_str(&ran.out).unwrap_or_else(|_| panic!("a decision, not {:?}", ran.out))
}

#[tokio::test]
async fn a_permission_request_is_a_hook_card_and_each_answer_is_its_decision() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let cases = [
        ("1", decided(json!({ "behavior": "allow" }))),
        (
            "2",
            decided(json!({ "behavior": "allow", "updatedPermissions": suggestions() })),
        ),
        (
            "3",
            decided(json!({ "behavior": "deny", "message": "Declined from Kampr" })),
        ),
    ];
    for (key, want) in cases {
        let name = c.fire(bash_request(Some(suggestions())));
        let card = c.question().await;
        assert_eq!(card["source"], "hook", "{card}");
        assert_eq!(card["header"], "Bash", "{card}");
        assert_eq!(card["question"], "Write a probe file", "{card}");
        assert_eq!(card["context"], "echo probe > probe.txt", "{card}");
        let keys: Vec<&str> = card["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["1", "2", "3"], "{card}");
        assert_eq!(card["options"][0]["label"], "Yes", "{card}");
        let always = card["options"][1]["label"].as_str().unwrap();
        assert!(
            always.starts_with("Yes, and ")
                && always.contains("Bash(echo:*)")
                && always.contains("acceptEdits"),
            "{card}"
        );
        assert_eq!(card["options"][2]["label"], "No", "{card}");
        assert!(
            !c.exited(&name),
            "the hook was answered before anybody pressed anything"
        );
        c.answer(key).await;
        let ran = c.ran(&name).await;
        assert_eq!(reply(&ran), want, "key {key}");
        c.cleared().await;
    }
}

#[tokio::test]
async fn a_tool_with_no_suggestions_offers_no_always_and_a_bare_tool_is_asked_about_by_name() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let name = c.fire(json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "mcp__probe__lookup",
        "tool_input": { "term": "x".repeat(3000) },
        "tool_use_id": "toolu_mcp",
    }));
    let card = c.question().await;
    assert_eq!(card["question"], "Allow mcp__probe__lookup?", "{card}");
    let keys: Vec<&str> = card["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["1", "3"], "{card}");
    let context = card["context"].as_str().unwrap();
    assert!(context.starts_with(r#"{"term":"xxx"#), "{context}");
    assert!(context.chars().count() <= 2001, "{}", context.len());
    let warm = get(
        &c.h.origin,
        &format!("/api/warm?pane={}", c.pane),
        &c.client.token,
    )
    .await;
    assert_eq!(warm["pending"]["source"], "hook", "{warm}");
    assert_eq!(warm["pending"]["question"], "Allow mcp__probe__lookup?", "{warm}");
    c.answer("2").await;
    let refused = c.client.reply("error").await;
    assert_eq!(refused["code"], "bad_request", "{refused}");
    assert!(!c.exited(&name));
    c.answer("1").await;
    assert_eq!(
        reply(&c.ran(&name).await),
        decided(json!({ "behavior": "allow" }))
    );
}

fn questions(multi: bool) -> Value {
    json!([
        { "question": "Which colour?", "header": "Colour", "multiSelect": false, "options": [
            { "label": "Red", "description": "Warm" },
            { "label": "Green", "description": "Calm" },
        ] },
        { "question": "Which shapes?", "header": "Shapes", "multiSelect": multi, "options": [
            { "label": "Circle", "description": "Round" },
            { "label": "Square", "description": "Boxy" },
            { "label": "Star", "description": "Pointy" },
        ] },
    ])
}

fn ask(multi: bool) -> Value {
    json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "AskUserQuestion",
        "tool_input": { "questions": questions(multi) },
        "tool_use_id": "toolu_ask",
    })
}

fn answered(multi: bool, answers: Value) -> Value {
    decided(json!({
        "behavior": "allow",
        "updatedInput": { "questions": questions(multi), "answers": answers },
    }))
}

#[tokio::test]
async fn a_question_is_asked_one_at_a_time_and_answered_with_its_labels() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let name = c.fire(ask(false));
    let first = c.question().await;
    assert_eq!(first["source"], "hook", "{first}");
    assert_eq!(first["question"], "Which colour?", "{first}");
    assert_eq!(first["header"], "Colour · 1 of 2", "{first}");
    assert_eq!(first["options"][1]["label"], "Green", "{first}");
    assert_eq!(first["options"][1]["detail"], "Calm", "{first}");
    assert!(first.get("multi").is_none(), "{first}");
    c.answer("2").await;
    let second = c.question().await;
    assert_eq!(second["question"], "Which shapes?", "{second}");
    assert_eq!(second["header"], "Shapes · 2 of 2", "{second}");
    assert!(!c.exited(&name), "answered before the last question");
    c.answer("3").await;
    assert_eq!(
        reply(&c.ran(&name).await),
        answered(
            false,
            json!({ "Which colour?": "Green", "Which shapes?": "Star" })
        )
    );
}

#[tokio::test]
async fn a_multiple_answer_question_ticks_on_a_press_and_commits_on_submit() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let name = c.fire(ask(true));
    c.question().await;
    c.answer("1").await;
    let second = c.question().await;
    assert_eq!(second["multi"], json!(true), "{second}");
    c.answer("1").await;
    c.answer("3").await;
    let ticked = c
        .client
        .until("both ticks published", |c| {
            c.others
                .iter()
                .rev()
                .find(|v| {
                    v["t"] == "pending"
                        && v["options"][0]["chosen"] == json!(true)
                        && v["options"][2]["chosen"] == json!(true)
                })
                .cloned()
        })
        .await;
    assert!(ticked["options"][1].get("chosen").is_none(), "{ticked}");
    assert!(!c.exited(&name), "a tick answered the question");
    c.client
        .send(json!({ "t": "answer.submit", "pane": c.pane }))
        .await;
    assert_eq!(
        reply(&c.ran(&name).await),
        answered(
            true,
            json!({ "Which colour?": "Red", "Which shapes?": "Circle, Star" })
        )
    );
}

#[tokio::test]
async fn a_held_request_is_let_go_when_its_tool_runs_or_its_hook_goes_away() {
    let mut c = Hooked::start().await;
    c.watch(false).await;

    let held = c.fire(bash_request(None));
    c.question().await;
    let other = c.fire(json!({
        "hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "toolu_other",
        "tool_input": {}, "tool_response": {},
    }));
    assert_eq!(c.ran(&other).await.out, "");
    c.settle(Duration::from_millis(300)).await;
    assert!(!c.exited(&held), "another tool's result let this one go");
    let post = c.fire(json!({
        "hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "toolu_bash",
        "tool_input": {}, "tool_response": {},
    }));
    let ran = c.ran(&held).await;
    assert_eq!((ran.out.as_str(), ran.code.as_str()), ("", "0"));
    assert_eq!(c.ran(&post).await.out, "");
    c.cleared().await;

    let killed = c.fire(bash_request(None));
    c.question().await;
    unsafe {
        libc::kill(c.pid(&killed), libc::SIGKILL);
    }
    c.cleared().await;
}

#[tokio::test]
async fn a_held_request_is_let_go_when_the_session_moves_on() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    for event in ["UserPromptSubmit", "Stop", "StopFailure", "SessionEnd"] {
        let held = c.fire(bash_request(None));
        c.question().await;
        c.fire(json!({ "hook_event_name": event, "prompt": "next" }));
        let ran = c.ran(&held).await;
        assert_eq!(ran.out, "", "{event}");
        c.cleared().await;
    }
}

#[tokio::test]
async fn a_hooked_dialog_is_never_read_off_the_screen_and_leaving_blocked_lets_it_go() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let said = c.fire(json!({ "hook_event_name": "UserPromptSubmit", "prompt": "go" }));
    c.ran(&said).await;

    c.paint("Do you want to proceed?\n\n1) Yes, from the screen\n2) No, from the screen\n");
    let pane = c.pane.clone();
    c.client.shows(&pane, "No, from the screen").await;
    c.status("waiting");
    let held = c.fire(bash_request(None));
    let card = c.question().await;
    assert_eq!(card["source"], "hook", "the screen was read first: {card}");
    c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
    c.settle(Duration::from_millis(1500)).await;
    let read: Vec<_> = c
        .client
        .others
        .iter()
        .filter(|v| v["t"] == "pending" && v["source"] == "screen")
        .collect();
    assert!(
        read.is_empty(),
        "the screen was read under a hook's question: {read:?}"
    );

    c.status("idle");
    let ran = c.ran(&held).await;
    assert_eq!(ran.out, "");
    c.cleared().await;
}

/// A hooked pane that goes blocked waits for its hook before its screen is read, and is read once
/// no hook comes.
#[tokio::test]
async fn a_hooked_pane_blocked_with_no_hook_coming_is_read_off_the_screen_after_the_grace() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let said = c.fire(json!({ "hook_event_name": "UserPromptSubmit", "prompt": "go" }));
    c.ran(&said).await;
    c.paint("Do you want to proceed?\n\n1) Yes, from the screen\n2) No, from the screen\n");
    let pane = c.pane.clone();
    c.client.shows(&pane, "No, from the screen").await;
    c.client.others.clear();
    c.status("waiting");
    c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
    let seen = Instant::now();
    let card = c
        .client
        .until("the screen's question", |c| {
            c.others
                .iter()
                .find(|v| v["t"] == "pending" && v["pane"] == pane && !v["question"].is_null())
                .cloned()
        })
        .await;
    assert_eq!(card["source"], "screen", "{card}");
    assert!(
        seen.elapsed() >= Duration::from_millis(1000),
        "read {:?} after the pane was seen blocked, before a hook had its chance",
        seen.elapsed()
    );
}

#[tokio::test]
async fn message_display_is_the_live_turn_until_the_transcript_has_it() {
    let mut c = Hooked::start().await;
    c.watch(true).await;
    let first = c.fire(json!({
        "hook_event_name": "MessageDisplay", "turn_id": "t1", "message_id": "m1", "index": 0,
        "final": false, "delta": "Hooked first paragraph.",
    }));
    let started = Instant::now();
    let ran = c.ran(&first).await;
    assert_eq!(ran.out, "");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    c.fire(json!({
        "hook_event_name": "MessageDisplay", "turn_id": "t1", "message_id": "m1", "index": 1,
        "final": true, "delta": "Hooked second paragraph.",
    }));
    let pane = c.pane.clone();
    let live = c
        .client
        .until("both paragraphs live", |c| {
            c.others
                .iter()
                .find(|v| {
                    v["t"] == "convo.turn"
                        && v["pane"] == pane
                        && v["turns"][0]["id"] == "live"
                        && v["turns"].to_string().contains("Hooked second paragraph.")
                })
                .cloned()
        })
        .await;
    let text = live["turns"][0]["blocks"][0]["text"].as_str().unwrap();
    assert_eq!(text, "Hooked first paragraph.\n\nHooked second paragraph.");

    let project = c.home().join(".claude/projects").join(c.cwd.replace('/', "-"));
    let record = json!({
        "type": "assistant", "uuid": "rec-1", "cwd": c.cwd, "sessionId": SESSION,
        "timestamp": time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap(),
        "message": { "content": [ { "type": "text", "text": "Hooked first paragraph.\n\nHooked second paragraph." } ] },
    });
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(project.join(format!("{SESSION}.jsonl")))
        .unwrap();
    std::io::Write::write_all(&mut file, format!("{record}\n").as_bytes()).unwrap();
    c.client
        .until("the live turn retired", |c| {
            c.others
                .iter()
                .find(|v| {
                    v["t"] == "convo.turn"
                        && v["turns"].as_array().is_some_and(|t| {
                            t.iter().any(|t| {
                                t["id"] == "live" && t["blocks"].as_array().is_none_or(|b| b.is_empty())
                            })
                        })
                })
                .cloned()
        })
        .await;
    c.client.others.clear();
    c.settle(Duration::from_millis(800)).await;
    let back: Vec<_> = c
        .client
        .others
        .iter()
        .filter(|v| {
            v["t"] == "convo.turn"
                && v["turns"].as_array().is_some_and(|t| {
                    t.iter()
                        .any(|t| t["id"] == "live" && t["blocks"].as_array().is_some_and(|b| !b.is_empty()))
                })
        })
        .collect();
    assert!(
        back.is_empty(),
        "the live turn came back after its record: {back:?}"
    );
}

#[tokio::test]
async fn a_submitted_prompt_is_heard_by_whoever_watches_the_conversation() {
    let mut c = Hooked::start().await;
    c.watch(true).await;
    c.fire(json!({ "hook_event_name": "UserPromptSubmit", "prompt": "exactly what was typed", "prompt_id": "p1" }));
    let heard = c.client.reply("convo.heard").await;
    assert_eq!(
        heard,
        json!({ "t": "convo.heard", "pane": c.pane, "text": "exactly what was typed" })
    );
}

/// A Claude that has been started and not yet asked anything has no transcript, and that is
/// exactly when the operator opens it and should be offered the hooks.
#[tokio::test]
async fn a_claude_with_nothing_written_yet_is_offered_the_hooks_when_its_conversation_opens() {
    let h = Harness::start().await;
    let home = h.home.as_ref().unwrap().path().to_path_buf();
    let claude = unwritten_claude(&h.work());
    let mut client = h.client().await;
    let pane = client
        .create(
            &[
                "env",
                &claude.display().to_string(),
                &home.display().to_string(),
                SESSION,
            ],
            json!({ "cwd": h.work().display().to_string() }),
        )
        .await;
    client.pane(&pane, |p| p["agent"] == "claude").await;
    client
        .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
        .await;
    let offered = client.reply("claude.hooks").await;
    let cwd = h.work().canonicalize().unwrap().display().to_string();
    assert_eq!(
        offered,
        json!({ "t": "claude.hooks", "pane": pane, "state": "absent", "ask": true, "project": cwd }),
    );
}

async fn hook_from_outside(socket: &Path, event: &Value) -> (std::process::Output, Duration) {
    use tokio::io::AsyncWriteExt;
    let started = Instant::now();
    let mut child = tokio::process::Command::new(kampr_testkit::kampr())
        .args(["hook", "--socket"])
        .arg(socket)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(event.to_string().as_bytes()).await.unwrap();
    drop(stdin);
    let out = tokio::time::timeout(SOON, child.wait_with_output())
        .await
        .expect("the hook returned")
        .unwrap();
    (out, started.elapsed())
}

#[tokio::test]
async fn an_event_from_a_claude_no_pane_hosts_is_answered_with_nothing_at_once() {
    let h = Harness::start().await;
    let socket = socket(h.home.as_ref().unwrap().path());
    let (out, took) = hook_from_outside(
        &socket,
        &json!({ "hook_event_name": "PermissionRequest", "session_id": "nobody", "tool_name": "Bash",
                 "tool_input": { "command": "true" }, "tool_use_id": "t" }),
    )
    .await;
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[tokio::test]
async fn kampr_hook_with_no_node_behind_its_socket_exits_zero_saying_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for socket in [dir.path().join("absent.sock"), dir.path().to_path_buf()] {
        let (out, _) = hook_from_outside(&socket, &json!({ "hook_event_name": "Stop" })).await;
        assert!(out.status.success(), "{out:?}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    }
}

fn command(home: &Path) -> String {
    let exe = kampr_keep::own_exe().unwrap().display().to_string();
    let sock = socket(home).display().to_string();
    format!("[ -x '{exe}' ] && '{exe}' hook --socket '{sock}' 2>/dev/null; exit 0")
}

fn exec_form(home: &Path) -> String {
    let exe = kampr_keep::own_exe().unwrap().display().to_string();
    let sock = socket(home).display().to_string();
    format!("[ -x '{exe}' ] && exec '{exe}' hook --socket '{sock}'; exit 0")
}

fn installed(home: &Path) -> Value {
    let run = |timeout: Option<u64>| {
        let mut hook = json!({ "type": "command", "command": command(home) });
        if let Some(timeout) = timeout {
            hook["timeout"] = json!(timeout);
        }
        hook
    };
    let tool = |timeout: Option<u64>| json!([ { "matcher": "*", "hooks": [ run(timeout) ] } ]);
    let any = |timeout: Option<u64>| json!([ { "hooks": [ run(timeout) ] } ]);
    json!({
        "PermissionRequest": tool(Some(3600)),
        "PostToolUse": tool(None),
        "PostToolUseFailure": tool(None),
        "PermissionDenied": tool(None),
        "UserPromptSubmit": any(Some(5)),
        "MessageDisplay": any(Some(5)),
        "Elicitation": any(Some(5)),
        "ElicitationResult": any(Some(5)),
        "Stop": any(Some(10)),
        "StopFailure": any(Some(10)),
        "SessionEnd": any(Some(10)),
    })
}

fn keys(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut current = String::new();
    while let Some(ch) = chars.next() {
        match (in_string, ch) {
            (true, '\\') => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (true, '"') => {
                in_string = false;
                let rest: String = chars
                    .clone()
                    .take_while(|c| c.is_whitespace() || *c == ':')
                    .collect();
                if depth == 1 && rest.contains(':') {
                    out.push(current.clone());
                }
            }
            (true, _) => current.push(ch),
            (false, '"') => {
                in_string = true;
                current.clear();
            }
            (false, '{') | (false, '[') => depth += 1,
            (false, '}') | (false, ']') => depth -= 1,
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn installing_for_the_user_writes_the_hooks_keeps_the_operators_keys_and_clears_the_project() {
    let mut c = Hooked::start().await;
    let home = c.home();
    let user = home.join(".claude/settings.json");
    std::fs::write(
        &user,
        r#"{"zeta": 1, "model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}, "alpha": [3, 1.5]}"#,
    )
    .unwrap();
    let project = Path::new(&c.cwd).join(".claude/settings.local.json");
    std::fs::create_dir_all(project.parent().unwrap()).unwrap();
    let old = json!({ "permissions": { "allow": ["Bash(ls)"] }, "hooks": {
        "Stop": [ { "hooks": [ { "type": "command", "command": "[ -x '/old/kampr' ] && exec '/old/kampr' hook --socket '/old.sock'; exit 0" } ] } ],
    } });
    std::fs::write(&project, old.to_string()).unwrap();

    c.watch(true).await;
    let before = c.hooks_frame().await;
    assert_eq!(
        before,
        json!({ "t": "claude.hooks", "pane": c.pane, "state": "stale", "scope": "project", "ask": true, "project": c.cwd }),
    );

    let ack = c
        .manage(json!({ "op": "claude.hooks.install", "at": c.pane, "mode": "user" }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let after = c.hooks_frame().await;
    assert_eq!(after["state"], "current", "{after}");
    assert_eq!(after["scope"], "user", "{after}");

    let text = std::fs::read_to_string(&user).unwrap();
    assert_eq!(keys(&text), ["zeta", "model", "hooks", "alpha"], "{text}");
    let written: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written["zeta"], json!(1));
    assert_eq!(written["alpha"], json!([3, 1.5]));
    let mut want = installed(&home);
    want["Stop"] = json!([
        { "hooks": [ { "type": "command", "command": "say done" } ] },
        want["Stop"][0].clone(),
    ]);
    assert_eq!(written["hooks"], want, "{text}");

    let cleared: Value = serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(cleared, json!({ "permissions": { "allow": ["Bash(ls)"] } }));

    let again = c
        .manage(json!({ "op": "claude.hooks.install", "at": c.pane, "mode": "user" }))
        .await;
    assert_eq!(again["ok"], json!(true), "{again}");
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        text,
        "a second install changed the file"
    );
}

#[tokio::test]
async fn installing_for_the_project_writes_the_local_file_and_takes_the_users_entries_out() {
    let mut c = Hooked::start().await;
    let home = c.home();
    let user = home.join(".claude/settings.json");
    std::fs::write(&user, json!({ "hooks": installed(&home) }).to_string()).unwrap();
    c.watch(true).await;
    let before = c.hooks_frame().await;
    assert_eq!(
        (before["state"].as_str(), before["scope"].as_str()),
        (Some("current"), Some("user"))
    );

    let ack = c
        .manage(json!({ "op": "claude.hooks.install", "at": c.pane, "mode": "project" }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let after = c.hooks_frame().await;
    assert_eq!(
        (after["state"].as_str(), after["scope"].as_str()),
        (Some("current"), Some("project"))
    );
    let project: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&c.cwd).join(".claude/settings.local.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(project, json!({ "hooks": installed(&home) }));
    let user: Value = serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
    assert_eq!(user, json!({}));
}

#[tokio::test]
async fn a_settings_file_that_is_not_json_is_refused_in_words_and_left_alone() {
    let mut c = Hooked::start().await;
    let user = c.home().join(".claude/settings.json");
    std::fs::write(&user, "{ \"model\": \"opus\", // a comment\n}").unwrap();
    c.watch(true).await;
    assert_eq!(c.hooks_frame().await["state"], "absent");
    let ack = c
        .manage(json!({ "op": "claude.hooks.install", "at": c.pane, "mode": "user" }))
        .await;
    assert_eq!(ack["ok"], json!(false), "{ack}");
    assert!(
        ack["message"]
            .as_str()
            .is_some_and(|m| m.contains("settings.json") && m.contains("JSON")),
        "{ack}"
    );
    assert_eq!(
        std::fs::read_to_string(&user).unwrap(),
        "{ \"model\": \"opus\", // a comment\n}"
    );
}

#[tokio::test]
async fn declining_is_remembered_across_a_restart() {
    let mut c = Hooked::start().await;
    c.watch(true).await;
    assert_eq!(c.hooks_frame().await["ask"], json!(true));
    let ack = c
        .manage(json!({ "op": "claude.hooks.decline", "at": c.pane }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let after = c.hooks_frame().await;
    assert_eq!(
        (after["ask"].clone(), after["state"].clone()),
        (json!(false), json!("absent"))
    );

    let pane = c.pane.clone();
    let h = c.h.restart().await;
    let mut client = h.client().await;
    client.pane(&pane, |p| p["has_conversation"] == json!(true)).await;
    client
        .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
        .await;
    assert_eq!(client.reply("claude.hooks").await["ask"], json!(false));
}

fn request(tool_use_id: &str, command: &str) -> Value {
    json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "tool_use_id": tool_use_id,
    })
}

#[tokio::test]
async fn requests_on_one_pane_queue_and_a_press_for_a_question_already_gone_decides_nothing() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let first = c.fire(request("toolu_a", "echo first"));
    let card = c.question().await;
    assert_eq!(card["id"], "toolu_a", "{card}");
    let second = c.fire(request("toolu_b", "echo second"));
    c.settle(Duration::from_millis(500)).await;
    assert!(!c.exited(&first), "the second request let the first one go");
    let pane = c.pane.clone();
    let shown: Vec<_> = c
        .client
        .others
        .iter()
        .filter(|v| v["t"] == "pending" && v["pane"] == pane && v["id"] == "toolu_b")
        .collect();
    assert!(
        shown.is_empty(),
        "the second request replaced the one on screen: {shown:?}"
    );

    c.client
        .send(json!({ "t": "answer", "pane": c.pane, "key": "1", "id": "toolu_b" }))
        .await;
    let refused = c.client.reply("error").await;
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("already been answered")),
        "{refused}"
    );
    c.settle(Duration::from_millis(300)).await;
    assert!(
        !c.exited(&first) && !c.exited(&second),
        "a stale press decided something"
    );

    c.client
        .send(json!({ "t": "answer", "pane": c.pane, "key": "3", "id": "toolu_a" }))
        .await;
    assert_eq!(
        reply(&c.ran(&first).await),
        decided(json!({ "behavior": "deny", "message": "Declined from Kampr" }))
    );
    let next = c.question().await;
    assert_eq!(
        (next["id"].as_str(), next["context"].as_str()),
        (Some("toolu_b"), Some("echo second"))
    );
    assert!(!c.exited(&second));
    c.answer("1").await;
    assert_eq!(
        reply(&c.ran(&second).await),
        decided(json!({ "behavior": "allow" }))
    );
}

#[tokio::test]
async fn each_question_of_a_question_tool_has_its_own_id() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let name = c.fire(ask(false));
    let first = c.question().await;
    assert_eq!(first["id"], "toolu_ask#0", "{first}");
    c.client
        .send(json!({ "t": "answer", "pane": c.pane, "key": "1", "id": "toolu_ask#0" }))
        .await;
    let second = c.question().await;
    assert_eq!(second["id"], "toolu_ask#1", "{second}");
    c.client
        .send(json!({ "t": "answer", "pane": c.pane, "key": "1", "id": "toolu_ask#0" }))
        .await;
    c.client.reply("error").await;
    c.client
        .send(json!({ "t": "answer", "pane": c.pane, "key": "2", "id": "toolu_ask#1" }))
        .await;
    assert_eq!(
        reply(&c.ran(&name).await),
        answered(
            false,
            json!({ "Which colour?": "Red", "Which shapes?": "Square" })
        )
    );
}

#[tokio::test]
async fn a_dialog_still_standing_when_its_hook_goes_is_read_off_the_screen_again() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    c.paint("Do you want to proceed?\n\n1) Yes, from the screen\n2) No, from the screen\n");
    c.status("waiting");
    let pane = c.pane.clone();
    c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
    let held = c.fire(bash_request(None));
    c.client
        .until("the hook's card", |c| {
            c.others
                .iter()
                .any(|v| v["t"] == "pending" && v["source"] == "hook" && !v["question"].is_null())
                .then_some(())
        })
        .await;
    c.client.others.clear();
    unsafe {
        libc::kill(c.pid(&held), libc::SIGKILL);
    }
    let read = c
        .client
        .until("the screen's question again", |c| {
            c.others
                .iter()
                .find(|v| v["t"] == "pending" && v["source"] == "screen" && !v["question"].is_null())
                .cloned()
        })
        .await;
    assert!(
        read["question"]
            .as_str()
            .is_some_and(|q| q.ends_with("Do you want to proceed?")),
        "{read}"
    );
}

#[tokio::test]
async fn a_finished_message_already_in_the_transcript_is_not_live_when_the_conversation_opens() {
    let mut c = Hooked::start().await;
    let shown = c.fire(json!({
        "hook_event_name": "MessageDisplay", "turn_id": "t1", "message_id": "m1", "index": 0,
        "final": true, "delta": "NATIVE TURN",
    }));
    c.ran(&shown).await;
    c.watch(true).await;
    c.settle(Duration::from_millis(1000)).await;
    let live: Vec<_> = c
        .client
        .others
        .iter()
        .filter(|v| v["t"] == "convo.turn" && v["turns"].to_string().contains("\"id\":\"live\""))
        .collect();
    assert!(
        live.is_empty(),
        "a recorded message was drawn again as live: {live:?}"
    );
}

#[tokio::test]
async fn a_cached_session_does_not_route_a_resumed_claude_in_another_pane() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let said = c.fire(json!({ "hook_event_name": "UserPromptSubmit", "prompt": "in a" }));
    c.ran(&said).await;
    let (other, inbox) = c.another().await;
    c.client
        .send(json!({ "t": "watch", "pane": other, "conversation": false }))
        .await;
    c.fire_in(&inbox, bash_request(None));
    let card = c
        .client
        .until("a card", |c| {
            c.others
                .iter()
                .find(|v| v["t"] == "pending" && !v["question"].is_null())
                .cloned()
        })
        .await;
    assert_eq!(card["pane"], json!(other), "routed by the session id: {card}");
}

/// A `claude` that never ran a hook: its finished message on the screen is all there is of it.
fn plain_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = dir.join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("claude");
    let script = r#"#!/bin/sh
home=$1; id=$2
cwd=$(pwd -P)
start=$(cut -d' ' -f22 /proc/$$/stat 2>/dev/null || LC_ALL=C TZ=UTC ps -o lstart= -p $$ | sed 's/^ *//; s/ *$//')
printf '{"pid":%s,"sessionId":"%s","cwd":"%s","procStart":"%s","version":"2.1.311","kind":"interactive","entrypoint":"cli","status":"waiting"}' $$ "$id" "$cwd" "$start" > "$home/.claude/sessions/$$.json"
printf '\033[2J\033[H● Plain screen message\n\n\342\235\257 \n'
while :; do sleep 1; done
"#;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn a_pane_is_hooked_only_while_the_claude_that_sent_the_hooks_runs() {
    let mut c = Hooked::then(Some(plain_claude)).await;
    let said = c.fire(json!({ "hook_event_name": "UserPromptSubmit", "prompt": "go" }));
    c.ran(&said).await;
    std::fs::write(c.inbox.join("exit"), "").unwrap();
    c.watch(true).await;
    c.client
        .until_within("the screen's live preview", Duration::from_secs(20), |c| {
            c.others
                .iter()
                .find(|v| {
                    v["t"] == "convo.turn"
                        && v["turns"].as_array().is_some_and(|t| {
                            t.iter()
                                .any(|t| t["id"] == "live" && t.to_string().contains("Plain screen message"))
                        })
                })
                .cloned()
        })
        .await;
}

#[tokio::test]
async fn a_request_on_a_pane_already_blocked_is_let_go_when_it_goes_idle() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    c.status("waiting");
    let pane = c.pane.clone();
    c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
    let held = c.fire(bash_request(None));
    c.question().await;
    c.status("idle");
    let ran = c.ran(&held).await;
    assert_eq!((ran.out.as_str(), ran.code.as_str()), ("", "0"));
    c.cleared().await;
}

#[tokio::test]
async fn a_request_is_let_go_when_its_pane_goes_away() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let held = c.fire(bash_request(None));
    c.question().await;
    let ack = c.manage(json!({ "op": "native.close", "at": c.pane })).await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let ran = c.ran(&held).await;
    assert_eq!((ran.out.as_str(), ran.code.as_str()), ("", "0"), "{}", ran.err);
}

#[tokio::test]
async fn an_install_of_the_old_exec_form_is_stale_and_updating_replaces_it() {
    let mut c = Hooked::start().await;
    let home = c.home();
    let mut old = installed(&home);
    for groups in old.as_object_mut().unwrap().values_mut() {
        for group in groups.as_array_mut().unwrap() {
            group["hooks"][0]["command"] = json!(exec_form(&home));
        }
    }
    let user = home.join(".claude/settings.json");
    std::fs::write(&user, json!({ "hooks": old }).to_string()).unwrap();
    c.watch(true).await;
    let before = c.hooks_frame().await;
    assert_eq!(
        (before["state"].as_str(), before["scope"].as_str()),
        (Some("stale"), Some("user"))
    );
    let ack = c
        .manage(json!({ "op": "claude.hooks.install", "at": c.pane, "mode": "user" }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    assert_eq!(c.hooks_frame().await["state"], "current");
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
    assert_eq!(written, json!({ "hooks": installed(&home) }));
}

#[tokio::test]
async fn a_settings_file_that_is_a_link_stays_a_link() {
    let mut c = Hooked::start().await;
    let home = c.home();
    let elsewhere = home.join("dotfiles");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let real = elsewhere.join("claude-settings.json");
    std::fs::write(&real, r#"{"model": "opus"}"#).unwrap();
    let user = home.join(".claude/settings.json");
    std::os::unix::fs::symlink(&real, &user).unwrap();
    c.watch(true).await;
    c.hooks_frame().await;
    let ack = c
        .manage(json!({ "op": "claude.hooks.install", "at": c.pane, "mode": "user" }))
        .await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    assert!(
        std::fs::symlink_metadata(&user).unwrap().file_type().is_symlink(),
        "the link was replaced"
    );
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
    assert_eq!(written, json!({ "model": "opus", "hooks": installed(&home) }));
}

fn plan_request(plan: &str) -> Value {
    json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "ExitPlanMode",
        "tool_input": { "plan": plan, "planFilePath": "/home/x/.claude/plans/p.md" },
        "tool_use_id": "toolu_plan",
    })
}

fn set_mode(mode: &str) -> Value {
    json!([ { "type": "setMode", "mode": mode, "destination": "session" } ])
}

/// A bare `allow` leaves plan mode's dialog standing (#635): the plan is approved with its own
/// input and the mode to leave it for, and sent back with the words that say what to change.
#[tokio::test]
async fn a_plan_is_approved_with_its_own_input_and_a_mode_or_sent_back_with_words() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let plan = format!("# Plan\n\n{}", "Write the file, then test it. ".repeat(300));
    let input = plan_request(&plan)["tool_input"].clone();
    let approved = |mode: &str| {
        decided(json!({ "behavior": "allow", "updatedInput": input, "updatedPermissions": set_mode(mode) }))
    };
    let cases = [
        ("1", None, approved("acceptEdits")),
        ("2", None, approved("default")),
        (
            "3",
            Some("say goodbye instead"),
            decided(json!({ "behavior": "deny", "message": "say goodbye instead" })),
        ),
        (
            "3",
            Some("  "),
            decided(json!({ "behavior": "deny", "message": "Declined from Kampr" })),
        ),
        (
            "3",
            None,
            decided(json!({ "behavior": "deny", "message": "Declined from Kampr" })),
        ),
    ];
    for (key, text, want) in cases {
        let name = c.fire(plan_request(&plan));
        let card = c.question().await;
        assert_eq!(card["header"], "Plan", "{card}");
        assert_eq!(card["question"], "Approve Claude's plan?", "{card}");
        assert_eq!(card["context"], json!(plan), "the plan was cut short");
        let options: Vec<(&str, &str, bool)> = card["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o["key"].as_str().unwrap(),
                    o["label"].as_str().unwrap(),
                    o["text"] == json!(true),
                )
            })
            .collect();
        assert_eq!(
            options,
            [
                ("1", "Yes, auto-accept edits", false),
                ("2", "Yes, manually approve edits", false),
                ("3", "No, tell Claude what to change", true),
            ],
            "{card}"
        );
        match text {
            Some(text) => c.answer_with(key, text).await,
            None => c.answer(key).await,
        }
        assert_eq!(reply(&c.ran(&name).await), want, "key {key} text {text:?}");
        c.cleared().await;
    }

    let name = c.fire(plan_request("Short plan"));
    c.question().await;
    c.answer_with("1", "words on a press").await;
    c.refused("takes no words").await;
    assert!(!c.exited(&name), "a press with words on it decided the plan");
    c.answer("1").await;
    assert_eq!(
        reply(&c.ran(&name).await)["hookSpecificOutput"]["decision"]["behavior"],
        "allow"
    );
}

/// The harness's own "Type something" (#639): a last option per question, answered with words.
#[tokio::test]
async fn each_question_ends_with_an_option_answered_in_typed_words() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    let name = c.fire(ask(true));
    let first = c.question().await;
    let last = first["options"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        last,
        json!({ "key": "3", "label": "Type something", "text": true }),
        "{first}"
    );
    assert!(
        first["options"].as_array().unwrap()[..2]
            .iter()
            .all(|o| o.get("text").is_none()),
        "{first}"
    );
    c.answer("3").await;
    c.refused("needs words").await;
    c.answer_with("3", "   ").await;
    c.refused("needs words").await;
    c.answer_with("1", "Mauve").await;
    c.refused("takes no words").await;
    c.answer_with("3", "Mauve, mostly").await;

    let second = c.question().await;
    assert_eq!(second["question"], "Which shapes?", "{second}");
    assert_eq!(second["options"][3]["key"], "4", "{second}");
    c.answer("2").await;
    c.client
        .until("the tick published", |c| {
            c.others
                .iter()
                .find(|v| v["t"] == "pending" && v["options"][1]["chosen"] == json!(true))
                .cloned()
        })
        .await;
    assert!(!c.exited(&name));
    c.answer_with("4", "Hexagon").await;
    // A multiple-answer question records several as one string joined with ", " (#634).
    assert_eq!(
        reply(&c.ran(&name).await),
        answered(
            true,
            json!({ "Which colour?": "Mauve, mostly", "Which shapes?": "Square, Hexagon" })
        )
    );
}

fn subagent(event: Value) -> Value {
    let mut event = event;
    event["agent_id"] = json!("a1b2c3");
    event["agent_type"] = json!("general-purpose");
    event
}

/// A subagent's dialog is not drawn on the main screen (#636), so its request outlives the main
/// agent's dialog closing, and its card says whose it is.
#[tokio::test]
async fn a_subagents_request_says_whose_it_is_and_outlasts_the_main_agents_dialog() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    c.status("waiting");
    let pane = c.pane.clone();
    c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
    let main = c.fire(request("toolu_main", "echo main"));
    let card = c.question().await;
    assert_eq!(
        (card["id"].as_str(), card["header"].as_str()),
        (Some("toolu_main"), Some("Bash"))
    );
    let theirs = c.fire(subagent(request("toolu_sub", "date > when.txt")));
    c.settle(Duration::from_millis(300)).await;
    assert!(!c.exited(&main) && !c.exited(&theirs));

    c.answer("1").await;
    assert_eq!(
        reply(&c.ran(&main).await),
        decided(json!({ "behavior": "allow" }))
    );
    let next = c.question().await;
    assert_eq!(next["id"], "toolu_sub", "{next}");
    assert_eq!(next["header"], "Bash · general-purpose subagent", "{next}");
    assert_eq!(next["context"], "date > when.txt", "{next}");

    c.status("idle");
    c.client.pane(&pane, |p| p["agent_status"] != "blocked").await;
    c.settle(Duration::from_millis(500)).await;
    assert!(
        !c.exited(&theirs),
        "the main dialog closing let the subagent's request go"
    );
    c.answer("1").await;
    assert_eq!(
        reply(&c.ran(&theirs).await),
        decided(json!({ "behavior": "allow" }))
    );
    c.cleared().await;

    let asked = c.fire(subagent(ask(false)));
    let card = c.question().await;
    assert_eq!(
        card["header"], "Colour · 1 of 2 · general-purpose subagent",
        "{card}"
    );
    c.fire(json!({ "hook_event_name": "Stop" }));
    assert_eq!(c.ran(&asked).await.out, "");
}

fn elicitation() -> Value {
    json!({
        "hook_event_name": "Elicitation",
        "mcp_server_name": "probe",
        "message": "Pick a colour and a count",
        "mode": "form",
        "requested_schema": {
            "type": "object",
            "properties": {
                "colour": { "type": "string", "title": "Colour" },
                "count": { "type": "integer" },
            },
            "required": ["colour"],
        },
    })
}

/// Claude draws no form until its `Elicitation` hook returns (#637), so the node answers it with
/// nothing at once (#638) and shows the request as one to answer at the terminal.
#[tokio::test]
async fn an_elicitation_is_never_held_and_is_shown_as_answered_at_the_terminal() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    for end in [
        json!({ "hook_event_name": "ElicitationResult", "mcp_server_name": "probe", "action": "cancel" }),
        json!({ "hook_event_name": "ElicitationResult", "mcp_server_name": "probe", "action": "accept",
                "content": { "colour": "red" } }),
        json!({ "hook_event_name": "Stop" }),
    ] {
        let pane = c.pane.clone();
        c.status("waiting");
        c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
        let started = Instant::now();
        let name = c.fire(elicitation());
        let ran = c.ran(&name).await;
        assert_eq!((ran.out.as_str(), ran.code.as_str()), ("", "0"), "{}", ran.err);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        let card = c.question().await;
        assert_eq!(card["source"], "hook", "{card}");
        assert_eq!(card["header"], "probe · MCP", "{card}");
        assert_eq!(card["question"], "Pick a colour and a count", "{card}");
        assert!(card.get("options").is_none_or(|o| o == &json!([])), "{card}");
        assert_eq!(
            card["context"], "Colour (colour) · string · required\ncount · integer",
            "{card}"
        );

        c.answer("1").await;
        c.refused("at the terminal").await;
        c.settle(Duration::from_millis(1200)).await;
        let read: Vec<_> = c
            .client
            .others
            .iter()
            .filter(|v| v["t"] == "pending" && v["source"] == "screen")
            .collect();
        assert!(read.is_empty(), "the screen was read under the form: {read:?}");

        // Its end is told by ElicitationResult, Esc included (#638), not by the pane's status.
        c.status("idle");
        c.client.pane(&pane, |p| p["agent_status"] != "blocked").await;
        c.settle(Duration::from_millis(500)).await;
        let cleared: Vec<_> = c
            .client
            .others
            .iter()
            .filter(|v| v["t"] == "pending" && v["question"].is_null())
            .collect();
        assert!(
            cleared.is_empty(),
            "the pane's status cleared the form: {cleared:?}"
        );

        c.fire(end.clone());
        c.cleared().await;
    }
}

#[tokio::test]
async fn an_install_from_before_elicitation_reads_stale() {
    let mut c = Hooked::start().await;
    let home = c.home();
    let mut old = installed(&home);
    let hooks = old.as_object_mut().unwrap();
    hooks.remove("Elicitation");
    hooks.remove("ElicitationResult");
    std::fs::write(
        home.join(".claude/settings.json"),
        json!({ "hooks": old }).to_string(),
    )
    .unwrap();
    c.watch(true).await;
    let frame = c.hooks_frame().await;
    assert_eq!(
        (frame["state"].as_str(), frame["scope"].as_str()),
        (Some("stale"), Some("user"))
    );
}

fn with_claude_on_path(config: &mut Config) {
    let bin = Path::new(&config.journals.home).join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    hooked_claude(&bin);
    config.fleet.path = format!("{}:/usr/bin:/bin", bin.display());
}

impl Hooked {
    async fn launched() -> Self {
        let h = Harness::start_with(Harness::home(), with_claude_on_path).await;
        let client = h.client().await;
        let inbox = h.home.as_ref().unwrap().path().join("box");
        let cwd = h.work().canonicalize().unwrap().display().to_string();
        Self {
            h,
            client,
            pane: String::new(),
            inbox,
            cwd,
            fired: 0,
            painted: Default::default(),
        }
    }

    fn claude_args(&self, inbox: &Path) -> Vec<String> {
        let home = self.home();
        vec![
            home.display().to_string(),
            SESSION.to_string(),
            kampr_testkit::kampr().display().to_string(),
            socket(&home).display().to_string(),
            inbox.display().to_string(),
        ]
    }

    async fn up(&mut self, op: Value) -> String {
        let ack = self.manage(op).await;
        assert_eq!(ack["ok"], json!(true), "{ack}");
        let pane = ack["id"].as_str().unwrap().to_string();
        self.client
            .pane(&pane, |p| {
                p["agent"] == "claude" && p["has_conversation"] == json!(true)
            })
            .await;
        pane
    }
}

async fn settings_in(c: &mut Hooked, inbox: &Path) -> Option<Value> {
    let up = inbox.join("up");
    c.client
        .until("the claude up", |_| up.exists().then_some(()))
        .await;
    let text = std::fs::read_to_string(inbox.join("settings")).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|_| panic!("JSON settings, not {text}")))
}

/// A Claude the node starts carries the installed hooks in `--settings`, byte for byte the same
/// commands, so one that also has them installed runs each once (#641), and is not offered the
/// install. herdr hands `agent.start`'s args over verbatim (#642).
#[tokio::test]
async fn a_claude_the_node_starts_carries_the_hooks_and_is_not_offered_the_install() {
    let mut c = Hooked::launched().await;
    let home = c.home();
    let want = json!({ "hooks": installed(&home) });
    let work = c.h.work().display().to_string();
    let bin = home.join("bin").join("claude");

    let inbox = home.join("box-agent");
    let args = c.claude_args(&inbox);
    let agent = c
        .up(json!({ "op": "agent.start", "kind": "claude", "provider": "native", "cwd": work, "args": args }))
        .await;
    assert_eq!(
        settings_in(&mut c, &inbox).await,
        Some(want.clone()),
        "agent.start"
    );

    let inbox = home.join("box-argv");
    let mut argv = vec![bin.display().to_string()];
    argv.extend(c.claude_args(&inbox));
    c.up(json!({ "op": "native.create", "node": NODE, "args": argv, "cwd": work }))
        .await;
    assert_eq!(
        settings_in(&mut c, &inbox).await,
        Some(want.clone()),
        "native.create args"
    );

    let inbox = home.join("box-line");
    let line = format!("{} {} ; true", bin.display(), c.claude_args(&inbox).join(" "));
    c.up(json!({ "op": "native.create", "node": NODE, "command": line, "cwd": work }))
        .await;
    assert_eq!(
        settings_in(&mut c, &inbox).await,
        Some(want),
        "native.create command"
    );

    c.pane = agent;
    c.watch(true).await;
    assert_eq!(
        c.hooks_frame().await,
        json!({ "t": "claude.hooks", "pane": c.pane, "state": "current", "ask": true, "project": c.cwd }),
    );

    c.inbox = home.join("box-agent");
    let name = c.fire(bash_request(None));
    c.question().await;
    c.answer("1").await;
    assert_eq!(
        reply(&c.ran(&name).await),
        decided(json!({ "behavior": "allow" }))
    );
}

#[tokio::test]
async fn a_claude_started_after_never_asking_again_has_no_hooks_put_on_it() {
    let mut c = Hooked::launched().await;
    let ack = c.manage(json!({ "op": "claude.hooks.decline" })).await;
    assert_eq!(ack["ok"], json!(true), "{ack}");
    let home = c.home();
    let inbox = home.join("box-agent");
    let args = c.claude_args(&inbox);
    let work = c.h.work().display().to_string();
    c.pane = c
        .up(json!({ "op": "agent.start", "kind": "claude", "provider": "native", "cwd": work, "args": args }))
        .await;
    assert_eq!(settings_in(&mut c, &inbox).await, None);
    c.watch(true).await;
    let frame = c.hooks_frame().await;
    assert_eq!(
        (frame["state"].as_str(), frame["ask"].clone()),
        (Some("absent"), json!(false))
    );
}

/// Claude's question as it draws it (`tests/fixtures/dialogs/claude-single.txt`), with the caret
/// left on one of its option rows: under its `❯` the rows read as a box being typed into.
fn dialog_on_screen() -> String {
    let rule = "─".repeat(60);
    format!(
        "\x1b[2J\x1b[H{rule}\n ☐ Indentation\n\nWhich indentation do you prefer?\n\n\
         ❯ 1. Tabs\n     Indent with tab characters.\n  2. Two spaces\n     Indent with two spaces per level.\n\
         \x20 3. Type something.\n{rule}\n  4. Chat about this\n\n\
         Enter to select · ↑/↓ to navigate · Esc to cancel\x1b[4A\x1b[21G"
    )
}

/// Only the options under the caret, which no screen read takes for a question: what a hook or
/// the pane's status says is standing is the only word that it is a dialog.
fn options_on_screen() -> String {
    "\x1b[2J\x1b[H❯ 1. Tabs\n     Indent with tab characters.\n  2. Type something.".into()
}

fn box_on_screen(typed: &str) -> String {
    format!("\x1b[2J\x1b[H● Done.\n\n{}\n❯\u{a0}{typed}", "─".repeat(60))
}

/// A dialog is not the box: while one stands the pane's half-typed line is nothing, whether a
/// hook asked it or the pane is blocked on it, and a whole dialog drawn on the screen is one
/// whatever else says so.
#[tokio::test]
async fn a_dialog_on_the_screen_is_never_published_as_the_half_typed_box() {
    for case in ["hooked", "blocked", "drawn"] {
        let mut c = Hooked::start().await;
        c.watch(true).await;
        let pane = c.pane.clone();
        c.paint(&box_on_screen("draft"));
        c.client
            .until(&format!("{case}: the box published"), |cl| {
                cl.others
                    .iter()
                    .find(|v| v["t"] == "convo.composer" && v["pane"] == pane && v["text"] == "draft")
                    .cloned()
            })
            .await;
        let held = match case {
            "hooked" => {
                let held = c.fire(ask(false));
                c.question().await;
                Some(held)
            }
            "blocked" => {
                c.status("waiting");
                c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
                None
            }
            _ => None,
        };
        c.client.others.clear();
        c.paint(&match case {
            "drawn" => dialog_on_screen(),
            _ => options_on_screen(),
        });
        c.client.shows(&pane, "Type something.").await;
        c.settle(Duration::from_millis(1500)).await;
        let read: Vec<_> = c
            .client
            .others
            .iter()
            .filter(|v| v["t"] == "convo.composer" && v["text"].as_str().is_some_and(|t| !t.is_empty()))
            .collect();
        assert!(
            read.is_empty(),
            "{case}: the dialog was published as the box: {read:?}"
        );

        if let Some(held) = held {
            c.answer("1").await;
            c.answer("1").await;
            c.ran(&held).await;
        }
        c.status("idle");
        c.paint(&box_on_screen("after"));
        c.client
            .until(&format!("{case}: the box published again"), |cl| {
                cl.others
                    .iter()
                    .find(|v| v["t"] == "convo.composer" && v["pane"] == pane && v["text"] == "after")
                    .cloned()
            })
            .await;
    }
}

/// A dialog the node has just answered through its hook is still drawn until Claude repaints, and
/// the pane is still `blocked` until its status catches up: reading it then would put a question
/// already answered back on every client.
#[tokio::test]
async fn a_dialog_the_node_answered_is_not_read_off_the_screen_while_it_is_still_drawn() {
    let mut c = Hooked::start().await;
    c.watch(false).await;
    c.paint("Do you want to proceed?\n\n1) Yes, from the screen\n2) No, from the screen\n");
    c.status("waiting");
    let pane = c.pane.clone();
    c.client.pane(&pane, |p| p["agent_status"] == "blocked").await;
    for (request, keys) in [(bash_request(None), vec!["1"]), (ask(false), vec!["1", "1"])] {
        let held = c.fire(request);
        c.client
            .until("the hook's card", |c| {
                c.others
                    .iter()
                    .any(|v| v["t"] == "pending" && v["source"] == "hook" && !v["question"].is_null())
                    .then_some(())
            })
            .await;
        for key in keys {
            c.answer(key).await;
        }
        c.ran(&held).await;
        c.client.others.clear();
        c.settle(Duration::from_millis(1500)).await;
        let read: Vec<_> = c
            .client
            .others
            .iter()
            .filter(|v| v["t"] == "pending" && v["source"] == "screen" && !v["question"].is_null())
            .collect();
        assert!(
            read.is_empty(),
            "an answered dialog came back off the screen: {read:?}"
        );
    }
}
