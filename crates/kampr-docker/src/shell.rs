use crate::Client;
use crate::api::Inspect;
use anyhow::{Result, bail};

pub const DEBUG_IMAGE: &str = "busybox:1.37-musl";

const SHELLS: [&str; 3] = ["/bin/bash", "/bin/sh", "/busybox/sh"];
const LINK_HOPS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shell {
    Own(String),
    Debug { image: String, name: String },
}

impl Shell {
    pub fn mode(&self) -> &'static str {
        match self {
            Self::Own(path) if path.ends_with("bash") => "bash",
            Self::Own(_) => "sh",
            Self::Debug { .. } => "debug",
        }
    }

    pub fn label(&self, container: &str) -> String {
        match self {
            Self::Own(_) => container.to_string(),
            Self::Debug { .. } => format!("{container} · debug"),
        }
    }

    pub fn argv(&self, docker: &str, inspect: &Inspect) -> Vec<String> {
        let mut argv: Vec<String> = [docker, "exec", "-it", "-e", "TERM=xterm-256color"]
            .map(str::to_string)
            .to_vec();
        match self {
            Self::Own(path) => {
                let workdir = match inspect.config.working_dir.trim() {
                    "" => "/",
                    dir => dir,
                };
                argv.extend(["-w", workdir, &inspect.id, path, "-l"].map(str::to_string));
            }
            Self::Debug { image, name } => {
                let target = format!("container:{}", inspect.id);
                argv[1] = "run".into();
                // Root in the sidecar is not enough to read `/proc/1/root` of a target running as
                // another uid; SYS_PTRACE is (measured). runc refuses `-w /proc/1/root` as a
                // breakout, so the shell moves there itself, and is still a shell where it cannot.
                argv.extend(
                    [
                        "--rm",
                        "--name",
                        name,
                        "--pull",
                        "never",
                        "--pid",
                        &target,
                        "--network",
                        &target,
                        "--cap-add",
                        "SYS_PTRACE",
                        image,
                        "sh",
                        "-c",
                        "cd /proc/1/root; exec sh -l",
                    ]
                    .map(str::to_string),
                );
            }
        }
        argv
    }
}

/// What a shell into the container runs: its own first, else a sidecar from `debug_image`.
pub async fn choose(client: &Client, inspect: &Inspect, debug_image: &str) -> Result<Shell> {
    let name = inspect.short_name();
    match inspect.state.status.as_str() {
        "running" => {}
        "paused" => bail!("{name} is paused — unpause it first"),
        _ => bail!("{name} isn't running — start it first"),
    }
    if let Some(path) = find(client, &inspect.id).await? {
        return Ok(Shell::Own(path));
    }
    if let Err(e) = crate::image::ensure(client, debug_image).await {
        bail!("{name} has no shell, and the debug image {debug_image} could not be pulled: {e:#}");
    }
    Ok(Shell::Debug {
        image: debug_image.to_string(),
        name: sidecar_name(&inspect.id),
    })
}

pub async fn find(client: &Client, id: &str) -> Result<Option<String>> {
    for shell in SHELLS {
        if executable(client, id, shell).await? {
            return Ok(Some(shell.to_string()));
        }
    }
    Ok(None)
}

/// The daemon stats a symlink rather than its target (a link to nothing answers 200), so a link
/// is followed here.
async fn executable(client: &Client, id: &str, path: &str) -> Result<bool> {
    let mut path = path.to_string();
    for _ in 0..LINK_HOPS {
        let Some(stat) = client.stat_path(id, &path).await? else {
            return Ok(false);
        };
        if stat.link_target.is_empty() {
            return Ok(stat.is_executable_file());
        }
        path = stat.link_target;
    }
    Ok(false)
}

fn sidecar_name(container_id: &str) -> String {
    let short: String = container_id.chars().take(12).collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "kampr-debug-{short}-{:06x}",
        (nanos ^ std::process::id()) & 0xff_ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect(working_dir: &str) -> Inspect {
        let mut inspect = Inspect {
            id: "f49c60fac115aaaa".into(),
            name: "/web".into(),
            ..Inspect::default()
        };
        inspect.config.working_dir = working_dir.into();
        inspect
    }

    #[test]
    fn a_shell_the_container_has_runs_as_a_login_shell_where_the_container_works() {
        let bash = Shell::Own("/bin/bash".into());
        assert_eq!(
            bash.argv("docker", &inspect("/srv/app")),
            [
                "docker",
                "exec",
                "-it",
                "-e",
                "TERM=xterm-256color",
                "-w",
                "/srv/app",
                "f49c60fac115aaaa",
                "/bin/bash",
                "-l"
            ]
        );
        let busybox = Shell::Own("/busybox/sh".into());
        let argv = busybox.argv("docker", &inspect(""));
        assert_eq!(argv[5..], ["-w", "/", "f49c60fac115aaaa", "/busybox/sh", "-l"]);
        assert_eq!((bash.mode(), busybox.mode()), ("bash", "sh"));
        assert_eq!(bash.label("web"), "web");
    }

    #[test]
    fn a_debug_shell_shares_the_targets_processes_and_network_and_starts_in_its_filesystem() {
        let debug = Shell::Debug {
            image: DEBUG_IMAGE.into(),
            name: "kampr-debug-f49c60fac115-0a0b0c".into(),
        };
        let argv = debug.argv("docker", &inspect("/srv/app"));
        assert_eq!(
            argv,
            [
                "docker",
                "run",
                "-it",
                "-e",
                "TERM=xterm-256color",
                "--rm",
                "--name",
                "kampr-debug-f49c60fac115-0a0b0c",
                "--pull",
                "never",
                "--pid",
                "container:f49c60fac115aaaa",
                "--network",
                "container:f49c60fac115aaaa",
                "--cap-add",
                "SYS_PTRACE",
                DEBUG_IMAGE,
                "sh",
                "-c",
                "cd /proc/1/root; exec sh -l"
            ]
        );
        assert_eq!(
            (debug.mode(), debug.label("web").as_str()),
            ("debug", "web · debug")
        );
    }

    #[test]
    fn a_sidecar_is_named_for_its_target() {
        let name = sidecar_name("f49c60fac115aaaabbbb");
        assert!(name.starts_with("kampr-debug-f49c60fac115-"), "{name}");
        assert_eq!(name.len(), "kampr-debug-f49c60fac115-".len() + 6, "{name}");
    }
}
