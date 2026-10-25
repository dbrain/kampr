use super::*;
use crate::live::Live;
use serde_json::{Value, json};

fn entries(raw: Value) -> Vec<TranscriptEntry> {
    serde_json::from_value(raw).expect("transcript entries")
}

fn blocks(raw: Value) -> Vec<ContentBlock> {
    serde_json::from_value(raw).expect("content blocks")
}

fn wire(turns: &[Turn]) -> Value {
    serde_json::to_value(turns).unwrap()
}

fn delegate_workers() -> Workers {
    Workers::of(
        &serde_json::from_value::<Vec<Subagent>>(json!([
            { "call_id": "call_7", "id": "s1.s1", "liveness": "Alive", "role": "TaskChimp",
              "state": "running", "task_id": "t-1", "title": "Make the file" }
        ]))
        .unwrap(),
    )
}

#[test]
fn a_transcript_maps_to_turns_the_way_docs_18_says() {
    let cases: Vec<(&str, Value, Workers, Value)> = vec![
        (
            "system entries and thinking are dropped, user and assistant are turns",
            json!([
                { "seq": 0, "role": "system", "blocks": [{ "type": "text", "text": "You are muffit" }] },
                { "seq": 1, "role": "user", "blocks": [{ "type": "text", "text": "hi flash-next" }] },
                { "seq": 2, "role": "assistant", "blocks": [
                    { "type": "thinking", "text": "brief", "collapsed_by_default": true },
                    { "type": "text", "text": "\n\nHi! Sanity check passes" } ] }
            ]),
            Workers::default(),
            json!([
                { "id": "m1", "role": "user", "blocks": [{ "b": "md", "text": "hi flash-next" }] },
                { "id": "m2", "role": "assistant", "blocks": [{ "b": "md", "text": "Hi! Sanity check passes" }] }
            ]),
        ),
        (
            "a tool entry folds its result into the call it answers",
            json!([
                { "seq": 3, "role": "assistant", "blocks": [
                    { "type": "text", "text": "Looking." },
                    { "type": "tool_call", "id": "c1", "tool": "bash",
                      "args": "{\"command\":\"ls -la\\n  src\"}", "status": "finished" } ] },
                { "seq": 4, "role": "tool", "blocks": [
                    { "type": "tool_result", "call_id": "c1", "summary": "a\nb\nc", "exit_status": 0,
                      "artifact_ref": null, "truncated": true, "total_lines": 40 } ] }
            ]),
            Workers::default(),
            json!([
                { "id": "m3", "role": "assistant", "blocks": [
                    { "b": "md", "text": "Looking." },
                    { "b": "tool", "name": "bash", "summary": "ls -la src", "lines": 40, "state": "done" },
                    { "b": "code", "text": "a\nb\nc", "role": "output" } ] }
            ]),
        ),
        (
            "parallel calls keep their own results, and a call with none yet is running",
            json!([
                { "seq": 5, "role": "assistant", "blocks": [
                    { "type": "tool_call", "id": "a", "tool": "read", "args": "{\"path\":\"x.rs\"}", "status": "finished" },
                    { "type": "tool_call", "id": "b", "tool": "read", "args": "{\"path\":\"y.rs\"}", "status": "finished" },
                    { "type": "tool_call", "id": "c", "tool": "grep", "args": "{\"pattern\":\"fn\"}", "status": "finished" } ] },
                { "seq": 6, "role": "tool", "blocks": [
                    { "type": "tool_result", "call_id": "b", "summary": "y" } ] },
                { "seq": 7, "role": "tool", "blocks": [
                    { "type": "tool_result", "call_id": "a", "summary": "x1\nx2" } ] }
            ]),
            Workers::default(),
            json!([
                { "id": "m5", "role": "assistant", "blocks": [
                    { "b": "tool", "name": "read", "summary": "x.rs", "lines": 2, "state": "done" },
                    { "b": "code", "text": "x1\nx2", "role": "output" },
                    { "b": "tool", "name": "read", "summary": "y.rs", "lines": 1, "state": "done" },
                    { "b": "code", "text": "y", "role": "output" },
                    { "b": "tool", "name": "grep", "summary": "fn", "state": "running" } ] }
            ]),
        ),
        (
            "a diff rides with the call that made it, rendered unified",
            json!([
                { "seq": 8, "role": "assistant", "blocks": [
                    { "type": "tool_call", "id": "e", "tool": "edit", "args": "{\"path\":\"src/a.rs\"}", "status": "finished" } ] },
                { "seq": 9, "role": "tool", "blocks": [
                    { "type": "tool_result", "call_id": "e", "summary": "edited" },
                    { "type": "diff", "call_id": "e", "path": "src/a.rs", "change_kind": "modify",
                      "hunks": [{ "old_start": 3, "old_lines": 1, "new_start": 3, "new_lines": 2,
                                  "old_text": "let a = 1;\n", "new_text": "let a = 2;\nlet b = 3;\n" }] } ] }
            ]),
            Workers::default(),
            json!([
                { "id": "m8", "role": "assistant", "blocks": [
                    { "b": "tool", "name": "edit", "summary": "src/a.rs", "lines": 1, "state": "done" },
                    { "b": "code", "text": "edited", "role": "output" },
                    { "b": "diff", "path": "src/a.rs",
                      "text": "@@ -3,1 +3,2 @@\n-let a = 1;\n+let a = 2;\n+let b = 3;" } ] }
            ]),
        ),
        (
            "a delegate call offers its worker once the worker is known",
            json!([
                { "seq": 10, "role": "assistant", "blocks": [
                    { "type": "tool_call", "id": "call_7", "tool": "delegate",
                      "args": "{\"brief\":\"Create made.txt containing hello.\",\"title\":\"Make the file\"}",
                      "status": "finished" } ] }
            ]),
            delegate_workers(),
            json!([
                { "id": "m10", "role": "assistant", "blocks": [
                    { "b": "tool", "name": "delegate", "summary": "Make the file", "state": "running" },
                    { "b": "sub", "id": "s1.s1", "kind": "worker", "title": "Make the file" } ] }
            ]),
        ),
        (
            "an image is its marker, and a turn of nothing shown is no turn",
            json!([
                { "seq": 11, "role": "user", "blocks": [
                    { "type": "text", "text": "look" }, { "type": "image", "mime": "image/png", "bytes": 120 } ] },
                { "seq": 12, "role": "assistant", "blocks": [{ "type": "thinking", "text": "hmm" }] },
                { "seq": 13, "role": "assistant", "blocks": [{ "type": "recalled", "tokens": 4, "sources": [] },
                                                              { "type": "notice", "text": "model changed" }] }
            ]),
            Workers::default(),
            json!([
                { "id": "m11", "role": "user", "blocks": [
                    { "b": "md", "text": "look" }, { "b": "md", "text": "[image · image/png]" } ] }
            ]),
        ),
    ];
    for (name, raw, workers, want) in cases {
        assert_eq!(wire(&turns(&entries(raw), &workers)), want, "{name}");
    }
}

