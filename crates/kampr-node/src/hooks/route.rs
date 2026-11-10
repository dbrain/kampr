use crate::herd::HerdModel;
use crate::sessions::Sessions;
use kampr_journal::PaneProcess;
use kampr_journal::process::{children, comm, harness_of};
use std::collections::{HashMap, HashSet};

const DEPTH: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pid: u32,
    start: Option<String>,
}

impl Proc {
    fn of(pid: u32) -> Self {
        Self {
            pid,
            start: PaneProcess::look_up(pid).start,
        }
    }

    pub fn alive(&self) -> bool {
        match &self.start {
            Some(start) => PaneProcess::look_up(self.pid).start.as_ref() == Some(start),
            None => comm(self.pid).is_some(),
        }
    }

    pub fn holds(&self, pid: u32) -> bool {
        self.alive() && path(&[self.pid], pid).is_some()
    }
}

pub struct Routed {
    pub pane: String,
    pub claude: Proc,
}

pub fn pane_of(sessions: &Sessions, herd: &HerdModel, pid: u32) -> Option<Routed> {
    herd.panes.iter().find_map(|pane| {
        let session = sessions.route(&pane.id)?;
        let local = session.local_pane(&pane.id)?;
        let roots: Vec<u32> = session.pane_processes(&local).iter().map(|p| p.pid).collect();
        let path = path(&roots, pid)?;
        Some(Routed {
            pane: pane.id.clone(),
            claude: Proc::of(claude_on(&path)),
        })
    })
}

// The hook's own shell is forked from Claude and may keep its name, so the outermost `claude`
// above the hook is the session's process; without one, the pane's own process stands in.
fn claude_on(path: &[u32]) -> u32 {
    path[..path.len().saturating_sub(1)]
        .iter()
        .copied()
        .find(|pid| harness_of(*pid) == Some(kampr_journal::claude::AGENT))
        .unwrap_or(path[0])
}

fn path(roots: &[u32], pid: u32) -> Option<Vec<u32>> {
    let mut parent: HashMap<u32, u32> = HashMap::new();
    let mut seen: HashSet<u32> = roots.iter().copied().collect();
    let mut level: Vec<u32> = roots.to_vec();
    for _ in 0..DEPTH {
        let mut next = Vec::new();
        for p in &level {
            for child in children(*p) {
                parent.entry(child).or_insert(*p);
                if seen.insert(child) {
                    next.push(child);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    if !seen.contains(&pid) {
        return None;
    }
    let mut chain = vec![pid];
    while let Some(up) = chain.last().and_then(|p| parent.get(p)).copied() {
        if chain.contains(&up) {
            break;
        }
        chain.push(up);
    }
    chain.reverse();
    Some(chain)
}
