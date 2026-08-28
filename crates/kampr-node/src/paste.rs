use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use kampr_journal::attach::MAX_BYTES;

/// How long a pasted file is kept before the next paste sweeps it away.
///
/// A pane that never reads its paste must not leave the bytes on the node for ever, and there is
/// no event that says a harness has finished with a path — so the only honest rule is a lifetime.
/// A day is long enough that an operator who pastes a screenshot and comes back after lunch still
/// has it, and short enough that a phone's camera roll cannot be accumulated here a picture at a
/// time.
const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// How many pasted files may sit in the directory at once, whatever their age.
///
/// The lifetime alone bounds a slow leak and not a fast one: nothing stops a client pasting in a
/// loop, and 8 MiB a time fills a disk long before anything is a day old. The oldest go first.
const KEEP: usize = 64;

/// What the node types into the pane once the bytes are written.
///
/// **The trailing space is the whole of it, and it is load-bearing.** The path used to be typed
/// bare, so it abutted whatever the operator typed next — `/…/shot-1.pngwhat is this` — and the
/// harness was handed one word. Nothing can fix the leading side from here: the node does not know
/// what is already on the pane's line.
pub fn typed(path: &Path) -> String {
    format!("{} ", path.display())
}

#[derive(Debug)]
pub enum PasteError {
    TooLarge(u64),
    Empty,
    Undecodable,
    Io(std::io::Error),
}

impl std::fmt::Display for PasteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(f, "{bytes} bytes is larger than this node will take"),
            Self::Empty => write!(f, "there were no bytes to paste"),
            Self::Undecodable => write!(f, "the body was not base64"),
            Self::Io(e) => write!(f, "the paste could not be written: {e}"),
        }
    }
}

/// Where a paste landed, as an absolute path on this node's own filesystem.
///
/// **The node picks every part of it.** A client says what the bytes are and nothing about where
/// they go: the directory is this node's, the extension is sniffed from the bytes themselves, and
/// only the stem carries anything the client asked for — sanitised to a filename that cannot be a
/// path. A client that could name the location would be a client that can write anywhere this
/// process can, which is a different verb entirely from the one this is.
pub fn write(state_dir: &Path, body: &[u8], name: Option<&str>) -> Result<PathBuf, PasteError> {
    if body.is_empty() {
        return Err(PasteError::Empty);
    }
    if body.len() as u64 > MAX_BYTES {
        return Err(PasteError::TooLarge(body.len() as u64));
    }
    let dir = state_dir.join("pastes");
    std::fs::create_dir_all(&dir).map_err(PasteError::Io)?;
    restrict(&dir);
    sweep(&dir);

    let stem = stem(name);
    let at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = dir.join(format!("{stem}-{at}.{}", extension(body)));
    let mut file = std::fs::File::create(&path).map_err(PasteError::Io)?;
    file.write_all(body).map_err(PasteError::Io)?;
    file.sync_all().map_err(PasteError::Io)?;
    Ok(path)
}

#[cfg(unix)]
fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict(_dir: &Path) {}

/// Removes what is past its lifetime, then what is past the count, oldest first.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let at = e.metadata().ok()?.modified().ok()?;
            Some((at, e.path()))
        })
        .collect();
    let now = SystemTime::now();
    found.retain(|(at, path)| {
        let stale = now.duration_since(*at).is_ok_and(|age| age > LIFETIME);
        if stale {
            let _ = std::fs::remove_file(path);
        }
        !stale
    });
    if found.len() < KEEP {
        return;
    }
    found.sort_by_key(|(at, _)| *at);
    for (_, path) in found.iter().take(found.len() + 1 - KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

/// A filename that cannot be a path, cannot be hidden, and cannot be empty.
///
/// The separators are the point: a stem is joined onto a directory this node chose, so anything
/// that could climb out of it — a separator, a `..`, a leading dot — is not a name and is dropped
/// rather than escaped.
fn stem(name: Option<&str>) -> String {
    let cleaned: String = name
        .unwrap_or_default()
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c,
            '-' | '_' => c,
            _ => '-',
        })
        .take(48)
        .collect();
    let trimmed = cleaned.trim_matches('-');
    match trimmed.is_empty() {
        true => "paste".to_string(),
        false => trimmed.to_string(),
    }
}

/// What the bytes are, read off the bytes.
///
/// **Never the client's word for it.** An extension decides what the harness on the other end
/// will do with the file, and a body that says `png` while holding something else is the whole
/// shape of the problem — so the only opinion that counts is the one the leading bytes carry.
/// Anything unrecognised that is valid UTF-8 is text, and anything else is opaque.
fn extension(body: &[u8]) -> &'static str {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const GIF87: &[u8] = b"GIF87a";
    const GIF89: &[u8] = b"GIF89a";
    const PDF: &[u8] = b"%PDF-";
    if body.starts_with(PNG) {
        return "png";
    }
    if body.starts_with(&[0xff, 0xd8, 0xff]) {
        return "jpg";
    }
    if body.starts_with(GIF87) || body.starts_with(GIF89) {
        return "gif";
    }
    if body.starts_with(b"RIFF") && body.len() > 12 && &body[8..12] == b"WEBP" {
        return "webp";
    }
    if body.starts_with(PDF) {
        return "pdf";
    }
    match std::str::from_utf8(body) {
        Ok(_) => "txt",
        Err(_) => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_path_is_typed_with_room_after_it_for_what_the_operator_says_next() {
        assert_eq!(
            typed(Path::new("/var/kampr/pastes/shot-1.png")),
            "/var/kampr/pastes/shot-1.png ",
        );
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kampr-paste-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn the_extension_comes_from_the_bytes_and_never_from_what_the_client_called_it() {
        let dir = scratch("sniff");
        let png = b"\x89PNG\r\n\x1a\n\x00rest of it";

        let path = write(&dir, png, Some("totally-a-script.sh")).expect("written");

        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_that_is_a_path_cannot_climb_out_of_the_directory_it_is_joined_to() {
        let dir = scratch("escape");

        let path = write(&dir, b"hello", Some("../../../etc/cron.d/evil")).expect("written");

        assert_eq!(
            path.parent(),
            Some(dir.join("pastes").as_path()),
            "a paste landed at {path:?}, outside the directory this node chose"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_body_past_the_ceiling_is_refused_rather_than_truncated() {
        let dir = scratch("ceiling");
        let huge = vec![0u8; MAX_BYTES as usize + 1];

        let refused = write(&dir, &huge, None);

        assert!(matches!(refused, Err(PasteError::TooLarge(_))), "{refused:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_paste_sweeps_the_directory_rather_than_letting_it_grow_without_end() {
        let dir = scratch("sweep");
        for i in 0..KEEP + 8 {
            write(&dir, format!("body {i}").as_bytes(), None).expect("written");
        }

        let left = std::fs::read_dir(dir.join("pastes")).expect("dir").count();

        assert!(left <= KEEP, "{left} pasted files were left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
