//! A container adopted (docs/19 "Adoption"): this tree's own static `kampr` copied into a real
//! container and run there as a node of its own, hosted over `docker exec`. Left out of the fast
//! profile, since the static build is minutes the first time.

use super::*;
use kampr_auth::MeshRole;
use kampr_testkit::docker::{Container, available};
use std::process::Command;
use std::sync::OnceLock;

const UP: Duration = Duration::from_secs(90);
/// Under the supervisor's 30 s idle wait, so only the `start` event's kick brings a child back
/// in time.
const BACK: Duration = Duration::from_secs(20);
/// Long enough for the supervisor to have found the container stopped and settled into that wait.
const PAST_BACKOFF: Duration = Duration::from_secs(3);
static STATIC: OnceLock<PathBuf> = OnceLock::new();
static RELEASES: OnceLock<String> = OnceLock::new();

fn adopting(config: &mut Config) {
    config.docker.enabled = None;
    config.auth.audit = true;
    config.docker.binary = STATIC.get().expect("a static kampr").display().to_string();
}

fn container_id(name: &str) -> String {
    let out = Command::new("docker")
        .args(["inspect", "-f", "{{.Id}}", name])
        .output()
        .expect("docker");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn docker<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(args: &[S]) {
    let out = Command::new("docker").args(args).output().expect("docker");
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

impl Client {
    async fn adopt(&mut self, name: &str) -> Value {
        self.send(json!({ "t": "manage", "op": "docker.adopt", "node": NODE, "id": name }))
            .await;
        self.until_within("the adoption's ack", UP, |c| {
            let at = c.others.iter().position(|v| v["t"] == "managed")?;
            Some(c.others.remove(at))
        })
        .await
    }

    async fn reachable_within(&mut self, id: &str, within: Duration) -> Value {
        self.until_within(&format!("{id} back"), within, |c| {
            c.nodes
                .get(id)
                .filter(|n| n["reachable"].as_bool().unwrap_or(true))
                .cloned()
        })
        .await
    }

    async fn reachable(&mut self, id: &str, want: bool) -> Value {
        self.until_within(&format!("{id} reachable={want}"), UP, |c| {
            c.nodes
                .get(id)
                .filter(|n| n["reachable"].as_bool().unwrap_or(true) == want)
                .cloned()
        })
        .await
    }
}

fn adopted_json(h: &Harness) -> Vec<Value> {
    let path = h.home.as_ref().unwrap().path().join("state/docker/adopted.json");
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

async fn settles(what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + UP;
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_adopted_container_is_a_node_of_its_own_until_it_is_released() {
    if !available() {
        return;
    }
    match kampr_testkit::static_kampr() {
        Ok(binary) => STATIC.get_or_init(|| binary),
        Err(why) => {
            eprintln!("skipped: no static kampr to copy into a container: {why}");
            return;
        }
    };
    let h = Harness::start_with(Harness::home(), adopting).await;
    let mut client = h.client().await;
    let hostname = kampr_testkit::docker::name().replace("kt-", "kth");
    let running = Container::run(&["--hostname", &hostname, "--", "sleep 600 & wait"]);
    let id = container_id(&running.0);

    let ack = client.adopt(&running.0).await;
    assert_eq!(ack["ok"], true, "{ack}");
    let child = format!("{NODE}-c-{}", &id[..12]);
    assert_eq!(ack["node"], child.as_str(), "{ack}");
    let node = client.reachable(&child, true).await;
    assert_eq!(node["parent"], NODE, "{node}");
    assert_eq!(node["name"], running.0.as_str(), "{node}");
    let entry = client
        .until("the container naming its child", |c| {
            c.nodes.get(NODE)?["containers"]
                .as_array()?
                .iter()
                .find(|e| e["name"] == running.0.as_str() && e["child"] == child.as_str())
                .cloned()
        })
        .await;
    assert_eq!(entry["id"], id.as_str(), "{entry}");
    assert_eq!(adopted_json(&h)[0]["name"], running.0.as_str());

    let ack = client
        .manage(json!({ "op": "native.create", "node": child, "args": ["sh", "-c", "echo in-$(cat /etc/hostname); exec cat"] }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    let pane = ack["id"].as_str().expect("a pane id").to_string();
    assert!(pane.starts_with(&format!("{child}/n:")), "{pane}");
    client.pane(&pane, |_| true).await;
    client.watch(&pane, false).await;
    client.shows(&pane, &format!("in-{hostname}")).await;

    docker(&["restart", "-t", "1", &running.0]);
    client.reachable(&child, false).await;
    let back = client.reachable_within(&child, BACK).await;
    assert_eq!(back["parent"], NODE, "{back}");

    docker(&["stop", "-t", "1", &running.0]);
    client.reachable(&child, false).await;
    tokio::time::sleep(PAST_BACKOFF).await;
    docker(&["start", &running.0]);
    let back = client.reachable_within(&child, BACK).await;
    assert_eq!(back["parent"], NODE, "{back}");

    docker(&["rm", "-f", &running.0]);
    client.reachable(&child, false).await;
    tokio::time::sleep(PAST_BACKOFF).await;
    docker(
        &[
            "run",
            "-d",
            "--name",
            &running.0,
            "--hostname",
            &hostname,
            "--entrypoint",
            "sh",
        ]
        .into_iter()
        .chain([kampr_testkit::docker::IMAGE, "-c", "sleep 600 & wait"])
        .collect::<Vec<_>>(),
    );
    assert_ne!(container_id(&running.0), id, "recreated");
    let back = client.reachable_within(&child, BACK).await;
    assert_eq!(
        back["parent"], NODE,
        "the same node in the recreated container: {back}"
    );

    let h = h.restart_with(adopting).await;
    let mut client = h.client().await;
    let resumed = client.reachable(&child, true).await;
    assert_eq!(
        resumed["parent"], NODE,
        "re-adopted when the host restarted: {resumed}"
    );

    let ack = client
        .manage(json!({ "op": "docker.release", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    let node = h.node.clone();
    let gone = child.clone();
    settles("the released child gone from the herd", move || {
        node.herd().node(&gone).is_none()
    })
    .await;
    assert!(adopted_json(&h).is_empty());
    let key = h
        .home
        .as_ref()
        .unwrap()
        .path()
        .join(format!("state/docker/keys/{}.key", running.0));
    assert!(!key.exists(), "{}", key.display());
    let peers = h.node.auth.store().mesh().nodes(MeshRole::Peer).await.unwrap();
    let enrolled = peers
        .iter()
        .find(|p| p.node_id == child)
        .expect("the child's enrolment");
    assert!(!enrolled.active(), "{enrolled:?}");

    let audit = std::fs::read_to_string(h.home.as_ref().unwrap().path().join("state/audit.jsonl")).unwrap();
    for op in ["docker.adopt", "docker.release"] {
        let line = audit
            .lines()
            .find(|l| l.contains(op))
            .unwrap_or_else(|| panic!("{op}: {audit}"));
        assert!(line.contains(&running.0), "{op} without its container: {line}");
    }
}

fn sha256_of(path: &str, container: Option<&str>) -> String {
    let out = match container {
        Some(name) => Command::new("docker")
            .args(["exec", name, "sha256sum", path])
            .output(),
        None => Command::new("sha256sum").arg(path).output(),
    }
    .expect("sha256sum");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The published release, fetched from GitHub and verified by the embedded installer exactly as
/// `kampr update` would verify it.
#[tokio::test]
async fn the_published_release_is_fetched_and_verified() {
    let root = tempfile::tempdir().unwrap();
    let releases = kampr_node::adopt::Releases::new(root.path(), "0.1.139", "dbrain/kampr", None);
    let fetched = releases
        .fetch("x86_64-unknown-linux-musl")
        .await
        .expect("v0.1.139's x86_64 musl release");
    let elf = kampr_docker::elf::read(&fetched).unwrap();
    assert_eq!(elf.arch, "amd64");
    assert!(!elf.interpreted, "{elf:?}");
}

/// A host whose own binary and `[docker] binary` are glibc builds is given this version's
/// release instead, verified once and kept; and an adopted container that is removed is listed
/// as failing until it is released.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_with_no_static_kampr_adopts_with_its_release_and_says_when_that_cannot_come_back() {
    if !available() {
        return;
    }
    let built = match kampr_testkit::static_kampr() {
        Ok(binary) => binary,
        Err(why) => {
            eprintln!("skipped: no static kampr to publish: {why}");
            return;
        }
    };
    let published = tempfile::tempdir().unwrap();
    let arch = std::env::consts::ARCH;
    let target = format!("{arch}-unknown-linux-musl");
    let release = kampr_testkit::release::Release::of(published.path(), &target, &built);
    RELEASES.set(release.base()).expect("one published release");
    let h = Harness::start_with(Harness::home(), |config| {
        config.docker.enabled = None;
        config.docker.binary = kampr_testkit::kampr().display().to_string();
        config.docker.release_base = RELEASES.get().cloned();
    })
    .await;
    let mut client = h.client().await;
    let running = Container::run(&["--", "sleep 600 & wait"]);

    let ack = client.adopt(&running.0).await;
    assert_eq!(ack["ok"], true, "{ack}");
    let child = ack["node"].as_str().expect("the child's id").to_string();
    client.reachable(&child, true).await;

    let cached = h
        .home
        .as_ref()
        .unwrap()
        .path()
        .join("state/docker/bin")
        .join(kampr_node::state::BUILD)
        .join(&target)
        .join("kampr");
    let want = sha256_of(&built.display().to_string(), None);
    assert!(!want.is_empty());
    assert_eq!(
        sha256_of(&cached.display().to_string(), None),
        want,
        "{}",
        cached.display()
    );
    assert_eq!(
        sha256_of("/.kampr/kampr", Some(&running.0)),
        want,
        "the container holds the fetched release"
    );

    docker(&["rm", "-f", &running.0]);
    let missing = client
        .until_within("the removed container listed as failing", UP, |c| {
            c.nodes.get(NODE)?["containers"]
                .as_array()?
                .iter()
                .find(|e| e["name"] == running.0.as_str() && e["adopt_error"].is_string())
                .cloned()
        })
        .await;
    assert_eq!(missing["state"], "missing", "{missing}");
    assert_eq!(missing["child"], child.as_str(), "{missing}");
    assert!(
        missing["adopt_error"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{} no longer exists", running.0)),
        "{missing}"
    );

    let ack = client
        .manage(json!({ "op": "docker.release", "node": NODE, "id": missing["id"] }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    client
        .until("the released container gone from the list", |c| {
            let listed = c.nodes.get(NODE)?["containers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            listed
                .iter()
                .all(|e| e["name"] != running.0.as_str())
                .then_some(())
        })
        .await;
}
