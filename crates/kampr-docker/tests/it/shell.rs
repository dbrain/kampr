//! Which shell a container has, asked of the daemon rather than of a shell that may not be there.

use super::available;
use kampr_docker::Client;
use kampr_testkit::docker::{Container, NoShellImage};

fn client() -> Client {
    Client::new(kampr_docker::socket::resolve(
        "",
        std::env::var("DOCKER_HOST").ok().as_deref(),
    ))
}

async fn found(container: &Container) -> Option<String> {
    kampr_docker::shell::find(&client(), &container.0)
        .await
        .expect("an answer")
}

#[tokio::test]
async fn the_first_shell_a_container_has_is_the_one_found() {
    if !available() {
        return;
    }
    let alpine = Container::run(&["--", "ln -s /nowhere /bin/bash; sleep 600"]);
    let debian = Container::of(&["--entrypoint", "sleep", "postgres:18.6", "600"]);
    let image = NoShellImage::build();
    let bare = Container::of(&[&image.0]);
    for _ in 0..50 {
        if alpine.status() == "running"
            && std::process::Command::new("docker")
                .args(["exec", &alpine.0, "test", "-L", "/bin/bash"])
                .status()
                .is_ok_and(|s| s.success())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert_eq!(
        found(&alpine).await.as_deref(),
        Some("/bin/sh"),
        "a /bin/bash that links to nothing is not a shell"
    );
    assert_eq!(found(&debian).await.as_deref(), Some("/bin/bash"));
    assert_eq!(found(&bare).await, None);
}
