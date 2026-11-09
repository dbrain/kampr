use super::PeerStdio;
use crate::state::Node;
use anyhow::{Context, Result, anyhow, bail};
use kampr_auth::{Entry, MeshRole, NodeIdentity};
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::{Notify, watch};
use tracing::{debug, info, warn};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const FIRST_RETRY: Duration = Duration::from_millis(250);
const LAST_RETRY: Duration = Duration::from_secs(30);
/// A link that stood this long was a success, and the next failure starts the backoff again.
const SETTLED: Duration = Duration::from_secs(10);
/// Spawns in a row that ended in an error before a child is reported failing; one is a container
/// stopping under it.
const FAILING: u32 = 3;

/// What to run and who it is. The caller keeps `key` (it is created there if missing) and
/// `node_id`, so a respawned child is the same node to everything that has met it.
#[derive(Debug, Clone)]
pub struct ChildSpec {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub node_id: String,
    pub node_name: String,
    pub key: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildState {
    Starting,
    Up,
    Down,
}

/// A child node's id under its host's: `<host>-c-<tag>`, cut to what the handshake accepts.
pub fn child_node_id(host: &str, tag: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect()
    };
    let tag = clean(tag);
    let suffix = format!("-c-{}", &tag[..tag.len().min(40)]);
    let host = clean(host);
    format!("{}{suffix}", &host[..host.len().min(64 - suffix.len())])
}

/// The running child and its supervisor. Dropping it stops the child, as [`Child::stop`] does
/// without waiting.
pub struct Child {
    state: watch::Receiver<ChildState>,
    failure: watch::Receiver<Option<String>>,
    pid: Arc<AtomicU32>,
    stop: watch::Sender<bool>,
    kick: Arc<Notify>,
    pubkey: String,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Child {
    pub fn state(&self) -> ChildState {
        *self.state.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<ChildState> {
        self.state.clone()
    }

    /// Why the last attempts to bring it up failed, until it is up again.
    pub fn failures(&self) -> watch::Receiver<Option<String>> {
        self.failure.clone()
    }

    pub fn pid(&self) -> Option<u32> {
        Some(self.pid.load(Ordering::Relaxed)).filter(|pid| *pid != 0)
    }

    /// Asks a supervisor that is waiting out a backoff, or waiting for its predicate, to look now.
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    /// Ends the command and the link, and keeps the child's enrolment for its next spawn.
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// [`Self::stop`], and the child's key is revoked here: nothing holding it is served again
    /// until a spawn enrols it afresh.
    pub async fn release(self, node: &Node) -> Result<()> {
        let pubkey = self.pubkey.clone();
        self.stop().await;
        node.auth
            .store()
            .mesh()
            .revoke(&pubkey, kampr_auth::now())
            .await?;
        node.peers.disconnect(&pubkey);
        Ok(())
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// Runs the command once: up while the link stands, down for good when it ends.
pub fn spawn(node: &Arc<Node>, spec: ChildSpec) -> Result<Child> {
    start(
        node,
        spec,
        None::<fn() -> std::future::Ready<Result<bool, String>>>,
    )
}

/// Runs the command again whenever it ends, with backoff, for as long as `should_run` says so.
/// Asked before every spawn; `Ok(false)` waits for [`Child::kick`] or the longest backoff, and an
/// error waits the same way and is the child's failure until it is up.
pub fn supervise<F, Fut>(node: &Arc<Node>, spec: ChildSpec, should_run: F) -> Result<Child>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<bool, String>> + Send + 'static,
{
    start(node, spec, Some(should_run))
}

fn start<F, Fut>(node: &Arc<Node>, spec: ChildSpec, should_run: Option<F>) -> Result<Child>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<bool, String>> + Send + 'static,
{
    if spec.argv.is_empty() {
        bail!("a child needs a command to run");
    }
    if !kampr_mesh::valid_node_id(&spec.node_id) {
        bail!("{:?} is not a node id the handshake accepts", spec.node_id);
    }
    let identity = NodeIdentity::load_or_create(&spec.key)
        .with_context(|| format!("the child key at {}", spec.key.display()))?;
    let (state_tx, state) = watch::channel(ChildState::Starting);
    let (failure_tx, failure) = watch::channel(None);
    let (stop, stopping) = watch::channel(false);
    let pid = Arc::new(AtomicU32::new(0));
    let kick = Arc::new(Notify::new());
    let run = Run {
        node: Arc::downgrade(node),
        spec,
        identity: identity.clone(),
        state: state_tx,
        failure: failure_tx,
        pid: pid.clone(),
        stopping,
    };
    let task = tokio::spawn(run.supervised(should_run, kick.clone()));
    Ok(Child {
        state,
        failure,
        pid,
        stop,
        kick,
        pubkey: identity.public_hex(),
        task: Some(task),
    })
}

struct Run {
    node: Weak<Node>,
    spec: ChildSpec,
    identity: NodeIdentity,
    state: watch::Sender<ChildState>,
    failure: watch::Sender<Option<String>>,
    pid: Arc<AtomicU32>,
    stopping: watch::Receiver<bool>,
}

impl Run {
    async fn supervised<F, Fut>(mut self, should_run: Option<F>, kick: Arc<Notify>)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool, String>> + Send + 'static,
    {
        let mut wait = FIRST_RETRY;
        let mut failed = 0;
        loop {
            if self.stopped() {
                break;
            }
            let wanted = match &should_run {
                Some(should_run) => should_run().await,
                None => Ok(true),
            };
            let wanted = match wanted {
                Ok(true) => true,
                Ok(false) => {
                    failed = 0;
                    self.fail(None);
                    false
                }
                Err(why) => {
                    self.fail(Some(why));
                    false
                }
            };
            if wanted {
                let began = tokio::time::Instant::now();
                match self.once().await {
                    Ok(()) => failed = 0,
                    Err(e) => {
                        let why = format!("{e:#}");
                        warn!(child = %self.spec.node_name, error = %why, "a child node ended");
                        failed += 1;
                        if failed >= FAILING {
                            self.fail(Some(why));
                        }
                    }
                }
                if began.elapsed() >= SETTLED {
                    wait = FIRST_RETRY;
                }
            }
            self.state.send_replace(ChildState::Down);
            if should_run.is_none() || self.stopped() {
                break;
            }
            let pause = if wanted { wait } else { LAST_RETRY };
            wait = (wait * 2).min(LAST_RETRY);
            tokio::select! {
                () = tokio::time::sleep(pause) => {}
                () = kick.notified() => {}
                () = asked_to_stop(&mut self.stopping) => break,
            }
        }
        self.state.send_replace(ChildState::Down);
    }

    fn stopped(&self) -> bool {
        *self.stopping.borrow()
    }

    fn fail(&self, why: Option<String>) {
        self.failure.send_if_modified(|was| {
            let changed = *was != why;
            *was = why;
            changed
        });
    }

    async fn once(&mut self) -> Result<()> {
        let node = self
            .node
            .upgrade()
            .ok_or_else(|| anyhow!("the host node has stopped"))?;
        let pubkey = self.identity.public_hex();
        node.auth
            .store()
            .mesh()
            .enrol(
                &pubkey,
                &self.spec.node_id,
                &self.spec.node_name,
                MeshRole::Peer,
                None,
                kampr_auth::now(),
            )
            .await?;
        let secret = std::fs::read_to_string(&self.spec.key)
            .with_context(|| format!("reading {}", self.spec.key.display()))?;
        let carried = PeerStdio {
            node_id: self.spec.node_id.clone(),
            node_name: self.spec.node_name.clone(),
            key: secret.trim().to_string(),
            hub_key: node.identity()?.public_hex(),
        };
        self.state.send_replace(ChildState::Starting);
        let mut child = Command::new(&self.spec.argv[0])
            .args(&self.spec.argv[1..])
            .envs(self.spec.env.iter().map(|(k, v)| (k, v)))
            .env(PeerStdio::ENV, carried.encode())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", self.spec.argv[0]))?;
        self.pid.store(child.id().unwrap_or(0), Ordering::Relaxed);
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            bail!("the child's pipes were not handed over");
        };
        let name = self.spec.node_name.clone();
        let _log = crate::mesh::AbortOnDrop(
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(child = %name, "{line}");
                }
            })
            .abort_handle(),
        );
        let ended = self.serve(&node, &pubkey, stdin, stdout).await;
        node.peers.disconnect(&pubkey);
        let _ = child.start_kill();
        let _ = child.wait().await;
        self.pid.store(0, Ordering::Relaxed);
        ended
    }

