use kampr_proc::Table;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    pub procs: Vec<Proc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proc {
    pub pid: u32,
    #[serde(default)]
    pub children: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syscall: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wchan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fd0: Option<String>,
}

const DEPTH: usize = 4;

pub(crate) fn read(child: u32, tty: Option<String>) -> ProcState {
    let mut procs = Vec::new();
    walk(child, 0, &mut procs);
    ProcState { tty, procs }
}

fn walk(pid: u32, depth: usize, out: &mut Vec<Proc>) {
    let table = kampr_proc::here();
    if table.comm(pid).is_none() {
        return;
    }
    // stat, syscall, wchan and fd/0 are Linux's own, and what the fleet's `waiting` reads.
    let base = format!("/proc/{pid}");
    let children = if depth < DEPTH {
        table.children(pid)
    } else {
        Vec::new()
    };
    out.push(Proc {
        pid,
        children: children.clone(),
        stat: read_trimmed(&format!("{base}/stat")),
        syscall: read_trimmed(&format!("{base}/syscall")),
        wchan: read_trimmed(&format!("{base}/wchan")),
        fd0: fs::read_link(format!("{base}/fd/0"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
    });
    for child in children {
        walk(child, depth + 1, out);
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim_end().to_string())
}
