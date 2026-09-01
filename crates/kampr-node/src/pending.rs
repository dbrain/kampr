use kampr_core::prompt::{is_chrome, numbered_option as numbered, unbox};
use kampr_core::wire::PendingOption;
use kampr_herdr::Herdr;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub question: String,
    pub options: Vec<PendingOption>,
}

/// Reads the question off the screen.
///
/// Claude does not write a pending tool request to its transcript until *after* it has been
/// answered — the file froze for 4m20s at the prompt and only then jumped to carry both the
/// `tool_use` and its result (probe #42). So the wording has to come from the pane, and `source`
/// on the wire says so. Codex does publish an unmatched call while it waits (probe #43), but the
/// wire shape is identical either way, so the screen path is the one implementation.
pub async fn read(herdr: &Herdr, pane_id: &str) -> Option<Pending> {
    let reply: Value = herdr
        .call(
            "pane.read",
            serde_json::json!({
                "pane_id": pane_id, "source": "visible", "format": "text", "strip_ansi": true
            }),
        )
        .await
        .ok()?;
    detect(reply["read"]["text"].as_str()?)
}

pub fn detect(screen: &str) -> Option<Pending> {
    let lines: Vec<&str> = screen.lines().collect();
    let cleaned: Vec<String> = lines.iter().map(|l| unbox(l)).collect();
    let options: Vec<(usize, PendingOption)> = cleaned
        .iter()
        .enumerate()
        .filter_map(|(i, line)| numbered(line).map(|o| (i, o)))
        .collect();

    // A prompt is a *run* of consecutive options starting at 1. One stray "1. " in build output
    // is not a question, and neither is a numbered list that starts at 3.
    //
    // Taken from the **bottom** of the screen, because a pane that is waiting has the thing it is
    // waiting on last: everything above the dialog is output that has already happened. Taking the
    // first run instead published three lines of a plan the agent had written earlier and left
    // standing above its own question, under the real question's "blocked" flag — options an
    // operator could press that answered nothing that was asked.
    for start in (0..options.len()).rev().filter(|i| options[*i].1.key == "1") {
        let mut chosen = vec![options[start].clone()];
        for pair in options[start + 1..].iter() {
            let expected = (chosen.len() + 1).to_string();
            let from = chosen.last().expect("non-empty").0 + 1;
            if pair.1.key != expected || blanks(&cleaned[from..pair.0]) > 1 {
                break;
            }
            chosen.push(pair.clone());
        }
        if chosen.len() < 2 {
            continue;
        }
        if let Some(question) = question_above(&lines, &cleaned, chosen[0].0) {
            return Some(Pending {
                question,
                options: chosen.into_iter().map(|(_, o)| o).collect(),
            });
        }
    }
    None
}

/// What separates two options of one menu from two numbered lines that merely follow each other.
///
/// Not a line count. An option carries its own description under it, and on a pane narrow enough
/// to wrap one that description is several lines — so a fixed gap of two truncated the menu at
/// whichever option wrapped first, and the run that was left was too short to be a menu at all.
/// A blank line is the separator a dialog uses; two of them mean the list ended.
fn blanks(between: &[String]) -> usize {
    between.iter().filter(|l| l.is_empty()).count()
}

/// How far above the options to look when no rule or border marks where the dialog starts.
const MAX_LOOKBACK: usize = 20;

