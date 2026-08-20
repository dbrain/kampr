use crate::herd::HerdModel;
use crate::wire::Wire;
use kampr_core::HerdrProvider;
use kampr_core::wire::ServerMsg;
use kampr_journal::{Journal, Registry as Journals, SessionKind, SessionRef};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tracing::debug;

/// Turns in a page. A client pages backwards from the cursor with `convo.load { before }`.
const PAGE: usize = 40;

/// How often a followed transcript is re-read. A transcript is a file that grows, so this is the
/// whole tail mechanism.
const POLL: Duration = Duration::from_millis(400);

/// How often the pane is asked which transcript it is on *now*. `/clear` opens a new file under
/// the same working directory and nothing announces it — but deriving the answer reads
/// directories, so it does not happen at the follow rate.
const RESOLVE_EVERY: Duration = Duration::from_secs(15);

/// How often to look again while there is nothing open. A harness started a moment ago has not
/// written its transcript yet, and the first page should not wait a whole [`RESOLVE_EVERY`].
const RETRY_EVERY: Duration = Duration::from_secs(2);

/// How many of those fast retries a pane gets before it falls back to [`RESOLVE_EVERY`]. Deriving
/// a transcript from a working directory nothing has ever run in is the one case that scans every
/// project directory, and it is also the case that never succeeds — so it must not stay hot.
const FAST_RETRIES: u32 = 5;

/// The open transcript behind one watched pane, shared between its pump and `convo.load`.
pub type Open = Arc<Mutex<Option<Box<dyn Journal>>>>;

pub fn open() -> Open {
    Arc::new(Mutex::new(None))
}

/// The session a pane has announced to herdr, when it has announced one.
///
/// Herdr 0.8.2 detects both harnesses by scraping the screen and leaves `agent_session` null, so
/// in practice this is empty and the working directory carries the resolution. An announcement is
/// exact, though, so it wins whenever a harness starts making one.
pub fn announced(provider: &HerdrProvider, local: &str) -> Option<SessionRef> {
    let snapshot = provider.snapshot();
    let session = snapshot.pane(local)?.agent_session.as_ref()?;
    Some(SessionRef {
        agent: session.agent.clone(),
        kind: match session.kind.as_str() {
            "path" => SessionKind::Path,
            _ => SessionKind::Id,
        },
        value: session.value.clone(),
    })
}

/// A page of turns running backwards from `before`, or from the newest turn when it is `None`.
/// `None` means this pane has no transcript open at all.
pub fn page(journal: &Open, pane: &str, before: Option<&str>) -> Option<ServerMsg> {
    let guard = journal.lock().unwrap();
    Some(ServerMsg::convo(pane, guard.as_ref()?.page_before(before, PAGE)))
}

type Handle = (Option<String>, Option<String>);
type Announce = Box<dyn Fn(&str) -> Option<SessionRef> + Send>;

pub struct ConvoCtx {
    pub journals: Arc<Journals>,
    pub herd: watch::Receiver<Arc<HerdModel>>,
    pub snapshot: Announce,
    pub wire: Arc<Wire>,
    pub global: String,
    pub local: String,
    pub journal: Open,
}

/// One pane's transcript for one client: the initial page, then the tail.
///
/// Turns arrive **revised, not appended** — `Journal::poll` returns what the last read added *or
/// changed*, so a tool turn comes back under its own id when its result lands. A client that
/// appends renders every tool twice, which is why the wire says match by id and replace.
pub async fn pump_convo(ctx: ConvoCtx) {
    let ConvoCtx {
        journals,
        mut herd,
        snapshot,
        wire,
        global,
        local,
        journal,
    } = ctx;

    let mut follow = tokio::time::interval(POLL);
    follow.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut recheck = tokio::time::interval(RESOLVE_EVERY);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    recheck.tick().await;
    let mut retry = tokio::time::interval(RETRY_EVERY);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut opened: Option<PathBuf> = None;
    let mut handle: Option<Handle> = None;
    let mut due = true;
    let mut misses = 0u32;

    loop {
        let now = pane_of(&herd, &global);
        // The harness or the directory moved, or the file went away: whatever is open is the
        // wrong conversation.
        if handle.as_ref() != Some(&now) || opened.as_deref().is_some_and(|p| !p.is_file()) {
            handle = Some(now.clone());
            opened = None;
            *journal.lock().unwrap() = None;
            due = true;
            misses = 0;
        }

        if due && opened.is_none() {
            due = false;
            match resolve(&journals, &snapshot, &local, &now) {
                None => misses += 1,
                Some(fresh) => {
                    misses = 0;
                    opened = Some(fresh.path().to_path_buf());
                    *journal.lock().unwrap() = Some(fresh);
                    // The first read is the page the client is about to be sent, so it must not
                    // also arrive behind it as a revision.
                    let _ = drain(&journal);
                    match page(&journal, &global, None) {
                        Some(first) if wire.send(&first) => {}
                        _ => return,
                    }
                }
            }
        }

        tokio::select! {
            _ = follow.tick() => {
                match drain(&journal) {
                    Ok(turns) if !turns.is_empty() => {
                        let revised = ServerMsg::ConvoTurn { pane: global.clone(), turns };
                        if !wire.send(&revised) {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        debug!(pane = %global, error = %e, "transcript unreadable; re-deriving");
                        opened = None;
                        *journal.lock().unwrap() = None;
                        due = true;
                    }
                }
            }
            _ = retry.tick(), if opened.is_none() && misses < FAST_RETRIES => due = true,
            _ = recheck.tick() => {
                let latest = resolve(&journals, &snapshot, &local, &now);
                if latest.as_deref().map(Journal::path) != opened.as_deref() {
                    opened = None;
                }
                due = true;
            }
            changed = herd.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

fn pane_of(herd: &watch::Receiver<Arc<HerdModel>>, global: &str) -> Handle {
    herd.borrow()
        .pane(global)
        .map(|p| (p.agent.clone(), p.cwd.clone()))
        .unwrap_or_default()
}

fn resolve(
    journals: &Journals,
    snapshot: &Announce,
    local: &str,
    (agent, cwd): &Handle,
) -> Option<Box<dyn Journal>> {
    let cwd = cwd.as_deref().map(Path::new);
    journals
        .open(agent.as_deref(), snapshot(local).as_ref(), cwd)
        .ok()
        .flatten()
}

fn drain(journal: &Open) -> Result<Vec<kampr_journal::Turn>, kampr_journal::JournalError> {
    match journal.lock().unwrap().as_mut() {
        Some(journal) => journal.poll(),
        None => Ok(Vec::new()),
    }
}
