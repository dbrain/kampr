//! An answer on its way out, as a stream of chunks rather than a buffer: the one shape the HTTP
//! route, the mesh pump and the hub's relay all hand bytes along in.

use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::pin::Pin;

use futures_util::Stream;
use kampr_journal::attach::{FILE, IMAGE, image_mime};
use kampr_journal::{Fetched, JournalError};
use kampr_mesh::Span;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// One piece of a body, and the most any one step of any stream here holds.
pub const CHUNK: usize = kampr_mesh::ATT_CHUNK_BYTES;

/// A file is never read whole, so this is not a memory bound; it is a sanity bound on what one
/// request may ask a node's disk for, far past anything a person downloads to a phone.
pub const STREAM_CEILING: u64 = 16 * 1024 * 1024 * 1024;

pub type Chunks = Pin<Box<dyn Stream<Item = io::Result<Vec<u8>>> + Send>>;

pub struct Served {
    pub kind: String,
    pub mime: Option<String>,
    pub name: Option<String>,
    /// Bytes in this body, or `None` when nothing knows until it ends.
    pub length: Option<u64>,
    pub slice: Option<Slice>,
    /// Whether a `Range` would have been honoured, which is what `Accept-Ranges` says.
    pub ranges: bool,
    pub body: Chunks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slice {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

#[derive(Debug)]
pub enum Refused {
    Missing,
    TooLarge(u64),
    Unsatisfiable(u64),
}

impl From<JournalError> for Refused {
    fn from(e: JournalError) -> Self {
        match e {
            JournalError::TooLarge(bytes) => Self::TooLarge(bytes),
            _ => Self::Missing,
        }
    }
}

/// `bytes=START-END` and `bytes=START-`. A suffix range, several ranges, or anything else is not
/// an error: RFC 9110 lets a server ignore a `Range` it does not serve, and the whole body is a
/// correct answer to every one of them.
pub fn parse_range(header: &str) -> Option<Span> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => None,
        end => Some(end.parse::<u64>().ok().filter(|end| *end >= start)?),
    };
    Some(Span { start, end })
}

fn slice_of(span: Span, total: u64) -> Result<Slice, Refused> {
    if span.start >= total {
        return Err(Refused::Unsatisfiable(total));
    }
    let end = span.end.map_or(total - 1, |end| end.min(total - 1));
    Ok(Slice {
        start: span.start,
        end,
        total,
    })
}

impl Served {
    pub fn whole(found: Fetched) -> Self {
        let Fetched {
            kind,
            mime,
            name,
            data,
        } = found;
        let length = Some(data.len() as u64);
        let body = futures_util::stream::unfold((data, 0usize), |(data, at)| async move {
            if at >= data.len() {
                return None;
            }
            let end = (at + CHUNK).min(data.len());
            let chunk = data[at..end].to_vec();
            Some((Ok(chunk), (data, end)))
        });
        Self {
            kind,
            mime,
            name,
            length,
            slice: None,
            ranges: false,
            body: Box::pin(body),
        }
    }

    /// A regular file already opened, read a chunk at a time as the body is polled.
    ///
    /// The type is decided from the file's own head whatever slice is asked for, so a range that
    /// starts mid-file is typed as the file is rather than by whatever bytes it happens to open on.
    pub fn file(path: &Path, file: std::fs::File, total: u64, span: Option<Span>) -> Result<Self, Refused> {
        if total == 0 {
            return Err(Refused::Missing);
        }
        if total > STREAM_CEILING {
            return Err(Refused::TooLarge(total));
        }
        let slice = span.map(|span| slice_of(span, total)).transpose()?;
        let (start, length) = match slice {
            Some(slice) => (slice.start, slice.end - slice.start + 1),
            None => (0, total),
        };
        let mut head = [0u8; 64];
        let seen = file.read_at(&mut head, 0).map_err(|_| Refused::Missing)?;
        let mime = image_mime(path).or_else(|| crate::attach::sniff(&head[..seen]));
        let kind = match image_mime(path) {
            Some(_) => IMAGE,
            None => FILE,
        };
        let file = tokio::fs::File::from_std(file);
        let body = futures_util::stream::unfold((file, start, length), |(mut file, at, left)| async move {
            if left == 0 {
                return None;
            }
            let read = async {
                if at > 0 {
                    file.seek(io::SeekFrom::Start(at)).await?;
                }
                let mut chunk = vec![0u8; CHUNK.min(left as usize)];
                let mut filled = 0;
                while filled < chunk.len() {
                    match file.read(&mut chunk[filled..]).await? {
                        0 => break,
                        n => filled += n,
                    }
                }
                // A file that shrank under the read must end the body short of its
                // `Content-Length` rather than pad it.
                if filled == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                chunk.truncate(filled);
                Ok(chunk)
            };
            match read.await {
                Ok(chunk) => {
                    let n = chunk.len() as u64;
                    Some((Ok(chunk), (file, 0, left - n)))
                }
                Err(e) => Some((Err(e), (file, 0, 0))),
            }
        });
        Ok(Self {
            kind: kind.to_string(),
            mime: mime.map(str::to_string),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            length: Some(length),
            slice,
            ranges: true,
            body: Box::pin(body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_range_forms_parse_and_everything_else_is_ignored() {
        assert_eq!(
            parse_range("bytes=5-9"),
            Some(Span {
                start: 5,
                end: Some(9)
            })
        );
        assert_eq!(parse_range("bytes=5-"), Some(Span { start: 5, end: None }));
        for ignored in ["bytes=-5", "bytes=9-5", "bytes=1-2,4-5", "items=1-2", "bytes=x-"] {
            assert_eq!(parse_range(ignored), None, "{ignored}");
        }
    }

    #[test]
    fn an_end_past_the_file_is_clamped_and_a_start_past_it_is_unsatisfiable() {
        let span = |start, end| Span { start, end };
        let slice = slice_of(span(5, Some(100)), 10).expect("a slice");
        assert_eq!((slice.start, slice.end, slice.total), (5, 9, 10));
        assert!(matches!(
            slice_of(span(10, None), 10),
            Err(Refused::Unsatisfiable(10))
        ));
    }
}
