//! A turn as the pane sees it: the live edge, the result, the tasks and their output.

use kampr_core::conversation::facts::{RunState, TaskState};
use kampr_core::conversation::{Command, Update};
use serde_json::json;

use super::*;

fn states(rig: &Rig) -> Vec<RunState> {
    rig.updates()
        .into_iter()
        .filter_map(|u| match u {
            Update::State(s) => Some(s),
            _ => None,
        })
        .collect()
}

#[test]
fn the_live_edge_maps_an_assistant_and_a_result() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(
        r#"{"type":"assistant","message":{"model":"sonnet","content":[{"type":"text","text":"hello"}]}}"#,
    );
    rig.line(r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.012,"usage":{"input_tokens":10,"output_tokens":5}}"#);
    let updates = rig.updates();
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, Update::Live(l) if l.text == "hello"))
    );
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, Update::Cost(c) if (c.usd - 0.012).abs() < 1e-9 && c.input_tokens == 10))
    );
    assert_eq!(states(&rig).last(), Some(&RunState::Idle));
}

#[test]
fn a_status_says_whether_the_turn_runs_and_a_mode_change_alone_does_not_stop_it() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"system","subtype":"status","status":"requesting"}"#);
    assert_eq!(states(&rig).last(), Some(&RunState::Running));
    rig.line(r#"{"type":"system","subtype":"status","status":null,"permissionMode":"plan"}"#);
    assert_eq!(
        states(&rig).last(),
        Some(&RunState::Running),
        "a mode change is not an idle turn"
    );
    let mode = rig.updates().into_iter().rev().find_map(|u| match u {
        Update::Session(s) => s.mode,
        _ => None,
    });
    assert_eq!(mode.as_deref(), Some("plan"));
    rig.line(r#"{"type":"system","subtype":"status","status":"compacting"}"#);
    assert_eq!(states(&rig).last(), Some(&RunState::Running));
}

#[test]
fn thinking_is_told_as_a_token_estimate() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"system","subtype":"thinking_tokens","tokens":120}"#);
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::Live(l) if l.thinking == Some(120)))
    );
}

#[test]
fn a_failing_turn_is_a_trouble_not_an_answer_and_the_next_good_one_clears_it() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"result","subtype":"success","is_error":true,"error":"API Error: 401 authentication_error"}"#);
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::Trouble(Some(t)) if t.kind == "api" && t.text.contains("401")))
    );
    rig.line(RESULT_USER);
    assert!(matches!(
        rig.updates().iter().rev().find_map(|u| match u {
            Update::Trouble(t) => Some(t.clone()),
            _ => None,
        }),
        Some(None)
    ));
}

#[test]
fn an_api_retry_is_a_trouble_with_its_attempt() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"system","subtype":"api_retry","attempt":2,"max_retries":4,"retry_delay_ms":1500,"error_status":529,"error":"overloaded"}"#);
    let t = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Trouble(Some(t)) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (
            t.retry.unwrap().attempt,
            t.retry.unwrap().of,
            t.retry.unwrap().in_ms
        ),
        (2, 4, 1500)
    );
}

#[test]
fn a_subagents_live_line_is_filed_under_its_name_not_the_parents() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"subagent_live","parent_tool_use_id":"tu1","task_id":"t1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"digging"}}}"#);
    let updates = rig.updates();
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, Update::SubLive { id, live } if id == "tu1" && live.text == "digging"))
    );
    assert!(!updates.iter().any(|u| matches!(u, Update::Live(_))));

    // The task's id stands in when no call is named, and a tool start is a tool the pane draws.
    rig.line(r#"{"type":"subagent_live","task_id":"t2","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu9","name":"Read"}}}"#);
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::SubLive { id, live } if id == "t2" && live.tools[0].tool == "Read"))
    );

    // Named by neither: dropped, not guessed at.
    let before = rig.updates().len();
    rig.line(r#"{"type":"subagent_live","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"?"}}}"#);
    assert_eq!(rig.updates().len(), before);
}

// #650: a subagent's own messages carry its call's id on the parent's stream.
#[test]
fn a_subagents_assistant_message_is_not_the_parents_words() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"assistant","parent_tool_use_id":"tu1","message":{"content":[{"type":"text","text":"found it"}]}}"#);
    let updates = rig.updates();
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, Update::SubLive { id, live } if id == "tu1" && live.text == "found it"))
    );
    assert!(!updates.iter().any(|u| matches!(u, Update::Live(_))));
}

