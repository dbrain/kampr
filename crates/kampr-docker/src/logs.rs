use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
    pub at: i64,
    pub stream: &'static str,
    pub line: String,
}

pub fn parse(bytes: &[u8], tty: bool) -> Vec<LogLine> {
    let mut out = Vec::new();
    match tty {
        true => push(&mut out, "stdout", bytes),
        false => {
            let mut rest = bytes;
            while rest.len() >= 8 {
                let size = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
                let Some(payload) = rest.get(8..8 + size) else {
                    break;
                };
                let stream = if rest[0] == 2 { "stderr" } else { "stdout" };
                push(&mut out, stream, payload);
                rest = &rest[8 + size..];
            }
        }
    }
    out
}

fn push(out: &mut Vec<LogLine>, stream: &'static str, payload: &[u8]) {
    let text = String::from_utf8_lossy(payload);
    for raw in text.split_terminator('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let stamped = raw
            .split_once(' ')
            .and_then(|(at, line)| Some((crate::api::unix(at)?, line)));
        let (at, line) = match stamped {
            Some((at, line)) => (at, line),
            None => (out.last().map_or(0, |l: &LogLine| l.at), raw),
        };
        out.push(LogLine {
            at,
            stream,
            line: line.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(logs: &[LogLine]) -> Vec<(&str, &str)> {
        logs.iter().map(|l| (l.stream, l.line.as_str())).collect()
    }

    #[test]
    fn a_container_without_a_tty_is_demultiplexed_into_its_two_streams() {
        let logs = parse(include_bytes!("../tests/fixtures/logs-mux.bin"), false);
        assert_eq!(logs.len(), 6, "{logs:?}");
        assert!(lines(&logs).contains(&("stdout", "out 7")), "{logs:?}");
        assert!(lines(&logs).contains(&("stderr", "err 7")), "{logs:?}");
        assert_eq!(logs[0].at, 1794219837);
        assert_eq!(logs[5].at, 1794219838);
    }

    #[test]
    fn a_tty_container_is_one_raw_stream() {
        let logs = parse(include_bytes!("../tests/fixtures/logs-tty.bin"), true);
        assert_eq!(lines(&logs), vec![("stdout", "hello tty")]);
        assert_eq!(logs[0].at, 1794219833);
    }

    #[test]
    fn a_frame_cut_short_keeps_what_came_before_it() {
        let mut bytes = include_bytes!("../tests/fixtures/logs-mux.bin").to_vec();
        bytes.truncate(bytes.len() - 3);
        assert_eq!(parse(&bytes, false).len(), 5);
    }
}
