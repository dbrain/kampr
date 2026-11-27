use std::path::PathBuf;

/// The prompt a session was created with: the creator leaves it here and the conversation sends
/// it after the account check, never the creator.
const OPENING: &str = "opening-prompt";
const HELD: &str = "held-words";

/// What a headless session was asked to say and has not said yet, kept in its keeper's directory
/// until it reaches the child, so a pane that leaves or a node that restarts does not drop it.
#[derive(Debug, Clone)]
pub(crate) struct Kept(PathBuf);

impl Kept {
    pub fn at(session: PathBuf) -> Self {
        Self(session)
    }

    pub fn set_opening(&self, prompt: &str) -> std::io::Result<()> {
        std::fs::write(self.0.join(OPENING), prompt)
    }

    pub fn opening(&self) -> Option<String> {
        std::fs::read_to_string(self.0.join(OPENING)).ok()
    }

    pub fn opened(&self) {
        let _ = std::fs::remove_file(self.0.join(OPENING));
    }

    pub fn held(&self) -> Vec<String> {
        std::fs::read(self.0.join(HELD))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// The memory copy is what is sent; a write that fails costs only the restart's copy.
    pub fn hold(&self, lines: &[String]) {
        let path = self.0.join(HELD);
        if lines.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let tmp = self.0.join(format!(".{HELD}.tmp"));
        let written = serde_json::to_vec(lines)
            .map_err(std::io::Error::from)
            .and_then(|bytes| std::fs::write(&tmp, bytes));
        if written.is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}
