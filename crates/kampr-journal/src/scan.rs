use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::attach::MAX_RECORD_BYTES;

/// Whole records off a transcript, one string at a time, with the same ceiling the fetch path
/// reads to: a record longer than [`MAX_RECORD_BYTES`] could never be served back either, so it
/// is discarded to its newline rather than held whole — a 204.8 MB single-line transcript is a
/// real shape.
///
/// **This yields exactly what [`crate::tail::FileJournal`] hands its parser, in the same order
/// and with the same lines left out.** A facet that names a turn by the id the parser minted from
/// a line's position depends on that: a line skipped here and kept there shifts every id after it.
pub fn records(transcript: &Path) -> impl Iterator<Item = String> {
    let mut reader = File::open(transcript).ok().map(BufReader::new);
    std::iter::from_fn(move || {
        let reader = reader.as_mut()?;
        loop {
            let mut buf = Vec::new();
            let read = reader
                .take(MAX_RECORD_BYTES + 1)
                .read_until(b'\n', &mut buf)
                .ok()?;
            if read == 0 {
                return None;
            }
            if !buf.ends_with(b"\n") && read as u64 > MAX_RECORD_BYTES {
                skip_record(reader);
                continue;
            }
            let text = String::from_utf8_lossy(&buf);
            let text = text.trim_end_matches('\n').trim_end_matches('\r');
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    })
}

fn skip_record(reader: &mut impl BufRead) {
    loop {
        let (done, used) = match reader.fill_buf() {
            Err(_) | Ok([]) => return,
            Ok(buf) => match buf.iter().position(|b| *b == b'\n') {
                Some(at) => (true, at + 1),
                None => (false, buf.len()),
            },
        };
        reader.consume(used);
        if done {
            return;
        }
    }
}