#[test]
fn the_live_turn_is_built_from_content_frames() {
    let cases: Vec<(&str, Value, Value)> = vec![
        (
            "text deltas are one block",
            json!([{ "type": "text", "text": "The parser" }, { "type": "text", "text": " is a state machine" }]),
            json!([{ "b": "md", "text": "The parser is a state machine" }]),
        ),
        (
            "a running call gathers its output, and its result completes it",
            json!([
                { "type": "tool_call", "id": "call_7", "tool": "delegate",
                  "args": "{\"brief\":\"Create made.txt containing hello.\",\"title\":\"Make the file\"}", "status": "started" },
                { "type": "tool_output_chunk", "call_id": "call_7", "stream": "stdout", "text": "Worker s1 on mock\n", "seq": 0 }
            ]),
            json!([
                { "b": "tool", "name": "delegate", "summary": "Make the file", "state": "running" },
                { "b": "sub", "id": "s1.s1", "kind": "worker", "title": "Make the file" },
                { "b": "code", "text": "Worker s1 on mock", "role": "output" }
            ]),
        ),
        (
            "the prompt being answered is not part of the reply",
            json!([{ "type": "user_message", "text": "second message" }]),
            json!(null),
        ),
    ];
    for (name, raw, want) in cases {
        let mut live = Live::default();
        for block in blocks(raw) {
            live.push(&block);
        }
        let got = live
            .turn(&delegate_workers())
            .map(|t| serde_json::to_value(t.blocks).unwrap());
        assert_eq!(got.unwrap_or(Value::Null), want, "{name}");
    }

    let mut live = Live::default();
    for block in blocks(json!([
        { "type": "tool_call", "id": "call_7", "tool": "delegate", "args": "{\"title\":\"t\"}", "status": "started" },
        { "type": "tool_result", "call_id": "call_7", "summary": "The worker completed the task.", "truncated": false },
        { "type": "tool_call", "id": "call_7", "tool": "delegate", "args": "", "status": "finished" }
    ])) {
        live.push(&block);
    }
    let turn = live.turn(&Workers::default()).expect("a live turn");
    assert_eq!(turn.id, "live");
    assert_eq!(
        serde_json::to_value(&turn.blocks).unwrap(),
        json!([
            { "b": "tool", "name": "delegate", "summary": "t", "lines": 1, "state": "done" },
            { "b": "code", "text": "The worker completed the task.", "role": "output" }
        ])
    );
    let withdrawn = live.reset().expect("a withdrawal for a shown turn");
    assert!(withdrawn.blocks.is_empty());
    assert!(live.reset().is_none(), "a withdrawal is owed once");
}

#[test]
fn a_failed_call_is_an_error() {
    let mut live = Live::default();
    for block in blocks(json!([
        { "type": "tool_call", "id": "x", "tool": "bash", "args": "{\"command\":\"false\"}", "status": "failed" }
    ])) {
        live.push(&block);
    }
    let turn = live.turn(&Workers::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&turn.blocks).unwrap(),
        json!([{ "b": "tool", "name": "bash", "summary": "false", "state": "error" }])
    );
}

#[test]
fn arguments_read_as_one_line() {
    let long = "x".repeat(200);
    let cases: Vec<(String, Option<String>)> = vec![
        (
            "{\"command\":\"cargo  test\\n -p a\"}".into(),
            Some("cargo test -p a".into()),
        ),
        ("{}".into(), None),
        ("".into(), None),
        ("{\"n\":3}".into(), Some("{\"n\":3}".into())),
        (
            format!("{{\"command\":\"{long}\"}}"),
            Some(format!("{}…", "x".repeat(120))),
        ),
    ];
    for (args, want) in cases {
        assert_eq!(one_line(&args), want, "{args}");
    }
}
