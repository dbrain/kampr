use serde_json::Value;

use crate::model::Attachment;

const SUMMARY_KEYS: &[&str] = &[
    "description",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "cmd",
    "command",
    "prompt",
];

pub fn summarise(input: &Value) -> Option<String> {
    let object = input.as_object()?;
    SUMMARY_KEYS
        .iter()
        .find_map(|k| object.get(*k).and_then(Value::as_str))
        .map(one_line)
}

pub fn one_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() <= 120 {
        return first.to_string();
    }
    let cut: String = first.chars().take(119).collect();
    format!("{cut}…")
}

pub fn count_lines(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    Some(text.lines().count() as u32)
}

/// What one tool result may put on the socket that carries terminal frames.
///
/// A result has no bound of its own — probe #247 measured a single **2.22 MB** record inside an
/// 88.7 MB rollout — and probe #257 measured what one big message costs a reader: 1 MiB sent as a
/// single message stopped a pane repainting for **1.318 s** on a 1 MB/s link, against an 84.6 ms
/// worst frame when the same bytes went as 64 KiB chunks. So the budget is a page's, divided by
/// the page: `kampr-node` serves 40 turns at a time, and 40 × 8 KiB is 320 KiB spread over 40
/// messages rather than one.
///
/// The line cap is the one that bites on the shape a reader actually meets — a `grep`, a test run
/// — where the lines are short and 8 KiB is several hundred of them. 120 is three screenfuls of
/// the 40-row grid the node reads off a pane, which is past the point where a reader opens the
/// pane instead.
const OUTPUT_BYTES: usize = 8 * 1024;
const OUTPUT_LINES: usize = 120;

/// The head of `text` within both caps, cut on a line boundary where there is one and on a
/// character boundary where there is not — a single unbroken line is a real shape (`cat` of a
/// minified file) and `String` indexing mid-character is a panic rather than a long block.
///
/// The head and not the tail: a client is told the true total on the card beside this, so what it
/// needs from the block is the beginning a reader can recognise it by.
pub fn clip(text: &str) -> String {
    let mut out = String::new();
    for (n, line) in text.lines().take(OUTPUT_LINES).enumerate() {
        let sep = usize::from(n > 0);
        if out.len() + sep + line.len() > OUTPUT_BYTES {
            let room = OUTPUT_BYTES.saturating_sub(out.len() + sep);
            let cut = floor_boundary(line, room);
            if cut > 0 {
                if sep == 1 {
                    out.push('\n');
                }
                out.push_str(&line[..cut]);
            }
            break;
        }
        if sep == 1 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn floor_boundary(line: &str, mut at: usize) -> usize {
    if at >= line.len() {
        return line.len();
    }
    while at > 0 && !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// An image the wire has no way to carry. It is an `md` block rather than a block of its own
/// because a client that does not know a `b` value drops the block silently, and the phones
/// already installed would go on showing a turn with its screenshot missing and nothing said.
pub fn image_marker(subtype: Option<&str>) -> String {
    match subtype {
        Some(kind) => format!("[image · {kind}]"),
        None => "[image]".to_string(),
    }
}

pub fn marker_of(att: &Attachment) -> String {
    image_marker(att.mime.as_deref().and_then(|m| m.strip_prefix("image/")))
}
