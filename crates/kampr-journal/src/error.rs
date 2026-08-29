use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("transcript root {0} is not a directory")]
    BadRoot(PathBuf),

    #[error("session reference {0:?} escapes the transcript root")]
    Escape(String),

    #[error("no transcript for session {0:?}")]
    NotFound(String),

    /// The harness named the session and the session has written nothing yet — a pane with
    /// an *empty* conversation, which is not the same answer as a pane with none (#311).
    #[error("session {0:?} has not written a transcript yet")]
    Unwritten(String),

    #[error("attachment of {0} bytes is past what this node will serve")]
    TooLarge(u64),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
