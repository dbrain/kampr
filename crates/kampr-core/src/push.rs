//! "Something changed" without a timer: one signal type every wake-on-change site shares, the
//! per-pane signals a [`crate::provider::Provider`] offers, a file watch and a process's exit.
//!
//! A pulse says only that something **may** have moved: a woken reader re-reads the state it
//! cares about. Pair a wait with a deadline wherever a missed bump would leave nothing to wake
//! the reader, so a lost signal costs latency and never correctness.

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::watch;
use tokio::time::Instant;

/// The producing half: a revision that only goes up. Cheap to bump with nobody listening, and
/// clones bump the same signal.
#[derive(Clone, Debug)]
pub struct Pulse(Arc<watch::Sender<u64>>);

/// The waiting half. Remembers which bump it last saw, so a bump between two waits is not lost.
/// Once every [`Pulse`] behind it is dropped it never wakes again: waits run to their deadline.
#[derive(Clone, Debug)]
pub struct PulseRx(watch::Receiver<u64>);

impl Pulse {
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(0).0))
    }

    pub fn bump(&self) {
        self.0.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Starts having seen every bump so far.
    pub fn subscribe(&self) -> PulseRx {
        PulseRx(self.0.subscribe())
    }
}

impl Default for Pulse {
    fn default() -> Self {
        Self::new()
    }
}

impl PulseRx {
    /// Marks every bump so far as seen. Call it before reading the state, so a bump that lands
    /// during the read wakes the next wait.
    pub fn seen(&mut self) {
        self.0.borrow_and_update();
    }

    /// Whether a bump has landed since the last one seen, without waiting or marking it.
    pub fn moved(&self) -> bool {
        self.0.has_changed().unwrap_or(false)
    }

    /// Until an unseen bump, which it marks seen. Never resolves once nothing can bump it.
    pub async fn changed(&mut self) {
        if self.0.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// True on an unseen bump (at once if one is waiting), false at `deadline`.
    pub async fn changed_before(&mut self, deadline: Instant) -> bool {
        tokio::select! {
            biased;
            _ = self.changed() => true,
            _ = tokio::time::sleep_until(deadline) => false,
        }
    }
}

impl From<watch::Receiver<u64>> for PulseRx {
    fn from(rx: watch::Receiver<u64>) -> Self {
        Self(rx)
    }
}

/// What a provider can say about one pane without being polled, from
/// [`crate::provider::Provider::pane_changes`]. `None` is a signal this provider cannot give, so
/// the caller keeps whatever timer it had for that question.
#[derive(Clone, Debug, Default)]
pub struct PaneChanges {
    /// What `read_scrollback` answers may have moved: rows entered or left history, it was
    /// re-wrapped or reset, or the alternate screen came or went.
    pub history: Option<PulseRx>,
    /// The foreground job or the alternate screen may have changed.
    pub screen: Option<PulseRx>,
    /// The pane's agent status may have changed.
    pub status: Option<PulseRx>,
}

/// A running watch over some files and directories. Dropping it stops its thread at once.
pub struct FileWatch {
    pulse: Pulse,
    _stop: kampr_watch::Stop,
}

impl FileWatch {
    pub fn changes(&self) -> PulseRx {
        self.pulse.subscribe()
    }
}

/// One pulse for every path given: a directory's entries coming, going or (on Linux) being
/// written, a file being written to. A path removed or renamed away bumps once and is watched no
/// more, so a caller that expects it back watches again. Every path must exist now.
pub fn watch_files(paths: &[PathBuf]) -> io::Result<FileWatch> {
    let (mut watch, stop) = kampr_watch::Watch::new()?;
    for path in paths {
        watch.add(path)?;
    }
    let pulse = Pulse::new();
    let bumps = pulse.clone();
    std::thread::Builder::new()
        .name("kampr-file-watch".into())
        .spawn(move || {
            loop {
                match watch.wait(None) {
                    Ok(kampr_watch::Waited::Changed(events)) if !events.is_empty() => bumps.bump(),
                    Ok(kampr_watch::Waited::Changed(_) | kampr_watch::Waited::TimedOut) => {}
                    Ok(kampr_watch::Waited::Stopped) => return,
                    Err(e) => {
                        tracing::warn!(error = %e, "a file watch failed; nothing more will wake it");
                        return;
                    }
                }
            }
        })?;
    Ok(FileWatch { pulse, _stop: stop })
}

/// Resolves when process `pid` has exited, at once if it already has. The pid is bound when this
/// is called, not when it is first polled; see [`kampr_watch::Exit`] on pid reuse. Needs a
/// runtime.
pub fn exited(pid: u32) -> io::Result<impl Future<Output = ()> + Send + 'static> {
    let fd = kampr_watch::Exit::open(pid)?
        .map(|exit| AsyncFd::with_interest(exit, Interest::READABLE))
        .transpose()?;
    Ok(async move {
        if let Some(fd) = fd {
            let _ = fd.readable().await;
        }
    })
}
