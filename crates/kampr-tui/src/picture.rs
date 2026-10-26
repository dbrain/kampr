use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The node's own ceiling on a paste, checked here so a picture that would be refused is not sent.
const MAX_BYTES: usize = 8 * 1024 * 1024;

const IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Where `ctrl+v` looks for a picture. A seam, because a test has no clipboard.
pub type Reader = Arc<dyn Fn() -> Option<Vec<u8>> + Send + Sync>;

pub fn system() -> Reader {
    Arc::new(from_clipboard)
}

/// herdr's `remote_image_paste` (`ctrl+v` under `herdr --remote`, #289), with the same reasoning
/// the wire's `paste` gives: a terminal's own image-paste protocol dies on the way to a pane on
/// another machine, so the picture is read here and sent as bytes.
fn from_clipboard() -> Option<Vec<u8>> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let types = run("wl-paste", &["--list-types"], Duration::from_millis(300))?;
        let mime = image_type(&String::from_utf8_lossy(&types))?;
        return run(
            "wl-paste",
            &["--no-newline", "--type", &mime],
            Duration::from_secs(3),
        );
    }
    if std::env::var_os("DISPLAY").is_some() {
        let types = run(
            "xclip",
            &["-selection", "clipboard", "-t", "TARGETS", "-o"],
            Duration::from_millis(300),
        )?;
        let mime = image_type(&String::from_utf8_lossy(&types))?;
        return run(
            "xclip",
            &["-selection", "clipboard", "-t", &mime, "-o"],
            Duration::from_secs(3),
        );
    }
    if cfg!(target_os = "macos") {
        return run("pngpaste", &["-"], Duration::from_secs(3)).filter(|b| !b.is_empty());
    }
    None
}

fn image_type(listed: &str) -> Option<String> {
    listed
        .lines()
        .map(str::trim)
        .find(|t| t.starts_with("image/") && IMAGES.iter().any(|ext| t.ends_with(ext)))
        .map(str::to_string)
}

/// A clipboard owner that has hung must not hang the keyboard with it.
fn run(program: &str, args: &[&str], within: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reading = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        out.by_ref()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()
            .map(|_| bytes)
    });
    let deadline = Instant::now() + within;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    reading
        .join()
        .ok()?
        .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_BYTES)
}

/// A terminal hands a dropped file over as a paste of its path. One path to a picture on this
/// machine is that picture; anything else is text.
pub fn dropped(text: &str) -> Option<(Vec<u8>, String)> {
    let text = text.trim();
    let text = text
        .strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .or_else(|| text.strip_prefix('"').and_then(|t| t.strip_suffix('"')))
        .unwrap_or(text);
    let text = text.strip_prefix("file://").unwrap_or(text);
    let text = text.replace("\\ ", " ");
    if text.contains('\n') {
        return None;
    }
    let path = Path::new(&text);
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !IMAGES.contains(&ext.as_str()) {
        return None;
    }
    let bytes = std::fs::read(path)
        .ok()
        .filter(|b| !b.is_empty() && b.len() <= MAX_BYTES)?;
    let name = path.file_stem()?.to_str()?.to_string();
    Some((bytes, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_picture_type_the_clipboard_offers_is_the_one_asked_for() {
        assert_eq!(
            image_type("text/plain\nimage/png\nimage/jpeg\n").as_deref(),
            Some("image/png")
        );
        assert_eq!(image_type("text/plain\nimage/svg+xml\n"), None);
    }
}
