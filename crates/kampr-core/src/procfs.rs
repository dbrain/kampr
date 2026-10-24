//! What the node can see about a pane by looking at its own machine.
//!
//! herdr's `pane.process_info` gives a shell pid and gives up on the job whenever the foreground
//! process group is the shell's own — which is every interactive shell on a machine that sources
//! ble.sh (probe #297). The node is on that machine, as that user, so the shell's children answer
//! the question herdr declined to: `claude`, `ssh` and `herdr` are all reachable there, under
//! shells whose child shares the shell's pgid.
//!
//! **This is not a second herd model.** herdr owns which panes exist and what is in them; this
//! reads one machine about one pane herdr has already named, and it answers nothing at all where
//! the machine will not describe its processes.

pub use kampr_herdr::model::is_shell;

use kampr_herdr::ForegroundProcess;
use kampr_proc::Table;
use std::collections::HashSet;

/// How far below the shell a walk goes.
///
/// The job is the shell's own child and a harness launched through a wrapper is one hop under
/// that, so the depth that answers both questions is small. What the bound actually buys is the
/// other direction: a pane running a build has a tree of hundreds under it, and none of it is
/// what the pane is doing.
const MAX_DEPTH: usize = 4;

/// The most processes one pane's walk will read. A ceiling, not a target: it is reached only by a
/// pane whose job has fanned out, and a set that large has stopped identifying anything.
const MAX_PROCESSES: usize = 64;

/// A read of one pane's process tree. **Never held across a sweep**: a `children` file goes on
/// naming a child that has exited until it is reaped, so the only honest walk is a fresh one.
#[derive(Debug, Clone, Default)]
pub struct Foreground {
    /// The nearest real job down each branch below the shell, in the order the shell forked them
    /// — which is a pipeline's own order. Empty for a pane sitting at its prompt.
    pub jobs: Vec<ForegroundProcess>,
    /// Every process the walk reached, job and descendant alike, **nearest to the shell first**.
    ///
    /// This is what a pid-keyed session marker is intersected with. It has to be the whole set
    /// rather than the one that matched a name, because a harness launched through a wrapper is
    /// named after the wrapper — and it has to be in this order, because an agent that spawns an
    /// agent leaves two markers under one shell and only the nearer one is the pane's.
    pub all: Vec<ForegroundProcess>,
}

/// What is running under `shell`, or nothing at all — which is the answer on a pane at its
/// prompt, on a host that will not say, and for a pid that has already gone. All three degrade
/// to the behaviour of a node that never looked.
pub fn below(table: &impl Table, shell: u32) -> Foreground {
    let mut walk = Walk {
        table,
        seen: HashSet::from([shell]),
        all: Vec::new(),
    };
    let jobs = walk.branches(shell, 0);
    // Read depth-first, because a branch is only resolved by following it — and offered up
    // breadth-first, because nearness to the shell is what makes a marker this pane's.
    walk.all.sort_by_key(|(depth, _)| *depth);
    Foreground {
        jobs,
        all: walk.all.into_iter().map(|(_, process)| process).collect(),
    }
}

/// **The staleness guard, and the whole of it.** A child stays listed under its parent until it
/// is reaped, so the list alone would name a job that has exited — worse than naming none, and
/// exactly what a pane at its prompt would then be called. One that has gone cannot be read, and
/// one that has exited unreaped has lost its command line, so a command line is the proof of life
/// this takes.
fn read(table: &impl Table, pid: u32) -> Option<ForegroundProcess> {
    let argv = table.argv(pid)?;
    let first = argv.first()?;
    // `comm` is what herdr's own names are compared against, and Linux caps it at 15 bytes — so a
    // long binary falls back to its own argv rather than to a truncation nothing would match.
    let name = match table.comm(pid) {
        Some(comm) if comm.len() < 15 => comm,
        _ => first.rsplit('/').next().unwrap_or(first).to_string(),
    };
    Some(ForegroundProcess {
        pid,
        name,
        cmdline: Some(argv.join(" ")),
        argv,
    })
}

struct Walk<'a, T> {
    table: &'a T,
    /// The kernel's child lists cannot loop, but a walk that trusts one unconditionally is a walk
    /// a wrong root can hang.
    seen: HashSet<u32>,
    all: Vec<(usize, ForegroundProcess)>,
}

