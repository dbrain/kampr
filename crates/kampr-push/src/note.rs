use serde::Serialize;

/// One agent pane that has just gone `blocked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Blocked {
    /// The global pane id, which is also the deep link.
    pub pane: String,
    pub node: String,
    pub agent: Option<String>,
    /// Workspace or pane label — what a human calls this agent.
    pub label: Option<String>,
    /// **The question itself.** Collie ships "which agent needs you" and makes you open the app to
    /// find out what it wants; the node already extracts this for the `pending` message, so
    /// withholding it here would be a choice rather than a limitation. It also earns its keep on
    /// Android, where the OS may hold the app long enough that a tap arrives before the tunnel is
    /// up — a body that says something useful is all there is until then.
    pub question: Option<String>,
}

impl Blocked {
    /// The agent's name for a human: its harness, then its label, then the raw id.
    fn who(&self) -> String {
        match (&self.agent, &self.label) {
            (Some(agent), Some(label)) => format!("{agent} · {label}"),
            (Some(agent), None) => agent.clone(),
            (None, Some(label)) => label.clone(),
            (None, None) => self.pane.clone(),
        }
    }
}

/// The payload a service worker receives. Versioned because a service worker outlives the page
/// that registered it and may be older than the node sending to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub v: u32,
    pub title: String,
    pub body: String,
    /// One tag for the whole feature, so a second notification **replaces** the first rather than
    /// stacking. The batch already carries every outstanding pane, so a stack would be the same
    /// information three times.
    pub tag: String,
    pub count: usize,
    /// Where a tap goes. The single-pane case opens that pane; a batch opens the triage list.
    pub pane: Option<String>,
    pub panes: Vec<Blocked>,
}

pub const TAG: &str = "kampr.blocked";
pub const VERSION: u32 = 1;

/// A body is a notification's whole content on a locked phone, and a question cut mid-word is
/// worse than a short one. Push services also cap the encrypted payload at 4096 bytes, which this
/// stays far inside.
const MAX_BODY: usize = 160;

impl Notification {
    /// **One notification for however many panes blocked together.** Three agents finishing a
    /// batch of edits at once is one event to a human, and three notifications racing each other
    /// is how a phone gets muted.
    pub fn batch(panes: Vec<Blocked>) -> Option<Self> {
        let first = panes.first()?;
        let (title, body) = match panes.len() {
            1 => (
                format!("{} needs you", first.who()),
                first
                    .question
                    .clone()
                    .map_or_else(|| "Waiting for an answer".to_string(), |q| trim(&q)),
            ),
            n => (
                format!("{n} agents need you"),
                names(&panes)
                    .into_iter()
                    .zip(&panes)
                    .map(|(who, p)| match &p.question {
                        Some(q) => format!("{who} — {}", trim(q)),
                        None => who,
                    })
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
        };
        Some(Self {
            v: VERSION,
            title,
            body: trim(&body),
            tag: TAG.to_string(),
            count: panes.len(),
            pane: (panes.len() == 1).then(|| first.pane.clone()),
            panes,
        })
    }
}

/// Two agents of the same harness in the same workspace render the same name, and a body that
/// says "claude · kampr — … · claude · kampr — …" names neither. Only the ambiguous ones get the
/// pane id appended; the rest stay readable.
fn names(panes: &[Blocked]) -> Vec<String> {
    let plain: Vec<String> = panes.iter().map(Blocked::who).collect();
    plain
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ambiguous = plain.iter().enumerate().any(|(j, other)| j != i && other == name);
            match ambiguous {
                true => format!("{name} ({})", short(&panes[i].pane)),
                false => name.clone(),
            }
        })
        .collect()
}

/// The herdr-local half of a global pane id — `w3:p2`, not the node ULID in front of it.
fn short(pane: &str) -> &str {
    pane.rsplit('/').next().unwrap_or(pane)
}

