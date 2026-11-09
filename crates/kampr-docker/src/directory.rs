use crate::api::{Event, Inspect};
use crate::client::{self, Client};
use crate::journal::{Entry, Journal};
use kampr_core::wire::ContainerEntry;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info};

const FIRST_RETRY: Duration = Duration::from_millis(250);
const LAST_RETRY: Duration = Duration::from_secs(5);

/// The daemon's containers, followed over `/events`. A daemon that is down lists nothing and is
/// retried for ever: Docker being away is a state of the herd, never a fault in it.
pub struct Directory {
    client: Client,
    required: bool,
    seen: Mutex<Seen>,
    journal: Mutex<Journal>,
    revision: watch::Sender<u64>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct Seen {
    containers: BTreeMap<String, Inspect>,
    up: bool,
    answered: bool,
}

#[derive(PartialEq)]
struct View {
    reported: Option<bool>,
    containers: Vec<ContainerEntry>,
}

impl Directory {
    /// `required` is a source the operator asked for, which reports `docker: false` while the
    /// socket is down; otherwise the source shows only once the daemon has answered.
    pub fn follow(client: Client, journal: Journal, required: bool) -> Arc<Self> {
        let (revision, _) = watch::channel(0);
        let directory = Arc::new(Self {
            client: client.clone(),
            required,
            seen: Mutex::default(),
            journal: Mutex::new(journal),
            revision,
            task: Mutex::new(None),
        });
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                let task = runtime.spawn(run(client, Arc::downgrade(&directory)));
                *directory.task.lock().unwrap() = Some(task);
            }
            Err(_) => tracing::warn!("docker's containers are not followed outside a runtime"),
        }
        directory
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn reported(&self) -> Option<bool> {
        reported(self.required, &self.seen.lock().unwrap())
    }

    pub fn containers(&self) -> Vec<ContainerEntry> {
        self.containers_at(now())
    }

    pub fn containers_at(&self, now: i64) -> Vec<ContainerEntry> {
        let seen = self.seen.lock().unwrap();
        let journal = self.journal.lock().unwrap();
        entries(&seen, &journal, now)
    }

    /// By full id, by name, or by an id prefix of at least twelve, as `docker` takes them.
    pub fn find(&self, id: &str) -> Option<Inspect> {
        let seen = self.seen.lock().unwrap();
        let found = seen
            .containers
            .values()
            .find(|c| c.id == id || c.short_name() == id || (id.len() >= 12 && c.id.starts_with(id)));
        found.cloned()
    }

    pub fn events_of(&self, id: &str, limit: usize) -> Vec<Entry> {
        self.journal
            .lock()
            .unwrap()
            .of(id, limit)
            .into_iter()
            .cloned()
            .collect()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    pub fn stop(&self) {
        if let Some(task) = self.task.lock().unwrap().take() {
            task.abort();
        }
    }

    fn view(&self, seen: &Seen, journal: &Journal) -> View {
        View {
            reported: reported(self.required, seen),
            containers: entries(seen, journal, now()),
        }
    }

    fn change(&self, change: impl FnOnce(&mut Seen, &mut Journal)) {
        let mut seen = self.seen.lock().unwrap();
        let mut journal = self.journal.lock().unwrap();
        let before = self.view(&seen, &journal);
        change(&mut seen, &mut journal);
        if self.view(&seen, &journal) != before {
            self.revision.send_modify(|n| *n += 1);
        }
    }

    fn since(&self) -> Option<i64> {
        self.journal.lock().unwrap().last_nano()
    }

    fn up(&self, containers: BTreeMap<String, Inspect>) {
        self.change(|seen, _| {
            if !seen.up {
                info!(socket = %self.client.socket().display(), "following docker's containers");
            }
            seen.containers = containers;
            seen.up = true;
            seen.answered = true;
        });
    }

    fn down(&self, why: &anyhow::Error) {
        self.change(|seen, _| {
            if seen.up {
                info!(error = %format!("{why:#}"), "docker went away");
            }
            debug!(error = %format!("{why:#}"), "docker is not answering");
            seen.containers.clear();
            seen.up = false;
        });
    }

    async fn apply(&self, event: Event) {
        if event.action.starts_with("exec_") {
            return;
        }
        let id = event.actor.id.clone();
        let inspected = match event.action.as_str() {
            "destroy" => None,
            _ => match self.client.inspect(&id).await {
                Ok(inspect) => Some(inspect),
                Err(e) if client::is_not_found(&e) => None,
                Err(e) => {
                    debug!(error = %format!("{e:#}"), container = %id, "could not inspect a container");
                    return;
                }
            },
        };
        self.change(|seen, journal| {
            if let Some(entry) = Entry::from_event(&event) {
                journal.record(entry, now());
            }
            match inspected {
                Some(inspect) => seen.containers.insert(id, inspect),
                None => seen.containers.remove(&id),
            };
        });
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        self.stop();
    }
}

fn reported(required: bool, seen: &Seen) -> Option<bool> {
    (required || seen.answered).then_some(seen.up)
}

fn entries(seen: &Seen, journal: &Journal, now: i64) -> Vec<ContainerEntry> {
    let crashes = crate::crashes::crashes(journal.entries(), now);
    let mut entries: Vec<ContainerEntry> = seen
        .containers
        .values()
        .map(|c| crate::entry::entry(c, crashes.get(&c.id)))
        .collect();
    crate::entry::sort(&mut entries);
    entries
}

async fn listed(client: &Client) -> anyhow::Result<BTreeMap<String, Inspect>> {
    let mut containers = BTreeMap::new();
    for id in client.ids().await? {
        match client.inspect(&id).await {
            Ok(inspect) => {
                containers.insert(id, inspect);
            }
            Err(e) if client::is_not_found(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(containers)
}

/// The stream is opened before the list is read, so a change between the two is an event still
/// to come rather than a change nobody reports.
async fn run(client: Client, directory: Weak<Directory>) {
    let mut retry = FIRST_RETRY;
    loop {
        let Some(since) = directory.upgrade().map(|d| d.since()) else {
            return;
        };
        let followed = async {
            let mut events = client.events(since).await?;
            let containers = listed(&client).await?;
            directory.upgrade().ok_or_else(gone)?.up(containers);
            retry = FIRST_RETRY;
            while let Some(event) = events.next().await {
                directory.upgrade().ok_or_else(gone)?.apply(event).await;
            }
            anyhow::Ok(anyhow::anyhow!("the event stream ended"))
        };
        let why = followed.await.unwrap_or_else(|e| e);
        match directory.upgrade() {
            Some(directory) => directory.down(&why),
            None => return,
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(LAST_RETRY);
    }
}

fn gone() -> anyhow::Error {
    anyhow::anyhow!("the directory was dropped")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
