mod facet;
mod record;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::adapter::{JournalAdapter, SessionKind, SessionRef};
use crate::attach::Fetched;
use crate::composer::{Caret, Composed, ComposerReader};
use crate::discover;
use crate::envelope::push_text;
use crate::error::JournalError;
use crate::facet::{FacetFold, Facets, Queued};
use crate::live::{LiveBlock, ScreenReader, StatusReader};
use crate::marker::SessionMarker;
use crate::model::{Attachment, Block, CodeRole, Role, ToolState, Turn, TurnKind};
use crate::output;
use crate::process::{PaneProcess, Started};
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::sub::{self, SubRef};
use crate::summary::{clip, count_lines, image_marker, one_line, summarise};
use crate::tail::{FileJournal, Journal, TranscriptParser};

use record::{Content, Record, Spawn};

/// The name herdr publishes for a pane running this harness, and therefore the key a pane's
/// conversation is looked up by. herdr labels a pane with its foreground process's name and
/// nothing else (#75), and `omp` runs as `bun` with `argv[0]` rewritten — `/proc/<pid>/comm`
/// reads `omp`, so that is the label.
pub const AGENT: &str = "omp";

/// `~/.omp/agent`, the directory the harness calls its agent dir. `sessions`, `blobs`,
/// `terminal-sessions` and the rest hang off it.
///
/// **Two ways an install moves it, and neither is guessable from here.** `PI_CODING_AGENT_DIR`
/// and `--profile <name>` (`~/.omp/profiles/<name>/agent`) both relocate the whole tree, and on
/// Linux an `XDG_DATA_HOME` set for the *agent's* process moves `sessions` to
/// `$XDG_DATA_HOME/omp/sessions`. A node reads its own environment, not the pane's, so it cannot
/// know which applies — a session under a moved root simply resolves to nothing rather than to
/// somebody else's conversation.
pub const HOME: &str = ".omp/agent";

/// The harness omp forked, under a home of its own.
///
/// **It shares the record grammar and the session path and almost nothing else** ([#490](#)).
/// `pi` 0.73.1 appends with `appendFileSync` and holds no descriptor, writes no breadcrumbs and
/// puts no run state in its terminal title — so none of omp's handles above the working
/// directory apply to it, and the two it does have are its own. Its shell tool puts the session
/// file it is on into the environment of every command it runs, and the subagents extension puts
/// the root session's id into the root process itself, where every descendant inherits it without
/// overwriting ([#542](#), [#543](#)) — which is what [`Self::named`] reads. On the screen, a turn
/// in flight is the TUI's spinner in the composer's top border, and nothing else paints it
/// ([#544](#), [#545](#), [#546](#)).
pub const PI_AGENT: &str = "pi";
pub const PI_HOME: &str = ".pi/agent";

const SESSIONS: &str = "sessions";

/// `terminal-sessions/<terminal-id>`: the working directory, the session file, and `fresh` when
/// that file has not been written yet. The id is the tty with `/dev/` cut off and `/` replaced by
/// `-` (`/dev/pts/3` → `pts-3`), falling back to a multiplexer's own pane variable when stdin has
/// no tty — which a pane always does.
const TERMINALS: &str = "terminal-sessions";

pub struct OmpAdapter {
    agent: String,
    root: TranscriptRoot,
}

/// A session file and what the harness recorded beside it, however it was found.
struct Found {
    transcript: Option<PathBuf>,
    session: String,
    cwd: Option<PathBuf>,
}

impl OmpAdapter {
    pub fn new(root: TranscriptRoot) -> Self {
        Self::named(AGENT, root)
    }

    pub fn named(agent: &str, root: TranscriptRoot) -> Self {
        Self {
            agent: agent.to_string(),
            root,
        }
    }

    fn sessions(&self) -> PathBuf {
        self.root.path().join(SESSIONS)
    }

    /// `sessions/<encoded-cwd>/<timestamp>_<id>.jsonl`. The bucket is derived from the working
    /// directory, which a pane does not always agree about, so an id is found by scanning them.
    fn find_by_id(&self, id: &str) -> Result<PathBuf, JournalError> {
        self.root.check_id(id)?;
        let suffix = format!("_{id}.jsonl");
        for bucket in discover::subdirectories(&self.sessions()) {
            let Ok(entries) = std::fs::read_dir(&bucket) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(&suffix))
                {
                    return Ok(path);
                }
            }
        }
        Err(JournalError::NotFound(id.to_string()))
    }

    /// The session file this pid is writing, from its own open file descriptors.
    ///
    /// **The strongest handle any harness this crate serves publishes.** omp opens its session
    /// JSONL once and keeps the descriptor for the life of the session
    /// (`session-storage.ts`: *"Open file once, keep fd for lifetime"*), so `/proc/<pid>/fd` is a
    /// kernel-held answer that cannot be stale: it moves with `/new`, it is gone when the process
    /// is, and nothing has to be validated against a recorded start time the way a written marker
    /// does. Measured on omp 18.1.10 in a herdr pane ([#481](#)).
    ///
    /// A subagent's transcript is open on the same descriptor table and is deliberately not
    /// this: it sits one level deeper, inside the directory named after the session file, and the
    /// parent goes on holding it after that agent has finished ([#481](#)).
    fn held(&self, pid: u32) -> Option<PathBuf> {
        let sessions = self.sessions();
        let mut newest: Option<(SystemTime, PathBuf)> = None;
        for entry in std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten() {
            let Ok(path) = std::fs::read_link(entry.path()) else {
                continue;
            };
            if !is_session_file(&sessions, &path) {
                continue;
            }
            let at = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if newest.as_ref().is_none_or(|(seen, _)| at > *seen) {
                newest = Some((at, path));
            }
        }
        newest.map(|(_, path)| path)
    }

    /// The breadcrumb omp writes for the terminal this pid is on: the session it opened, before
    /// there is a file for [`Self::held`] to find.
    ///
    /// A session is memory-only until its first assistant message, so between opening a pane and
    /// the model's first answer there is no session file at all — the same gap that made Kampr
    /// serve the pane next door for Claude (#260, #311). The breadcrumb is written at that
    /// moment, and carries a third line, `fresh`, while the file is still unwritten ([#482](#)).
    ///
    /// **Keyed by tty, so it outlives the process that wrote it**, and pts numbers are reused.
    /// The guard is the breadcrumb's own mtime: it is written when the session opens, so one
    /// older than this process is a previous omp's, and following it would be the mistake this
    /// whole ladder exists to stop ([#482](#)).
    fn crumb(&self, process: &PaneProcess) -> Option<Found> {
        // **A breadcrumb is keyed by terminal, and a terminal outlives the program that wrote
        // one.** Nothing removes a crumb when omp exits (#482), so the file goes on naming a
        // session for every process that ever runs on that tty afterwards — the pane's own login
        // shell included, whose start time is older than the crumb and therefore passes the guard
        // below. `Registry::marker` asks *every* adapter about *every* process in the pipeline, so
        // without this an omp that has been quit answers for the `claude` started in its place.
        // The process's own name is what settles it, read from procfs rather than from herdr.
        if !self.is_the_harness(process.pid) {
            return None;
        }
        let path = self.root.path().join(TERMINALS).join(tty_id(process.pid)?);
        self.crumb_at(&path, process.started)
    }

    /// Whether this pid is running *this* harness, by the name procfs gives it.
    fn is_the_harness(&self, pid: u32) -> bool {
        crate::process::comm(pid).as_deref() == Some(self.agent.as_str())
    }

    fn crumb_at(&self, path: &Path, started: Started) -> Option<Found> {
        let written = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        if started.at().is_some_and(|started| written < started) {
            return None;
        }
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        let cwd = lines.next()?;
        let named = lines.next()?;
        Some(Found {
            session: session_id(Path::new(named))?,
            cwd: Some(PathBuf::from(cwd)),
            // `contain` canonicalises, so a session whose file has not been written yet resolves
            // to nothing here — which is this pane's state rather than a failure to find it.
            transcript: self.root.contain(named).ok(),
        })
    }

    fn found(&self, process: &PaneProcess) -> Option<Found> {
        if let Some(transcript) = self.held(process.pid) {
            return Some(Found {
                session: session_id(&transcript)?,
                cwd: declared_cwd(&transcript),
                transcript: Some(transcript),
            });
        }
        self.crumb(process)
    }

    /// What the kernel still holds of a process's environment, for the two variables pi's own
    /// code puts there.
    ///
    /// `/proc/<pid>/environ` is the environment as the kernel keeps it, and it moves with the
    /// process: a variable the running process sets appears in it, and the whole answer is gone
    /// when the process is. Neither can stale the way a file on disk can, which is the property
    /// the held descriptor has and nothing pi writes down does ([#542](#)).
    fn session_env(pid: u32) -> Option<(Option<String>, Option<String>)> {
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        let mut file = None;
        let mut parent = None;
        for entry in env.split(|b| *b == 0) {
            let Ok(text) = std::str::from_utf8(entry) else {
                continue;
            };
            let Some((key, value)) = text.split_once('=') else {
                continue;
            };
            match key {
                "PI_SESSION_FILE" => file = Some(value.to_string()),
                "PI_SUBAGENT_PARENT_SESSION" => parent = Some(value.to_string()),
                _ => {}
            }
        }
        Some((file, parent))
    }

    /// The session the pane's processes name in their own environment, and the process that
    /// named it.
    ///
    /// **The pipeline, not the name.** The pane's own pi carries the root session's id once the
    /// subagents extension has run, its tool children carry the session file, and a subagent's
    /// children carry both — the file that is the subagent's own and the id that is the pane's.
    /// A process is therefore asked in pipeline order, and the answer it gives is taken by kind
    /// rather than by which process gave it: the id a child names for its parent is the pane's
    /// session whatever the child is on, and a session file a child names for itself is the
    /// pane's session unless the child is a subagent's, in which case it names the parent's id
    /// too and the id is what is taken ([#543](#)).
    fn environment<'a>(&self, pipeline: &'a [PaneProcess]) -> Option<(Found, &'a PaneProcess)> {
        let sessions = self.sessions();
        let mut parent: Option<(String, &PaneProcess)> = None;
        for process in pipeline {
            let Some((file, parent_id)) = Self::session_env(process.pid) else {
                continue;
            };
            if let Some(id) = parent_id {
                if parent.is_none() {
                    parent = Some((id, process));
                }
                continue;
            }
            let Some(file) = file else {
                continue;
            };
            let path = PathBuf::from(file.as_str());
            if is_session_file(&sessions, &path) {
                return Some((
                    Found {
                        session: session_id(&path)?,
                        cwd: declared_cwd(&path),
                        transcript: self.root.contain(&file).ok(),
                    },
                    process,
                ));
            }
        }
        let (id, process) = parent?;
        let path = self.find_by_id(&id).ok()?;
        Some((
            Found {
                session: id,
                cwd: declared_cwd(&path),
                transcript: Some(path),
            },
            process,
        ))
    }
}

