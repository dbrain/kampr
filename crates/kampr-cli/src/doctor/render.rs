use super::{Report, Status};

const LABEL: usize = 12;
const WIDTH: usize = 92;

pub fn print(report: &Report) {
    match (&report.node, &report.node_id) {
        (Some(name), Some(id)) => println!("kampr doctor — {name} ({id})   {}", report.build),
        _ => println!("kampr doctor   {}", report.build),
    }
    println!();
    for check in &report.checks {
        let head = format!("  {:<6}{:<LABEL$}", mark(check.status), check.id);
        for (n, line) in wrap(&check.detail, WIDTH - head.len()).into_iter().enumerate() {
            match n {
                0 => println!("{head}{line}"),
                _ => println!("{:width$}{line}", "", width = head.len()),
            }
        }
        if let Some(fix) = &check.fix {
            println!("{:width$}→ {fix}", "", width = head.len());
        }
    }
    println!();
    println!("{}", summary(report));
}

fn summary(report: &Report) -> String {
    let (failed, warned) = report.counts();
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    match (failed, warned) {
        (0, 0) => "Everything checks out.".to_string(),
        (0, w) => format!("Nothing is broken; {} worth reading.", plural(w, "warning")),
        (f, 0) => format!("{} to fix.", plural(f, "problem")),
        (f, w) => format!("{} to fix, {}.", plural(f, "problem"), plural(w, "warning")),
    }
}

fn mark(status: Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Warn => "warn",
        Status::Fail => "FAIL",
    }
}

/// Hard-wrapped rather than left to the terminal: a check that wraps at the edge of an 80-column
/// pane loses the alignment that makes the column scannable in the first place.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::super::Check;
    use super::*;

    #[test]
    fn wrapping_never_loses_a_word_and_never_exceeds_the_width() {
        let text = "trust_proxy is on while the node itself answers on 0.0.0.0:8790 — anyone who \
                    can reach it directly can forge X-Forwarded-For";
        let lines = wrap(text, 40);
        assert!(
            lines.iter().all(|l| l.len() <= 40 || !l.contains(' ')),
            "{lines:?}"
        );
        assert_eq!(
            lines.join(" ").split_whitespace().count(),
            text.split_whitespace().count()
        );
    }

    #[test]
    fn the_summary_counts_what_it_says_it_counts() {
        let report = |checks| Report {
            ok: true,
            node: None,
            node_id: None,
            build: "test",
            checks,
        };
        assert_eq!(
            summary(&report(vec![Check::ok("a", "x")])),
            "Everything checks out."
        );
        assert_eq!(
            summary(&report(vec![Check::fail("a", "x"), Check::warn("b", "y")])),
            "1 problem to fix, 1 warning."
        );
        assert_eq!(
            summary(&report(vec![Check::fail("a", "x"), Check::fail("b", "y")])),
            "2 problems to fix."
        );
    }
}
