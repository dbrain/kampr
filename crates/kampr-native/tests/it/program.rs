//! What programs report over `OSC 7501` reaches a session's status through a real keeper, and what
//! ends with the process stops being said (docs/24).

use kampr_core::program::ProgramState;
use kampr_core::provider::{AgentStatus, Input, Provider};
use kampr_keep::Launch;
use kampr_native::{NativeConfig, NativeProvider, NewSession};
use std::time::{Duration, Instant};

const SOON: Duration = Duration::from_secs(20);

struct Root(tempfile::TempDir);

impl Root {
    fn new() -> Self {
        Self(
            tempfile::Builder::new()
                .prefix("kq")
                .tempdir_in("/tmp")
                .expect("a root"),
        )
    }

    fn provider(&self) -> NativeProvider {
        let mut config = NativeConfig::new(Some(self.0.path().to_path_buf()), Some(kampr_testkit::keeper()))
            .expect("a config");
        config.launch = Some(Launch::Detached);
        NativeProvider::new(config).expect("a provider")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        kampr_testkit::stop_keepers(self.0.path());
    }
}

async fn session(provider: &NativeProvider, script: &str) -> String {
    provider
        .create(NewSession {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cols: 80,
            rows: 24,
            ..NewSession::default()
        })
        .await
        .expect("a session")
}

async fn status(provider: &NativeProvider, pane: &str) -> Option<AgentStatus> {
    let panes = provider.list_panes().await.ok()?;
    panes
        .into_iter()
        .find(|p| p.pane_id == pane)
        .map(|p| p.agent_status)
}

async fn until<T>(what: &str, mut probe: impl AsyncFnMut() -> Option<T>) -> T {
    let end = Instant::now() + SOON;
    loop {
        if let Some(found) = probe().await {
            return found;
        }
        assert!(Instant::now() < end, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A worker's permission is the session's, whatever the root says, and answering it gives the
/// session back: the status ladder reads the fold, not the last report.
#[tokio::test(flavor = "multi_thread")]
async fn a_workers_block_is_the_sessions_status_until_it_is_cleared() {
    let root = Root::new();
    let provider = root.provider();
    let pane = session(&provider, "exec cat").await;
    let say = |bytes: &'static str| {
        let provider = &provider;
        let pane = &pane;
        async move {
            provider
                .write_pane(pane, Input::Bytes(bytes.as_bytes().to_vec()))
                .await
                .expect("typed");
        }
    };
    say("\x1b]7501;state=working:app=claude-code\x07\n").await;
    until("working", async || {
        (status(&provider, &pane).await == Some(AgentStatus::Working)).then_some(())
    })
    .await;
    say("\x1b]7501;state=blocked:app=claude-code:id=agent-1:kind=permission:msg=YXBwcm92ZQ==\x07\n").await;
    until("blocked", async || {
        (status(&provider, &pane).await == Some(AgentStatus::Blocked)).then_some(())
    })
    .await;
    assert_eq!(
        provider.program(&pane).and_then(|p| p.message).as_deref(),
        Some("approve"),
        "what the pending strip shows"
    );
    say("\x1b]7501;state=blocked:id=agent-2:msg=c2Vjb25k\x07\n").await;
    say("\x1b]7501;state=clear:id=agent-1\x07\n").await;
    until("still blocked on the second worker", async || {
        (provider.program(&pane).and_then(|p| p.message).as_deref() == Some("second")).then_some(())
    })
    .await;
    assert_eq!(status(&provider, &pane).await, Some(AgentStatus::Blocked));
    say("\x1b]7501;state=clear:id=agent-2\x07\n").await;
    until("working again", async || {
        (status(&provider, &pane).await == Some(AgentStatus::Working)).then_some(())
    })
    .await;
}

/// `working` and `blocked` end with the process that reported them; what finished stays, and a
/// node that starts over the ended session reads the same, from the log or from a checkpoint.
#[tokio::test(flavor = "multi_thread")]
async fn what_was_running_ends_with_the_session_and_what_finished_is_kept_across_a_restart() {
    let root = Root::new();
    let first = root.provider();
    let pane = session(
        &first,
        "printf '\\033]7501;state=working\\007\\033]7501;state=blocked:id=w\\007\\033]7501;state=error:id=e:msg=Ym9vbQ==\\007'; sleep 1; exit 3",
    )
    .await;
    for (provider, which) in [
        (&first, "the node that watched it"),
        (&root.provider(), "a node started after it ended"),
    ] {
        let left = until(which, async || {
            let program = provider.program(&pane)?;
            (program.state == ProgramState::Error).then_some(program)
        })
        .await;
        assert_eq!(left.message.as_deref(), Some("boom"), "{which}");
    }
}
