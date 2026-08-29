use crate::herd::HerdModel;
use crate::pending;
use crate::sessions::Sessions;
use kampr_auth::Store;
use kampr_core::provider::AgentStatus;
use kampr_push::{Blocked, Change, Outcome, Reach, Sender, Vapid};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

/// How many changes may be queued for batching before a further one has to wait.
///
/// A notification is a *summary of now*, so a backlog of them is worthless by the time it drains —
/// bounding the queue is the honest behaviour. A change that does not fit is not recorded as
/// notified, so the next herd update re-derives it in full ([`watch_herd`]).
const QUEUE: usize = 64;

/// The node's push channel.
///
/// `None` for the VAPID key is the whole of "this node cannot push": a Tier 0 origin is not a
/// secure context, so no browser on it can even register a service worker, and advertising
/// `caps.push` there would be offering a control that fails at the last step (findings §3.7).
pub struct Push {
    vapid: Option<Arc<Vapid>>,
    sender: Option<Sender>,
    changes: Option<mpsc::Sender<Change>>,
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
            changes: None,
        }
    }

    /// `reach` is [`Reach::Public`] everywhere a node is built. It is a parameter rather than a
    /// constant because a test that runs its own push service runs it on loopback, which is the
    /// one address a real endpoint may never be.
    pub fn new(vapid: Arc<Vapid>, reach: Reach) -> Result<Self, kampr_push::SenderError> {
        Ok(Self {
            sender: Some(Sender::new(vapid.clone(), reach)?),
            vapid: Some(vapid),
            changes: None,
        })
    }

    pub fn available(&self) -> bool {
        self.vapid.is_some()
    }

    /// What a browser passes as `applicationServerKey`. Absent rather than empty when this node
    /// cannot push, so a client cannot accidentally subscribe against nothing.
    pub fn public_key(&self) -> Option<String> {
        self.vapid.as_ref().map(|v| v.public_key_b64())
    }

    fn feed(&mut self) -> mpsc::Receiver<Change> {
        let (tx, rx) = mpsc::channel(QUEUE);
        self.changes = Some(tx);
        rx
    }

    /// A test-visible hand-delivery of one change: the batching and the fan-out are the parts
    /// worth proving, and they are the same code either way.
    pub async fn deliver(&self, store: &Store, change: &Change) -> usize {
        let Some(sender) = &self.sender else {
            return 0;
        };
        let now = kampr_auth::now();
        // The cleared panes are looked up too, and for the same reason the outstanding ones are:
        // a device eligible for a pane is a device that was told about it, and that is who is owed
        // the payload that takes the prompt down. A pane that has closed is still a row in the
        // rules table, so this answers for one that is gone from the herd.
        let mut eligible: HashMap<String, Vec<kampr_auth::PushSubscription>> = HashMap::new();
        let panes = change
            .outstanding
            .iter()
            .map(|p| p.pane.clone())
            .chain(change.cleared.iter().cloned());
        for pane in panes {
            match store.push_targets(&pane, now).await {
                Ok(targets) => {
                    eligible.insert(pane, targets);
                }
                Err(e) => warn!(pane = %pane, error = %e, "could not read push targets"),
            }
        }
        let mut sent = 0;
        for (target, note) in kampr_push::per_target(change, &eligible) {
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
    pub changes: mpsc::Receiver<Change>,
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
    // Absent rather than offered and failing: a node whose sender would not build cannot push, and
    // `hello` says so, so the subscribe button is never drawn (ARCHITECTURE §3).
    let mut push = match Push::new(vapid.clone(), Reach::Public) {
        Ok(push) => push,
        Err(e) => {
            tracing::error!(error = %e, "web push is unavailable: the sender would not build");
            return (Arc::new(Push::disabled()), Vec::new());
        }
    };
    info!(key = %vapid.public_key_b64(), subject = %vapid.subject(), "web push is available");
    let changes = push.feed();
    let push = Arc::new(push);
    let ctx = PushCtx {
        push: push.clone(),
        store,
        sessions,
        herd,
        changes,
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
        mut changes,
    } = ctx;
    let watcher = tokio::spawn(watch_herd(
        herd,
        sessions,
        push.changes.clone().expect("a live push channel has a feed"),
    ));
    while let Some(change) = kampr_push::collect(&mut changes, kampr_push::WINDOW).await {
        let sent = push.deliver(&store, &change).await;
        debug!(
            outstanding = change.outstanding.len(),
            fresh = change.fresh.len(),
            cleared = change.cleared.len(),
            sent,
            "push change delivered"
        );
    }
    watcher.abort();
}

/// Turns the herd model into changes to the blocked set.
///
/// **The model is the source of truth, and the per-pane `pane.agent_status_changed` subscription
/// only makes it arrive sooner.** A missed event costs one poll interval here and nothing else,
/// which is exactly why the subscription is safe to rebuild whenever the agent-pane set moves.
///
/// **Both edges matter.** A pane that stopped being blocked is why a prompt answered at the desk
/// used to sit on a phone until somebody tapped it: the rising edge was the only thing anybody
/// sent, and one tag means the last notification stands until another replaces it.
async fn watch_herd(
    mut herd: watch::Receiver<Arc<HerdModel>>,
    sessions: Arc<Sessions>,
    out: mpsc::Sender<Change>,
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
        // A pane that stayed blocked is not a change, and rebuilding the herd every three seconds
        // is not either. Leaving early here is also what keeps `question_for` — a read against a
        // real herdr, per outstanding pane — off the poll path.
        if now == previously {
            continue;
        }
        let fresh: HashSet<String> = now.difference(&previously).cloned().collect();
        let cleared: HashSet<String> = previously.difference(&now).cloned().collect();
        let mut outstanding = Vec::with_capacity(now.len());
        for pane in model.panes.iter().filter(|p| now.contains(&p.id)) {
            outstanding.push(Blocked {
                pane: pane.id.clone(),
                node: pane.node_id.clone(),
                agent: pane.agent.clone(),
                label: pane.label.clone().or_else(|| pane.workspace.clone()),
                question: question_for(&sessions, &pane.id).await,
            });
        }
        // Bounded on purpose, and what a full queue costs is a *delay* rather than a notification:
        // `previously` is left where it was, so the next herd update re-derives the same change
        // against the same baseline and offers it again. The queued changes in front of it are the
        // older news, and they are about to drain anyway.
        match out.try_send(Change {
            outstanding,
            fresh,
            cleared,
        }) {
            Ok(()) => previously = now,
            Err(_) => debug!("push queue is full; this change waits for the next herd update"),
        }
    }
}

