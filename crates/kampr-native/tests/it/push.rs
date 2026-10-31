//! A native session says when it changed, so nothing that follows one waits on a timer.

use kampr_core::provider::{AgentStatus, Input, Provider};
use kampr_core::push::PulseRx;
use kampr_core::registry::{HistoryPolicy, PaneRegistry, RegistryConfig};
use kampr_keep::Launch;
use kampr_native::{NativeConfig, NativeProvider, NewSession};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The node's output window, `live::WORKING`.
const WORKING: Duration = Duration::from_millis(1500);

struct Root(tempfile::TempDir);

impl Root {
    fn new() -> Self {
        Self(
            tempfile::Builder::new()
                .prefix("kp")
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

async fn ring_lines(registry: &PaneRegistry, pane: &str) -> Vec<String> {
    let Ok(Some(doc)) = registry.scrollback(pane).await else {
        return Vec::new();
    };
    doc.rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Rows in the provider's own history, the screen below it left out.
async fn provider_history(provider: &NativeProvider, pane: &str) -> Vec<String> {
    let Ok(Some(raw)) = provider.read_scrollback(pane).await else {
        return Vec::new();
    };
    let mut lines: Vec<String> = raw.text.lines().map(|l| l.to_string()).collect();
    lines.truncate(lines.len().saturating_sub(usize::from(raw.viewport_rows)));
    lines
}

/// The rate estimate waits up to `row_budget / rate` between reads of a pane that is producing,
/// because herdr announces no output: a row a second is read every eight. A native session
/// announces its history, so each row reaches the ring as it leaves the screen.
#[tokio::test(flavor = "multi_thread")]
async fn a_trickling_pane_reaches_the_ring_as_its_rows_enter_history() {
    let root = Root::new();
    let provider = Arc::new(root.provider());
    let pane = provider
        .create(NewSession {
            argv: vec!["sh".into(), "-i".into()],
            cols: 40,
            rows: 4,
            env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), "/dev/null".into())],
            ..NewSession::default()
        })
        .await
        .expect("a session");
    let registry = PaneRegistry::with_config(
        provider.clone(),
        RegistryConfig {
            history: HistoryPolicy {
                row_budget: 8,
                fastest: Duration::from_millis(20),
                quiet: Duration::from_secs(30),
                idle: Duration::from_secs(600),
            },
            ..RegistryConfig::default()
        },
    );
    let _watch = registry.watch(&pane).await.expect("watched");
    provider
        .write_pane(
            &pane,
            Input::Bytes(b"for i in 1 2 3 4 5 6 7 8; do echo r$((i+100)); sleep 0.6; done\r".to_vec()),
        )
        .await
        .expect("typed");
    let started = Instant::now();
    let mut lags = Vec::new();
    for n in 101..=104 {
        let want = format!("r{n}");
        while !provider_history(&provider, &pane).await.contains(&want) {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "{want} never left the screen"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let entered = Instant::now();
        while !ring_lines(&registry, &pane).await.contains(&want) {
            assert!(
                entered.elapsed() < Duration::from_secs(20),
                "{want} never reached the ring"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        lags.push(entered.elapsed());
    }
    assert!(
        lags.iter().all(|lag| *lag < Duration::from_millis(300)),
        "how long each row took to reach the ring after it entered history: {lags:?}"
    );
}

#[cfg(target_os = "linux")]
fn watch_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|tasks| {
            tasks
                .flatten()
                .filter(|t| {
                    std::fs::read_to_string(t.path().join("comm"))
                        .is_ok_and(|c| c.trim() == "kampr-native-wa")
                })
                .count()
        })
        .unwrap_or_default()
}

/// Reads this process's own threads, which is one test's alone under nextest.
#[cfg(target_os = "linux")]
#[test]
fn a_dropped_provider_takes_its_watch_thread_with_it_at_once() {
    let root = Root::new();
    let provider = root.provider();
    let made = Instant::now();
    // A spawned thread names itself, so it is not yet called anything when `provider` returns.
    while watch_threads() != 1 {
        assert!(made.elapsed() < Duration::from_secs(5), "no watch thread");
        std::thread::sleep(Duration::from_millis(2));
    }
    let dropped = Instant::now();
    drop(provider);
    while watch_threads() > 0 {
        assert!(dropped.elapsed() < Duration::from_secs(5), "the thread stayed");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        dropped.elapsed() < Duration::from_millis(100),
        "the thread outlived its provider by {:?}",
        dropped.elapsed()
    );
}

async fn shell(root: &Root, ps1: &str) -> (NativeProvider, String) {
    let provider = root.provider();
    let pane = provider
        .create(NewSession {
            argv: vec!["sh".into(), "-i".into()],
            cols: 80,
            rows: 24,
            env: vec![("PS1".into(), ps1.into()), ("ENV".into(), "/dev/null".into())],
            ..NewSession::default()
        })
        .await
        .expect("a session");
    (provider, pane)
}

async fn status(provider: &NativeProvider, pane: &str) -> AgentStatus {
    let panes = provider.list_panes().await.expect("panes");
    panes
        .into_iter()
        .find(|p| p.pane_id == pane)
        .expect("the pane")
        .agent_status
}

async fn typed(provider: &NativeProvider, pane: &str, line: &str) {
    provider
        .write_pane(pane, Input::Bytes(line.as_bytes().to_vec()))
        .await
        .expect("typed");
}

async fn moves(rx: &mut PulseRx, what: &str) -> Instant {
    assert!(
        rx.changed_before(tokio::time::Instant::now() + Duration::from_secs(10))
            .await,
        "{what} never moved"
    );
    Instant::now()
}

/// Every thread a native session keeps, the provider's directory watch left out.
#[cfg(target_os = "linux")]
fn session_wakes() -> u64 {
    std::fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|t| {
            std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| {
                let c = c.trim();
                c.starts_with("kampr-native-") && c != "kampr-native-wa"
            })
        })
        .filter_map(|t| std::fs::read_to_string(t.path().join("status")).ok())
        .flat_map(|status| {
            status
                .lines()
                .filter(|l| l.contains("ctxt_switches"))
                .filter_map(|l| l.split_whitespace().last()?.parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .sum()
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_idle_session_wakes_nothing() {
    let root = Root::new();
    let (provider, pane) = shell(&root, "$ ").await;
    let mut statuses = provider.pane_changes(&pane).status.expect("a status pulse");
    tokio::time::sleep(Duration::from_secs(3)).await;
    statuses.seen();
    let before = session_wakes();
    assert!(before > 0, "no session threads were found");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(session_wakes() - before, 0, "an idle session's threads woke");
    assert!(!statuses.moved(), "an idle session's status moved");
}

/// A job that starts or ends is seen by the look its own output or its leader's end asks for,
/// not by a periodic one, so it is announced in far less than the old 250 ms tick; an end is seen
/// as the leader dies, before any look that output after it would ask for.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_starting_and_ending_is_announced_at_once() {
    let root = Root::new();
    let (provider, pane) = shell(&root, "").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pidfile = root.0.path().join("job.pid");
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for _ in 0..4 {
        let _ = std::fs::remove_file(&pidfile);
        let mut screen = provider.pane_changes(&pane).screen.expect("a screen pulse");
        typed(
            &provider,
            &pane,
            &format!("sh -c 'echo $$ > {}; exec sleep 30'\r", pidfile.display()),
        )
        .await;
        let asked = Instant::now();
        starts.push(moves(&mut screen, "a starting job").await - asked);
        let pid: i32 = loop {
            if let Some(pid) = std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|s| s.trim().parse().ok())
            {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        tokio::time::sleep(Duration::from_millis(400)).await;
        screen.seen();
        let killed = Instant::now();
        unsafe { libc::kill(pid, libc::SIGKILL) };
        ends.push(moves(&mut screen, "an ending job").await - killed);
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    assert!(
        starts.iter().all(|d| *d < Duration::from_millis(150))
            && ends.iter().all(|d| *d < Duration::from_millis(40)),
        "starts announced after {starts:?}, ends after {ends:?}"
    );
}

/// The output window closes on its own deadline, not at the next tick after it. The echo of a
/// keystroke is output that is not work, and it moved the old tick off the window's end.
#[tokio::test(flavor = "multi_thread")]
async fn working_drops_to_idle_on_the_deadline() {
    let root = Root::new();
    let (provider, pane) = shell(&root, "$ ").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut late = Vec::new();
    for _ in 0..3 {
        let mut statuses = provider.pane_changes(&pane).status.expect("a status pulse");
        typed(&provider, &pane, "sleep 0.5; echo done\r").await;
        let working = loop {
            let at = moves(&mut statuses, "the status").await;
            if status(&provider, &pane).await == AgentStatus::Working {
                break at;
            }
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        statuses.seen();
        typed(&provider, &pane, " ").await;
        let idle = moves(&mut statuses, "the status").await;
        assert_ne!(status(&provider, &pane).await, AgentStatus::Working);
        late.push((idle - working).saturating_sub(WORKING));
        typed(&provider, &pane, "\x15").await;
    }
    assert!(
        late.iter().all(|d| *d < Duration::from_millis(60)),
        "how long past the window each status dropped: {late:?}"
    );
}

/// A harness's first report changes which source the herd reads its status from, even when the
/// word it reports is the one its output had already earned.
#[tokio::test(flavor = "multi_thread")]
async fn a_report_from_a_pane_already_working_moves_the_status_pulse() {
    let root = Root::new();
    let (provider, pane) = shell(&root, "$ ").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut statuses = provider.pane_changes(&pane).status.expect("a status pulse");
    typed(
        &provider,
        &pane,
        "for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo x; sleep 0.1; done; \
         printf '\\033]7501;state=working:app=claude-code\\007'; sleep 5\r",
    )
    .await;
    loop {
        moves(&mut statuses, "the status").await;
        if status(&provider, &pane).await == AgentStatus::Working {
            break;
        }
    }
    statuses.seen();
    assert!(
        statuses
            .changed_before(tokio::time::Instant::now() + Duration::from_secs(3))
            .await,
        "the report moved nothing"
    );
}
