use crate::herd::HerdModel;
use crate::wire::Wire;
use kampr_core::provider::AgentStatus;
use kampr_core::wire::ServerMsg;
use kampr_core::{HerdrProvider, PaneRegistry};
use kampr_journal::{
    Change, Harness, Journal, JournalError, Registry as Journals, Role, SessionKind, SessionRef, Turn, Watch,
};
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

/// How often the visible screen is re-read while a turn is in progress.
///
/// Not a re-parse of the pane: the emulator already holds the grid the client is watching, so a
/// tick copies forty rows of text out of it and walks them upwards from the composer. It runs only
/// while the pane reports `working`, and only for a client that asked for the conversation — an
/// idle pane and a pane nobody is reading the conversation of both cost nothing at all.
const LIVE_POLL: Duration = Duration::from_millis(200);

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

/// Everything the pane's host knows about *which session* the pane is having, as opposed to which
/// directory it is in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    /// The session a pane has announced to herdr, when it has announced one. Herdr 0.8.2 detects
    /// both harnesses by scraping the screen and leaves `agent_session` null (probe #75), so in
    /// practice this is empty — but an announcement is exact, so it wins whenever a harness
    /// starts making one.
    pub announced: Option<SessionRef>,
    /// The harness process in the pane. This is what actually identifies a session, and it is
    /// what changes when an agent is quit and a fresh one started in the same pane.
    pub harness: Harness,
}

pub fn identity(provider: &HerdrProvider, local: &str) -> Identity {
    let snapshot = provider.snapshot();
    let announced = snapshot
        .pane(local)
        .and_then(|p| p.agent_session.as_ref())
        .map(|session| SessionRef {
            agent: session.agent.clone(),
            kind: match session.kind.as_str() {
                "path" => SessionKind::Path,
                _ => SessionKind::Id,
            },
            value: session.value.clone(),
        });
    Identity {
        announced,
        harness: provider.agent_harness(local),
    }
}

/// A page of turns running backwards from `before`, or from the newest turn when it is `None`.
/// `None` means this pane has no transcript open at all.
pub fn page(journal: &Open, pane: &str, before: Option<&str>) -> Option<ServerMsg> {
    let guard = journal.lock().unwrap();
    Some(ServerMsg::convo(pane, guard.as_ref()?.page_before(before, PAGE)))
}

/// What a pane has to keep for its conversation to be the same conversation: the harness, the
/// working directory, and the session inside them. A change to any of the three is a different
/// transcript, and the one the client is holding has to be taken off the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Handle {
    agent: Option<String>,
    cwd: Option<String>,
    identity: Identity,
}

type Look = Box<dyn Fn(&str) -> Identity + Send>;

pub struct ConvoCtx {
    pub journals: Arc<Journals>,
    pub panes: Arc<PaneRegistry>,
    pub herd: watch::Receiver<Arc<HerdModel>>,
    pub identity: Look,
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
        panes,
        mut herd,
        identity,
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

    let mut live_poll = tokio::time::interval(LIVE_POLL);
    live_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut opened: Option<PathBuf> = None;
    let mut handle: Option<Handle> = None;
    let mut due = true;
    let mut misses = 0u32;
    let mut live = Watch::default();
    // The transcript the client is holding turns from, and their ids, from the moment this pump
    // let go of it. See [`withdraw`].
    let mut shown: Option<(PathBuf, Vec<String>)> = None;
    let mut was_working = false;