impl JournalAdapter for OmpAdapter {
    fn agent(&self) -> &str {
        &self.agent
    }

    fn root(&self) -> &TranscriptRoot {
        &self.root
    }

    fn locate(&self, session: &SessionRef) -> Result<PathBuf, JournalError> {
        match session.kind {
            SessionKind::Id => self.find_by_id(&session.value),
            SessionKind::Path => self.root.contain(&session.value),
        }
    }

    fn locate_by_process(&self, process: &PaneProcess) -> Result<PathBuf, JournalError> {
        match self.found(process) {
            Some(Found {
                transcript: Some(path),
                ..
            }) => Ok(path),
            Some(Found { session, .. }) => Err(JournalError::Unwritten(session)),
            None => Err(JournalError::NotFound(process.pid.to_string())),
        }
    }

    /// Both handles are the pane's *own* process, so a pipeline is walked rather than searched:
    /// the first pid in it that is writing an omp session, or that opened one on this tty, is the
    /// harness. A pane herdr can only describe as `bash` is identified exactly by either.
    ///
    /// For pi the handle is the environment its processes carry, and the walk is the same shape:
    /// the first answer in pipeline order is the pane's session, and a hit skips the held and
    /// crumb walks pi never pays ([#542](#), [#543](#)).
    fn marker(&self, pipeline: &[PaneProcess]) -> Option<SessionMarker> {
        if self.agent == PI_AGENT {
            let (found, process) = self.environment(pipeline)?;
            return Some(SessionMarker {
                agent: self.agent.clone(),
                pid: process.pid,
                session: found.session,
                cwd: found.cwd,
                name: None,
                name_source: None,
                // Nothing pi writes to disk says what it is doing, and its terminal title carries
                // no run state either ([#490](#)): the session file grows only when a message
                // completes, so a tail cannot tell a model that is thinking from one that is
                // blocked on a person. What the screen says is read where the screen is —
                // `pi_status`.
                status: None,
                transcript: found.transcript,
                started: process.started,
            });
        }
        pipeline.iter().find_map(|process| {
            let found = self.found(process)?;
            Some(SessionMarker {
                agent: self.agent.clone(),
                pid: process.pid,
                session: found.session,
                cwd: found.cwd,
                name: None,
                name_source: None,
                // Nothing omp writes to disk says what it is doing: the session file grows only
                // when a message completes, and `tool_execution_start` is written before the
                // approval dialog rather than after it, so a tail cannot tell a model that is
                // thinking from one that is blocked on a person. What omp does publish is its
                // terminal title, and that is read where the title is — see `title_status`.
                status: None,
                transcript: found.transcript,
                started: process.started,
            })
        })
    }

    /// The bucket omp files a directory's sessions in — a hint only, because every candidate is
    /// checked against the `cwd` its own header declares.
    fn locate_by_cwd(&self, cwd: &Path, since: Option<SystemTime>) -> Result<PathBuf, JournalError> {
        let sessions = self.sessions();
        let declared = |record: &Value| {
            if record.get("type").and_then(Value::as_str) != Some("session") {
                return None;
            }
            record.get("cwd").and_then(Value::as_str).map(str::to_string)
        };
        let named = sessions.join(bucket(self.home(), &std::env::temp_dir(), cwd));
        if named.is_dir()
            && let Some(found) = discover::newest_declaring(
                discover::jsonl_files(&named),
                cwd,
                since,
                discover::Silent::Belongs,
                declared,
            )
        {
            return Ok(found);
        }
        let everything = discover::subdirectories(&sessions)
            .iter()
            .flat_map(|dir| discover::jsonl_files(dir))
            .collect();
        discover::newest_declaring(everything, cwd, since, discover::Silent::Refuse, declared)
            .ok_or_else(|| discover::not_found(cwd))
    }

    fn parser(&self) -> Box<dyn TranscriptParser> {
        Box::new(OmpParser::default())
    }

