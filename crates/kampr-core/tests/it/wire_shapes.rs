//! The wire is additive only: old clients are installed on real phones. Every message is built
//! here with every optional field present, reduced to the paths and JSON types it carries, and
//! compared with `tests/fixtures/wire-shapes.txt`. A path that disappears or changes type fails;
//! a new one passes and is written into the snapshot with `KAMPR_BLESS_WIRE=1`.

use kampr_core::provider::AgentStatus;
use kampr_core::wire::*;
use kampr_core::{ClientMsg, Question, ServerMsg};
use kampr_journal::{
    Attachment, Block, CodeRole, EditKeys, Facets, Role as Speaker, ToolState, Turn, TurnKind,
};
use kampr_term::{CellAttrs, Color};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

const SNAPSHOT: &str = "tests/fixtures/wire-shapes.txt";
const TAGS: &[&str] = &["t", "b", "shape"];

fn shapes(side: &str, message: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let tag = message["t"].as_str().expect("every message has a t");
    walk(&format!("{side} {tag} "), message, &mut out);
    out
}

fn walk(path: &str, value: &Value, out: &mut BTreeSet<String>) {
    let kind = match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    out.insert(format!("{path}: {kind}"));
    match value {
        Value::Array(items) => {
            for item in items {
                let tag = TAGS
                    .iter()
                    .find_map(|key| item[key].as_str().map(|v| format!("[{key}={v}]")))
                    .unwrap_or_else(|| "[]".to_string());
                walk(&format!("{path}{tag}"), item, out);
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                walk(&format!("{path}.{key}"), field, out);
            }
        }
        _ => {}
    }
}

fn runs() -> Vec<RowRuns> {
    vec![RowRuns {
        row: 0,
        runs: vec![Run {
            s: 1,
            x: "é".into(),
            l: Some(0),
            w: 2,
            m: vec!["\u{301}".into()],
        }],
    }]
}

fn cursor() -> Cursor {
    Cursor {
        col: 1,
        row: 2,
        visible: true,
    }
}

fn question(shape: Value) -> Question {
    serde_json::from_value(json!({
        "prompt": "Proceed?", "context": ["line"], "shape": shape, "inferred": true
    }))
    .expect("a question")
}

fn pane(question: Question) -> PaneEntry {
    PaneEntry {
        id: "n/w1:p1".into(),
        node_id: "n".into(),
        workspace_id: Some("n/w1".into()),
        tab_id: Some("n/w1:t1".into()),
        workspace: Some("w".into()),
        tab: Some("t".into()),
        cwd: Some("/".into()),
        label: Some("claude".into()),
        agent: Some("claude".into()),
        agent_status: AgentStatus::Working,
        cols: Some(80),
        rows: 24,
        scrollback_rows: 10,
        has_conversation: true,
        converses: true,
        watchers: Some(2),
        updated_at: Some("2026-01-01T00:00:00Z".into()),
        detail: Some("d".into()),
        fleet: Some(FleetEntry {
            cohort: "c".into(),
            command: "ls".into(),
            state: "waiting".into(),
            question: Some(question),
            exit_code: Some(1),
            signal: Some(9),
            quiet_seconds: Some(3),
            blind: true,
            started_unix: 1,
        }),
        cmd: Some("claude".into()),
        argv: Some("claude --x".into()),
        title: Some("title".into()),
        provider: Some("native".into()),
        session_id: Some("01k".into()),
        on_desk: Some("w1:p3".into()),
        detached: Some(false),
        size_holder: Some("phone:pixel".into()),
        exited: Some(ExitEntry {
            code: Some(0),
            signal: Some(9),
        }),
    }
}

fn node() -> NodeEntry {
    NodeEntry {
        id: "n".into(),
        name: "n".into(),
        kind: "local".into(),
        online: true,
        reachable: Some(true),
        native: Some(true),
        rtt_ms: Some(1.5),
        herdr_version: Some("0.9.1".into()),
        build: Some("0.1.0".into()),
        update: Some("0.1.1".into()),
        detail: Some("d".into()),
    }
}

fn option() -> PendingOption {
    PendingOption {
        key: "1".into(),
        label: "Yes".into(),
        detail: Some("d".into()),
        chosen: true,
    }
}

fn herd() -> (Vec<NodeEntry>, Vec<PaneEntry>) {
    let numbered = question(json!({ "shape": "numbered", "options": [option()] }));
    let confirm = question(json!({ "shape": "confirm", "options": [option()], "default_key": "y" }));
    (vec![node()], vec![pane(numbered), pane(confirm)])
}

