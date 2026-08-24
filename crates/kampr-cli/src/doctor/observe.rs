//! The binary half of herdr, which the socket half says nothing about.
//!
//! A node reaches herdr two ways: the socket, for the herd and for input, and a spawned
//! `herdr terminal session observe`, which is the entire grid stream. The unit pins
//! `HERDR_SOCKET_PATH`, so the socket half works under a service manager whatever the
//! environment; the binary half was resolved through a `PATH` the service does not have. The
//! result passed every check this command made and showed a blank grid in every client.

use super::{Check, Status};
use crate::service;
use kampr_herdr::locate::{Found, NotFound, Origin, Search, locate};
use kampr_node::Config;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

/// Long enough for a cold page-in of a 20 MB binary, short enough that `kampr doctor` still
/// answers on a wedged host.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn check(config: &Config, installed: bool) -> Check {
    let binary = config.herdr.binary.as_str();
    let here = locate(binary, &Search::from_env());
    // `Some(None)`: a unit is installed and its manager could not be asked what PATH it will hand
    // the node — which is not the same as there being no unit.
    let manager = installed.then(service::manager_path);
    let under = manager
        .as_ref()
        .map(|path| locate(binary, &service_search(path.clone())));

    // The service's answer wins where there is one: it is the environment the node actually runs
    // in, and this command's own PATH is the thing that made the old check a lie.
    let resolved = under
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .or(here.as_ref().ok())
        .cloned();
    let Some(found) = resolved else {
        return missing(here.expect_err("neither search found a binary"));
    };
    let version = match version(&found.path).await {
        Ok(version) => version,
        Err(why) => {
            return Check::fail(
                "observe",
                format!("{} is there and did not run: {why}", found.path.display()),
            )
            .fix("install a herdr that runs on this host, or set [herdr] binary in config.toml");
        }
    };
    verdict(&found, &version, under, manager.flatten())
}

fn verdict(
    found: &Found,
    version: &str,
    under: Option<Result<Found, NotFound>>,
    manager_path: Option<String>,
) -> Check {
    let at = format!(
        "{version} at {} ({}) — every pane's grid is streamed by this binary, not by the socket",
        found.path.display(),
        how(found.origin)
    );
    match (under, manager_path) {
        (Some(Err(_)), Some(path)) => Check::fail(
            "observe",
            format!(
                "{at}. The service will not find it: the kampr.service manager's PATH is {path}, \
                 and a bare `{}` is resolved through that and not through your shell's",
                found_name(found)
            ),
        )
        .fix(PIN),
        (Some(Err(_)), None) => Check::warn(
            "observe",
            format!(
                "{at}. Could not ask the service manager which PATH the node will have, and a \
                 bare name is resolved through it rather than through your shell's"
            ),
        )
        .fix(PIN),
        _ => Check::new("observe", Status::Ok, at),
    }
}

const PIN: &str = "kampr service install — it records the resolved path in config.toml";

fn found_name(found: &Found) -> String {
    found.path.file_name().map_or_else(
        || found.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn missing(error: NotFound) -> Check {
    let fix = match error.explicit {
        true => "install herdr there, or point [herdr] binary in config.toml at the one you have",
        false => "install herdr, or set [herdr] binary in config.toml to its absolute path",
    };
    // Not a warning: `pane.read` still answers, so the herd, the pane list and input all keep
    // working, and every grid stays blank for ever.
    Check::fail("observe", format!("{error}; no pane can stream its grid")).fix(fix)
}

fn how(origin: Origin) -> &'static str {
    match origin {
        Origin::Configured => "pinned in config.toml",
        Origin::Injected => "named by HERDR_BIN_PATH",
        Origin::Path => "on PATH",
        Origin::BesideKampr => "beside the kampr binary",
        Origin::Prefix => "in the usual install prefix",
    }
}

/// What the node will resolve from under its unit: the manager's `PATH`, and the directory
/// holding the kampr that unit runs — which need not be the one running this command.
fn service_search(path: Option<String>) -> Search {
    Search {
        injected: None,
        path: path.map(Into::into),
        beside: service::unit_binary()
            .or_else(|| std::env::current_exe().ok())
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        ..Search::from_env()
    }
}

async fn version(path: &Path) -> Result<String, String> {
    let output = tokio::time::timeout(VERSION_TIMEOUT, Command::new(path).arg("--version").output())
        .await
        .map_err(|_| format!("no answer in {} seconds", VERSION_TIMEOUT.as_secs()))?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("`--version` exited {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn found(origin: Origin) -> Found {
        Found {
            path: PathBuf::from("/home/x/.local/bin/herdr"),
            origin,
        }
    }

    fn not_found() -> NotFound {
        NotFound {
            binary: "herdr".into(),
            tried: vec![PathBuf::from("/usr/bin/herdr")],
            explicit: false,
        }
    }

    #[test]
    fn a_binary_this_shell_can_run_and_the_service_cannot_is_a_failure_and_says_whose_path() {
        let check = verdict(
            &found(Origin::Path),
            "herdr 0.8.2",
            Some(Err(not_found())),
            Some("/usr/local/bin:/usr/bin:/bin".into()),
        );
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("/usr/local/bin:/usr/bin:/bin"), "{check:?}");
        assert!(check.detail.contains("herdr 0.8.2"), "{check:?}");
        assert!(check.fix.unwrap().contains("kampr service install"));
    }

    #[test]
    fn a_manager_that_cannot_be_asked_is_a_warning_rather_than_a_verdict() {
        let check = verdict(&found(Origin::Path), "herdr 0.8.2", Some(Err(not_found())), None);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("Could not ask"), "{check:?}");
    }

    #[test]
    fn a_binary_both_can_run_is_ok_and_names_where_it_came_from() {
        for origin in [
            Origin::Configured,
            Origin::Injected,
            Origin::Path,
            Origin::BesideKampr,
            Origin::Prefix,
        ] {
            let check = verdict(
                &found(origin),
                "herdr 0.8.2",
                Some(Ok(found(origin))),
                Some("/usr/bin".into()),
            );
            assert_eq!(check.status, Status::Ok, "{check:?}");
            assert!(check.detail.contains(how(origin)), "{check:?}");
            assert!(check.fix.is_none(), "nothing to fix: {check:?}");
        }
    }

    #[test]
    fn a_node_with_no_service_is_judged_on_its_own_shell_alone() {
        let check = verdict(&found(Origin::Path), "herdr 0.8.2", None, None);
        assert_eq!(check.status, Status::Ok, "{check:?}");
    }

    /// A blank grid for ever is not a warning, and the remedy differs: a path the operator wrote
    /// is theirs to correct, a name nobody can find is a missing install.
    #[test]
    fn nothing_to_run_is_a_failure_whose_remedy_matches_what_was_asked_for() {
        let searched = missing(not_found());
        assert_eq!(searched.status, Status::Fail);
        assert!(searched.detail.contains("/usr/bin/herdr"), "{searched:?}");
        assert!(
            searched.detail.contains("no pane can stream its grid"),
            "{searched:?}"
        );
        assert!(searched.fix.unwrap().contains("install herdr"));

        let explicit = missing(NotFound {
            binary: "/opt/herdr".into(),
            tried: vec![PathBuf::from("/opt/herdr")],
            explicit: true,
        });
        assert!(explicit.detail.contains("/opt/herdr"), "{explicit:?}");
        assert!(explicit.fix.unwrap().contains("config.toml"));
    }
}
