//! `kampr doctor` on a node beside a muffit daemon that will not take its token: the failure that
//! left a host's muffit panes missing with nothing in any log.

use crate::cli::{Cli, check, set_config, stdout};
use std::io::{Read, Write};
use std::net::TcpListener;

/// A daemon that answers every request with `status` and `body`, for as long as the test runs.
fn daemon(status: &str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn dead() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn doctor_muffit(url: &str) -> serde_json::Value {
    let cli = Cli::new();
    cli.init();
    set_config(
        &cli,
        "[muffit]\n",
        &format!("[muffit]\nurl = \"{url}\"\ntoken = \"t\"\n"),
    );
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&cli.run(&["doctor", "--json"]))).expect("json");
    check(&json, "muffit").clone()
}

#[test]
fn doctor_tells_a_daemon_that_refuses_the_token_from_one_that_is_not_there() {
    let refusing = daemon("401 Unauthorized", r#"{"error":"wrong bearer token"}"#);
    let refused = doctor_muffit(&refusing);
    assert_eq!(refused["status"], "fail", "{refused:#}");
    let detail = refused["detail"].as_str().unwrap();
    assert!(
        detail.contains(&refusing) && detail.contains("401") && detail.contains("wrong bearer token"),
        "{refused:#}"
    );
    assert!(
        refused["fix"].as_str().unwrap().contains("[muffit] url"),
        "{refused:#}"
    );

    let answering = daemon("200 OK", r#"{"sessions":[]}"#);
    assert_eq!(doctor_muffit(&answering)["status"], "ok");

    let absent = doctor_muffit(&dead());
    assert_eq!(
        absent["status"], "ok",
        "a daemon that is down is a supported state: {absent:#}"
    );
    assert!(
        absent["detail"].as_str().unwrap().contains("no daemon answers"),
        "{absent:#}"
    );
}
