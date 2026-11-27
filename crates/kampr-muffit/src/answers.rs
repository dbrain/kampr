//! An answer to a muffit ask: which one it is for, what the person meant, and the daemon's REST
//! that takes it. The conversation and the node's fallback for an unwatched pane share it.

use crate::client::{Answer, Answered, Client};
use crate::wire::{PermissionPrompt, Question};
use kampr_core::conversation::Reply;
use serde_json::{Value, json};

pub const ANSWERED: &str = "that question has already been answered";

/// The ask a pressed key and its words are for, and the reply they mean. A question's id is
/// `q<id>` (`source::question_ask`); with no id, the one standing. A question's choice is its
/// option's label, so the answer can't land on another question's option of the same number. A
/// `reply` beside a question's key is the permission shape every client sends, and is ignored.
pub fn reply_of(
    permission: Option<&PermissionPrompt>,
    question: Option<&Question>,
    key: &str,
    id: Option<&str>,
    text: Option<&str>,
    reply: Option<&Reply>,
) -> Result<(String, Reply), &'static str> {
    let words = text.map(str::trim).filter(|t| !t.is_empty());
    let for_question = match id {
        Some(id) => id.starts_with('q'),
        None => question.is_some() && permission.is_none(),
    };
    if for_question {
        let question = question.ok_or(ANSWERED)?;
        let ask = format!("q{}", question.id);
        if id.is_some_and(|id| id != ask) {
            return Err(ANSWERED);
        }
        if let Some(Reply::Dismiss | Reply::Choose { .. }) = reply {
            return Ok((ask, reply.cloned().expect("matched above")));
        }
        let chosen = match words {
            Some(words) if question.free_text => words.to_string(),
            Some(_) => return Err("this question takes one of its answers, not words"),
            None => key
                .parse::<usize>()
                .ok()
                .and_then(|n| question.options.get(n.checked_sub(1)?))
                .ok_or("that is not one of the question's answers")?
                .clone(),
        };
        return Ok((
            ask,
            Reply::Choose {
                answers: vec![chosen],
            },
        ));
    }
    if words.is_some() {
        return Err("a permission takes no words");
    }
    let prompt = permission.ok_or(ANSWERED)?;
    let ask = prompt.id.to_string();
    if id.is_some_and(|id| id != ask) {
        return Err(ANSWERED);
    }
    let reply = match (reply, key) {
        (Some(reply), _) => reply.clone(),
        (None, "1") => Reply::Allow {
            input: None,
            remember: false,
        },
        (None, "2") => Reply::Allow {
            input: None,
            remember: true,
        },
        (None, "3") => Reply::Deny { message: None },
        _ => return Err("a muffit prompt is answered 1, 2 or 3"),
    };
    Ok((ask, reply))
}

/// What the daemon's `question.answer` takes for a choice: the option's index, or the words
/// where the question takes words.
pub fn question_answer(question: &Question, reply: &Reply) -> Option<Value> {
    let Reply::Choose { answers } = reply else {
        return None;
    };
    let said = answers.first()?;
    match question.options.iter().position(|o| o == said) {
        Some(index) => Some(json!(index)),
        None => question.free_text.then(|| json!(said)),
    }
}

pub fn permission_answer(reply: &Reply) -> Option<Answer> {
    match reply {
        Reply::Allow { remember: true, .. } => Some(Answer::Always),
        Reply::Allow { .. } => Some(Answer::Allow),
        Reply::Deny { .. } => Some(Answer::Deny),
        _ => None,
    }
}

pub enum Outcome {
    Taken,
    /// Nothing stands for it any more: answered elsewhere or withdrawn.
    Gone,
    Refused(String),
}

/// Posts the answer to whichever of the two asks `ask` names.
pub async fn post(
    client: &Client,
    session: &str,
    question: Option<&Question>,
    ask: &str,
    reply: &Reply,
) -> Outcome {
    let sent = match ask.strip_prefix('q') {
        Some(id) => {
            let Some(question) = question.filter(|q| q.id.to_string() == id) else {
                return Outcome::Gone;
            };
            let Some(answer) = question_answer(question, reply) else {
                return Outcome::Refused("that is not one of the question's answers".into());
            };
            client.answer_question(session, question.id, answer).await
        }
        // The daemon decides whether a prompt still waits: one this side thinks gone is asked
        // anyway, and its 409 is what clears the strip.
        None => {
            let Ok(id) = ask.parse::<u64>() else {
                return Outcome::Gone;
            };
            let Some(answer) = permission_answer(reply) else {
                return Outcome::Refused("a permission is allowed or denied".into());
            };
            client.answer(session, id, answer).await
        }
    };
    match sent {
        Ok(Answered::Taken) => Outcome::Taken,
        Ok(Answered::NoLongerWaiting) => Outcome::Gone,
        Err(e) => Outcome::Refused(format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Question {
        Question {
            id: 4,
            text: "Which approach?".into(),
            options: vec!["Refactor".into(), "Patch".into()],
            free_text: true,
        }
    }

    fn prompt() -> PermissionPrompt {
        PermissionPrompt {
            id: 7,
            tool: "bash".into(),
            action: "ls".into(),
        }
    }

    #[test]
    fn a_key_and_its_words_name_the_ask_and_the_reply() {
        let allow = Reply::Allow {
            input: None,
            remember: false,
        };
        let choose = |s: &str| Reply::Choose {
            answers: vec![s.into()],
        };
        let q = question();
        let p = prompt();
        type Case<'a> = (
            Option<&'a str>,
            &'a str,
            Option<&'a str>,
            Option<&'a Reply>,
            Result<(String, Reply), &'a str>,
        );
        let cases: [Case; 7] = [
            (
                Some("q4"),
                "2",
                None,
                Some(&allow),
                Ok(("q4".into(), choose("Patch"))),
            ),
            (
                Some("q4"),
                "3",
                Some("neither"),
                None,
                Ok(("q4".into(), choose("neither"))),
            ),
            (Some("q3"), "1", None, None, Err(ANSWERED)),
            (
                Some("q4"),
                "9",
                None,
                None,
                Err("that is not one of the question's answers"),
            ),
            (
                Some("7"),
                "2",
                None,
                None,
                Ok((
                    "7".into(),
                    Reply::Allow {
                        input: None,
                        remember: true,
                    },
                )),
            ),
            (
                None,
                "3",
                None,
                None,
                Ok(("7".into(), Reply::Deny { message: None })),
            ),
            (Some("6"), "1", None, None, Err(ANSWERED)),
        ];
        for (id, key, text, reply, want) in cases {
            assert_eq!(
                reply_of(Some(&p), Some(&q), key, id, text, reply),
                want,
                "{id:?} {key}"
            );
        }
        assert_eq!(
            reply_of(None, Some(&q), "1", None, None, None),
            Ok(("q4".into(), choose("Refactor")))
        );
        assert_eq!(question_answer(&q, &choose("Patch")), Some(json!(1)));
        assert_eq!(question_answer(&q, &choose("my own")), Some(json!("my own")));
    }
}
