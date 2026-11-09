use super::Check;
use kampr_node::Config;
use std::path::Path;

pub async fn check(config: &Config, state_dir: &Path) -> Check {
    let host = std::env::var("DOCKER_HOST").ok();
    let socket = kampr_docker::socket::resolve(&config.docker.socket, host.as_deref());
    let Some(source) = config.docker.resolve(host.as_deref()) else {
        return Check::ok(
            "docker",
            match config.docker.enabled {
                Some(false) => "off ([docker] enabled = false)".to_string(),
                _ => format!("no daemon at {}; no containers are listed", socket.display()),
            },
        );
    };
    let answers = kampr_docker::Client::new(&source.socket).ping().await;
    match (answers, source.required) {
        (true, _) => {
            let client = kampr_docker::Client::new(&source.socket);
            let reaping = without_init(&client, state_dir).await;
            let mut detail = format!("{} answers; its containers are listed", source.socket.display());
            if !reaping.is_empty() {
                detail += &format!(
                    "; adopted without --init, so an ended session stays a zombie until the \
                     container restarts: {}",
                    reaping.join(", ")
                );
            }
            Check::ok("docker", detail)
        }
        (false, true) => Check::warn(
            "docker",
            format!(
                "{} does not answer, so the herd says docker is down",
                source.socket.display()
            ),
        )
        .fix("start docker, or set [docker] socket, or [docker] enabled = false"),
        (false, false) => Check::ok(
            "docker",
            format!(
                "{} does not answer; containers show once it does",
                source.socket.display()
            ),
        ),
    }
}

/// A PID 1 that does not reap leaves every exited keeper and shell behind as a zombie (docs/19).
async fn without_init(client: &kampr_docker::Client, state_dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for record in kampr_node::adopt::Ledger::open(&state_dir.join("docker")).records() {
        if let Ok(inspect) = client.inspect(&record.name).await
            && inspect.host_config.init != Some(true)
        {
            names.push(record.name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;

    #[tokio::test]
    async fn docker_is_a_warning_only_where_somebody_asked_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::bootstrap("doctor");
        config.docker.socket = dir.path().join("docker.sock").display().to_string();
        for (enabled, status) in [
            (None, Status::Ok),
            (Some(true), Status::Warn),
            (Some(false), Status::Ok),
        ] {
            config.docker.enabled = enabled;
            assert_eq!(check(&config, dir.path()).await.status, status, "{enabled:?}");
        }
        std::fs::write(dir.path().join("docker.sock"), "").unwrap();
        config.docker.enabled = None;
        let stale = check(&config, dir.path()).await;
        assert_eq!(stale.status, Status::Ok, "{stale:?}");
        assert!(stale.detail.contains("does not answer"), "{stale:?}");
    }

    #[tokio::test]
    async fn an_adoption_whose_pid_1_does_not_reap_is_named() {
        if !kampr_testkit::docker::available() {
            return;
        }
        use kampr_testkit::docker::Container;
        let bare = Container::run(&["--", "sleep 600"]);
        let reaped = Container::run(&["--init", "--", "sleep 600"]);
        let state = tempfile::tempdir().unwrap();
        let ledger = kampr_node::adopt::Ledger::open(&state.path().join("docker"));
        for name in [&bare.0, &reaped.0] {
            ledger
                .remember(kampr_node::adopt::Record {
                    name: name.clone(),
                    node_id: format!("01JH-c-{name}"),
                })
                .unwrap();
        }
        let mut config = Config::bootstrap("doctor");
        config.docker.enabled = Some(true);
        let line = check(&config, state.path()).await;
        assert!(line.detail.contains("--init"), "{line:?}");
        assert!(line.detail.contains(&bare.0), "{line:?}");
        assert!(!line.detail.contains(&reaped.0), "{line:?}");
    }
}