/// The question, chosen from inside the dialog rather than from whatever line happens to be
/// nearest.
///
/// "Nearest non-empty line above" published `"Security guide"` on Claude's trust prompt — an OSC 8
/// link label that `pane.read` flattens to bare text (probe #36), sitting four non-blank lines
/// below the real question. So: never look above the rule or border that opens the dialog, prefer
/// a line that actually asks something, and cut it at the question mark rather than at the width
/// the harness happened to wrap at.
fn question_above(lines: &[&str], cleaned: &[String], options_at: usize) -> Option<String> {
    let floor = lines[..options_at]
        .iter()
        .rposition(|l| is_rule(l))
        .map_or(options_at.saturating_sub(MAX_LOOKBACK), |at| at + 1);
    let prose = |at: usize| {
        let line = &cleaned[at];
        !line.is_empty() && line.chars().count() > 2 && numbered(line).is_none() && !is_reference(line)
    };
    let Some(asked) = (floor..options_at)
        .rev()
        .find(|&at| prose(at) && cleaned[at].contains('?'))
    else {
        return (floor..options_at)
            .rev()
            .find(|&at| prose(at))
            .map(|at| unmark(&cleaned[at]));
    };

    // The harness wrapped the sentence at the pane's width, so the question begins wherever that
    // wrapping began — not on the row the question mark happened to land on, which on an 80-column
    // pane published "you want me to get the card?" and nothing of what the card was for.
    let mut from = asked;
    while from > floor && prose(from - 1) && !ends_a_sentence(&cleaned[from - 1]) {
        from -= 1;
    }
    let joined = unmark(
        &cleaned[from..=asked]
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let cut = joined.rfind('?')?;
    Some(joined[..=cut].to_string())
}

/// Where a wrapped sentence *cannot* have continued. A hard wrap breaks a line between words, so
/// a line that closes one is the end of what came before rather than the head of the question.
fn ends_a_sentence(line: &str) -> bool {
    matches!(line.chars().last(), Some('.' | '?' | '!' | ':'))
}

/// The glyph a harness puts in front of a line to say who is speaking. `●` opens every message
/// Claude writes and `❯` marks the row under the cursor; neither is a word, and both were being
/// published as the first characters of the question.
fn unmark(line: &str) -> String {
    line.trim_start_matches(['\u{25cf}', '\u{23fa}', '\u{2022}', '\u{276f}', '>', '*'])
        .trim()
        .to_string()
}

/// A horizontal rule or a box's top edge: the line that says the dialog starts here.
fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(is_chrome)
}

