use tokio::sync::mpsc;

pub mod abilities;
pub mod ask;
pub mod command;
pub mod composer;
pub mod control;
pub mod facts;
pub mod update;

pub use abilities::*;
pub use ask::*;
pub use command::*;
pub use composer::*;
pub use control::*;
pub use facts::*;
pub use update::*;

pub use kampr_journal::facet::Running;
/// The turn, block and facet vocabulary a conversation speaks. It is defined in `kampr-journal`,
/// which keeps the parsers that fill it; this is where the seam names it.
pub use kampr_journal::{
    Attachment, Block, CodeRole, ConvoFound, ConvoHit, Facets, Page, Role, Title, TitleSource, ToolState,
    Turn, TurnKind,
};

/// What a conversation sends the pane: one value in, one `bool` out, and `false` ends it.
pub type Emit = Box<dyn FnMut(update::Update) -> bool + Send>;

#[async_trait::async_trait]
pub trait Conversation: Send + 'static {
    async fn run(self: Box<Self>, commands: mpsc::Receiver<Command>, emit: Emit);
}

#[async_trait::async_trait]
pub trait Source: Send + Sync + 'static {
    fn kind(&self) -> &'static str;
    async fn open(&self, pane: &str) -> Result<Box<dyn Conversation>, Refusal>;
    async fn create(&self, new: NewSession) -> Result<String, Refusal>;
    async fn resumable(&self, cwd: Option<&std::path::Path>) -> Vec<Stored>;
    async fn end(&self, pane: &str, how: Ending) -> Result<(), Refusal>;
}

/// A source's no: the message is what the pane is told.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub message: String,
}

impl Refusal {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub struct NewSession {
    pub cwd: Option<std::path::PathBuf>,
    pub prompt: Option<String>,
}

/// A stored conversation a source can be asked back, by id.
#[derive(Debug, Clone)]
pub struct Stored {
    pub id: String,
    pub cwd: Option<std::path::PathBuf>,
    pub title: Option<String>,
}

pub enum Ending {
    End,
    ToTerminal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Dummy;

    #[async_trait::async_trait]
    impl Conversation for Dummy {
        async fn run(self: Box<Self>, commands: mpsc::Receiver<Command>, mut emit: Emit) {
            drop(commands);
            (emit)(Update::Failed("the end".into()));
        }
    }

    #[tokio::test]
    async fn a_conversation_runs_to_its_false() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let keep = seen.clone();
        Box::new(Dummy)
            .run(
                mpsc::channel::<Command>(1).1,
                Box::new(move |update| {
                    keep.lock().unwrap().push(update);
                    true
                }),
            )
            .await;
        assert_eq!(&*seen.lock().unwrap(), &[Update::Failed("the end".into())]);
    }

    #[derive(Default)]
    struct SourceDummy;

    #[async_trait::async_trait]
    impl Source for SourceDummy {
        fn kind(&self) -> &'static str {
            "dummy"
        }
        async fn open(&self, _pane: &str) -> Result<Box<dyn Conversation>, Refusal> {
            Ok(Box::new(Dummy))
        }
        async fn create(&self, _new: NewSession) -> Result<String, Refusal> {
            Ok("s1".into())
        }
        async fn resumable(&self, _cwd: Option<&std::path::Path>) -> Vec<Stored> {
            Vec::new()
        }
        async fn end(&self, _pane: &str, _how: Ending) -> Result<(), Refusal> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_source_opens_a_conversation() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let keep = seen.clone();
        let conversation = SourceDummy.open("p1").await.unwrap();
        conversation
            .run(
                mpsc::channel::<Command>(1).1,
                Box::new(move |update| {
                    keep.lock().unwrap().push(update);
                    true
                }),
            )
            .await;
        assert_eq!(&*seen.lock().unwrap(), &[Update::Failed("the end".into())]);
    }
}
