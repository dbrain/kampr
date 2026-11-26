//! What the failures of a turn are worth, read off the transcript.
//!
//! The failing turn's `result` line says `subtype: "success"`; the fields that mean failure are
//! `is_error` and `terminal_reason` (#651). The retries are `system/api_error` records (#652), and
//! the sentence the TUI prints is the `isApiErrorMessage` record.

use crate::common::*;
use kampr_journal::{Retry, Trouble, TroubleNews, Turn};
use serde_json::json;

fn ids(turns: &[Turn]) -> Vec<&str> {
    turns.iter().map(|t| t.id.as_str()).collect()
}

fn prompt(uuid: &str, parent: &str, text: &str) -> serde_json::Value {
    json!({
        "type": "user",
        "uuid": uuid,
        "parentUuid": parent,
        "timestamp": "2026-08-20T02:56:27.000Z",
        "message": { "role": "user", "content": text },
    })
}

fn said(uuid: &str, parent: &str, text: &str) -> serde_json::Value {
    json!({
        "type": "assistant",
        "uuid": uuid,
        "parentUuid": parent,
        "timestamp": "2026-08-20T02:56:29.000Z",
        "message": { "role": "assistant", "content": text },
    })
}

fn api_error(uuid: &str, parent: &str, attempt: u32, of: u32, in_ms: u64, status: u16) -> serde_json::Value {
    json!({
        "type": "system",
        "subtype": "api_error",
        "uuid": uuid,
        "parentUuid": parent,
        "timestamp": "2026-08-20T02:56:28.000Z",
        "error": {
            "status": status,
            "message": "Overloaded",
            "formatted": format!("API Error: {status}"),
            "requestId": "req-1",
            "isNetworkDown": false,
        },
        "retryAttempt": attempt,
        "maxRetries": of,
        "retryInMs": in_ms,
    })
}

fn api_message(uuid: &str, parent: &str, kind: &str, status: Option<u16>, text: &str) -> serde_json::Value {
    let mut record = json!({
        "type": "assistant",
        "uuid": uuid,
        "parentUuid": parent,
        "timestamp": "2026-08-20T02:56:30.000Z",
        "isApiErrorMessage": true,
        "error": kind,
        "message": { "role": "assistant", "model": "<synthetic>", "content": text },
    });
    if let Some(status) = status {
        record["apiErrorStatus"] = json!(status);
    }
    record
}

fn result_line(
    uuid: &str,
    parent: &str,
    is_error: bool,
    terminal_reason: Option<&str>,
    text: &str,
) -> serde_json::Value {
    let mut record = json!({
        "type": "result",
        "subtype": "success",
        "uuid": uuid,
        "parentUuid": parent,
        "timestamp": "2026-08-20T02:56:31.000Z",
        "is_error": is_error,
        "result": text,
    });
    if let Some(reason) = terminal_reason {
        record["terminal_reason"] = json!(reason);
    }
    record
}

#[test]
fn a_retry_is_trouble_with_its_attempt_and_delay() {
    let mut scratch = scratch_claude(
        "trouble-retry",
        &[
            prompt("u0", "root", "try again"),
            api_error("s1", "u0", 1, 2, 8000, 529),
        ],
    );
    let turns = scratch.turns();
    assert_eq!(ids(&turns), ["u0"], "a retry carries no turn");
    assert_eq!(
        scratch.journal.take_trouble(),
        vec![TroubleNews::Trouble(Trouble {
            kind: "overloaded".into(),
            text: "API Error: 529".into(),
            status: Some(529),
            retry: Some(Retry {
                attempt: 1,
                of: 2,
                in_ms: 8000
            }),
        })]
    );
}

