//! The herd follows a session's foreground job by itself: a job that starts or ends with nothing
//! on the screen the status reads still changes the herd.

use kampr_core::desk::Desk;
use kampr_core::provider::{Input, Provider};
use kampr_keep::Launch;
use kampr_native::{NativeConfig, NativeProvider, NewSession};
use std::time::{Duration, Instant};

const SOON: Duration = Duration::from_secs(10);

struct Root(tempfile::TempDir);

impl Drop for Root {
    fn drop(&mut self) {
        for session in kampr_keep::list(self.0.path()).unwrap_or_default() {
            if let Some(keeper) = session.keeper() {
                unsafe {
                    libc::kill(-(keeper.child as i32), libc::SIGKILL);
                    libc::kill(keeper.keeper as i32, libc::SIGKILL);
                }
            }
        }
    }
}

async fn cmd(provider: &NativeProvider, pane: &str) -> Option<String> {
    let panes = provider.list_panes().await.expect("panes");
    panes.into_iter().find(|p| p.pane_id == pane).and_then(|p| p.cmd)
}

/// Waits until the herd changes to a `cmd` of `want`, and says what it saw instead: each topology
/// change, and what a fresh read gives at the end.
async fn changes_to(provider: &NativeProvider, pane: &str, want: Option<&str>) {
    let mut changes = provider.changes();
    let mut seen = Vec::new();
    let deadline = Instant::now() + SOON;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        if tokio::time::timeout(left, changes.changed()).await.is_err() {
            break;
        }
        let now = cmd(provider, pane).await;
        if now.as_deref() == want {
            return;
        }
        seen.push(now);
    }
    panic!(
        "the herd never changed to {want:?}; at each change it read {seen:?}, and reads {:?} now",
        cmd(provider, pane).await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_ends_without_a_word_changes_the_herd() {
    unsafe { std::env::set_var("KAMPR_DEBUG_JOB", "1") };
    let root = Root(
        tempfile::Builder::new()
            .prefix("kj")
            .tempdir_in("/tmp")
            .expect("a root"),
    );
    let mut config = NativeConfig::new(Some(root.0.path().to_path_buf()), Some(kampr_testkit::keeper()))
        .expect("a config");
    config.launch = Some(Launch::Detached);
    let provider = NativeProvider::new(config).expect("a provider");
    let pane = provider
        .create(NewSession {
            argv: vec!["sh".into(), "-i".into()],
            cwd: None,
            label: None,
            cols: 80,
            rows: 24,
            env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), "/dev/null".into())],
            cohort: None,
        })
        .await
        .expect("a session");
    tokio::time::sleep(Duration::from_millis(500)).await;
    provider
        .write_pane(&pane, Input::Bytes(b"cat\r".to_vec()))
        .await
        .expect("typed");
    changes_to(&provider, &pane, Some("cat")).await;
    provider
        .write_pane(&pane, Input::Bytes(b"\x04".to_vec()))
        .await
        .expect("typed");
    changes_to(&provider, &pane, None).await;
}