    loop {
        let now = pane_of(&herd, &global, &identity, &local);
        let working = status_of(&herd, &global) == AgentStatus::Working;
        // A turn that ends without the status moving is covered by the transcript catching up,
        // but a turn the operator interrupts leaves its half-written text on the screen forever —
        // so leaving `working` withdraws whatever is showing.
        //
        // The transcript is read first, and in this order deliberately: the harness writes the
        // record and *then* goes idle, so withdrawing before that read leaves the client with
        // neither the preview nor its replacement for as long as the follow tick takes to notice.
        if (!working || opened.is_none()) && live.showing() {
            if !flush(&journal, &wire, &global).await {
                return;
            }
            if send_live(&wire, &global, live.stop()).is_err() {
                return;
            }
        }
        // The harness moved, the directory moved, the *session inside them* moved, or the file
        // went away: whatever is open is the wrong conversation. A pane whose agent was quit and
        // restarted looks identical in every field but the process, which is why the process is
        // in the handle.
        if handle.as_ref() != Some(&now) || opened.as_deref().is_some_and(|p| !p.is_file()) {
            handle = Some(now.clone());
            opened = None;
            shown = close(&journal).or(shown);
            due = true;
            misses = 0;
        }

        // A pane starting a turn is about to have a transcript whether or not it had one before,
        // so the retries a fresh session already spent looking for a file that did not exist yet
        // are given back. Without this a first message waits out [`RESOLVE_EVERY`].
        if working && !was_working && opened.is_none() {
            due = true;
            misses = 0;
        }
        was_working = working;

        if due && opened.is_none() {
            due = false;
            match resolve(&journals, &now) {
                None => misses += 1,
                Some(fresh) => {
                    misses = 0;
                    if !withdraw(&wire, &global, &mut shown, fresh.path()) {
                        return;
                    }
                    opened = Some(fresh.path().to_path_buf());
                    *journal.lock().unwrap() = Some(fresh);
                    // The first read is the page the client is about to be sent, so it must not
                    // also arrive behind it as a revision.
                    let _ = drain(&journal).await;
                    match page(&journal, &global, None) {
                        Some(first) if wire.send(&first) => {}
                        _ => return,
                    }
                }
            }
        }

        tokio::select! {
            _ = follow.tick() => {
                match drain(&journal).await {
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
                        shown = close(&journal).or(shown);
                        due = true;
                    }
                }
            }
            // A preview is the one thing on this socket that can be dropped without loss: the
            // record behind it is still coming, and a client that is already behind does not want
            // a fifth revision of a message it has not drawn yet.
            _ = live_poll.tick(), if working && opened.is_some() && !wire.outbox().congested() => {
                let change = match panes.screen(&local) {
                    Some(rows) => {
                        let borrowed: Vec<&str> = rows.iter().map(String::as_str).collect();
                        let seen = journal.lock().unwrap().as_ref().and_then(|j| j.preview(&borrowed));
                        live.observe(seen)
                    }
                    None => live.stop(),
                };
                if send_live(&wire, &global, change).is_err() {
                    return;
                }
            }
            _ = retry.tick(), if opened.is_none() && misses < FAST_RETRIES => due = true,
            _ = recheck.tick() => {
                let latest = resolve(&journals, &now);
                if latest.as_deref().map(Journal::path) != opened.as_deref() {
                    opened = None;
                    shown = close(&journal).or(shown);
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

/// Sends whatever the transcript has grown by, if anything. A read that fails is left to the
/// follow tick, which re-derives the transcript rather than dropping a turn.
async fn flush(journal: &Open, wire: &Wire, pane: &str) -> bool {
    match drain(journal).await {
        Ok(turns) if !turns.is_empty() => wire.send(&ServerMsg::ConvoTurn {
            pane: pane.to_string(),
            turns,
        }),
        _ => true,
    }
}

/// A live turn is a *revision* like any other, which is what lets it be withdrawn: the same id
/// with no blocks, and a client that matches by id and replaces is rid of it.
fn send_live(wire: &Wire, pane: &str, change: Change) -> Result<(), ()> {
    let turns = match change {
        Change::Held => return Ok(()),
        Change::Show(turn) => vec![turn],
        Change::Retire => vec![kampr_journal::retired()],
    };
    match wire.send(&ServerMsg::ConvoTurn {
        pane: pane.to_string(),
        turns,
    }) {
        true => Ok(()),
        false => Err(()),
    }
}

fn status_of(herd: &watch::Receiver<Arc<HerdModel>>, global: &str) -> AgentStatus {
    herd.borrow()
        .pane(global)
        .map(|p| p.agent_status)
        .unwrap_or_default()
}

fn pane_of(herd: &watch::Receiver<Arc<HerdModel>>, global: &str, look: &Look, local: &str) -> Handle {
    let (agent, cwd) = herd
        .borrow()
        .pane(global)
        .map(|p| (p.agent.clone(), p.cwd.clone()))
        .unwrap_or_default();
    let identity = match agent.is_some() {
        true => look(local),
        false => Identity::default(),
    };
    Handle { agent, cwd, identity }
}

fn resolve(journals: &Journals, handle: &Handle) -> Option<Box<dyn Journal>> {
    journals
        .open(
            handle.agent.as_deref(),
            handle.identity.announced.as_ref(),
            handle.cwd.as_deref().map(Path::new),
            &handle.identity.harness,
        )
        .ok()
        .flatten()
}

/// Lets go of the open transcript and reports what the client is left holding from it.
fn close(journal: &Open) -> Option<(PathBuf, Vec<String>)> {
    let old = journal.lock().unwrap().take()?;
    Some((old.path().to_path_buf(), old.turn_ids()))
}

/// Takes the previous conversation off the client before a different one is sent.
///
/// **A page merges; it does not replace.** Turns are matched by id, and the ids of another
/// session's transcript match nothing — so a fresh page for a *different* transcript arrives
/// above the old conversation instead of in place of it, which reads exactly like the panel
/// refusing to update. Withdrawing first is the existing retirement mechanism applied to the
/// whole conversation: a turn carrying no blocks is not drawn.
fn withdraw(wire: &Wire, pane: &str, shown: &mut Option<(PathBuf, Vec<String>)>, fresh: &Path) -> bool {
    let Some((path, ids)) = shown.take() else {
        return true;
    };
    if path == fresh || ids.is_empty() {
        return true;
    }
    let turns = ids
        .into_iter()
        .map(|id| Turn::new(id, Role::Assistant, None))
        .collect();
    wire.send(&ServerMsg::ConvoTurn {
        pane: pane.to_string(),
        turns,
    })
}

/// **Off the executor**, because the lock it takes is the one `convo.load` also wants and the
/// work under it is a file read: `poll` re-reads a transcript that can be tens of megabytes, and
/// the first one after a resolve parses the whole file.
async fn drain(journal: &Open) -> Result<Vec<Turn>, JournalError> {
    let journal = journal.clone();
    tokio::task::spawn_blocking(move || match journal.lock().unwrap().as_mut() {
        Some(journal) => journal.poll(),
        None => Ok(Vec::new()),
    })
    .await
    .unwrap_or_else(|e| Err(JournalError::Io(std::io::Error::other(e))))
}
