use super::Check;
use crate::report::Local;
use crate::service::{self, Supervisor};
use kampr_fleet::PathOrigin;
use kampr_node::Config;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Everything on disk that is as sensitive as the terminals it guards, and the mode it has to
/// have. The write-ahead log and the shared-memory file are listed by name because they are the
/// ones sqlite creates for itself at the process umask — the audit found `-wal` world-readable
/// with credential digests in it while the database beside it was 0600.
pub fn files(config_dir: &Path, state_dir: &Path) -> Vec<Check> {
    let db = Config::state_db(state_dir);
    let mut wanted: Vec<(PathBuf, u32)> = vec![
        (state_dir.to_path_buf(), 0o700),
        (config_dir.to_path_buf(), 0o700),
        (db.clone(), 0o600),
        (Config::audit_path(state_dir), 0o600),
        (Config::node_key_path(config_dir), 0o600),
        (Config::vapid_path(state_dir), 0o600),
    ];
    wanted.extend(["-wal", "-shm"].into_iter().map(|suffix| {
        let mut path = db.as_os_str().to_os_string();
        path.push(suffix);
        (PathBuf::from(path), 0o600)
    }));

    let wrong: Vec<(PathBuf, u32)> = wanted
        .iter()
        .filter_map(|(path, want)| {
            let found = mode(path)? & 0o777;
            (found != *want).then(|| (path.clone(), found))
        })
        .collect();
    if wrong.is_empty() {
        return vec![Check::ok(
            "permissions",
            format!(
                "{} and {} are private, and so are the database, its two sidecars, node.key and \
                 vapid.pem",
                state_dir.display(),
                config_dir.display()
            ),
        )];
    }
    let detail = wrong
        .iter()
        .map(|(path, found)| {
            let want = wanted.iter().find(|(p, _)| p == path).map_or(0o600, |(_, w)| *w);
            format!("{} is {found:04o} and should be {want:04o}", path.display())
        })
        .collect::<Vec<_>>()
        .join("; ");
    let named = |mode: u32| -> Option<String> {
        let paths: Vec<String> = wrong
            .iter()
            .filter(|(path, _)| wanted.iter().any(|(p, w)| p == path && *w == mode))
            .map(|(path, _)| path.display().to_string())
            .collect();
        (!paths.is_empty()).then(|| format!("chmod {mode:o} {}", paths.join(" ")))
    };
    let fix = [named(0o700), named(0o600)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" && ");
    vec![Check::fail("permissions", detail).fix(fix)]
}

#[cfg(unix)]
pub fn mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(std::fs::metadata(path).ok()?.permissions().mode())
}

#[cfg(not(unix))]
pub fn mode(_path: &Path) -> Option<u32> {
    None
}

/// Every node built before the bundle was staged served a placeholder page and nothing said so.
pub fn bundle() -> Check {
    if kampr_node::assets::has_bundle() {
        return Check::ok("bundle", "a client bundle is compiled into this binary");
    }
    Check::fail(
        "bundle",
        "no client bundle is compiled into this binary — every browser that opens the node gets \
         the placeholder page, not Kampr",
    )
    .fix("build the client and stage it into crates/kampr-node/dist/ before cargo build")
}

pub async fn service(config: &Config, state: &service::State) -> Check {
    let listening = crate::report::reachable(&config.origin()).await;
    let where_ = format!("{} at {}", config.server.bind, config.origin());
    if !state.installed {
        if Supervisor::detect() == Supervisor::Unsupported {
            return Check::warn(
                "service",
                format!(
                    "this host has no systemd, so there is no user unit to install and nothing \
                     restarts the node ({where_})"
                ),
            )
            .fix("run `kampr serve` under whatever already supervises this box");
        }
        return Check::warn(
            "service",
            format!("no supervised unit is installed, so nothing restarts the node ({where_})"),
        )
        .fix("kampr service install");
    }
    let detail = format!(
        "{} ({}), {where_}, listening {}",
        state.active,
        state.enabled,
        yes_no(listening)
    );
    if state.failed {
        return Check::fail(
            "service",
            format!("{detail}; last result {} at {}", state.result, state.since),
        )
        .fix("systemctl --user status kampr.service");
    }
    if !state.running {
        return Check::fail("service", format!("{detail}; stopped since {}", state.since))
            .fix(service::start_hint());
    }
    if !listening {
        return Check::fail(
            "service",
            format!("{detail}; the unit is running but nothing answers on the port"),
        )
        .fix("KAMPR_LOG=debug systemctl --user restart kampr.service");
    }
    Check::ok("service", detail)
}