impl<T: Table> Walk<'_, T> {
    fn branches(&mut self, pid: u32, depth: usize) -> Vec<ForegroundProcess> {
        self.table
            .children(pid)
            .into_iter()
            .flat_map(|child| self.nearest(child, depth))
            .collect()
    }

    /// The nearest thing down this branch that is not a shell.
    ///
    /// A shell is descended through rather than named: `bash /path/to/script` is how a job gets
    /// run, and naming the pane `bash` would be naming it after the one word the template exists
    /// to avoid. The whole branch is read into [`Walk::all`] either way, because a harness
    /// launched through a wrapper sits below the job the pane is named after.
    fn nearest(&mut self, pid: u32, depth: usize) -> Vec<ForegroundProcess> {
        if depth >= MAX_DEPTH || self.all.len() >= MAX_PROCESSES || !self.seen.insert(pid) {
            return Vec::new();
        }
        let Some(process) = read(self.table, pid) else {
            return Vec::new();
        };
        let shell = is_shell(&process.name);
        self.all.push((depth, process.clone()));
        let below = self.branches(pid, depth + 1);
        match shell {
            true => below,
            false => vec![process],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_proc::Procfs;

    struct Fake(tempfile::TempDir);

    impl Fake {
        fn new() -> Self {
            Self(tempfile::tempdir().expect("tempdir"))
        }

        fn below(&self, shell: u32) -> Foreground {
            below(&Procfs::at(self.0.path()), shell)
        }

        fn process(&self, pid: u32, argv: &[&str], children: &[u32]) {
            let task = self
                .0
                .path()
                .join(pid.to_string())
                .join("task")
                .join(pid.to_string());
            std::fs::create_dir_all(&task).expect("mkdir");
            let dir = self.0.path().join(pid.to_string());
            std::fs::write(dir.join("cmdline"), argv.join("\0")).expect("cmdline");
            std::fs::write(dir.join("comm"), format!("{}\n", argv[0])).expect("comm");
            let listed: Vec<String> = children.iter().map(u32::to_string).collect();
            std::fs::write(task.join("children"), listed.join(" ")).expect("children");
        }

        /// A process that has exited and not been reaped: the entry is still there, and the
        /// command line is gone.
        fn zombie(&self, pid: u32) {
            self.process(pid, &["gone"], &[]);
            std::fs::write(self.0.path().join(pid.to_string()).join("cmdline"), "").expect("empty");
        }
    }

    #[test]
    fn a_shell_with_nothing_under_it_names_no_job() {
        let fake = Fake::new();
        fake.process(1, &["bash"], &[]);
        assert!(fake.below(1).jobs.is_empty());
    }

    #[test]
    fn a_root_that_cannot_be_read_at_all_names_no_job_rather_than_refusing_to_answer() {
        assert!(below(&Procfs::at("/nowhere-at-all"), 1).jobs.is_empty());
    }

    #[test]
    fn a_child_that_has_exited_is_not_a_job() {
        let fake = Fake::new();
        fake.process(1, &["bash"], &[2, 3]);
        fake.zombie(2);
        assert!(fake.below(1).jobs.is_empty(), "and 3 does not exist");
    }

    #[test]
    fn a_wrapper_shell_is_descended_through_and_the_job_below_it_is_the_answer() {
        let fake = Fake::new();
        fake.process(1, &["bash"], &[2]);
        fake.process(2, &["bash", "/usr/bin/llm-review"], &[3]);
        fake.process(3, &["claude", "--print"], &[]);
        let walked = fake.below(1);
        assert_eq!(walked.jobs.len(), 1);
        assert_eq!(walked.jobs[0].name, "claude");
        assert_eq!(
            walked.all.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![2, 3],
            "and the wrapper is still a candidate pid even though it is not the name"
        );
    }

    /// `hod-scripts/llm-review` is a real shape on this machine: a shell whose job is an agent
    /// whose job is another agent. Both write a pid-keyed session marker, and only the nearer one
    /// is the session the pane is having.
    #[test]
    fn the_nearest_processes_come_first_so_a_pane_matches_before_what_its_job_spawned() {
        let fake = Fake::new();
        fake.process(1, &["bash"], &[2, 4]);
        fake.process(2, &["claude"], &[3]);
        fake.process(3, &["claude", "--print"], &[]);
        fake.process(4, &["ssh", "elsewhere"], &[]);
        assert_eq!(
            fake.below(1).all.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![2, 4, 3],
        );
    }

    #[test]
    fn a_pipeline_keeps_every_member_in_the_order_the_shell_forked_them() {
        let fake = Fake::new();
        fake.process(1, &["bash"], &[3, 2]);
        fake.process(2, &["sleep", "9"], &[]);
        fake.process(3, &["cat"], &[]);
        let jobs = fake.below(1).jobs;
        assert_eq!(
            jobs.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![2, 3],
            "listed by pid, which is fork order, not by the order the children file happened to be written in"
        );
    }

    #[test]
    fn the_walk_stops_rather_than_reading_a_whole_machine_out_of_one_pane() {
        let fake = Fake::new();
        for pid in 1..=(MAX_DEPTH as u32 + 4) {
            fake.process(pid, &["bash"], &[pid + 1]);
        }
        assert_eq!(fake.below(1).all.len(), MAX_DEPTH);
    }
}