/// Cuts at a word boundary, and only when there is one to cut at — a 200-character path with no
/// spaces is better truncated hard than left whole.
fn trim(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= MAX_BODY {
        return text;
    }
    let head: String = text.chars().take(MAX_BODY).collect();
    let cut = head
        .rfind(' ')
        .filter(|at| *at > MAX_BODY / 2)
        .unwrap_or(head.len());
    format!("{}…", head[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(pane: &str, agent: &str, question: Option<&str>) -> Blocked {
        Blocked {
            pane: format!("01J/{pane}"),
            node: "01J".into(),
            agent: Some(agent.into()),
            label: Some("kampr".into()),
            question: question.map(str::to_string),
        }
    }

    /// The thing Collie's own architecture doc calls its known gap. The node already has the
    /// question — it publishes it as `pending` — so a notification that only says which agent is
    /// blocked is withholding what it has.
    #[test]
    fn one_blocked_agent_carries_its_question_in_the_body() {
        let note = Notification::batch(vec![blocked(
            "w3:p2",
            "claude",
            Some("Do you want to make this edit?"),
        )])
        .unwrap();
        assert_eq!(note.title, "claude · kampr needs you");
        assert_eq!(note.body, "Do you want to make this edit?");
        assert_eq!(note.pane.as_deref(), Some("01J/w3:p2"));
        assert_eq!(note.count, 1);
    }

    #[test]
    fn an_agent_with_no_question_still_says_something_useful() {
        let note = Notification::batch(vec![blocked("w3:p2", "claude", None)]).unwrap();
        assert_eq!(note.body, "Waiting for an answer");
    }

    /// Three agents blocking together is one event to a human. Racing three notifications at them
    /// is how the whole feature gets turned off.
    #[test]
    fn simultaneous_blocks_are_one_notification_naming_every_pane() {
        let note = Notification::batch(vec![
            blocked("w1:p1", "claude", Some("Run the tests?")),
            blocked("w2:p1", "codex", Some("Apply the patch?")),
            blocked("w3:p1", "claude", None),
        ])
        .unwrap();
        assert_eq!(note.count, 3);
        assert_eq!(note.title, "3 agents need you");
        assert!(note.body.contains("Run the tests?"), "{}", note.body);
        assert!(note.body.contains("Apply the patch?"), "{}", note.body);
        assert_eq!(note.panes.len(), 3);
        assert_eq!(
            note.pane, None,
            "a batch opens the triage list, not one of its panes"
        );
    }

    /// The tag is what makes the second notification replace the first. Without it a phone that
    /// was away for an hour shows a column of stale prompts.
    /// Two claudes in one workspace are two different agents, and a body naming both the same way
    /// tells you nothing about which is which.
    #[test]
    fn identical_names_in_one_batch_are_disambiguated_by_pane() {
        let note = Notification::batch(vec![
            blocked("w1:p1", "claude", Some("Proceed?")),
            blocked("w1:p3", "claude", Some("Proceed?")),
            blocked("w2:p1", "codex", Some("Patch?")),
        ])
        .unwrap();
        assert!(note.body.contains("(w1:p1)"), "{}", note.body);
        assert!(note.body.contains("(w1:p3)"), "{}", note.body);
        assert!(
            !note.body.contains("codex · kampr ("),
            "an unambiguous name stays readable: {}",
            note.body
        );
    }

    #[test]
    fn every_notification_shares_one_tag() {
        let a = Notification::batch(vec![blocked("w1:p1", "claude", None)]).unwrap();
        let b = Notification::batch(vec![blocked("w2:p1", "codex", None)]).unwrap();
        assert_eq!(a.tag, b.tag);
    }

    #[test]
    fn nothing_blocked_is_no_notification_rather_than_an_empty_one() {
        assert!(Notification::batch(Vec::new()).is_none());
    }

    #[test]
    fn a_long_question_is_cut_at_a_word_and_a_long_unbroken_one_is_cut_anyway() {
        let wordy = "word ".repeat(80);
        let note = Notification::batch(vec![blocked("w1:p1", "claude", Some(&wordy))]).unwrap();
        assert!(note.body.chars().count() <= MAX_BODY + 1, "{}", note.body);
        assert!(note.body.ends_with('…'));
        assert!(!note.body.contains("  "), "newlines and runs collapse");

        let unbroken = "x".repeat(400);
        let hard = Notification::batch(vec![blocked("w1:p1", "claude", Some(&unbroken))]).unwrap();
        assert!(hard.body.chars().count() <= MAX_BODY + 1);
    }

    #[test]
    fn a_shell_pane_with_no_harness_is_still_nameable() {
        let note = Notification::batch(vec![Blocked {
            pane: "01J/w1:p1".into(),
            node: "01J".into(),
            agent: None,
            label: None,
            question: None,
        }])
        .unwrap();
        assert_eq!(note.title, "01J/w1:p1 needs you");
    }
}
