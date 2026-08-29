use std::path::PathBuf;

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
}
