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
    let base = format!("/proc/{pid}");
    let Ok(stat) = fs::read_to_string(format!("{base}/stat")) else {
        return;
    };
    let children = if depth < DEPTH {
        children(&base)
    } else {
        Vec::new()
    };
    out.push(Proc {
        pid,
        children: children.clone(),
        stat: Some(stat.trim_end().to_string()),
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

fn children(base: &str) -> Vec<u32> {
    let Ok(threads) = fs::read_dir(format!("{base}/task")) else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = threads
        .flatten()
        .filter_map(|t| fs::read_to_string(t.path().join("children")).ok())
        .flat_map(|text| {
            text.split_ascii_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim_end().to_string())
}