/// Enabled is not the same as surviving. A `systemd --user` manager is torn down with the user's
/// last session and is not started at boot without lingering, so an installed unit on a
/// non-lingering user is a node that dies at logout and never comes back.
pub fn linger(state: &service::State) -> Check {
    if !state.installed {
        return Check::ok(
            "linger",
            "no unit is installed, so there is nothing yet for a reboot to lose",
        );
    }
    match Supervisor::detect() {
        Supervisor::Launchd => Check::warn(
            "linger",
            "the agent is loaded into gui/$(id -u), a domain that only exists once someone logs \
             in at the screen — a Mac that reboots to the login window does not start the node \
             until you do",
        )
        .fix("for a headless Mac, run it as a LaunchDaemon in /Library/LaunchDaemons instead"),
        Supervisor::Unsupported => Check::warn(
            "linger",
            "a unit file is here but this host has no systemd to read it",
        )
        .fix("run `kampr serve` under whatever already supervises this box"),
        Supervisor::Systemd => {
            let user = service::username();
            match service::linger(&user) {
                Some(true) => Check::ok(
                    "linger",
                    format!("lingering is on for {user}, so systemd starts the user manager at boot"),
                ),
                Some(false) => Check::fail(
                    "linger",
                    format!(
                        "the unit is installed but lingering is off for {user}: systemd tears the \
                         user manager down when your last session ends and does not start it at \
                         boot, so the node dies at logout and does not come back after a reboot"
                    ),
                )
                .fix(format!("loginctl enable-linger {user}")),
                None => Check::warn(
                    "linger",
                    format!("could not tell whether lingering is on for {user}"),
                )
                .fix(format!("loginctl enable-linger {user}")),
            }
        }
    }
}

pub async fn access(local: &Local) -> Vec<Check> {
    vec![devices(local).await, recovery(local).await]
}

async fn devices(local: &Local) -> Check {
    let devices = match local.auth.devices().await {
        Ok(devices) => devices,
        Err(e) => return Check::fail("devices", format!("the device store did not answer: {e}")),
    };
    let now = kampr_auth::now();
    let active = devices.iter().filter(|d| d.active(now)).count();
    let revoked = devices.iter().filter(|d| d.revoked_at.is_some()).count();
    let expired = devices.len() - active - revoked;
    let detail = format!("{active} enrolled, {revoked} revoked, {expired} expired");
    if active == 0 {
        return Check::warn("devices", format!("{detail} — nothing can reach this node")).fix("kampr setup");
    }
    Check::ok("devices", detail)
}

async fn recovery(local: &Local) -> Check {
    match local.auth.has_recovery().await {
        Err(e) => Check::fail("recovery", format!("the device store did not answer: {e}")),
        Ok(false) => Check::warn(
            "recovery",
            "no recovery code — losing every paired device would mean losing this node",
        )
        .fix("kampr recover --new"),
        Ok(true) => {
            let issued = local
                .auth
                .store()
                .recovery_issued_at()
                .await
                .ok()
                .flatten()
                .map_or_else(|| "unknown".into(), stamp);
            let attempts = local.auth.store().recovery_attempts().await.unwrap_or_default();
            let detail = format!("a recovery code is live, issued {issued}");
            if attempts == 0 {
                return Check::ok("recovery", detail);
            }
            Check::warn(
                "recovery",
                format!("{detail}, with {attempts} failed attempt(s) against it"),
            )
            .fix("kampr recover --new   (if you did not make those attempts)")
        }
    }
}

