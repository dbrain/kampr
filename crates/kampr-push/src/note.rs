use serde::Serialize;

/// What a notification is about.
///
/// **Two kinds, two tags, two independent sets.** They are not folded into one notification
/// because they differ in the two things a notification is made of: urgency — a blocked agent is
/// a question waiting on a person, a finished one is news — and *what replaces what*. One tag
/// means the newest payload is the only thing on the screen, so a finished agent sharing the
/// blocked tag would take a live question off the phone. A second tag is a second slot, and each
/// slot carries the whole of its own set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Blocked,
    Done,
}

/// One tag per kind, so a second notification of the same kind **replaces** the first rather than
/// stacking — the batch already carries every pane of that kind, so a stack would be the same
/// information three times — while the other kind's slot is left alone.
pub const TAG_BLOCKED: &str = "kampr.blocked";
pub const TAG_DONE: &str = "kampr.done";

impl Kind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Blocked => TAG_BLOCKED,
            Self::Done => TAG_DONE,
        }
    }

    /// The payload version a client must have declared before this kind may be sent to it.
    ///
    /// `Blocked` asks for nothing, because every client that ever subscribed was built for it.
    /// `Done` asks for [`VERSION`], because a client with one notification slot renders it as the
    /// blocked one — see the note on `VERSION`.
    pub fn min_payload_version(self) -> i64 {
        match self {
            Self::Blocked => 1,
            Self::Done => VERSION as i64,
        }
    }
}

/// One agent pane a notification is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Agent {
    /// The global pane id, which is also the deep link.
    pub pane: String,
    pub node: String,
    pub agent: Option<String>,
    /// Workspace or pane label — what a human calls this agent.
    pub label: Option<String>,
    /// The one line under the name, and what it says depends on the kind.
    ///
    /// **Blocked: the question itself.** Collie ships "which agent needs you" and makes you open
    /// the app to find out what it wants; the node already extracts this for the `pending`
    /// message, so withholding it here would be a choice rather than a limitation. It also earns
    /// its keep on Android, where the OS may hold the app long enough that a tap arrives before
    /// the tunnel is up — a body that says something useful is all there is until then.
    ///
    /// **Done: where it ran.** Deliberately *not* the agent's closing message: resolving that
    /// means locating and parsing the transcript, measured at 1.99 s on a 30.7 MB one (#409), and
    /// this runs inside a 900 ms collection window for every pane that finished at once. A
    /// working directory is already in the herd model, tells three simultaneous agents apart, and
    /// is not a guess.
    pub detail: Option<String>,
}

impl Agent {
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
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// The tag for this notification's kind. A client keys its own slot on this rather than on
    /// `kind`, which is why a worker that has never heard of `done` still keeps the two apart.
    pub tag: String,
    pub count: usize,
    /// Where a tap goes. The single-pane case opens that pane; a batch opens the triage list.
    pub pane: Option<String>,
    pub panes: Vec<Agent>,
    /// Whether this is news. **A notification is a summary of now, not an edge**, so one is also
    /// sent when the outstanding set *shrinks* — and buzzing a phone to tell it there is less
    /// waiting is how the feature gets turned off. A client that does not know this field treats
    /// the payload as an ordinary notification, which is the old behaviour and is not wrong.
    pub alert: bool,
}

/// v2 added [`Notification::alert`] and `count: 0`. v3 added [`Kind`] and the second tag.
///
/// **v3 is why a subscription records the payload version it can read.** A client older than it
/// posts every payload into its one blocked slot whatever the tag says, so a `done` sent to one
/// would overwrite a live question — the one degradation this feature may not cause. The gate is
/// in [`crate::Store::push_targets`](../../kampr_auth/struct.Store.html), not here: an old client
/// is never sent a `done` at all, rather than sent one it renders wrongly.
pub const VERSION: u32 = 3;

/// A body is a notification's whole content on a locked phone, and a question cut mid-word is
/// worse than a short one. Push services also cap the encrypted payload at 4096 bytes, which this
/// stays far inside.
const MAX_BODY: usize = 160;

