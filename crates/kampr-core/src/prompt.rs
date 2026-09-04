//! Reading option lists off a screen, shared by the two detectors that need it.
//!
//! `kampr_node::pending` reads an agent's TUI dialog — a box, a rule, a run of options down the
//! left. [`crate::question`] reads a line-oriented CLI prompt, where the options are usually on
//! one line and the question is the unterminated text after them. Those are different screens and
//! they stay different detectors; what they share is how a single option is spelled, and that
//! lives here so it is spelled once.

use crate::wire::PendingOption;

/// One `1. Yes` or `2) No`, with any selection caret in front of it stripped.
///
/// Two digits at most: a three-digit run is a numbered list in some output, not a menu somebody
/// is expected to choose from.
pub fn numbered_option(line: &str) -> Option<PendingOption> {
    let body = line.trim_start_matches(['❯', '>', '➤', '*', ' ']).trim_start();
    let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    let rest = &body[digits.len()..];
    let label = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?.trim();
    if label.is_empty() {
        return None;
    }
    Some(PendingOption {
        key: digits,
        label: label.to_string(),
        detail: None,
        chosen: false,
    })
}

/// One option of a dialog that marks its choice with a **cursor** rather than a number:
/// `❯ Approve` beside `  Deny`, or `❯ ○ main` beside `  ○ a topic branch`.
///
/// **A line on its own is not evidence of anything here.** Without a number, an unfocused option
/// is indistinguishable from any indented line of prose — which is why `at` is on this at all: a
/// run is anchored on the row that carries the cursor and gathered from the rows whose *label*
/// starts in the same column, so the description omp indents under an option is excluded by the
/// two columns it is further in. The caller is per-harness for the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    pub focused: bool,
    /// The column the label starts at, past the cursor and any radio glyph.
    pub at: usize,
    pub label: String,
    /// `Some` when the glyph in front of the label is a **checkbox** rather than a radio, and
    /// whether it is ticked. A checkbox is the only thing on the screen that says the question
    /// takes several answers, and on such a question a press is a tick rather than an answer
    /// (#421) — so losing it in the strip means offering a press that looks like one and is not.
    pub ticked: Option<bool>,
}

/// The cursor a harness draws against the row it would commit.
const CURSORS: [char; 3] = ['\u{276f}', '\u{27a4}', '\u{25b8}'];

/// The radio glyph an option may carry between the cursor and its label.
const RADIOS: [char; 4] = ['\u{25cb}', '\u{25cf}', '\u{25c9}', '\u{25ef}'];

/// The checkbox glyphs, empty and ticked. A checkbox is the only thing on the screen that says a
/// question takes several answers.
const BOXES: [char; 2] = ['\u{2610}', '\u{2611}'];

pub fn marked_option(line: &str) -> Option<Marked> {
    let mut at = 0;
    let mut rest = line;
    let eat = |rest: &mut &str, at: &mut usize, wanted: &[char]| {
        let mut taken = false;
        while let Some(c) = rest.chars().next() {
            if c == ' ' {
                *at += 1;
                *rest = &rest[1..];
            } else if !taken && wanted.contains(&c) {
                taken = true;
                *at += 1;
                *rest = &rest[c.len_utf8()..];
            } else {
                break;
            }
        }
        taken
    };
    let focused = eat(&mut rest, &mut at, &CURSORS);
    let boxed = rest.chars().next().filter(|c| BOXES.contains(c));
    if boxed.is_none() {
        eat(&mut rest, &mut at, &RADIOS);
    } else {
        eat(&mut rest, &mut at, &BOXES);
    }
    let label = rest.trim_end();
    let ok = !label.is_empty()
        && label.chars().count() > 1
        && !label.chars().all(is_chrome)
        && numbered_option(label).is_none();
    ok.then(|| Marked {
        focused,
        at,
        label: label.to_string(),
        ticked: boxed.map(|c| c == BOXES[1]),
    })
}

/// Every `1) a  2) b` on one line, in the order written.
///
/// A CLI packs a provider list onto one row where a TUI puts each on its own, so the line parser
/// alone would find only the first. Rejects a bare digit that is not followed by `)` or `.`, so
/// `Total Installed Size: 9.69 MiB` yields nothing.
pub fn numbered_run(line: &str) -> Vec<PendingOption> {
    let chars: Vec<char> = line.chars().collect();
    let mut marks: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !is_word(chars[i - 1])) {
            let start = i;
            let mut digits = String::new();
            while i < chars.len() && chars[i].is_ascii_digit() && digits.len() < 2 {
                digits.push(chars[i]);
                i += 1;
            }
            if i < chars.len()
                && matches!(chars[i], ')' | '.')
                && chars.get(i + 1).is_some_and(|c| c.is_whitespace())
            {
                marks.push((start, digits));
                i += 1;
                continue;
            }
        }
        i += 1;
    }

    let mut out = Vec::new();
    for (index, (at, digits)) in marks.iter().enumerate() {
        let from = at + digits.len() + 1;
        let to = marks.get(index + 1).map_or(chars.len(), |(next, _)| *next);
        let label: String = chars[from.min(chars.len())..to.min(chars.len())].iter().collect();
        let label = label.trim();
        if !label.is_empty() {
            out.push(PendingOption {
                key: digits.clone(),
                label: label.to_string(),
                detail: None,
                chosen: false,
            });
        }
    }
    out
}

