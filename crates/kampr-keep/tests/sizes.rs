mod common;

use common::{Live, root, sh, until};
use kampr_keep::{
    How, Launch, Replay, Resize, SEGMENT, Size, SizedReader, borrow, claim_size, size_at, sizes,
};
use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SOON: Duration = Duration::from_secs(10);

const ROUNDS: &str = r#"stty -echo -opost; echo READY; while read k; do echo "SIZE $k $(stty size)"; done"#;

fn in_its_own_dir(script: &str, root: &Path) -> kampr_keep::Spec {
    let mut spec = sh(script);
    spec.cwd = Some(root.join(&spec.id).to_string_lossy().into_owned());
    spec.log_cap = Some(1 << 30);
    spec
}

fn type_line(master: &OwnedFd, line: &str) {
    rustix::io::write(master, format!("{line}\n").as_bytes()).expect("write");
}

fn size(cols: u16, rows: u16, holder: &str) -> Size {
    Size {
        cols,
        rows,
        holder: holder.into(),
    }
}

// The burst is written to the slave as the program would, and the claim follows its last write at
// once, so the keeper still has bytes of it unread when the request arrives.
fn round(live: &Live, master: &OwnedFd, slave: &std::fs::File, k: usize, cols: u16, rows: u16) {
    use std::io::Write;
    let burst: String = (0..3000)
        .map(|i| format!("B {k} {i} ......................................\n"))
        .collect();
    (&*slave).write_all(burst.as_bytes()).expect("burst");
    claim_size(&live.session, cols, rows, &format!("test:{k}")).expect("claim");
    type_line(master, &k.to_string());
    live.wait_for_log(&format!("SIZE {k} {rows} {cols}\n"), SOON);
}

fn slave_of(live: &Live) -> std::fs::File {
    use std::os::unix::fs::OpenOptionsExt;
    let tty = kampr_keep::state(&live.session).expect("state").tty.expect("tty");
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(tty)
        .expect("slave")
}

fn replay(dir: &Path, from: u64) -> Vec<(u64, Size, String)> {
    tail(dir, from, None)
}

fn tail(dir: &Path, from: u64, done: Option<&AtomicBool>) -> Vec<(u64, Size, String)> {
    let mut reader = SizedReader::at(dir, from).expect("reader");
    let mut lines = Vec::new();
    let mut partial = Vec::new();
    let mut line_start = reader.offset();
    let mut buf = vec![0u8; 7919];
    loop {
        let at = reader.offset();
        match reader.read(&mut buf).expect("read") {
            Replay::Data(n) => {
                for (i, &b) in buf[..n].iter().enumerate() {
                    if partial.is_empty() {
                        line_start = at + i as u64;
                    }
                    partial.push(b);
                    if b == b'\n' {
                        let text = String::from_utf8_lossy(&partial).trim_end().to_string();
                        lines.push((line_start, reader.size().clone(), text));
                        partial.clear();
                    }
                }
                assert_eq!(reader.offset(), at + n as u64);
            }
            Replay::Resize(_) => {
                assert!(partial.is_empty(), "a resize split a line at {at}");
                lines.push((at, reader.size().clone(), "RESIZE".into()));
            }
            Replay::Truncated { oldest, size } => {
                lines.push((oldest, size, "TRUNCATED".into()));
            }
            Replay::CaughtUp => match done {
                Some(done)
                    if !done.load(Ordering::Relaxed) || reader.offset() < reader.end().expect("end") =>
                {
                    reader.wait(Duration::from_millis(100)).expect("wait");
                }
                _ => return lines,
            },
        }
    }
}