    async fn serve(
        &mut self,
        node: &Arc<Node>,
        pubkey: &str,
        stdin: tokio::process::ChildStdin,
        stdout: tokio::process::ChildStdout,
    ) -> Result<()> {
        let mut link = kampr_mesh::stdio::link(stdout, stdin);
        let (identity, presence) = (node.identity()?, node.presence());
        let accepted = tokio::select! {
            accepted = tokio::time::timeout(
                HANDSHAKE_TIMEOUT,
                kampr_mesh::accept(&mut link, &identity, &presence, node.auth.store(), kampr_auth::now()),
            ) => accepted.map_err(|_| anyhow!("the child never finished the handshake"))??,
            () = asked_to_stop(&mut self.stopping) => return Ok(()),
        };
        // Any key enrolled here would pass `accept`; this link is for the one the host minted.
        if accepted.node.pubkey != pubkey {
            bail!("the child answered with a key that is not the one it was given");
        }
        node.auth.audit().record(
            &Entry::new("mesh.joined").peer("stdio").detail(serde_json::json!({
                "node": accepted.node.node_id,
                "name": accepted.node.name,
                "fingerprint": accepted.node.fingerprint(),
                "adopted": true,
            })),
        );
        info!(child = %self.spec.node_name, node = %self.spec.node_id, "a child node is up");
        self.state.send_replace(ChildState::Up);
        self.fail(None);
        let (out, incoming) = link.split();
        let peers = node.peers.clone();
        let mut serving = tokio::spawn(async move { peers.serve_adopted(accepted, out, incoming).await });
        tokio::select! {
            _ = &mut serving => {}
            () = asked_to_stop(&mut self.stopping) => {
                node.peers.disconnect(pubkey);
                let _ = serving.await;
            }
        }
        Ok(())
    }
}

async fn asked_to_stop(stopping: &mut watch::Receiver<bool>) {
    let _ = stopping.wait_for(|stop| *stop).await;
}

#[cfg(test)]
mod tests {
    use super::child_node_id;

    #[test]
    fn a_child_node_id_is_always_one_the_handshake_accepts() {
        assert_eq!(child_node_id("01JHOST", "0123456789ab"), "01JHOST-c-0123456789ab");
        assert_eq!(child_node_id("01JHOST", "web/1 x"), "01JHOST-c-web1x");
        let long = child_node_id(&"h".repeat(80), &"t".repeat(80));
        assert!(long.len() <= 64 && kampr_mesh::valid_node_id(&long), "{long}");
        assert!(long.ends_with(&format!("-c-{}", "t".repeat(40))), "{long}");
    }
}
