use kampr_client::Pending;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{HookChoice, clip};
use crate::theme::Theme;

/// The strip, and where each option's key chip landed so a click can answer it.
#[derive(Default)]
pub(super) struct Strip {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) chips: Vec<(u16, u16)>,
    pub(super) submit: Option<(u16, u16)>,
    /// Which of [`Self::lines`] the chips were laid on. A row rather than a constant because the
    /// desk line stands above the prompt and moves it.
    pub(super) chip_row: u16,
}

/// A line the operator left at the pane's own keyboard. Dim rather than alarming: it is context for
/// the reply about to be appended to it, not a fault.
pub(super) fn waiting(line: &str, theme: &Theme, width: u16) -> Line<'static> {
    // At least one character of the line survives however narrow the pane is: a bare ellipsis
    // says a line is waiting and refuses to say anything about it.
    let room = (width.saturating_sub(16) as usize).max(2);
    let shown: String = match line.chars().count() > room {
        true => line.chars().take(room - 1).chain(['…']).collect(),
        false => line.to_string(),
    };
    Line::from(vec![
        Span::styled(
            " at the desk ".to_string(),
            Style::default().fg(theme.on_accent).bg(theme.working),
        ),
        Span::styled(format!(" {shown}"), Style::default().fg(theme.dim)),
    ])
}

/// One line, and each choice's chip clipped to what fits: a chip drawn past the edge is not a
/// chip anybody can click.
pub(super) fn hooking(
    state: &str,
    offer: &[HookChoice],
    theme: &Theme,
    width: u16,
) -> (Line<'static>, Vec<(u16, u16)>) {
    let said = match state {
        "stale" => " claude hooks · stale ",
        _ => " claude hooks ",
    };
    let wide = |label: &str| (" alt+x ".len() + label.chars().count() + 3) as u16;
    let room = width.saturating_sub(said.chars().count() as u16);
    let whole = offer.iter().map(|c| wide(&c.label)).sum::<u16>() <= room;
    let mut spans = vec![Span::styled(
        said.to_string(),
        Style::default().fg(theme.on_accent).bg(theme.working),
    )];
    let mut x = said.chars().count() as u16 + 1;
    spans.push(Span::styled(
        " ".to_string(),
        Style::default().bg(theme.blocked_bg),
    ));
    let mut chips = Vec::new();
    for choice in offer {
        let label = match whole {
            true => &choice.label,
            false => &choice.short,
        };
        let key = format!(" alt+{} ", choice.key);
        let label = format!(" {label}  ");
        let chip = (key.chars().count() + label.chars().count()) as u16;
        if x + chip > width {
            break;
        }
        chips.push((x, chip));
        x += chip;
        spans.push(Span::styled(
            key,
            Style::default()
                .fg(theme.on_accent)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            label,
            Style::default().fg(theme.text).bg(theme.blocked_bg),
        ));
    }
    (Line::from(spans), chips)
}

