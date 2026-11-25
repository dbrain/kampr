use crate::follow::{self, Follow};
use crate::provider::AGENT;
use crate::wire::{Image, PermissionPrompt, SessionState};
use kampr_core::conversation::abilities::{
    AskAbility, Fact, InputAbility, Lifecycle, Stream, Steer,
};
use kampr_core::conversation::{
    Abilities, Ask, AskOption, Command, Conversation, Ending, Emit, NewSession, Refusal, Source,
    Stored, Update,
};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

const COMMANDS: usize = 64;

pub struct MuffitSource {
    provider: Arc<crate::provider::MuffitProvider>,
}

impl MuffitSource {
    pub fn new(provider: Arc<crate::provider::MuffitProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl Source for MuffitSource {
    fn kind(&self) -> &'static str {
        AGENT
    }

    async fn open(&self, pane: &str) -> Result<Box<dyn Conversation>, Refusal> {
        let session = crate::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a muffit session")))?;
        Ok(Box::new(MuffitConversation::new(
            self.provider.clone(),
            session.to_string(),
        )))
    }

    async fn create(&self, new: NewSession) -> Result<String, Refusal> {
        let cwd = new
            .cwd
            .as_ref()
            .map(|cwd| cwd.display().to_string())
            .filter(|cwd| !cwd.is_empty())
            .ok_or_else(|| Refusal::new("a new muffit session needs a working directory"))?;
        self.provider
            .client()
            .create(&cwd, new.prompt.as_deref())
            .await
            .map_err(|e| Refusal::new(format!("{e:#}")))
    }

    async fn resumable(&self, _cwd: Option<&Path>) -> Vec<Stored> {
        self.provider
            .client()
            .sessions()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|session| {
                matches!(
                    session.state,
                    SessionState::Completed | SessionState::Failed | SessionState::Paused
                )
            })
            .map(|session| Stored {
                id: session.id,
                cwd: session.cwd.map(std::path::PathBuf::from),
                title: session.title,
            })
            .collect()
    }

    async fn end(&self, pane: &str, _how: Ending) -> Result<(), Refusal> {
        let session = crate::session_of(pane)
            .ok_or_else(|| Refusal::new(format!("{pane} is not a muffit session")))?;
        self.provider
            .client()
            .stop(session)
            .await
            .map_err(|e| Refusal::new(format!("{e:#}")))
    }
}

/// The follower a muffit pane is followed by: `Follow`, renamed to the seam and given the
/// commands and updates the seam speaks.
pub struct MuffitConversation {
    provider: Arc<crate::provider::MuffitProvider>,
    session: String,
}

impl MuffitConversation {
    pub fn new(provider: Arc<crate::provider::MuffitProvider>, session: String) -> Self {
        Self { provider, session }
    }

    pub fn abilities() -> Abilities {
        let capabilities = follow::CAPABILITIES;
        let has = |capability: &str| capabilities.contains(&capability);
        let mut abilities = Abilities::default();
        if has("input") {
            abilities.input.insert(InputAbility::Images);
        }
        if has("interrupt") {
            abilities.steer.insert(Steer::Interrupt);
            abilities.steer.insert(Steer::Continue);
        }
        abilities.steer.insert(Steer::Pause);
        abilities.steer.insert(Steer::Resume);
        abilities.steer.insert(Steer::Stop);
        abilities.steer.insert(Steer::Compact);
        if has("permissions") {
            abilities.asks.insert(AskAbility::Permission);
            abilities.asks.insert(AskAbility::Always);
        }
        if has("status") {
            abilities.streams.insert(Stream::Text);
            abilities.streams.insert(Stream::TaskOutput);
            abilities.facts.insert(Fact::Context);
            abilities.facts.insert(Fact::Cost);
            abilities.facts.insert(Fact::Trouble);
        }
        abilities.lifecycle.insert(Lifecycle::Create);
        abilities.lifecycle.insert(Lifecycle::Resume);
        abilities.lifecycle.insert(Lifecycle::End);
        abilities
    }
}

#[async_trait::async_trait]
impl Conversation for MuffitConversation {
    async fn run(self: Box<Self>, commands: mpsc::Receiver<Command>, mut emit: Emit) {
        let MuffitConversation { provider, session } = *self;
        (emit)(Update::Abilities(Self::abilities()));
        (emit)(Update::Actions(follow::actions()));
        let (outgoing, incoming) = mpsc::channel(COMMANDS);
        let mapper = tokio::spawn(commands_to_follow(commands, outgoing));
        let client = provider.client().clone();
        let revision = provider.revision();
        let alive_session = session.clone();
        let follow = Follow {
            client,
            session,
            commands: incoming,
            alive: Box::new(move || provider.is_session_live(&alive_session)),
            revision,
        };
        follow.run(move |update| emit(update_of(update))).await;
        mapper.abort();
    }
}

