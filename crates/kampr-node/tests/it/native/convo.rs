//! A native Claude's conversation reaches the client when it changes, not on the pump's next tick:
//! a transcript line, a `/clear` that opens a new transcript, and a key typed into the box.

use super::*;
use std::io::Write;
use tokio::time::Instant;

const FIRST: &str = "11111111-1111-4111-8111-111111111111";
const SECOND: &str = "22222222-2222-4222-8222-222222222222";

/// A `claude` that writes a transcript under the project directory its working directory names,
/// with or without the marker 2.1.239 publishes, and then sits at its box with the tty echoing.
fn claude_at_its_box(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("claude");
    let script = r#"#!/bin/sh
home=$1; id=$2; marker=$3
cwd=$(pwd -P)
start=$(cut -d' ' -f22 /proc/$$/stat 2>/dev/null || LC_ALL=C TZ=UTC ps -o lstart= -p $$ | sed 's/^ *//; s/ *$//')
project="$home/.claude/projects/$(printf %s "$cwd" | tr / -)"
mkdir -p "$project" "$home/.claude/sessions"
now=$(date -u +%Y-%m-%dT%H:%M:%S.999Z)
printf '{"type":"user","uuid":"%s-1","cwd":"%s","timestamp":"%s","message":{"role":"user","content":"hello"}}\n' "$id" "$cwd" "$now" > "$project/$id.jsonl"
printf '{"type":"assistant","uuid":"%s-2","cwd":"%s","timestamp":"%s","message":{"content":[{"type":"text","text":"FIRST TURN"}]}}\n' "$id" "$cwd" "$now" >> "$project/$id.jsonl"
if [ "$marker" = yes ]; then
  printf '{"pid":%s,"sessionId":"%s","cwd":"%s","procStart":"%s","version":"2.1.239","kind":"interactive","entrypoint":"cli","status":"idle"}' $$ "$id" "$cwd" "$start" > "$home/.claude/sessions/$$.json"
fi
stty -icanon
printf '\342\235\257\302\240'
while :; do sleep 1; done
"#;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Claude {
    h: Harness,
    client: Client,
    pane: String,
    project: PathBuf,
}

impl Claude {
    async fn start(marker: bool) -> Self {
        let h = Harness::start().await;
        let home = h.home.as_ref().unwrap().path().to_path_buf();
        let work = h.work();
        let claude = claude_at_its_box(&work);
        let mut client = h.client().await;
        let pane = client
            .create(
                &[
                    &claude.display().to_string(),
                    &home.display().to_string(),
                    FIRST,
                    if marker { "yes" } else { "no" },
                ],
                json!({ "cwd": work.display().to_string() }),
            )
            .await;
        client
            .pane(&pane, |p| {
                p["agent"] == "claude" && p["has_conversation"] == json!(true)
            })
            .await;
        let cwd = work.canonicalize().unwrap().display().to_string();
        let project = home.join(".claude/projects").join(cwd.replace('/', "-"));
        client
            .send(json!({ "t": "watch", "pane": pane, "conversation": true }))
            .await;
        let page = client.reply("convo").await;
        assert!(page["turns"].to_string().contains("FIRST TURN"), "{page}");
        Self {
            h,
            client,
            pane,
            project,
        }
    }

    fn record(&self, session: &str, uuid: &str, text: &str, ahead: i64) -> String {
        let cwd = self.h.work().canonicalize().unwrap().display().to_string();
        let at = (time::OffsetDateTime::now_utc() + time::Duration::seconds(ahead))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let record = json!({
            "type": "assistant", "uuid": uuid, "cwd": cwd, "timestamp": at, "sessionId": session,
            "message": { "content": [ { "type": "text", "text": text } ] },
        });
        format!("{record}\n")
    }

