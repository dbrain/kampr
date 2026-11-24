//! `kampr doctor` asking the running node whether it follows Docker, against the machine's own
//! daemon; skipped where there is none.

use crate::cli::{Cli, check, set_config, stdout};

/// The socket answering is not the node listing: on the operator's Mac doctor said "its containers
/// are listed" while the running node had never followed the daemon.
#[test]
fn doctor_asks_the_running_node_whether_it_follows_docker() {
    if !kampr_testkit::docker::available() {
        return;
    }
    let cli = Cli::new();
    cli.init();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    set_config(
        &cli,
        "bind = \"127.0.0.1:8790\"",
        &format!("bind = \"127.0.0.1:{port}\""),
    );
    set_config(&cli, "[herdr]\n", "[herdr]\nenabled = false\n");
    set_config(&cli, "[docker]\n", "[docker]\nenabled = false\n");
    let node = cli.serve();
    set_config(&cli, "[docker]\nenabled = false\n", "[docker]\n");

    let docker = doctor_docker(&cli);
    assert_eq!(docker["status"], "warn", "{docker:#}");
    assert!(
        docker["detail"]
            .as_str()
            .unwrap()
            .contains("running node is not following"),
        "{docker:#}"
    );
    assert!(docker["fix"].as_str().is_some(), "{docker:#}");

    drop(node);
    let _node = cli.serve();
    let docker = doctor_docker(&cli);
    assert_eq!(docker["status"], "ok", "{docker:#}");
    assert!(
        docker["detail"]
            .as_str()
            .unwrap()
            .contains("the running node lists"),
        "{docker:#}"
    );
}

fn doctor_docker(cli: &Cli) -> serde_json::Value {
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    check(&json, "docker").clone()
}
