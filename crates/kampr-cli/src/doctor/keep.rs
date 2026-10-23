use super::Check;
use kampr_keep::SessionDir;
use kampr_node::Config;
use std::collections::HashMap;
use std::process::{Command, Stdio};

const ID: &str = "keep";
const UPGRADE: &str = "systemctl --user kill -s USR1 'kampr-keep-*'";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    id: String,
    live: bool,
    keeper: Option<u32>,
    unit: Option<String>,
    stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Unit {
    active: String,
    fds: u32,
}

/// What the keepers on this host need, read without asking any of them anything: a keeper serves
/// no protocol, so its directory, its cgroup and its unit are the whole of what there is to read.
pub fn check(config: &Config) -> Check {
    let root = match config.native.resolve() {
        Ok(native) => native.root,
        Err(e) => {
            return Check::warn(ID, format!("the session root could not be made: {e:#}"))
                .fix("set native.root in config.toml to a directory this user owns");
        }
    };
    let kept: Vec<Kept> = kampr_keep::list_tree(&root)
        .unwrap_or_default()
        .iter()
        .map(kept)
        .collect();
    let units: Vec<&str> = kept.iter().filter_map(|k| k.unit.as_deref()).collect();
    judge(&kept, &show(&units))
}

fn kept(session: &SessionDir) -> Kept {
    let live = session.alive();
    let keeper = session.keeper().filter(|_| live);
    let unit = kampr_keep::unit_name(session.id());
    Kept {
        id: session.id().to_string(),
        live,
        keeper: keeper.as_ref().map(|k| k.keeper),
        // A detached keeper (no systemd, or a test) is in nobody's unit, and asking systemd about
        // a unit name that was never made would read as a failed one.
        unit: keeper.as_ref().filter(|k| in_unit(k.keeper, &unit)).map(|_| unit),
        stale: kampr_keep::stale(session),
    }
}

fn in_unit(pid: u32, unit: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .is_ok_and(|cgroup| cgroup.contains(&format!("/{unit}.service")))
}

/// `None` is a user manager that did not answer, which is not the same as units that are down.
fn show(units: &[&str]) -> Option<HashMap<String, Unit>> {
    if units.is_empty() {
        return Some(HashMap::new());
    }
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "--property=Id,ActiveState,NFileDescriptorStore",
            "--",
        ])
        .args(units)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(parse_show(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_show(text: &str) -> HashMap<String, Unit> {
    text.split("\n\n")
        .filter_map(|block| {
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                    .map(str::to_string)
            };
            let id = field("Id")?;
            Some((
                id.trim_end_matches(".service").to_string(),
                Unit {
                    active: field("ActiveState").unwrap_or_default(),
                    fds: field("NFileDescriptorStore")
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0),
                },
            ))
        })
        .collect()
}

fn judge(kept: &[Kept], units: &Option<HashMap<String, Unit>>) -> Check {
    if kept.is_empty() {
        return Check::ok(ID, "no sessions kept on this host");
    }
    let live: Vec<&Kept> = kept.iter().filter(|k| k.live).collect();
    let under_systemd = live.iter().filter(|k| k.unit.is_some()).count();
    let mut detail = format!(
        "{} kept: {} live ({under_systemd} under systemd, {} detached), {} ended",
        kept.len(),
        live.len(),
        live.len() - under_systemd,
        kept.len() - live.len(),
    );

    let Some(units) = units else {
        return Check::warn(
            ID,
            format!("{detail}; the user manager did not answer for {under_systemd} keeper units"),
        )
        .fix("systemctl --user status 'kampr-keep-*'");
    };
    let named = |pick: &dyn Fn(&Unit) -> bool| -> Vec<&str> {
        live.iter()
            .filter(|k| k.unit.as_ref().is_some_and(|u| units.get(u).is_none_or(pick)))
            .map(|k| k.id.as_str())
            .collect()
    };
    let down = named(&|u| u.active != "active");
    if !down.is_empty() {
        return Check::fail(
            ID,
            format!(
                "{detail}; the keeper unit is not active for {}, so nothing holds the PTY of a \
                 program that is still running",
                down.join(", ")
            ),
        )
        .fix(format!("systemctl --user status {}", unit_list(&down)));
    }
    let unstored = named(&|u| u.active == "active" && u.fds == 0);
    let stale: Vec<&Kept> = live.iter().copied().filter(|k| k.stale).collect();
    if unstored.is_empty() && stale.is_empty() {
        return Check::ok(ID, detail);
    }
    let mut fixes = Vec::new();
    if !unstored.is_empty() {
        detail.push_str(&format!(
            "; systemd's fd store holds no PTY for {}, so a keeper crash would hang its program up \
             (#590)",
            unstored.join(", ")
        ));
        fixes.push("close and restart those sessions once their work is done".to_string());
    }
    if !stale.is_empty() {
        detail.push_str(&format!(
            "; {} still run the binary from before it was replaced, which the node re-execs at \
             its start and every minute after",
            stale.iter().map(|k| k.id.as_str()).collect::<Vec<_>>().join(", ")
        ));
        if stale.iter().any(|k| k.unit.is_some()) {
            fixes.push(UPGRADE.to_string());
        }
        let detached: Vec<String> = stale
            .iter()
            .filter(|k| k.unit.is_none())
            .filter_map(|k| k.keeper.map(|p| p.to_string()))
            .collect();
        if !detached.is_empty() {
            fixes.push(format!("kill -USR1 {}", detached.join(" ")));
        }
    }
    Check::warn(ID, detail).fix(fixes.join("; "))
}

