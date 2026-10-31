use kampr_core::push::{self, Pulse};
use std::time::Duration;
use tokio::time::Instant;

#[tokio::test(start_paused = true)]
async fn a_bump_wakes_a_waiter_and_a_deadline_ends_one_that_hears_nothing() {
    let pulse = Pulse::new();
    let mut rx = pulse.subscribe();
    let start = Instant::now();
    assert!(!rx.changed_before(start + Duration::from_secs(5)).await);
    assert_eq!(start.elapsed(), Duration::from_secs(5));

    let bumper = pulse.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        bumper.bump();
    });
    let start = Instant::now();
    assert!(rx.changed_before(start + Duration::from_secs(60)).await);
    assert_eq!(start.elapsed(), Duration::from_secs(1));
    assert!(!rx.moved(), "a change is seen once");
}

#[tokio::test(start_paused = true)]
async fn a_bump_before_the_wait_is_not_lost_and_seen_forgets_it() {
    let pulse = Pulse::new();
    let mut rx = pulse.subscribe();
    pulse.bump();
    assert!(rx.moved());
    assert!(rx.changed_before(Instant::now()).await, "already there");
    pulse.bump();
    rx.seen();
    assert!(!rx.changed_before(Instant::now() + Duration::from_secs(1)).await);
}

#[tokio::test(start_paused = true)]
async fn a_pulse_nobody_can_bump_any_more_waits_out_its_deadline_rather_than_spinning() {
    let pulse = Pulse::new();
    let mut rx = pulse.subscribe();
    drop(pulse);
    let start = Instant::now();
    assert!(!rx.changed_before(start + Duration::from_secs(30)).await);
    assert_eq!(start.elapsed(), Duration::from_secs(30));
    assert!(
        tokio::time::timeout(Duration::from_secs(3600), rx.changed())
            .await
            .is_err(),
        "never wakes"
    );
}

#[tokio::test]
async fn a_watched_file_pulses_on_each_append_and_stops_with_its_watch() {
    let dir = tempfile::tempdir().expect("a dir");
    let path = dir.path().join("transcript.jsonl");
    std::fs::write(&path, b"{}\n").unwrap();
    let watch = push::watch_files(&[dir.path().to_path_buf(), path.clone()]).expect("watched");
    let mut rx = watch.changes();
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    for _ in 0..2 {
        std::io::Write::write_all(&mut file, b"{}\n").unwrap();
        assert!(rx.changed_before(Instant::now() + Duration::from_secs(5)).await);
    }
    std::fs::write(dir.path().join("new"), b"").unwrap();
    assert!(rx.changed_before(Instant::now() + Duration::from_secs(5)).await);
    drop(watch);
    let start = Instant::now();
    assert!(!rx.changed_before(start + Duration::from_secs(1)).await);
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "the stopped watch is closed, not spinning"
    );
}

#[tokio::test]
async fn an_exit_resolves_when_the_process_ends() {
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("sleep");
    let pid = child.id().expect("a pid");
    let exited = push::exited(pid).expect("watched");
    tokio::pin!(exited);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut exited)
            .await
            .is_err(),
        "still running"
    );
    child.start_kill().unwrap();
    tokio::time::timeout(Duration::from_secs(5), exited)
        .await
        .expect("seen to end");
    child.wait().await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), push::exited(pid).expect("asked"))
        .await
        .expect("a process already gone has exited");
}
