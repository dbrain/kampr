use std::borrow::Cow;

use crate::model::{Block, Role, Turn};

// Text a harness injects into a *user* record that nobody typed. Measured across every transcript
// on this machine (#286): claude opens a user record with `<task-notification>` 214 times against
// 259 real prompts, and codex opens one with `<environment_context>` 16 times. It reaches the
// reader as "You: <task-notification>…", and it is worse than noise — the client splits an
// exchange on every user turn, so a background agent finishing mid-answer cuts the reply it
// interrupted in half.
//
// The siblings are here because they arrive in the same envelope group as the openers measured:
// a slash command writes `<command-name>`, `<command-message>` and `<command-args>` as one block.
const ENVELOPES: &[(&str, &str)] = &[
    ("<task-notification>", "</task-notification>"),
    ("<command-name>", "</command-name>"),
    ("<command-message>", "</command-message>"),
    ("<command-args>", "</command-args>"),
    ("<local-command-stdout>", "</local-command-stdout>"),
    ("<local-command-stderr>", "</local-command-stderr>"),
    ("<local-command-caveat>", "</local-command-caveat>"),
    ("<system-reminder>", "</system-reminder>"),
    ("<environment_context>", "</environment_context>"),
    ("<user_instructions>", "</user_instructions>"),
    ("<recommended_plugins>", "</recommended_plugins>"),
];

/// What is left of a user record once the harness's own envelopes are taken out of it. Prose with
/// a reminder stapled to the end keeps the prose; a record that was nothing but envelope comes
/// back empty, and an empty block is one the caller does not push.
///
/// An envelope with no closing tag runs to the end of the text: a truncated one is still not
/// something a person said.
pub fn spoken(text: &str) -> Cow<'_, str> {
    let mut kept: Option<String> = None;
    let mut rest = text;
    while let Some((at, open, close)) = ENVELOPES
        .iter()
        .filter_map(|(open, close)| rest.find(open).map(|at| (at, *open, *close)))
        .min_by_key(|(at, ..)| *at)
    {
        let buf = kept.get_or_insert_with(String::new);
        buf.push_str(&rest[..at]);
        let after = &rest[at + open.len()..];
        rest = match after.find(close) {
            Some(end) => &after[end + close.len()..],
            None => "",
        };
    }
    match kept {
        None => Cow::Borrowed(text),
        Some(mut buf) => {
            buf.push_str(rest);
            Cow::Owned(buf.trim().to_string())
        }
    }
}

/// The one way a text block reaches a turn. A user record is the only place a harness envelope
/// arrives (#286) — an assistant that writes one is quoting it — and a user record with nothing
/// but envelope in it contributes no block, which leaves the turn empty and the caller drops it.
pub fn push_text(turn: &mut Turn, text: String) {
    if turn.role != Role::User {
        turn.blocks.push(Block::md(text));
        return;
    }
    let said = spoken(&text);
    if !said.trim().is_empty() {
        turn.blocks.push(Block::md(said.into_owned()));
    }
}

/// What is left of a command result once the harness's own header is taken off it.
///
/// Both shell harnesses write the same shape: a run of bookkeeping lines — the chunk id, the wall
/// time, the exit status — and then a lone `Output:` opening the bytes the command actually
/// produced. The status is already on the card's own `state`, and counting the header would put
/// four lines on a card over a one-line command, which is the defect the result block exists to
/// end. Text that opens with none of `heads` is not a header and is handed back whole.
pub fn after_header<'a>(text: &'a str, heads: &[&str]) -> &'a str {
    let mut rest = text;
    let mut taken = 0;
    loop {
        let (line, after) = match rest.find('\n') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, ""),
        };
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line == "Output:" {
            return if taken == 0 { text } else { after };
        }
        if !heads.iter().any(|head| line.starts_with(head)) {
            return if taken == 0 { text } else { rest };
        }
        rest = after;
        taken += 1;
        if rest.is_empty() {
            return "";
        }
    }
}

#[cfg(test)]
mod tests {
    use super::spoken;

    #[test]
    fn prose_is_handed_back_untouched_and_unallocated() {
        let said = "have another look at the width inference";
        assert!(matches!(spoken(said), std::borrow::Cow::Borrowed(_)));
        assert_eq!(spoken(said), said);
    }

    #[test]
    fn a_record_that_is_nothing_but_envelope_says_nothing() {
        let notification = "<task-notification>\n<task-id>b88dc5bqu</task-id>\n\
             <summary>Agent \"Claude code review\" finished</summary>\n</task-notification>";
        assert_eq!(spoken(notification), "");
        assert_eq!(
            spoken(
                "<command-name>/compact</command-name>\n<command-message>compact</command-message>\n\
                 <command-args></command-args>"
            ),
            ""
        );
        assert_eq!(
            spoken("<environment_context>\n<cwd>/home/u</cwd>\n</environment_context>"),
            ""
        );
    }

    #[test]
    fn what_the_operator_typed_survives_the_envelope_stapled_to_it() {
        assert_eq!(
            spoken("bump the nodes<system-reminder>the date has changed</system-reminder>"),
            "bump the nodes"
        );
        assert_eq!(
            spoken("<local-command-caveat>generated locally</local-command-caveat>\nfix CI"),
            "fix CI"
        );
    }

    // The one that is not a hypothetical: a notification arriving mid-session is truncated by the
    // tail reader at whatever byte the writer had reached.
    #[test]
    fn an_envelope_with_no_end_runs_to_the_end_of_the_record() {
        assert_eq!(spoken("<task-notification>\n<task-id>b88"), "");
    }
}
