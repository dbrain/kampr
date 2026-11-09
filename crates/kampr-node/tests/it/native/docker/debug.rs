//! A shell into a container with none of its own: a debug sidecar sharing its processes and
//! network. Left out of `make test-fast`, since the debug image may have to be pulled.

use super::*;
use kampr_testkit::docker::NoShellImage;
use std::process::Command;

fn sidecars(container_id: &str) -> Vec<String> {
    let out = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=kampr-debug-{}", &container_id[..12]),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .expect("docker");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

async fn none_left(container_id: &str) {
    let deadline = std::time::Instant::now() + SOON;
    while !sidecars(container_id).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "left behind: {:?}",
            sidecars(container_id)
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn a_container_with_no_shell_gets_a_debug_sidecar_that_goes_when_its_pane_or_target_does() {
    let Some((_h, mut client)) = docker_node().await else {
        return;
    };
    let image = NoShellImage::build();
    let bare = Container::of(&["--user", "1000", &image.0]);
    let entry = client.container(&bare.0, |e| e["state"] == "running").await;
    let full_id = entry["id"].as_str().unwrap().to_string();

    let ack = client
        .manage(json!({ "op": "docker.shell", "node": NODE, "id": bare.0 }))
        .await;
    assert_eq!(
        (&ack["ok"], &ack["shell"]),
        (&json!(true), &json!("debug")),
        "{ack}"
    );
    let id = ack["id"].as_str().unwrap().to_string();
    let pane = client.pane(&id, |p| p["container"] == bare.0.as_str()).await;
    assert_eq!(pane["label"], format!("{} · debug", bare.0), "{pane}");
    client.watch(&id, false).await;
    client.type_text(&id, "pwd; ls; echo k$((6*7))\r").await;
    let screen = client.shows(&id, "k42").await;
    assert!(screen.iter().any(|r| r.contains("/proc/1/root")), "{screen:?}");
    assert!(screen.iter().any(|r| r.contains("nats-server")), "{screen:?}");
    client
        .type_text(&id, "echo p$((6*7)) $(netstat -ltn | grep -c :4222)\r")
        .await;
    client.shows(&id, "p42 1").await;
    assert_eq!(sidecars(&full_id).len(), 1);
    client.type_text(&id, "exit 3\r").await;
    client.pane(&id, |p| p["exited"]["code"] == 3).await;
    none_left(&full_id).await;

    let ack = client
        .manage(json!({ "op": "docker.shell", "node": NODE, "id": bare.0 }))
        .await;
    let id = ack["id"].as_str().expect("a second debug shell").to_string();
    client.pane(&id, |_| true).await;
    client.watch(&id, false).await;
    client.type_text(&id, "echo q$((6*7))\r").await;
    client.shows(&id, "q42").await;
    Command::new("docker")
        .args(["kill", &bare.0])
        .output()
        .expect("docker");
    client.pane(&id, |p| !p["exited"].is_null()).await;
    none_left(&full_id).await;
}