/// What the seam says, in the follower's words. A command the follower has no word for is
/// dropped: the pane asked for something the source did not declare.
async fn commands_to_follow(
    mut commands: mpsc::Receiver<Command>,
    outgoing: mpsc::Sender<follow::Command>,
) {
    while let Some(command) = commands.recv().await {
        let mapped = match command {
            Command::Input {
                text,
                msg_id,
                images,
                ..
            } => Some(follow::Command::Input {
                text,
                msg_id,
                images: images
                    .into_iter()
                    .map(|image| Image {
                        mime: image.mime,
                        data: image.data,
                    })
                    .collect(),
            }),
            Command::Older { before } => Some(follow::Command::Older { before }),
            Command::Find { query, cap } => Some(follow::Command::Find { query, cap }),
            Command::Sub { id, before } => Some(follow::Command::Sub { id, before }),
            Command::Steer(kind, text) => Some(follow::Command::Steer { kind, text }),
            Command::Set { control, value } => Some(follow::Command::Set { control, value }),
            Command::Act { action, args } => Some(follow::Command::Act { action, args }),
            Command::Task { id, from } => Some(follow::Command::FollowTask { id, from }),
            Command::Unfollow { id } => Some(follow::Command::Unfollow { id }),
            _ => None,
        };
        let Some(mapped) = mapped else {
            continue;
        };
        if outgoing.send(mapped).await.is_err() {
            return;
        }
    }
}

fn update_of(update: follow::Update) -> Update {
    match update {
        follow::Update::Page(page) => Update::Page(page),
        follow::Update::Turns(turns) => Update::Turns(turns),
        follow::Update::Older(page) => Update::Older(page),
        follow::Update::Facets(facets) => Update::Facets(facets),
        follow::Update::Pending(prompt) => Update::Asks(match prompt {
            Some(prompt) => vec![ask_of(&prompt)],
            None => Vec::new(),
        }),
        follow::Update::Found { query, found } => Update::Found { query, found },
        follow::Update::SubPage { id, page } => Update::SubPage { id, page },
        follow::Update::SubTurns { id, turns } => Update::SubTurns { id, turns },
        follow::Update::State(state) => Update::State(state),
        follow::Update::Tasks(tasks) => Update::Tasks(tasks),
        follow::Update::TaskOutput {
            id,
            at,
            text,
            ended,
        } => Update::TaskOutput { id, at, text, ended },
        follow::Update::Trouble(trouble) => Update::Trouble(trouble),
        follow::Update::Session(facts) => Update::Session(facts),
        follow::Update::Controls(controls) => Update::Controls(controls),
        follow::Update::Actions(actions) => Update::Actions(actions),
        follow::Update::Context(context) => Update::Context(context),
        follow::Update::Cost(cost) => Update::Cost(cost),
        follow::Update::Live(live) => Update::Live(live),
        follow::Update::Acted { action, ok, text } => Update::Acted { action, ok, text },
        follow::Update::Missing(message) => Update::Missing(message),
        follow::Update::Failed(message) => Update::Failed(message),
    }
}