/// The question, read the same way the `pending` message reads it — off the screen, because
/// Claude publishes nothing about a pending request until after it is answered (probe #42).
async fn question_for(sessions: &Sessions, global: &str) -> Option<String> {
    let session = sessions.route(global)?;
    let local = session.local_pane(global)?;
    pending::read(&session.herdr, &local).await.map(|p| p.question)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use kampr_core::provider::PaneInfo;
    use kampr_core::wire::PaneEntry;
    use std::time::Duration;

    fn herd(blocked: &[&str]) -> HerdModel {
        HerdModel {
            nodes: Vec::new(),
            panes: blocked
                .iter()
                .map(|id| {
                    PaneEntry::new(
                        "01J",
                        &PaneInfo {
                            pane_id: (*id).to_string(),
                            agent: Some("claude".into()),
                            agent_status: AgentStatus::Blocked,
                            ..PaneInfo::default()
                        },
                        false,
                    )
                })
                .collect(),
        }
    }

    /// A change, or a named failure. `recv().await` on a watcher that has stopped sending the
    /// edge under test hangs forever, and a test that hangs stalls the suite instead of reporting
    /// what is missing.
    async fn next(queued: &mut mpsc::Receiver<Change>, what: &str) -> Change {
        match tokio::time::timeout(Duration::from_secs(2), queued.recv()).await {
            Ok(Some(change)) => change,
            _ => panic!("no change arrived: {what}"),
        }
    }

    fn sessions() -> Arc<Sessions> {
        let mut config = Config::bootstrap("push");
        config.herdr.socket = "/nowhere/kampr-push-test.sock".into();
        config.herdr.binary = "/nowhere/kampr-push-test-herdr".into();
        Sessions::open(&config)
    }

    /// A queue smaller than the changes arriving at it.
    ///
    /// `try_send` drops the *value being sent*, so a full queue costs the newest change — and if
    /// `previously` were advanced anyway, the panes in that change would never be offered again
    /// for as long as they stayed blocked. A blocked agent nobody is told about is the whole
    /// feature failing quietly.
    #[tokio::test]
    async fn a_change_that_did_not_fit_the_queue_is_offered_again_rather_than_recorded_as_notified() {
        let panes = ["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w1:p5"];
        let (out, mut queued) = mpsc::channel(1);
        let (herd_tx, herd_rx) = watch::channel(Arc::new(HerdModel::default()));
        let watcher = tokio::spawn(watch_herd(herd_rx, sessions(), out));

        let mut seen: HashSet<String> = HashSet::new();
        for at in 1..=panes.len() {
            herd_tx.send_replace(Arc::new(herd(&panes[..at])));
            while let Ok(Some(change)) = tokio::time::timeout(Duration::from_millis(200), queued.recv()).await
            {
                seen.extend(change.outstanding.into_iter().map(|p| p.pane));
            }
        }
        // One last update with nothing new in it: a change that was dropped is re-derived from the
        // same baseline, so the pane it named still arrives.
        herd_tx.send_replace(Arc::new(herd(&panes)));
        while let Ok(Some(change)) = tokio::time::timeout(Duration::from_millis(200), queued.recv()).await {
            seen.extend(change.outstanding.into_iter().map(|p| p.pane));
        }
        watcher.abort();

        let expected: HashSet<String> = panes.iter().map(|p| format!("01J/{p}")).collect();
        assert_eq!(
            seen, expected,
            "every pane that is still blocked is notified about"
        );
    }

    /// And the edge rule the queue fix must not undo: a pane that was already blocked is not a
    /// change, however many times the herd is rebuilt under it. This is also what keeps the
    /// per-pane `question_for` read off the three-second poll.
    #[tokio::test]
    async fn a_pane_that_was_already_blocked_is_not_notified_about_again() {
        let (out, mut queued) = mpsc::channel(16);
        let (herd_tx, herd_rx) = watch::channel(Arc::new(HerdModel::default()));
        let watcher = tokio::spawn(watch_herd(herd_rx, sessions(), out));

        let model = Arc::new(herd(&["w1:p1"]));
        for _ in 0..3 {
            herd_tx.send_replace(model.clone());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        watcher.abort();

        let mut count = 0;
        while queued.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(
            count, 1,
            "three rebuilds of one blocked pane are one notification"
        );
    }

    /// The falling edge, which nothing used to send. A pane answered anywhere — at the desk, in
    /// the TUI, on another phone — leaves the blocked set, and that is a change the devices which
    /// were told about it have to hear.
    #[tokio::test]
    async fn a_pane_that_stops_being_blocked_is_a_change_naming_it_as_cleared() {
        let (out, mut queued) = mpsc::channel(16);
        let (herd_tx, herd_rx) = watch::channel(Arc::new(HerdModel::default()));
        let watcher = tokio::spawn(watch_herd(herd_rx, sessions(), out));

        herd_tx.send_replace(Arc::new(herd(&["w1:p1", "w2:p1"])));
        let first = next(&mut queued, "two panes blocked").await;
        assert_eq!(first.fresh.len(), 2);
        assert!(first.cleared.is_empty());

        herd_tx.send_replace(Arc::new(herd(&["w2:p1"])));
        let second = next(&mut queued, "one of the two was answered").await;
        watcher.abort();

        assert!(second.fresh.is_empty(), "an answer is not news to alert about");
        assert_eq!(
            second.cleared.iter().collect::<Vec<_>>(),
            vec!["01J/w1:p1"],
            "the answered pane is named so the devices that saw it can be found"
        );
        assert_eq!(
            second
                .outstanding
                .iter()
                .map(|p| p.pane.as_str())
                .collect::<Vec<_>>(),
            vec!["01J/w2:p1"],
            "and the one still waiting stays named, or the phone loses it too"
        );
    }

    /// The last one going is the case the whole feature exists for: nothing outstanding is still
    /// a change, and it is the one that takes the prompt off the phone.
    #[tokio::test]
    async fn the_last_blocked_pane_being_answered_is_still_a_change() {
        let (out, mut queued) = mpsc::channel(16);
        let (herd_tx, herd_rx) = watch::channel(Arc::new(HerdModel::default()));
        let watcher = tokio::spawn(watch_herd(herd_rx, sessions(), out));

        herd_tx.send_replace(Arc::new(herd(&["w1:p1"])));
        next(&mut queued, "the pane blocked").await;
        herd_tx.send_replace(Arc::new(HerdModel::default()));
        let change = next(&mut queued, "the last blocked pane was answered").await;
        watcher.abort();

        assert!(change.outstanding.is_empty());
        assert_eq!(change.cleared.len(), 1);
    }
}
