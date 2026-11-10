use super::Hooks;
use rustix::event::{PollFd, PollFlags, Timespec};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

const MOST: u64 = 4 << 20;
const LOOK: Duration = Duration::from_millis(250);
const NOW: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "another node is answering on this socket",
        ));
    }
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(hooks: Weak<Hooks>, listener: UnixListener) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Some(hooks) = hooks.upgrade() else { return };
        tokio::spawn(connection(hooks, stream));
    }
}

async fn connection(hooks: Arc<Hooks>, mut stream: UnixStream) {
    let Ok(peer) = stream.peer_cred() else { return };
    if peer.uid() != rustix::process::getuid().as_raw() {
        return;
    }
    let mut event = Vec::new();
    if (&mut stream)
        .take(MOST + 1)
        .read_to_end(&mut event)
        .await
        .is_err()
        || event.len() as u64 > MOST
    {
        return;
    }
    let Ok(event) = serde_json::from_slice(&event) else {
        return;
    };
    let pid = peer.pid().and_then(|p| u32::try_from(p).ok());
    hooks.handle(pid, event, stream).await;
}

pub async fn hold(mut stream: UnixStream, answered: oneshot::Receiver<String>) {
    tokio::select! {
        reply = answered => {
            if let Ok(reply) = reply {
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        }
        _ = hung_up(&stream) => {}
    }
}

struct Stop(Arc<AtomicBool>);

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

// The hook half-closed its end when it sent the event, so a read only ever says EOF from here
// on; POLLHUP on the write side is what says the process holding the other end has gone. macOS
// raises it only for a poll that asks for writability (and reads the half-close itself as a
// hangup when asked for readability), and a live peer is always writable, so this samples.
async fn hung_up(stream: &UnixStream) {
    let Ok(fd) = stream.as_fd().try_clone_to_owned() else {
        return std::future::pending().await;
    };
    let stop = Arc::new(AtomicBool::new(false));
    let _stop = Stop(stop.clone());
    let watched = tokio::task::spawn_blocking(move || {
        while !stop.load(Ordering::Relaxed) {
            let mut fds = [PollFd::new(&fd, PollFlags::OUT)];
            if rustix::event::poll(&mut fds, Some(&NOW)).unwrap_or(0) > 0
                && fds[0].revents().intersects(PollFlags::HUP | PollFlags::ERR)
            {
                return true;
            }
            std::thread::sleep(LOOK);
        }
        false
    })
    .await;
    if !matches!(watched, Ok(true)) {
        std::future::pending::<()>().await;
    }
}
