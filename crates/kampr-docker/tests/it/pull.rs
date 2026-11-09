//! A debug image fetched from its registry when the machine does not have it. Left out of
//! `make test-fast`; skipped where the registry cannot be reached.

use super::available;
use kampr_docker::Client;
use std::net::ToSocketAddrs;
use std::process::Command;
use std::time::Duration;

const PULLED: &str = "busybox:1.37.0-musl";

fn registry_reachable() -> bool {
    let reachable = ("registry-1.docker.io", 443)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .is_some_and(|addr| std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok());
    if !reachable {
        eprintln!("skipped: registry-1.docker.io is not reachable");
    }
    reachable
}

fn present(reference: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", reference])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[tokio::test]
async fn an_image_the_machine_lacks_is_pulled_and_one_that_does_not_exist_says_why() {
    if !available() || !registry_reachable() {
        return;
    }
    let client = Client::new(kampr_docker::socket::resolve(
        "",
        std::env::var("DOCKER_HOST").ok().as_deref(),
    ));
    let _ = Command::new("docker").args(["rmi", PULLED]).output();
    assert!(!present(PULLED));

    kampr_docker::image::ensure(&client, PULLED)
        .await
        .expect("pulled");
    assert!(present(PULLED));
    kampr_docker::image::ensure(&client, PULLED)
        .await
        .expect("already there");

    let e = kampr_docker::image::ensure(&client, "busybox:kt-no-such-tag")
        .await
        .expect_err("no such tag");
    assert!(format!("{e:#}").contains("kt-no-such-tag"), "{e:#}");
}
