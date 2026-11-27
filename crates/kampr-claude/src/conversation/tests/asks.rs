//! What a person's answer becomes on the child's stdin. Every body is checked against the
//! binary's own schema (`strict`): the mock this crate used to answer to accepted anything,
//! `updatedInput: null` among it.

use kampr_core::conversation::{Command, Reply, Update};
use serde_json::{Value, json};

use super::*;
use crate::strict;

fn answer(rig: &mut Rig, ask: &str, reply: Reply) {
    rig.command(Command::Answer {
        ask: ask.into(),
        reply,
    });
}

fn body(rig: &Rig) -> Value {
    let answers = rig.answers();
    let last = answers
        .last()
        .unwrap_or_else(|| panic!("nothing was answered: {:?}", rig.written()));
    last["response"]["response"].clone()
}

fn allowed() -> Reply {
    Reply::Allow {
        input: None,
        remember: false,
    }
}

#[test]
fn an_allow_echoes_the_requests_own_input() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_bash("r1"));
    assert_eq!(rig.last_asks()[0].id.as_deref(), Some("r1"));
    answer(&mut rig, "r1", allowed());
    strict::permission_answer(rig.answers().last().unwrap()).expect("a body the binary accepts");
    assert_eq!(body(&rig)["behavior"], "allow");
    assert_eq!(body(&rig)["updatedInput"], json!({"command": "touch probe-file"}));
    assert!(
        rig.last_asks().is_empty(),
        "an answered request is no longer asked"
    );
}

#[test]
fn an_allow_with_edited_input_sends_the_edit() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_bash("r1"));
    answer(
        &mut rig,
        "r1",
        Reply::Allow {
            input: Some(json!({"command": "ls"})),
            remember: false,
        },
    );
    assert_eq!(body(&rig)["updatedInput"], json!({"command": "ls"}));
}

#[test]
fn an_always_applies_the_requests_own_suggestions() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_bash("r1"));
    assert!(
        rig.last_asks()[0]
            .options
            .iter()
            .any(|o| o.label == "Always allow")
    );
    answer(
        &mut rig,
        "r1",
        Reply::Allow {
            input: None,
            remember: true,
        },
    );
    strict::permission_answer(rig.answers().last().unwrap()).unwrap();
    assert_eq!(body(&rig)["updatedPermissions"][0]["mode"], "acceptEdits");

    let mut plain = Rig::allowed(Opts::default());
    plain.line(&can_use_bash("r2"));
    answer(&mut plain, "r2", allowed());
    assert!(body(&plain).get("updatedPermissions").is_none());
}

#[test]
fn a_deny_carries_a_message_the_model_reads() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_bash("r1"));
    answer(&mut rig, "r1", Reply::Deny { message: None });
    strict::permission_answer(rig.answers().last().unwrap()).unwrap();
    assert!(!body(&rig)["message"].as_str().unwrap().is_empty());

    rig.line(&can_use_bash("r2"));
    answer(
        &mut rig,
        "r2",
        Reply::Deny {
            message: Some("not now".into()),
        },
    );
    assert_eq!(body(&rig)["message"], "not now");
}

// #645: the answers go in `updatedInput.answers`, keyed by the question, a multi-select joined
// with ", " (#634).
#[test]
fn a_question_is_answered_through_updated_input_answers_once_every_part_is() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_question("q1"));
    let asks = rig.last_asks();
    assert_eq!(asks.len(), 2, "each question is an ask: {asks:?}");
    assert_eq!(asks[0].options[1].label, "Blue");
    assert!(asks[1].multi);

    answer(
        &mut rig,
        "q1#0",
        Reply::Choose {
            answers: vec!["Blue".into()],
        },
    );
    assert!(rig.answers().is_empty(), "the binary waits for every question");
    assert_eq!(rig.last_asks().len(), 1);

    answer(
        &mut rig,
        "q1#1",
        Reply::Choose {
            answers: vec!["Apple".into(), "Cherry".into()],
        },
    );
    strict::permission_answer(rig.answers().last().unwrap()).unwrap();
    let input = &body(&rig)["updatedInput"];
    assert_eq!(
        input["answers"],
        json!({"Which colour?": "Blue", "Which fruits?": "Apple, Cherry"})
    );
    assert_eq!(input["questions"].as_array().unwrap().len(), 2);
    assert!(rig.last_asks().is_empty());
}

#[test]
fn a_question_that_is_not_answered_with_a_choice_stays_standing() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_question("q1"));
    answer(&mut rig, "q1#0", allowed());
    assert!(rig.answers().is_empty());
    assert_eq!(rig.last_asks().len(), 2);
    assert!(!rig.notices().is_empty());
    answer(&mut rig, "q1#0", Reply::Dismiss);
    assert_eq!(body(&rig)["behavior"], "deny");
}

