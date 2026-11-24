use super::Check;
use crate::dirs::Dirs;
use crate::service::Supervisor;
use kampr_node::Config;
use std::path::Path;
use std::time::{Duration, Instant};

/// What the node running on this machine says of Docker, as its own herd reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Running {
    /// `docker` on the node's herd entry: absent until the daemon has answered it.
    Says(Option<bool>),
    Unasked(String),
}

/// A node that has just started has not heard from its daemon yet, so an answer short of `true`
/// is waited on for this long before it is believed.
const ASK_FOR: Duration = Duration::from_secs(4);

/// Over the device the CLI mints itself for the node on this machine (`kampr_client::resolve`),
/// the same one a bare `kampr` uses.
pub async fn running(config: &Config, dirs: &Dirs) -> Running {
    let resolved = tokio::time::timeout(
        ASK_FOR,
        kampr_client::resolve(&dirs.config(), dirs.state_override(), true),
    )
    .await;
    let session = match resolved {
        Ok(Ok(resolved)) if matches!(resolved.session.via, kampr_client::Via::LocalNode { .. }) => {
            resolved.session
        }
        Ok(Ok(_)) | Ok(Err(_)) => return Running::Unasked("no node answers on this machine".into()),
        Err(_) => return Running::Unasked("the node on this machine did not answer in time".into()),
    };
    let client = kampr_client::Client::start(session);
    let mut events = client.events();
    let deadline = Instant::now() + ASK_FOR;
    let mut said = None;
    loop {
        if let Some(node) = client.state().herd.node(&config.node_id) {
            said = Some(node.docker);
            if node.docker == Some(true) {
                break;
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left.min(Duration::from_millis(200)), events.recv()).await {
            Ok(Ok(kampr_client::Event::Refused { .. })) => {
                return Running::Unasked("the node turned this CLI's device away".into());
            }
            _ if left.is_zero() => break,
            _ => {}
        }
    }
    said.map_or_else(
        || Running::Unasked("the node sent no herd in time".into()),
        Running::Says,
    )
}

/// `service_path` is the `PATH` the node's unit gives it, where one is installed.
pub async fn check(
    config: &Config,
    state_dir: &Path,
    service_path: Option<&str>,
    running: Option<Running>,
    restart: &str,
) -> Check {
    let host = std::env::var("DOCKER_HOST").ok();
    let Some(source) = config.docker.resolve(host.as_deref()) else {
        return Check::ok("docker", "off ([docker] enabled = false)");
    };
    let (daemon, answers) = daemon(&source, state_dir).await;
    let daemon = judge(daemon, answers, running, restart);
    match service_path.filter(|path| !super::host::resolves(path, "docker")) {
        // The daemon is reached over its socket, so containers list while every adopt and shell,
        // which run the `docker` CLI, fail with "starting docker: No such file or directory".
        Some(path) => Check::warn(
            "docker",
            format!(
                "{}; but the service's PATH is {path} and has no `docker` on it, so adopting a \
                 container or opening a shell in one fails",
                daemon.detail
            ),
        )
        .fix(match Supervisor::detect() {
            Supervisor::Launchd => "kampr service install — it writes a PATH carrying docker into the plist",
            _ => "add the directory holding docker to the unit's PATH: systemctl --user edit kampr.service",
        }),
        _ => daemon,
    }
}

/// The socket answering this command is not the node listing: a node started before the config
/// changed, or one that predates following a socket that appears late, follows nothing.
fn judge(daemon: Check, answers: bool, running: Option<Running>, restart: &str) -> Check {
    match running {
        None => daemon,
        Some(Running::Unasked(why)) => Check {
            detail: format!(
                "{}; whether the running node follows it was not established: {why}",
                daemon.detail
            ),
            ..daemon
        },
        Some(Running::Says(Some(true))) if answers => Check {
            detail: daemon.detail.replace(
                "its containers are listed",
                "the running node lists its containers",
            ),
            ..daemon
        },
        // The node's own word wins: this command may have resolved a different socket.
        Some(Running::Says(Some(true))) => Check::ok(
            "docker",
            format!("the running node lists its containers ({})", daemon.detail),
        ),
        Some(Running::Says(_)) if answers => Check::warn(
            "docker",
            format!(
                "{}, but the running node is not following it, so no containers are listed",
                daemon.detail.replace("; its containers are listed", "")
            ),
        )
        .fix(format!("restart the node: {restart}")),
        Some(Running::Says(_)) => daemon,
    }
}

async fn daemon(source: &kampr_node::config::DockerSource, state_dir: &Path) -> (Check, bool) {
    let answers = kampr_docker::Client::new(&source.socket).ping().await;
    let check = match (answers, source.required) {
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
    };
    (check, answers)
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
            assert_eq!(
                check(&config, dir.path(), None, None, "").await.status,
                status,
                "{enabled:?}"
            );
        }
        std::fs::write(dir.path().join("docker.sock"), "").unwrap();
        config.docker.enabled = None;
        let stale = check(&config, dir.path(), None, None, "").await;
        assert_eq!(stale.status, Status::Ok, "{stale:?}");
        assert!(stale.detail.contains("does not answer"), "{stale:?}");
    }

    #[test]
    fn the_running_node_has_the_last_word_on_whether_containers_are_listed() {
        let up = || Check::ok("docker", "/s answers; its containers are listed");
        let down = || Check::ok("docker", "/s does not answer; containers show once it does");
        let says = |v| Some(Running::Says(v));
        for (name, daemon, answers, running, status, detail) in [
            (
                "not asked",
                up(),
                true,
                None,
                Status::Ok,
                "its containers are listed",
            ),
            (
                "follows",
                up(),
                true,
                says(Some(true)),
                Status::Ok,
                "the running node lists",
            ),
            (
                "not following",
                up(),
                true,
                says(None),
                Status::Warn,
                "running node is not following",
            ),
            (
                "reported down",
                up(),
                true,
                says(Some(false)),
                Status::Warn,
                "not following",
            ),
            (
                "nobody answers",
                down(),
                false,
                says(None),
                Status::Ok,
                "does not answer",
            ),
            (
                "node sees more",
                down(),
                false,
                says(Some(true)),
                Status::Ok,
                "the running node lists",
            ),
            (
                "unreachable node",
                up(),
                true,
                Some(Running::Unasked("x".into())),
                Status::Ok,
                "was not established: x",
            ),
        ] {
            let check = judge(daemon, answers, running, "restart it");
            assert_eq!(check.status, status, "{name}: {check:?}");
            assert!(check.detail.contains(detail), "{name}: {check:?}");
            assert_eq!(check.fix.is_some(), status == Status::Warn, "{name}: {check:?}");
        }
    }

    /// The daemon answers over its socket, so containers listed while adopt and shell-in, which run
    /// the `docker` CLI, failed under launchd's default PATH.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_service_path_without_docker_is_named() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::bootstrap("doctor");
        config.docker.socket = dir.path().join("docker.sock").display().to_string();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let path = bin.display().to_string();

        let without = check(&config, dir.path(), Some(&path), None, "").await;
        assert_eq!(without.status, Status::Warn, "{without:?}");
        assert!(without.detail.contains(&path), "{without:?}");

        let docker = bin.join("docker");
        std::fs::write(&docker, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            check(&config, dir.path(), Some(&path), None, "").await.status,
            Status::Ok
        );
        assert_eq!(
            check(&config, dir.path(), None, None, "").await.status,
            Status::Ok
        );

        config.docker.enabled = Some(false);
        std::fs::remove_file(&docker).unwrap();
        assert_eq!(
            check(&config, dir.path(), Some(&path), None, "").await.status,
            Status::Ok,
            "nobody needs docker on a node that does not follow it"
        );
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
        let line = check(&config, state.path(), None, None, "").await;
        assert!(line.detail.contains("--init"), "{line:?}");
        assert!(line.detail.contains(&bare.0), "{line:?}");
        assert!(!line.detail.contains(&reaped.0), "{line:?}");
    }
}