    /// A launched conversation is filed relative to the launching one, so the parser is told where
    /// on disk it is reading rather than only which agent and which relative path.
    fn open_path(&self, path: PathBuf) -> Box<dyn Journal> {
        let mut parser = OmpParser {
            filed: Some(Filed {
                agent: self.agent.clone(),
                root: self.root.clone(),
                transcript: path.clone(),
            }),
            ..OmpParser::default()
        };
        parser.set_origin(crate::attach::Origin::new(&self.agent, &self.root, &path));
        Box::new(FileJournal::new(path, Box::new(parser), self.screen()))
    }

    fn facets(&self, transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
        facet::collect(transcript, marker)
    }

    fn fold(&self) -> Option<Box<dyn FacetFold>> {
        Some(Box::new(facet::Fold::default()))
    }

    fn screen(&self) -> Option<ScreenReader> {
        Some(if self.agent == PI_AGENT { pi_live } else { live })
    }

    fn composer(&self) -> Option<ComposerReader> {
        Some(composer)
    }

    fn queued(&self) -> Option<crate::facet::QueuedReader> {
        Some(if self.agent == PI_AGENT { pi_queued } else { queued })
    }

    fn status(&self) -> Option<StatusReader> {
        (self.agent == PI_AGENT).then_some(pi_status)
    }

    fn sticky(&self) -> bool {
        self.agent == PI_AGENT
    }

    fn attachment(&self, record: &str, index: u32) -> Result<Fetched, JournalError> {
        let refuse = || JournalError::NotFound(index.to_string());
        let Record::Message(entry) = serde_json::from_str(record).map_err(|_| refuse())? else {
            return Err(refuse());
        };
        crate::attach::nth(record::attachments(&entry), index)
    }
}

/// The composer's own row opens with this and nothing else does *below* it — a tool card's bottom
/// border is the same two characters, so the marker alone is not enough and the composer is taken
/// as the last row that carries it.
const COMPOSER: &str = "╰─";

/// The column the operator's first character lands in, and the indent every wrapped row of the
/// composer carries: `╰─ ` is three cells. Measured off the caret, which read col 3 on an empty
/// composer, col 42 with 39 characters typed, and col 21 on the third row of a wrapped one
/// ([#496](#)).
const INPUT: usize = 3;

/// What omp paints while a turn is in flight, two columns in, and paints nothing like when idle.
const WORKING: char = '⎋';

/// The message omp is painting, lifted off the foot of the visible screen.
///
/// **omp marks nothing.** Its assistant messages and the operator's own prompts are both plain
/// text one column in, wrapped to the same column, with no glyph on either and only a blank row
/// between them — so [`crate::live::read`], which is built on a marker in column zero, cannot be
/// used here and neither can any rule that recognises a message *as* a message. Three measured
/// things make the read honest without one:
///
/// - **It only runs while omp says it is working.** `⎋ Working…` is on the screen for exactly as
///   long as a turn is in flight — through an open dialog included ([#487](#)) — and nothing else
///   paints it ([#496](#)). Without that gate the rotating
///   `Tip:` line omp draws one column in — over an idle pane, where the whole conversation is
///   already in the transcript — would read as a message being written.
/// - **The block ends at the blank row above it**, so a walk that would have reached the
///   operator's own prompt has already stopped. Its prompt is in the transcript by then anyway,
///   and [`crate::live::preview`] drops a block the transcript already carries.
/// - **Only a block that grows between two polls is published** ([`crate::live::Watch`]).
///
/// `clipped` is always true, and that is not a limitation being papered over: with no marker
/// opening a message there is nothing on the screen that says a block *starts* where the walk
/// stopped, so the redundancy check has to be `contains` rather than `starts_with`.
pub fn live(screen: &[&str]) -> Option<LiveBlock> {
    let composer = screen.iter().rposition(|line| line.starts_with(COMPOSER))?;
    let mut body: Vec<&str> = Vec::new();
    let mut working = false;
    for line in screen[..composer].iter().rev() {
        if line.trim().is_empty() {
            if body.is_empty() {
                continue;
            }
            break;
        }
        if is_status(line) {
            continue;
        }
        if line.trim_start().starts_with(WORKING) {
            working = true;
            continue;
        }
        match line.strip_prefix(' ').filter(|rest| !rest.starts_with(' ')) {
            Some(text) => body.push(text.trim_end()),
            // A tool card, a dialog, the welcome panel: column zero is a boundary, and reaching
            // one is the walk leaving whatever it was reading.
            None => break,
        }
    }
    if !working {
        return None;
    }
    let text = body
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    (!text.is_empty()).then_some(LiveBlock { text, clipped: true })
}

/// omp's status row, which sits directly above the composer and is one column in like a message.
/// It opens with the same vocabulary the terminal title does — `π` when it is the operator's
/// turn, a spinner frame while it works — which is the one thing that separates it from prose.
fn is_status(line: &str) -> bool {
    matches!(line.strip_prefix(' ').and_then(|rest| rest.chars().next()),
        Some(c) if c == 'π' || SPINNER.contains(c))
}

/// The line the operator has typed and not sent.
///
/// Its own reader rather than [`crate::composer::read`] for one measured reason: omp opens the row
/// with **two** characters, `╰─`, where the three harnesses that share that reader open with one.
/// Everything else about the read is the same shape and the same caret rule — a caret resting at
/// the input column is an empty composer whatever is painted to the right of it, which is also
/// where `ctrl+a` leaves it on a full line — measured at col 3 for both ([#496](#)).
pub fn composer(screen: &[&str], caret: Caret) -> Option<Composed> {
    let head = screen.iter().rposition(|line| line.starts_with(COMPOSER))?;
    let mut last = head;
    for (at, line) in screen.iter().enumerate().skip(head + 1) {
        let indented = line.bytes().take(INPUT).filter(|b| *b == b' ').count() == INPUT;
        if !indented || line.trim().is_empty() {
            break;
        }
        last = at;
    }
    let row = caret.row as usize;
    if row < head || row > last || (row == head && caret.col as usize <= INPUT) {
        return None;
    }
    let mut text = screen[head][COMPOSER.len()..].trim_end().to_string();
    for line in &screen[head + 1..=last] {
        text.push_str(line[INPUT..].trim_end());
    }
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(Composed {
        text,
        clear: Some(CLEAR),
    })
}

/// The prompts the operator has sent that omp has not started on yet.
///
/// **The screen is the only account of one.** omp writes a steering prompt down when it
/// *delivers* it and not before — measured at 24 s after it was typed — so a session with three
/// waiting is byte-identical on disk to one with none ([#489](#)). What it draws is a
/// ` Steering · N` header and one numbered row per prompt, closed by a hint row, and a prompt too
/// long for a row is **truncated with an ellipsis rather than wrapped** ([#496](#)) — so what is
/// published here is what the operator can see, ellipsis included, rather than a sentence
/// reassembled out of rows the harness never drew.
pub fn queued(screen: &[&str]) -> Vec<Queued> {
    let Some(head) = screen
        .iter()
        .rposition(|line| line.trim_start().starts_with(STEERING))
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for line in screen.iter().skip(head + 1) {
        let Some(text) = numbered(line, found.len() + 1) else {
            break;
        };
        found.push(Queued {
            text: text.to_string(),
            at: None,
        });
    }
    found
}

const STEERING: &str = "Steering ";

