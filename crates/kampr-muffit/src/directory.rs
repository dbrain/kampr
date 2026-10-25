use crate::client::{self, Client};
use crate::wire::{ServerFrame, SessionInfo, SessionState, StoredSession};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info};

const FIRST_RETRY: Duration = Duration::from_millis(250);
const LAST_RETRY: Duration = Duration::from_secs(5);
const STORE_POLL: Duration = Duration::from_secs(30);
const STORED_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
const STORED_KEPT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub info: SessionInfo,
    pub live: bool,
}

#[derive(Default)]
struct Held {
    live: BTreeMap<String, SessionInfo>,
    stored: BTreeMap<String, SessionInfo>,
}

/// The daemon's sessions: the live ones followed over `GET /sessions/stream`, and the finished
/// conversations of the last week read from `GET /sessions?all=true`, because muffit lists a
/// session as live only while a turn runs or a client is attached (docs/18).
///
/// A daemon that is down lists nothing and is retried for ever: muffit being away is a state of
/// the herd, never a fault in it.
pub struct Directory {
    held: Mutex<Held>,
    revision: watch::Sender<u64>,
    moved: Arc<Notify>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Directory {
    pub fn follow(client: Client) -> Arc<Self> {
        let (revision, _) = watch::channel(0);
        let directory = Arc::new(Self {
            held: Mutex::default(),
            revision,
            moved: Arc::default(),
            tasks: Mutex::new(Vec::new()),
        });
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                let stream = runtime.spawn(stream(client.clone(), Arc::downgrade(&directory)));
                let moved = directory.moved.clone();
                let store = runtime.spawn(store(client, Arc::downgrade(&directory), moved));
                *directory.tasks.lock().unwrap() = vec![stream, store];
            }
            Err(_) => tracing::warn!("muffit's sessions are not followed outside a runtime"),
        }
        directory
    }

    /// Live rows win over the store's for the same id.
    pub fn sessions(&self) -> Vec<Listed> {
        let held = self.held.lock().unwrap();
        let live = held.live.values().map(|info| Listed {
            info: info.clone(),
            live: true,
        });
        let stored = held
            .stored
            .values()
            .filter(|info| !held.live.contains_key(&info.id))
            .map(|info| Listed {
                info: info.clone(),
                live: false,
            });
        live.chain(stored).collect()
    }

    pub fn session(&self, id: &str) -> Option<SessionInfo> {
        let held = self.held.lock().unwrap();
        held.live.get(id).or_else(|| held.stored.get(id)).cloned()
    }

    pub fn is_live(&self, id: &str) -> bool {
        self.held.lock().unwrap().live.contains_key(id)
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    pub fn stop(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    fn change(&self, change: impl FnOnce(&mut Held) -> bool) {
        let changed = change(&mut self.held.lock().unwrap());
        if changed {
            self.revision.send_modify(|n| *n += 1);
        }
    }

    fn replace(&self, sessions: Vec<SessionInfo>) {
        let next: BTreeMap<String, SessionInfo> = sessions
            .into_iter()
            .filter(|s| s.state != SessionState::Stopped && !is_worker(&s.id))
            .map(|s| (s.id.clone(), s))
            .collect();
        self.change(|held| {
            let changed = held.live != next;
            held.live = next;
            changed
        });
    }

    fn upsert(&self, session: SessionInfo) {
        self.change(|held| match session.state {
            _ if is_worker(&session.id) => false,
            SessionState::Stopped => {
                let live = held.live.remove(&session.id).is_some();
                held.stored.remove(&session.id).is_some() || live
            }
            _ if held.live.get(&session.id) == Some(&session) => false,
            _ => {
                held.live.insert(session.id.clone(), session);
                true
            }
        });
    }

    fn store(&self, rows: Vec<StoredSession>) {
        let next = kept(rows, now_ms());
        self.change(|held| {
            let changed = held.stored != next;
            held.stored = next;
            changed
        });
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        self.stop();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The finished conversations worth a pane: the newest twenty touched in the last week. A live
/// one is the stream's, and takes none of the twenty.
fn kept(mut rows: Vec<StoredSession>, now: u64) -> BTreeMap<String, SessionInfo> {
    let since = now.saturating_sub(STORED_FOR.as_millis() as u64);
    rows.retain(|r| {
        !r.live && r.updated_at_ms >= since && r.state != SessionState::Stopped && !is_worker(&r.id)
    });
    rows.sort_by_key(|r| std::cmp::Reverse(r.updated_at_ms));
    rows.into_iter()
        .take(STORED_KEPT)
        .map(|r| {
            (
                r.id.clone(),
                SessionInfo {
                    id: r.id,
                    state: r.state,
                    turns: 0,
                    title: r.title,
                    cwd: r.project,
                    branch: r.branch,
                    model: None,
                    permission: None,
                },
            )
        })
        .collect()
}

async fn stream(client: Client, directory: Weak<Directory>) {
    let mut retry = FIRST_RETRY;
    let mut was_up = false;
    loop {
        match client.socket("/sessions/stream", &[]).await {
            Ok(mut socket) => {
                if !was_up {
                    info!("following the muffit daemon's sessions");
                    was_up = true;
                }
                while let Some(frame) = client::next(&mut socket).await {
                    let Some(directory) = directory.upgrade() else {
                        return;
                    };
                    match frame {
                        ServerFrame::Sessions { sessions } => {
                            retry = FIRST_RETRY;
                            directory.replace(sessions);
                        }
                        ServerFrame::Session { session } => directory.upsert(session),
                        _ => continue,
                    }
                    // A session that left the live list is in the store now, and one that joined
                    // it has just been touched.
                    directory.moved.notify_one();
                }
                debug!("the muffit session stream closed");
            }
            Err(e) => {
                if was_up {
                    info!(error = %format!("{e:#}"), "the muffit daemon went away");
                    was_up = false;
                }
                debug!(error = %format!("{e:#}"), "muffit is not answering");
            }
        }
        match directory.upgrade() {
            Some(directory) => directory.replace(Vec::new()),
            None => return,
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(LAST_RETRY);
    }
}

async fn store(client: Client, directory: Weak<Directory>, moved: Arc<Notify>) {
    loop {
        let rows = client.stored().await;
        let Some(held) = directory.upgrade() else {
            return;
        };
        match rows {
            Ok(rows) => held.store(rows),
            Err(e) => {
                debug!(error = %format!("{e:#}"), "reading muffit's stored sessions");
                held.store(Vec::new());
            }
        }
        drop(held);
        tokio::select! {
            _ = moved.notified() => {}
            _ = tokio::time::sleep(STORE_POLL) => {}
        }
    }
}

/// A worker is `<parent>.s<n>` (kampr.md §6): a subconversation of its parent, never a pane.
pub fn is_worker(id: &str) -> bool {
    id.rsplit_once('.')
        .and_then(|(_, last)| last.strip_prefix('s'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_keeps_the_newest_twenty_of_the_last_week() {
        let now = 30 * 24 * 3_600_000;
        let row = |id: &str, ago_hours: u64, state: SessionState| StoredSession {
            id: id.into(),
            live: false,
            project: Some("/srv".into()),
            state,
            title: None,
            branch: None,
            updated_at_ms: now - ago_hours * 3_600_000,
        };
        let mut rows: Vec<StoredSession> = (0..25)
            .map(|n| row(&format!("s{n}"), n + 1, SessionState::Completed))
            .collect();
        rows.push(row("old", 8 * 24, SessionState::Completed));
        rows.push(row("ended", 0, SessionState::Stopped));
        rows.push(row("s0.s1", 0, SessionState::Completed));
        rows.push(StoredSession {
            live: true,
            ..row("live", 0, SessionState::Running)
        });
        let kept = kept(rows, now);
        let want: Vec<String> = (0..20).map(|n| format!("s{n}")).collect();
        let mut got: Vec<String> = kept.keys().cloned().collect();
        got.sort_by_key(|id| id[1..].parse::<u32>().unwrap());
        assert_eq!(got, want);
        assert_eq!(kept["s3"].cwd.as_deref(), Some("/srv"));
    }

    #[test]
    fn a_worker_is_told_from_a_session_by_its_id() {
        for (id, worker) in [
            ("s1.s1", true),
            ("06GQ1XBH4RX0X5GT0Y75S8H87V.s12", true),
            ("a.s1.s3", true),
            ("06GQ1XBH4RX0X5GT0Y75S8H87V", false),
            ("web.sx", false),
            ("v1.s", false),
        ] {
            assert_eq!(super::is_worker(id), worker, "{id}");
        }
    }
}
