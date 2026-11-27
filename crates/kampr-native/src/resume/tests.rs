use super::store::{Owner, Store};
use super::{Mode, Resume, stores_for};
use kampr_core::resume::Command;
use std::collections::HashSet;
use std::sync::{Arc, Barrier};

#[test]
fn nothing_is_stored_for_root_and_everything_for_anyone_else() {
    for (uid, stored) in [(0, false), (1, true), (1000, true), (65534, true)] {
        assert_eq!(
            stores_for(rustix::process::Uid::from_raw(uid)),
            stored,
            "uid {uid}"
        );
    }
}

fn owner(pid: u32) -> Owner {
    Owner {
        pid,
        start: None,
        time: 1,
        command: Command {
            argv: vec!["true".into()],
            cwd: None,
        },
        at: 1,
        tried: false,
    }
}

// Each taker opens its own store, as `kampr attach --restored` in another process does.
#[test]
fn a_record_raced_for_by_separate_stores_is_taken_once() {
    for round in 0..40 {
        let dir = tempfile::tempdir().unwrap();
        let resume = Resume {
            mode: Mode::Ask,
            dir: dir.path().to_path_buf(),
        };
        Store::new(resume.dir.clone())
            .update("abc", |file| {
                file.offset = 1;
                file.owners = vec![owner(7), owner(8)];
            })
            .unwrap();
        let takers = 8;
        let gate = Arc::new(Barrier::new(takers));
        let taken = (0..takers)
            .map(|_| {
                let (resume, gate) = (resume.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    super::take(&resume, &HashSet::new(), "abc/7").is_some()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|&t| t)
            .count();
        assert_eq!(taken, 1, "round {round}: one record was launched {taken} times");
        let left = Store::new(resume.dir.clone()).read("abc");
        assert_eq!(
            left.owners.iter().map(|o| o.pid).collect::<Vec<_>>(),
            [8],
            "round {round}: the other owner was lost"
        );
    }
}