/// `   2. and tag the release too` — the row for the `nth` prompt, and nothing else.
///
/// The hint omp draws under the last prompt is at the same indent — `   └ Alt+Up/Shift+Up to edit`
/// — and is excluded by carrying no ordinal at all. Checking *which* ordinal rather than merely
/// stripping one is the same rule the numbered dialog detector runs on, and it is what stops a run
/// continuing across a gap into whatever the harness drew next.
fn numbered(line: &str, nth: usize) -> Option<&str> {
    let text = line.trim_start().strip_prefix(&format!("{nth}. "))?.trim_end();
    (!text.is_empty()).then_some(text)
}

/// One `ctrl+u` takes the whole buffer, wrapped or not: measured against a 39-character line and
/// against a wrapped one, both leaving the row `╰─` and the caret at the input column ([#496](#)).
/// `ctrl+a ctrl+k` clears it too and is not used — one keystroke is fewer than two, and `ctrl+c`
/// is never spent on a harness this crate serves.
const CLEAR: &str = "\u{15}";

impl OmpAdapter {
    /// The home the agent dir hangs off, for the bucket encoding that is written relative to it.
    fn home(&self) -> Option<&Path> {
        self.root.path().parent().and_then(Path::parent)
    }
}

/// omp's own state, out of the terminal title it writes.
///
/// `utils/title-generator.ts` composes every title as the brand, a **separator that carries
/// the run state**, and the session label: `>` when it is the operator's turn, one of ten
/// braille spinner frames while it works, and `!` when it is blocked on a person — an
/// approval dialog or an `ask`. Measured live on omp 18.1.10 through herdr's own
/// `pane.terminal_title`: `π > project` idle, `π ⠹ Running a slow command now` working,
/// `π ! Running a slow command now` at an approval prompt ([#486](#)).
///
/// **This is the only status signal omp has, and herdr publishes none.** herdr carries no
/// detection manifest for `omp`, so `agent explain` returns no rules at all and the pane
/// reports `idle` — measured continuously across a whole working turn and across an approval
/// dialog — while the title beside it says otherwise ([#485](#)).
///
/// The disabled form is `π: label`, with no space before the colon, and it is refused rather
/// than read as idle: an operator who turned `tui.titleState` off has not said the session is
/// waiting for them.
pub fn title_status(title: &str) -> Option<&'static str> {
    let separator = title.strip_prefix("π ")?.chars().next()?;
    match separator {
        '>' => Some("idle"),
        '!' => Some("waiting"),
        _ if SPINNER.contains(separator) => Some("busy"),
        _ => None,
    }
}

/// The ten frames `title-generator.ts` cycles a working title through, at 80 ms. The same ten
/// frames, on the same 80 ms cadence, are the TUI's loader and what pi's composer border spins
/// through while a turn is in flight ([#544](#)).
const SPINNER: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

/// A row the editor draws for its borders: a run of `─` opening in column zero, at least 12 of
/// them. Conversation, prompts and cards all sit one column in, so nothing else on a pi screen
/// opens there — which is what lets the reader find the footer without knowing where it starts.
fn border_row(line: &str) -> bool {
    line.starts_with('─') && line.chars().filter(|c| *c == '─').count() >= 12
}

/// Whether a row carries one of the loader's frames.
fn has_frame(line: &str) -> bool {
    line.chars().any(|c| SPINNER.contains(c))
}

/// What pi's screen says the session is doing, for the pane the screen belongs to.
///
/// **The screen is the only state signal pi has.** herdr carries no detection manifest for it,
/// so a pane reports `idle` through a whole working turn — the defect omp had before its title
/// ([#485](#)) — and pi, unlike omp, puts no run state in its terminal title either ([#490](#)).
/// What it does paint is the loader: its ten braille frames in the composer's top border while a
/// turn is in flight, and in the status line above the border while a retry or a compaction runs.
/// Idle, the border is a plain run of dashes and no frame is on the screen. Measured live on pi
/// 0.73.1 in both states: `── ⠹ Working ──…` working, `────…` idle ([#544](#), [#545](#)).
///
/// **The read is deliberately vocabulary, not layout.** A border row is a run of `─` opening in
/// column zero, and the state is a frame wherever it sits between the two borders. A border
/// glyph or a footer rearrangement the read does not recognise answers `None`, which is the
/// reader having nothing to say — and the pane degrades to the status herdr gives it rather than
/// to a guess. A frame the model's own text paints in the footer region answers busy for the
/// frames it lasts, and the next read takes the answer back.
pub fn pi_status(screen: &[&str]) -> Option<&'static str> {
    let bottom = screen.iter().rposition(|line| border_row(line))?;
    let top = screen[..bottom].iter().rposition(|line| border_row(line))?;
    let footer = &screen[top.saturating_sub(2)..=bottom];
    if footer.iter().any(|line| has_frame(line)) {
        Some("busy")
    } else {
        Some("idle")
    }
}

/// The message pi is painting, lifted off the top of the footer.
///
/// **pi marks nothing.** Like omp, its assistant text is plain prose one column in, wrapped to
/// the same column, with a blank row between blocks and no glyph on the message itself — so the
/// read walks up from the composer's top border and stops at the first blank row, the same walk
/// omp's takes from its composer row ([#496](#)).
///
/// The rows between the conversation and the border are pi's own and are skipped rather than
/// read: the queue it draws there ([`pi_queued`]) and the status line a retry or a compaction
/// puts there, the latter carrying a spinner frame and so refused as message text.
///
/// `clipped` is always true, for the same reason omp's is: with no marker opening a message there
/// is nothing on the screen that says a block starts where the walk stopped, so the redundancy
/// check has to be `contains` rather than `starts_with`.
pub fn pi_live(screen: &[&str]) -> Option<LiveBlock> {
    let bottom = screen.iter().rposition(|line| border_row(line))?;
    let top = screen[..bottom].iter().rposition(|line| border_row(line))?;
    let mut body: Vec<&str> = Vec::new();
    for line in screen[..top].iter().rev() {
        if has_frame(line) {
            continue;
        }
        if line.starts_with(" Steering: ") || line.starts_with(" Follow-up: ") || line.starts_with(" ↳") {
            continue;
        }
        if line.trim().is_empty() {
            if body.is_empty() {
                continue;
            }
            break;
        }
        match line.strip_prefix(' ').filter(|rest| !rest.starts_with(' ')) {
            Some(text) => body.push(text.trim_end()),
            None => break,
        }
    }
    let text = body
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    (!text.is_empty()).then_some(LiveBlock { text, clipped: true })
}

