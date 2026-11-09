//! Running a Kampr node inside a container on this machine (docs/19 "Adoption").

mod binary;
mod inject;
mod ledger;
mod listed;
mod release;

pub use binary::Binaries;
pub use ledger::{Ledger, Record};
pub use listed::annotate;
pub use release::Releases;

use crate::child::{self, Child, ChildSpec, ChildState};
use crate::manage::ManageError;
use crate::mesh::AbortOnDrop;
use crate::state::{BUILD, Node};
use kampr_auth::NodeIdentity;
use kampr_docker::{Client, Directory, is_not_found};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// A copy into the container, its node starting and its first herd crossing the link.
const LISTED: Duration = Duration::from_secs(60);
/// How long a remembered container may be missing before its adoption is failing: long enough
/// for `docker compose up` to recreate it.
const GONE: Duration = Duration::from_secs(20);

struct Adopted {
    child: Child,
    _failures: AbortOnDrop,
}

#[derive(Default)]
pub struct Children {
    running: Mutex<BTreeMap<String, Adopted>>,
    binaries: OnceLock<Arc<Binaries>>,
}

impl Children {
    fn kick_stopped(&self, docker: &Directory) {
        for (name, adopted) in self.running.lock().unwrap().iter() {
            let running = docker.find(name).is_some_and(|c| c.state.running);
            if running && adopted.child.state() == ChildState::Down {
                adopted.child.kick();
            }
        }
    }

    fn take(&self, name: &str) -> Option<Child> {
        self.running.lock().unwrap().remove(name).map(|a| a.child)
    }

    pub fn clear(&self) {
        self.running.lock().unwrap().clear();
    }
}

fn binaries(node: &Node) -> Arc<Binaries> {
    let docker = &node.config.docker;
    node.children
        .binaries
        .get_or_init(|| {
            Arc::new(Binaries {
                configured: docker.configured(),
                own: std::env::current_exe().ok(),
                fetch: docker.fetches(),
                releases: Releases::new(
                    &node.state_dir,
                    BUILD,
                    &node.config.update.repo,
                    docker.release_base.clone(),
                ),
            })
        })
        .clone()
}

fn bad(e: anyhow::Error) -> ManageError {
    ManageError::BadRequest(format!("{e:#}"))
}

fn host(node: &Node) -> Result<(Arc<Directory>, Arc<Ledger>), ManageError> {
    let primary = node.primary();
    match (&primary.docker, &primary.adopted) {
        (Some(docker), Some(ledger)) => Ok((docker.clone(), ledger.clone())),
        _ => Err(ManageError::BadRequest(
            "this node does not follow a docker daemon".into(),
        )),
    }
}

/// The child's node id once the herd lists it, or once [`LISTED`] has passed.
pub async fn adopt(node: &Arc<Node>, id: &str) -> Result<String, ManageError> {
    let (docker, ledger) = host(node)?;
    let client = docker.client();
    let inspect = client.inspect(id).await.map_err(crate::docker::failed(id))?;
    let name = inspect.short_name().to_string();
    if !inspect.state.running {
        return Err(ManageError::BadRequest(format!("{name} is not running")));
    }
    inject::prepare(client, &binaries(node), &inspect)
        .await
        .map_err(ManageError::BadRequest)?;
    let record = ledger.get(&name).unwrap_or_else(|| Record {
        node_id: child::child_node_id(node.node_id(), &inspect.id[..inspect.id.len().min(12)]),
        name: name.clone(),
    });
    ledger.remember(record.clone()).map_err(bad)?;
    supervise(node, &docker, &ledger, &record).map_err(bad)?;
    info!(container = %name, node = %record.node_id, "adopted a container");
    listed(node, &record.node_id).await;
    Ok(record.node_id)
}

/// Stops the child, revokes its key and forgets the container. Whatever runs inside it stays.
pub async fn release(node: &Arc<Node>, id: &str) -> Result<(), ManageError> {
    let (docker, ledger) = host(node)?;
    let name = docker
        .find(id)
        .map(|c| c.short_name().to_string())
        .unwrap_or_else(|| id.to_string());
    if ledger.get(&name).is_none() {
        return Err(ManageError::BadRequest(format!("{name} is not adopted")));
    }
    let key = ledger.key(&name);
    let pubkey = NodeIdentity::load_or_create(&key)
        .map_err(|e| bad(e.into()))?
        .public_hex();
    match node.children.take(&name) {
        Some(child) => child.release(node).await.map_err(bad)?,
        None => {
            node.auth
                .store()
                .mesh()
                .revoke(&pubkey, kampr_auth::now())
                .await
                .map_err(|e| bad(e.into()))?;
        }
    }
    ledger.forget(&name).map_err(bad)?;
    if let Err(e) = std::fs::remove_file(&key) {
        warn!(key = %key.display(), error = %e, "could not delete a released container's key");
    }
    node.peers.forget(&pubkey);
    info!(container = %name, "released a container");
    Ok(())
}

