use super::store::{Owner, Store};
use super::{Resume, read_token};
use kampr_core::resume::{Action, Freshness, Token, Verdict, read};
use kampr_keep::SessionDir;
use kampr_proc::Table as _;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Between the owner ending and its record going: a keeper that is going down takes its child with
/// it, and that is not the program deciding it needs no resuming.
const GRACE: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_millis(500);
const KEEPER_WAIT: Duration = Duration::from_secs(2);

/// Reads one session's `OSC 7577` reports against its token and keeps what they accept.
pub(crate) struct Intake {
    store: Store,
    session: SessionDir,
    token: Token,
    label: Option<String>,
    stopped: AtomicBool,
}

impl Intake {
    /// `None` for a session with no token: resume is off, the node is root, the session is a
    /// Docker shell, or it predates tokens.
    pub fn open(resume: Option<&Resume>, session: &SessionDir, known: Option<Token>) -> Option<Arc<Self>> {
        let resume = resume.filter(|r| r.active())?;
        let token = known.or_else(|| read_token(session.path()))?;
        let intake = Arc::new(Self {
            store: Store::new(resume.dir.clone()),
            session: session.clone(),
            token,
            label: session.spec().ok().and_then(|spec| spec.label),
            stopped: AtomicBool::new(false),
        });
        intake.reconcile();
        Some(intake)
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// `replayed` is a report read again after a restart: it counts only past what was stored, and
    /// its age says nothing.
    pub fn report(self: &Arc<Self>, body: &[u8], offset: u64, replayed: bool) {
        let session = self.session.id();
        if offset <= self.store.read(session).offset {
            return;
        }
        let freshness = if replayed {
            Freshness::Replayed
        } else {
            Freshness::Live { now: now() }
        };
        let verdict = read(body, &self.token, freshness);
        let watch = self
            .store
            .update(session, |file| match verdict {
                Verdict::Ignored => None,
                Verdict::Rejected { pid } => {
                    file.owners.retain(|o| o.pid != pid);
                    file.offset = offset;
                    None
                }
                Verdict::Accepted(report) => {
                    let start = self.owns(report.pid)?;
                    let known = file
                        .owners
                        .iter()
                        .position(|o| o.pid == report.pid && o.start == start);
                    if known.is_some_and(|at| file.owners[at].time > report.time) {
                        return None;
                    }
                    file.owners.retain(|o| o.pid != report.pid);
                    file.offset = offset;
                    file.label = self.label.clone();
                    match report.action {
                        Action::Clear => None,
                        Action::Set(command) => {
                            file.owners.push(Owner {
                                pid: report.pid,
                                start: start.clone(),
                                time: report.time,
                                command,
                                at: offset,
                                tried: false,
                            });
                            Some((report.pid, start))
                        }
                    }
                }
            })
            .ok()
            .flatten();
        if let Some((pid, start)) = watch {
            self.watch(pid, start);
        }
    }

    /// The owner's start token when `pid` is alive and the session's own child or below it. Anything
    /// else may have been named by a report that was not written by the session's program.
    fn owns(&self, pid: u32) -> Option<Option<String>> {
        let child = self.child()?;
        let table = kampr_proc::here();
        let start = table.start(pid)?;
        let mut seen = HashSet::from([child]);
        let mut queue = vec![child];
        while let Some(at) = queue.pop() {
            if at == pid {
                return Some(Some(start.token));
            }
            for next in table.children(at) {
                if seen.insert(next) {
                    queue.push(next);
                }
            }
        }
        None
    }

    /// `keeper.json` is written once the child is running, so a child that reports at once can be
    /// read before it names the child.
    fn child(&self) -> Option<u32> {
        let until = std::time::Instant::now() + KEEPER_WAIT;
        loop {
            if let Some(keeper) = self.session.keeper() {
                return Some(keeper.child);
            }
            if std::time::Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A node that was away while an owner ended, or that starts beside one still running.
    fn reconcile(self: &Arc<Self>) {
        let running = self
            .store
            .update(self.session.id(), |file| {
                let mut running = Vec::new();
                file.owners.retain(|o| {
                    let alive = lives(o.pid, &o.start);
                    if alive {
                        running.push((o.pid, o.start.clone()));
                    }
                    alive
                });
                running
            })
            .unwrap_or_default();
        for (pid, start) in running {
            self.watch(pid, start);
        }
    }

    fn watch(self: &Arc<Self>, pid: u32, start: Option<String>) {
        let intake = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("kampr-resume-{}", self.session.id()))
            .spawn(move || {
                loop {
                    while lives(pid, &start) {
                        if intake.stopped.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(POLL);
                    }
                    std::thread::sleep(GRACE);
                    if !lives(pid, &start) {
                        break;
                    }
                }
                if intake.stopped.load(Ordering::Relaxed)
                    || intake.session.exit().is_some()
                    || !intake.session.alive()
                {
                    return;
                }
                let _ = intake.store.update(intake.session.id(), |file| {
                    file.owners.retain(|o| !(o.pid == pid && o.start == start))
                });
            });
        if let Err(e) = spawned {
            tracing::debug!(session = self.session.id(), error = %e, "watching a resume owner");
        }
    }
}

/// A process mid-`exec` reads with an empty command line for a moment, so one look is no proof
/// it has gone; the watcher looks again after [`GRACE`].
fn lives(pid: u32, start: &Option<String>) -> bool {
    let table = kampr_proc::here();
    table.argv(pid).is_some() && table.start(pid).map(|s| s.token) == *start
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
