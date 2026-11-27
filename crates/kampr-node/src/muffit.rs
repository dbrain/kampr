//! What a node does with a muffit pane (docs/18): no terminal pump, a conversation through the
//! registry, and answers through it (or muffit's REST) rather than keystrokes.

pub mod standing;

use crate::conversations::Conversations;
use crate::wire::Wire;
use kampr_core::conversation::composer::Line;
use kampr_core::conversation::{Command, Image, Reply, When};
use kampr_core::provider::Input;
use kampr_core::wire::ErrorCode;
use kampr_muffit::{MuffitProvider, answers};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// What the session waits on, from the session stream the node already holds: no watcher needed.
pub fn standing(link: &crate::sessions::MuffitLink) -> Vec<kampr_core::conversation::Ask> {
    link.provider
        .info(&link.session)
        .map(|info| kampr_muffit::asks_of(info.permission.as_ref(), info.question.as_ref()))
        .unwrap_or_default()
}

/// An answer on a muffit pane: through the conversation the socket has open, else straight to
/// the daemon's REST, so a prompt answered from the herd without opening the pane still lands.
// too_many_arguments: the answer frame's fields and where it goes, one flat hand-off as `input` is
#[allow(clippy::too_many_arguments)]
pub async fn answer(
    link: &crate::sessions::MuffitLink,
    conversations: &Conversations,
    wire: &Wire,
    pane: &str,
    key: &str,
    id: Option<&str>,
    text: Option<&str>,
    reply: Option<&Reply>,
) {
    let (permission, question) = link
        .provider
        .info(&link.session)
        .map(|info| (info.permission, info.question))
        .unwrap_or_default();
    let (ask, reply) = match answers::reply_of(permission.as_ref(), question.as_ref(), key, id, text, reply) {
        Ok(named) => named,
        Err(why) => {
            wire.error(ErrorCode::BadRequest, why, Some(pane));
            return;
        }
    };
    let command = Command::Answer {
        ask: ask.clone(),
        reply: reply.clone(),
    };
    if conversations.command(pane, command).is_ok() {
        return;
    }
    let posted = answers::post(
        link.provider.client(),
        &link.session,
        question.as_ref(),
        &ask,
        &reply,
    )
    .await;
    match posted {
        answers::Outcome::Taken => {}
        answers::Outcome::Gone => {
            wire.send(&crate::conversations::pending(pane, &[]));
        }
        answers::Outcome::Refused(why) => {
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
