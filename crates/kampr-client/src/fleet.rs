//! Turning one instruction into one op per host.
//!
//! The fan-out lives here rather than in a client's key handler because it is the same arithmetic
//! wherever it is asked for — a terminal, a phone, a script — and because getting it wrong means
//! running a command on a machine nobody meant to include.

use crate::herd::Herd;
use kampr_core::wire::PaneEntry;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FanOutError {
    #[error("there is nothing to run")]
    Empty,
    #[error("{0}")]
    Unbalanced(String),
    #[error("no node in this herd can be reached, so there is nowhere to run it")]
    NowhereToRun,
}

/// One `fleet.run` per online node, all carrying the same cohort.
///
/// **Reachable, not `online`.** `online` is the node's *herdr* health, and a fleet run needs no
/// herdr at all — a machine whose herdr is down still runs commands perfectly well, and skipping it
/// meant refusing to reach a host that was sitting right there. What is skipped is a node nothing
/// can be asked of: a peer being served from memory with its link down. Those are not queued
/// either, because a command that runs when a host comes back hours later is a command nobody is
/// watching.
pub fn fan_out(command: &str, herd: &Herd) -> Result<Vec<Value>, FanOutError> {
    let line = command.trim();
    if line.is_empty() {
        return Err(FanOutError::Empty);
    }
    balanced(line)?;
    let cohort = ulid::Ulid::generate().to_string();
    let ops: Vec<Value> = herd
        .nodes
        .iter()
        .filter(|node| node.is_reachable())
        .map(|node| {
            json!({
                "op": "fleet.run",
                "node": node.id,
                "cohort": cohort,
                "command": line,
            })
        })
        .collect();
    if ops.is_empty() {
        return Err(FanOutError::NowhereToRun);
    }
    Ok(ops)
}

/// The one thing checked before a line reaches five machines: that its quotes close.
///
/// **The shell does the rest, deliberately.** `&&`, `|`, `;`, globs, `~` and redirection all mean
/// what they mean in the operator's own terminal, because that is what they asked for and because
/// re-implementing a shell's word splitting in two languages to *avoid* a shell was the more
/// dangerous of the two options. What is worth catching here is the typo, because an unclosed
/// quote is a mistake every host would report identically and a run nobody meant to start.
///
/// A backslash escapes the next character outside single quotes, so `echo \"` is balanced and
/// `echo "a \" b"` is too. Inside `'…'` nothing escapes, which is the shell's rule.
///
/// Kept in step with `dev.kampr.shared.model.balanced`.
fn balanced(command: &str) -> Result<(), FanOutError> {
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (Some(q), '\\') => {
                chars.next();
                let _ = q;
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\\') => {
                chars.next();
            }
            (None, '\'') | (None, '"') => quote = Some(c),
            (None, _) => {}
        }
    }
    match quote {
        Some(_) => Err(FanOutError::Unbalanced(
            "that command has a quote that never closes".into(),
        )),
        None => Ok(()),
    }
}

/// Which other hosts in this run are asking **exactly** the same thing.
#[derive(Debug, Clone)]
pub struct Matching<'a> {
    pub target: &'a PaneEntry,
    /// Every other waiting pane in the cohort whose question is identical.
    pub others: Vec<&'a PaneEntry>,
    /// Waiting panes in the cohort that are asking something else, named so the operator can see
    /// what is *not* being answered. The silent third of a fleet is what bites you.
    pub differing: Vec<&'a PaneEntry>,
}

impl Matching<'_> {
    /// How many hosts one answer would reach.
    pub fn reach(&self) -> usize {
        1 + self.others.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    #[error("that pane is not a fleet run")]
    NotAFleetRun,
    #[error("that host is not waiting for anything")]
    NotWaiting,
    #[error("a password is answered one host at a time")]
    Secret,
}

