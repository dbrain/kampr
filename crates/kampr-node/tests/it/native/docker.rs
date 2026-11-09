//! A node's Docker containers (docs/19), against the machine's own daemon with throwaway `kt-*`
//! containers; skipped where there is none.

use super::*;
use kampr_testkit::docker::{Container, available};

mod debug;
mod shell;

fn with_docker(config: &mut Config) {
    config.docker.enabled = None;
    config.auth.audit = true;
}

impl Client {
    async fn container(&mut self, name: &str, want: impl Fn(&Value) -> bool) -> Value {
        self.until(&format!("{name} in the herd as wanted"), |c| {
            c.nodes.get(NODE)?["containers"]
                .as_array()?
                .iter()
                .find(|e| e["name"] == name && want(e))
                .cloned()
        })
        .await
    }
}

async fn docker_node() -> Option<(Harness, Client)> {
    if !available() {
        return None;
    }
    let h = Harness::start_with(Harness::home(), with_docker).await;
    let client = h.client().await;
    Some((h, client))
}

#[tokio::test]
async fn the_herd_lists_the_machines_containers_and_why_one_keeps_stopping() {
    let Some((_h, mut client)) = docker_node().await else {
        return;
    };
    let looping = Container::run(&["--restart", "on-failure:3", "--", "exit 3"]);
    let composed = Container::run(&[
        "--label",
        "com.docker.compose.project=ktproject",
        "--label",
        "com.docker.compose.service=web",
        "--",
        "sleep 600",
    ]);

    let crashed = client
        .container(&looping.0, |e| e["crashes"] == 4 && e["state"] == "exited")
        .await;
    assert_eq!(crashed["restarts"], 3, "{crashed}");
    assert_eq!(crashed["exit_code"], 3, "{crashed}");
    assert_eq!(crashed["last_crash"]["exit_code"], 3, "{crashed}");
    assert_eq!(crashed["image"], kampr_testkit::docker::IMAGE, "{crashed}");

    let running = client.container(&composed.0, |e| e["state"] == "running").await;
    assert_eq!(running["project"], "ktproject", "{running}");
    assert_eq!(running["service"], "web", "{running}");
    assert!(running.get("exit_code").is_none(), "{running}");
    assert_eq!(client.nodes[NODE]["docker"], true);
    let hello = client.others.iter().find(|f| f["t"] == "hello").expect("a hello");
    assert_eq!(hello["caps"]["docker"], true, "{hello}");
}

