//! The billing guards (docs/21): each one has to stop the child, not only say it did, so every
//! test reads what was written to the child and whether it was hung up on.

use kampr_core::conversation::facts::{Fix, RunState};
use kampr_core::conversation::{Command, Reply, Update};
use serde_json::json;

use super::*;
use crate::config::{Billing, Overage};

fn opts(policy: Billing, overage: Overage, cap: u32) -> Opts {
    Opts {
        policy,
        overage,
        cap,
        opening: None,
    }
}

fn plan() -> Opts {
    opts(Billing::Plan, Overage::Refuse, 20)
}

/// A turn in flight: the operator's word was sent and has not been answered.
fn mid_turn(rig: &mut Rig) {
    rig.command(input("go"));
    rig.line(TURN_INIT);
}

fn stopped(rig: &Rig) {
    assert!(
        !rig.requests("interrupt").is_empty(),
        "the turn in flight was not interrupted: {:?}",
        rig.written()
    );
    assert!(rig.closed(), "the child was not hung up on");
    assert!(
        rig.machine.done(),
        "the conversation went on after the session ended"
    );
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::State(RunState::Ended))),
        "the pane was not told the session ended"
    );
}

fn untouched(rig: &Rig) {
    assert!(rig.requests("interrupt").is_empty(), "{:?}", rig.written());
    assert!(!rig.closed());
    assert!(!rig.machine.done());
}

// guard 3
#[test]
fn guard3_an_account_that_names_a_key_ends_the_session_before_a_word_is_sent() {
    let mut rig = Rig::started(plan());
    rig.command(input("hello"));
    rig.line(&initialize_response("t.1", KEY_ACCOUNT));
    let trouble = rig.billing_trouble().expect("a billing trouble");
    assert!(trouble.contains("key in the environment"), "{trouble}");
    assert!(trouble.contains("ANTHROPIC_API_KEY"), "{trouble}");
    assert!(
        rig.words().is_empty(),
        "a word reached a session that was refused"
    );
    stopped(&rig);
}

#[test]
fn guard3_a_logged_out_account_ends_the_session() {
    let mut rig = Rig::started(plan());
    rig.line(&initialize_response("t.1", LOGGED_OUT));
    assert!(rig.billing_trouble().unwrap().contains("no login"));
    stopped(&rig);
}

#[test]
fn guard3_an_initialize_that_fails_does_not_allow_the_session() {
    let mut rig = Rig::started(plan());
    rig.line(&error_response("t.1", "not initialised"));
    assert!(rig.billing_trouble().unwrap().contains("could not be checked"));
    stopped(&rig);
}

#[test]
fn guard3_a_plan_account_is_allowed_and_a_key_is_under_api_billing() {
    let rig = Rig::allowed(plan());
    assert!(rig.billing_trouble().is_none());
    untouched(&rig);
    let mut api = Rig::started(opts(Billing::Api, Overage::Refuse, 20));
    api.line(&initialize_response("t.1", KEY_ACCOUNT));
    assert!(api.billing_trouble().is_none());
    untouched(&api);
}

// A word asked for before the account answers is kept, not lost, and goes out once allowed.
#[test]
fn a_word_sent_before_the_account_check_is_kept_and_sent_once_it_allows() {
    let mut rig = Rig::started(plan());
    rig.command(input("first"));
    assert!(rig.words().is_empty(), "sent before guard 3 had answered");
    rig.line(&initialize_response("t.1", PLAN_ACCOUNT));
    let words = rig.words();
    assert_eq!(words.len(), 1, "{:?}", rig.written());
    assert_eq!(words[0]["message"]["content"], "first");
}

#[test]
fn the_prompt_a_session_was_created_with_waits_for_the_account_check_too() {
    let mut rig = Rig::opened(Opts {
        opening: Some("start here".into()),
        ..plan()
    });
    rig.item(Item::Replayed { alive: true });
    assert!(rig.words().is_empty());
    rig.line(&initialize_response("t.1", PLAN_ACCOUNT));
    assert_eq!(rig.words()[0]["message"]["content"], "start here");
}