/// The other hosts one answer would reach, or why it would reach none.
///
/// The terminal client renders the count; the Compose board is what actually sends to all of them,
/// so the recipients are derived there rather than a second time here.
///
/// **Byte-identical, not merely similar.** The prompt, the shape, the options and their order all
/// have to match, because "these two look alike" is exactly the reasoning that sends `y` to the
/// host that was asking something else.
pub fn matching<'a>(herd: &'a Herd, pane_id: &str) -> Result<Matching<'a>, AnswerError> {
    let target = herd.pane(pane_id).ok_or(AnswerError::NotAFleetRun)?;
    let fleet = target.fleet.as_ref().ok_or(AnswerError::NotAFleetRun)?;
    let question = fleet.question.as_ref().ok_or(AnswerError::NotWaiting)?;
    if question.secret() {
        // A password sent to five hosts because five prompts said "Password:" is a password given
        // to whichever of them was asking for something else. The prompt text is no evidence at
        // all here — every one of them says the same word.
        return Err(AnswerError::Secret);
    }

    let mut others = Vec::new();
    let mut differing = Vec::new();
    for pane in herd.panes.iter() {
        if pane.id == target.id {
            continue;
        }
        let Some(other) = pane.fleet.as_ref() else {
            continue;
        };
        if other.cohort != fleet.cohort {
            continue;
        }
        match other.question.as_ref() {
            Some(q) if q == question && other.command == fleet.command => others.push(pane),
            Some(_) => differing.push(pane),
            None => {}
        }
    }
    Ok(Matching {
        target,
        others,
        differing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::wire::NodeEntry;

    fn herd(nodes: &[(&str, bool)]) -> Herd {
        reachability(
            &nodes
                .iter()
                .map(|(id, up)| (*id, *up, Some(*up)))
                .collect::<Vec<_>>(),
        )
    }

    fn reachability(nodes: &[(&str, bool, Option<bool>)]) -> Herd {
        let mut h = Herd::default();
        h.apply(
            nodes
                .iter()
                .map(|(id, online, reachable)| {
                    let mut value = json!({
                        "id": id, "name": id, "kind": "peer", "online": online
                    });
                    if let Some(r) = reachable {
                        value["reachable"] = json!(r);
                    }
                    serde_json::from_value::<NodeEntry>(value).expect("a node")
                })
                .collect(),
            Vec::new(),
        );
        h
    }

    #[test]
    fn one_command_becomes_one_op_per_online_node_sharing_a_cohort() {
        let ops = fan_out("pacman -Syu", &herd(&[("a", true), ("b", true)])).expect("ops");
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0]["command"], json!("pacman -Syu"));
        assert_eq!(ops[0]["cohort"], ops[1]["cohort"], "one run, one cohort");
        assert_ne!(ops[0]["node"], ops[1]["node"]);
    }

    #[test]
    fn a_host_whose_herdr_is_down_is_still_run_on() {
        // The correction: `online` is herdr's health, and a fleet run does not need herdr. A
        // machine sitting right there was being skipped for a reason that had nothing to do with
        // whether it could run the command.
        let ops = fan_out("uptime", &reachability(&[("a", false, Some(true))])).expect("ops");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["node"], "a");
    }

    #[test]
    fn an_older_node_that_does_not_say_falls_back_to_online() {
        // Additive: a node from before the field behaves exactly as it did.
        assert_eq!(
            fan_out("uptime", &reachability(&[("a", true, None)]))
                .expect("ops")
                .len(),
            1
        );
        assert_eq!(
            fan_out("uptime", &reachability(&[("a", false, None)])),
            Err(FanOutError::NowhereToRun)
        );
    }

    #[test]
    fn an_unreachable_node_is_skipped_rather_than_queued() {
        let ops = fan_out("uptime", &herd(&[("a", true), ("b", false)])).expect("ops");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["node"], "a");
    }

    #[test]
    fn a_herd_with_nobody_home_is_an_error_and_not_an_empty_success() {
        assert_eq!(
            fan_out("uptime", &reachability(&[("a", false, Some(false))])),
            Err(FanOutError::NowhereToRun)
        );
    }

    #[test]
    fn nothing_to_run_is_refused() {
        assert_eq!(fan_out("   ", &herd(&[("a", true)])), Err(FanOutError::Empty));
    }

    /// The operator's line reaches the node as they typed it, and nothing here splits it. That is
    /// the whole change: a pipeline is a pipeline.
    #[test]
    fn a_pipeline_reaches_the_node_as_one_line_rather_than_as_words() {
        let ops = fan_out(r#"find . -name "*.rs" | wc -l"#, &herd(&[("a", true)])).expect("ops");
        assert_eq!(ops[0]["command"], json!(r#"find . -name "*.rs" | wc -l"#));
        assert!(
            ops[0].get("args").is_none(),
            "an argv would be a second answer to disagree with"
        );
    }

    #[test]
    fn an_unclosed_quote_is_refused_rather_than_fanned_out() {
        assert!(matches!(
            fan_out(r#"echo "oops"#, &herd(&[("a", true)])),
            Err(FanOutError::Unbalanced(_))
        ));
    }

    /// A backslash escapes a quote, and a run that would have worked in their own terminal must
    /// not be refused by a checker that is cruder than the shell it stands in front of.
    #[test]
    fn an_escaped_quote_is_not_an_unclosed_one() {
        assert!(balanced(r#"echo \""#).is_ok());
        assert!(balanced(r#"echo "a \" b""#).is_ok());
        assert!(balanced(r#"echo "don't""#).is_ok());
        assert!(balanced(r#"grep 'a "b" c'"#).is_ok());
        // Inside single quotes a backslash is a literal backslash, which is the shell's rule.
        assert!(balanced(r#"echo 'a\'"#).is_ok());
    }
}
