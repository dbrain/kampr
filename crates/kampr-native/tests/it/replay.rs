//! Replays of a real keeper's log: a node rebuilt over the same root must hold the screen and
//! history the node that watched it live holds, whatever the session was resized to and however
//! much of its log was trimmed.

use kampr_core::provider::Provider;
use kampr_keep::{Launch, Replay, SessionDir, SizedReader, Spec};
use kampr_native::{NativeConfig, NativeProvider, Screen};
use std::path::Path;
use std::time::{Duration, Instant};

const SOON: Duration = Duration::from_secs(20);
const SCROLLBACK: usize = 10_000;
const MIB: u64 = 1 << 20;

/// After each `go`, output whose shape depends on the size at the moment: the size itself, a bar
/// at the right edge and the bottom, and a line longer than the narrowest width.
const SIZED: &str = r#"stty -echo; printf '\033]2;sized\007\033[1;35m'
while read l; do case $l in
  big) head -c 3000000 /dev/zero | tr '\0' y | fold -w 70; echo big-done;;
  *) stty size; printf '\033[999C|\r\n'; seq -s, 1 40; printf 'at \033[999B\033[999C#\r\n'; echo "done $l";;
esac; done"#;

struct Root(tempfile::TempDir);

impl Root {
    fn new() -> Self {
        Self(
            tempfile::Builder::new()
                .prefix("kr")
                .tempdir_in("/tmp")
                .expect("a root"),
        )
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn provider(&self) -> NativeProvider {
        let mut config = NativeConfig::new(Some(self.path().to_path_buf()), Some(kampr_testkit::keeper()))
            .expect("a config");
        config.launch = Some(Launch::Detached);
        NativeProvider::new(config).expect("a provider")
    }

    fn spawn(&self, log_cap: Option<u64>) -> (SessionDir, String) {
        let mut spec = Spec::new(vec!["sh".into(), "-c".into(), SIZED.into()]);
        spec.cols = 100;
        spec.rows = 30;
        spec.log_cap = log_cap;
        let session = kampr_keep::spawn(self.path(), spec, Launch::Detached, &kampr_testkit::keeper())
            .expect("a session");
        let pane = format!("n:{}", session.id());
        (session, pane)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        for session in kampr_keep::list(self.path()).unwrap_or_default() {
            if let Some(keeper) = session.keeper() {
                unsafe {
                    libc::kill(-(keeper.child as i32), libc::SIGKILL);
                    libc::kill(keeper.keeper as i32, libc::SIGKILL);
                }
            }
        }
    }
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + SOON;
    while !ok() {
        assert!(Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn log_bounds(session: &SessionDir) -> (u64, u64) {
    let reader = SizedReader::at(session.path(), 0).expect("a reader");
    (reader.start().expect("start"), reader.end().expect("end"))
}

/// Typed straight at the master, as `kampr attach` would: the provider under test may be gone.
fn type_line(session: &SessionDir, line: &str) {
    let master = kampr_keep::borrow(session).expect("the master");
    rustix::io::write(&master, format!("{line}\n").as_bytes()).expect("typed");
}

async fn step(provider: &NativeProvider, session: &SessionDir, pane: &str, line: &str) {
    type_line(session, line);
    let done = format!("done {line}");
    until(&done, || {
        provider.visible(pane).is_some_and(|v| v.contains(&done))
    })
    .await;
    settle(provider, session, pane).await;
}

/// Until the provider has fed the whole log and the log has stopped growing.
async fn settle(provider: &NativeProvider, session: &SessionDir, pane: &str) {
    let mut last = None;
    until("the log to settle", || {
        let now = (log_bounds(session).1, shown(provider, pane));
        let same = last.as_ref() == Some(&now);
        last = Some(now);
        same
    })
    .await;
}

fn shown(provider: &NativeProvider, pane: &str) -> String {
    let scrollback = futures_lite(provider.read_scrollback(pane));
    format!(
        "{:?}\n{}\n{}",
        provider.size(pane),
        provider.visible(pane).unwrap_or_default(),
        scrollback.map(|s| s.text).unwrap_or_default()
    )
}

fn futures_lite<T>(f: impl std::future::Future<Output = anyhow::Result<Option<T>>>) -> Option<T> {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f)).expect("a read")
}

async fn resize(provider: &NativeProvider, pane: &str, cols: u16, rows: u16) {
    provider.resize(pane, cols, rows, "test").await.expect("a resize");
}

async fn rebuilt(root: &Root, session: &SessionDir, pane: &str, last: &str) -> (NativeProvider, String) {
    let provider = root.provider();
    until(last, || provider.visible(pane).is_some_and(|v| v.contains(last))).await;
    settle(&provider, session, pane).await;
    let shown = shown(&provider, pane);
    (provider, shown)
}

fn checkpoints(session: &SessionDir) -> Vec<u64> {
    let mut found: Vec<u64> = std::fs::read_dir(session.path())
        .expect("the session dir")
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("screen.")?.parse().ok())
        .collect();
    found.sort_unstable();
    found
}

/// **Each resize lands at the byte it happened**, for a node that watched it and for one built
/// afterwards over the same root, with the checkpoint the first one left and without it.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_resized_mid_output_rebuilds_exactly_as_it_was_watched() {
    let root = Root::new();
    let (session, pane) = root.spawn(None);
    let live = root.provider();
    step(&live, &session, &pane, "first").await;
    for (cols, rows) in [(50, 20), (120, 25), (40, 30), (90, 24)] {
        resize(&live, &pane, cols, rows).await;
        step(&live, &session, &pane, &format!("{cols}x{rows}")).await;
    }
    let watched = shown(&live, &pane);
    assert!(
        watched.contains("24 90") && watched.contains("20 50"),
        "{watched}"
    );

