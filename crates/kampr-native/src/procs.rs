use kampr_core::procfs::{self, is_shell};
use kampr_journal::process::{self, PaneProcess};
use kampr_journal::{agy, claude, codex, omp};
use kampr_proc::Table;

const HARNESSES: &[&str] = &[claude::AGENT, codex::AGENT, omp::AGENT, omp::PI_AGENT, agy::AGENT];

pub(crate) struct Foreground {
    pub pid: u32,
    pub name: String,
    pub argv: Vec<String>,
}

/// The PTY's foreground process group leader, which is what a terminal means by "the job". A
/// group whose leader has already exited falls back to the session's own child.
pub(crate) fn foreground(child: u32) -> Option<Foreground> {
    let pid = match kampr_proc::here().tpgid(child) {
        Some(group) if group > 0 && process::comm(group).is_some() => group,
        _ => child,
    };
    Some(Foreground {
        pid,
        name: process::comm(pid)?,
        argv: cmdline(pid),
    })
}

/// A shell is never the job, as on a herdr pane: at its prompt there is none, and under ble.sh the
/// job shares the shell's group (#297), so it is read from below the shell.
pub(crate) fn job(fg: &Foreground) -> Option<String> {
    if !is_shell(&fg.name) {
        return Some(fg.name.clone());
    }
    procfs::below(&kampr_proc::here(), fg.pid)
        .jobs
        .into_iter()
        .next()
        .map(|job| job.name)
}

/// Matched on the process name and on the first two words of its command line, because an npm
/// install runs a harness as `node …/codex` and omp runs under `bun`. Searched breadth first below
/// the job, so a wrapper script (or a shell under it) is looked through and the shallowest match
/// wins: a harness's own children are not the harness.
pub(crate) fn harness(fg: &Foreground) -> Option<(&'static str, PaneProcess)> {
    if let Some(name) = named(&fg.name, &fg.argv) {
        return Some((name, PaneProcess::look_up(fg.pid)));
    }
    descendants(fg.pid).into_iter().skip(1).find_map(|pid| {
        let name = named(&process::comm(pid)?, &cmdline(pid))?;
        Some((name, PaneProcess::look_up(pid)))
    })
}

pub(crate) fn descendants(root: u32) -> Vec<u32> {
    let mut found = vec![root];
    let mut at = 0;
    while let Some(&pid) = found.get(at) {
        for child in process::children(pid) {
            if !found.contains(&child) {
                found.push(child);
            }
        }
        at += 1;
    }
    found
}

fn named(comm: &str, argv: &[String]) -> Option<&'static str> {
    std::iter::once(comm)
        .chain(argv.iter().take(2).map(|a| stem(a)))
        .find_map(|n| HARNESSES.iter().find(|h| **h == n).copied())
}

fn stem(arg: &str) -> &str {
    let base = arg.rsplit('/').next().unwrap_or(arg);
    base.split_once('.').map_or(base, |(stem, _)| stem)
}

fn cmdline(pid: u32) -> Vec<String> {
    kampr_proc::here().argv(pid).unwrap_or_default()
}

pub(crate) fn cwd(pid: u32) -> Option<String> {
    kampr_proc::here().cwd(pid).map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::named;

    #[test]
    fn a_harness_is_known_by_its_name_or_by_what_its_runtime_was_handed() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (comm, args, want) in [
            ("claude", argv(&["claude", "--resume"]), Some("claude")),
            (
                "node",
                argv(&["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"]),
                Some("codex"),
            ),
            ("bun", argv(&["bun", "/home/x/.bun/bin/omp"]), Some("omp")),
            ("bash", argv(&["bash", "-l"]), None),
            ("node", argv(&["node", "server.js", "claude"]), None),
        ] {
            assert_eq!(named(comm, &args), want, "{comm} {args:?}");
        }
    }
}
