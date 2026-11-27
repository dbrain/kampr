//! What the pane's commands become on the child's stdin.

use std::sync::atomic::Ordering;

use kampr_core::conversation::abilities::{Fact, Steer};
use kampr_core::conversation::{Command, FileRef, SuggestKind, Suggestion, Update, When};
use serde_json::{Value, json};

use super::*;

fn controls(rig: &Rig) -> Vec<kampr_core::conversation::Control> {
    rig.updates()
        .into_iter()
        .rev()
        .find_map(|u| match u {
            Update::Controls(c) => Some(c),
            _ => None,
        })
        .expect("controls")
}

fn value_of(rig: &Rig, id: &str) -> Option<Value> {
    controls(rig)
        .into_iter()
        .find(|c| c.id == id)
        .and_then(|c| c.value)
}

fn set(rig: &mut Rig, control: &str, value: Value) {
    rig.command(Command::Set {
        control: control.into(),
        value,
    });
}

#[test]
fn a_model_is_set_with_set_model_and_only_from_the_list() {
    let mut rig = Rig::allowed(Opts::default());
    set(&mut rig, "model", json!("haiku"));
    let sent = rig.requests("set_model");
    assert_eq!(sent[0]["request"]["model"], "haiku");
    rig.line(&response(sent[0]["request_id"].as_str().unwrap(), json!({})));
    assert_eq!(value_of(&rig, "model"), Some(json!("haiku")));

    set(&mut rig, "model", json!("gpt-9"));
    assert_eq!(
        rig.requests("set_model").len(),
        1,
        "a model nobody listed was sent"
    );
    assert!(!rig.notices().is_empty());
}

#[test]
fn the_model_control_lists_what_initialize_listed_and_names_the_current_one() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(TURN_INIT);
    let model = controls(&rig).into_iter().find(|c| c.id == "model").unwrap();
    let values: Vec<_> = model.options.iter().map(|o| o.value.as_str()).collect();
    assert_eq!(values, ["default", "haiku"]);
    assert_eq!(
        model.value,
        Some(json!("default")),
        "claude-opus-5-5 is what default resolves to"
    );
}

#[test]
fn a_permission_mode_is_set_and_bypass_is_a_separate_toggle() {
    let mut rig = Rig::allowed(Opts::default());
    set(&mut rig, "mode", json!("plan"));
    let sent = rig.requests("set_permission_mode");
    assert_eq!(sent[0]["request"]["mode"], "plan");
    rig.line(&response(
        sent[0]["request_id"].as_str().unwrap(),
        json!({"mode": "plan"}),
    ));
    assert_eq!(value_of(&rig, "mode"), Some(json!("plan")));

    set(&mut rig, "mode", json!("bypassPermissions"));
    assert_eq!(
        rig.requests("set_permission_mode").len(),
        1,
        "bypass is not a mode this control sets"
    );

    set(&mut rig, "bypass", json!(true));
    let sent = rig.requests("set_permission_mode");
    assert_eq!(sent[1]["request"]["mode"], "bypassPermissions");
    rig.line(&response(sent[1]["request_id"].as_str().unwrap(), json!({})));
    assert_eq!(value_of(&rig, "bypass"), Some(json!(true)));
    set(&mut rig, "bypass", json!(false));
    assert_eq!(
        rig.requests("set_permission_mode")[2]["request"]["mode"],
        "default"
    );
}

// The binary has no `set_fast_mode`: its schema takes fast mode as a setting.
#[test]
fn fast_mode_is_a_setting_not_a_request_of_its_own() {
    let mut rig = Rig::allowed(Opts::default());
    set(&mut rig, "fast", json!(true));
    assert!(rig.requests("set_fast_mode").is_empty());
    let sent = rig.requests("apply_flag_settings");
    assert_eq!(sent[0]["request"]["settings"], json!({"fastMode": true}));
    rig.line(&response(sent[0]["request_id"].as_str().unwrap(), json!({})));
    assert_eq!(value_of(&rig, "fast"), Some(json!(true)));
}

#[test]
fn fast_mode_is_not_offered_where_the_session_says_it_needs_an_opt_in() {
    let mut rig = Rig::started(Opts::default());
    rig.line(&initialize_response_sdk("t.1"));
    assert_eq!(value_of(&rig, "fast"), None);
    assert!(controls(&rig).iter().all(|c| c.id != "fast"));
}

