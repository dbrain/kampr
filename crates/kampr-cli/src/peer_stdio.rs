//! `kampr serve --peer-stdio`: a node whose stdin and stdout are its one mesh link, to the host
//! that spawned it (docs/19 "Adoption"). It binds nothing and logs to stderr.

use crate::dirs::Dirs;
use anyhow::{Context, Result};
use kampr_node::Node;
use kampr_node::child::{PeerStdio, prepare, serve_parent};
use std::os::fd::{FromRawFd, OwnedFd};

/// Before the runtime: the variable carries the child's secret key and is taken out of the
/// environment while this process is still one thread, so no session started here inherits it.
pub fn run(dirs: &Dirs) -> Result<()> {
    let spec = PeerStdio::from_env();
    unsafe { std::env::remove_var(PeerStdio::ENV) };
    let spec = spec?;
    let link = take_stdout()?;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("KAMPR_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let runtime = tokio::runtime::Runtime::new()?;
    let served = runtime.block_on(async {
        let (config, state) = prepare(&dirs.config(), dirs.state_override(), &spec)?;
        let node = Node::start(config, &state).await?;
        let writer = tokio::fs::File::from_std(std::fs::File::from(link));
        serve_parent(node, &spec.hub_key, tokio::io::stdin(), writer).await
    });
    if let Err(e) = &served {
        tracing::error!(error = %format!("{e:#}"), "the link to the host ended");
    }
    // The blocking read of stdin cannot be cancelled, and dropping the runtime would wait on it.
    std::process::exit(i32::from(served.is_err()))
}

/// The link keeps stdout to itself: anything else that writes to fd 1 lands on stderr instead of
/// in the middle of a mesh message.
fn take_stdout() -> Result<OwnedFd> {
    let link = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if link < 0 {
        return Err(std::io::Error::last_os_error()).context("keeping stdout for the link");
    }
    if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error()).context("pointing stdout at stderr");
    }
    Ok(unsafe { OwnedFd::from_raw_fd(link) })
}