#[test]
fn a_word_held_for_the_account_check_outlives_a_node_restart() {
    let mut rig = Rig::started(plan());
    rig.command(input("before the check"));
    let mut rig = rig.restarted(plan());
    rig.item(Item::Replayed { alive: true });
    rig.line(&initialize_response("t.1", PLAN_ACCOUNT));
    let texts: Vec<_> = rig
        .words()
        .iter()
        .map(|w| w["message"]["content"].clone())
        .collect();
    assert_eq!(texts, [json!("before the check")]);
}

#[test]
fn words_held_for_an_overage_outlive_a_node_restart() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Ask, 20));
    mid_turn(&mut rig);
    rig.line(RATE_OVERAGE);
    rig.command(input("while held"));
    let mut rig = rig.restarted(opts(Billing::Plan, Overage::Allow, 20));
    rig.item(Item::Replayed { alive: true });
    rig.line(&initialize_response("t.1", PLAN_ACCOUNT));
    let texts: Vec<_> = rig
        .words()
        .iter()
        .map(|w| w["message"]["content"].clone())
        .collect();
    assert_eq!(
        texts,
        [json!("go"), json!("while held")],
        "the stopped prompt, then the held one"
    );
}

// guard 4
#[test]
fn guard4_a_turn_paid_by_a_key_is_interrupted_and_the_session_ends() {
    let mut rig = Rig::allowed(plan());
    mid_turn(&mut rig);
    rig.line(TURN_INIT_KEY);
    assert!(rig.billing_trouble().unwrap().contains("key in the environment"));
    stopped(&rig);
}

#[test]
fn guard4_a_key_is_allowed_under_api_billing() {
    let mut rig = Rig::allowed(opts(Billing::Api, Overage::Refuse, 20));
    mid_turn(&mut rig);
    rig.line(TURN_INIT_KEY);
    untouched(&rig);
}

// guard 6, on the shape the binary sends
#[test]
fn guard6_an_overage_is_interrupted_and_ends_the_session_by_default() {
    let mut rig = Rig::allowed(plan());
    mid_turn(&mut rig);
    rig.line(RATE_PLAIN);
    untouched(&rig);
    rig.line(RATE_OVERAGE);
    assert!(rig.billing_trouble().unwrap().contains("usage credits"));
    stopped(&rig);
}

#[test]
fn guard6_ask_holds_the_turn_and_everything_after_it_until_a_person_allows() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Ask, 20));
    mid_turn(&mut rig);
    rig.line(RATE_OVERAGE);
    assert_eq!(
        rig.requests("interrupt").len(),
        1,
        "the turn spending credits is stopped"
    );
    assert!(!rig.closed(), "ask does not end the session");
    assert!(
        rig.last_asks().iter().any(|a| a.id.as_deref() == Some("overage")),
        "{:?}",
        rig.last_asks()
    );
    assert!(rig.billing_trouble().is_none(), "ask holds, it does not refuse");

    let sent = rig.words().len();
    rig.command(input("while held"));
    assert_eq!(
        rig.words().len(),
        sent,
        "a word went out while credits were not allowed"
    );

    // The same event again while held is not a second ask.
    rig.line(RATE_OVERAGE);
    assert_eq!(rig.last_asks().len(), 1);

    rig.command(Command::Answer {
        ask: "overage".into(),
        reply: Reply::Allow {
            input: None,
            remember: false,
        },
    });
    let texts: Vec<_> = rig
        .words()
        .iter()
        .map(|w| w["message"]["content"].clone())
        .collect();
    assert_eq!(
        texts,
        [json!("go"), json!("go"), json!("while held")],
        "the stopped prompt, then the held one"
    );
    // Allowed once, the event is not asked about again.
    rig.line(RATE_OVERAGE);
    assert!(rig.last_asks().is_empty());
}