#[test]
fn a_setting_the_session_refuses_is_said_and_not_shown_as_changed() {
    let mut rig = Rig::allowed(Opts::default());
    set(&mut rig, "fast", json!(true));
    let id = rig.request_id("apply_flag_settings");
    rig.line(&error_response(&id, "fast mode is not available"));
    assert_eq!(value_of(&rig, "fast"), Some(json!(false)));
    assert!(
        rig.notices()
            .iter()
            .any(|n| n.contains("fast mode is not available")),
        "{:?}",
        rig.notices()
    );
}

#[test]
fn a_control_nobody_listed_is_refused() {
    let mut rig = Rig::allowed(Opts::default());
    let before = rig.written().len();
    set(&mut rig, "effort", json!("high"));
    assert_eq!(rig.written().len(), before);
    assert!(!rig.notices().is_empty());
}

// #677: a slash command is read off a user line, not off a control request.
#[test]
fn an_action_is_a_slash_command_sent_as_the_text_of_a_prompt() {
    let mut rig = Rig::allowed(Opts::default());
    rig.command(Command::Act {
        action: "compact".into(),
        args: Value::Null,
    });
    rig.command(Command::Act {
        action: "model".into(),
        args: json!("sonnet"),
    });
    let texts: Vec<_> = rig
        .words()
        .iter()
        .map(|w| w["message"]["content"].clone())
        .collect();
    assert_eq!(texts, [json!("/compact"), json!("/model sonnet")]);
    assert!(rig.requests("compact").is_empty() && rig.requests("model").is_empty());
}

#[test]
fn an_action_waits_for_the_account_check_like_any_other_word() {
    let mut rig = Rig::started(Opts::default());
    rig.command(Command::Act {
        action: "compact".into(),
        args: Value::Null,
    });
    assert!(rig.words().is_empty());
    rig.line(&initialize_response("t.1", PLAN_ACCOUNT));
    assert_eq!(rig.words().len(), 1);
}

#[test]
fn the_actions_are_the_commands_the_session_listed() {
    let rig = Rig::allowed(Opts::default());
    let actions = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Actions(a) => Some(a),
            _ => None,
        })
        .expect("actions");
    assert_eq!(
        actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["compact", "model"]
    );
    assert_eq!(actions[0].label, "/compact");
    assert_eq!(actions[0].detail.as_deref(), Some("shrink the context"));
}

#[test]
fn when_the_pane_chose_rides_on_the_user_line() {
    for (when, priority) in [
        (When::Now, Some("now")),
        (When::Next, Some("next")),
        (When::Idle, None),
    ] {
        let mut rig = Rig::allowed(Opts::default());
        rig.command(Command::Input {
            text: "go".into(),
            msg_id: "m1".into(),
            images: vec![],
            files: vec![],
            when,
        });
        let word = &rig.words()[0];
        assert_eq!(word["uuid"], "m1");
        assert_eq!(word["priority"].as_str(), priority);
    }
}

#[test]
fn files_become_at_refs_and_images_become_blocks() {
    let mut rig = Rig::allowed(Opts::default());
    rig.command(Command::Input {
        text: "read this".into(),
        msg_id: "m1".into(),
        images: vec![kampr_core::conversation::Image {
            mime: "image/png".into(),
            data: "AAAA".into(),
        }],
        files: vec![FileRef {
            path: "/tmp/a.txt".into(),
        }],
        when: When::Idle,
    });
    let content = &rig.words()[0]["message"]["content"];
    assert_eq!(content[0]["text"], "read this @/tmp/a.txt");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
}

#[test]
fn an_unqueue_cancels_the_queued_message_and_drops_a_held_copy() {
    let mut rig = Rig::allowed(Opts::default());
    rig.command(Command::Unqueue { msg_id: "m9".into() });
    let sent = rig.requests("cancel_async_message");
    assert_eq!(sent[0]["request"]["message_uuid"], "m9");

    let mut held = Rig::started(Opts::default());
    held.command(input("held"));
    held.command(Command::Unqueue { msg_id: "m1".into() });
    held.line(&initialize_response("t.1", PLAN_ACCOUNT));
    assert!(held.words().is_empty(), "a word the pane took back was sent");
}

