use std::path::{Component, Path, PathBuf};

use crate::error::JournalError;

/// A containment root. Everything a pane can name — a session id, a session path — is resolved
/// through here, so a hostile or merely stale pane announcement cannot reach outside the
/// configured transcript directory.
#[derive(Debug, Clone)]
pub struct TranscriptRoot(PathBuf);

impl TranscriptRoot {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let path = path.as_ref();
        let canonical = path
            .canonicalize()
            .map_err(|_| JournalError::BadRoot(path.to_path_buf()))?;
        if !canonical.is_dir() {
            return Err(JournalError::BadRoot(canonical));
        }
        Ok(Self(canonical))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Resolves a caller-supplied path against the root and proves the result is inside it.
    /// Canonicalisation is what makes this safe against symlinks, so the target must exist.
    pub fn contain(&self, candidate: &str) -> Result<PathBuf, JournalError> {
        if candidate.is_empty() || candidate.contains('\0') {
            return Err(JournalError::Escape(candidate.to_string()));
        }
        let raw = Path::new(candidate);
        let joined = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            self.0.join(raw)
        };
        let resolved = joined
            .canonicalize()
            .map_err(|_| JournalError::NotFound(candidate.to_string()))?;
        if !resolved.starts_with(&self.0) {
            return Err(JournalError::Escape(candidate.to_string()));
        }
        Ok(resolved)
    }

    /// A session id becomes a filename fragment, so it may only be a single path-safe segment.
    pub fn check_id(&self, id: &str) -> Result<(), JournalError> {
        let ok = !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && Path::new(id).components().count() == 1
            && !matches!(
                Path::new(id).components().next(),
                Some(Component::ParentDir) | Some(Component::CurDir)
            );
        if ok {
            Ok(())
        } else {
            Err(JournalError::Escape(id.to_string()))
        }
    }
}