#[test]
fn guard6_ask_answered_no_ends_the_session() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Ask, 20));
    mid_turn(&mut rig);
    rig.line(RATE_OVERAGE);
    rig.command(Command::Answer {
        ask: "overage".into(),
        reply: Reply::Deny { message: None },
    });
    stopped(&rig);
}

#[test]
fn guard6_allow_lets_the_turn_run() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Allow, 20));
    mid_turn(&mut rig);
    rig.line(RATE_OVERAGE);
    untouched(&rig);
    assert!(rig.billing_trouble().is_none());
}

// guard 7, on the shape the binary sends
#[test]
fn guard7_the_unattended_cap_ends_the_session() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Refuse, 2));
    rig.line(RESULT_TASK);
    untouched(&rig);
    rig.line(RESULT_TASK);
    assert!(rig.billing_trouble().unwrap().contains("self-started"));
    stopped(&rig);
}

#[test]
fn guard7_a_persons_turn_resets_the_count() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Refuse, 2));
    for line in [RESULT_TASK, RESULT_USER, RESULT_TASK, RESULT_USER, RESULT_TASK] {
        rig.line(line);
    }
    untouched(&rig);
    assert!(rig.billing_trouble().is_none());
}

#[test]
fn guard7_a_word_from_the_operator_resets_the_count() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Refuse, 2));
    rig.line(RESULT_TASK);
    rig.command(input("keep going"));
    rig.line(RESULT_TASK);
    untouched(&rig);
}

// guard 10
fn usage(five: f64, resets: &str) -> serde_json::Value {
    json!({
        "subscription_type": "max",
        "rate_limits_available": true,
        "rate_limits": {
            "five_hour": {"utilization": five, "resets_at": resets},
            "seven_day": {"utilization": 50, "resets_at": "2026-11-30T20:59:59Z"},
        },
        "extra_usage": null,
    })
}

#[test]
fn guard10_the_limits_are_read_before_a_prompt_and_a_healthy_plan_is_not_held() {
    let mut rig = Rig::allowed(plan());
    let id = rig.request_id("get_usage");
    rig.line(&response(&id, usage(94.0, "2026-11-27T07:09:59Z")));
    let limits = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Limits(l) => Some(l),
            _ => None,
        })
        .expect("the limits are shown");
    assert_eq!(limits.plan.as_deref(), Some("Claude Max"));
    assert_eq!(limits.usage.unwrap()["five_hour"], 94.0);
    rig.command(input("go"));
    assert_eq!(rig.words().len(), 1);
}

#[test]
fn guard10_a_session_at_its_limit_is_told_so_and_its_prompt_is_held() {
    let mut rig = Rig::allowed(plan());
    let id = rig.request_id("get_usage");
    rig.line(&response(&id, usage(100.0, "2026-11-27T07:09:59Z")));
    assert_eq!(
        rig.trouble_fix(),
        Some(Fix::Wait {
            until: "2026-11-27T07:09:59Z".into()
        })
    );
    rig.command(input("go"));
    assert!(rig.words().is_empty(), "a prompt ran into a spent window");

    // The window resets: a fresh read releases what was held.
    rig.command(Command::Refresh(
        kampr_core::conversation::abilities::Fact::Limits,
    ));
    let id = rig.request_id("get_usage");
    rig.line(&response(&id, usage(3.0, "2026-11-27T12:09:59Z")));
    assert_eq!(rig.words().len(), 1, "{:?}", rig.written());
}

// #685: an account with no usage credits has nothing to hold a prompt back from; the API refuses.
#[test]
fn guard10_a_spent_window_on_an_account_that_cannot_pay_with_credits_is_told_and_not_held() {
    let mut rig = Rig::allowed(plan());
    let id = rig.request_id("get_usage");
    let mut body = usage(100.0, "2026-11-27T07:09:59Z");
    body["rate_limits"]["extra_usage"] = json!({"is_enabled": false, "credits_can_pay": "no"});
    rig.line(&response(&id, body));
    assert!(matches!(rig.trouble_fix(), Some(Fix::Wait { .. })));
    rig.command(input("go"));
    assert_eq!(rig.words().len(), 1);
    rig.command(Command::Refresh(
        kampr_core::conversation::abilities::Fact::Limits,
    ));
    let id = rig.request_id("get_usage");
    rig.line(&response(&id, usage(3.0, "2026-11-27T12:09:59Z")));
    assert!(matches!(
        rig.updates().iter().rev().find_map(|u| match u {
            Update::Trouble(t) => Some(t.clone()),
            _ => None,
        }),
        Some(None)
    ));
}