    fn append(&self, session: &str, line: &str) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.project.join(format!("{session}.jsonl")))
            .unwrap();
        file.write_all(line.as_bytes()).unwrap();
    }

    /// How long until a conversation message carrying `text` reached the client.
    async fn arrives(&mut self, text: &str, since: Instant) -> Duration {
        let pane = self.pane.clone();
        self.client
            .until(&format!("{text:?} in the conversation"), |c| {
                let at = c.others.iter().position(|v| {
                    (v["t"] == "convo" || v["t"] == "convo.turn")
                        && v["pane"] == pane
                        && v["turns"].to_string().contains(text)
                })?;
                Some(c.others.remove(at))
            })
            .await;
        since.elapsed()
    }

    /// Settles past the pump's own floor, so what is measured next is the wake and not the pace.
    async fn quiet(&mut self) {
        let until = Instant::now() + Duration::from_millis(150);
        while let left = until.saturating_duration_since(Instant::now())
            && !left.is_zero()
        {
            self.client.pump(left).await;
        }
    }
}

/// **A line written to the transcript is on the client at once**, rather than at the next of the
/// pump's 400 ms re-reads: each lands well inside that, and with the reads in step with the
/// writes, a tick would make every one of them wait most of it.
#[tokio::test]
async fn a_line_written_to_the_transcript_reaches_the_client_at_once() {
    let mut c = Claude::start(true).await;
    let mut slowest = Duration::ZERO;
    for n in 0..4 {
        c.quiet().await;
        let text = format!("LINE {n}");
        let line = c.record(FIRST, &format!("{FIRST}-line-{n}"), &text, 0);
        let written = Instant::now();
        c.append(FIRST, &line);
        slowest = slowest.max(c.arrives(&text, written).await);
    }
    assert!(
        slowest < Duration::from_millis(100),
        "a line took {slowest:?} to arrive"
    );
}

/// **A `/clear` under a harness that names no session is seen when its new transcript appears**
/// (#259). The pane's handle does not move, so the only news is the file, and it used to wait out a
/// 15 s recheck.
#[tokio::test]
async fn a_new_transcript_in_the_panes_directory_replaces_the_conversation_at_once() {
    let mut c = Claude::start(false).await;
    c.quiet().await;
    let line = c.record(SECOND, &format!("{SECOND}-1"), "AFTER CLEAR", 2);
    let written = Instant::now();
    c.append(SECOND, &line);
    let took = c.arrives("AFTER CLEAR", written).await;
    assert!(
        took < Duration::from_millis(250),
        "the new conversation took {took:?}"
    );
}

/// **What is typed into the box follows the frame that drew it**, not the next of a 100 ms loop:
/// measured from the grid patch carrying the key to the `convo.composer` carrying it.
#[tokio::test]
async fn a_key_typed_into_the_box_follows_its_frame_to_the_reply_box() {
    let mut c = Claude::start(true).await;
    let pane = c.pane.clone();
    let mut typed = String::new();
    let mut slowest = Duration::ZERO;
    for key in ["p", "u", "s", "h", "!"] {
        c.quiet().await;
        typed.push_str(key);
        c.client.type_text(&pane, key).await;
        let want = typed.clone();
        let mut drawn = None;
        let mut composed = None;
        let deadline = Instant::now() + SOON;
        while drawn.is_none() || composed.is_none() {
            assert!(
                Instant::now() < deadline,
                "{want:?} never reached both; screen {:?}",
                c.client.screen(&pane)
            );
            c.client.pump(Duration::from_millis(200)).await;
            if drawn.is_none() && c.client.screen(&pane).iter().any(|r| r.ends_with(want.as_str())) {
                drawn = Some(Instant::now());
            }
            if composed.is_none()
                && let Some(at) = c.client.others.iter().position(|v| {
                    v["t"] == "convo.composer" && v["pane"] == pane && v["text"] == want.as_str()
                })
            {
                c.client.others.remove(at);
                composed = Some(Instant::now());
            }
        }
        slowest = slowest.max(composed.unwrap().saturating_duration_since(drawn.unwrap()));
    }
    assert!(
        slowest < Duration::from_millis(40),
        "the box lagged its frame by {slowest:?}"
    );
}