fn turns() -> Vec<Turn> {
    let attachment = Attachment {
        id: "a".into(),
        kind: "image".into(),
        mime: Some("image/png".into()),
        bytes: Some(1),
        name: Some("a.png".into()),
    };
    vec![Turn {
        id: "t".into(),
        role: Speaker::Assistant,
        at: Some("2026-01-01T00:00:00Z".into()),
        kind: Some(TurnKind::Compact),
        blocks: vec![
            Block::Md {
                text: "x".into(),
                att: Some(attachment),
            },
            Block::Code {
                lang: Some("rs".into()),
                text: "x".into(),
                role: Some(CodeRole::Output),
            },
            Block::Tool {
                name: "Bash".into(),
                summary: Some("ls".into()),
                lines: Some(1),
                state: ToolState::Done,
            },
            Block::Diff {
                path: Some("a.rs".into()),
                text: "+".into(),
            },
            Block::Sub {
                id: "s".into(),
                kind: Some("task".into()),
                title: Some("t".into()),
                depth: Some(1),
            },
        ],
    }]
}

fn facets() -> Facets {
    serde_json::from_value(json!({
        "title": { "text": "t", "source": "manual" },
        "timings": [{ "turn": "t", "duration_ms": 1, "messages": 1 }],
        "queued": [{ "text": "q", "at": "2026-01-01T00:00:00Z" }],
        "mode": { "mode": "plan", "permission": "default" },
        "compactions": [{ "at": "a", "trigger": "auto", "pre_tokens": 1, "post_tokens": 1, "dropped_tokens": 1 }],
        "running": [{ "call": "c", "kind": "k", "name": "n", "title": "t", "since": "s" }],
        "status": "working"
    }))
    .expect("facets")
}

fn server() -> Vec<ServerMsg> {
    let (nodes, panes) = herd();
    let pane = || "n/w1:p1".to_string();
    vec![
        ServerMsg::Hello(Hello {
            protocol: PROTOCOL,
            node_id: "n".into(),
            node_name: "n".into(),
            build: "0.1.0".into(),
            role: Role::Full,
            caps: Caps {
                push: true,
                scrollback: true,
                conversation: true,
            },
        }),
        ServerMsg::RoleChanged { role: Role::Readonly },
        ServerMsg::Herd {
            nodes: nodes.clone(),
            panes: panes.clone(),
        },
        ServerMsg::HerdPatch {
            added: HerdDelta {
                nodes: nodes.clone(),
                panes: panes.clone(),
            },
            changed: HerdDelta { nodes, panes },
            removed_ids: vec!["n/w1:p2".into()],
        },
        ServerMsg::Styles(Styles {
            from: 1,
            styles: vec![Style {
                fg: Color::Indexed(1),
                bg: Color::Rgb(1, 2, 3),
                attrs: CellAttrs {
                    bold: true,
                    dim: true,
                    italic: true,
                    underline: true,
                    blink: true,
                    reverse: true,
                    strike: true,
                    hidden: true,
                    ul: Color::Rgb(4, 5, 6),
                },
            }],
        }),
        ServerMsg::Find {
            pane: pane(),
            query: "q".into(),
            matches: vec![FindMatch {
                from_bottom: 1,
                col: 1,
                end_from_bottom: 1,
                end_col: 2,
                text: "q".into(),
            }],
            total: 1,
            current: Some(0),
        },
        ServerMsg::ConvoFind {
            pane: pane(),
            query: "q".into(),
            matches: vec![ConvoMatch {
                turn: "t".into(),
                role: "user".into(),
                at: Some("a".into()),
                from_end: 1,
                hits: 1,
                text: "q".into(),
            }],
            total: 1,
        },
        ServerMsg::GridReset {
            pane: pane(),
            cols: 80,
            rows: 24,
            rows_data: runs(),
            cursor: cursor(),
            links: vec!["https://example.com".into()],
            reflow: Some(kampr_core::Reflow::On),
        },
        ServerMsg::GridPatch {
            pane: pane(),
            rows: runs(),
            cursor: cursor(),
            links: vec!["https://example.com".into()],
        },
        ServerMsg::Scrollback {
            pane: pane(),
            from_top: 0,
            rows: runs(),
            total_rows: 1,
            complete: true,
            capped: false,
            era: 1,
        },
        ServerMsg::Convo {
            pane: pane(),
            cursor: Some("c".into()),
            more: true,
            fresh: true,
            sub: Some("s".into()),
            turns: turns(),
        },
        ServerMsg::ConvoTurn {
            pane: pane(),
            sub: Some("s".into()),
            turns: turns(),
        },
        ServerMsg::Pending {
            pane: pane(),
            question: Some("q".into()),
            options: vec![option()],
            source: PendingSource::Screen,
            header: Some("h".into()),
            multi: true,
        },
        ServerMsg::ConvoFacets {
            pane: pane(),
            facets: facets(),
        },
        ServerMsg::ConvoComposer {
            pane: pane(),
            text: Some("t".into()),
            clear: Some("\u{15}".into()),
            caret: Some(1),
            keys: Some(EditKeys {
                back: "\u{7f}",
                left: "\u{1b}[D",
                right: "\u{1b}[C",
                newline: "\u{1b}\r",
            }),
        },
        ServerMsg::Error {
            code: ErrorCode::BadRequest,
            message: "m".into(),
            pane: Some(pane()),
            node: Some("n".into()),
        },
        ServerMsg::Pong { n: 1 },
    ]
}