/// A URL or a bare path is something the dialog is *about*, never what it is asking.
fn is_reference(line: &str) -> bool {
    !line.contains(char::is_whitespace)
        && (line.starts_with("http://")
            || line.starts_with("https://")
            || line.starts_with('/')
            || line.starts_with("~/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = r#"
● I'll update the config.

╭──────────────────────────────────────────────────────────────╮
│ Do you want to make this edit to config.toml?                │
│                                                              │
│ ❯ 1. Yes                                                     │
│   2. Yes, and don't ask again this session                   │
│   3. No, and tell Claude what to do differently (esc)        │
╰──────────────────────────────────────────────────────────────╯
"#;

    /// Captured verbatim from a real `claude` 2.1.237 trust prompt through
    /// `pane.read visible strip_ansi`, in a headless herdr session. "Security guide" is an OSC 8
    /// link label — `pane.read` drops the hyperlink and leaves the label as ordinary text
    /// (probe #36) — and it is the nearest non-empty line above the options, which is exactly why
    /// "nearest line above" published it as the question.
    const CLAUDE_TRUST: &str = concat!(
        "[21:15:13 dbrain@comingclean kampr-convo-scratch]$ claude\n",
        "\n",
        "─────────────────────────────────────────────────────────────────────────────────────────────\n",
        " Accessing workspace:\n",
        "\n",
        " /tmp/kampr-convo-scratch\n",
        "\n",
        " Quick safety check: Is this a project you created or one you trust? (Like your own code, a\n",
        " well-known open source project, or work from your team). If not, take a moment to review\n",
        " what's in this folder first.\n",
        "\n",
        " Claude Code'll be able to read, edit, and execute files here.\n",
        "\n",
        " Security guide\n",
        "\n",
        " \u{276f} 1. Yes, I trust this folder\n",
        "   2. No, exit\n",
        "\n",
        " Enter to confirm \u{b7} Esc to cancel\n",
    );

    #[test]
    fn the_trust_prompt_publishes_the_question_and_not_the_link_label() {
        let p = detect(CLAUDE_TRUST).unwrap();
        assert_eq!(
            p.question,
            "Quick safety check: Is this a project you created or one you trust?"
        );
        assert_eq!(p.options.len(), 2);
        assert_eq!(p.options[0].key, "1");
        assert_eq!(p.options[0].label, "Yes, I trust this folder");
        assert_eq!(p.options[1].label, "No, exit");
    }

    #[test]
    fn nothing_above_the_rule_that_opens_a_dialog_is_read_as_its_question() {
        // The shell command that launched the harness sits above the rule and is not the prompt.
        let screen = "$ claude --permission-mode default\n─────────\n\n1. Yes\n2. No\n";
        assert_eq!(detect(screen), None, "no question inside the dialog at all");
    }

    #[test]
    fn a_url_or_a_bare_path_is_never_the_question() {
        let screen =
            "Proceed with the install\n\nhttps://example.com/docs\n/home/u/project\n\n1. Yes\n2. No\n";
        assert_eq!(detect(screen).unwrap().question, "Proceed with the install");
    }

    /// The other real dialog, captured the same way: `claude` 2.1.237 held at a Bash permission
    /// prompt. This one has no box at all — a horizontal rule opens it — and four lines of
    /// command detail sit between the question and the options.
    const CLAUDE_BASH: &str = concat!(
        "\u{fe0f} Crunched for 4s\n",
        "\n",
        "\u{276f} use the Bash tool to run exactly: curl -s https://example.com | head -3\n",
        "\n",
        "  Fetching example.com and showing first 3 lines\n",
        "  \u{23ce}  $ curl -s https://example.com | head -3\n",
        "\n",
        "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\n",
        " Bash command\n",
        "\n",
        "   curl -s https://example.com | head -3\n",
        "   Fetch example.com and show first 3 lines\n",
        "\n",
        " This command requires approval\n",
        "\n",
        " Do you want to proceed?\n",
        " \u{276f} 1. Yes\n",
        "   2. Yes, and don\u{2019}t ask again for: curl -s https://example.com\n",
        "   3. No\n",
        "\n",
        " Esc to cancel \u{b7} Tab to amend \u{b7} ctrl+e to explain\n",
    );

    #[test]
    fn the_bash_permission_prompt_still_reads_correctly() {
        let p = detect(CLAUDE_BASH).unwrap();
        assert_eq!(p.question, "Do you want to proceed?");
        assert_eq!(p.options.len(), 3);
        assert_eq!(p.options[0].label, "Yes");
        assert!(p.options[1].label.starts_with("Yes, and don"));
        assert_eq!(p.options[2].label, "No");
    }

    #[test]
    fn a_claude_permission_prompt_is_read_off_the_screen() {
        let p = detect(CLAUDE).unwrap();
        assert_eq!(p.question, "Do you want to make this edit to config.toml?");
        assert_eq!(p.options.len(), 3);
        assert_eq!(
            p.options[0],
            PendingOption {
                key: "1".into(),
                label: "Yes".into()
            }
        );
        assert_eq!(p.options[1].key, "2");
        assert!(p.options[1].label.starts_with("Yes, and don't ask again"));
    }

    #[test]
    fn a_bare_prompt_with_no_box_works_too() {
        let screen = "Allow this command?\n\n1) Yes\n2) No\n";
        let p = detect(screen).unwrap();
        assert_eq!(p.question, "Allow this command?");
        assert_eq!(p.options.len(), 2);
        assert_eq!(p.options[1].label, "No");
    }

    /// The report, on 0.1.44: *"claude asked me a question, kampr told me it was blocked and had a
    /// question, listed 3 options but they didn't match the actual options"*. Transcribed off the
    /// operator's screenshot — the three it listed were a plan the agent had written earlier and
    /// left standing above the dialog, which is the **first** run of `1.` on the visible screen.
    const CLAUDE_PLAN_ABOVE_A_DIALOG: &str = concat!(
        "\u{25cf} Right \u{2014} three arms, then:\n",
        "\n",
        "  1. Build exllamav3 for 86;120, load SC_4.00bpw_H5_V6 with plain autosplit, no TP. \
          Question: does it load on this card pair, and what's decode t/s?\n",
        "  2. If it clears \u{2014} add TP + MTP, compare to 41-42 t/s.\n",
        "  3. Only if it lands within ~10% is the quality win worth the second stack.\n",
        "\n",
        "\u{25cf} The 2-card arms need the 3060, which gemma is holding with 8.9 GiB. It's \
          `restart: unless-stopped` and your notes warn other sessions drive it. How do you want \
          me to get the card?\n",
        "\n",
        "\u{276f} 1. Stop gemma, bench, restart it\n",
        "     I `docker stop kobbler-llama-server-1`, run the full arm set, then bring it back up.\n",
        "  2. Single-card arm only for now\n",
        "     I run A1 on the 5060 Ti alone and report that number, then stop and wait.\n",
        "  3. I'll stop gemma myself, tell me when\n",
        "     You take it down when it suits you and say go; I run everything then.\n",
        "  4. Type something.\n",
        "\n",
        "  5. Chat about this\n",
        "\n",
        "Enter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel\n",
    );

    #[test]
    fn the_dialog_at_the_foot_of_the_screen_beats_a_numbered_list_left_standing_above_it() {
        let p = detect(CLAUDE_PLAN_ABOVE_A_DIALOG).expect("the dialog was not read at all");
        assert_eq!(
            p.question,
            "The 2-card arms need the 3060, which gemma is holding with 8.9 GiB. It's \
             `restart: unless-stopped` and your notes warn other sessions drive it. How do you \
             want me to get the card?",
        );
        assert_eq!(
            p.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(),
            vec![
                "Stop gemma, bench, restart it",
                "Single-card arm only for now",
                "I'll stop gemma myself, tell me when",
                "Type something.",
                "Chat about this",
            ],
        );
    }

    /// The same screen on an 80-column pane, which is the width most hosts run. Every option's
    /// description wraps, so the options themselves are three and four rows apart — and a fixed
    /// two-row gap cut the menu after the first one, leaving a run too short to be a menu and
    /// handing the screen back to the plan above it.
    const CLAUDE_DIALOG_WRAPPED_AT_80: &str = concat!(
        "\u{25cf} Right \u{2014} three arms, then:\n",
        "\n",
        "  1. Build exllamav3 for 86;120, load SC_4.00bpw_H5_V6 with plain autosplit, no\n",
        "     TP. Question: does it load on this card pair, and what's decode t/s?\n",
        "  2. If it clears \u{2014} add TP + MTP, compare to 41-42 t/s.\n",
        "  3. Only if it lands within ~10% is the quality win worth the second stack.\n",
        "\n",
        "\u{25cf} The 2-card arms need the 3060, which gemma is holding with 8.9 GiB. It's\n",
        "  `restart: unless-stopped` and your notes warn other sessions drive it. How do\n",
        "  you want me to get the card?\n",
        "\n",
        "\u{276f} 1. Stop gemma, bench, restart it\n",
        "     I `docker stop kobbler-llama-server-1`, run the full arm set, then bring\n",
        "     it straight back up.\n",
        "  2. Single-card arm only for now\n",
        "     I run A1 on the 5060 Ti alone and report that number, then stop and wait\n",
        "     for you to pick a window.\n",
        "  3. I'll stop gemma myself, tell me when\n",
        "     You take it down when it suits you and say go; I run everything then.\n",
        "  4. Type something.\n",
        "\n",
        "  5. Chat about this\n",
        "\n",
        "Enter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel\n",
    );

    #[test]
    fn an_option_whose_description_wraps_does_not_cut_the_menu_short() {
        let p = detect(CLAUDE_DIALOG_WRAPPED_AT_80).expect("the dialog was not read at all");
        assert_eq!(
            p.question,
            "The 2-card arms need the 3060, which gemma is holding with 8.9 GiB. It's \
             `restart: unless-stopped` and your notes warn other sessions drive it. How do you \
             want me to get the card?",
            "the question was published as the fragment the wrap happened to leave",
        );
        assert_eq!(
            p.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(),
            vec![
                "Stop gemma, bench, restart it",
                "Single-card arm only for now",
                "I'll stop gemma myself, tell me when",
                "Type something.",
                "Chat about this",
            ],
        );
    }

    /// The other half of joining a wrapped question: only the sentence being asked, never the
    /// output that happened to be printed above it. A hard wrap breaks a line mid-sentence, so a
    /// line that ends one is where the question starts.
    #[test]
    fn output_finished_above_the_question_is_not_joined_onto_it() {
        let screen = "Running the suite.\nAll 5 passed.\nDo you want to proceed?\n1. Yes\n2. No\n";
        assert_eq!(detect(screen).unwrap().question, "Do you want to proceed?");
    }

    /// The blank line a dialog puts above its question is the other half of that, and it is the
    /// half that has to hold at *any* pane width.
    ///
    /// This is `live::a_blocked_agent_pane_publishes_the_question_from_the_screen`'s own screen,
    /// off a real 93-column herdr pane (probe #406): a shell echoing the `printf` that raises the
    /// dialog, then the dialog. The echo ends in `'`, so it is not sentence-final and the join
    /// walks straight through it — whether it *reaches* it turns on where the ambient `PS1` pushes
    /// the wrap, which is a property of the machine and not of this code. Every real dialog
    /// captured here opens with a rule and puts a blank line above its question (probe #407);
    /// that blank is what cuts the join at 4 columns of prompt and at 45 alike.
    #[test]
    fn a_shell_echo_above_a_dialog_is_not_joined_onto_the_question_at_any_prompt_width() {
        let command = r"printf '\nDo you want to make this edit?\n\n 1. Yes\n 2. No\n'";
        for prompt in [
            "ci$ ",
            "runner@runnervmgx7h7:/tmp$ ",
            "[17:36:12 dbrain@comingclean tmp]$ ",
            "[21:15:13 dbrain@comingclean kampr-scratch]$ ",
        ] {
            let echo: Vec<String> = format!("{prompt}{command}")
                .chars()
                .collect::<Vec<_>>()
                .chunks(93)
                .map(|row| row.iter().collect())
                .collect();
            let screen = format!(
                "{}\n\nDo you want to make this edit?\n\n 1. Yes\n 2. No\n{prompt}",
                echo.join("\n")
            );
            let width = prompt.chars().count();
            let p = detect(&screen).unwrap_or_else(|| panic!("no dialog at a {width}-column prompt"));
            assert_eq!(
                p.question,
                "Do you want to make this edit?",
                "at a {width}-column prompt, over {} rows of echo",
                echo.len()
            );
        }
    }

    /// A URL is what the dialog is about, and a query string in one is not the thing being asked.
    #[test]
    fn a_question_mark_inside_a_url_is_not_a_question() {
        let screen = "Fetch the release notes\nhttps://example.com/notes?v=2\n\n1. Yes\n2. No\n";
        assert_eq!(detect(screen).unwrap().question, "Fetch the release notes");
    }

    #[test]
    fn ordinary_output_is_not_mistaken_for_a_prompt() {
        assert_eq!(detect("running tests\n1. ok\nall good\n"), None);
        assert_eq!(detect(""), None);
        assert_eq!(detect("3. third\n4. fourth\n"), None);
    }

    #[test]
    fn a_numbered_list_far_from_a_question_is_not_a_prompt() {
        // Options must run consecutively; a gap means these are two unrelated lines.
        let screen = "Steps\n1. clone\n\n\n\n2. build\n";
        assert_eq!(detect(screen), None);
    }

    /// The report, with the screen attached: *"that's our UI … we're parsing the Claude questions
    /// into those chips/buttons"*, next to a shot of three chips whose labels ran on into a panel
    /// standing beside them.
    ///
    /// A harness that draws two panels side by side puts both on the same screen *row*, and
    /// `pane.read` hands back rows. Stripping the box from the two ends of a row left every glyph
    /// in the middle of it, so an option's label was everything to its right — its own words, then
    /// the border between the columns, then whatever the panel next door had painted on that line.
    ///
    /// Transcribed off the operator's screenshot: a question with three options down the left and
    /// a preview of the focused one boxed on the right.
    const CLAUDE_TWO_COLUMN_DIALOG: &str = concat!(
        "\u{250c} Fit scope \u{2510}\n",
        "\n",
        "For the conversation view: which elements should shrink to fit their content instead of \
         filling the turn frame?\n",
        "\n",
        "\u{276f} 1. Cards + tables         \u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}\n",
        "     (Recommended)            \u{2502} \u{250c} you \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510} \u{2502}\n",
        "  2. Everything but prose     \u{2502} \u{2502} run the tests \u{2502} \u{2502}\n",
        "  3. Tables only              \u{2502} \u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518} \u{2502}\n",
        "                              \u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}\n",
        "\n",
        "Enter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel\n",
    );

    #[test]
    fn a_panel_standing_beside_the_options_is_not_part_of_their_labels() {
        let p = detect(CLAUDE_TWO_COLUMN_DIALOG).expect("the dialog was not read at all");
        assert_eq!(
            p.question,
            "For the conversation view: which elements should shrink to fit their content instead \
             of filling the turn frame?",
        );
        assert_eq!(
            p.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(),
            vec!["Cards + tables", "Everything but prose", "Tables only"],
        );
    }

    #[test]
    fn the_question_is_the_nearest_real_line_above_the_options() {
        let screen =
            "old output\n\n│ Run `rm -rf /tmp/x`? │\n│                     │\n│ 1. Yes │\n│ 2. No  │\n";
        assert_eq!(detect(screen).unwrap().question, "Run `rm -rf /tmp/x`?");
    }
}
