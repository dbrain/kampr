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
        kampr_testkit::stop_keepers(self.0.path());
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
        "the herd never changed to {want:?}; at each change it read {seen:?}, and reads {:?} now; \
         the screen:\n{}",
        cmd(provider, pane).await,
        provider.visible(pane).unwrap_or_default()
    );
}

async fn shell() -> (Root, NativeProvider, String) {
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
            workspace: None,
            container: None,
        })
        .await
        .expect("a session");
    tokio::time::sleep(Duration::from_millis(500)).await;
    (root, provider, pane)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_ends_without_a_word_changes_the_herd() {
    let (_root, provider, pane) = shell().await;
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

/// A job read by the herd and ended before the follower next looks: what the herd was told is
/// what has to be taken back, whether or not the follower ever saw it. Measured on CI's macOS
/// runner, where this `cat` lived less than one look.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_the_herd_was_told_of_is_taken_back_even_if_it_ended_between_looks() {
    let (_root, provider, pane) = shell().await;
    for _ in 0..10 {
        provider
            .write_pane(&pane, Input::Bytes(b"cat\r".to_vec()))
            .await
            .expect("typed");
        let deadline = Instant::now() + SOON;
        while cmd(&provider, &pane).await.as_deref() != Some("cat") {
            assert!(Instant::now() < deadline, "cat never ran");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        provider
            .write_pane(&pane, Input::Bytes(b"\x04".to_vec()))
            .await
            .expect("typed");
        changes_to(&provider, &pane, None).await;
    }
}

/// macOS names a process after the file its symlink resolved to, so Claude's installed
/// `~/.local/bin/claude` ran as `2.1.308` (its `versions/` file); the job is what was typed. The link
/// is to `sh`, which waits on the terminal under any name: ubuntu-26.04's coreutils is one
/// multi-call binary that refuses a name that is none of its tools.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_run_through_a_symlink_is_named_by_the_link() {
    let (root, provider, pane) = shell().await;
    let link = root.0.path().join("widget");
    std::os::unix::fs::symlink(which("sh"), &link).expect("a link");
    provider
        .write_pane(&pane, Input::Bytes(format!("{}\r", link.display()).into_bytes()))
        .await
        .expect("typed");
    changes_to(&provider, &pane, Some("widget")).await;
}

fn which(program: &str) -> std::path::PathBuf {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|dir| std::path::Path::new(dir).join(program))
        .find(|p| p.is_file())
        .expect("on PATH")
}
