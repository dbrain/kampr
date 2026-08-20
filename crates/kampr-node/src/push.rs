use crate::herd::HerdModel;
use crate::pending;
use crate::sessions::Sessions;
use kampr_auth::Store;
use kampr_core::provider::AgentStatus;
use kampr_push::{Blocked, Outcome, Sender, Vapid};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

/// How many blocked panes may be queued for batching before the oldest are dropped.
///
/// A notification is a *summary of now*, so a backlog of them is worthless by the time it drains —
/// bounding the queue is the honest behaviour, and the herd poll behind it means the next status
/// change produces a fresh one anyway.
const QUEUE: usize = 64;

/// The node's push channel.
///
/// `None` for the VAPID key is the whole of "this node cannot push": a Tier 0 origin is not a
/// secure context, so no browser on it can even register a service worker, and advertising
/// `caps.push` there would be offering a control that fails at the last step (findings §3.7).
pub struct Push {
    vapid: Option<Arc<Vapid>>,
    sender: Option<Sender>,
    blocked: Option<mpsc::Sender<Blocked>>,
}

impl std::fmt::Debug for Push {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push")
            .field("available", &self.available())
            .finish()
    }
}

impl Push {
    pub fn disabled() -> Self {
        Self {
            vapid: None,
            sender: None,
            blocked: None,
        }
    }

    pub fn new(vapid: Arc<Vapid>) -> Self {
        Self {
            sender: Some(Sender::new(vapid.clone())),
            vapid: Some(vapid),
            blocked: None,
        }
    }

    pub fn available(&self) -> bool {
        self.vapid.is_some()
    }

    /// What a browser passes as `applicationServerKey`. Absent rather than empty when this node
    /// cannot push, so a client cannot accidentally subscribe against nothing.
    pub fn public_key(&self) -> Option<String> {
        self.vapid.as_ref().map(|v| v.public_key_b64())
    }

    fn feed(&mut self) -> mpsc::Receiver<Blocked> {
        let (tx, rx) = mpsc::channel(QUEUE);
        self.blocked = Some(tx);
        rx
    }

    /// A test-visible hand-delivery of one batch: the batching and the fan-out are the parts
    /// worth proving, and they are the same code either way.
    pub async fn deliver(&self, store: &Store, panes: Vec<Blocked>) -> usize {
        let Some(sender) = &self.sender else {
            return 0;
        };
        let now = kampr_auth::now();
        let mut eligible: HashMap<String, Vec<kampr_auth::PushSubscription>> = HashMap::new();
        for pane in &panes {
            match store.push_targets(&pane.pane, now).await {
                Ok(targets) => {
                    eligible.insert(pane.pane.clone(), targets);
                }
                Err(e) => warn!(pane = %pane.pane, error = %e, "could not read push targets"),
            }
        }
        let mut sent = 0;
        for (target, note) in kampr_push::per_target(&panes, &eligible) {
            match sender.send(&target, &note).await {
                Outcome::Delivered => {
                    sent += 1;
                    let _ = store.mark_push_sent(&target.id, now).await;
                }
                Outcome::Gone => {
                    let _ = store.forget_push_endpoint(&target.endpoint).await;
                }
                Outcome::Failed => {}
            }
        }
        sent
    }
}

pub struct PushCtx {
    pub push: Arc<Push>,
    pub store: Store,
    pub sessions: Arc<Sessions>,
    pub herd: watch::Receiver<Arc<HerdModel>>,
    pub blocked: mpsc::Receiver<Blocked>,
}

/// Starts the push channel, returning it and the two tasks that drive it.
///
/// The watcher is **one per node**, not one per connected client: three phones watching the same
/// herd is still one agent going blocked, and a per-session watcher would send three.
pub fn start(
    vapid: Option<Arc<Vapid>>,
    store: Store,
    sessions: Arc<Sessions>,
    herd: watch::Receiver<Arc<HerdModel>>,
) -> (Arc<Push>, Vec<tokio::task::JoinHandle<()>>) {
    let Some(vapid) = vapid else {
        return (Arc::new(Push::disabled()), Vec::new());
    };
    info!(key = %vapid.public_key_b64(), subject = %vapid.subject(), "web push is available");
    let mut push = Push::new(vapid);
    let blocked = push.feed();
    let push = Arc::new(push);
    let ctx = PushCtx {
        push: push.clone(),
        store,
        sessions,
        herd,
        blocked,
    };
    let tasks = vec![tokio::spawn(run(ctx))];
    (push, tasks)
}

async fn run(ctx: PushCtx) {
    let PushCtx {
        push,
        store,
        sessions,
        herd,
        mut blocked,
    } = ctx;
    let watcher = tokio::spawn(watch_herd(
        herd,
        sessions,
        push.blocked.clone().expect("a live push channel has a feed"),
    ));
    while let Some(batch) = kampr_push::collect(&mut blocked, kampr_push::WINDOW).await {
        let count = batch.len();
        let sent = push.deliver(&store, batch).await;
        debug!(panes = count, sent, "push batch delivered");
    }
    watcher.abort();
}

/// Turns the herd model into blocked-pane events.
///
/// **The model is the source of truth, and the per-pane `pane.agent_status_changed` subscription
/// only makes it arrive sooner.** A missed event costs one poll interval here and nothing else,
/// which is exactly why the subscription is safe to rebuild whenever the agent-pane set moves.
async fn watch_herd(
    mut herd: watch::Receiver<Arc<HerdModel>>,
    sessions: Arc<Sessions>,
    out: mpsc::Sender<Blocked>,
) {
    let mut previously: HashSet<String> = HashSet::new();
    loop {
        if herd.changed().await.is_err() {
            return;
        }
        let model = herd.borrow_and_update().clone();
        let now: HashSet<String> = model
            .panes
            .iter()
            .filter(|p| p.agent_status == AgentStatus::Blocked)
            .map(|p| p.id.clone())
            .collect();
        // Only the edge. A pane that was already blocked when this loop last looked has already
        // been notified about, and re-sending on every poll is how a phone gets muted.
        for id in now.difference(&previously) {
            let Some(pane) = model.pane(id) else { continue };
            let blocked = Blocked {
                pane: pane.id.clone(),
                node: pane.node_id.clone(),
                agent: pane.agent.clone(),
                label: pane.label.clone().or_else(|| pane.workspace.clone()),
                question: question_for(&sessions, &pane.id).await,
            };
            // Bounded on purpose: a notification is a summary of now, so dropping the oldest is
            // better than delivering a backlog that is already wrong.
            if out.try_send(blocked).is_err() {
                debug!(pane = %id, "push queue is full; dropping the oldest block");
            }
        }
        previously = now;
    }
}

/// The question, read the same way the `pending` message reads it — off the screen, because
/// Claude publishes nothing about a pending request until after it is answered (probe #42).
async fn question_for(sessions: &Sessions, global: &str) -> Option<String> {
    let session = sessions.route(global)?;
    let local = session.local_pane(global)?;
    pending::read(&session.herdr, &local).await.map(|p| p.question)
}
