use std::path::PathBuf;

pub const DEFAULT: &str = "/var/run/docker.sock";

/// The configured socket, else `$DOCKER_HOST` when it is a unix one, else Docker's own default.
/// A TCP `DOCKER_HOST` is not followed: it would be a daemon on another machine.
pub fn resolve(configured: &str, docker_host: Option<&str>) -> PathBuf {
    let configured = configured.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured.strip_prefix("unix://").unwrap_or(configured));
    }
    docker_host
        .and_then(|h| h.trim().strip_prefix("unix://"))
        .filter(|p| !p.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_the_configured_one_then_a_unix_docker_host_then_the_default() {
        for (configured, host, want) in [
            ("/run/x.sock", Some("unix:///run/y.sock"), "/run/x.sock"),
            ("unix:///run/x.sock", None, "/run/x.sock"),
            (
                "",
                Some("unix:///run/user/1000/docker.sock"),
                "/run/user/1000/docker.sock",
            ),
            ("", Some("tcp://10.0.0.1:2375"), DEFAULT),
            ("", None, DEFAULT),
        ] {
            assert_eq!(
                resolve(configured, host),
                PathBuf::from(want),
                "{configured} {host:?}"
            );
        }
    }
}
