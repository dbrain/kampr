use crate::api::{Inspect, unix};
use crate::crashes::Crashes;
use kampr_core::wire::ContainerEntry;

/// How far apart the journal's `die` and inspect's `FinishedAt` may be and still be one exit.
const SAME_EXIT_SECS: i64 = 2;

pub fn entry(inspect: &Inspect, crashes: Option<&Crashes>) -> ContainerEntry {
    let state = &inspect.state;
    let finished_at = unix(&state.finished_at);
    let ended = finished_at.is_some() && !state.running;
    let last_crash = crashes.and_then(|c| c.last.clone()).map(|mut crash| {
        crash.oom |= state.oom_killed
            && crash.exit_code == state.exit_code
            && finished_at.is_some_and(|f| (f - crash.at).abs() <= SAME_EXIT_SECS);
        crash
    });
    ContainerEntry {
        id: inspect.id.clone(),
        name: inspect.short_name().to_string(),
        image: inspect.config.image.clone(),
        state: state.status.clone(),
        health: state
            .health
            .as_ref()
            .map(|h| h.status.clone())
            .filter(|s| !s.is_empty() && s != "none"),
        started_at: unix(&state.started_at),
        finished_at,
        exit_code: ended.then_some(state.exit_code),
        oom: state.oom_killed,
        restarts: inspect.restart_count,
        crashes: crashes.map_or(0, |c| c.count),
        last_crash,
        project: inspect.label("com.docker.compose.project").map(str::to_string),
        service: inspect.label("com.docker.compose.service").map(str::to_string),
        ports: ports(inspect),
        child: None,
        adopt_error: None,
    }
}

fn ports(inspect: &Inspect) -> Vec<String> {
    let mut ports: Vec<String> = inspect
        .network_settings
        .ports
        .iter()
        .flat_map(|(inside, bindings)| {
            bindings
                .iter()
                .flatten()
                .filter(|b| !b.host_port.is_empty())
                .map(move |b| format!("{}→{inside}", b.host_port))
        })
        .collect();
    ports.sort();
    ports.dedup();
    ports
}

pub fn sort(entries: &mut [ContainerEntry]) {
    entries.sort_by(|a, b| (&a.project, &a.name).cmp(&(&b.project, &b.name)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::wire::CrashEntry;

    fn inspect(fixture: &str) -> Inspect {
        serde_json::from_str(fixture).unwrap()
    }

    #[test]
    fn a_running_container_says_what_it_is_and_how_it_is() {
        let e = entry(
            &inspect(include_str!("../tests/fixtures/inspect-running.json")),
            None,
        );
        assert_eq!(e.id.len(), 64);
        assert_eq!(e.name, "kt-fx-run");
        assert_eq!(e.image, "nats:2.15.2-alpine");
        assert_eq!(e.state, "running");
        assert_eq!(e.health.as_deref(), Some("healthy"));
        assert_eq!(e.started_at, Some(1794219833));
        assert_eq!(
            (e.finished_at, e.exit_code, e.oom, e.restarts),
            (None, None, false, 0)
        );
        assert_eq!(e.project.as_deref(), Some("ktproj"));
        assert_eq!(e.service.as_deref(), Some("web"));
        assert_eq!(e.ports, vec!["18222→4222/tcp".to_string()]);
        assert_eq!((e.crashes, &e.last_crash, &e.child), (0, &None, &None));
    }

    #[test]
    fn an_exited_container_says_how_it_ended() {
        let oom = entry(&inspect(include_str!("../tests/fixtures/inspect-oom.json")), None);
        assert_eq!(
            (oom.state.as_str(), oom.exit_code, oom.oom),
            ("exited", Some(137), true)
        );
        assert!(oom.finished_at.is_some());
        assert_eq!(oom.health, None);
        let looped = entry(
            &inspect(include_str!("../tests/fixtures/inspect-loop.json")),
            None,
        );
        assert_eq!((looped.restarts, looped.exit_code), (2, Some(3)));
    }

    #[test]
    fn the_journal_counts_crashes_and_inspect_confirms_an_oom_it_missed() {
        let oom = inspect(include_str!("../tests/fixtures/inspect-oom.json"));
        let finished = crate::api::unix(&oom.state.finished_at).unwrap();
        let crashes = Crashes {
            count: 2,
            last: Some(CrashEntry {
                at: finished,
                exit_code: 137,
                oom: false,
            }),
        };
        let e = entry(&oom, Some(&crashes));
        assert_eq!(e.crashes, 2);
        assert_eq!(
            e.last_crash,
            Some(CrashEntry {
                at: finished,
                exit_code: 137,
                oom: true
            })
        );

        let older = Crashes {
            count: 1,
            last: Some(CrashEntry {
                at: finished - 600,
                exit_code: 1,
                oom: false,
            }),
        };
        assert!(
            !entry(&oom, Some(&older)).last_crash.unwrap().oom,
            "an older crash is not this one"
        );
    }

    #[test]
    fn containers_sort_by_project_then_name() {
        let named = |project: Option<&str>, name: &str| ContainerEntry {
            name: name.into(),
            project: project.map(str::to_string),
            ..ContainerEntry::default()
        };
        let mut all = vec![
            named(Some("b"), "a"),
            named(None, "z"),
            named(Some("a"), "y"),
            named(Some("a"), "x"),
            named(None, "c"),
        ];
        sort(&mut all);
        let order: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, vec!["c", "z", "x", "y", "a"]);
    }
}