impl Notification {
    /// **One notification for however many panes changed together.** Three agents finishing a
    /// batch of edits at once is one event to a human, and three notifications racing each other
    /// is how a phone gets muted.
    pub fn batch(kind: Kind, panes: Vec<Agent>) -> Option<Self> {
        Self::build(kind, panes, true)
    }

    /// The same summary, without the buzz: what a device is sent when a pane it was told about
    /// left this kind's set. It names everything *still* outstanding for that device, so the one
    /// that left the shade goes and the rest stay named.
    ///
    /// Nothing outstanding is a notification too — the one that says so and carries `count: 0`,
    /// which is how a client is told to take the prompt down. `batch` returns `None` for an empty
    /// set because an empty *alert* is nothing; an empty *resync* is the whole point.
    pub fn resync(kind: Kind, panes: Vec<Agent>) -> Self {
        Self::build(kind, panes, false).unwrap_or_else(|| Self::clear(kind))
    }

    fn clear(kind: Kind) -> Self {
        let (title, body) = match kind {
            Kind::Blocked => ("Answered elsewhere", "Nothing is waiting on you now"),
            Kind::Done => ("Caught up", "Every finished agent has been seen"),
        };
        Self {
            v: VERSION,
            kind,
            title: title.to_string(),
            body: body.to_string(),
            tag: kind.tag().to_string(),
            count: 0,
            pane: None,
            panes: Vec::new(),
            alert: false,
        }
    }

    fn build(kind: Kind, panes: Vec<Agent>, alert: bool) -> Option<Self> {
        let first = panes.first()?;
        let (title, body) = match (kind, panes.len()) {
            (Kind::Blocked, 1) => (
                format!("{} needs you", first.who()),
                first
                    .detail
                    .clone()
                    .unwrap_or_else(|| "Waiting for an answer".to_string()),
            ),
            (Kind::Blocked, n) => (format!("{n} agents need you"), lines(&panes)),
            (Kind::Done, 1) => (
                format!("{} finished", first.who()),
                first
                    .detail
                    .clone()
                    .unwrap_or_else(|| "It finished while you were away".to_string()),
            ),
            (Kind::Done, n) => (format!("{n} agents finished"), lines(&panes)),
        };
        Some(Self {
            v: VERSION,
            kind,
            title,
            body: trim(&body),
            tag: kind.tag().to_string(),
            count: panes.len(),
            pane: (panes.len() == 1).then(|| first.pane.clone()),
            panes,
            alert,
        })
    }
}