fn client() -> Vec<ClientMsg> {
    let pane = || "n/w1:p1".to_string();
    vec![
        ClientMsg::Watch {
            pane: pane(),
            scrollback: true,
            conversation: true,
            reflow: Some(ReflowAsk { cols: 40 }),
        },
        ClientMsg::Unwatch { pane: pane() },
        ClientMsg::AnswerSubmit { pane: pane() },
        ClientMsg::DoneRead {
            pane: pane(),
            at: Some("2026-10-14T09:00:00Z".into()),
        },
        ClientMsg::Input {
            pane: pane(),
            text: Some("t".into()),
            b64: Some("dA==".into()),
            keys: Some(vec!["Enter".into()]),
            typed: true,
        },
        ClientMsg::Paste {
            pane: pane(),
            b64: "dA==".into(),
            name: Some("a.png".into()),
        },
        ClientMsg::Answer {
            pane: pane(),
            key: "1".into(),
        },
        ClientMsg::ConvoLoad {
            pane: pane(),
            before: Some("t".into()),
        },
        ClientMsg::ConvoSub {
            pane: pane(),
            id: "s".into(),
            before: Some("t".into()),
        },
        ClientMsg::ConvoFind {
            pane: pane(),
            query: "q".into(),
        },
        ClientMsg::Find {
            pane: pane(),
            query: "q".into(),
            backward: false,
            from: Some(1),
        },
        ClientMsg::Resync,
        ClientMsg::Ping { n: 1 },
    ]
}

fn current() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for message in server() {
        out.extend(shapes(
            "server",
            &serde_json::to_value(message).expect("serialises"),
        ));
    }
    for message in client() {
        let value = serde_json::to_value(&message).expect("serialises");
        let back: ClientMsg = serde_json::from_value(value.clone()).expect("a node reads it back");
        assert_eq!(serde_json::to_value(back).unwrap(), value, "{value}");
        out.extend(shapes("client", &value));
    }
    out
}

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT)
}

fn snapshot() -> BTreeSet<String> {
    std::fs::read_to_string(snapshot_path())
        .expect("the snapshot")
        .lines()
        .map(str::to_string)
        .collect()
}

fn broken(before: &BTreeSet<String>, now: &BTreeSet<String>) -> Vec<String> {
    before.difference(now).cloned().collect()
}

fn tags_in(source: &str, enumeration: &str) -> BTreeSet<String> {
    let start = source
        .find(&format!("pub enum {enumeration} {{"))
        .expect("the enum");
    let body = &source[start..start + source[start..].find("\n}\n").expect("its end")];
    body.lines()
        .filter_map(|line| line.strip_prefix("    #[serde(rename = \""))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

#[test]
fn no_wire_field_is_removed_or_retyped() {
    let now = current();
    if std::env::var_os("KAMPR_BLESS_WIRE").is_some() {
        let mut text: Vec<&str> = now.iter().map(String::as_str).collect();
        text.push("");
        std::fs::write(snapshot_path(), text.join("\n")).expect("blessed");
        return;
    }
    let before = snapshot();
    let gone = broken(&before, &now);
    assert!(
        gone.is_empty(),
        "an old client depends on every one of these, and it is gone or changed type:\n{}",
        gone.join("\n")
    );
    let added = now.difference(&before).count();
    if added > 0 {
        eprintln!("{added} new wire paths: KAMPR_BLESS_WIRE=1 records them so they are guarded too");
    }
}

#[test]
fn every_message_type_has_a_sample() {
    let source = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/wire.rs"))
        .expect("wire.rs");
    let sampled = |side: &str| -> BTreeSet<String> {
        current()
            .iter()
            .filter_map(|line| line.strip_prefix(&format!("{side} ")))
            .filter_map(|rest| rest.split(' ').next())
            .map(str::to_string)
            .collect()
    };
    assert_eq!(sampled("server"), tags_in(&source, "ServerMsg"));
    assert_eq!(sampled("client"), tags_in(&source, "ClientMsg"));
}

#[test]
fn a_removed_or_retyped_field_breaks_and_an_added_one_does_not() {
    let before = shapes("server", &json!({ "t": "x", "a": 1, "b": "s" }));
    let retyped = shapes("server", &json!({ "t": "x", "a": "1", "b": "s" }));
    let removed = shapes("server", &json!({ "t": "x", "b": "s" }));
    let added = shapes("server", &json!({ "t": "x", "a": 1, "b": "s", "c": true }));
    assert_eq!(broken(&before, &retyped), vec!["server x .a: integer"]);
    assert_eq!(broken(&before, &removed), vec!["server x .a: integer"]);
    assert!(broken(&before, &added).is_empty());
}
