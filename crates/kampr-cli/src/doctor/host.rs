use super::Check;
use crate::report::Local;
use crate::service;
use kampr_node::Config;
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
                "{} and {} are private, and so are the database, its two sidecars and node.key",
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

pub async fn service(config: &Config) -> Check {
    let state = service::details();
    let listening = crate::report::reachable(&config.origin()).await;
    let where_ = format!("{} at {}", config.server.bind, config.origin());
    if !state.installed {
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
}
