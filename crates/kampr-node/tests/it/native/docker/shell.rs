//! What `docker.shell` finds to run in a container, and what it refuses before spawning anything.

use super::*;
use kampr_testkit::docker::NoShellImage;

async fn opened(client: &mut Client, container: &str) -> (Value, String) {
    client.container(container, |e| e["state"] == "running").await;
    let ack = client
        .manage(json!({ "op": "docker.shell", "node": NODE, "id": container }))
        .await;
    assert_eq!(ack["ok"], true, "{ack}");
    let id = ack["id"].as_str().expect("the shell's pane").to_string();
    (ack, id)
}

async fn runs(client: &mut Client, id: &str, exe: &str) {
    client.pane(id, |_| true).await;
    client.watch(id, false).await;
    client
        .type_text(id, "echo k$((6*7))-$(readlink /proc/$$/exe)\r")
        .await;
    client.shows(id, &format!("k42-{exe}")).await;
}

#[tokio::test]
async fn a_shell_runs_bash_where_there_is_one_and_sh_where_there_is_not() {
    let Some((_h, mut client)) = docker_node().await else {
        return;
    };
    let alpine = Container::run(&["--", "sleep 600"]);
    let debian = Container::of(&["--entrypoint", "sleep", "postgres:18.6", "600"]);

    let (ack, id) = opened(&mut client, &alpine.0).await;
    assert_eq!(ack["shell"], "sh", "{ack}");
    runs(&mut client, &id, "/bin/busybox").await;

    let (ack, id) = opened(&mut client, &debian.0).await;
    assert_eq!(ack["shell"], "bash", "{ack}");
    runs(&mut client, &id, "/usr/bin/bash").await;
}

#[tokio::test]
async fn a_shell_into_a_stopped_container_is_refused_with_what_to_do() {
    let Some((_h, mut client)) = docker_node().await else {
        return;
    };
    let stopped = Container::run(&["--", "exit 0"]);
    client.container(&stopped.0, |e| e["state"] == "exited").await;
    let ack = client
        .manage(json!({ "op": "docker.shell", "node": NODE, "id": stopped.0 }))
        .await;
    assert_eq!(ack["code"], "bad_request", "{ack}");
    assert_eq!(
        ack["message"],
        format!("{} isn't running — start it first", stopped.0),
        "{ack}"
    );
}

fn unpullable_debug_image(config: &mut Config) {
    with_docker(config);
    config.docker.debug_image = "kt-absent.invalid/none:1".into();
}

#[tokio::test]
async fn a_container_with_no_shell_and_no_debug_image_to_be_had_is_refused_with_why() {
    if !available() {
        return;
    }
    let h = Harness::start_with(Harness::home(), unpullable_debug_image).await;
    let mut client = h.client().await;
    let image = NoShellImage::build();
    let bare = Container::of(&[&image.0]);
    client.container(&bare.0, |e| e["state"] == "running").await;

    let ack = client
        .manage(json!({ "op": "docker.shell", "node": NODE, "id": bare.0 }))
        .await;
    assert_eq!(ack["code"], "bad_request", "{ack}");
    let message = ack["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with(&format!(
            "{} has no shell, and the debug image kt-absent.invalid/none:1 could not be pulled: ",
            bare.0
        )),
        "{ack}"
    );
    assert!(client.panes.values().all(|p| p["container"] != bare.0.as_str()));
}
