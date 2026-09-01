use crate::herd::HerdModel;
use crate::wire::Wire;
use kampr_core::provider::AgentStatus;
use kampr_core::wire::ServerMsg;
use kampr_core::{HerdrProvider, PaneRegistry};
use kampr_journal::{
    Change, Composed, ComposerFeed, ComposerReader, FacetFeed, Facets, Harness, Journal, JournalError,
    Registry as Journals, Role, SessionKind, SessionMarker, SessionRef, Turn, Watch,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tracing::debug;

/// Turns in a page, as a floor rather than a ceiling. A client pages backwards from the cursor
/// with `convo.load { before }`, and a page runs back past this to the question that opens the
/// reply it landed in — see `FileJournal::page_before`, where the rule and its bound live.
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
/// the same working directory and nothing announces it (#259) — but deriving the answer reads
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

/// Everything one pane's conversation cost to find, parse and fold, kept past the pump that did
/// it so that coming back to a pane is not doing it again. See [`crate::warm`] for why it is the
/// node that keeps this and not the session.
///
/// The pump owns this for as long as it runs. It is deliberately the whole of what an open costs
/// and nothing that merely describes one: `opened` and `handle` are what say the transcript below
/// is still the right transcript, and without them a warm pump would page the conversation of a
/// session the pane has since left.
pub struct Warm {
    pub journal: Open,
    pub opened: Option<PathBuf>,
    pub handle: Option<Handle>,
    pub facets: Option<FacetFeed>,
}

impl Warm {
    /// Nothing here was ever opened, so there is nothing here to keep.
    pub fn cold(&self) -> bool {
        self.opened.is_none()
    }

    /// The transcript went away, moved, or was never found: whatever was parsed off it is not this
    /// pane's conversation any more.
    fn forget(&mut self) {
        self.opened = None;
        self.facets = None;
    }
}

pub type Warmth = Arc<Mutex<Warm>>;

pub fn warmth() -> Warmth {
    Arc::new(Mutex::new(Warm {
        journal: open(),
        opened: None,
        handle: None,
        facets: None,
    }))
}

/// The transcript this client is holding turns from, and their ids.
///
/// **Kept by the client rather than by the pump, because the two have different lifetimes.** A
/// pump is created by `watch` and aborted by `unwatch` — which is what leaving a pane's screen
/// does — while the turns already drawn live as long as the app does: nothing on the client
/// prunes a pane it has stopped watching. A transcript that moves in that gap is therefore served
/// by a pump that has shown this client nothing and would withdraw nothing, and the new page
/// lands above a conversation that is still on the screen. See [`withdraw`].
pub type Held = Arc<Mutex<Option<(PathBuf, Vec<String>)>>>;

pub fn held() -> Held {
    Arc::new(Mutex::new(None))
}

/// The launched conversation this client currently has open on a pane, if any, followed for as
/// long as it has it open.
///
/// **A subagent's transcript grows while it runs**, and the reason to open one is to watch it
/// work — so a reader who has to close and re-open it to see the next step is being handed a
/// snapshot of something live. One at a time is the whole rule: opening another replaces this,
/// leaving the pane replaces it with nothing, and a client that never opens one pays for none of
/// it.
pub type Followed = Arc<Mutex<Option<(String, Box<dyn Journal>)>>>;

pub fn followed() -> Followed {
    Arc::new(Mutex::new(None))
}

/// Everything the pane's host knows about *which session* the pane is having, as opposed to which
/// directory it is in.
///
/// **Only what identifies a session belongs here, because this is compared.** A [`Handle`] that
/// differs is a different conversation, and the pump answers that by taking the open transcript
/// off the client and paging a fresh one — so a field that merely *describes* the session and
/// changes while it runs, such as the marker's `status` flipping between busy and idle, would tear
/// a conversation down and re-page it mid-turn. That is #314's defect wearing a different hat.
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

/// What the pane's own harness says it is on, when it says anything.
///
/// **This is the exact handle, and it exists before the transcript does.** Herdr detects an agent
/// by scraping the screen, so a pane is not looked up at all until that scrape lands (#75), and
/// `has_conversation` then means *a transcript file resolves* — which it does not for the whole
/// gap between a session opening and its first prompt, measured at 2 min 42 s (#311). Matching the
/// pane's whole pipeline against the markers the harness itself writes closes both, and it closes
/// them on **pid** rather than on a name, so a pane ble.sh lets herdr describe only as `bash`
/// (#297) is still identified exactly.
///
/// Cheap enough for the pump's own loop, which is where it runs: the marker file is opened by pid
/// rather than searched for, and only a hit goes on to scan the project directories. Measured on
/// this machine at **34 us on a hit and 1.7 us on a miss**, against a loop that turns at most five
/// times a second per watched pane — and a miss is every pane that is not an agent.
pub fn marker_of(journals: &Journals, provider: &HerdrProvider, local: &str) -> Option<SessionMarker> {
    journals.marker(&provider.pane_processes(local))
}

pub fn identity(journals: &Journals, provider: &HerdrProvider, local: &str) -> Identity {
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
        // Herdr's own announcement first where there is one — it is the harness speaking through
        // the host that owns the pane — and the marker underneath it, which is every case herdr
        // has not spoken about yet and (today) every case at all.
        announced: announced
            .or_else(|| marker_of(journals, provider, local).map(|m| SessionRef::id(m.agent, m.session))),
        harness: provider.agent_harness(local),
    }
}

