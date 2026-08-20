use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Every write a device makes, one JSON object per line.
///
/// Mode 0600 because the detail field carries what was typed — an answer to a permission prompt,
/// a command, a reply. This file is as sensitive as the terminals it describes.
/// One previous generation is kept. This is a forensic trail, not an archive: keeping more of it
/// than the operator has disk for is how the trail stops existing at all.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct AuditLog {
    path: PathBuf,
    file: Mutex<Option<File>>,
    written: Mutex<u64>,
    max_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry<'a> {
    pub at: String,
    pub action: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl<'a> Entry<'a> {
    pub fn new(action: &'a str) -> Self {
        Self {
            at: OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default(),
            action,
            device: None,
            device_name: None,
            role: None,
            peer: None,
            pane: None,
            detail: None,
        }
    }

    pub fn device(mut self, id: &'a str, name: &'a str, role: &'a str) -> Self {
        self.device = Some(id);
        self.device_name = Some(name);
        self.role = Some(role);
        self
    }

    pub fn peer(mut self, peer: &'a str) -> Self {
        self.peer = Some(peer);
        self
    }

    pub fn pane(mut self, pane: &'a str) -> Self {
        self.pane = Some(pane);
        self
    }

    pub fn detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

impl AuditLog {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Self::with_max_bytes(path, DEFAULT_MAX_BYTES)
    }

    pub fn with_max_bytes(path: &Path, max_bytes: u64) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            crate::files::private_dir(dir)?;
        }
        let log = Self {
            path: path.to_path_buf(),
            file: Mutex::new(None),
            written: Mutex::new(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)),
            max_bytes,
        };
        log.with_file(|_| Ok(()))?;
        Ok(log)
    }

    /// Drops every record. For tests and for a node that has no state directory yet.
    pub fn disabled() -> Self {
        Self {
            path: PathBuf::new(),
            file: Mutex::new(None),
            written: Mutex::new(0),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    pub fn record(&self, entry: &Entry<'_>) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        let line = match serde_json::to_string(entry) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "audit entry would not serialise");
                return;
            }
        };
        if let Err(e) = self.with_file(|f| {
            f.write_all(line.as_bytes())?;
            f.write_all(b"\n")
        }) {
            tracing::warn!(error = %e, path = %self.path.display(), "audit write failed");
            return;
        }
        let mut written = self.written.lock().unwrap();
        *written += line.len() as u64 + 1;
        if *written >= self.max_bytes {
            *written = 0;
            drop(written);
            if let Err(e) = self.rotate() {
                tracing::warn!(error = %e, "audit rotation failed");
            }
        }
    }

    fn rotate(&self) -> std::io::Result<()> {
        let mut slot = self.file.lock().unwrap();
        slot.take();
        let rolled = self.path.with_extension("jsonl.1");
        std::fs::rename(&self.path, &rolled)?;
        crate::files::chmod(&rolled, 0o600)?;
        *slot = Some(open_private(&self.path)?);
        Ok(())
    }

    fn with_file<T>(&self, f: impl FnOnce(&mut File) -> std::io::Result<T>) -> std::io::Result<T> {
        let mut slot = self.file.lock().unwrap();
        if slot.is_none() {
            *slot = Some(open_private(&self.path)?);
        }
        f(slot.as_mut().expect("just opened"))
    }
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_record_is_one_json_line_at_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::open(&path).unwrap();
        log.record(&Entry::new("input").device("d1", "phone", "full").pane("n/w1:p1"));
        log.record(&Entry::new("answer").detail(serde_json::json!({ "key": "1" })));

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["action"], "input");
        assert_eq!(first["device"], "d1");
        assert_eq!(first["pane"], "n/w1:p1");
        assert!(first["at"].as_str().unwrap().contains('T'));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the audit log echoes reply text");
        }
    }

    #[test]
    fn the_log_rotates_instead_of_growing_until_the_disk_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::with_max_bytes(&path, 4096).unwrap();
        for n in 0..4000 {
            log.record(&Entry::new("input").detail(serde_json::json!({ "n": n })));
        }
        let live = std::fs::metadata(&path).unwrap().len();
        assert!(live <= 4096, "the live log grew to {live}");
        let rolled = path.with_extension("jsonl.1");
        assert!(rolled.exists(), "the previous generation is kept");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&rolled).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_disabled_log_writes_nothing_and_does_not_panic() {
        AuditLog::disabled().record(&Entry::new("input"));
    }
}
