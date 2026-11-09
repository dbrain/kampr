//! Against the machine's own Docker daemon, with throwaway `kt-*` containers. Skipped where there
//! is no daemon or no `nats:2.15.2-alpine` to make them from.

use kampr_core::wire::ContainerEntry;
use kampr_docker::{Client, Directory, Journal};
use kampr_testkit::docker::Container;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

mod pull;
mod shell;

const SOON: Duration = Duration::from_secs(30);

fn available() -> bool {
    kampr_docker::socket::resolve("", std::env::var("DOCKER_HOST").ok().as_deref()).exists()
        && kampr_testkit::docker::available()
}

fn follow(journal: &Path) -> Arc<Directory> {
    let socket = kampr_docker::socket::resolve("", std::env::var("DOCKER_HOST").ok().as_deref());
    Directory::follow(Client::new(socket), Journal::open(journal), false)
}

async fn until(directory: &Directory, name: &str, want: impl Fn(&ContainerEntry) -> bool) -> ContainerEntry {
    let mut revision = directory.subscribe();
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let found = directory.containers().into_iter().find(|c| c.name == name);
        if let Some(found) = found.filter(|c| want(c)) {
            return found;
        }
        let last = directory.containers().into_iter().find(|c| c.name == name);
        assert!(
            tokio::time::Instant::now() < deadline,
            "{name} never as wanted: {last:?}"
        );
        let _ = tokio::time::timeout(Duration::from_millis(500), revision.changed()).await;
    }
}

#[tokio::test]
async fn a_crash_loop_is_journalled_and_counted() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let directory = follow(&dir.path().join("events.jsonl"));
    until_up(&directory).await;
    let looping = Container::run(&["--restart", "on-failure:3", "--", "exit 3"]);
    let entry = until(&directory, &looping.0, |c| c.crashes >= 4 && c.state == "exited").await;
    assert_eq!(entry.restarts, 3);
    assert_eq!(entry.exit_code, Some(3));
    let last = entry.last_crash.expect("a last crash");
    assert_eq!((last.exit_code, last.oom), (3, false));
    let journal = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
    let dies = journal
        .lines()
        .filter(|l| l.contains(&looping.0) && l.contains("\"action\":\"die\""))
        .count();
    assert_eq!(dies, 4, "{journal}");
}

#[tokio::test]
async fn an_oom_kill_says_so() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let directory = follow(&dir.path().join("events.jsonl"));
    until_up(&directory).await;
    let oom = Container::run(&["-m", "16m", "--", r#"x=a; while true; do x="$x$x"; done"#]);
    let entry = until(&directory, &oom.0, |c| c.last_crash.is_some()).await;
    assert!(entry.oom, "{entry:?}");
    assert_eq!(entry.exit_code, Some(137));
    assert_eq!(entry.last_crash.map(|c| (c.exit_code, c.oom)), Some((137, true)));
}

#[tokio::test]
async fn a_node_that_was_away_reads_what_it_missed_from_where_it_stopped() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let first = follow(&path);
    until_up(&first).await;
    let marker = Container::run(&["--", "sleep 600"]);
    let entry = until(&first, &marker.0, |c| c.state == "running").await;
    let deadline = tokio::time::Instant::now() + SOON;
    while first.events_of(&entry.id, 1).is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the marker's start was never journalled"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(first);

    let missed = Container::run(&["--", "exit 5"]);
    wait_exited(&missed).await;
    let second = follow(&path);
    let entry = until(&second, &missed.0, |c| c.last_crash.is_some()).await;
    assert_eq!(entry.last_crash.unwrap().exit_code, 5);
}

#[tokio::test]
async fn a_detail_reads_logs_stats_env_and_the_journal() {
    if !available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let directory = follow(&dir.path().join("events.jsonl"));
    until_up(&directory).await;
    let running = Container::run(&[
        "-e",
        "KT_SEEN=yes",
        "--",
        "echo hello-kt; echo oops >&2; sleep 600",
    ]);
    let entry = until(&directory, &running.0, |c| c.state == "running").await;
    let detail = kampr_docker::detail(&directory, &entry.id).await.unwrap();
    assert_eq!(detail["name"], running.0.as_str());
    assert!(
        detail["env"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "KT_SEEN=yes"),
        "{detail}"
    );
    let lines: Vec<(String, String)> = detail["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["stream"].as_str().unwrap().into(),
                l["line"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert!(lines.contains(&("stdout".into(), "hello-kt".into())), "{lines:?}");
    assert!(lines.contains(&("stderr".into(), "oops".into())), "{lines:?}");
    assert!(detail["stats"]["mem_used"].as_u64().unwrap() > 0, "{detail}");
    assert!(detail["stats"]["pids"].as_u64().unwrap() >= 1, "{detail}");
    assert_eq!(
        detail["events"].as_array().unwrap().last().unwrap()["action"],
        "start"
    );
}

#[tokio::test]
async fn a_socket_nobody_answers_is_an_empty_list_and_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let journal = || Journal::open(&dir.path().join("events.jsonl"));
    let missing = dir.path().join("no-docker.sock");
    let optional = Directory::follow(Client::new(&missing), journal(), false);
    let required = Directory::follow(Client::new(&missing), journal(), true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(optional.reported(), None);
    assert_eq!(required.reported(), Some(false));
    assert!(required.containers().is_empty());
}

async fn until_up(directory: &Directory) {
    let deadline = tokio::time::Instant::now() + SOON;
    while directory.reported() != Some(true) {
        assert!(tokio::time::Instant::now() < deadline, "docker never answered");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_exited(container: &Container) {
    let deadline = tokio::time::Instant::now() + SOON;
    while container.status() != "exited" {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} never exited",
            container.0
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