#[test]
fn a_replay_applies_every_resize_at_the_byte_the_pty_was_resized() {
    let root = root();
    let mut spec = in_its_own_dir(ROUNDS, root.path());
    spec.cols = 80;
    spec.rows = 24;
    let live = Live::spawn(root.path(), spec, Launch::Detached);
    let master = borrow(&live.session).expect("borrow");
    let slave = slave_of(&live);
    live.wait_for_log("READY\n", SOON);
    type_line(&master, "init");
    live.wait_for_log("SIZE init 24 80\n", SOON);
    let done = AtomicBool::new(false);
    let wanted = [(100u16, 30u16), (60, 20), (132, 50), (80, 24)];
    let tailed = std::thread::scope(|s| {
        let tailing = s.spawn(|| tail(live.session.path(), 0, Some(&done)));
        for (k, &(cols, rows)) in wanted.iter().enumerate() {
            round(&live, &master, &slave, k, cols, rows);
        }
        done.store(true, Ordering::Relaxed);
        tailing.join().expect("tail")
    });

    let dir = live.session.path();
    let journal = sizes(dir).expect("journal");
    let got: Vec<(u64, u16, u16, String)> = journal
        .iter()
        .map(|r| (r.seq, r.cols, r.rows, r.holder.clone()))
        .collect();
    let want: Vec<(u64, u16, u16, String)> = wanted
        .iter()
        .enumerate()
        .map(|(k, &(c, r))| (k as u64, c, r, format!("test:{k}")))
        .collect();
    assert_eq!(got, want);
    assert_eq!(live.session.size(), Some(size(80, 24, "test:3")));

    let lines = replay(dir, 0);
    assert_eq!(
        tailed, lines,
        "tailing live saw the resizes elsewhere than a replay does"
    );
    let mut expected = size(80, 24, "spawn");
    let mut resizes = 0;
    let mut bursts = 0;
    for (at, size, text) in &lines {
        if text == "RESIZE" {
            let record = &journal[resizes];
            assert_eq!(*at, record.offset);
            expected = record.size();
            assert_eq!(*size, expected);
            resizes += 1;
            continue;
        }
        if text == "READY" || text == "SIZE init 24 80" {
            continue;
        }
        let k: usize = text
            .split(' ')
            .nth(1)
            .and_then(|k| k.parse().ok())
            .unwrap_or_else(|| panic!("odd line {text:?}"));
        if text.starts_with("B ") {
            assert_eq!(resizes, k, "{text:?} at {at} came after its resize");
            bursts += 1;
        } else {
            assert!(text.starts_with("SIZE "), "{text:?}");
            assert_eq!(resizes, k + 1, "{text:?} at {at} came before its resize");
            let (cols, rows) = wanted[k];
            assert_eq!(text, &format!("SIZE {k} {rows} {cols}"));
        }
        assert_eq!(size, &expected, "{text:?} at {at}");
        assert_eq!(size_at(dir, *at).expect("size_at"), expected, "{text:?} at {at}");
    }
    assert_eq!((resizes, bursts), (wanted.len(), 3000 * wanted.len()));

    for (k, record) in journal.iter().enumerate() {
        let mid = SizedReader::at(dir, record.offset + 7).expect("reader");
        assert_eq!(mid.size(), &record.size(), "record {k}");
        let before = size_at(dir, record.offset - 1).expect("size_at");
        let previous = match k {
            0 => size(80, 24, "spawn"),
            _ => journal[k - 1].size(),
        };
        assert_eq!(before, previous, "record {k}");
    }
}

#[test]
fn a_resize_with_no_output_after_it_still_reaches_a_tailing_reader() {
    let root = root();
    let live = Live::spawn(
        root.path(),
        sh("stty -echo; echo READY; exec cat"),
        Launch::Detached,
    );
    let master = borrow(&live.session).expect("borrow");
    live.wait_for_log("READY", SOON);
    type_line(&master, "hello");
    live.wait_for_log("hello", SOON);
    let mut reader = SizedReader::at(live.session.path(), 0).expect("reader");
    let mut buf = vec![0u8; 4096];
    while reader.read(&mut buf).expect("read") != Replay::CaughtUp {}
    let end = reader.offset();

    let waiter = std::thread::spawn(move || {
        let woke = reader.wait(SOON).expect("wait");
        (woke, reader.read(&mut buf).expect("read"), reader)
    });
    std::thread::sleep(Duration::from_millis(200));
    claim_size(&live.session, 111, 33, "phone:test").expect("claim");
    let (woke, read, reader) = waiter.join().expect("waiter");
    assert!(woke, "the reader slept through a resize");
    match read {
        Replay::Resize(Resize {
            offset,
            cols: 111,
            rows: 33,
            ..
        }) => assert_eq!(offset, end),
        other => panic!("{other:?}"),
    }
    assert_eq!(reader.size(), &size(111, 33, "phone:test"));
}