/// A page of turns running backwards from `before`, or from the newest turn when it is `None`.
/// `None` means this pane has no transcript open at all.
pub fn page(journal: &Open, pane: &str, before: Option<&str>, fresh: bool) -> Option<ServerMsg> {
    let guard = journal.lock().unwrap();
    Some(ServerMsg::convo(
        pane,
        guard.as_ref()?.page_before(before, PAGE),
        fresh,
    ))
}

/// The first message a re-opened transcript sends the client.
///
/// **A page merges by prepending what the client does not recognise**, which is a rule written for
/// `convo.load` — that pages *backwards*, so everything unknown in it really is older. Re-opening
/// the transcript a client is already holding pages *forwards*: what it is missing is whatever was
/// written while the pump was down, and a page files those at the top, above a conversation from
/// hours earlier, on a view pinned to the bottom. The turn is then recorded as delivered and never
/// re-sent, so it is not lost so much as permanently misfiled — which is exactly how it was
/// reported: an answer that never appeared while every later turn arrived perfectly well.
///
/// Clients already installed on phones cannot be fixed from here, so the node does not send them a
/// page it knows they will misfile. The turns go out as `convo.turn` instead, which replaces by id
/// and **appends** the rest — the shape the tail already uses, so nothing on the wire is new.
///
/// Only while the two still overlap. A gap wide enough that nothing in the page is on the client's
/// screen cannot be ordered from either end, and that is a replacing page saying so.
fn reopened(journal: &Open, pane: &str, showing: Option<&[String]>) -> Option<ServerMsg> {
    let Some(ids) = showing else {
        return page(journal, pane, None, true);
    };
    let turns = journal.lock().unwrap().as_ref()?.page_before(None, PAGE).turns;
    match turns.iter().any(|turn| ids.contains(&turn.id)) {
        true => Some(ServerMsg::ConvoTurn {
            pane: pane.to_string(),
            sub: None,
            turns,
        }),
        false => page(journal, pane, None, true),
    }
}

/// What a pane has to keep for its conversation to be the same conversation: the harness, the
/// working directory, and the session inside them. A change to any of the three is a different
/// transcript, and the one the client is holding has to be taken off the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Handle {
    agent: Option<String>,
    cwd: Option<String>,
    identity: Identity,
}

type Look = Box<dyn Fn(&str) -> Identity + Send>;

/// The session marker, asked for separately from [`Identity`] and deliberately so: it carries what
/// *describes* a session rather than what identifies one, and putting it where the pump compares
/// would tear a conversation down every time the harness went from busy to idle.
type Describe = Box<dyn Fn(&str) -> Option<SessionMarker> + Send>;