fn unit_list(ids: &[&str]) -> String {
    ids.iter()
        .map(|id| kampr_keep::unit_name(id))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::super::Status;
    use super::*;

    fn kept(id: &str, live: bool, unit: bool, stale: bool) -> Kept {
        Kept {
            id: id.into(),
            live,
            keeper: live.then_some(4242),
            unit: (live && unit).then(|| kampr_keep::unit_name(id)),
            stale,
        }
    }

    fn units(states: &[(&str, &str, u32)]) -> Option<HashMap<String, Unit>> {
        Some(
            states
                .iter()
                .map(|(id, active, fds)| {
                    (
                        kampr_keep::unit_name(id),
                        Unit {
                            active: active.to_string(),
                            fds: *fds,
                        },
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn every_keeper_fault_has_its_own_verdict_and_fix() {
        for (name, kept, units, status, detail, fix) in [
            ("nothing", vec![], units(&[]), Status::Ok, "no sessions", None),
            (
                "healthy",
                vec![kept("a", true, true, false), kept("b", false, false, false)],
                units(&[("a", "active", 1)]),
                Status::Ok,
                "2 kept: 1 live (1 under systemd, 0 detached), 1 ended",
                None,
            ),
            (
                "unit down",
                vec![kept("a", true, true, false)],
                units(&[("a", "failed", 1)]),
                Status::Fail,
                "not active for a",
                Some("systemctl --user status kampr-keep-a"),
            ),
            (
                "unit unknown to systemd",
                vec![kept("a", true, true, false)],
                units(&[]),
                Status::Fail,
                "not active for a",
                Some("kampr-keep-a"),
            ),
            (
                "fd store empty",
                vec![kept("a", true, true, false)],
                units(&[("a", "active", 0)]),
                Status::Warn,
                "fd store holds no PTY for a",
                Some("close and restart"),
            ),
            (
                "stale under systemd",
                vec![kept("a", true, true, true)],
                units(&[("a", "active", 1)]),
                Status::Warn,
                "a still run the binary",
                Some(UPGRADE),
            ),
            (
                "stale and detached",
                vec![kept("a", true, false, true)],
                units(&[]),
                Status::Warn,
                "a still run the binary",
                Some("kill -USR1 4242"),
            ),
            (
                "no user manager",
                vec![kept("a", true, true, false)],
                None,
                Status::Warn,
                "did not answer",
                Some("systemctl --user status"),
            ),
        ] {
            let check = judge(&kept, &units);
            assert_eq!(check.status, status, "{name}: {check:?}");
            assert!(check.detail.contains(detail), "{name}: {check:?}");
            match fix {
                Some(fix) => assert!(
                    check.fix.as_deref().is_some_and(|f| f.contains(fix)),
                    "{name}: {check:?}"
                ),
                None => assert_eq!(check.fix, None, "{name}"),
            }
        }
    }

    #[test]
    fn systemctl_show_is_read_one_unit_per_block() {
        let units = parse_show(
            "Id=kampr-keep-a.service\nActiveState=active\nNFileDescriptorStore=1\n\n\
             NFileDescriptorStore=0\nId=kampr-keep-b.service\nActiveState=inactive\n",
        );
        assert_eq!(
            units.get("kampr-keep-a"),
            Some(&Unit {
                active: "active".into(),
                fds: 1
            })
        );
        assert_eq!(
            units.get("kampr-keep-b"),
            Some(&Unit {
                active: "inactive".into(),
                fds: 0
            })
        );
    }
}