#[test]
fn trimming_the_log_keeps_the_latest_resize_before_its_first_byte() {
    let root = root();
    let mut spec = sh("stty -echo -opost; echo READY; while read n; do seq 1 $n; echo DONE $n; done");
    spec.log_cap = Some(2 * SEGMENT);
    let live = Live::spawn(root.path(), spec, Launch::Detached);
    let master = borrow(&live.session).expect("borrow");
    live.wait_for_log("READY\n", SOON);
    for (k, (cols, rows)) in [(90u16, 30u16), (70, 25), (120, 40)].into_iter().enumerate() {
        claim_size(&live.session, cols, rows, &format!("early:{k}")).expect("claim");
        type_line(&master, &(10 + k).to_string());
        live.wait_for_log(&format!("DONE {}\n", 10 + k), SOON);
    }
    type_line(&master, "500000");
    live.wait_for_log("DONE 500000\n", SOON);
    claim_size(&live.session, 100, 35, "late").expect("claim");

    let dir = live.session.path();
    let start = SizedReader::at(dir, 0).expect("reader").start().expect("start");
    assert!(start > 0, "nothing was trimmed");
    let journal = sizes(dir).expect("journal");
    let kept: Vec<(u64, u16, String)> = journal
        .iter()
        .map(|r| (r.seq, r.cols, r.holder.clone()))
        .collect();
    assert_eq!(kept, vec![(2, 120, "early:2".into()), (3, 100, "late".into())]);
    assert!(journal[0].offset < start && journal[1].offset > start);
    assert_eq!(size_at(dir, start).expect("size_at"), size(120, 40, "early:2"));

    let lines = replay(dir, 0);
    assert_eq!(lines[0], (start, size(120, 40, "early:2"), "TRUNCATED".into()));
    let resizes: Vec<&(u64, Size, String)> = lines.iter().filter(|l| l.2 == "RESIZE").collect();
    assert_eq!(resizes.len(), 1);
    assert_eq!(resizes[0].1, size(100, 35, "late"));
}

#[test]
fn a_re_execd_keeper_carries_on_the_same_journal() {
    let root = root();
    let live = Live::spawn(
        root.path(),
        sh("stty -echo -opost; echo READY; while read l; do echo \"SIZE $(stty size)\"; done"),
        Launch::Detached,
    );
    let master = borrow(&live.session).expect("borrow");
    live.wait_for_log("READY\n", SOON);
    let keeper = live.keeper_pid();
    claim_size(&live.session, 101, 31, "before").expect("claim");
    type_line(&master, "x");
    live.wait_for_log("SIZE 31 101\n", SOON);

    unsafe { libc::kill(keeper as i32, libc::SIGUSR1) };
    assert!(until(SOON, || live
        .session
        .keeper()
        .is_some_and(|k| k.how == How::ReExec && k.keeper == keeper)));
    claim_size(&live.session, 77, 22, "after").expect("claim after the re-exec");
    type_line(&master, "x");
    live.wait_for_log("SIZE 22 77\n", SOON);

    let journal = sizes(live.session.path()).expect("journal");
    let got: Vec<(u64, u16, String)> = journal
        .iter()
        .map(|r| (r.seq, r.cols, r.holder.clone()))
        .collect();
    assert_eq!(got, vec![(0, 101, "before".into()), (1, 77, "after".into())]);
    assert!(journal[0].offset < journal[1].offset);
}

#[test]
fn a_zero_size_is_refused_and_journals_nothing() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let err = claim_size(&live.session, 0, 24, "zero").expect_err("a zero-width claim");
    assert!(err.to_string().contains("size"), "{err}");
    assert!(sizes(live.session.path()).expect("journal").is_empty());
}
