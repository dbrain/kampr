use kampr_journal::process::{self, PaneProcess};
use kampr_journal::{agy, claude, codex, omp};

const HARNESSES: &[&str] = &[claude::AGENT, codex::AGENT, omp::AGENT, omp::PI_AGENT, agy::AGENT];

pub(crate) struct Foreground {
    pub pid: u32,
    pub name: String,
    pub argv: Vec<String>,
}

/// The PTY's foreground process group leader, which is what a terminal means by "the job". A
/// group whose leader has already exited falls back to the session's own child.
pub(crate) fn foreground(child: u32) -> Option<Foreground> {
    let pid = match tpgid(child) {
        Some(group) if group > 0 && process::comm(group).is_some() => group,
        _ => child,
    };
    Some(Foreground {
        pid,
        name: process::comm(pid)?,
        argv: cmdline(pid),
    })
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

fn tpgid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(") ")?;
    rest.split_ascii_whitespace()
        .nth(5)?
        .parse::<i64>()
        .ok()?
        .try_into()
        .ok()
}

fn cmdline(pid: u32) -> Vec<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|raw| {
            raw.split(|&b| b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn cwd(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.display().to_string())
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