/// Every remembered adoption supervised again, and every child whose container starts looked
/// at now rather than at the end of its backoff.
pub fn follow(node: &Arc<Node>) -> Option<tokio::task::JoinHandle<()>> {
    let (docker, ledger) = host(node).ok()?;
    for record in ledger.records() {
        if let Err(e) = supervise(node, &docker, &ledger, &record) {
            let why = format!("could not resume adopting {}: {e:#}", record.name);
            warn!(container = %record.name, error = %why, "could not resume an adoption");
            ledger.fail(&record.name, Some(why));
        }
    }
    let node = Arc::downgrade(node);
    Some(tokio::spawn(kick_on_change(node, docker)))
}

async fn kick_on_change(node: Weak<Node>, docker: Arc<Directory>) {
    let mut changes = docker.subscribe();
    while changes.changed().await.is_ok() {
        let Some(node) = node.upgrade() else {
            return;
        };
        node.children.kick_stopped(&docker);
    }
}

fn supervise(
    node: &Arc<Node>,
    docker: &Directory,
    ledger: &Arc<Ledger>,
    record: &Record,
) -> anyhow::Result<()> {
    let mut children = node.children.running.lock().unwrap();
    if let Some(adopted) = children.get(&record.name) {
        adopted.child.kick();
        return Ok(());
    }
    let client = docker.client().clone();
    let spec = ChildSpec {
        argv: exec(&record.name),
        env: vec![(
            "DOCKER_HOST".into(),
            format!("unix://{}", client.socket().display()),
        )],
        node_id: record.node_id.clone(),
        node_name: record.name.clone(),
        key: ledger.key(&record.name),
    };
    let ready = Arc::new(Ready {
        client,
        name: record.name.clone(),
        binaries: binaries(node),
        missing_since: Mutex::new(None),
    });
    let child = child::supervise(node, spec, move || {
        let ready = ready.clone();
        async move { ready.check().await }
    })?;
    let failures = tokio::spawn(report(child.failures(), ledger.clone(), record.name.clone()));
    children.insert(
        record.name.clone(),
        Adopted {
            child,
            _failures: AbortOnDrop(failures.abort_handle()),
        },
    );
    Ok(())
}

async fn report(
    mut failures: tokio::sync::watch::Receiver<Option<String>>,
    ledger: Arc<Ledger>,
    name: String,
) {
    loop {
        let why = failures.borrow_and_update().clone();
        ledger.fail(&name, why);
        if failures.changed().await.is_err() {
            return;
        }
    }
}

/// By name, so a container recreated under it is the one reached; the key rides in the
/// environment `docker exec -e NAME` copies from, never in an argv.
fn exec(name: &str) -> Vec<String> {
    [
        "docker",
        "exec",
        "-i",
        "-e",
        child::PeerStdio::ENV,
        name,
        inject::BINARY,
        "serve",
        "--peer-stdio",
        "--config-dir",
        "/.kampr/config",
        "--state-dir",
        "/.kampr/state",
    ]
    .map(String::from)
    .to_vec()
}

struct Ready {
    client: Client,
    name: String,
    binaries: Arc<Binaries>,
    missing_since: Mutex<Option<Instant>>,
}

impl Ready {
    /// Asked before every spawn, so a recreated container or an upgraded host is copied to first.
    /// A daemon that does not answer is not this adoption failing: the node says docker is down.
    async fn check(&self) -> Result<bool, String> {
        let inspect = match self.client.inspect(&self.name).await {
            Ok(inspect) => inspect,
            Err(e) if is_not_found(&e) => return self.missing(),
            Err(_) => return Ok(false),
        };
        *self.missing_since.lock().unwrap() = None;
        if !inspect.state.running {
            return Ok(false);
        }
        match inject::prepare(&self.client, &self.binaries, &inspect).await {
            Ok(copied) => {
                if copied {
                    info!(container = %self.name, "copied kampr into a container");
                }
                Ok(true)
            }
            Err(why) => {
                warn!(container = %self.name, error = %why, "could not copy kampr into a container");
                Err(why)
            }
        }
    }

    fn missing(&self) -> Result<bool, String> {
        let since = *self
            .missing_since
            .lock()
            .unwrap()
            .get_or_insert_with(Instant::now);
        match since.elapsed() >= GONE {
            true => Err(format!(
                "{} no longer exists; release it to forget the adoption, or create it again to \
                 have it adopted",
                self.name
            )),
            false => Ok(false),
        }
    }
}

async fn listed(node: &Node, id: &str) {
    let mut herd = node.subscribe_herd();
    let deadline = tokio::time::Instant::now() + LISTED;
    loop {
        if herd
            .borrow_and_update()
            .node(id)
            .is_some_and(|n| n.is_reachable())
        {
            return;
        }
        if tokio::time::timeout_at(deadline, herd.changed()).await.is_err() {
            warn!(node = %id, "an adopted container's node is not in the herd yet");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_exec_names_the_container_and_carries_the_key_by_name_alone() {
        let argv = super::exec("db-1");
        assert_eq!(
            argv[..6],
            ["docker", "exec", "-i", "-e", "KAMPR_PEER_STDIO", "db-1"]
        );
        assert!(argv.iter().all(|a| !a.contains('=')), "{argv:?}");
        assert!(argv.ends_with(&["--state-dir".into(), "/.kampr/state".into()]));
    }
}
