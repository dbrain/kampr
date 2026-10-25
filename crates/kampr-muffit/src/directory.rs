use crate::client::{self, Client};
use crate::wire::{ServerFrame, SessionInfo, SessionState};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info};

const FIRST_RETRY: Duration = Duration::from_millis(250);
const LAST_RETRY: Duration = Duration::from_secs(5);

/// The daemon's live sessions, followed over `GET /sessions/stream`. A finished conversation
/// nobody is attached to is not one, as a closed claude is not a pane (docs/18).
///
/// A daemon that is down lists nothing and is retried for ever: muffit being away is a state of
/// the herd, never a fault in it.
pub struct Directory {
    live: Mutex<BTreeMap<String, SessionInfo>>,
    revision: watch::Sender<u64>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Directory {
    pub fn follow(client: Client) -> Arc<Self> {
        let (revision, _) = watch::channel(0);
        let directory = Arc::new(Self {
            live: Mutex::default(),
            revision,
            tasks: Mutex::new(Vec::new()),
        });
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                let stream = runtime.spawn(stream(client, Arc::downgrade(&directory)));
                *directory.tasks.lock().unwrap() = vec![stream];
            }
            Err(_) => tracing::warn!("muffit's sessions are not followed outside a runtime"),
        }
        directory
    }

    pub fn sessions(&self) -> Vec<SessionInfo> {
        self.live.lock().unwrap().values().cloned().collect()
    }

    pub fn session(&self, id: &str) -> Option<SessionInfo> {
        self.live.lock().unwrap().get(id).cloned()
    }

    pub fn is_live(&self, id: &str) -> bool {
        self.live.lock().unwrap().contains_key(id)
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    pub fn stop(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    fn change(&self, change: impl FnOnce(&mut BTreeMap<String, SessionInfo>) -> bool) {
        let changed = change(&mut self.live.lock().unwrap());
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
        self.change(|live| {
            let changed = *live != next;
            *live = next;
            changed
        });
    }

    fn upsert(&self, session: SessionInfo) {
        self.change(|live| match session.state {
            _ if is_worker(&session.id) => false,
            SessionState::Stopped => live.remove(&session.id).is_some(),
            _ if live.get(&session.id) == Some(&session) => false,
            _ => {
                live.insert(session.id.clone(), session);
                true
            }
        });
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn stream(client: Client, directory: Weak<Directory>) {
    let mut retry = FIRST_RETRY;
    let mut was_up = false;
    loop {
        match client.socket("/sessions/stream", None, &[]).await {
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

/// A worker is `<parent>.s<n>` (kampr.md §6): a subconversation of its parent, never a pane.
pub fn is_worker(id: &str) -> bool {
    id.rsplit_once('.')
        .and_then(|(_, last)| last.strip_prefix('s'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
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
