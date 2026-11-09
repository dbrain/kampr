use crate::api::{Inspect, unix};
use crate::directory::Directory;
use crate::journal::Entry;
use crate::logs::{self, LogLine};
use crate::stats::{self, Stats};
use anyhow::Result;
use kampr_core::wire::ContainerEntry;
use serde_json::{Value, json};

pub const LOG_LINES: usize = 200;
pub const EVENTS: usize = 100;

/// Read fresh rather than from the directory: the health log changes every probe and the
/// directory does not follow it.
pub async fn detail(directory: &Directory, id: &str) -> Result<Value> {
    let client = directory.client();
    let inspect = client.inspect(id).await?;
    let (logs, stats) = tokio::join!(client.logs(&inspect.id, LOG_LINES), async {
        match inspect.state.running {
            true => client.stats(&inspect.id).await.ok(),
            false => None,
        }
    });
    let entry = directory
        .containers()
        .into_iter()
        .find(|c| c.id == inspect.id)
        .unwrap_or_else(|| crate::entry::entry(&inspect, None));
    // A `none` logging driver cannot be read back; the rest of the sheet still can.
    let logs = logs
        .map(|raw| logs::parse(&raw, inspect.config.tty))
        .unwrap_or_default();
    Ok(build(
        &inspect,
        &entry,
        &logs,
        stats.as_ref().map(stats::parse).as_ref(),
        &directory.events_of(&inspect.id, EVENTS),
    ))
}

pub fn build(
    inspect: &Inspect,
    entry: &ContainerEntry,
    logs: &[LogLine],
    stats: Option<&Stats>,
    events: &[Entry],
) -> Value {
    let mut detail = serde_json::to_value(entry).unwrap_or_else(|_| json!({}));
    let policy = &inspect.host_config.restart_policy;
    let fields = [
        ("command", json!(command(inspect))),
        ("created", json!(unix(&inspect.created))),
        (
            "restart_policy",
            json!(match policy.maximum_retry_count {
                0 => policy.name.clone(),
                n => format!("{}:{n}", policy.name),
            }),
        ),
        (
            "limits",
            json!({ "memory": inspect.host_config.memory, "nano_cpus": inspect.host_config.nano_cpus }),
        ),
        ("env", json!(inspect.config.env)),
        ("labels", json!(inspect.config.labels)),
        (
            "mounts",
            inspect
                .mounts
                .iter()
                .map(|m| {
                    json!({ "src": m.source, "dst": m.destination, "mode": if m.rw { "rw" } else { "ro" } })
                })
                .collect(),
        ),
        (
            "networks",
            json!(inspect.network_settings.networks.keys().collect::<Vec<_>>()),
        ),
        (
            "health_log",
            inspect
                .state
                .health
                .iter()
                .flat_map(|h| &h.log)
                .map(|p| {
                    json!({ "start": unix(&p.start), "end": unix(&p.end),
                            "exit_code": p.exit_code, "output": p.output })
                })
                .collect(),
        ),
        ("logs", json!(logs)),
        (
            "events",
            events
                .iter()
                .map(|e| {
                    let mut event = json!({ "at": e.at, "action": e.action });
                    for (key, value) in [
                        ("exit_code", json!(e.exit_code)),
                        ("signal", json!(e.signal)),
                        ("health", json!(e.health)),
                    ] {
                        if !value.is_null() {
                            event[key] = value;
                        }
                    }
                    event
                })
                .collect(),
        ),
    ];
    for (key, value) in fields {
        detail[key] = value;
    }
    if let Some(stats) = stats {
        detail["stats"] = json!(stats);
    }
    detail
}

fn command(inspect: &Inspect) -> String {
    std::iter::once(&inspect.path)
        .chain(&inspect.args)
        .filter(|a| !a.is_empty())
        .map(|a| {
            match a
                .chars()
                .any(|c| c.is_whitespace() || "'\"$;&|<>()`\\".contains(c))
            {
                true => format!("'{}'", a.replace('\'', r"'\''")),
                false => a.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::tests::journalled;

    fn running() -> Inspect {
        serde_json::from_str(include_str!("../tests/fixtures/inspect-running.json")).unwrap()
    }

    #[test]
    fn a_detail_is_the_entry_and_everything_the_sheet_shows() {
        let inspect = running();
        let entry = crate::entry::entry(&inspect, None);
        let logs = logs::parse(include_bytes!("../tests/fixtures/logs-mux.bin"), false);
        let stats =
            stats::parse(&serde_json::from_str(include_str!("../tests/fixtures/stats.json")).unwrap());
        let events = journalled(include_str!("../tests/fixtures/events.jsonl"));
        let mine: Vec<Entry> = events.into_iter().filter(|e| e.id == inspect.id).collect();
        let d = build(&inspect, &entry, &logs, Some(&stats), &mine);

        assert_eq!(d["name"], "kt-fx-run");
        assert_eq!(d["health"], "healthy");
        assert!(
            d["command"].as_str().unwrap().starts_with("sh -c 'i=0;"),
            "{}",
            d["command"]
        );
        assert_eq!(d["created"], 1794219833);
        assert_eq!(d["restart_policy"], "no");
        assert_eq!(d["limits"]["memory"], 64 << 20);
        assert!(d["env"].as_array().unwrap().contains(&json!("FOO=bar")));
        assert_eq!(d["labels"]["com.docker.compose.service"], "web");
        assert_eq!(d["networks"], json!(["bridge"]));
        assert_eq!(d["health_log"].as_array().unwrap().len(), 4);
        assert_eq!(d["health_log"][0]["output"], "ok\n");
        assert_eq!(d["logs"].as_array().unwrap().len(), 6);
        assert_eq!(d["logs"][0]["stream"], "stdout");
        assert_eq!(d["stats"]["mem_limit"], 67108864);
        let actions: Vec<_> = d["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].clone())
            .collect();
        assert_eq!(actions, vec![json!("start"), json!("health_status")]);
        assert_eq!(d["events"][1]["health"], "healthy");
        assert!(d["events"][0].get("exit_code").is_none());

        let stopped = build(&inspect, &entry, &[], None, &[]);
        assert!(stopped.get("stats").is_none());
    }

    #[test]
    fn a_mount_says_where_it_comes_from_and_whether_it_is_writable() {
        let inspect: Inspect =
            serde_json::from_str(include_str!("../tests/fixtures/inspect-mounted.json")).unwrap();
        let d = build(&inspect, &crate::entry::entry(&inspect, None), &[], None, &[]);
        assert_eq!(
            d["mounts"],
            json!([{ "src": "/tmp", "dst": "/host-tmp", "mode": "ro" }])
        );
    }
}
