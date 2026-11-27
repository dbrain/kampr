//! A conversation opens on the log of everything the child has said, because the keeper outlives
//! the node: the replay rebuilds what stands and says nothing to the child on the way.

use kampr_core::conversation::Update;
use kampr_core::conversation::facts::RunState;
use serde_json::json;

use super::*;

fn past_request(id: &str, subtype: &str) -> Item {
    Item::Past(json!({"type":"control_request","request_id":id,"request":{"subtype":subtype}}).to_string())
}

fn past_answer(id: &str) -> Item {
    Item::Past(
        json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":{"behavior":"allow","updatedInput":{}}}})
            .to_string(),
    )
}

/// What an earlier run of this node left in the log: an `initialize` and its plan answer.
fn earlier_run(rig: &mut Rig) {
    rig.item(past_request("old.1", "initialize"));
    rig.line(&initialize_response("old.1", PLAN_ACCOUNT));
}

#[test]
fn a_request_already_answered_does_not_come_back_as_an_ask() {
    let mut rig = Rig::opened(Opts::default());
    earlier_run(&mut rig);
    rig.line(&can_use_bash("r1"));
    rig.item(past_answer("r1"));
    rig.item(Item::Replayed { alive: true });
    assert!(
        !rig.updates().iter().any(|u| matches!(u, Update::Asks(_))),
        "an answered request was asked again: {:?}",
        rig.updates()
    );
}

#[test]
fn a_request_still_standing_is_asked_once_the_replay_is_over() {
    let mut rig = Rig::opened(Opts::default());
    earlier_run(&mut rig);
    rig.line(&can_use_bash("r1"));
    rig.line(&can_use_bash("r2"));
    rig.item(past_answer("r1"));
    assert!(
        !rig.updates().iter().any(|u| matches!(u, Update::Asks(_))),
        "said before the replay ended"
    );
    rig.item(Item::Replayed { alive: true });
    let asks = rig.last_asks();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].id.as_deref(), Some("r2"));
}

#[test]
fn a_replay_writes_nothing_to_the_child_but_one_read_at_the_end() {
    let mut rig = Rig::opened(Opts::default());
    earlier_run(&mut rig);
    for _ in 0..3 {
        rig.line(RESULT_USER);
    }
    assert!(
        rig.written().is_empty(),
        "the replay wrote to the child: {:?}",
        rig.written()
    );
    rig.item(Item::Replayed { alive: true });
    assert_eq!(rig.requests("get_context_usage").len(), 1);
    assert_eq!(rig.requests("get_usage").len(), 1);
    assert!(
        rig.requests("initialize").is_empty(),
        "the session was initialised by an earlier run"
    );
}

#[test]
fn a_replay_says_what_it_rebuilt_once() {
    let mut rig = Rig::opened(Opts::default());
    earlier_run(&mut rig);
    rig.line(RESULT_USER);
    rig.line(r#"{"type":"system","subtype":"task_started","task_id":"t1","task_type":"local_bash","is_backgrounded":true}"#);
    rig.item(Item::Replayed { alive: true });
    let updates = rig.updates();
    let count = |f: fn(&Update) -> bool| updates.iter().filter(|u| f(u)).count();
    assert_eq!(count(|u| matches!(u, Update::Session(_))), 1);
    assert_eq!(count(|u| matches!(u, Update::Controls(_))), 1);
    assert_eq!(count(|u| matches!(u, Update::Tasks(t) if t.len() == 1)), 1);
    assert_eq!(count(|u| matches!(u, Update::Cost(_))), 1);
    assert_eq!(count(|u| matches!(u, Update::Live(_))), 0);
}

#[test]
fn a_replay_still_stops_what_a_guard_stops() {
    let mut rig = Rig::opened(Opts::default());
    rig.item(past_request("old.1", "initialize"));
    rig.line(&initialize_response("old.1", KEY_ACCOUNT));
    assert!(
        rig.closed(),
        "an earlier run died before it ended a session paid by a key"
    );
    assert!(rig.machine.done());
}

#[test]
fn a_session_a_guard_ended_says_why_when_it_is_opened_again() {
    let mut rig = Rig::opened(Opts::default());
    rig.item(past_request("old.1", "initialize"));
    rig.line(&initialize_response("old.1", KEY_ACCOUNT));
    rig.item(Item::Replayed { alive: false });
    rig.item(Item::Exited);
    assert!(rig.billing_trouble().unwrap().contains("key in the environment"));
}

#[test]
fn a_child_that_is_gone_is_ended_and_nothing_is_sent_to_it() {
    let mut rig = Rig::opened(Opts::default());
    earlier_run(&mut rig);
    rig.item(Item::Replayed { alive: false });
    rig.item(Item::Exited);
    assert!(rig.written().is_empty(), "{:?}", rig.written());
    assert!(matches!(
        rig.updates().last(),
        Some(Update::State(RunState::Ended))
    ));
    assert!(rig.machine.done());
}

#[test]
fn a_session_that_ended_while_nobody_watched_is_not_a_pane_that_hangs() {
    let mut rig = Rig::opened(Opts::default());
    rig.item(Item::Replayed { alive: false });
    rig.item(Item::Exited);
    assert!(
        rig.machine.done(),
        "the conversation waits on a child that will never speak"
    );
}

// Request ids of an earlier run are in the log, and an answer to one is not this run's.
#[test]
fn an_answer_to_a_request_this_run_never_sent_is_not_read() {
    let mut rig = Rig::allowed(Opts::default());
    rig.line(&response("old.7", json!({"totalTokens": 5, "maxTokens": 10})));
    assert!(!rig.updates().iter().any(|u| matches!(u, Update::Context(_))));
}

#[test]
fn a_session_opened_with_a_prompt_in_hand_sends_it_after_the_replay_when_allowed() {
    let mut rig = Rig::opened(Opts {
        opening: Some("start here".into()),
        ..Opts::default()
    });
    earlier_run(&mut rig);
    rig.item(Item::Replayed { alive: true });
    assert_eq!(rig.words()[0]["message"]["content"], "start here");
}

// A replay reads what already happened: stopping a turn then would stop whatever is running now.
#[test]
fn a_held_overage_found_in_the_log_is_asked_again_and_interrupts_nothing() {
    let mut rig = Rig::opened(Opts {
        overage: crate::config::Overage::Ask,
        ..Opts::default()
    });
    earlier_run(&mut rig);
    rig.line(r#"{"type":"system","subtype":"status","status":"requesting"}"#);
    rig.line(RATE_OVERAGE);
    rig.item(Item::Replayed { alive: true });
    assert!(rig.requests("interrupt").is_empty(), "{:?}", rig.written());
    assert!(rig.last_asks().iter().any(|a| a.id.as_deref() == Some("overage")));
}