#[test]
fn guard10_ask_asks_and_allow_does_not_hold() {
    let mut asked = Rig::allowed(opts(Billing::Plan, Overage::Ask, 20));
    let id = asked.request_id("get_usage");
    asked.line(&response(&id, usage(100.0, "2026-11-27T07:09:59Z")));
    assert!(
        asked
            .last_asks()
            .iter()
            .any(|a| a.id.as_deref() == Some("overage"))
    );
    asked.command(input("go"));
    assert!(asked.words().is_empty());

    let mut open = Rig::allowed(opts(Billing::Plan, Overage::Allow, 20));
    let id = open.request_id("get_usage");
    open.line(&response(&id, usage(100.0, "2026-11-27T07:09:59Z")));
    open.command(input("go"));
    assert_eq!(open.words().len(), 1);
}

// A slash command runs no turn of the operator's words, so only prompts wait for the answer.
#[test]
fn guard10_ask_holds_a_prompt_and_lets_a_slash_command_through() {
    let mut rig = Rig::allowed(opts(Billing::Plan, Overage::Ask, 20));
    let id = rig.request_id("get_usage");
    rig.line(&response(&id, usage(100.0, "2026-11-27T07:09:59Z")));
    rig.command(input("go"));
    rig.command(input("/compact keep the plan"));
    rig.command(Command::Act {
        action: "compact".into(),
        args: json!(""),
    });
    let texts: Vec<_> = rig
        .words()
        .iter()
        .map(|w| w["message"]["content"].clone())
        .collect();
    assert_eq!(texts, [json!("/compact keep the plan"), json!("/compact")]);
}

// guard 8
#[test]
fn guard8_what_costs_money_carries_a_sentence_and_nothing_else_does() {
    let rig = Rig::allowed(plan());
    let controls = rig
        .updates()
        .into_iter()
        .rev()
        .find_map(|u| match u {
            Update::Controls(c) => Some(c),
            _ => None,
        })
        .expect("controls");
    let confirm = |id: &str| {
        controls
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no {id}: {controls:?}"))
            .confirm
            .clone()
    };
    assert!(confirm("fast").unwrap().contains("usage credits"));
    assert!(confirm("bypass").unwrap().contains("without asking"));
    assert!(confirm("mode").is_none());
    assert!(
        confirm("model").is_none(),
        "no model is claimed to be outside the plan"
    );
    let mode = controls.iter().find(|c| c.id == "mode").unwrap();
    assert!(
        mode.options.iter().all(|o| o.value != "bypassPermissions"),
        "bypass is its own control, with its own sentence"
    );
}

// guard 9
#[test]
fn guard9_the_billing_is_on_the_screen() {
    let mut rig = Rig::allowed(plan());
    let session = |rig: &Rig| {
        rig.updates()
            .into_iter()
            .rev()
            .find_map(|u| match u {
                Update::Session(s) => Some(s),
                _ => None,
            })
            .expect("a session update")
    };
    assert_eq!(session(&rig).billing.provider.as_deref(), Some("Claude Max"));
    assert!(session(&rig).billing.key.is_none());
    rig.line(RATE_OVERAGE);
    assert!(session(&rig).billing.rate_limited || rig.closed());

    let mut key = Rig::started(opts(Billing::Any, Overage::Refuse, 20));
    key.line(&initialize_response("t.1", KEY_ACCOUNT));
    assert_eq!(session(&key).billing.key.as_deref(), Some("ANTHROPIC_API_KEY"));
}