/// Whether the keys are `1`, `2`, `3`… with nothing missing.
///
/// One stray `1)` in build output is not a menu, and a list that starts at 3 is not one either.
pub fn is_consecutive_from_one(options: &[PendingOption]) -> bool {
    options.len() >= 2
        && options
            .iter()
            .enumerate()
            .all(|(i, o)| o.key == (i + 1).to_string())
}

/// Strips the box a TUI draws around a prompt, and everything a *second* panel painted on the same
/// row.
///
/// Herdr's `strip_ansi` removes the colour, not the border glyphs, so the box has to come off
/// here. Taking it off the two ends is not enough: a harness that draws two panels side by side
/// puts both of them on one screen row, and `pane.read` hands back rows — so an option's label ran
/// on through the border between the columns and into whatever stood next to it. The operator saw
/// it as chips reading `Everything but prose │ │ run the tests │`.
///
/// The cut is at the first **box-drawing** glyph (U+2500–U+257F), which is a class no CLI puts
/// inside a label. Deliberately not at ASCII `|`: a permission prompt's own option is routinely
/// `Yes, and don't ask again for: curl -s https://example.com | head -3`, and cutting there would
/// hide the half of the command that matters.
pub fn unbox(line: &str) -> String {
    let trimmed = line.trim().trim_matches(is_chrome).trim();
    match trimmed.find(is_column_rule) {
        Some(at) => trimmed[..at].trim_end().to_string(),
        None => trimmed.to_string(),
    }
}

fn is_column_rule(c: char) -> bool {
    ('\u{2500}'..='\u{257f}').contains(&c)
}

pub fn is_chrome(c: char) -> bool {
    matches!(
        c,
        '│' | '┃' | '║' | '|' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘'
    ) || matches!(c, '─' | '━' | '═' | '-' | '·' | '⎯')
}

/// Removes CSI and OSC sequences from raw pty bytes already decoded to text.
///
/// A fleet supervisor reads its own pty, so unlike a herdr `pane.read` nothing has stripped these
/// for it — and pacman writes `\x1b[?25h` immediately after its question mark (probe #336), which
/// is enough to defeat any match anchored at the end of the line. `sudo`'s OSC 3008 audit record
/// arrives the same way.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC runs to BEL or ST (ESC \).
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('(') | Some(')') | Some('#') => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '.' || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caret_in_front_of_an_option_is_not_part_of_its_label() {
        let o = numbered_option("❯ 1. Yes, I trust this folder").expect("an option");
        assert_eq!(o.key, "1");
        assert_eq!(o.label, "Yes, I trust this folder");
    }

    #[test]
    fn a_row_carrying_two_panels_ends_at_the_border_between_them() {
        assert_eq!(
            unbox("  2. Everything but prose     \u{2502} \u{2502} run the tests \u{2502} \u{2502}"),
            "2. Everything but prose",
        );
        assert_eq!(
            unbox("\u{276f} 1. Cards + tables   \u{250c}\u{2500}\u{2500}\u{2500}\u{2510} \u{2502} x"),
            "\u{276f} 1. Cards + tables",
        );
    }

    #[test]
    fn a_pipe_inside_a_command_is_not_a_border() {
        // The label a Bash permission prompt actually offers. Cutting at ASCII `|` would publish
        // half a command as the whole of it.
        let line = "\u{2502} 2. Yes, and don't ask again for: curl -s https://example.com | head -3 \u{2502}";
        assert_eq!(
            unbox(line),
            "2. Yes, and don't ask again for: curl -s https://example.com | head -3",
        );
    }

    #[test]
    fn three_digits_are_output_and_not_a_menu() {
        assert!(numbered_option("100. done").is_none());
    }

    #[test]
    fn a_cli_packs_its_options_onto_one_line() {
        let run = numbered_run("   1) linux-firmware  2) linux-firmware-git  3) skip");
        assert_eq!(run.len(), 3);
        assert_eq!(run[0].label, "linux-firmware");
        assert_eq!(run[2].label, "skip");
        assert!(is_consecutive_from_one(&run));
    }

    #[test]
    fn a_size_is_not_a_menu() {
        // The line right above pacman's real prompt (probe #336). A version number is the classic
        // way a digit scanner invents options that were never offered.
        assert!(numbered_run("Total Installed Size:  9.69 MiB").is_empty());
        assert!(numbered_run("warning: bash-5.3.15-2 is up to date -- reinstalling").is_empty());
    }

    #[test]
    fn a_run_that_does_not_start_at_one_is_not_a_menu() {
        let run = numbered_run("3) c  4) d");
        assert!(!is_consecutive_from_one(&run));
    }

    #[test]
    fn the_cursor_show_pacman_writes_after_its_question_is_stripped() {
        // #336: the real bytes. Without this the prompt does not end at `]` and no shape matches.
        assert_eq!(
            strip_ansi(":: Proceed with installation? [Y/n] \u{1b}[?25h"),
            ":: Proceed with installation? [Y/n] "
        );
    }

    #[test]
    fn sudos_osc_audit_record_leaves_nothing_behind() {
        // #336: sudo wraps the run in OSC 3008, terminated by ST rather than BEL.
        let raw = "\u{1b}]3008;start=6f89;user=dbrain;targetuser=root\u{1b}\\ready";
        assert_eq!(strip_ansi(raw), "ready");
    }

    #[test]
    fn an_osc_terminated_by_bel_also_ends() {
        assert_eq!(strip_ansi("\u{1b}]0;a title\u{7}after"), "after");
    }
}