fn lines(panes: &[Agent]) -> String {
    names(panes)
        .into_iter()
        .zip(panes)
        .map(|(who, p)| match &p.detail {
            Some(detail) => format!("{who} — {}", trim(detail)),
            None => who,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Two agents of the same harness in the same workspace render the same name, and a body that
/// says "claude · kampr — … · claude · kampr — …" names neither. Only the ambiguous ones get the
/// pane id appended; the rest stay readable.
fn names(panes: &[Agent]) -> Vec<String> {
    let plain: Vec<String> = panes.iter().map(Agent::who).collect();
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

    fn agent(pane: &str, harness: &str, detail: Option<&str>) -> Agent {
        Agent {
            pane: format!("01J/{pane}"),
            node: "01J".into(),
            agent: Some(harness.into()),
            label: Some("kampr".into()),
            detail: detail.map(str::to_string),
        }
    }

    /// The thing Collie's own architecture doc calls its known gap. The node already has the
    /// question — it publishes it as `pending` — so a notification that only says which agent is
    /// blocked is withholding what it has.
    #[test]
    fn one_blocked_agent_carries_its_question_in_the_body() {
        let note = Notification::batch(
            Kind::Blocked,
            vec![agent("w3:p2", "claude", Some("Do you want to make this edit?"))],
        )
        .unwrap();
        assert_eq!(note.title, "claude · kampr needs you");
        assert_eq!(note.body, "Do you want to make this edit?");
        assert_eq!(note.pane.as_deref(), Some("01J/w3:p2"));
        assert_eq!(note.count, 1);
    }

    #[test]
    fn an_agent_with_no_question_still_says_something_useful() {
        let note = Notification::batch(Kind::Blocked, vec![agent("w3:p2", "claude", None)]).unwrap();
        assert_eq!(note.body, "Waiting for an answer");
    }

    /// The other half of the feature: an agent that finished while nobody was looking is the
    /// operator's unread flag, and it says so in its own words rather than borrowing the blocked
    /// one's. Its body is where it ran, which is what tells three of them apart.
    #[test]
    fn one_finished_agent_says_it_finished_and_where() {
        let note =
            Notification::batch(Kind::Done, vec![agent("w3:p2", "claude", Some("~/dev/kampr"))]).unwrap();
        assert_eq!(note.title, "claude · kampr finished");
        assert_eq!(note.body, "~/dev/kampr");
        assert_eq!(note.pane.as_deref(), Some("01J/w3:p2"));
    }

    #[test]
    fn a_finished_agent_with_no_directory_still_says_something_useful() {
        let note = Notification::batch(Kind::Done, vec![agent("w3:p2", "claude", None)]).unwrap();
        assert_eq!(note.body, "It finished while you were away");
    }

    /// The whole reason the two kinds are not folded into one notification. One tag is one slot,
    /// so a finished agent arriving under the blocked tag would take a live question off the
    /// phone — and the question is the thing a person is actually needed for.
    #[test]
    fn a_finished_agent_never_lands_in_the_blocked_notifications_slot() {
        let blocked =
            Notification::batch(Kind::Blocked, vec![agent("w1:p1", "claude", Some("Proceed?"))]).unwrap();
        let done = Notification::batch(Kind::Done, vec![agent("w2:p1", "codex", None)]).unwrap();
        assert_ne!(
            blocked.tag, done.tag,
            "one tag is one slot: sharing it means the newer payload erases the older"
        );
        assert_eq!(blocked.tag, TAG_BLOCKED);
        assert_eq!(done.tag, TAG_DONE);
        assert_ne!(Notification::resync(Kind::Blocked, Vec::new()).tag, TAG_DONE);
        assert_ne!(Notification::resync(Kind::Done, Vec::new()).tag, TAG_BLOCKED);
    }

    /// Three agents blocking together is one event to a human. Racing three notifications at them
    /// is how the whole feature gets turned off.
    #[test]
    fn simultaneous_blocks_are_one_notification_naming_every_pane() {
        let note = Notification::batch(
            Kind::Blocked,
            vec![
                agent("w1:p1", "claude", Some("Run the tests?")),
                agent("w2:p1", "codex", Some("Apply the patch?")),
                agent("w3:p1", "claude", None),
            ],
        )
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

    #[test]
    fn simultaneous_finishes_are_one_notification_too() {
        let note = Notification::batch(
            Kind::Done,
            vec![
                agent("w1:p1", "claude", Some("~/dev/kampr")),
                agent("w2:p1", "codex", Some("~/dev/herdr")),
            ],
        )
        .unwrap();
        assert_eq!(note.title, "2 agents finished");
        assert!(note.body.contains("~/dev/kampr"), "{}", note.body);
        assert!(note.body.contains("~/dev/herdr"), "{}", note.body);
        assert_eq!(note.pane, None);
    }

    /// Two claudes in one workspace are two different agents, and a body naming both the same way
    /// tells you nothing about which is which.
    #[test]
    fn identical_names_in_one_batch_are_disambiguated_by_pane() {
        let note = Notification::batch(
            Kind::Blocked,
            vec![
                agent("w1:p1", "claude", Some("Proceed?")),
                agent("w1:p3", "claude", Some("Proceed?")),
                agent("w2:p1", "codex", Some("Patch?")),
            ],
        )
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
    fn every_notification_of_one_kind_shares_that_kinds_tag() {
        let a = Notification::batch(Kind::Blocked, vec![agent("w1:p1", "claude", None)]).unwrap();
        let b = Notification::batch(Kind::Blocked, vec![agent("w2:p1", "codex", None)]).unwrap();
        assert_eq!(a.tag, b.tag);
    }

    #[test]
    fn nothing_blocked_is_no_notification_rather_than_an_empty_one() {
        assert!(Notification::batch(Kind::Blocked, Vec::new()).is_none());
        assert!(Notification::batch(Kind::Done, Vec::new()).is_none());
    }

    /// The gap this feature closes. A prompt answered at the desk left the phone showing it until
    /// somebody tapped it, because the node only ever sent rising edges. Nothing outstanding is a
    /// payload now, and `count: 0` is what tells a client to take the prompt down.
    #[test]
    fn nothing_outstanding_is_the_notification_that_takes_the_prompt_down() {
        for kind in [Kind::Blocked, Kind::Done] {
            let clear = Notification::resync(kind, Vec::new());
            assert_eq!(clear.count, 0);
            assert!(clear.panes.is_empty());
            assert_eq!(clear.pane, None);
            assert!(!clear.alert);
            assert_eq!(clear.tag, kind.tag(), "it has to replace what it is clearing");
            assert!(
                !clear.title.is_empty() && !clear.body.is_empty(),
                "a worker older than v2 shows whatever arrives, so an empty title is a blank prompt"
            );
        }
    }

    /// Answering one of three must leave the other two named rather than clearing the lot — and
    /// must not buzz, because there is less waiting than there was.
    #[test]
    fn answering_one_of_three_resyncs_to_the_two_that_are_left_without_alerting() {
        let note = Notification::resync(
            Kind::Blocked,
            vec![
                agent("w2:p1", "codex", Some("Apply the patch?")),
                agent("w3:p1", "claude", None),
            ],
        );
        assert_eq!(note.count, 2);
        assert_eq!(note.title, "2 agents need you");
        assert!(note.body.contains("Apply the patch?"), "{}", note.body);
        assert!(!note.alert);
    }

    /// The two constructors differ in exactly one field. A resync that shaped its body differently
    /// would make the shade flicker between two renderings of the same herd.
    #[test]
    fn a_resync_and_an_alert_render_the_same_herd_identically() {
        for kind in [Kind::Blocked, Kind::Done] {
            let panes = vec![agent("w1:p1", "claude", Some("Proceed?"))];
            let alerting = Notification::batch(kind, panes.clone()).unwrap();
            let quiet = Notification::resync(kind, panes);
            assert!(alerting.alert);
            assert!(!quiet.alert);
            assert_eq!(alerting.title, quiet.title);
            assert_eq!(alerting.body, quiet.body);
            assert_eq!(alerting.pane, quiet.pane);
        }
    }

    #[test]
    fn a_long_question_is_cut_at_a_word_and_a_long_unbroken_one_is_cut_anyway() {
        let wordy = "word ".repeat(80);
        let note = Notification::batch(Kind::Blocked, vec![agent("w1:p1", "claude", Some(&wordy))]).unwrap();
        assert!(note.body.chars().count() <= MAX_BODY + 1, "{}", note.body);
        assert!(note.body.ends_with('…'));
        assert!(!note.body.contains("  "), "newlines and runs collapse");

        let unbroken = "x".repeat(400);
        let hard =
            Notification::batch(Kind::Blocked, vec![agent("w1:p1", "claude", Some(&unbroken))]).unwrap();
        assert!(hard.body.chars().count() <= MAX_BODY + 1);
    }

    #[test]
    fn a_shell_pane_with_no_harness_is_still_nameable() {
        let note = Notification::batch(
            Kind::Blocked,
            vec![Agent {
                pane: "01J/w1:p1".into(),
                node: "01J".into(),
                agent: None,
                label: None,
                detail: None,
            }],
        )
        .unwrap();
        assert_eq!(note.title, "01J/w1:p1 needs you");
    }

    /// The wire is read by clients that were written before this field existed, and `kind` is the
    /// field they have never heard of. It has to be a plain lowercase word rather than a tagged
    /// enum, because a payload is JSON a service worker reads with `note.kind`.
    #[test]
    fn the_kind_serialises_as_a_bare_word() {
        let note = Notification::batch(Kind::Done, vec![agent("w1:p1", "claude", None)]).unwrap();
        let json = serde_json::to_value(&note).unwrap();
        assert_eq!(json["kind"], "done");
        assert_eq!(json["v"], 3);
        assert_eq!(json["tag"], TAG_DONE);
    }
}
