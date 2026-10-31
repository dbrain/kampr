use kampr_watch::{Event, Exit, Waited, Watch};
use std::time::{Duration, Instant};

const SOON: Duration = Duration::from_secs(5);

/// Everything the watch reports until `want` holds of one event, or `SOON` passes.
fn seen(watch: &mut Watch, want: impl Fn(&Event) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + SOON;
    let mut all = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match watch.wait(Some(left)).expect("a wait") {
            Waited::Changed(events) => {
                let done = events.iter().any(&want);
                all.extend(events);
                if done {
                    return all;
                }
            }
            Waited::TimedOut => break,
            Waited::Stopped => panic!("stopped"),
        }
    }
    panic!("never saw it; saw {all:?}");
}

fn named(event: &Event, name: &str) -> bool {
    event.name.as_deref().is_none_or(|n| n == name)
}

#[test]
fn a_directory_reports_entries_coming_going_and_written() {
    let dir = tempfile::tempdir().expect("a dir");
    let (mut watch, _stop) = Watch::new().expect("a watch");
    let key = watch.add(dir.path()).expect("watched");
    std::fs::write(dir.path().join("a.tmp"), b"x").unwrap();
    std::fs::rename(dir.path().join("a.tmp"), dir.path().join("a")).unwrap();
    seen(&mut watch, |e| e.key == key && named(e, "a"));
    std::fs::remove_file(dir.path().join("a")).unwrap();
    seen(&mut watch, |e| e.key == key && named(e, "a"));
}

#[test]
fn a_file_reports_each_append() {
    let dir = tempfile::tempdir().expect("a dir");
    let path = dir.path().join("transcript.jsonl");
    std::fs::write(&path, b"{}\n").unwrap();
    let (mut watch, _stop) = Watch::new().expect("a watch");
    let key = watch.add(&path).expect("watched");
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    for _ in 0..2 {
        std::io::Write::write_all(&mut file, b"{}\n").unwrap();
        seen(&mut watch, |e| e.key == key && !e.gone);
    }
}

#[test]
fn a_removed_path_is_reported_gone_once() {
    let dir = tempfile::tempdir().expect("a dir");
    let sub = dir.path().join("session");
    std::fs::create_dir(&sub).unwrap();
    let (mut watch, _stop) = Watch::new().expect("a watch");
    let key = watch.add(&sub).expect("watched");
    std::fs::remove_dir(&sub).unwrap();
    seen(&mut watch, |e| e.key == key && e.gone);
    assert!(
        !matches!(watch.wait(Some(Duration::from_millis(100))).unwrap(), Waited::Changed(e) if e.iter().any(|e| e.key == key)),
        "a dead key says nothing more"
    );
}

#[test]
fn dropping_the_stop_ends_a_wait_with_no_deadline_at_once() {
    let dir = tempfile::tempdir().expect("a dir");
    let (mut watch, stop) = Watch::new().expect("a watch");
    watch.add(dir.path()).expect("watched");
    let waiter = std::thread::spawn(move || {
        let started = Instant::now();
        let first = watch.wait(None).expect("a wait");
        let again = watch.wait(None).expect("a wait");
        (first, again, started.elapsed())
    });
    std::thread::sleep(Duration::from_millis(50));
    let dropped = Instant::now();
    drop(stop);
    let (first, again, _) = waiter.join().unwrap();
    assert_eq!((first, again), (Waited::Stopped, Waited::Stopped));
    assert!(
        dropped.elapsed() < Duration::from_millis(100),
        "took {:?}",
        dropped.elapsed()
    );
}

fn sleeper(secs: &str) -> std::process::Child {
    std::process::Command::new("sleep")
        .arg(secs)
        .spawn()
        .expect("sleep")
}

#[test]
fn an_exit_is_seen_when_it_happens_and_not_before() {
    let mut child = sleeper("30");
    let exit = Exit::open(child.id()).expect("opened").expect("running");
    assert!(
        !exit.wait(Some(Duration::from_millis(50))).unwrap(),
        "still running"
    );
    child.kill().unwrap();
    assert!(exit.wait(Some(SOON)).unwrap());
    child.wait().unwrap();
}

#[test]
fn a_process_already_gone_is_none() {
    let mut child = sleeper("0");
    let pid = child.id();
    child.wait().unwrap();
    assert!(Exit::open(pid).expect("asked").is_none());
}

#[test]
fn a_watch_taken_off_wakes_nobody_with_nothing() {
    let dir = tempfile::tempdir().expect("a dir");
    let (mut watch, _stop) = Watch::new().expect("a watch");
    let key = watch.add(dir.path()).expect("watched");
    watch.remove(key);
    match watch.wait(Some(Duration::from_millis(200))).expect("a wait") {
        Waited::TimedOut => {}
        other => panic!("woke for a watch it was told to drop: {other:?}"),
    }
}
