use std::path::PathBuf;

use crate::process::Started;

/// What a harness writes down about a session while it is running, keyed on the pid running it.
///
/// **It exists before the transcript does.** `~/.claude/sessions/<pid>.json` is written when the
/// session opens and removed when it exits; the transcript is not created until the first prompt
/// is submitted, measured minutes later. So `transcript: None` is an agent pane with an *empty*
/// conversation, which is a third answer beside "this pane has a conversation" and "this pane has
/// none", and the two must not be allowed to drift into each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMarker {
    pub agent: String,
    pub pid: u32,
    pub session: String,
    pub cwd: Option<PathBuf>,
    pub name: Option<String>,
    pub name_source: Option<String>,
    /// What the harness last said about itself: `busy`, `shell`, `idle` or `waiting`, rewritten
    /// **in place** within ~100 ms of every transition, so it tracks a live session rather than
    /// being stamped once. An `IN_MODIFY` watch on this file is therefore a push feed for a
    /// pane's agent status — stronger than a screen scrape, which cannot see a pane whose
    /// harness has stopped writing to the terminal.
    pub status: Option<String>,
    pub transcript: Option<PathBuf>,
    /// When *this run* of the harness started, from the marker's own `startedAt`.
    ///
    /// **The one fact the transcript does not hold, and the harness has been writing it all
    /// along.** A restart leaves nothing in the file that separates it from a pause — one
    /// `sessionId` and one `version` across a 70-hour transcript, 59 gaps over ten minutes, and no
    /// record at the seam — so nothing read from the transcript alone can tell work the current run
    /// launched from work its predecessor left open. `startedAt` tells it exactly, and it is the
    /// harness's own answer rather than an inference about its process, so it says the same thing
    /// on a host with no procfs.
    ///
    /// It is the *run*, not the conversation: a `--continue` appends to the transcript it resumes
    /// and stamps its own marker with the resume, measured end to end.
    pub started: Started,
}