#[test]
fn an_interrupt_is_the_binarys_own_and_the_state_comes_back_from_the_turn() {
    let mut rig = Rig::allowed(Opts::default());
    rig.clear();
    rig.command(Command::Steer(Steer::Interrupt, None));
    assert_eq!(rig.requests("interrupt").len(), 1);
    assert!(
        !rig.updates().iter().any(|u| matches!(u, Update::State(_))),
        "{:?}",
        rig.updates()
    );
}

#[test]
fn a_steer_the_source_did_not_declare_writes_nothing() {
    let mut rig = Rig::allowed(Opts::default());
    let before = rig.written().len();
    rig.command(Command::Steer(Steer::Continue, Some("go on".into())));
    rig.command(Command::Steer(Steer::Stop, None));
    assert_eq!(rig.written().len(), before);
}

#[test]
fn a_command_completes_from_the_catalog_the_session_listed() {
    let mut rig = Rig::allowed(Opts::default());
    rig.command(Command::Suggest {
        kind: SuggestKind::Commands,
        query: "CO".into(),
    });
    let items = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Suggested { items, .. } => Some(items),
            _ => None,
        })
        .expect("suggestions");
    assert_eq!(
        items,
        [Suggestion {
            id: "/compact".into(),
            label: "/compact".into(),
            detail: Some("shrink the context".into()),
        }]
    );
}

#[test]
fn a_file_completion_asks_the_binary_and_its_answer_reaches_the_pane() {
    let mut rig = Rig::allowed(Opts::default());
    rig.command(Command::Suggest {
        kind: SuggestKind::Files,
        query: "mock".into(),
    });
    let sent = rig.requests("file_suggestions");
    assert_eq!(sent[0]["request"]["query"], "mock");
    rig.line(&response(
        sent[0]["request_id"].as_str().unwrap(),
        json!({"suggestions": [{"path": "mock.py", "name": "mock.py"}]}),
    ));
    let items = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Suggested { items, query, .. } if query == "mock" => Some(items),
            _ => None,
        })
        .expect("suggestions");
    assert_eq!(items[0].label, "mock.py");
}

#[test]
fn the_context_is_read_at_open_and_after_a_turn_and_on_refresh() {
    let mut rig = Rig::allowed(Opts::default());
    assert_eq!(rig.requests("get_context_usage").len(), 1);
    rig.line(RESULT_USER);
    assert_eq!(rig.requests("get_context_usage").len(), 2);
    rig.command(Command::Refresh(Fact::Context));
    assert_eq!(rig.requests("get_context_usage").len(), 3);

    let id = rig.request_id("get_context_usage");
    rig.line(&response(
        &id,
        json!({"totalTokens": 17172, "maxTokens": 1000000}),
    ));
    let context = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Context(c) => Some(c),
            _ => None,
        })
        .expect("context");
    assert_eq!((context.used, context.total), (17172, 1000000));
}

// Probe #668: the older binary's shape.
#[test]
fn a_context_answer_in_the_breakdown_shape_reaches_the_pane() {
    let mut rig = Rig::allowed(Opts::default());
    let id = rig.request_id("get_context_usage");
    rig.line(&response(
        &id,
        json!({"context_breakdown": {"input_tokens": 10, "output_tokens": 5, "max_tokens": 200000}}),
    ));
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::Context(c) if (c.used, c.total) == (15, 200000)))
    );
}

#[test]
fn a_word_that_did_not_reach_the_child_is_said() {
    let mut rig = Rig::allowed(Opts::default());
    rig.refusing.store(true, Ordering::SeqCst);
    rig.command(input("lost?"));
    assert!(
        rig.notices().iter().any(|n| n.contains("did not take")),
        "a send that failed was swallowed: {:?}",
        rig.notices()
    );
}

#[test]
fn a_word_to_a_child_that_has_gone_is_left_to_its_exit_to_explain() {
    let mut rig = Rig::allowed(Opts::default());
    rig.exited.store(true, Ordering::SeqCst);
    rig.command(input("anyone?"));
    assert!(rig.notices().is_empty(), "{:?}", rig.notices());
}

#[test]
fn a_word_to_a_session_that_was_refused_is_not_sent() {
    let mut rig = Rig::started(Opts::default());
    rig.line(&initialize_response("t.1", KEY_ACCOUNT));
    let before = rig.written().len();
    rig.command(input("hello"));
    assert_eq!(rig.written().len(), before);
}