pub struct ConvoCtx {
    pub journals: Arc<Journals>,
    /// This node's own, and the only reason the pump needs it: `pastes/` under it is what tells a
    /// path in the operator's own turn from a path they typed (`pasted::Shown`).
    pub state_dir: PathBuf,
    pub panes: Arc<PaneRegistry>,
    pub herd: watch::Receiver<Arc<HerdModel>>,
    pub identity: Look,
    pub describe: Describe,
    pub wire: Arc<Wire>,
    pub global: String,
    pub local: String,
    /// Everything this pane's conversation last cost to open, which on a re-watch is everything
    /// this pump does not have to do again.
    pub warm: Warmth,
    pub held: Held,
    pub followed: Followed,
}

/// One pane's transcript for one client: the initial page, then the tail.
///
/// Turns arrive **revised, not appended** — `Journal::poll` returns what the last read added *or
/// changed*, so a tool turn comes back under its own id when its result lands. A client that
/// appends renders every tool twice, which is why the wire says match by id and replace.
pub async fn pump_convo(ctx: ConvoCtx) {
    let ConvoCtx {
        journals,
        state_dir,
        panes,
        mut herd,
        identity,
        describe,
        wire,
        global,
        local,
        warm,
        held,
        followed,
    } = ctx;
    let journal = warm.lock().unwrap().journal.clone();

    let mut follow = tokio::time::interval(POLL);
    follow.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut recheck = tokio::time::interval(RESOLVE_EVERY);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    recheck.tick().await;
    let mut retry = tokio::time::interval(RETRY_EVERY);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut live_poll = tokio::time::interval(LIVE_POLL);
    live_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Where the pump before this one got to, which on a re-watch is a transcript already found,
    // already parsed and already folded (#409). A cold pane answers `None` to both and everything
    // below runs exactly as it did.
    let (mut opened, mut handle) = {
        let warm = warm.lock().unwrap();
        (warm.opened.clone(), warm.handle.clone())
    };
    // The conversation still has to be *sent*: this pump has shown this client nothing, and the
    // resolve below is the only thing that would have. One shot, and only while the handle it was
    // kept under is still the handle the pane is on.
    let mut inherited = opened.is_some();
    // The last session this pane was *seen* on, which is not the same as the last handle it had.
    // An agent quit and run again in the same terminal passes through a tick that names nothing
    // at all — the process is gone and there is no marker to read — so a rule comparing two
    // neighbouring handles saw `A -> none` and then `none -> B`, was certain of neither, and left
    // the first session's conversation on the screen for the reader to page and be refused.
    let mut named = handle.as_ref().and_then(|h| h.identity.announced.clone());
    let mut due = true;
    let mut misses = 0u32;
    let mut live = Watch::default();
    let mut was_working = false;
    let mut desk = ComposerFeed::default();
    let mut composer: Option<ComposerReader> = None;

    loop {
        let now = pane_of(&herd, &global, &identity, &local);
        let status = status_of(&herd, &global);
        let working = status == AgentStatus::Working;
        // A pane waiting on the operator is not a pane that has stopped. The transcript is frozen
        // for as long as it waits (#42), so its screen is the only account of the message it is
        // asking about — and withdrawing the preview there took that message off the conversation
        // at the one moment the operator needed it (#410).
        let asking = status == AgentStatus::Blocked;
        let live_now = working || asking;
        // A turn that ends without the status moving is covered by the transcript catching up,
        // but a turn the operator interrupts leaves its half-written text on the screen forever —
        // so leaving `working` withdraws whatever is showing.
        //
        // The transcript is read first, and in this order deliberately: the harness writes the
        // record and *then* goes idle, so withdrawing before that read leaves the client with
        // neither the preview nor its replacement for as long as the follow tick takes to notice.
        if (!live_now || opened.is_none()) && live.showing() {
            if !flush(&journal, &wire, &global, &held).await {
                return;
            }
            if send_live(&wire, &global, live.stop(), &held).is_err() {
                return;
            }
        }
        // The harness moved, the directory moved, the *session inside them* moved, or the file
        // went away: whatever is open is the wrong conversation. A pane whose agent was quit and
        // restarted looks identical in every field but the process, which is why the process is
        // in the handle.
        if handle.as_ref() != Some(&now) || opened.as_deref().is_some_and(|p| !p.is_file()) {
            let elsewhere = moved(named.as_ref(), now.identity.announced.as_ref());
            if let Some(session) = now.identity.announced.clone() {
                named = Some(session);
            }
            handle = Some(now.clone());
            opened = None;
            inherited = false;
            release(&journal, &held);
            {
                let mut warm = warm.lock().unwrap();
                warm.handle = handle.clone();
                warm.forget();
            }
            // The pane named a *different* session, so what the client is holding
            // belongs to the one before it — and the replacement does not exist for
            // as long as it takes to send a first message (#259, #311). Waiting for
            // one leaves the previous conversation on the screen taking no new turns,
            // which is the panel that will not update (#260).
            if elsewhere && !retire(&wire, &global, &held) {
                return;
            }
            due = true;
            misses = 0;
            composer = journals.composer(now.agent.as_deref());
        }

        // **An inherited transcript is only this pane's if the pane still resolves to it** — and
        // that is only in doubt where the handle cannot answer. A harness that names its session
        // has already been compared above: `/clear` mints a new id and rewrites the pane's marker
        // in place before the next prompt is submitted (#393), so the handle moved and took the
        // transcript with it. A harness that names nothing is resolved by *working directory*,
        // where a new transcript leaves the handle identical — and serving the old one would be
        // the conversation showing one session while the terminal beside it shows another.
        //
        // Asked only there, because asking is not free: re-deriving from a directory reads the
        // head and the tail of every candidate in it, measured at **290 ms** against the 55 ms the
        // whole warm re-open otherwise costs (#412).
        if inherited && now.identity.announced.is_none() && opened != located(&journals, &now) {
            inherited = false;
            opened = None;
            release(&journal, &held);
            warm.lock().unwrap().forget();
            due = true;
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
            match resolve(&journals, &state_dir, &now) {
                None => misses += 1,
                Some(fresh) => {
                    misses = 0;
                    let path = fresh.path().to_path_buf();
                    // Which transcript a pane is being served, and on the strength of which
                    // handle. Nothing recorded this: a pane served the *wrong* conversation left
                    // no trace at all, so an operator who saw a stale panel could only argue
                    // about it from file timestamps afterwards and never establish what the node
                    // actually opened. The audit records that a pane was watched and not what
                    // answered — the #233 shape, where every surface looks healthy and one of the
                    // answers behind them is wrong.
                    tracing::info!(
                        pane = %global,
                        transcript = %path.display(),
                        handle = if now.identity.announced.is_some() { "session" } else { "cwd" },
                        "conversation opened",
                    );
                    opened = Some(path.clone());
                    inherited = false;
                    *journal.lock().unwrap() = Some(fresh);
                    warm.lock().unwrap().opened = opened.clone();
                    if !deliver(&journal, &wire, &global, &held, &path).await
                        || !publish_facets(
                            &warm,
                            &journals,
                            &wire,
                            &global,
                            &path,
                            now.agent.as_deref(),
                            describe(&local),
                        )
                        .await
                    {
                        return;
                    }
                }
            }
        }

        // A pump that inherited an open transcript has done none of the above, so nothing has put
        // the conversation on this client's socket. The reader gets it now — off a parse and a
        // fold that already exist, which is the whole of what warmth buys.
        if std::mem::take(&mut inherited)
            && let Some(path) = opened.clone()
            && (!deliver(&journal, &wire, &global, &held, &path).await
                || !publish_facets(
                    &warm,
                    &journals,
                    &wire,
                    &global,
                    &path,
                    now.agent.as_deref(),
                    describe(&local),
                )
                .await)
        {
            return;
        }

        tokio::select! {
            _ = follow.tick() => {
                // The launched conversation the reader has open, on the same tick and by the same
                // rule as the pane's own: whatever it has grown by, under its own name so nothing
                // files a subagent's words as the parent's.
                match drain_sub(&followed).await {
                    Some((id, turns)) if !turns.is_empty() => {
                        if !wire.send(&ServerMsg::ConvoTurn {
                            pane: global.clone(),
                            sub: Some(id),
                            turns,
                        }) {
                            return;
                        }
                    }
                    _ => {}
                }
                match drain(&journal).await {
                    Ok(turns) if !turns.is_empty() => {
                        holding(&held, &turns);
                        let revised = ServerMsg::ConvoTurn { pane: global.clone(), sub: None, turns };
                        if !wire.send(&revised) {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        debug!(pane = %global, error = %e, "transcript unreadable; re-deriving");
                        opened = None;
                        release(&journal, &held);
                        warm.lock().unwrap().forget();
                        due = true;
                    }
                }
                // What the harness wrote down beside the turns moves while the turns do: a prompt
                // the operator queues mid-turn is a record like any other, and it is one the
                // client cannot see any other way until the harness gets round to it. The fold
                // reads what the transcript has grown by, and answers `None` when nothing it
                // carries has moved — a `convo.facets` every 400 ms per pane would be a frame for
                // nothing.
                if let Some(path) = opened.clone()
                    && let Some(moved) = refold(&warm, &path, describe(&local)).await
                    && !wire.send(&ServerMsg::ConvoFacets { pane: global.clone(), facets: moved })
                {
                    return;
                }
                // What the operator has left in the pane's own composer. Read off the grid the
                // client is already streaming, so it costs a walk of the rows and no I/O at all;
                // published only when it moves, so a composer nobody is typing into is free. On
                // this tick rather than the live preview's, because a half-typed line is most
                // interesting when the pane is *idle* and the preview only runs while it works.
                if opened.is_some()
                    && !wire.outbox().congested()
                    && let Some(moved) = desk.moved(desk_line(&panes, &local, composer))
                    && !wire.send(&ServerMsg::ConvoComposer {
                        pane: global.clone(),
                        text: moved.as_ref().map(|c| c.text.clone()),
                        clear: moved.and_then(|c| c.clear).map(str::to_string),
                    })
                {
                    return;
                }
            }
            // A preview is the one thing on this socket that can be dropped without loss: the
            // record behind it is still coming, and a client that is already behind does not want
            // a fifth revision of a message it has not drawn yet.
            _ = live_poll.tick(), if live_now && opened.is_some() && !wire.outbox().congested() => {
                let change = match panes.screen(&local) {
                    Some(screen) => {
                        let borrowed: Vec<&str> = screen.rows.iter().map(String::as_str).collect();
                        let seen = journal.lock().unwrap().as_ref().and_then(|j| j.preview(&borrowed));
                        live.observe(seen, asking)
                    }
                    None => live.stop(),
                };
                if send_live(&wire, &global, change, &held).is_err() {
                    return;
                }
            }
            _ = retry.tick(), if opened.is_none() && misses < FAST_RETRIES => due = true,
            _ = recheck.tick() => {
                let latest = resolve(&journals, &state_dir, &now);
                if latest.as_deref().map(Journal::path) != opened.as_deref() {
                    opened = None;
                    release(&journal, &held);
                    warm.lock().unwrap().forget();
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

/// Puts the open transcript on this client's socket: whatever it has grown by since the last read,
/// and then the conversation itself.
///
/// One path, because a pump that had to find and parse the file and a pump that inherited one
/// already parsed differ in what it *cost* and in nothing about what the reader is owed. A warm
/// pump that skipped this would sit on a socket that had been sent no conversation at all.
async fn deliver(journal: &Open, wire: &Wire, pane: &str, held: &Held, transcript: &Path) -> bool {
    // What the client is holding *of this transcript*, when this node knows. A page for a
    // transcript it is already showing merges into it; every other page replaces it. Read before
    // [`withdraw`], which takes the record.
    let showing_already = held
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(path, _)| path == transcript)
        .map(|(_, ids)| ids.clone());
    if !withdraw(wire, pane, held, transcript) {
        return false;
    }
    // The first read is the page the client is about to be sent, so it must not also arrive behind
    // it as a revision. On a warm pump it is the handful of records written while nobody was
    // watching; on a cold one it is the whole file.
    let _ = drain(journal).await;
    match reopened(journal, pane, showing_already.as_deref()) {
        Some(first) if wire.send(&first) => {
            // The other half of the same blind spot: which shape went out, and how much of it. A
            // page and a revision are the difference between a client drawing the conversation and
            // a client merging into one it is assumed to already have, and a pane reported as
            // showing nothing has no other way to be told apart from a pane sent nothing.
            let (kind, count) = match &first {
                ServerMsg::Convo { turns, .. } => ("page", turns.len()),
                ServerMsg::ConvoTurn { turns, .. } => ("revision", turns.len()),
                _ => ("other", 0),
            };
            tracing::info!(pane = %pane, kind, turns = count, "conversation delivered");
        }
        _ => return false,
    }
    *held.lock().unwrap() = showing(journal);
    true
}

/// The facets that go with the conversation just delivered.
///
/// Off the executor: the fold's first read is the whole transcript (154 ms for 29.4 MB), which is
/// a cost a conversation opening can carry and a poll cannot. The fold is kept — across a
/// re-watch as well as across a tick (#409) — so every read after the first costs the records the
/// transcript has grown by.
///
/// A fold that has not moved answers nothing, which is the right answer to the client that has
/// been following it and the wrong one to a client that has just arrived: it has never seen these
/// facets. So an opening publishes what the fold last had when it has nothing newer. A harness
/// with nothing to say sends `{}` and the client draws nothing, so there is no case to suppress.
async fn publish_facets(
    warm: &Warmth,
    journals: &Journals,
    wire: &Wire,
    pane: &str,
    transcript: &Path,
    agent: Option<&str>,
    marker: Option<SessionMarker>,
) -> bool {
    // Absent on a cold open, and absent on a pump aborted while the fold was out on its blocking
    // read — which is the one way warmth can lose it, and a pane that then published no facets for
    // the rest of its life would be a worse bug than the one this fixes.
    let cold = {
        let mut warm = warm.lock().unwrap();
        match warm.facets.is_none() {
            true => {
                warm.facets = Some(journals.fold(agent));
                true
            }
            false => false,
        }
    };
    let opening = match refold(warm, transcript, marker).await {
        Some(moved) => moved,
        None if cold => Facets::default(),
        None => warm
            .lock()
            .unwrap()
            .facets
            .as_ref()
            .map(FacetFeed::last)
            .unwrap_or_default(),
    };
    wire.send(&ServerMsg::ConvoFacets {
        pane: pane.to_string(),
        facets: opening,
    })
}

/// Sends whatever the transcript has grown by, if anything. A read that fails is left to the
/// follow tick, which re-derives the transcript rather than dropping a turn.
async fn flush(journal: &Open, wire: &Wire, pane: &str, held: &Held) -> bool {
    match drain(journal).await {
        Ok(turns) if !turns.is_empty() => {
            holding(held, &turns);
            wire.send(&ServerMsg::ConvoTurn {
                pane: pane.to_string(),
                sub: None,
                turns,
            })
        }
        _ => true,
    }
}

/// A live turn is a *revision* like any other, which is what lets it be withdrawn: the same id
/// with no blocks, and a client that matches by id and replaces is rid of it.
fn send_live(wire: &Wire, pane: &str, change: Change, held: &Held) -> Result<(), ()> {
    let turns = match change {
        Change::Held => return Ok(()),
        Change::Show(turn) => vec![turn],
        Change::Retire => vec![kampr_journal::retired()],
    };
    holding(held, &turns);
    match wire.send(&ServerMsg::ConvoTurn {
        pane: pane.to_string(),
        sub: None,
        turns,
    }) {
        true => Ok(()),
        false => Err(()),
    }
}

/// What the pane's composer holds now, for a harness whose composer has been measured.
///
/// The keystroke that clears it rides back with the text rather than being looked up by the
/// client, because it is a per-harness *measurement* and the node is where measurements live — a
/// phone already installed cannot be corrected when a harness changes its mind about what empties
/// a box, and the three harnesses served here do not agree on it in the first place.
fn desk_line(panes: &PaneRegistry, local: &str, reader: Option<ComposerReader>) -> Option<Composed> {
    let reader = reader?;
    let screen = panes.screen(local)?;
    let rows: Vec<&str> = screen.rows.iter().map(String::as_str).collect();
    reader(&rows, screen.caret)
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

/// Which transcript this pane resolves to *now*, without opening it.
///
/// The same ladder [`resolve`] walks and none of the work: a marker read or a directory listing,
/// no parse and no fold. That is what makes it cheap enough to ask before trusting an inherited
/// one.
fn located(journals: &Journals, handle: &Handle) -> Option<PathBuf> {
    journals
        .locate(
            handle.agent.as_deref(),
            handle.identity.announced.as_ref(),
            handle.cwd.as_deref().map(Path::new),
            &handle.identity.harness,
        )
        .ok()
        .flatten()
}

fn resolve(journals: &Journals, state_dir: &Path, handle: &Handle) -> Option<Box<dyn Journal>> {
    let opened = journals
        .open(
            handle.agent.as_deref(),
            handle.identity.announced.as_ref(),
            handle.cwd.as_deref().map(Path::new),
            &handle.identity.harness,
        )
        .ok()
        .flatten()?;
    // Wrapped here rather than at each send: every turn this pane will ever put on the wire comes
    // out of this journal, so a paste shown on one path and not another is not expressible.
    Some(crate::pasted::Shown::over(opened, state_dir))
}

/// Lets go of the open transcript and records what the client is left holding from it.
///
/// A transcript that was never open leaves the record alone: what the client is holding is then
/// whatever an earlier one put there, and it is still on the screen.
fn release(journal: &Open, held: &Held) {
    let Some(old) = journal.lock().unwrap().take() else {
        return;
    };
    *held.lock().unwrap() = Some((old.path().to_path_buf(), old.turn_ids()));
}

/// What the client is holding from the transcript that is open *now*.
fn showing(journal: &Open) -> Option<(PathBuf, Vec<String>)> {
    let guard = journal.lock().unwrap();
    let open = guard.as_ref()?;
    Some((open.path().to_path_buf(), open.turn_ids()))
}

/// Adds to what the client is holding, for as long as it is holding the open transcript.
///
/// The record is kept current at every send rather than only when a transcript is closed, because
/// the pump is *aborted* as often as it finishes: `unwatch` and a re-watch both stop it where it
/// stands, and a record written only on the way out would never be written at all.
fn holding(held: &Held, turns: &[Turn]) {
    let mut guard = held.lock().unwrap();
    let Some((_, ids)) = guard.as_mut() else {
        return;
    };
    for turn in turns {
        if !ids.contains(&turn.id) {
            ids.push(turn.id.clone());
        }
    }
}

/// Takes the previous conversation off the client before a different one is sent.
///
/// **A page merges; it does not replace.** Turns are matched by id, and the ids of another
/// session's transcript match nothing — so a fresh page for a *different* transcript arrives
/// above the old conversation instead of in place of it, which reads exactly like the panel
/// refusing to update. Withdrawing first is the existing retirement mechanism applied to the
/// whole conversation: a turn carrying no blocks is not drawn.
fn withdraw(wire: &Wire, pane: &str, held: &Held, fresh: &Path) -> bool {
    let holding = held.lock().unwrap().take();
    match holding {
        Some((path, _)) if path == fresh => true,
        Some((_, ids)) => send_retirement(wire, pane, ids),
        None => true,
    }
}

/// The same withdrawal with no replacement to compare against: whatever the client is
/// holding, it is holding it of a conversation this pane has left.
fn retire(wire: &Wire, pane: &str, held: &Held) -> bool {
    let holding = held.lock().unwrap().take();
    match holding {
        Some((_, ids)) => send_retirement(wire, pane, ids),
        None => true,
    }
}

fn send_retirement(wire: &Wire, pane: &str, ids: Vec<String>) -> bool {
    if ids.is_empty() {
        return true;
    }
    let turns = ids
        .into_iter()
        .map(|id| Turn::new(id, Role::Assistant, None))
        .collect();
    wire.send(&ServerMsg::ConvoTurn {
        pane: pane.to_string(),
        sub: None,
        turns,
    })
}

/// Whether the pane named a *different* session, rather than merely stopped naming the
/// one it had.
///
/// A pane's agent goes absent for real, and more than once: herdr holds `unknown` for
/// **3.3 s** after a label attaches with nothing matching, and hands out one `idle` on
/// the way down when an agent exits (#360). Either leaves the pane with no identity
/// for a tick, and a conversation withdrawn on every one of those is its own defect —
/// a worse one than a dated view. Two names that *disagree* is the case a node is
/// certain about, and the only one worth acting on.
///
/// So `was` is the last name the pane was seen under and not the name it had a tick ago: an
/// absence between the two is exactly the state a restarted agent passes through, and comparing
/// across it is the difference between `/clear` and Ctrl-C followed by `claude`.
fn moved(was: Option<&SessionRef>, now: Option<&SessionRef>) -> bool {
    matches!((was, now), (Some(was), Some(now)) if was != now)
}

/// Whatever the followed conversation has grown by, and which one it was.
///
/// A read that fails takes the follow down rather than retrying: unlike the pane's own transcript
/// there is nothing to re-derive it from — the reader asked for one file by name, and if it has
/// gone the honest answer is to stop rather than to guess at another.
async fn drain_sub(followed: &Followed) -> Option<(String, Vec<Turn>)> {
    let followed = followed.clone();
    let read = tokio::task::spawn_blocking(move || {
        let mut guard = followed.lock().unwrap();
        let (id, journal) = guard.as_mut()?;
        match journal.poll() {
            Ok(turns) => Some((id.clone(), turns)),
            Err(_) => {
                *guard = None;
                None
            }
        }
    })
    .await;
    read.ok().flatten()
}

/// Whatever the transcript has grown by, folded onto the facets already collected off it, and
/// `None` when none of them moved.
///
/// **Off the executor like the tail beside it**, and for a stronger reason than the tail has: the
/// read is normally the handful of records since the last tick, but the fold resets and reads the
/// file whole whenever the transcript it is on shrinks under it — and it opens and stats a file
/// either way, which is not work a tokio worker should be holding.
///
/// A fold whose blocking task did not come back is dropped rather than replaced with a fresh one
/// that would silently re-send everything: the next resolve builds one, and until then this pane
/// publishes no facets rather than the wrong ones.
async fn refold(warm: &Warmth, transcript: &Path, marker: Option<SessionMarker>) -> Option<Facets> {
    let mut held = warm.lock().unwrap().facets.take()?;
    let transcript = transcript.to_path_buf();
    let (held, moved) = tokio::task::spawn_blocking(move || {
        let moved = held.moved(&transcript, marker.as_ref());
        (held, moved)
    })
    .await
    .ok()?;
    warm.lock().unwrap().facets = Some(held);
    moved
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

#[cfg(test)]
mod tests {
    use super::*;

    fn on(session: &str) -> SessionRef {
        SessionRef::id("claude", session)
    }

    /// A pane's agent goes absent and comes back — herdr's 3.3 s of `unknown` after a
    /// label attaches, and the single `idle` it emits as one exits (#360). The pane has
    /// not gone anywhere, and a conversation that empties itself while nothing is wrong
    /// is its own defect.
    #[test]
    fn a_pane_that_stops_naming_its_session_keeps_the_conversation_it_had() {
        let named = on("a");
        assert!(!moved(Some(&named), None));
        assert!(!moved(None, Some(&named)));
        assert!(!moved(Some(&named), Some(&named)));
    }

    /// Two names that disagree is the case this node is certain about: whatever the
    /// client is holding, it is not of the session the pane is on now.
    #[test]
    fn a_pane_that_names_a_different_session_has_left_the_one_on_the_screen() {
        assert!(moved(Some(&on("a")), Some(&on("b"))));
    }

    /// The restart, which is the shape the last-seen name exists for: quitting an agent
    /// leaves the pane naming nothing for as long as it takes to start another, so the
    /// two sessions either side of that gap are never neighbours.
    #[test]
    fn an_agent_quit_and_run_again_is_a_different_session_across_the_gap() {
        assert!(!moved(Some(&on("a")), None));
        assert!(moved(Some(&on("a")), Some(&on("b"))));
    }
}
