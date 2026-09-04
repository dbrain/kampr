use crate::herd::HerdModel;
use crate::pending;
use crate::sessions::Sessions;
use kampr_auth::Store;
use kampr_core::provider::AgentStatus;
use kampr_core::wire::PaneEntry;
use kampr_push::{Agent, Change, Kind, Outcome, Reach, Sender, Vapid};
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
/// One queue per notification kind.
///
/// **Not one queue carrying both.** The collection window folds everything that lands inside it
/// into a single change, and folding a `done` into a `blocked` would produce a payload naming
/// panes of both kinds under one of the two tags — which is exactly the "silently unsays the
/// rest" failure the set-not-edge rule exists to prevent, arriving from a new direction.
#[derive(Clone)]
struct Feeds {
    blocked: mpsc::Sender<Change>,
    done: mpsc::Sender<Change>,
}

impl Feeds {
    fn of(&self, kind: Kind) -> &mpsc::Sender<Change> {
        match kind {
            Kind::Blocked => &self.blocked,
            Kind::Done => &self.done,
        }
    }
}

pub struct Push {
    vapid: Option<Arc<Vapid>>,
    sender: Option<Sender>,
    changes: Option<Feeds>,
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

    fn feed(&mut self) -> (mpsc::Receiver<Change>, mpsc::Receiver<Change>) {
        let (blocked, blocked_rx) = mpsc::channel(QUEUE);
        let (done, done_rx) = mpsc::channel(QUEUE);
        self.changes = Some(Feeds { blocked, done });
        (blocked_rx, done_rx)
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
        let needs = change.kind.min_payload_version();
        for pane in panes {
            match store.push_targets(&pane, now, needs).await {
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
    pub blocked: mpsc::Receiver<Change>,
    pub done: mpsc::Receiver<Change>,
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
    let (blocked, done) = push.feed();
    let push = Arc::new(push);
    let ctx = PushCtx {
        push: push.clone(),
        store,
        sessions,
        herd,
        blocked,
        done,
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
        blocked,
        done,
    } = ctx;
    let feeds = push.changes.clone().expect("a live push channel has a feed");
    let watcher = tokio::spawn(watch_herd(herd, sessions, feeds));
    // Two windows, not one: a `done` landing beside a `blocked` must not hold the question back,
    // and neither may be folded into the other's payload.
    let quiet = tokio::spawn(pipeline(push.clone(), store.clone(), done));
    pipeline(push, store, blocked).await;
    quiet.abort();
    watcher.abort();
}

async fn pipeline(push: Arc<Push>, store: Store, mut changes: mpsc::Receiver<Change>) {
    while let Some(change) = kampr_push::collect(&mut changes, kampr_push::WINDOW).await {
        let sent = push.deliver(&store, &change).await;
        debug!(
            kind = ?change.kind,
            outstanding = change.outstanding.len(),
            fresh = change.fresh.len(),
            cleared = change.cleared.len(),
            sent,
            "push change delivered"
        );
    }
}

/// Turns the herd model into changes to each notification kind's set.
///
/// **The model is the source of truth, and the per-pane `pane.agent_status_changed` subscription
/// only makes it arrive sooner.** A missed event costs one poll interval here and nothing else,
/// which is exactly why the subscription is safe to rebuild whenever the agent-pane set moves.
///
/// **Both edges matter.** A pane that left a set is why a prompt answered at the desk used to sit
/// on a phone until somebody tapped it: the rising edge was the only thing anybody sent, and one
/// tag per kind means the last notification stands until another replaces it. A `done` falls the
/// same way — the operator focusing the pane at the desk is what destroys herdr's marker (#357,
/// #396), and the phone has to be told.
///
/// **One pass over the herd, two sets, two baselines.** They are tracked separately so a change
/// one kind could not queue never advances the other's baseline, and so a herd rebuild that moved
/// only one of them wakes only the devices that kind reaches.
async fn watch_herd(mut herd: watch::Receiver<Arc<HerdModel>>, sessions: Arc<Sessions>, out: Feeds) {
    let mut previously: HashMap<Kind, HashSet<String>> = HashMap::new();
    loop {
        if herd.changed().await.is_err() {
            return;
        }
        let model = herd.borrow_and_update().clone();
        for kind in [Kind::Blocked, Kind::Done] {
            let seen = previously.entry(kind).or_default();
            let now: HashSet<String> = model
                .panes
                .iter()
                .filter(|p| p.agent_status == status_for(kind))
                .map(|p| p.id.clone())
                .collect();
            // A pane that stayed in the set is not a change, and rebuilding the herd every three
            // seconds is not either. Leaving early here is also what keeps `question_for` — a read
            // against a real herdr, per outstanding pane — off the poll path.
            if now == *seen {
                continue;
            }
            let fresh: HashSet<String> = now.difference(seen).cloned().collect();
            let cleared: HashSet<String> = seen.difference(&now).cloned().collect();
            let mut outstanding = Vec::with_capacity(now.len());
            for pane in model.panes.iter().filter(|p| now.contains(&p.id)) {
                outstanding.push(Agent {
                    pane: pane.id.clone(),
                    node: pane.node_id.clone(),
                    agent: pane.agent.clone(),
                    label: pane.label.clone().or_else(|| pane.workspace.clone()),
                    detail: detail_for(kind, pane, &sessions).await,
                });
            }
            // Bounded on purpose, and what a full queue costs is a *delay* rather than a
            // notification: this kind's baseline is left where it was, so the next herd update
            // re-derives the same change against the same baseline and offers it again. The
            // queued changes in front of it are the older news, and they are about to drain
            // anyway.
            match out.of(kind).try_send(Change {
                kind,
                outstanding,
                fresh,
                cleared,
            }) {
                Ok(()) => *seen = now,
                Err(_) => debug!(
                    ?kind,
                    "push queue is full; this change waits for the next herd update"
                ),
            }
        }
    }
}

fn status_for(kind: Kind) -> AgentStatus {
    match kind {
        Kind::Blocked => AgentStatus::Blocked,
        Kind::Done => AgentStatus::Done,
    }
}

/// The one line under an agent's name, which is a different fact for each kind.
///
/// **The finished case reads nothing.** Its honest analogue of the question would be the agent's
/// closing message, and resolving that means locating and parsing the transcript — 1.99 s on a
/// 30.7 MB one (#409), inside a 900 ms window, for every pane that finished at once. The working
/// directory is already in the herd model, tells three simultaneous agents apart, and is not a
/// guess about what the agent did.
async fn detail_for(kind: Kind, pane: &PaneEntry, sessions: &Sessions) -> Option<String> {
    match kind {
        Kind::Blocked => question_for(sessions, &pane.id, pane.agent.as_deref()).await,
        Kind::Done => pane.cwd.as_deref().map(|cwd| home_relative(cwd, home())),
    }
}

fn home() -> Option<&'static str> {
    static HOME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    HOME.get_or_init(|| std::env::var("HOME").ok())
        .as_deref()
        .filter(|home| !home.is_empty())
}

/// `~/dev/kampr` rather than `/home/dbrain/dev/kampr`: a lock screen has one line for this, and
/// the prefix every path shares is the part that carries no information.
fn home_relative(path: &str, home: Option<&str>) -> String {
    match home.and_then(|home| path.strip_prefix(home)) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// The question, read the same way the `pending` message reads it — off the screen, because
/// Claude publishes nothing about a pending request until after it is answered (probe #42).
async fn question_for(sessions: &Sessions, global: &str, agent: Option<&str>) -> Option<String> {
    let session = sessions.route(global)?;
    let local = session.local_pane(global)?;
    pending::read(&session.herdr, &local, agent)
        .await
        .map(|p| p.question)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use kampr_core::provider::PaneInfo;
    use std::time::Duration;

    fn herd(status: AgentStatus, panes: &[&str]) -> HerdModel {
        HerdModel {
            nodes: Vec::new(),
            panes: panes
                .iter()
                .map(|id| {
                    PaneEntry::new(
                        "01J",
                        &PaneInfo {
                            pane_id: (*id).to_string(),
                            agent: Some("claude".into()),
                            agent_status: status,
                            cwd: Some("/srv/build/kampr".into()),
                            ..PaneInfo::default()
                        },
                        false,
                    )
                })
                .collect(),
        }
    }

    fn blocked_herd(panes: &[&str]) -> HerdModel {
        herd(AgentStatus::Blocked, panes)
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

    struct Watched {
        herd: watch::Sender<Arc<HerdModel>>,
        blocked: mpsc::Receiver<Change>,
        done: mpsc::Receiver<Change>,
        task: tokio::task::JoinHandle<()>,
    }

    fn watching(depth: usize) -> Watched {
        let (blocked, blocked_rx) = mpsc::channel(depth);
        let (done, done_rx) = mpsc::channel(depth);
        let (herd, herd_rx) = watch::channel(Arc::new(HerdModel::default()));
        let task = tokio::spawn(watch_herd(herd_rx, sessions(), Feeds { blocked, done }));
        Watched {
            herd,
            blocked: blocked_rx,
            done: done_rx,
            task,
        }
    }

    /// A queue smaller than the changes arriving at it.
    ///
    /// `try_send` drops the *value being sent*, so a full queue costs the newest change — and if
    /// the baseline were advanced anyway, the panes in that change would never be offered again
    /// for as long as they stayed blocked. A blocked agent nobody is told about is the whole
    /// feature failing quietly.
    #[tokio::test]
    async fn a_change_that_did_not_fit_the_queue_is_offered_again_rather_than_recorded_as_notified() {
        let panes = ["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w1:p5"];
        let mut watched = watching(1);

        let mut seen: HashSet<String> = HashSet::new();
        for at in 1..=panes.len() {
            watched.herd.send_replace(Arc::new(blocked_herd(&panes[..at])));
            while let Ok(Some(change)) =
                tokio::time::timeout(Duration::from_millis(200), watched.blocked.recv()).await
            {
                seen.extend(change.outstanding.into_iter().map(|p| p.pane));
            }
        }
        // One last update with nothing new in it: a change that was dropped is re-derived from the
        // same baseline, so the pane it named still arrives.
        watched.herd.send_replace(Arc::new(blocked_herd(&panes)));
        while let Ok(Some(change)) =
            tokio::time::timeout(Duration::from_millis(200), watched.blocked.recv()).await
        {
            seen.extend(change.outstanding.into_iter().map(|p| p.pane));
        }
        watched.task.abort();

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
        let mut watched = watching(16);

        let model = Arc::new(blocked_herd(&["w1:p1"]));
        for _ in 0..3 {
            watched.herd.send_replace(model.clone());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        watched.task.abort();

        let mut count = 0;
        while watched.blocked.try_recv().is_ok() {
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
        let mut watched = watching(16);

        watched
            .herd
            .send_replace(Arc::new(blocked_herd(&["w1:p1", "w2:p1"])));
        let first = next(&mut watched.blocked, "two panes blocked").await;
        assert_eq!(first.fresh.len(), 2);
        assert!(first.cleared.is_empty());

        watched.herd.send_replace(Arc::new(blocked_herd(&["w2:p1"])));
        let second = next(&mut watched.blocked, "one of the two was answered").await;
        watched.task.abort();

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
        let mut watched = watching(16);

        watched.herd.send_replace(Arc::new(blocked_herd(&["w1:p1"])));
        next(&mut watched.blocked, "the pane blocked").await;
        watched.herd.send_replace(Arc::new(HerdModel::default()));
        let change = next(&mut watched.blocked, "the last blocked pane was answered").await;
        watched.task.abort();

        assert!(change.outstanding.is_empty());
        assert_eq!(change.cleared.len(), 1);
    }

    /// The other half of the feature. herdr's `done` is a pane that finished `working`→`idle`
    /// while nobody was looking at it (#357) — the operator's unread flag — and it now reaches
    /// the phone on its own queue, carrying where it ran.
    #[tokio::test]
    async fn a_pane_that_finished_unwatched_is_a_change_of_its_own_kind() {
        let mut watched = watching(16);

        watched
            .herd
            .send_replace(Arc::new(herd(AgentStatus::Done, &["w1:p1"])));
        let change = next(&mut watched.done, "a pane finished").await;
        watched.task.abort();

        assert_eq!(change.kind, Kind::Done);
        assert_eq!(change.fresh.len(), 1);
        assert_eq!(change.outstanding[0].pane, "01J/w1:p1");
        assert_eq!(
            change.outstanding[0].detail.as_deref(),
            Some("/srv/build/kampr"),
            "where it ran is what tells three finished agents apart"
        );
    }

    /// And it falls the same way. Focusing the pane at the desk is the one thing that destroys
    /// herdr's `done` marker (#357, #396), so a phone still showing it is showing something the
    /// operator has already dealt with.
    #[tokio::test]
    async fn a_finished_pane_that_was_seen_at_the_desk_is_a_change_naming_it_as_cleared() {
        let mut watched = watching(16);

        watched
            .herd
            .send_replace(Arc::new(herd(AgentStatus::Done, &["w1:p1"])));
        next(&mut watched.done, "a pane finished").await;
        watched
            .herd
            .send_replace(Arc::new(herd(AgentStatus::Idle, &["w1:p1"])));
        let change = next(&mut watched.done, "the operator focused it at the desk").await;
        watched.task.abort();

        assert!(change.outstanding.is_empty());
        assert_eq!(change.cleared.iter().collect::<Vec<_>>(), vec!["01J/w1:p1"]);
    }

    /// **The reason there are two queues and not one.** A finished agent and a blocked one land
    /// in the same herd update all the time — one agent finishing while another asks a question
    /// is the ordinary case — and the collection window folds everything on a queue into a single
    /// payload under a single tag. One queue would have produced a notification naming both under
    /// one of the two tags, which is the set-not-edge rule failing from a new direction.
    #[tokio::test]
    async fn a_finished_pane_and_a_blocked_one_in_the_same_update_never_share_a_payload() {
        let mut watched = watching(16);

        let mut model = blocked_herd(&["w1:p1"]);
        model.panes.extend(herd(AgentStatus::Done, &["w2:p1"]).panes);
        watched.herd.send_replace(Arc::new(model));

        let blocked = next(&mut watched.blocked, "one pane blocked").await;
        let done = next(&mut watched.done, "another finished in the same update").await;
        watched.task.abort();

        assert_eq!(blocked.kind, Kind::Blocked);
        assert_eq!(
            blocked
                .outstanding
                .iter()
                .map(|p| p.pane.as_str())
                .collect::<Vec<_>>(),
            vec!["01J/w1:p1"],
        );
        assert_eq!(done.kind, Kind::Done);
        assert_eq!(
            done.outstanding
                .iter()
                .map(|p| p.pane.as_str())
                .collect::<Vec<_>>(),
            vec!["01J/w2:p1"],
        );
    }

    /// A blocked pane going quiet must not disturb the finished set's baseline, and the other way
    /// round. Shared state between the two is how one kind's queue pressure silences the other.
    #[tokio::test]
    async fn one_kinds_set_moving_is_not_a_change_to_the_other() {
        let mut watched = watching(16);

        watched.herd.send_replace(Arc::new(blocked_herd(&["w1:p1"])));
        next(&mut watched.blocked, "the pane blocked").await;
        watched
            .herd
            .send_replace(Arc::new(herd(AgentStatus::Done, &["w2:p1"])));
        next(&mut watched.done, "another pane finished").await;
        let cleared = next(&mut watched.blocked, "the blocked pane was answered").await;
        watched.task.abort();

        assert_eq!(cleared.kind, Kind::Blocked);
        assert!(
            watched.done.try_recv().is_err(),
            "the finished set did not move, so nothing on its queue should have"
        );
    }

    /// Pure, and taking the home rather than reading it: a test that sets `HOME` sets it for
    /// every other test in the binary, and the one next door asserts a path that is *not*
    /// shortened.
    #[test]
    fn a_path_under_the_operators_home_is_shortened_and_anything_else_is_left_alone() {
        let home = Some("/home/nobody");
        assert_eq!(home_relative("/home/nobody/dev/kampr", home), "~/dev/kampr");
        assert_eq!(home_relative("/home/nobody", home), "~");
        assert_eq!(home_relative("/srv/build", home), "/srv/build");
        assert_eq!(
            home_relative("/home/nobodyelse/dev", home),
            "/home/nobodyelse/dev",
            "a prefix match is not a path match"
        );
        assert_eq!(
            home_relative("/srv/build", None),
            "/srv/build",
            "a node with no HOME shortens nothing rather than guessing one"
        );
    }
}
