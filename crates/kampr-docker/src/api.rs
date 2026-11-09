use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Inspect {
    pub id: String,
    pub name: String,
    pub image: String,
    pub created: String,
    pub path: String,
    #[serde(deserialize_with = "nullable")]
    pub args: Vec<String>,
    pub state: State,
    pub restart_count: u32,
    pub config: Config,
    pub host_config: HostConfig,
    #[serde(deserialize_with = "nullable")]
    pub mounts: Vec<Mount>,
    pub network_settings: NetworkSettings,
}

impl Inspect {
    pub fn short_name(&self) -> &str {
        self.name.trim_start_matches('/')
    }

    pub fn label(&self, key: &str) -> Option<&str> {
        self.config.labels.get(key).map(String::as_str)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct State {
    pub status: String,
    pub running: bool,
    #[serde(rename = "OOMKilled")]
    pub oom_killed: bool,
    pub exit_code: i64,
    pub started_at: String,
    pub finished_at: String,
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Health {
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub log: Vec<Probe>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Probe {
    pub start: String,
    pub end: String,
    pub exit_code: i64,
    pub output: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Config {
    pub image: String,
    #[serde(deserialize_with = "nullable")]
    pub env: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub labels: BTreeMap<String, String>,
    pub working_dir: String,
    pub tty: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct HostConfig {
    pub restart_policy: RestartPolicy,
    pub init: Option<bool>,
    pub readonly_rootfs: bool,
    pub memory: i64,
    pub nano_cpus: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RestartPolicy {
    pub name: String,
    pub maximum_retry_count: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Mount {
    pub source: String,
    pub destination: String,
    #[serde(rename = "RW")]
    pub rw: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NetworkSettings {
    #[serde(deserialize_with = "nullable")]
    pub ports: BTreeMap<String, Option<Vec<Binding>>>,
    #[serde(deserialize_with = "nullable")]
    pub networks: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Binding {
    pub host_ip: String,
    pub host_port: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Summary {
    #[serde(rename = "Id")]
    pub id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Event {
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(rename = "Action")]
    pub action: String,
    #[serde(rename = "Actor")]
    pub actor: Actor,
    pub time: i64,
    #[serde(rename = "timeNano")]
    pub time_nano: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Actor {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Attributes", deserialize_with = "nullable")]
    pub attributes: BTreeMap<String, String>,
}

/// Docker writes `0001-01-01T00:00:00Z` for a time that never happened.
pub fn unix(rfc3339: &str) -> Option<i64> {
    let at = OffsetDateTime::parse(rfc3339, &Rfc3339).ok()?.unix_timestamp();
    (at > 0).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_times_read_as_unix_seconds_and_the_zero_time_as_none() {
        for (text, want) in [
            ("2026-11-09T10:23:53.7443047Z", Some(1794219833)),
            ("2026-11-09T21:23:54.51154029+11:00", Some(1794219834)),
            ("0001-01-01T00:00:00Z", None),
            ("", None),
        ] {
            assert_eq!(unix(text), want, "{text}");
        }
    }

    #[test]
    fn an_inspect_reads_its_state_health_and_settings() {
        let running: Inspect =
            serde_json::from_str(include_str!("../tests/fixtures/inspect-running.json")).unwrap();
        assert_eq!(running.short_name(), "kt-fx-run");
        assert!(running.state.running);
        assert_eq!(running.state.health.as_ref().unwrap().status, "healthy");
        assert_eq!(running.state.health.as_ref().unwrap().log.len(), 4);
        assert_eq!(running.label("com.docker.compose.project"), Some("ktproj"));
        assert_eq!(running.host_config.memory, 64 << 20);
        assert!(running.config.env.iter().any(|e| e == "FOO=bar"));
        let oom: Inspect = serde_json::from_str(include_str!("../tests/fixtures/inspect-oom.json")).unwrap();
        assert!(oom.state.oom_killed);
        assert_eq!(oom.state.exit_code, 137);
        let mounted: Inspect =
            serde_json::from_str(include_str!("../tests/fixtures/inspect-mounted.json")).unwrap();
        assert_eq!(mounted.mounts[0].destination, "/host-tmp");
        assert!(!mounted.mounts[0].rw);
    }
}