/// The context, where there is one, stands between the question and its keys; [`Strip::chip_row`]
/// carries out which line the chips landed on.
pub(super) fn prompt(pending: &Pending, theme: &Theme, width: u16) -> Strip {
    let Some(question) = pending.question.as_deref() else {
        return Strip::default();
    };
    // The dialog's own title in front of the question, where it draws one. Two lines is the whole
    // budget here — a row this strip takes is a row the pane never gets (#373, #374) — so the
    // header rides on the question's line rather than earning one, and the per-option descriptions
    // the Compose client draws have nowhere to go at all.
    let asked = match pending.header.as_deref() {
        Some(header) => format!("{header} · {question}"),
        None => question.to_string(),
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            " ⚑ ".to_string(),
            Style::default()
                .fg(theme.blocked)
                .bg(theme.blocked_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            clip(&asked, width.saturating_sub(4) as usize),
            Style::default()
                .fg(theme.text)
                .bg(theme.blocked_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    if let Some(context) = pending.context.as_deref().filter(|c| !c.trim().is_empty()) {
        lines.push(context_line(context.trim(), theme, width));
    }
    // Only the keys the node offered. It decides whether a submit key follows, per harness —
    // claude selects on the bare digit, codex needs an Enter (#43) — so nothing is synthesised.
    let mut spans = vec![Span::styled(
        " ".to_string(),
        Style::default().bg(theme.blocked_bg),
    )];
    let mut chips = Vec::new();
    let mut x = 1u16;
    for option in &pending.options {
        let key = format!(" {} ", option.key);
        let chip = key.chars().count() as u16;
        chips.push((x, chip));
        x += chip;
        spans.push(Span::styled(
            key,
            Style::default()
                .fg(theme.on_accent)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        // A tick is drawn rather than left implicit: on a question that takes several answers the
        // node strips the checkbox out of the label to publish `chosen`, and a reader who cannot
        // see which are ticked cannot tell what pressing again would do.
        let typed = if option.text { "…" } else { "" };
        let label = match (pending.multi, option.chosen) {
            (true, true) => format!(" ☑ {}{typed}  ", option.label),
            (true, false) => format!(" ☐ {}{typed}  ", option.label),
            _ => format!(" {}{typed}  ", option.label),
        };
        x += label.chars().count() as u16;
        spans.push(Span::styled(
            label,
            Style::default().fg(theme.text).bg(theme.blocked_bg),
        ));
    }
    if pending.options.is_empty() {
        // A hook that offers nothing is a form Claude draws itself, an MCP server's elicitation,
        // and only the terminal can fill it in.
        let said = match pending.source.as_str() {
            "hook" => "answer at the terminal · prefix shift+v",
            _ => "no keys were offered",
        };
        spans.push(Span::styled(
            said.to_string(),
            Style::default().fg(theme.dim).bg(theme.blocked_bg),
        ));
    }
    // A press on a question taking several answers ticks a box and does not answer it, so the
    // commit is its own chip, and it is only offered once there is something to commit.
    let mut submit = None;
    if pending.multi {
        match pending.options.iter().any(|o| o.chosen) {
            true => {
                let chip = " enter ";
                let wide = (chip.chars().count() + " Submit".chars().count()) as u16;
                submit = Some((x, wide));
                spans.push(Span::styled(
                    chip.to_string(),
                    Style::default()
                        .fg(theme.on_accent)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    " Submit".to_string(),
                    Style::default().fg(theme.text).bg(theme.blocked_bg),
                ));
            }
            false => spans.push(Span::styled(
                "· tick one, then enter submits".to_string(),
                Style::default().fg(theme.mute).bg(theme.blocked_bg),
            )),
        }
    }
    let chip_row = lines.len() as u16;
    lines.push(Line::from(spans));
    Strip {
        lines,
        chips,
        submit,
        chip_row,
    }
}

const READ_ALL: &str = "  alt+o reads it all";

/// One row whatever the context's length, so a plan never takes the transcript's rows; the rest
/// is a keystroke away rather than lost.
fn context_line(context: &str, theme: &Theme, width: u16) -> Line<'static> {
    let room = width.saturating_sub(4) as usize;
    let first = context.lines().next().unwrap_or_default();
    let more = context.contains('\n') || Span::raw(first).width() > room;
    let hint = more && room >= READ_ALL.len() + 24;
    let said = match context.contains('\n') {
        true => format!("{first} …"),
        false => first.to_string(),
    };
    let shown = clip(&said, room - if hint { READ_ALL.len() } else { 0 });
    let mut spans = vec![Span::styled(
        format!("   {shown}"),
        Style::default().fg(theme.dim).bg(theme.blocked_bg),
    )];
    if hint {
        spans.push(Span::styled(
            READ_ALL.to_string(),
            Style::default().fg(theme.mute).bg(theme.blocked_bg),
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    fn option(key: &str, label: &str, chosen: bool) -> kampr_client::PendingOption {
        kampr_client::PendingOption {
            key: key.into(),
            label: label.into(),
            chosen,
            ..kampr_client::PendingOption::default()
        }
    }

    fn said(pending: &Pending) -> Vec<String> {
        prompt(pending, &crate::theme::PHOSPHOR, 120)
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    /// Two lines is the whole budget, so the dialog's own title rides on the question's line
    /// rather than earning one of its own.
    #[test]
    fn a_dialogs_title_reaches_a_terminal_without_costing_the_pane_a_row() {
        let strip = prompt(
            &Pending {
                pane: "p".into(),
                question: Some("Which indentation do you prefer?".into()),
                header: Some("Indentation".into()),
                options: vec![option("1", "Tabs", false)],
                ..Pending::default()
            },
            &crate::theme::PHOSPHOR,
            120,
        );
        assert_eq!(strip.lines.len(), 2, "the strip grew a row");
        assert!(
            strip.lines[0]
                .spans
                .iter()
                .any(|s| s.content.contains("Indentation · Which indentation")),
            "{:?}",
            said(&Pending::default()),
        );
    }

    #[test]
    fn a_question_that_takes_several_answers_offers_its_commit_only_once_something_is_ticked() {
        let mut pending = Pending {
            pane: "p".into(),
            question: Some("Which test suites should I run?".into()),
            multi: true,
            options: vec![option("1", "unit", true), option("2", "integration", false)],
            ..Pending::default()
        };
        let lines = said(&pending);
        assert!(lines[1].contains("☑ unit"), "{lines:?}");
        assert!(lines[1].contains("☐ integration"), "{lines:?}");
        assert!(lines[1].contains(" enter  Submit"), "{lines:?}");
        assert!(prompt(&pending, &crate::theme::PHOSPHOR, 120).submit.is_some());

        pending.options[0].chosen = false;
        let lines = said(&pending);
        assert!(!lines[1].contains("Submit"), "{lines:?}");
        assert!(prompt(&pending, &crate::theme::PHOSPHOR, 120).submit.is_none());
    }

    /// And a single-answer question is untouched: no boxes, no warning, because a press there
    /// really is the answer.
    #[test]
    fn a_single_answer_question_is_drawn_exactly_as_it_was() {
        let lines = said(&Pending {
            pane: "p".into(),
            question: Some("Do you want to make this edit?".into()),
            options: vec![option("1", "Yes", false), option("2", "No", false)],
            ..Pending::default()
        });
        assert!(
            lines[1].contains(" Yes  ") && lines[1].contains(" No  "),
            "{lines:?}"
        );
        assert!(
            !lines[1].contains('☐') && !lines[1].contains("Submit") && !lines[1].contains("tick one"),
            "{lines:?}"
        );
    }
}