#[test]
fn the_retry_kind_follows_the_status() {
    let mut scratch = scratch_claude(
        "trouble-kinds",
        &[
            prompt("u0", "root", "go"),
            api_error("s1", "u0", 1, 10, 4000, 429),
            api_error("s2", "u0", 2, 10, 8000, 500),
            api_error("s3", "u0", 3, 10, 16000, 401),
        ],
    );
    scratch.turns();
    let news = scratch.journal.take_trouble();
    let kinds: Vec<&str> = news
        .iter()
        .filter_map(|n| match n {
            TroubleNews::Trouble(t) => Some(t.kind.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["rate-limit", "server-error", "login-expired"]);
}

#[test]
fn a_logged_out_turn_is_named_not_logged_in() {
    let mut scratch = scratch_claude(
        "trouble-logout",
        &[
            prompt("u0", "root", "hi"),
            api_message(
                "a1",
                "u0",
                "authentication_failed",
                None,
                "Not logged in · Please run /login",
            ),
        ],
    );
    let turns = scratch.turns();
    assert_eq!(
        md_texts(&turns),
        ["hi", "Not logged in · Please run /login"],
        "the sentence stays in the conversation too"
    );
    assert_eq!(
        scratch.journal.take_trouble(),
        vec![TroubleNews::Trouble(Trouble {
            kind: "not-logged-in".into(),
            text: "Not logged in · Please run /login".into(),
            status: None,
            retry: None,
        })]
    );
}

#[test]
fn an_expired_login_is_told_from_the_sentence() {
    let mut scratch = scratch_claude(
        "trouble-expired",
        &[
            prompt("u0", "root", "hi"),
            api_message(
                "a1",
                "u0",
                "authentication_failed",
                Some(401),
                "Failed to authenticate. API Error: 401 OAuth access token is invalid.",
            ),
        ],
    );
    scratch.turns();
    assert_eq!(
        scratch.journal.take_trouble(),
        vec![TroubleNews::Trouble(Trouble {
            kind: "login-expired".into(),
            text: "Failed to authenticate. API Error: 401 OAuth access token is invalid.".into(),
            status: Some(401),
            retry: None,
        })]
    );
}

#[test]
fn a_refused_turn_is_trouble_from_its_result_line() {
    let mut scratch = scratch_claude(
        "trouble-result",
        &[
            prompt("u0", "root", "hi"),
            result_line(
                "r1",
                "u0",
                true,
                Some("api_error"),
                "Claude Code can't use your Claude subscription, usually because a payment is past due",
            ),
        ],
    );
    scratch.turns();
    assert_eq!(
        scratch.journal.take_trouble(),
        vec![TroubleNews::Trouble(Trouble {
            kind: "api-error".into(),
            text: "Claude Code can't use your Claude subscription, usually because a payment is past due"
                .into(),
            status: None,
            retry: None,
        })]
    );
}

#[test]
fn a_result_line_that_says_success_is_not_trouble() {
    let mut scratch = scratch_claude(
        "trouble-ok-result",
        &[
            prompt("u0", "root", "hi"),
            said("a1", "u0", "there"),
            result_line("r1", "a1", false, None, "there"),
        ],
    );
    scratch.turns();
    assert!(scratch.journal.take_trouble().is_empty());
}

#[test]
fn a_new_prompt_clears_the_trouble_before_it() {
    let mut scratch = scratch_claude(
        "trouble-prompt-clear",
        &[
            prompt("u0", "root", "hi"),
            api_error("s1", "u0", 1, 2, 8000, 529),
            prompt("u2", "s1", "again"),
        ],
    );
    scratch.turns();
    assert_eq!(
        scratch.journal.take_trouble(),
        vec![
            TroubleNews::Trouble(Trouble {
                kind: "overloaded".into(),
                text: "API Error: 529".into(),
                status: Some(529),
                retry: Some(Retry {
                    attempt: 1,
                    of: 2,
                    in_ms: 8000
                }),
            }),
            TroubleNews::Clear,
        ]
    );
}

#[test]
fn a_turn_that_ends_well_clears_the_trouble() {
    let mut scratch = scratch_claude(
        "trouble-result-clear",
        &[
            prompt("u0", "root", "hi"),
            api_error("s1", "u0", 1, 2, 8000, 529),
            said("a1", "s1", "made it"),
            result_line("r1", "a1", false, None, "made it"),
        ],
    );
    scratch.turns();
    let news = scratch.journal.take_trouble();
    assert_eq!(news.len(), 2);
    assert!(matches!(&news[0], TroubleNews::Trouble(t) if t.kind == "overloaded"));
    assert_eq!(news[1], TroubleNews::Clear);
}

#[test]
fn a_turn_with_no_failure_carries_no_trouble() {
    let mut scratch = scratch_claude(
        "trouble-none",
        &[
            prompt("u0", "root", "hi"),
            said("a1", "u0", "there"),
            result_line("r1", "a1", false, None, "there"),
        ],
    );
    scratch.turns();
    assert!(scratch.journal.take_trouble().is_empty());
}

#[test]
fn trouble_is_taken_once() {
    let mut scratch = scratch_claude(
        "trouble-once",
        &[prompt("u0", "root", "hi"), api_error("s1", "u0", 1, 2, 8000, 529)],
    );
    scratch.turns();
    assert!(!scratch.journal.take_trouble().is_empty());
    assert!(
        scratch.journal.take_trouble().is_empty(),
        "a second take has nothing"
    );
}

#[test]
fn a_taken_trouble_does_not_litter_the_turns() {
    let mut scratch = scratch_claude(
        "trouble-no-turns",
        &[prompt("u0", "root", "hi"), api_error("s1", "u0", 1, 2, 8000, 529)],
    );
    let first = scratch.turns();
    assert_eq!(ids(&first), ["u0"]);
    assert!(first.iter().all(|t| t.id != "s1"), "the retry is not a turn");
    let again: Vec<Turn> = scratch.journal.poll().unwrap();
    assert!(again.is_empty(), "nothing grew after the first read");
}
