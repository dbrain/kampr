use crate::repo;
use std::collections::BTreeMap;
use std::path::Path;

const LOG: &str = "docs/03-probe-log.md";

/// Words that put a `#N` in somebody else's numbering — herdr's changelog, an upstream issue.
const FOREIGN: &[&str] = &["changelog", "issue", "issues", "pr", "pull"];

fn rows(text: &str) -> BTreeMap<u32, Vec<String>> {
    let mut rows: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        let Some(cell) = line.strip_prefix('|').and_then(|rest| rest.split('|').next()) else {
            continue;
        };
        if let Ok(n) = cell.trim().trim_matches('~').trim().parse::<u32>() {
            rows.entry(n).or_default().push(line.to_string());
        }
    }
    rows
}

fn numbering_faults(rows: &BTreeMap<u32, Vec<String>>) -> Vec<String> {
    let last = rows.keys().next_back().copied().unwrap_or(0);
    let mut faults: Vec<String> = (1..=last)
        .filter(|n| !rows.contains_key(n))
        .map(|n| format!("#{n} is missing: rows must run 1..{last} with no gaps"))
        .collect();
    faults.extend(
        rows.iter()
            .filter(|(_, lines)| lines.len() > 1)
            .map(|(n, lines)| format!("#{n} is on {} rows", lines.len())),
    );
    faults
}

fn citations(text: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let bytes = line.as_bytes();
        for (at, _) in line.match_indices('#') {
            let digits: String = line[at + 1..].chars().take_while(char::is_ascii_digit).collect();
            let after = bytes.get(at + 1 + digits.len()).copied();
            let before = at.checked_sub(1).map(|i| bytes[i]);
            if digits.is_empty()
                || digits.len() > 4
                || after.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                || before.is_some_and(|b| b.is_ascii_alphanumeric() || b"&/_".contains(&b))
            {
                continue;
            }
            let word = line[..at].split_whitespace().next_back().unwrap_or("");
            let word = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase();
            if FOREIGN.contains(&word.as_str()) {
                continue;
            }
            out.push((index + 1, digits.parse().expect("digits")));
        }
    }
    out
}

fn dangling(root: &Path, last: u32) -> Vec<String> {
    let mut files = Vec::new();
    for dir in ["crates", "client", "docs", ".claude/skills"] {
        files.extend(repo::files(&root.join(dir), &["rs", "kt", "kts", "md"]));
    }
    files.extend(["AGENTS.md", "README.md", "ARCHITECTURE.md"].map(|f| root.join(f)));
    files
        .iter()
        .flat_map(|file| {
            citations(&repo::read(file))
                .into_iter()
                .filter(|(_, n)| *n > last)
                .map(move |(line, n)| format!("{}:{line} cites #{n}", repo::relative(file)))
        })
        .collect()
}

fn rewritten(before: &BTreeMap<u32, Vec<String>>, now: &BTreeMap<u32, Vec<String>>) -> Vec<String> {
    before
        .iter()
        .filter_map(|(n, old)| match now.get(n) {
            None => Some(format!("#{n} was removed")),
            Some(new) if new == old => None,
            Some(new) if new.iter().all(|row| row.contains("~~")) => None,
            Some(_) => Some(format!("#{n} was edited without being struck through")),
        })
        .collect()
}

#[test]
fn the_probe_log_is_numbered_one_to_n_with_no_gaps_or_duplicates() {
    let faults = numbering_faults(&rows(&repo::read(&repo::root().join(LOG))));
    assert!(faults.is_empty(), "{LOG}:\n{}", faults.join("\n"));
}

#[test]
fn every_probe_citation_points_at_a_row_that_exists() {
    let root = repo::root();
    let last = *rows(&repo::read(&root.join(LOG)))
        .keys()
        .next_back()
        .expect("rows");
    let faults = dangling(&root, last);
    assert!(
        faults.is_empty(),
        "the log ends at #{last}:\n{}",
        faults.join("\n")
    );
}

/// Read-only against the remote's main. A checkout with no such ref — a shallow CI clone of a
/// pull request — has nothing to compare against, and says so rather than passing silently.
#[test]
fn rows_already_on_main_are_never_rewritten() {
    let root = repo::root();
    let Ok(shown) = std::process::Command::new("git")
        .current_dir(&root)
        .args(["show", &format!("origin/main:{LOG}")])
        .output()
    else {
        eprintln!("skipped: no git");
        return;
    };
    if !shown.status.success() {
        eprintln!("skipped: no origin/main to compare against");
        return;
    }
    let before = rows(&String::from_utf8_lossy(&shown.stdout));
    let faults = rewritten(&before, &rows(&repo::read(&root.join(LOG))));
    assert!(
        faults.is_empty(),
        "rule 1 — append, never renumber:\n{}",
        faults.join("\n")
    );
}

#[test]
fn the_checks_see_a_gap_a_duplicate_a_dangling_citation_and_a_rewrite() {
    let log = "| # | Claim |\n|---|---|\n| 1 | a |\n| ~~2~~ | b |\n| 2 | c |\n| 4 | d |\n";
    let faults = numbering_faults(&rows(log));
    assert!(faults.iter().any(|f| f.contains("#3 is missing")), "{faults:?}");
    assert!(faults.iter().any(|f| f.contains("#2 is on 2 rows")), "{faults:?}");

    let cited = citations("see #12 and #3, not r#\"x\"#, &#123;, #[derive], #FFF or (changelog #3519)");
    assert_eq!(cited, vec![(1, 12), (1, 3)]);

    let before = rows("| 1 | a |\n| 2 | b |\n| 3 | c |\n");
    let after = rows("| 1 | a |\n| ~~2~~ | ~~b~~ see #3 |\n| 3 | changed |\n");
    assert_eq!(
        rewritten(&before, &after),
        vec!["#3 was edited without being struck through"]
    );
}
