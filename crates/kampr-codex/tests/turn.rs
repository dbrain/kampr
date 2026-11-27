//! A conversation against the probe's mock: create a thread, open it, run a turn to
//! completion, interrupt a running one, and watch a failed turn become a Trouble.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kampr_codex::CodexSource;
use kampr_core::conversation::{
    Block, Command, Conversation, NewSession, Role, RunState, Source, Steer, Stream, Turn, Update, When,
};
use tokio::sync::mpsc;

const REPLY: &str = "Hello from the mock codex.";

fn mock() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("research/probe/codex-app-server/mock-stdio.py")
        .canonicalize()
        .unwrap()
}

/// Every update the conversation has sent the pane, in order.
struct Seen(Arc<Mutex<Vec<Update>>>);

impl Seen {
    fn new() -> Self {
        Seen(Arc::new(Mutex::new(Vec::new())))
    }

    fn emit(&self) -> Box<dyn FnMut(Update) -> bool + Send> {
        let seen = self.0.clone();
        Box::new(move |update| {
            seen.lock().unwrap().push(update);
            true
        })
    }

    fn all(&self) -> Vec<Update> {
        self.0.lock().unwrap().clone()
    }

    /// The index of the first update that matches, if any.
    fn index_of(&self, want: &dyn Fn(&Update) -> bool) -> Option<usize> {
        self.0.lock().unwrap().iter().position(want)
    }

    /// Wait until an update matches, or fail with everything seen so far.
    async fn until(&self, what: &str, want: &dyn Fn(&Update) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if self.index_of(want).is_some() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; saw: {:#?}",
                self.all()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn source(home: &Path, mode: Option<&str>) -> CodexSource {
    let mut env = vec![("CODEX_HOME".to_string(), home.display().to_string())];
    if let Some(mode) = mode {
        env.push(("KAMPR_MOCK_MODE".to_string(), mode.to_string()));
    }
    CodexSource::new("python3", vec![mock().to_string_lossy().into()], env)
}

async fn run(conversation: Box<dyn Conversation>) -> (mpsc::Sender<Command>, Seen) {
    let (tx, rx) = mpsc::channel::<Command>(16);
    let seen = Seen::new();
    tokio::spawn(conversation.run(rx, seen.emit()));
    (tx, seen)
}

async fn input(tx: &mpsc::Sender<Command>, text: &str) {
    tx.send(Command::Input {
        text: text.into(),
        msg_id: format!("m{}", text.len()),
        images: vec![],
        files: vec![],
        when: When::Idle,
    })
    .await
    .unwrap();
}

fn says_reply(block: &Block) -> bool {
    matches!(block, Block::Md { text, .. } if text.contains(REPLY))
}

fn has_reply_turns(turns: &[Turn]) -> bool {
    turns
        .iter()
        .any(|t| t.role == Role::Assistant && t.blocks.iter().any(says_reply))
}

fn live_with(text: &str) -> impl Fn(&Update) -> bool + '_ {
    move |update| matches!(update, Update::Live(live) if live.text.contains(text))
}

#[tokio::test]
async fn a_turn_completes_and_the_thread_remembers_it() {
    let home = tempfile::tempdir().unwrap();
    let source = source(home.path(), None);

    let id = source
        .create(NewSession {
            cwd: Some(home.path().to_path_buf()),
            prompt: None,
        })
        .await
        .unwrap();
    assert!(!id.is_empty());

    // resumable sees the thread the source just created
    let stored = source.resumable(None).await;
    let ours = stored
        .iter()
        .find(|s| s.id == id)
        .expect("the new thread is resumable");
    assert_eq!(ours.cwd, Some(home.path().to_path_buf()));

    let (tx, seen) = run(source.open(&id).await.unwrap()).await;

    // open told us what this session is
    seen.until("session", &|u| matches!(u, Update::Session(s) if s.id == id))
        .await;
    seen.until(
        "abilities",
        &|u| matches!(u, Update::Abilities(a) if a.streams.contains(&Stream::Text)),
    )
    .await;

    input(&tx, "Say hello").await;

    seen.until("running", &|u| matches!(u, Update::State(RunState::Running)))
        .await;
    seen.until("the streamed reply", &live_with(REPLY)).await;
    seen.until(
        "the finished turns",
        &|u| matches!(u, Update::Turns(turns) if has_reply_turns(turns)),
    )
    .await;
    seen.until("idle", &|u| matches!(u, Update::State(RunState::Idle)))
        .await;

    // the finished turn survives: a second pane on the same thread gets it in its page
    let (_tx2, seen2) = run(source.open(&id).await.unwrap()).await;
    seen2
        .until(
            "history",
            &|u| matches!(u, Update::Page(page) if has_reply_turns(&page.turns)),
        )
        .await;
}

#[tokio::test]
async fn an_interrupt_ends_the_running_turn() {
    let home = tempfile::tempdir().unwrap();
    let source = source(home.path(), Some("slow"));

    let id = source
        .create(NewSession {
            cwd: None,
            prompt: None,
        })
        .await
        .unwrap();
    let (tx, seen) = run(source.open(&id).await.unwrap()).await;

    input(&tx, "Take your time").await;
    seen.until("the first word", &live_with("Hello")).await;

    let live_index = seen.index_of(&live_with("Hello")).unwrap();
    tx.send(Command::Steer(Steer::Interrupt, None)).await.unwrap();
    seen.until("the interrupt idled us", &|u| {
        matches!(u, Update::State(RunState::Idle))
    })
    .await;
    // the idle came after the live we saw, i.e. the interrupt did it
    let idle_index = seen
        .all()
        .iter()
        .rposition(|u| matches!(u, Update::State(RunState::Idle)))
        .unwrap();
    assert!(idle_index > live_index);
}

#[tokio::test]
async fn a_failed_turn_becomes_a_trouble() {
    let home = tempfile::tempdir().unwrap();
    let source = source(home.path(), Some("error429"));

    let id = source
        .create(NewSession {
            cwd: None,
            prompt: None,
        })
        .await
        .unwrap();
    let (tx, seen) = run(source.open(&id).await.unwrap()).await;

    input(&tx, "Please fail").await;
    seen.until(
        "the 429",
        &|u| matches!(u, Update::Trouble(Some(t)) if t.text.contains("429")),
    )
    .await;
    seen.until("idle", &|u| matches!(u, Update::State(RunState::Idle)))
        .await;
}

#[tokio::test]
async fn creating_with_a_prompt_runs_the_first_turn_to_completion() {
    let home = tempfile::tempdir().unwrap();
    let source = source(home.path(), None);

    let id = source
        .create(NewSession {
            cwd: None,
            prompt: Some("Say hello".into()),
        })
        .await
        .unwrap();

    let (tx, seen) = run(source.open(&id).await.unwrap()).await;
    // the first turn finished before the pane opened: its turn is in the history page
    seen.until(
        "history",
        &|u| matches!(u, Update::Page(page) if has_reply_turns(&page.turns)),
    )
    .await;
    drop(tx);
}
