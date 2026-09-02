//! What a tool call *produced*, as opposed to what it was asked to do.
//!
//! The `lines` on a tool card has always counted the result, and the only block beside it was the
//! call's own input — so a `Bash` card said "13 lines" over a one-line command and expanding it
//! showed the command. These cover the block that carries the result, the cap that keeps it off
//! the socket whole, and the calls it is worth carrying for.

use crate::common::*;
use kampr_journal::{Block, CodeRole, Turn};
use serde_json::{Value, json};

fn call(uuid: &str, tool: &str, input: Value) -> Value {
    json!({
        "type": "assistant",
        "uuid": uuid,
        "message": { "content": [ { "type": "tool_use", "id": "toolu_1", "name": tool, "input": input } ] },
    })
}

fn result(uuid: &str, text: &str, is_error: bool) -> Value {
    json!({
        "type": "user",
        "uuid": uuid,
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": text, "is_error": is_error }
        ] },
    })
}

fn outputs(turns: &[Turn]) -> Vec<&str> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Code {
                role: Some(CodeRole::Output),
                text,
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn lines_of(turns: &[Turn]) -> Option<u32> {
    turns.iter().flat_map(|t| &t.blocks).find_map(|b| match b {
        Block::Tool { lines, .. } => *lines,
        _ => None,
    })
}

#[test]
fn a_bash_result_rides_beside_the_command_and_the_count_is_the_result() {
    let mut scratch = scratch_claude(
        "output-bash",
        &[
            call(
                "a1",
                "Bash",
                json!({ "command": "herdr pane list", "description": "list panes" }),
            ),
            result("a2", "w3:p2\nw3:p3\nw3:p4\n", false),
        ],
    );
    let turns = scratch.turns();

    let blocks = &turns[0].blocks;
    assert_eq!(blocks.len(), 3, "card, command, result — {blocks:#?}");
    assert_eq!(
        blocks[1],
        Block::Code {
            lang: Some("bash".into()),
            text: "herdr pane list".into(),
            role: None,
        },
        "the command keeps the shape every installed client already renders"
    );
    assert_eq!(
        blocks[2],
        Block::Code {
            lang: None,
            text: "w3:p2\nw3:p3\nw3:p4".into(),
            role: Some(CodeRole::Output),
        }
    );
    assert_eq!(
        lines_of(&turns),
        Some(3),
        "`lines` counts the result, which is what the card has always claimed"
    );
}

/// The number on the card is what tells a reader the block under it is not the whole of it, so it
/// stays the true total and the block is what shrinks.
#[test]
fn a_result_past_the_cap_is_cut_while_the_card_still_counts_all_of_it() {
    let body: String = (0..400).map(|n| format!("row {n}\n")).collect();
    let mut scratch = scratch_claude(
        "output-cap",
        &[
            call("a1", "Bash", json!({ "command": "seq 400" })),
            result("a2", &body, false),
        ],
    );
    let turns = scratch.turns();

    let carried = outputs(&turns);
    assert_eq!(carried.len(), 1);
    let carried = carried[0];
    assert!(
        carried.lines().count() < 400,
        "400 lines went out whole: {} lines",
        carried.lines().count()
    );
    assert!(carried.len() <= 8 * 1024, "{} bytes on the wire", carried.len());
    assert!(
        carried.starts_with("row 0\nrow 1\n"),
        "the cut is from the far end"
    );
    assert_eq!(
        lines_of(&turns),
        Some(400),
        "the count is the true total or a client cannot say it was cut"
    );
}

/// Under the line cap and far over the byte cap, which is the shape a `grep` of a minified file
/// or a wide table arrives in.
#[test]
fn a_result_of_long_lines_is_cut_on_bytes_rather_than_lines() {
    let body: String = (0..60).map(|n| format!("{n:0<400}\n")).collect();
    let mut scratch = scratch_claude(
        "output-wide",
        &[
            call("a1", "Bash", json!({ "command": "cat wide" })),
            result("a2", &body, false),
        ],
    );
    let turns = scratch.turns();

    let carried = outputs(&turns);
    assert_eq!(carried.len(), 1);
    assert!(
        carried[0].len() <= 8 * 1024,
        "{} bytes on the wire, under the line cap the whole way",
        carried[0].len()
    );
    assert_eq!(lines_of(&turns), Some(60));
}

/// One line longer than the byte cap has no line boundary to cut on, and a `String` sliced at an
/// arbitrary byte is a panic rather than a long block.
///
/// **The character is three bytes and not two.** The byte cap is 8192, which is even, so a
/// two-byte character lands the naive cut on a boundary by luck and a test written with one
/// passes with the walk removed.
#[test]
fn one_enormous_line_is_cut_on_a_character_boundary() {
    let body = "…".repeat(40 * 1024);
    let mut scratch = scratch_claude(
        "output-unbroken",
        &[
            call("a1", "Bash", json!({ "command": "cat minified.js" })),
            result("a2", &body, false),
        ],
    );
    let turns = scratch.turns();

    let carried = outputs(&turns);
    assert_eq!(carried.len(), 1, "a single unbroken line is still a result");
    assert!(
        carried[0].len() <= 8 * 1024,
        "{} bytes on the wire",
        carried[0].len()
    );
    assert!(carried[0].chars().all(|c| c == '…'), "cut mid-character");
}

/// The wire's own rule for a tool turn — revised in place, "never append, or every tool renders
/// twice". A result delivered twice is the shape that proves the revision is a revision.
#[test]
fn a_result_that_lands_twice_carries_one_output_block() {
    let mut scratch = scratch_claude(
        "output-twice",
        &[
            call("a1", "Bash", json!({ "command": "echo hi" })),
            result("a2", "hi\nthere\n", false),
            result("a3", "hi\nthere\n", false),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(outputs(&turns), ["hi\nthere"], "{:#?}", turns[0].blocks);
    assert_eq!(turns[0].blocks.len(), 3);
}

/// The whole transcript read again, which is what a truncated file or a restarted follow does.
#[test]
fn a_transcript_read_again_carries_one_output_block() {
    let records = [
        call("a1", "Bash", json!({ "command": "echo hi" })),
        result("a2", "hi\nthere\n", false),
    ];
    let mut scratch = scratch_claude("output-reread", &records);
    scratch.turns();

    let body = lines(&records);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&scratch.transcript)
        .expect("the transcript");
    std::io::Write::write_all(&mut file, body.as_bytes()).expect("append");
    drop(file);
    scratch.turns();

    let turns = scratch.journal.page_before(None, 40).turns;
    assert_eq!(outputs(&turns), ["hi\nthere"], "{:#?}", turns[0].blocks);
}

/// Two calls in one assistant record. The output has to land in its own call's group or a reader
/// is shown one command's answer under another command — and a card records *where* it sits, so a
/// block inserted in front of one has to move it.
///
/// **Both settle orders.** With the earlier card settled last, nothing shifts under the later one
/// and the bookkeeping is unobservable; with it settled first, a later card left pointing at the
/// block pushed in front of it never settles at all.
#[test]
fn parallel_calls_each_keep_their_own_result() {
    for order in [["toolu_1", "toolu_2"], ["toolu_2", "toolu_1"]] {
        let mut records = vec![json!({
            "type": "assistant",
            "uuid": "a1",
            "message": { "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "first" } },
                { "type": "tool_use", "id": "toolu_2", "name": "Bash", "input": { "command": "second" } }
            ] },
        })];
        for (n, call) in order.iter().enumerate() {
            let out = if *call == "toolu_1" {
                "first out"
            } else {
                "second out"
            };
            records.push(json!({
                "type": "user",
                "uuid": format!("r{n}"),
                "message": { "content": [
                    { "type": "tool_result", "tool_use_id": call, "content": out, "is_error": false }
                ] },
            }));
        }
        let mut scratch = scratch_claude(&format!("output-parallel-{}", order[0]), &records);
        let turns = scratch.turns();

        let shape: Vec<String> = turns[0]
            .blocks
            .iter()
            .map(|b| match b {
                Block::Tool { name, lines, .. } => format!("tool {name} {lines:?}"),
                Block::Code { role: None, text, .. } => format!("in {text}"),
                Block::Code { text, .. } => format!("out {text}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "tool Bash Some(1)",
                "in first",
                "out first out",
                "tool Bash Some(1)",
                "in second",
                "out second out"
            ],
            "settled {order:?}"
        );
    }
}

/// `Read` has a better surface already — the client fetches the real file from the path on the
/// card — so repeating a clipped copy of it costs bytes and buys a reader nothing.
#[test]
fn a_read_carries_no_output_block() {
    let mut scratch = scratch_claude(
        "output-read",
        &[
            call("a1", "Read", json!({ "file_path": "/home/u/demo/notes.md" })),
            result("a2", "1\tnew line\n2\t\n", false),
        ],
    );
    let turns = scratch.turns();

    assert!(outputs(&turns).is_empty(), "{:#?}", turns[0].blocks);
    assert_eq!(lines_of(&turns), Some(2), "the count is unchanged either way");
}

/// An error's text is the whole message, whatever the call was.
#[test]
fn a_read_that_failed_carries_its_error() {
    let mut scratch = scratch_claude(
        "output-read-error",
        &[
            call("a1", "Read", json!({ "file_path": "/home/u/demo/gone.md" })),
            result("a2", "File does not exist.", true),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        outputs(&turns),
        ["File does not exist."],
        "{:#?}",
        turns[0].blocks
    );
}

#[test]
fn a_result_with_nothing_in_it_carries_no_block_and_no_count() {
    let mut scratch = scratch_claude(
        "output-empty",
        &[
            call("a1", "Bash", json!({ "command": "touch x" })),
            result("a2", "", false),
        ],
    );
    let turns = scratch.turns();

    assert!(outputs(&turns).is_empty(), "{:#?}", turns[0].blocks);
    assert_eq!(
        lines_of(&turns),
        None,
        "nothing was produced, so nothing is claimed"
    );
}

/// The field is absent on every block that is not a result, which is what makes it additive: an
/// installed client reads a `code` block exactly as it read one before.
#[test]
fn the_role_is_absent_on_an_ordinary_code_block() {
    let block = Block::Code {
        lang: Some("bash".into()),
        text: "echo hi".into(),
        role: None,
    };

    assert_eq!(
        serde_json::to_value(&block).unwrap(),
        json!({ "b": "code", "lang": "bash", "text": "echo hi" })
    );
    assert_eq!(
        serde_json::to_value(Block::Code {
            lang: None,
            text: "hi".into(),
            role: Some(CodeRole::Output),
        })
        .unwrap(),
        json!({ "b": "code", "text": "hi", "role": "output" })
    );
}

// ---- codex ------------------------------------------------------------------------------------

/// The header Codex writes above every command's own output — the chunk id, the wall time, the
/// exit status, the token count — measured on 1706 real `exec_command` results.
fn chunked(body: &str) -> String {
    format!(
        "Chunk ID: a35e7f\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 10\nOutput:\n{body}"
    )
}

fn codex_exec(call: &str, cmd: &str) -> Value {
    json!({ "type": "response_item", "payload": {
        "type": "function_call", "name": "exec_command", "call_id": call,
        "arguments": json!({ "cmd": cmd }).to_string() } })
}

fn codex_output(call: &str, output: &str) -> Value {
    json!({ "type": "response_item", "payload": {
        "type": "function_call_output", "call_id": call, "output": output } })
}

/// The card is what the reader sees first, so it counts the bytes the command produced and not
/// the four lines of bookkeeping Codex wrapped them in.
#[test]
fn a_codex_command_carries_its_output_without_the_header_around_it() {
    let mut scratch = scratch_codex(
        "codex-output",
        &[
            codex_exec("c1", "herdr pane list"),
            codex_output("c1", &chunked("w3:p2\nw3:p3\nw3:p4\n")),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        turns[0].blocks[2],
        Block::Code {
            lang: None,
            text: "w3:p2\nw3:p3\nw3:p4".into(),
            role: Some(CodeRole::Output),
        },
        "{:#?}",
        turns[0].blocks
    );
    assert_eq!(lines_of(&turns), Some(3), "and the count is the output's");
}

/// The same guard the Claude harness carries: a result that lands a second time revises the block
/// already there. Codex keeps its call-to-card map for the life of the parse, so a rollout that
/// records an output twice settles the same card twice.
#[test]
fn a_codex_result_that_lands_twice_carries_one_output_block() {
    let mut scratch = scratch_codex(
        "codex-output-twice",
        &[
            codex_exec("c1", "echo hi"),
            codex_output("c1", &chunked("hi\nthere\n")),
            codex_output("c1", &chunked("hi\nthere\n")),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(outputs(&turns), ["hi\nthere"], "{:#?}", turns[0].blocks);
    assert_eq!(turns[0].blocks.len(), 3, "card, command, result");
}

/// Code mode: the input is JavaScript in a `code` block of its own, so the result has to land
/// *after* it rather than between the card and the script that produced it.
#[test]
fn a_code_mode_script_keeps_its_output_under_the_script() {
    let mut scratch = scratch_codex(
        "codex-code-mode",
        &[
            json!({ "type": "response_item", "payload": {
                "type": "custom_tool_call", "name": "exec", "call_id": "c1",
                "input": "const r = await sh('ls');" } }),
            json!({ "type": "response_item", "payload": {
                "type": "custom_tool_call_output", "call_id": "c1", "output": [
                    { "type": "input_text", "text": "Script completed\nWall time 0.0 seconds\nOutput:\n" },
                    { "type": "input_text", "text": "notes.md\n" } ] } }),
        ],
    );
    let turns = scratch.turns();

    let shape: Vec<String> = turns[0]
        .blocks
        .iter()
        .map(|b| match b {
            Block::Tool { name, .. } => format!("tool {name}"),
            Block::Code { role: None, text, .. } => format!("in {text}"),
            Block::Code { text, .. } => format!("out {text}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        ["tool exec", "in const r = await sh('ls');", "out notes.md"],
        "{:#?}",
        turns[0].blocks
    );
}

/// `apply_patch`'s result is `Success. Updated the following files:` over the `diff` block already
/// beside it, and `update_plan`'s is the literal string `Plan updated` on every one of the 19 this
/// machine has recorded. Neither is worth a page's bytes.
#[test]
fn a_codex_patch_and_a_plan_carry_no_output_block() {
    let mut scratch = scratch_codex(
        "codex-no-output",
        &[
            json!({ "type": "response_item", "payload": {
                "type": "custom_tool_call", "name": "apply_patch", "call_id": "c1",
                "input": "*** Begin Patch\n*** Update File: /home/u/demo/notes.md\n@@\n-old\n+new\n*** End Patch" } }),
            json!({ "type": "response_item", "payload": {
                "type": "custom_tool_call_output", "call_id": "c1",
                "output": "{\"output\": \"Success. Updated the following files:\\nM /home/u/demo/notes.md\\n\", \"metadata\": {\"exit_code\": 0}}" } }),
            json!({ "type": "response_item", "payload": {
                "type": "function_call", "name": "update_plan", "call_id": "c2", "arguments": "{}" } }),
            codex_output("c2", "Plan updated"),
        ],
    );
    let turns = scratch.turns();

    assert!(outputs(&turns).is_empty(), "{turns:#?}");
    assert!(
        diff_blocks(&turns).len() == 1,
        "the patch still has its own better surface"
    );
}

/// An error's text is the whole message, whatever the call was — and a patch that did not apply
/// says why in the same field the successful one wastes.
#[test]
fn a_codex_patch_that_failed_carries_its_error() {
    let mut scratch = scratch_codex(
        "codex-patch-error",
        &[
            json!({ "type": "response_item", "payload": {
                "type": "custom_tool_call", "name": "apply_patch", "call_id": "c1",
                "input": "*** Begin Patch\n*** Update File: /home/u/demo/gone.md\n@@\n-old\n+new\n*** End Patch" } }),
            json!({ "type": "response_item", "payload": {
                "type": "custom_tool_call_output", "call_id": "c1",
                "output": "apply_patch verification failed: /home/u/demo/gone.md does not exist" } }),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        outputs(&turns),
        ["apply_patch verification failed: /home/u/demo/gone.md does not exist"],
        "{:#?}",
        turns[0].blocks
    );
}

/// A `write_stdin` that hit a closed session writes no header at all — no chunk id, no `Output:` —
/// and the whole of what it says is the failure. Stripping a header that is not there would take
/// the message.
#[test]
fn a_result_with_no_header_on_it_is_carried_whole() {
    let mut scratch = scratch_codex(
        "codex-headerless",
        &[
            json!({ "type": "response_item", "payload": {
                "type": "function_call", "name": "write_stdin", "call_id": "c1", "arguments": "{}" } }),
            codex_output(
                "c1",
                "write_stdin failed: stdin is closed for this session; rerun the command",
            ),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        outputs(&turns),
        ["write_stdin failed: stdin is closed for this session; rerun the command"],
        "{:#?}",
        turns[0].blocks
    );
}

// ---- agy --------------------------------------------------------------------------------------

fn agy_call(name: &str, args: Value) -> Value {
    json!({ "source": "MODEL", "type": "PLANNER_RESPONSE",
            "tool_calls": [ { "name": name, "args": args } ] })
}

/// The two stamps every `agy` result opens with, and the exit line a command's result carries
/// under them.
fn agy_result(body: &str) -> Value {
    json!({ "source": "MODEL", "type": "GENERIC", "content":
        format!("Created At: 2026-08-23T01:08:32+10:00\nCompleted At: 2026-08-23T01:08:32+10:00\n{body}") })
}

#[test]
fn an_agy_command_carries_its_output_without_the_exit_line() {
    let mut scratch = scratch_agy(
        "agy-output",
        &[
            agy_call(
                "run_command",
                json!({ "toolSummary": "List the directory", "CommandLine": "ls" }),
            ),
            agy_result("\nThe command exited with code 0.\nOutput:\nnotes.md\nsrc\n"),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        turns[0].blocks[2],
        Block::Code {
            lang: None,
            text: "notes.md\nsrc".into(),
            role: Some(CodeRole::Output),
        },
        "{:#?}",
        turns[0].blocks
    );
    assert_eq!(lines_of(&turns), Some(2), "and the count is the output's");
}

/// `agy` records a shell failure only in the prose of its result, so the text a failed command
/// printed is the whole of what the reader has.
#[test]
fn a_failed_agy_command_carries_what_it_printed() {
    let mut scratch = scratch_agy(
        "agy-error",
        &[
            agy_call(
                "run_command",
                json!({ "CommandLine": "cat /nonexistent/missing" }),
            ),
            agy_result(
                "\nThe command exited with code 1.\nOutput:\ncat: /nonexistent/missing: No such file or directory\n",
            ),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        outputs(&turns),
        ["cat: /nonexistent/missing: No such file or directory"],
        "{:#?}",
        turns[0].blocks
    );
}

/// `view_file`'s result is the file, under a header of its own and with a line number stapled to
/// every line — and the client fetches the real one from the path on the card. `find_by_name` has
/// no such surface, and its result *is* the answer.
#[test]
fn an_agy_file_view_carries_no_output_block_and_a_search_does() {
    let mut scratch = scratch_agy(
        "agy-view",
        &[
            agy_call("view_file", json!({ "AbsolutePath": "/home/u/demo/notes.md" })),
            agy_result("File Path: `file:///home/u/demo/notes.md`\nTotal Lines: 2\n1: one\n2: two"),
            agy_call("find_by_name", json!({ "Pattern": "*.rs" })),
            agy_result("Found 2 results\nsrc/lib.rs\nsrc/main.rs"),
        ],
    );
    let turns = scratch.turns();

    assert_eq!(
        outputs(&turns),
        ["Found 2 results\nsrc/lib.rs\nsrc/main.rs"],
        "{turns:#?}"
    );
}

/// The edit tool puts its diff in the *result*, so the result already has a better surface beside
/// the card and a clipped copy of it under that would be the same thing twice.
#[test]
fn an_agy_edit_carries_its_diff_and_nothing_else() {
    let mut scratch = scratch_agy(
        "agy-edit",
        &[
            agy_call(
                "replace_file_content",
                json!({ "TargetFile": "/home/u/demo/notes.md" }),
            ),
            agy_result(
                "The following changes were made by the replace_file_content tool to: /home/u/demo/notes.md.\n\
                 [diff_block_start]\n@@ -1,1 +1,1 @@\n-old line\n+new line\n[diff_block_end]",
            ),
        ],
    );
    let turns = scratch.turns();

    assert!(outputs(&turns).is_empty(), "{turns:#?}");
    assert_eq!(diff_blocks(&turns).len(), 1);
}
