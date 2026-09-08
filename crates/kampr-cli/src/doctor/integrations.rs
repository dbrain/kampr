use super::{Check, Status};
use kampr_herdr::Herdr;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

/// herdr answers this in microseconds when it is up, and the check above this one has already
/// reported a herdr that is not.
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Deserialize)]
struct Reply {
    integrations: Vec<Integration>,
}

#[derive(Debug, Clone, Deserialize)]
struct Integration {
    target: String,
    /// Whether the harness itself is on this host at all. An integration for an agent nobody has
    /// installed is not news.
    available: bool,
    /// `not_installed`, `current` or `outdated`.
    state: String,
}

/// **What herdr can be told about an agent, and what it is being told.**
///
/// A harness with no screen-detection manifest — `omp` is the one that matters here, and 0.9 still
/// ships none (#511) — can only report its state through an integration, a hook installed into the
/// harness's own config. Without one the pane reports `idle` through a whole working turn and
/// through an open approval dialog (#485). That is not a fault anybody can see from the pane, and
/// it is the single most common "why does Kampr think this agent is idle" question there is. So it
/// is answered here, where somebody is already asking what is wrong.
///
/// **`outdated` warns; `not_installed` does not, and that line is deliberate.** It is the same one
/// herdr draws for itself: its own attention badge fires for `outdated` only, with a regression
/// test pinning it, and an integration nobody ever installed nags nobody. Kampr installs nothing
/// and never will — `docs/14-agent-status.md` is the rule, no hooks and no MCP — so this reports
/// and stops there.
pub async fn check(socket: &Path) -> Check {
    let Ok(reply) = Herdr::new(socket)
        .with_timeout(TIMEOUT)
        .call::<Reply>("integration.list", serde_json::json!({}))
        .await
    else {
        return Check::warn("integrations", "herdr did not answer `integration.list`").fix("herdr");
    };
    let installed: Vec<&str> = reply
        .integrations
        .iter()
        .filter(|i| i.state == "current")
        .map(|i| i.target.as_str())
        .collect();
    let outdated: Vec<&str> = reply
        .integrations
        .iter()
        .filter(|i| i.state == "outdated")
        .map(|i| i.target.as_str())
        .collect();
    let absent: Vec<&str> = reply
        .integrations
        .iter()
        .filter(|i| i.available && i.state == "not_installed")
        .map(|i| i.target.as_str())
        .collect();

    let detail = format!(
        "{} installed ({}); {} on this host without one ({})",
        installed.len(),
        listed(&installed),
        absent.len(),
        listed(&absent),
    );
    if outdated.is_empty() {
        return Check::new("integrations", Status::Ok, detail);
    }
    Check::warn(
        "integrations",
        format!("{detail}; {} out of date ({})", outdated.len(), listed(&outdated)),
    )
    .fix(format!("herdr integration install {}", outdated.join(" ")))
}

fn listed(names: &[&str]) -> String {
    match names.is_empty() {
        true => "none".into(),
        false => names.join(", "),
    }
}
