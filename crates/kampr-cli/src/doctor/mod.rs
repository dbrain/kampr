//! One command that says what is wrong, in one screen.
//!
//! Every check answers three questions in this order: what is true, why that matters, and the
//! command that changes it. A check with no fix to offer is a check that should not exist.

mod assetlinks;
mod cert;
mod exposure;
mod herd;
mod host;
mod observe;
mod origin;
mod render;

use crate::dirs::Dirs;
use crate::report::{Local, StoreProblem};
use anyhow::Result;
use kampr_node::Config;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Check {
    pub fn new(id: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn ok(id: &'static str, detail: impl Into<String>) -> Self {
        Self::new(id, Status::Ok, detail)
    }

    pub fn warn(id: &'static str, detail: impl Into<String>) -> Self {
        Self::new(id, Status::Warn, detail)
    }

    pub fn fail(id: &'static str, detail: impl Into<String>) -> Self {
        Self::new(id, Status::Fail, detail)
    }

    pub fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub build: &'static str,
    pub checks: Vec<Check>,
}

impl Report {
    fn new(config: Option<&Config>, checks: Vec<Check>) -> Self {
        Self {
            ok: !checks.iter().any(|c| c.status == Status::Fail),
            node: config.map(|c| c.node_name.clone()),
            node_id: config.map(|c| c.node_id.clone()),
            build: kampr_node::BUILD,
            checks,
        }
    }

    pub fn counts(&self) -> (usize, usize) {
        let count = |status| self.checks.iter().filter(|c| c.status == status).count();
        (count(Status::Fail), count(Status::Warn))
    }
}

/// Exits non-zero on any failure, so `kampr doctor` composes into a health check.
pub async fn run(dirs: &Dirs, json: bool) -> Result<()> {
    let report = collect(dirs).await;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render::print(&report);
    }
    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

async fn collect(dirs: &Dirs) -> Report {
    let config_dir = dirs.config();
    let config = match Config::load(&config_dir) {
        Ok(config) => config,
        Err(e) => {
            return Report::new(None, vec![Check::fail("config", e.to_string()).fix("kampr init")]);
        }
    };
    let state_dir = config.resolve_state_dir(dirs.state_override());
    let mut checks = vec![Check::ok(
        "config",
        format!("{}", Config::path(&config_dir).display()),
    )];
    let service = crate::service::details();
    checks.extend(herd::checks(&config, service.installed).await);
    checks.extend(exposure::checks(&config));
    checks.push(origin::check(&config).await);
    checks.push(assetlinks::check(&config).await);
    checks.extend(host::files(&config_dir, &state_dir));
    checks.push(host::bundle());
    checks.push(host::service(&config, &service).await);
    checks.push(host::linger(&service));
    checks.push(host::fleet_path(&config));

    // The device store is the one check that needs the database open, and a database that will
    // not open is itself the answer.
    match Local::open(&config_dir, Some(&state_dir)).await {
        Ok(local) => checks.extend(host::access(&local).await),
        // `kampr init --force` used to be the remedy here for every failure. It opens the same
        // database and dies with the same error, and for a permissions or locking problem it
        // would have re-paired every device to fix nothing.
        Err(e) => checks.push(match e.downcast_ref::<StoreProblem>() {
            Some(problem) => Check::fail(
                "devices",
                format!("the device store did not open: {}", problem.detail),
            )
            .fix(problem.fix.clone()),
            None => Check::fail("devices", format!("the device store did not open: {e}")).fix(format!(
                "check {} is readable by this user",
                Config::state_db(&state_dir).display()
            )),
        }),
    }
    Report::new(Some(&config), checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_not_ok_when_anything_failed() {
        let warn = Report::new(None, vec![Check::warn("a", "x"), Check::ok("b", "y")]);
        assert!(warn.ok, "a warning is not a broken node");
        assert_eq!(warn.counts(), (0, 1));

        let broken = Report::new(None, vec![Check::fail("a", "x"), Check::warn("b", "y")]);
        assert!(!broken.ok);
        assert_eq!(broken.counts(), (1, 1));
    }
}