#[test]
fn a_session_id_the_child_reports_is_what_the_pane_is_told() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&TURN_INIT.replace("\"s1\"", "\"s2\""));
    let id = rig
        .updates()
        .into_iter()
        .rev()
        .find_map(|u| match u {
            Update::Session(s) => Some(s.id),
            _ => None,
        })
        .unwrap();
    assert_eq!(id, "s2");
}

const TASK: &str = r#"{"type":"system","subtype":"task_started","task_id":"t1","tool_use_id":"tu1","task_type":"local_bash","is_backgrounded":true,"prompt":"for i in 1 2 3; do echo tick; done"}"#;

#[test]
fn a_task_is_listed_with_its_call_and_marked_ended_when_it_is() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(TASK);
    let tasks = rig
        .updates()
        .into_iter()
        .rev()
        .find_map(|u| match u {
            Update::Tasks(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (
            tasks[0].id.as_str(),
            tasks[0].call.as_deref(),
            tasks[0].backgrounded
        ),
        ("t1", Some("tu1"), true)
    );
    rig.line(r#"{"type":"system","subtype":"task_updated","task_id":"t1","status":"completed"}"#);
    let ended = rig
        .updates()
        .into_iter()
        .rev()
        .find_map(|u| match u {
            Update::Tasks(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(ended[0].state, TaskState::Ended);
    assert_eq!(
        ended[0].call.as_deref(),
        Some("tu1"),
        "an update that names no call keeps the one it had"
    );
}

// #647: the binary answers with the tail of the output and its total, so following is polling.
#[test]
fn a_followed_task_is_polled_and_its_output_reaches_the_pane_only_when_it_grows() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(TASK);
    rig.command(Command::Task {
        id: "t1".into(),
        from: 0,
    });
    let first = rig.requests("get_task_output");
    assert_eq!(first[0]["request"]["task_id"], "t1");
    rig.line(&response(
        first[0]["request_id"].as_str().unwrap(),
        json!({"output": "tick 1\ntick 2\n", "total_bytes": 14, "truncated": false}),
    ));
    let output = |rig: &Rig| -> Vec<(u64, String)> {
        rig.updates()
            .into_iter()
            .filter_map(|u| match u {
                Update::TaskOutput {
                    at,
                    text,
                    ended: None,
                    ..
                } => Some((at, text)),
                _ => None,
            })
            .collect()
    };
    assert_eq!(output(&rig), [(0, "tick 1\ntick 2\n".to_string())]);

    rig.machine.tick();
    let again = rig.requests("get_task_output");
    assert_eq!(again.len(), 2, "a followed task is read again each tick");
    rig.line(&response(
        again[1]["request_id"].as_str().unwrap(),
        json!({"output": "tick 1\ntick 2\n", "total_bytes": 14, "truncated": false}),
    ));
    assert_eq!(output(&rig).len(), 1, "nothing grew, nothing is said");

    rig.machine.tick();
    let third = rig.requests("get_task_output");
    rig.line(&response(
        third[2]["request_id"].as_str().unwrap(),
        json!({"output": "tick 3\n", "total_bytes": 21, "truncated": false}),
    ));
    assert_eq!(
        output(&rig)[1],
        (14, "tick 3\n".to_string()),
        "the tail's offset is the total less its length"
    );
}

#[test]
fn an_unfollowed_task_is_not_polled_and_a_finished_one_says_so() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(TASK);
    rig.command(Command::Task {
        id: "t1".into(),
        from: 0,
    });
    rig.command(Command::Unfollow { id: "t1".into() });
    let polls = rig.requests("get_task_output").len();
    rig.machine.tick();
    assert_eq!(rig.requests("get_task_output").len(), polls);

    rig.command(Command::Task {
        id: "t1".into(),
        from: 0,
    });
    rig.line(r#"{"type":"system","subtype":"task_notification","task_id":"t1","status":"completed"}"#);
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::TaskOutput { ended: Some(e), .. } if e.ok))
    );
    let polls = rig.requests("get_task_output").len();
    rig.machine.tick();
    assert_eq!(
        rig.requests("get_task_output").len(),
        polls,
        "a task that ended is not polled again"
    );
}

#[test]
fn what_the_source_declares_is_what_it_does() {
    use kampr_core::conversation::abilities::{AskAbility, Stream};
    let rig = Rig::opened(Opts::default());
    let abilities = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Abilities(a) => Some(a),
            _ => None,
        })
        .unwrap();
    assert!(abilities.streams.contains(&Stream::SubLive) && abilities.streams.contains(&Stream::TaskOutput));
    for ask in [
        AskAbility::Question,
        AskAbility::Plan,
        AskAbility::Form,
        AskAbility::EditInput,
    ] {
        assert!(abilities.asks.contains(&ask), "{ask:?}");
    }
}
