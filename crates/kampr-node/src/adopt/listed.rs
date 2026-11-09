use super::Ledger;
use kampr_core::wire::ContainerEntry;

/// Which containers are adopted and whether that is failing, and a row for each remembered one
/// the daemon no longer has, so it can be released. Only while the daemon answers: a daemon that
/// is down lists nothing, and that is not every adoption gone.
pub fn annotate(ledger: &Ledger, containers: &mut Vec<ContainerEntry>, daemon_up: bool) {
    for container in containers.iter_mut() {
        container.child = ledger.child_of(&container.name);
        if container.child.is_some() {
            container.adopt_error = ledger.failure(&container.name);
        }
    }
    if !daemon_up {
        return;
    }
    for record in ledger.records() {
        if containers.iter().any(|c| c.name == record.name) {
            continue;
        }
        containers.push(ContainerEntry {
            id: record.name.clone(),
            name: record.name.clone(),
            state: "missing".into(),
            adopt_error: ledger.failure(&record.name),
            child: Some(record.node_id),
            ..ContainerEntry::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::Record;
    use super::*;

    fn entry(name: &str) -> ContainerEntry {
        ContainerEntry {
            id: format!("{name}-id"),
            name: name.into(),
            state: "running".into(),
            ..ContainerEntry::default()
        }
    }

    #[test]
    fn an_adoption_that_is_failing_says_why_and_a_vanished_one_is_still_listed() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(dir.path());
        for (name, node_id) in [("db", "01JH-c-a"), ("web", "01JH-c-b"), ("gone", "01JH-c-c")] {
            ledger
                .remember(Record {
                    name: name.into(),
                    node_id: node_id.into(),
                })
                .unwrap();
        }
        ledger.fail(
            "web",
            Some("kampr could not be copied into web: it has no space left".into()),
        );
        ledger.fail("gone", Some("gone no longer exists".into()));
        ledger.fail("stray", Some("not adopted".into()));

        let mut listed = vec![entry("db"), entry("web"), entry("stray")];
        annotate(&ledger, &mut listed, true);
        let by = |name: &str| listed.iter().find(|c| c.name == name).unwrap().clone();
        assert_eq!(by("db").child.as_deref(), Some("01JH-c-a"));
        assert_eq!(by("db").adopt_error, None);
        assert_eq!(
            by("web").adopt_error.as_deref(),
            Some("kampr could not be copied into web: it has no space left")
        );
        assert_eq!(by("stray").adopt_error, None, "only an adoption fails");
        let gone = by("gone");
        assert_eq!(
            (gone.id.as_str(), gone.state.as_str(), gone.child.as_deref()),
            ("gone", "missing", Some("01JH-c-c"))
        );
        assert_eq!(gone.adopt_error.as_deref(), Some("gone no longer exists"));

        let mut down = Vec::new();
        annotate(&ledger, &mut down, false);
        assert!(down.is_empty(), "{down:?}");
    }
}