// #674, #635: approval is an allow with the plan's own input; the mode it leaves for rides as a
// `setMode`.
#[test]
fn a_plan_is_approved_with_its_input_and_a_mode_or_sent_back_with_feedback() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_plan("p1"));
    let ask = &rig.last_asks()[0];
    assert_eq!(ask.context.as_deref(), Some("1. echo planned"));
    answer(
        &mut rig,
        "p1",
        Reply::Plan {
            approve: true,
            mode: Some("default".into()),
            feedback: None,
        },
    );
    strict::permission_answer(rig.answers().last().unwrap()).unwrap();
    assert_eq!(body(&rig)["updatedInput"]["plan"], "1. echo planned");
    assert_eq!(
        body(&rig)["updatedPermissions"][0],
        json!({"type":"setMode","mode":"default","destination":"session"})
    );

    rig.line(&can_use_plan("p2"));
    answer(
        &mut rig,
        "p2",
        Reply::Plan {
            approve: false,
            mode: None,
            feedback: Some("say goodbye".into()),
        },
    );
    strict::permission_answer(rig.answers().last().unwrap()).unwrap();
    assert_eq!(body(&rig)["message"], "say goodbye");
}

// The hooked plan card's three answers, and the mode each leaves plan mode for (#635).
#[test]
fn a_plan_offers_the_hooked_cards_answers_and_each_press_sets_its_mode() {
    let cases = [
        ("1", None, Some("acceptEdits")),
        ("2", None, Some("default")),
        ("3", Some("tighten step two"), None),
    ];
    for (key, words, mode) in cases {
        let mut rig = Rig::allowed(Opts::default());
        rig.line(&can_use_plan("p1"));
        let ask = &rig.last_asks()[0];
        let offered: Vec<(&str, &str, bool)> = ask
            .options
            .iter()
            .map(|o| (o.key.as_str(), o.label.as_str(), o.text))
            .collect();
        assert_eq!(
            offered,
            [
                ("1", "Yes, auto-accept edits", false),
                ("2", "Yes, manually approve edits", false),
                ("3", "No, tell Claude what to change", true),
            ]
        );
        rig.command(Command::Press {
            ask: "p1".into(),
            key: key.into(),
            text: words.map(str::to_string),
        });
        strict::permission_answer(rig.answers().last().unwrap()).unwrap();
        match mode {
            Some(mode) => {
                assert_eq!(body(&rig)["updatedInput"]["plan"], "1. echo planned");
                assert_eq!(
                    body(&rig)["updatedPermissions"],
                    json!([{"type":"setMode","mode":mode,"destination":"session"}])
                );
            }
            None => assert_eq!(body(&rig)["message"], "tighten step two"),
        }
    }
}

// #684: `{action, content}`, and a content only on accept.
#[test]
fn a_form_is_answered_with_an_action_and_its_values() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&elicitation("e1"));
    let ask = &rig.last_asks()[0];
    assert_eq!(ask.question, "What is the answer?");
    assert_eq!(ask.by.as_deref(), Some("elicit-probe"));
    answer(
        &mut rig,
        "e1",
        Reply::Form {
            values: json!({"answer": "forty-two"}),
        },
    );
    assert_eq!(
        body(&rig),
        json!({"action": "accept", "content": {"answer": "forty-two"}})
    );

    rig.line(&elicitation("e2"));
    answer(&mut rig, "e2", Reply::Deny { message: None });
    assert_eq!(body(&rig), json!({"action": "decline"}));
    rig.line(&elicitation("e3"));
    answer(&mut rig, "e3", Reply::Dismiss);
    assert_eq!(body(&rig), json!({"action": "cancel"}));
}

#[test]
fn a_request_this_source_cannot_answer_is_answered_with_an_error_not_left_open() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"control_request","request_id":"h1","request":{"subtype":"hook_callback","callback_id":"c"}}"#);
    let answers = rig.answers();
    assert_eq!(answers[0]["response"]["subtype"], "error");
    assert_eq!(answers[0]["response"]["request_id"], "h1");
    assert!(rig.last_asks().is_empty());
}

#[test]
fn an_answer_that_names_nothing_standing_is_said_and_writes_nothing() {
    let mut rig = Rig::allowed(Opts::default());
    answer(&mut rig, "nope", allowed());
    assert!(rig.answers().is_empty());
    assert!(!rig.notices().is_empty());
}

#[test]
fn an_old_client_that_names_no_ask_answers_the_oldest() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&can_use_bash("r1"));
    rig.line(&can_use_bash("r2"));
    assert_eq!(
        rig.last_asks().len(),
        2,
        "a request standing beside another is asked too"
    );
    answer(&mut rig, "", allowed());
    assert_eq!(rig.answers().last().unwrap()["response"]["request_id"], "r1");
    assert!(matches!(rig.updates().last(), Some(Update::Asks(a)) if a.len() == 1));
}

#[test]
fn a_subagents_request_says_whose_it_is() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(r#"{"type":"control_request","request_id":"r9","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{},"agent_id":"a7"}}"#);
    assert_eq!(rig.last_asks()[0].by.as_deref(), Some("a7"));
}
