use crate::journal::Entry;
use kampr_core::wire::CrashEntry;
use std::collections::HashMap;

pub const WINDOW_SECS: i64 = 24 * 3600;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Crashes {
    pub count: u32,
    pub last: Option<CrashEntry>,
}

/// How close an `oom`, `kill` or `stop` has to be to a `die` to be its cause. Measured: `oom` lands
/// ~100 ms before its `die`, and `docker stop` journals `kill`, `stop` and `die` in one second.
const BESIDE_SECS: i64 = 2;

pub fn crashes(entries: &[Entry], now: i64) -> HashMap<String, Crashes> {
    let mut by_id: HashMap<&str, Vec<&Entry>> = HashMap::new();
    for e in entries {
        by_id.entry(&e.id).or_default().push(e);
    }
    by_id
        .into_iter()
        .filter_map(|(id, mine)| {
            let near = |die: &Entry, action: &str, before: i64, after: i64| {
                mine.iter()
                    .any(|e| e.action == action && e.at >= die.at - before && e.at <= die.at + after)
            };
            let mut found = Crashes::default();
            for die in mine
                .iter()
                .filter(|e| e.action == "die" && e.exit_code.is_some_and(|c| c != 0))
            {
                if near(die, "kill", BESIDE_SECS, 0) || near(die, "stop", BESIDE_SECS, BESIDE_SECS) {
                    continue;
                }
                if die.at > now - WINDOW_SECS {
                    found.count += 1;
                }
                found.last = Some(CrashEntry {
                    at: die.at,
                    exit_code: die.exit_code.unwrap_or_default(),
                    oom: near(die, "oom", BESIDE_SECS, 0),
                });
            }
            found.last.is_some().then(|| (id.to_string(), found))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::tests::journalled;

    fn of<'a>(entries: &[Entry], all: &'a HashMap<String, Crashes>, name: &str) -> Option<&'a Crashes> {
        let id = &entries.iter().find(|e| e.name == name)?.id;
        all.get(id)
    }

    #[test]
    fn a_restart_policy_loop_counts_every_non_zero_exit() {
        let entries = journalled(include_str!("../tests/fixtures/events-life.jsonl"));
        let all = crashes(&entries, 1794219976);
        let looped = of(&entries, &all, "kt-fx-loop").expect("the loop crashed");
        assert_eq!(looped.count, 3);
        let last = looped.last.as_ref().unwrap();
        assert_eq!((last.at, last.exit_code, last.oom), (1794219975, 3, false));
    }

    #[test]
    fn an_oom_event_before_a_die_marks_the_crash_oom() {
        let entries = journalled(include_str!("../tests/fixtures/events.jsonl"));
        let all = crashes(&entries, 1794219840);
        let oom = of(&entries, &all, "kt-fx-oom").unwrap();
        assert_eq!(oom.count, 1);
        assert_eq!(
            oom.last,
            Some(CrashEntry {
                at: 1794219834,
                exit_code: 137,
                oom: true
            })
        );
        let exit = of(&entries, &all, "kt-fx-exit").unwrap();
        assert_eq!(
            exit.last,
            Some(CrashEntry {
                at: 1794219834,
                exit_code: 3,
                oom: false
            })
        );
    }

    #[test]
    fn a_clean_exit_or_one_somebody_asked_for_is_not_a_crash() {
        let entries = journalled(include_str!("../tests/fixtures/events-life.jsonl"));
        let mut alone: Vec<Entry> = entries.iter().filter(|e| e.action == "die").cloned().collect();
        for e in &mut alone {
            e.exit_code = Some(0);
        }
        let all = crashes(&alone, 1794219976);
        assert!(all.is_empty(), "exit 0 is no crash: {all:?}");

        let mut stopped = entries.clone();
        for e in stopped
            .iter_mut()
            .filter(|e| e.name == "kt-fx-life" && e.action == "die")
        {
            e.exit_code = Some(143);
        }
        let all = crashes(&stopped, 1794219976);
        assert_eq!(
            of(&stopped, &all, "kt-fx-life"),
            None,
            "a die beside a kill or a stop was asked for"
        );
    }

    #[test]
    fn crashes_count_a_day_and_the_last_one_is_kept_after_it() {
        let entries = journalled(include_str!("../tests/fixtures/events-life.jsonl"));
        let all = crashes(&entries, 1794219975 + WINDOW_SECS + 1);
        let looped = of(&entries, &all, "kt-fx-loop").unwrap();
        assert_eq!(looped.count, 0);
        assert_eq!(looped.last.as_ref().unwrap().exit_code, 3);
    }
}