#[tokio::test]
async fn a_shell_into_a_container_is_a_native_pane_that_names_it() {
    let Some((h, mut client)) = docker_node().await else {
        return;
    };
    let hostname = kampr_testkit::docker::name().replace("kt-", "kth");
    let running = Container::run(&["--hostname", &hostname, "--", "sleep 600"]);
    client.container(&running.0, |e| e["state"] == "running").await;

    let ack = client
        .manage(json!({ "op": "docker.shell", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    let id = ack["id"].as_str().expect("the shell's pane").to_string();
    assert!(id.starts_with(&format!("{NODE}/n:")), "{id}");
    let pane = client.pane(&id, |p| p["container"] == running.0.as_str()).await;
    assert_eq!(pane["label"], running.0.as_str(), "{pane}");
    assert_eq!(pane["provider"], "native", "{pane}");

    client.watch(&id, false).await;
    client.type_text(&id, "echo in-$(hostname)\r").await;
    client.shows(&id, &format!("in-{hostname}")).await;
    client.type_text(&id, "exit 4\r").await;
    let ended = client.pane(&id, |p| p["exited"]["code"] == 4).await;
    assert_eq!(
        ended["container"],
        running.0.as_str(),
        "an ended shell still says where it was: {ended}"
    );
    drop(h);
}

#[tokio::test]
async fn a_detail_rides_its_ack_and_only_the_ops_that_change_something_are_audited() {
    let Some((h, mut client)) = docker_node().await else {
        return;
    };
    let running = Container::run(&[
        "-e",
        "KT_SEEN=yes",
        "--",
        "trap 'exit 0' TERM; echo hello-kt; sleep 600 & wait",
    ]);
    let entry = client.container(&running.0, |e| e["state"] == "running").await;

    let ack = client
        .manage(json!({ "op": "docker.detail", "node": NODE, "id": running.0, "rid": 9 }))
        .await;
    assert_eq!((&ack["ok"], &ack["rid"]), (&json!(true), &json!(9)), "{ack}");
    let detail = &ack["detail"];
    assert_eq!(detail["id"], entry["id"], "{detail}");
    assert!(
        detail["env"].as_array().unwrap().contains(&json!("KT_SEEN=yes")),
        "{detail}"
    );
    assert!(
        detail["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["line"] == "hello-kt"),
        "{detail}"
    );
    assert!(detail["stats"]["mem_used"].as_u64().unwrap() > 0, "{detail}");

    let ack = client
        .manage(json!({ "op": "docker.stop", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    client.container(&running.0, |e| e["state"] == "exited").await;
    let ack = client
        .manage(json!({ "op": "docker.start", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    client.container(&running.0, |e| e["state"] == "running").await;
    let ack = client
        .manage(json!({ "op": "docker.restart", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");

    let audit = std::fs::read_to_string(h.home.as_ref().unwrap().path().join("state/audit.jsonl"))
        .unwrap_or_default();
    for op in ["docker.stop", "docker.start", "docker.restart"] {
        let line = audit
            .lines()
            .find(|l| l.contains(op))
            .unwrap_or_else(|| panic!("{op}: {audit}"));
        assert!(line.contains(&running.0), "{op} without its container: {line}");
    }
    assert!(
        !audit.contains("docker.detail"),
        "a polled read was audited: {audit}"
    );
}

#[tokio::test]
async fn a_container_can_be_killed_and_then_removed() {
    let Some((_h, mut client)) = docker_node().await else {
        return;
    };
    let running = Container::run(&["--", "trap '' TERM; sleep 600 & wait"]);
    client.container(&running.0, |e| e["state"] == "running").await;

    let ack = client
        .manage(json!({ "op": "docker.kill", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    let killed = client.container(&running.0, |e| e["state"] == "exited").await;
    assert_eq!(killed["exit_code"], 137, "{killed}");

    let ack = client
        .manage(json!({ "op": "docker.remove", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    client
        .until(&format!("{} gone from the herd", running.0), |c| {
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

#[tokio::test]
async fn an_op_on_a_container_that_is_not_there_is_refused() {
    let Some((_h, mut client)) = docker_node().await else {
        return;
    };
    let ack = client
        .manage(json!({ "op": "docker.restart", "node": NODE, "id": "kt-never-was" }))
        .await;
    assert_eq!(
        (&ack["ok"], &ack["code"]),
        (&json!(false), &json!("unknown_pane")),
        "{ack}"
    );
    let ack = client
        .manage(json!({ "op": "docker.detail", "node": NODE, "id": "kt-never-was" }))
        .await;
    assert_eq!(
        (&ack["ok"], &ack["code"]),
        (&json!(false), &json!("unknown_pane")),
        "{ack}"
    );
    let ack = client
        .manage(json!({ "op": "docker.restart", "node": NODE }))
        .await;
    assert_eq!(ack["code"], "bad_request", "{ack}");
}

#[tokio::test]
async fn a_binary_that_would_not_run_in_the_container_is_refused_before_anything_is_copied() {
    if !available() {
        return;
    }
    let h = Harness::start_with(Harness::home(), |config| {
        with_docker(config);
        config.docker.binary = kampr_testkit::kampr().display().to_string();
        config.docker.fetch = Some(false);
    })
    .await;
    let mut client = h.client().await;
    let running = Container::run(&["--", "sleep 600"]);
    client.container(&running.0, |e| e["state"] == "running").await;

    let ack = client
        .manage(json!({ "op": "docker.adopt", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(
        (&ack["ok"], &ack["code"]),
        (&json!(false), &json!("bad_request")),
        "{ack}"
    );
    let message = ack["message"].as_str().unwrap_or_default();
    assert!(message.contains("dynamically linked"), "{ack}");
    assert!(message.contains("[docker] fetch = false"), "{ack}");
    let out = std::process::Command::new("docker")
        .args(["exec", &running.0, "ls", "/.kampr"])
        .output()
        .expect("docker");
    assert!(!out.status.success(), "something was copied in");
    assert!(
        !h.home
            .as_ref()
            .unwrap()
            .path()
            .join("state/docker/adopted.json")
            .exists()
    );

    let ack = client
        .manage(json!({ "op": "docker.release", "node": NODE, "id": running.0 }))
        .await;
    assert_eq!(
        ack["code"], "bad_request",
        "releasing what was never adopted: {ack}"
    );
}

#[tokio::test]
async fn a_container_with_a_read_only_root_is_refused_in_words_and_shell_in_is_offered() {
    if !available() {
        return;
    }
    let h = Harness::start_with(Harness::home(), |config| {
        with_docker(config);
        config.docker.fetch = Some(false);
    })
    .await;
    let mut client = h.client().await;
    let sealed = Container::run(&["--read-only", "--", "sleep 600"]);
    client.container(&sealed.0, |e| e["state"] == "running").await;

    let ack = client
        .manage(json!({ "op": "docker.adopt", "node": NODE, "id": sealed.0 }))
        .await;
    assert_eq!(ack["code"], "bad_request", "{ack}");
    assert_eq!(
        ack["message"].as_str().unwrap_or_default(),
        format!(
            "{} has a read-only root filesystem (read_only: true), so kampr can't be copied into \
             it — adopting needs a writable /. Shell in still works.",
            sealed.0
        ),
        "{ack}"
    );
    assert!(
        !h.home
            .as_ref()
            .unwrap()
            .path()
            .join("state/docker/adopted.json")
            .exists()
    );
}
