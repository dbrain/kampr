//! What a node does with a muffit pane (docs/18): no terminal pump, a conversation through the
//! registry, and questions and answers over muffit's REST rather than keystrokes.

use crate::conversations::Conversations;
use crate::wire::Wire;
use kampr_core::conversation::composer::Line;
use kampr_core::conversation::{Command, Image, Reply, When};
use kampr_core::provider::Input;
use kampr_core::wire::{ErrorCode, PendingOption, PendingSource, ServerMsg};
use kampr_muffit::wire::PermissionPrompt;
use kampr_muffit::{Answer, MuffitProvider};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const ANSWERS: [(&str, &str, Answer); 3] = [
    ("1", "Allow", Answer::Allow),
    ("2", "Always allow", Answer::Always),
    ("3", "Deny", Answer::Deny),
];

pub fn answer_for(key: &str) -> Option<Answer> {
    ANSWERS.iter().find(|(k, _, _)| *k == key).map(|(_, _, a)| *a)
}

/// The pane's answer key for a seam reply: muffit's prompt is allow, always, deny.
pub fn key_of(reply: &Reply) -> Option<&'static str> {
    match reply {
        Reply::Allow { remember: true, .. } => Some("2"),
        Reply::Allow { .. } => Some("1"),
        Reply::Deny { .. } => Some("3"),
        _ => None,
    }
}

pub fn pending(pane: &str, prompt: Option<&PermissionPrompt>) -> ServerMsg {
    ServerMsg::Pending {
        pane: pane.to_string(),
        question: prompt.map(|p| p.action.clone()),
        header: prompt.map(|p| p.tool.clone()),
        multi: false,
        context: None,
        id: prompt.map(|p| p.id.to_string()),
        options: match prompt {
            Some(_) => ANSWERS
                .iter()
                .map(|(key, label, _)| PendingOption {
                    key: (*key).into(),
                    label: (*label).into(),
                    detail: None,
                    chosen: false,
                    text: false,
                })
                .collect(),
            None => Vec::new(),
        },
        source: PendingSource::Transcript,
    }
}

/// The prompt a muffit session is waiting on, off the session stream the node already holds.
pub fn prompt(provider: &MuffitProvider, session: &str) -> Option<PermissionPrompt> {
    provider.info(session)?.permission
}

pub enum AnswerResult {
    Taken,
    Cleared,
    BadKey,
    Refused(String),
}

/// Answers the session's waiting prompt with the key, through the daemon's REST.
pub async fn answer(provider: &MuffitProvider, session: &str, key: &str) -> AnswerResult {
    let Some(answer) = answer_for(key) else {
        return AnswerResult::BadKey;
    };
    let Some(prompt) = prompt(provider, session) else {
        return AnswerResult::Cleared;
    };
    match provider.client().answer(session, prompt.id, answer).await {
        Ok(kampr_muffit::Answered::Taken) => AnswerResult::Taken,
        Ok(kampr_muffit::Answered::NoLongerWaiting) => AnswerResult::Cleared,
        Err(e) => AnswerResult::Refused(format!("{e:#}")),
    }
}

pub fn answer_result(wire: &Wire, pane: &str, result: AnswerResult) {
    match result {
        AnswerResult::Taken => {}
        AnswerResult::Cleared => {
            wire.send(&pending(pane, None));
        }
        AnswerResult::BadKey => {
            wire.error(
                ErrorCode::BadRequest,
                "a muffit prompt is answered 1, 2 or 3",
                Some(pane),
            );
        }
        AnswerResult::Refused(why) => {
            wire.error(ErrorCode::SessionUnavailable, &why, Some(pane));
        }
    }
}

/// The composer line behind an `m:` pane: its steps go to the conversation, and only what the
/// conversation is gone takes the REST path directly.
// too_many_arguments: the pane's wiring is one flat hand-off; a params struct would hide which caller owns each piece
#[allow(clippy::too_many_arguments)]
pub async fn input(
    lines: &mut HashMap<(String, Option<String>), Line>,
    provider: &Arc<MuffitProvider>,
    pane: &str,
    local: &str,
    input: Input,
    typist: Option<String>,
    conversations: &Conversations,
    wire: &Arc<Wire>,
    state_dir: &Path,
) {
    use kampr_core::conversation::composer::Step;
    let line = lines.entry((pane.to_string(), typist)).or_default();
    let steps = match input {
        Input::Bytes(bytes) => line.feed(&String::from_utf8_lossy(&bytes)),
        Input::Paste(text) => line.feed(&kampr_core::paste::bracketed(&text)),
        Input::Keys(keys) => line.keys(&keys),
    };
    let session = kampr_muffit::session_of(local).unwrap_or_default();
    for step in steps {
        let sent = match step {
            Step::Interrupt => provider.client().interrupt(session).await,
            Step::Submit(text) => {
                let msg_id = ulid::Ulid::generate().to_string();
                let (text, images) = crate::paste::lift_images(state_dir, &text);
                match conversations
                    .send(
                        pane,
                        Command::Input {
                            text: text.clone(),
                            msg_id,
                            images: images
                                .iter()
                                .map(|image| Image {
                                    mime: image.mime.clone(),
                                    data: image.data.clone(),
                                })
                                .collect(),
                            files: Vec::new(),
                            when: When::Idle,
                        },
                    )
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(_) => provider.client().input(session, &text, &images, None).await,
                }
            }
        };
        if let Err(e) = sent {
            wire.error(ErrorCode::SessionUnavailable, &format!("{e:#}"), Some(pane));
            return;
        }
    }
}