    for checkpointed in [false, true] {
        if checkpointed {
            live.shutdown();
            assert!(
                !checkpoints(&session).is_empty(),
                "a node going away leaves a checkpoint"
            );
        } else {
            for at in checkpoints(&session) {
                std::fs::remove_file(session.path().join(format!("screen.{at:020}"))).unwrap();
            }
        }
        let (_node, shown) = rebuilt(&root, &session, &pane, "done 90x24").await;
        assert_eq!(shown, watched, "rebuilt with a checkpoint: {checkpointed}");
    }
}

/// **A log trimmed past its cap rebuilds from the newest checkpoint and the bytes after it**,
/// with the pen, title and modes the program set long before the oldest byte still held.
#[tokio::test(flavor = "multi_thread")]
async fn a_trimmed_log_rebuilds_exactly_from_a_checkpoint_and_its_tail() {
    let root = Root::new();
    let (session, pane) = root.spawn(Some(2 * MIB));
    let live = root.provider();
    step(&live, &session, &pane, "first").await;
    type_line(&session, "big");
    until("big-done", || {
        live.visible(&pane).is_some_and(|v| v.contains("big-done"))
    })
    .await;
    step(&live, &session, &pane, "after").await;
    let (start, end) = log_bounds(&session);
    assert!(start > 0, "the log was never trimmed");
    let saved = checkpoints(&session);
    assert!(
        saved.iter().any(|&at| at >= start && at < end),
        "no retained checkpoint short of the end: {saved:?} in {start}..{end}"
    );
    let watched = shown(&live, &pane);
    assert!(
        watched.contains("\x1b[0;1;38;5;5mdone after"),
        "the pen is in the rows"
    );

    let (_node, shown) = rebuilt(&root, &session, &pane, "done after").await;
    assert_eq!(shown, watched);
}

/// **A checkpoint the log was trimmed past while no node was running is never used**: the replay
/// starts at the oldest byte, exactly as a reader with no checkpoint at all would.
#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_trimmed_past_while_the_node_was_down_is_ignored() {
    let root = Root::new();
    let (session, pane) = root.spawn(Some(2 * MIB));
    let live = root.provider();
    step(&live, &session, &pane, "before").await;
    live.shutdown();
    drop(live);
    let stale = checkpoints(&session);
    assert!(!stale.is_empty());

    type_line(&session, "big");
    type_line(&session, "later");
    let deadline = Instant::now() + SOON;
    while log_bounds(&session).0 <= *stale.last().unwrap() {
        assert!(
            Instant::now() < deadline,
            "the log was never trimmed past {stale:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (_node, shown) = rebuilt(&root, &session, &pane, "done later").await;
    assert!(
        !shown.contains("done before"),
        "the stale checkpoint was spliced in"
    );
    assert_eq!(shown, from_oldest(&session));
}

/// The rows and size a reader with no checkpoint rebuilds, in the provider's own reading shape.
fn from_oldest(session: &SessionDir) -> String {
    let mut reader = SizedReader::at(session.path(), log_bounds(session).0).expect("a reader");
    let size = reader.size().clone();
    let mut screen = Screen::new(size.cols, size.rows, SCROLLBACK);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match reader.read(&mut buf).expect("a read") {
            Replay::Data(n) => {
                screen.feed(&buf[..n]);
            }
            Replay::Resize(to) => screen.resize(to.cols, to.rows),
            Replay::Truncated { .. } => panic!("trimmed while reading"),
            Replay::CaughtUp => break,
        }
    }
    let end = screen.first_row() + screen.history_rows();
    let (_, rows) = screen.size();
    let visible = screen
        .read_rows(end, end + u32::from(rows) - 1)
        .unwrap()
        .join("\n");
    format!(
        "{:?}\n{visible}\n{}",
        Some(screen.size()),
        screen
            .scrollback(Some(end.saturating_sub(1000).max(screen.first_row())))
            .text
    )
}

/// **A checkpoint that cannot be trusted is passed over** for the next one, or for the whole log.
#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_or_mislabelled_checkpoint_is_ignored_safely() {
    let root = Root::new();
    let (session, pane) = root.spawn(None);
    let live = root.provider();
    step(&live, &session, &pane, "one").await;
    resize(&live, &pane, 60, 20).await;
    step(&live, &session, &pane, "two").await;
    let watched = shown(&live, &pane);
    live.shutdown();
    drop(live);
    let good = checkpoints(&session);
    let newest = *good.last().expect("a checkpoint");
    let bytes = std::fs::read(session.path().join(format!("screen.{newest:020}"))).unwrap();
    let (_, end) = log_bounds(&session);
    let resized = kampr_keep::sizes(session.path())
        .unwrap()
        .last()
        .expect("the resize")
        .offset;

    let mut flipped = bytes.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x5a;
    for (what, offset, content, beside_a_good_one) in [
        ("garbage", end, b"garbage".to_vec(), false),
        ("a flipped byte", end, flipped, false),
        ("cut short", end, bytes[..bytes.len() / 3].to_vec(), false),
        (
            "someone else's offset",
            (resized + newest) / 2,
            bytes.clone(),
            false,
        ),
        ("past the log's end", end + 10, bytes.clone(), true),
    ] {
        for at in checkpoints(&session) {
            std::fs::remove_file(session.path().join(format!("screen.{at:020}"))).unwrap();
        }
        if beside_a_good_one {
            std::fs::write(session.path().join(format!("screen.{newest:020}")), &bytes).unwrap();
        }
        std::fs::write(session.path().join(format!("screen.{offset:020}")), &content).unwrap();
        let (node, shown) = rebuilt(&root, &session, &pane, "done two").await;
        assert_eq!(shown, watched, "{what}");
        drop(node);
    }
}