fn stamp(unix: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| unix.to_string())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Whether a bare `name` runs on `path`, asked the way `exec` asks it: the first entry holding a
/// file this user may execute.
fn resolves(path: &str, name: &str) -> bool {
    path.split(':')
        .filter(|entry| !entry.is_empty())
        .map(|entry| std::path::Path::new(entry).join(name))
        .any(|candidate| {
            std::fs::metadata(&candidate)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

/// What a fleet run on this host will find, and where that came from.
///
/// Worth a line of its own because the failure it explains is silent and fans out: the node is a
/// service, a service manager's `PATH` has no `~/.local/bin` in it, and `kampr update` across the
/// herd then fails on every host with a message about a file (#392). A reader who can see the
/// `PATH` can see why in one glance.
pub fn fleet_path(config: &Config) -> Check {
    let configured = Some(config.fleet.path.clone()).filter(|p| !p.is_empty());
    let Some(path) = kampr_fleet::fleet_path(configured) else {
        return Check::warn(
            "fleet path",
            "this node has no PATH at all to give a fleet run, so a bare command name resolves to \
             nothing on this host",
        )
        .fix("set fleet.path in config.toml");
    };
    let source = match path.origin {
        PathOrigin::Configured => "fleet.path in config.toml",
        PathOrigin::Login => "this user's login shell",
        PathOrigin::Inherited => "this process's own environment",
    };
    let note = format!("{} ({source})", path.value);
    // `ok` because a PATH was read, while the one command the operator most often fans out cannot
    // be found on it, is the shape this project has paid for before (#233): the check answered its
    // own question and not the operator's. Two hosts on this herd read their login shell correctly
    // and still had no `~/.local/bin` on it (#419).
    let unreachable: Vec<&str> = ["kampr", "herdr"]
        .into_iter()
        .filter(|name| !resolves(&path.value, name))
        .collect();
    if !unreachable.is_empty() {
        return Check::warn(
            "fleet path",
            format!(
                "{note} — {} is not on it, so `kampr run {}` fails on this host with a message \
                 about a file",
                unreachable.join(" and "),
                unreachable[0],
            ),
        )
        .fix("set fleet.path in config.toml to a PATH carrying it");
    }
    match path.origin {
        // The rung that means the login shell could not be read. It is what every fleet run got
        // before there was anything else to get, so it is not a failure — but it is the shape the
        // report was about, and a reader looking for "why did my command not run" has to see it.
        PathOrigin::Inherited => Check::warn(
            "fleet path",
            format!(
                "{note} — the login shell could not be read, so a fleet run gets a service \
                 manager's PATH rather than yours"
            ),
        )
        .fix("set fleet.path in config.toml to the PATH your commands need"),
        _ => Check::ok("fleet path", note),
    }
}

#[cfg(test)]
mod tests {
    use super::super::Status;
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn a_loosened_sidecar_is_caught_even_when_the_database_is_right() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let config = dir.path().join("config");
        kampr_auth::private_dir(&state).unwrap();
        kampr_auth::private_dir(&config).unwrap();
        let store = kampr_auth::Store::open(&Config::state_db(&state)).await.unwrap();
        store.issue_recovery(0).await.unwrap();
        assert_eq!(files(&config, &state)[0].status, Status::Ok);

        let wal = state.join("kampr.db-wal");
        std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o644)).unwrap();
        let check = &files(&config, &state)[0];
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("kampr.db-wal"), "{}", check.detail);
        assert!(check.fix.as_ref().unwrap().contains("chmod"));
    }

    /// `vapid.pem` is the private key every push subscription is bound to, and it lives beside the
    /// files this check already guards.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_loosened_push_key_is_caught() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let config = dir.path().join("config");
        kampr_auth::private_dir(&state).unwrap();
        kampr_auth::private_dir(&config).unwrap();
        let store = kampr_auth::Store::open(&Config::state_db(&state)).await.unwrap();
        store.issue_recovery(0).await.unwrap();
        let vapid = Config::vapid_path(&state);
        std::fs::write(&vapid, "x").unwrap();
        std::fs::set_permissions(&vapid, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(files(&config, &state)[0].status, Status::Ok);

        std::fs::set_permissions(&vapid, std::fs::Permissions::from_mode(0o644)).unwrap();
        let check = &files(&config, &state)[0];
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("vapid.pem"), "{}", check.detail);
    }

    /// A PATH read from the right shell, that a fleet run cannot run `kampr` on, is the report the
    /// operator filed: `doctor` said `ok` while `kampr update` across the herd failed.
    #[cfg(unix)]
    #[test]
    fn a_path_that_cannot_run_the_command_it_is_for_is_not_an_ok() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        let path = bin.display().to_string();

        assert!(!resolves(&path, "kampr"), "an empty directory resolves nothing");

        for name in ["kampr", "herdr"] {
            let file = bin.join(name);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            // Present but not executable is not a command, and `exec` agrees.
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(!resolves(&path, name), "{name} is not executable");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(resolves(&path, "kampr") && resolves(&path, "herdr"));
        assert!(
            !resolves(&format!("{}:{}", empty.display(), empty.display()), "kampr"),
            "no entry of the PATH holds it",
        );
    }
}