fn ask_of(prompt: &PermissionPrompt) -> Ask {
    Ask {
        id: Some(prompt.id.to_string()),
        kind: AskAbility::Permission,
        by: None,
        header: Some(prompt.tool.clone()),
        question: prompt.action.clone(),
        multi: false,
        context: None,
        options: [
            ("1", "Allow"),
            ("2", "Always allow"),
            ("3", "Deny"),
        ]
        .into_iter()
        .map(|(key, label)| AskOption {
            key: key.into(),
            label: label.into(),
            detail: None,
            chosen: false,
            text: false,
        })
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::conversation::{
        ContextUse, Cost, FileRef, Image as SeamImage, Live as SeamLive, Reply, RunState,
        SessionFacts, Steer, When,
    };
    use kampr_journal::{ConvoFound, Facets, Page, Turn};

    fn prompt() -> PermissionPrompt {
        PermissionPrompt {
            id: 1,
            tool: "bash".into(),
            action: "run the tests".into(),
        }
    }

    #[test]
    fn a_prompt_is_an_ask_with_its_three_answers() {
        let ask = ask_of(&prompt());
        assert_eq!(ask.id.as_deref(), Some("1"));
        assert_eq!(ask.kind, AskAbility::Permission);
        assert_eq!(ask.header.as_deref(), Some("bash"));
        assert_eq!(ask.question, "run the tests");
        assert!(!ask.multi);
        assert_eq!(
            ask.options
                .iter()
                .map(|o| (o.key.as_str(), o.label.as_str()))
                .collect::<Vec<_>>(),
            [("1", "Allow"), ("2", "Always allow"), ("3", "Deny")]
        );
    }

    #[test]
    fn the_updates_the_follower_speaks_are_the_seams() {
        let page = Page {
            turns: vec![Turn::new("m1", kampr_journal::Role::User, None)],
            cursor: None,
            more: false,
        };
        let cases: [(follow::Update, Update); 21] = [
            (
                follow::Update::Page(page.clone()),
                Update::Page(page.clone()),
            ),
            (
                follow::Update::Turns(page.turns.clone()),
                Update::Turns(page.turns.clone()),
            ),
            (follow::Update::Older(page.clone()), Update::Older(page)),
            (follow::Update::Facets(Facets::default()), Update::Facets(Facets::default())),
            (
                follow::Update::Pending(Some(prompt())),
                Update::Asks(vec![ask_of(&prompt())]),
            ),
            (follow::Update::Pending(None), Update::Asks(Vec::new())),
            (
                follow::Update::Found {
                    query: "q".into(),
                    found: ConvoFound {
                        total: 1,
                        hits: Vec::new(),
                    },
                },
                Update::Found {
                    query: "q".into(),
                    found: ConvoFound {
                        total: 1,
                        hits: Vec::new(),
                    },
                },
            ),
            (
                follow::Update::SubPage {
                    id: "s1.s1".into(),
                    page: Page {
                        turns: Vec::new(),
                        cursor: None,
                        more: false,
                    },
                },
                Update::SubPage {
                    id: "s1.s1".into(),
                    page: Page {
                        turns: Vec::new(),
                        cursor: None,
                        more: false,
                    },
                },
            ),
            (
                follow::Update::SubTurns {
                    id: "s1.s1".into(),
                    turns: Vec::new(),
                },
                Update::SubTurns {
                    id: "s1.s1".into(),
                    turns: Vec::new(),
                },
            ),
            (
                follow::Update::Missing("no such conversation".into()),
                Update::Missing("no such conversation".into()),
            ),
            (
                follow::Update::State(RunState::Running),
                Update::State(RunState::Running),
            ),
            (follow::Update::Tasks(Vec::new()), Update::Tasks(Vec::new())),
            (
                follow::Update::TaskOutput {
                    id: "call_7".into(),
                    at: 3,
                    text: "a line".into(),
                    ended: None,
                },
                Update::TaskOutput {
                    id: "call_7".into(),
                    at: 3,
                    text: "a line".into(),
                    ended: None,
                },
            ),
            (follow::Update::Trouble(None), Update::Trouble(None)),
            (
                follow::Update::Session(SessionFacts {
                    id: "s1".into(),
                    cwd: None,
                    title: None,
                    model: None,
                    mode: None,
                    version: None,
                    account: None,
                    billing: Default::default(),
                    started: None,
                }),
                Update::Session(SessionFacts {
                    id: "s1".into(),
                    cwd: None,
                    title: None,
                    model: None,
                    mode: None,
                    version: None,
                    account: None,
                    billing: Default::default(),
                    started: None,
                }),
            ),
            (follow::Update::Controls(Vec::new()), Update::Controls(Vec::new())),
            (follow::Update::Actions(Vec::new()), Update::Actions(Vec::new())),
            (
                follow::Update::Context(ContextUse { used: 1, total: 0 }),
                Update::Context(ContextUse { used: 1, total: 0 }),
            ),
            (
                follow::Update::Cost(Cost {
                    usd: 0.0,
                    input_tokens: 1,
                    output_tokens: 2,
                }),
                Update::Cost(Cost {
                    usd: 0.0,
                    input_tokens: 1,
                    output_tokens: 2,
                }),
            ),
            (
                follow::Update::Live(SeamLive::default()),
                Update::Live(SeamLive::default()),
            ),
            (
                follow::Update::Acted {
                    action: "context.compact".into(),
                    ok: true,
                    text: None,
                },
                Update::Acted {
                    action: "context.compact".into(),
                    ok: true,
                    text: None,
                },
            ),
        ];
        for (follower, seam) in cases {
            assert_eq!(update_of(follower), seam);
        }
        assert_eq!(
            update_of(follow::Update::Failed("it failed".into())),
            Update::Failed("it failed".into())
        );
    }

    #[tokio::test]
    async fn the_commands_the_pane_sends_are_the_follower_s() {
        let (incoming, received) = mpsc::channel::<Command>(4);
        let (follow_tx, mut follow_rx) = mpsc::channel::<follow::Command>(16);
        let mapper = tokio::spawn(commands_to_follow(received, follow_tx));
        incoming
            .send(Command::Input {
                text: "hello".into(),
                msg_id: "m1".into(),
                images: vec![SeamImage {
                    mime: "image/png".into(),
                    data: "aGk=".into(),
                }],
                files: vec![FileRef {
                    path: "/tmp/x".into(),
                }],
                when: When::Idle,
            })
            .await
            .unwrap();
        incoming
            .send(Command::Answer {
                ask: "1".into(),
                reply: Reply::Dismiss,
            })
            .await
            .unwrap();
        incoming
            .send(Command::Older {
                before: Some("m50".into()),
            })
            .await
            .unwrap();
        incoming
            .send(Command::Find {
                query: "message 7".into(),
                cap: 50,
            })
            .await
            .unwrap();
        incoming
            .send(Command::Sub {
                id: "s1.s1".into(),
                before: None,
            })
            .await
            .unwrap();
        incoming
            .send(Command::Steer(Steer::Interrupt, None))
            .await
            .unwrap();
        incoming
            .send(Command::Set {
                control: "model".into(),
                value: serde_json::json!("mock"),
            })
            .await
            .unwrap();
        incoming
            .send(Command::Act {
                action: "compact".into(),
                args: serde_json::json!({}),
            })
            .await
            .unwrap();
        incoming
            .send(Command::Task {
                id: "call_7".into(),
                from: 0,
            })
            .await
            .unwrap();
        incoming
            .send(Command::Unfollow {
                id: "call_7".into(),
            })
            .await
            .unwrap();
        drop(incoming);
        mapper.await.unwrap();
        let mut got = Vec::new();
        while let Ok(command) = follow_rx.try_recv() {
            got.push(command);
        }
        assert_eq!(
            got,
            vec![
                follow::Command::Input {
                    text: "hello".into(),
                    msg_id: "m1".into(),
                    images: vec![Image {
                        mime: "image/png".into(),
                        data: "aGk=".into(),
                    }],
                },
                follow::Command::Older {
                    before: Some("m50".into()),
                },
                follow::Command::Find {
                    query: "message 7".into(),
                    cap: 50,
                },
                follow::Command::Sub {
                    id: "s1.s1".into(),
                    before: None,
                },
                follow::Command::Steer {
                    kind: Steer::Interrupt,
                    text: None,
                },
                follow::Command::Set {
                    control: "model".into(),
                    value: serde_json::json!("mock"),
                },
                follow::Command::Act {
                    action: "compact".into(),
                    args: serde_json::json!({}),
                },
                follow::Command::FollowTask {
                    id: "call_7".into(),
                    from: 0,
                },
                follow::Command::Unfollow {
                    id: "call_7".into(),
                },
            ]
        );
    }

    #[test]
    fn the_abilities_are_what_the_greeting_lists() {
        let abilities = MuffitConversation::abilities();
        assert!(abilities.input.contains(&InputAbility::Images));
        assert!(abilities.steer.contains(&Steer::Interrupt));
        assert!(abilities.steer.contains(&Steer::Continue));
        assert!(abilities.steer.contains(&Steer::Pause));
        assert!(abilities.steer.contains(&Steer::Resume));
        assert!(abilities.steer.contains(&Steer::Stop));
        assert!(abilities.steer.contains(&Steer::Compact));
        assert!(abilities.asks.contains(&AskAbility::Permission));
        assert!(abilities.asks.contains(&AskAbility::Always));
        assert!(abilities.streams.contains(&Stream::Text));
        assert!(abilities.streams.contains(&Stream::TaskOutput));
        assert!(abilities.facts.contains(&Fact::Context));
        assert!(abilities.facts.contains(&Fact::Cost));
        assert!(abilities.facts.contains(&Fact::Trouble));
        assert!(abilities.lifecycle.contains(&Lifecycle::Create));
        assert!(abilities.lifecycle.contains(&Lifecycle::Resume));
        assert!(abilities.lifecycle.contains(&Lifecycle::End));
    }
}