/// The prompts the operator has sent that pi has not started on yet.
///
/// **The screen is the only account of one**, for the reason omp's is ([#489](#)): a queued
/// prompt is not written to the transcript until it is delivered. What pi draws is a `Steering: `
/// row per prompt it will take mid-turn, a `Follow-up: ` row per one it will take after the turn
/// ends, and a hint row under the last — all one column in, truncated to the row rather than
/// wrapped. A prompt too long for a row is cut, so what is published is what the operator can
/// see rather than a sentence reassembled out of rows pi never drew ([#546](#)).
pub fn pi_queued(screen: &[&str]) -> Vec<Queued> {
    let Some(hint) = screen.iter().rposition(|line| line.starts_with(" ↳")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for line in screen[..hint].iter().rev() {
        let Some(text) = line
            .strip_prefix(" Steering: ")
            .or_else(|| line.strip_prefix(" Follow-up: "))
        else {
            break;
        };
        found.push(Queued {
            text: text.trim_end().to_string(),
            at: None,
        });
    }
    found.reverse();
    found
}

/// A session file rather than one of the transcripts it launched: `sessions/<bucket>/<file>.jsonl`
/// exactly, where a subagent's is `sessions/<bucket>/<session>/<name>.jsonl`.
fn is_session_file(sessions: &Path, path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl") && path.parent().and_then(Path::parent) == Some(sessions)
}

/// `<timestamp>_<id>.jsonl` — the id is what `--resume` takes and what a pane announces.
fn session_id(transcript: &Path) -> Option<String> {
    let stem = transcript.file_stem()?.to_str()?;
    Some(stem.split_once('_').map_or(stem, |(_, id)| id).to_string())
}

fn declared_cwd(transcript: &Path) -> Option<PathBuf> {
    discover::head(transcript)
        .into_iter()
        .find_map(|value| match serde_json::from_value::<Record>(value) {
            Ok(Record::Session(header)) => header.cwd.map(PathBuf::from),
            _ => None,
        })
}

/// omp's own name for a working directory's bucket (`session-paths.ts`): home-relative with the
/// separators replaced, the temp root the same under a `-tmp` prefix, and everything else the
/// whole absolute path wrapped in a pair of dashes.
fn bucket(home: Option<&Path>, temp: &Path, cwd: &Path) -> String {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    for (prefix, root) in [("-", home), ("-tmp", Some(temp))] {
        let Some(root) = root.map(|r| r.canonicalize().unwrap_or_else(|_| r.to_path_buf())) else {
            continue;
        };
        if let Ok(relative) = cwd.strip_prefix(&root) {
            let encoded = flatten(relative);
            return match (encoded.is_empty(), prefix.ends_with('-')) {
                (true, _) => prefix.to_string(),
                (false, true) => format!("{prefix}{encoded}"),
                (false, false) => format!("{prefix}-{encoded}"),
            };
        }
    }
    format!("--{}--", flatten(cwd.strip_prefix("/").unwrap_or(&cwd)))
}

fn flatten(path: &Path) -> String {
    path.to_string_lossy().replace(['/', '\\', ':'], "-")
}

/// `pts-3` for `/dev/pts/3`, from field 7 of `/proc/<pid>/stat`.
///
/// The device number rather than `fd/0`: a pane's harness is the node's own user here, but a job
/// running as root refuses `fd/0` to anything that did not fork it (#332), and `stat` is
/// world-readable. Only pts devices are answered, because only a pty is what a pane holds and a
/// wrong guess here names another terminal's breadcrumb.
fn tty_id(pid: u32) -> Option<String> {
    tty_from_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

fn tty_from_stat(stat: &str) -> Option<String> {
    const PTS_MAJOR: u32 = 136;
    let fields = &stat[stat.rfind(") ")? + 2..];
    let tty: u32 = fields.split_whitespace().nth(4)?.parse().ok()?;
    let major = (tty >> 8) & 0xfff;
    let minor = (tty & 0xff) | ((tty >> 12) & 0xfff00);
    (major == PTS_MAJOR).then(|| format!("pts-{minor}"))
}

struct Filed {
    agent: String,
    root: TranscriptRoot,
    transcript: PathBuf,
}

/// Which entries are on the path the session is actually on.
///
/// **An omp session is a tree read here as a line, and a rewind is what makes the difference
/// visible.** `/tree`, `/branch` and the double-Escape selector move a *leaf pointer* and leave
/// the abandoned branch in the file: measured, a rewind past one prompt kept both of its records
/// and gave the next prompt the *previous* answer as its parent ([#495](#)). A reader taking the
/// file in order then publishes turns the operator took back, which is the one way a conversation
/// here can be wrong rather than merely thin.
///
/// So every entry's parent is kept — the bookkeeping ones too, because the chain runs through a
/// `title_change` and a `model_change` as much as through a message — and a record whose parent is
/// not the leaf recomputes the path. What falls off it is retired.
#[derive(Default)]
struct Branches {
    parent: HashMap<String, Option<String>>,
    live: HashSet<String>,
    leaf: Option<String>,
}

impl Branches {
    /// Records one entry and answers with the ids that have just left the live path.
    fn walked(&mut self, id: &str, parent: Option<&str>) -> Vec<String> {
        self.parent.insert(id.to_string(), parent.map(str::to_string));
        if parent.map(str::to_string) == self.leaf {
            self.live.insert(id.to_string());
            self.leaf = Some(id.to_string());
            return Vec::new();
        }
        let mut fresh = HashSet::new();
        let mut at = Some(id.to_string());
        while let Some(step) = at {
            if !fresh.insert(step.clone()) {
                break;
            }
            at = self.parent.get(&step).cloned().flatten();
        }
        let dropped = self.live.difference(&fresh).cloned().collect();
        self.live = fresh;
        self.leaf = Some(id.to_string());
        dropped
    }
}

#[derive(Default)]
pub struct OmpParser {
    store: TurnStore,
    branches: Branches,
    tool_turns: HashMap<String, (String, usize)>,
    /// The agents a card has already been minted for. A spawn is named either by the call's own
    /// `name` or by the acknowledgement that answers it, and both ends mint.
    minted: HashSet<String>,
    seq: u64,
    origin: Option<crate::attach::Origin>,
    filed: Option<Filed>,
}

impl TranscriptParser for OmpParser {
    fn push_line(&mut self, line: &str, at: u64) {
        let seq = self.seq;
        self.seq += 1;
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        if let Some((id, parent)) = record.walked().or_else(|| record::Node::of(line)) {
            for dropped in self.branches.walked(&id, parent.as_deref()) {
                // The retirement the wire already has: a turn under its own id carrying no blocks
                // is not drawn, and a page still holding one draws nothing for it.
                self.store.retire(&dropped);
            }
        }
        self.ingest(record, seq, at);
    }

    fn set_origin(&mut self, origin: crate::attach::Origin) {
        self.origin = Some(origin);
    }

    fn reset(&mut self) {
        *self = Self {
            origin: self.origin.take(),
            filed: self.filed.take(),
            ..Self::default()
        };
    }

    fn store(&self) -> &TurnStore {
        &self.store
    }

    fn store_mut(&mut self) -> &mut TurnStore {
        &mut self.store
    }
}

impl OmpParser {
    fn ingest(&mut self, record: Record, seq: u64, at: u64) {
        match record {
            Record::Message(entry) => {
                let atts = match &self.origin {
                    Some(origin) => crate::attach::headers(origin, at, &record::attachments(&entry)),
                    None => Vec::new(),
                };
                let id = entry.id.clone().unwrap_or_else(|| format!("o{seq}"));
                self.message(entry.message, id, entry.timestamp, atts);
            }
            Record::Compaction(compacted) => {
                let Some(summary) = compacted.summary.filter(|s| !s.trim().is_empty()) else {
                    return;
                };
                let id = compacted.id.unwrap_or_else(|| format!("o{seq}"));
                let mut turn = Turn::new(id, Role::Assistant, compacted.timestamp);
                turn.kind = Some(TurnKind::Compact);
                turn.blocks.push(Block::md(summary));
                self.store.push(turn);
            }
            _ => {}
        }
    }

    fn message(&mut self, message: record::Message, id: String, at: Option<String>, atts: Vec<Attachment>) {
        let mut atts = atts.into_iter();
        match message {
            record::Message::User { content, .. } => {
                let mut turn = Turn::new(id, Role::User, at);
                self.words(&mut turn, content, &mut atts);
                if !turn.blocks.is_empty() {
                    self.store.push(turn);
                }
            }
            record::Message::Assistant { content, .. } => {
                let mut turn = Turn::new(id.clone(), Role::Assistant, at);
                match content {
                    Content::Blocks(blocks) => {
                        for block in blocks {
                            self.block(block, &id, &mut turn, &mut atts);
                        }
                    }
                    other => self.words(&mut turn, other, &mut atts),
                }
                if !turn.blocks.is_empty() {
                    self.store.push(turn);
                }
            }
            record::Message::ToolResult {
                tool_call_id,
                content,
                is_error,
                details,
                ..
            } => self.settle(&tool_call_id, &record::result_text(&content), is_error, &details),
            record::Message::BashExecution {
                command,
                output,
                exit_code,
            } => {
                let mut turn = Turn::new(id, Role::User, at);
                let failed = exit_code.is_some_and(|code| code != 0);
                let text = output.unwrap_or_default();
                turn.blocks.push(Block::Tool {
                    summary: Some(one_line(&command)),
                    lines: count_lines(&text),
                    state: if failed { ToolState::Error } else { ToolState::Done },
                    name: "bash".into(),
                });
                turn.blocks.push(Block::Code {
                    lang: Some("bash".into()),
                    text: command,
                    role: None,
                });
                if !text.is_empty() {
                    turn.blocks.push(Block::Code {
                        lang: None,
                        text: clip(&text),
                        role: Some(CodeRole::Output),
                    });
                }
                self.store.push(turn);
            }
            record::Message::Other => {}
        }
    }

    fn words(&mut self, turn: &mut Turn, content: Content, atts: &mut impl Iterator<Item = Attachment>) {
        match content {
            Content::Text(text) => push_text(turn, text),
            Content::Blocks(blocks) => {
                for block in blocks {
                    match &block {
                        record::Block::Text { text } => push_text(turn, text.clone()),
                        record::Block::Image { .. } => turn.blocks.push(Block::Md {
                            text: image_marker(record::subtype(&block)),
                            att: record::picture(&block).and_then(|_| atts.next()),
                        }),
                        _ => {}
                    }
                }
            }
            Content::Anything(_) => {}
        }
    }

    fn block(
        &mut self,
        block: record::Block,
        turn_id: &str,
        turn: &mut Turn,
        atts: &mut impl Iterator<Item = Attachment>,
    ) {
        match block {
            record::Block::Text { text } => push_text(turn, text),
            record::Block::Image { .. } => turn.blocks.push(Block::Md {
                text: image_marker(record::subtype(&block)),
                att: record::picture(&block).and_then(|_| atts.next()),
            }),
            record::Block::ToolCall { id, name, arguments } => {
                let at = turn.blocks.len();
                turn.blocks.push(Block::Tool {
                    summary: summarise(&arguments),
                    lines: None,
                    state: ToolState::Running,
                    name: name.clone(),
                });
                if let Some(command) = arguments.get("command").and_then(Value::as_str) {
                    turn.blocks.push(Block::Code {
                        lang: Some("bash".into()),
                        text: command.to_string(),
                        role: None,
                    });
                }
                if name == TASK {
                    for spawn in record::spawns(&arguments) {
                        if let Some(block) = self.launched(&spawn) {
                            turn.blocks.push(block);
                        }
                    }
                }
                self.tool_turns.insert(id, (turn_id.to_string(), at));
            }
            record::Block::Other => {}
        }
    }

    /// The card for one spawn. omp files a launched agent's transcript beside the launching
    /// session — `<session>/<agent-name>.jsonl` — so the handle is the name, and a name is all
    /// this needs: the file is written as the agent works, and asking for it here would refuse a
    /// launch that has not written its first line ([#483](#)).
    fn launched(&mut self, spawn: &Spawn) -> Option<Block> {
        let name = spawn.name.clone()?;
        self.mint(&name, spawn.kind.clone(), spawn.task.clone())
    }

    fn mint(&mut self, name: &str, kind: Option<String>, title: Option<String>) -> Option<Block> {
        let filed = self.filed.as_ref()?;
        if !self.minted.insert(name.to_string()) {
            return None;
        }
        let transcript = sub::tree(&filed.transcript).join(format!("{name}.jsonl"));
        Some(Block::Sub {
            id: SubRef::new(&filed.agent, &filed.root, &transcript).encode(),
            kind,
            title,
            depth: None,
        })
    }

    fn settle(&mut self, call_id: &str, text: &str, is_error: bool, details: &Value) {
        let Some((target, card)) = self.tool_turns.get(call_id).cloned() else {
            return;
        };
        // A detached spawn answers its own call with `Spawned agent `x``, and that acknowledgement
        // is the only place omp publishes the name it generated for an unnamed one. The name is
        // minted from a `task` result only: any other tool result can carry the same sentence as
        // quoted text, and a card minted from a quote is a card for an agent that was never
        // spawned. The tool is read before the turn is taken, because minting wants the parser
        // and the revise wants the store.
        let task = self
            .store
            .position(&target)
            .and_then(|at| self.store.turns().get(at))
            .and_then(|turn| turn.tool_block(card))
            .is_some_and(|block| matches!(block, Block::Tool { name, .. } if name.as_str() == TASK));
        let launched: Vec<Block> = if task {
            record::spawned(text)
                .iter()
                .filter_map(|name| self.mint(name, None, None))
                .collect()
        } else {
            Vec::new()
        };
        let Some(turn) = self.store.revise(&target) else {
            return;
        };
        let mut carry = false;
        if let Some(Block::Tool {
            state, lines, name, ..
        }) = turn.tool_block_mut(card)
        {
            *state = if is_error {
                ToolState::Error
            } else {
                ToolState::Done
            };
            *lines = count_lines(text);
            carry = lines.is_some() && (is_error || RESULT_IS_THE_POINT.contains(&name.as_str()));
        }
        for block in launched {
            turn.blocks.push(block);
        }
        if let Some((path, text)) = record::unified_patch(details) {
            turn.blocks.push(Block::Diff { path, text });
        }
        if carry {
            output::place(turn, card, clip(text));
        }
    }
}

const TASK: &str = "task";

/// The calls whose result *is* the point, and so the only ones worth the bytes above.
///
/// The same three the Claude adapter carries and for the same reasons: `read`'s result is a file
/// the client fetches from the path on the card, and `edit`'s is the `diff` block beside it —
/// its own text is the new anchored snapshot the next edit has to quote, which is bookkeeping
/// between the harness and its model. An error is carried whatever the call was, because then the
/// text is the whole message.
const RESULT_IS_THE_POINT: &[&str] = &["bash", "glob", "grep"];

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory that goes away with the value that owns it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("kampr-omp-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(SESSIONS).join("-tmp-komp")).expect("scratch");
            std::fs::create_dir_all(dir.join(TERMINALS)).expect("scratch");
            Self(dir)
        }

        fn adapter(&self) -> OmpAdapter {
            OmpAdapter::new(TranscriptRoot::new(&self.0).expect("root"))
        }

        fn session(&self, name: &str) -> PathBuf {
            let path = self.0.join(SESSIONS).join("-tmp-komp").join(name);
            std::fs::write(&path, "{}\n").expect("session");
            path
        }

        fn crumb(&self, cwd: &str, session: &Path, fresh: bool) -> PathBuf {
            let path = self.0.join(TERMINALS).join("pts-9");
            let tail = if fresh { "\nfresh" } else { "" };
            std::fs::write(&path, format!("{cwd}\n{}{tail}\n", session.display())).expect("crumb");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SESSION: &str = "2026-09-04T12-29-19-072Z_01a06c65-1560-710f-9ae4-8bb687cce92e.jsonl";
    const ID: &str = "01a06c65-1560-710f-9ae4-8bb687cce92e";

    #[test]
    fn a_subagents_transcript_is_not_mistaken_for_the_session_it_belongs_to() {
        let sessions = Path::new("/home/x/.omp/agent/sessions");
        assert!(is_session_file(
            sessions,
            Path::new("/home/x/.omp/agent/sessions/-dev-kampr/2026-09-04T12-27-18-328Z_01a0.jsonl")
        ));
        assert!(!is_session_file(
            sessions,
            Path::new("/home/x/.omp/agent/sessions/-dev-kampr/2026-09-04T12-27-18-328Z_01a0/prober.jsonl")
        ));
    }

    #[test]
    fn a_working_title_is_read_by_the_frame_it_is_on() {
        assert_eq!(title_status("π > project"), Some("idle"));
        assert_eq!(title_status("π ⠹ Running a slow command"), Some("busy"));
        assert_eq!(title_status("π ! Running a slow command"), Some("waiting"));
        // `tui.titleState` off, and an operator who turned the state off has not said idle.
        assert_eq!(title_status("π: project"), None);
        assert_eq!(title_status("dbrain@comingclean:~"), None);
    }

    /// Verbatim `/proc/<pid>/stat` of the omp on `/dev/pts/3` that wrote
    /// `tests/fixtures/live/omp-probe.jsonl`, cut after the field this reads.
    const OMP_STAT: &str =
        "1423965 (omp) S 1423234 1423965 1423234 34819 1423965 4194304 242628 268753 0 2 1315 166";

    #[test]
    fn the_terminal_a_breadcrumb_is_filed_under_is_read_off_the_process() {
        assert_eq!(tty_from_stat(OMP_STAT).as_deref(), Some("pts-3"));
        // A process with no controlling terminal, and a comm that would shift a naive field count.
        assert_eq!(
            tty_from_stat("7 (a) b) S 1 7 7 0 -1 4194304 1 2 0 0 3 4").as_deref(),
            None
        );
    }

    #[test]
    fn a_session_that_has_written_nothing_yet_is_named_by_its_breadcrumb_without_a_transcript() {
        let scratch = Scratch::new("fresh");
        let adapter = scratch.adapter();
        let unwritten = scratch.0.join(SESSIONS).join("-tmp-komp").join(SESSION);
        let crumb = scratch.crumb("/tmp/komp", &unwritten, true);
        let found = adapter.crumb_at(&crumb, Started::Unknown).expect("a crumb");
        assert_eq!(found.session, ID);
        assert_eq!(found.transcript, None, "the file does not exist yet");
        assert_eq!(found.cwd.as_deref(), Some(Path::new("/tmp/komp")));
    }

    /// The other half of the same guard, and the one that stops a *live* pane being answered for.
    ///
    /// A crumb is keyed by terminal and nothing removes one, so the file goes on naming a session
    /// for every process that ever runs on that tty — including the pane's own login shell, whose
    /// start time is older than the crumb and so passes the mtime guard above it. `Registry::marker`
    /// asks every adapter about every process in the pipeline, so this is what stops a quit omp
    /// answering for the `claude` started in its place.
    #[test]
    fn a_breadcrumb_is_only_read_for_a_process_that_is_this_harness() {
        let scratch = Scratch::new("comm");
        let mine = crate::process::comm(std::process::id()).expect("this process has a name");
        assert_ne!(mine, AGENT, "the test binary is not an omp");
        assert!(!scratch.adapter().is_the_harness(std::process::id()));
        let named = OmpAdapter::named(&mine, TranscriptRoot::new(&scratch.0).expect("root"));
        assert!(
            named.is_the_harness(std::process::id()),
            "and the refusal is the name rather than the read"
        );
        // A pid the kernel does not have is nobody's harness.
        assert!(!named.is_the_harness(u32::MAX));
    }

    #[test]
    fn a_breadcrumb_older_than_the_process_reading_it_is_a_previous_omp_on_the_same_tty() {
        let scratch = Scratch::new("stale");
        let adapter = scratch.adapter();
        let session = scratch.session(SESSION);
        let crumb = scratch.crumb("/tmp/komp", &session, false);
        assert!(adapter.crumb_at(&crumb, Started::Unknown).is_some());
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(
            adapter.crumb_at(&crumb, Started::At(later)).is_none(),
            "pts numbers are reused, and a crumb written before this process is not its own"
        );
    }

    /// The descriptor walk, against a file this very process is holding open — which is what an
    /// omp does for the life of a session.
    #[test]
    fn the_session_a_process_holds_open_is_the_session_it_is_on() {
        let scratch = Scratch::new("held");
        let adapter = scratch.adapter();
        let session = scratch.session(SESSION);
        let sub = scratch
            .0
            .join(SESSIONS)
            .join("-tmp-komp")
            .join(SESSION.trim_end_matches(".jsonl"));
        std::fs::create_dir_all(&sub).expect("sub dir");
        let launched = sub.join("prober.jsonl");
        std::fs::write(&launched, "{}\n").expect("sub");
        let _held = std::fs::File::open(&launched).expect("open sub");
        assert_eq!(
            adapter.held(std::process::id()),
            None,
            "a launched agent's is not the session's"
        );
        let _open = std::fs::File::open(&session).expect("open session");
        assert_eq!(
            adapter.held(std::process::id()).as_deref(),
            Some(session.canonicalize().expect("canonical").as_path())
        );
    }

    #[test]
    fn a_working_directory_is_bucketed_the_way_omp_files_it() {
        let home = Path::new("/home/dbrain");
        let temp = Path::new("/tmp");
        assert_eq!(
            bucket(Some(home), temp, Path::new("/home/dbrain/dev/kampr")),
            "-dev-kampr"
        );
        assert_eq!(bucket(Some(home), temp, home), "-");
        assert_eq!(bucket(Some(home), temp, Path::new("/var/lib/x")), "--var-lib-x--");
        // Measured: the bucket omp made for this repo's own scratch project.
        assert_eq!(
            bucket(
                Some(home),
                temp,
                Path::new("/tmp/claude-1000/-home-dbrain-dev-kampr/22464faf/scratchpad/probe/project")
            ),
            "-tmp-claude-1000--home-dbrain-dev-kampr-22464faf-scratchpad-probe-project"
        );
    }
    const BORDER: &str = "────────────────────────────────────────────────────────────";

    /// A pi screen the way the measured one reads: the conversation one column in, a blank row,
    /// the queue when there is one, the composer's two borders, and the path and stats under
    /// them.
    fn pi_screen(conversation: &[&str], working: bool, queued: &[&str]) -> Vec<String> {
        let mut screen: Vec<String> = Vec::new();
        for line in conversation {
            screen.push(line.to_string());
        }
        screen.push(String::new());
        for prompt in queued {
            screen.push(format!(" Steering: {prompt}"));
        }
        if !queued.is_empty() {
            screen.push(" ↳ Alt+Up to edit all queued messages".to_string());
        }
        if working {
            screen.push(format!(
                "── ⠹ Working {}",
                BORDER.chars().skip(2).collect::<String>()
            ));
        } else {
            screen.push(BORDER.to_string());
        }
        screen.push(String::new());
        screen.push(BORDER.to_string());
        screen.push("/tmp".to_string());
        screen.push("↑15k ↓464 8.0%/201k (auto)".to_string());
        screen
    }

    fn rows(screen: &[String]) -> Vec<&str> {
        screen.iter().map(String::as_str).collect()
    }

    #[test]
    fn the_frame_between_the_borders_is_pi_s_running_state() {
        let busy = pi_screen(&[" the lighthouse remains a promise"], true, &[]);
        assert_eq!(pi_status(&rows(&busy)), Some("busy"));
        let idle = pi_screen(&[" the lighthouse remains a promise"], false, &[]);
        assert_eq!(pi_status(&rows(&idle)), Some("idle"));
        // A retry or a compaction spins above the border rather than in it.
        let mut compacting = pi_screen(&[" the lighthouse remains a promise"], false, &[]);
        compacting.insert(
            compacting.len() - 5,
            " ⠹ Compacting the conversation…".to_string(),
        );
        assert_eq!(pi_status(&rows(&compacting)), Some("busy"));
        // A screen with no footer for the reader to find is nothing to read, not idle.
        assert_eq!(pi_status(&["$ ", " some shell output"]), None);
    }

    #[test]
    fn the_block_above_the_footer_is_the_message_being_painted() {
        let screen = pi_screen(
            &[" the lighthouse remains a", " promise to the sailor"],
            true,
            &[],
        );
        let block = pi_live(&rows(&screen)).expect("a block");
        assert_eq!(block.text, "the lighthouse remains a\npromise to the sailor");
        assert!(block.clipped);
        // The queue's own rows are status, not message text.
        let queued = pi_screen(
            &[" 1. Blue whale"],
            true,
            &["THIRD PROMPT: and the color of the sky."],
        );
        let block = pi_live(&rows(&queued)).expect("a block");
        assert_eq!(block.text, "1. Blue whale");
        assert!(pi_live(&["$ ", " some shell output"]).is_none());
    }

    #[test]
    fn the_prompts_above_the_hint_are_the_queue() {
        let screen = pi_screen(
            &[" 1. Blue whale"],
            true,
            &["THIRD PROMPT: and the color of the sky."],
        );
        let queued = pi_queued(&rows(&screen));
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].text, "THIRD PROMPT: and the color of the sky.");
        let none = pi_screen(&[" 1. Blue whale"], true, &[]);
        assert!(pi_queued(&rows(&none)).is_empty());
    }

    /// The pid is visible to procfs at the fork, but the kernel commits the new `envp` only when
    /// the exec completes, and an `environ` read in between succeeds with zero bytes — 198 of 200
    /// immediate reads in the measurement, none five milliseconds on. Production scans processes
    /// that have been running, so it never meets the window; a test that spawns and reads at once
    /// does, and waits it out the way a scan tick would.
    fn environ_settled(pid: u32) -> (Option<String>, Option<String>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some((file, parent)) = OmpAdapter::session_env(pid)
                && (file.is_some() || parent.is_some())
            {
                return (file, parent);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the child's environ never settled"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn the_variables_a_process_carries_are_the_ones_the_kernel_keeps_of_it() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .env_remove("PI_SESSION_FILE")
            .env_remove("PI_SUBAGENT_PARENT_SESSION")
            .env("PI_SESSION_FILE", "/tmp/kampr-env-test/session.jsonl")
            .env("PI_SUBAGENT_PARENT_SESSION", "the-root-id")
            .spawn()
            .expect("sleep");
        let (file, parent) = environ_settled(child.id());
        assert_eq!(file.as_deref(), Some("/tmp/kampr-env-test/session.jsonl"));
        assert_eq!(parent.as_deref(), Some("the-root-id"));
        child.kill().expect("kill");
        child.wait().expect("wait");
    }

    #[test]
    fn a_pane_is_the_session_its_processes_name_in_their_environment() {
        let scratch = Scratch::new("env");
        let adapter = scratch.adapter();
        let session = scratch.session("2026-01-01T00-00-00Z_env.jsonl");
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .env_remove("PI_SESSION_FILE")
            .env_remove("PI_SUBAGENT_PARENT_SESSION")
            .env("PI_SESSION_FILE", session.to_str().expect("utf8"))
            .spawn()
            .expect("sleep");
        let process = PaneProcess {
            pid: child.id(),
            ..PaneProcess::default()
        };
        let pipeline = [process];
        // The pipeline answers once the exec has committed its environment.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let (found, who) = loop {
            if let Some(answer) = adapter.environment(&pipeline) {
                break answer;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the environment never named the session"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(who.pid, child.id());
        assert_eq!(found.session, "env");
        assert_eq!(
            found.transcript.as_deref(),
            Some(session.canonicalize().expect("canonical").as_path())
        );
        child.kill().expect("kill");
        child.wait().expect("wait");
    }

    const USER_LINE: &str = r#"{"type":"message","id":"u1","parentId":null,"timestamp":"2026-01-01T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"show me the log"}],"timestamp":1}}"#;
    const QUOTED_RESULT: &str = r#"{"type":"message","id":"t1","parentId":"a1","timestamp":"2026-01-01T00:00:03Z","message":{"role":"toolResult","toolCallId":"c1","content":[{"type":"text","text":"the log says: Spawned agent `FakeAgent` (job `FakeAgent`)"}],"isError":false,"details":{}}}"#;
    const TASK_RESULT: &str = r#"{"type":"message","id":"t1","parentId":"a1","timestamp":"2026-01-01T00:00:03Z","message":{"role":"toolResult","toolCallId":"c1","content":[{"type":"text","text":"Spawned agent `RealAgent` (job `RealAgent`). Its result auto-delivers on yield."}],"isError":false,"details":{}}}"#;

    fn a_call(tool: &str) -> String {
        format!(
            r#"{{"type":"message","id":"a1","parentId":"u1","timestamp":"2026-01-01T00:00:02Z","message":{{"role":"assistant","content":[{{"type":"toolCall","id":"c1","name":"{tool}","arguments":{{"command":"cat log"}}}}],"timestamp":2,"stopReason":"toolUse"}}}}"#
        )
    }

    fn parser_with_filed(scratch: &Scratch, session: &Path) -> OmpParser {
        OmpParser {
            filed: Some(Filed {
                agent: "omp".into(),
                root: TranscriptRoot::new(&scratch.0).expect("root"),
                transcript: session.to_path_buf(),
            }),
            ..OmpParser::default()
        }
    }

    #[test]
    fn a_spawn_quoted_in_a_result_other_than_task_mints_no_card() {
        let scratch = Scratch::new("quoted");
        let session = scratch.session("2026-01-01T00-00-00Z_quoted.jsonl");
        let mut parser = parser_with_filed(&scratch, &session);
        parser.push_line(USER_LINE, 0);
        parser.push_line(&a_call("bash"), 100);
        parser.push_line(QUOTED_RESULT, 200);
        let turns = parser.store_mut().drain_changed();
        assert!(
            turns
                .iter()
                .all(|t| t.blocks.iter().all(|b| !matches!(b, Block::Sub { .. }))),
            "a quote is not a spawn: {turns:?}"
        );
    }

    #[test]
    fn a_spawn_named_by_its_task_result_still_mints_its_card() {
        let scratch = Scratch::new("task");
        let session = scratch.session("2026-01-01T00-00-00Z_task.jsonl");
        let mut parser = parser_with_filed(&scratch, &session);
        parser.push_line(USER_LINE, 0);
        parser.push_line(&a_call("task"), 100);
        parser.push_line(TASK_RESULT, 200);
        let turns = parser.store_mut().drain_changed();
        assert!(
            turns
                .iter()
                .any(|t| t.blocks.iter().any(|b| matches!(b, Block::Sub { .. }))),
            "the task's own acknowledgement still mints: {turns:?}"
        );
    }
}
