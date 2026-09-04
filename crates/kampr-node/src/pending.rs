use kampr_core::prompt::{is_chrome, marked_option as marked, numbered_option as numbered, unbox};
use kampr_core::wire::PendingOption;
use kampr_herdr::Herdr;
use serde_json::Value;

#[derive(Debug, Clone, Eq)]
pub struct Pending {
    pub question: String,
    pub options: Vec<PendingOption>,
    /// The dialog's own two-word title, drawn above the question.
    pub header: Option<String>,
    /// Whether the dialog takes several answers at once, read off the checkboxes it draws.
    pub multi: bool,
    /// Which option the harness's own cursor is on, for a dialog that has one instead of numbers.
    ///
    /// **The client is never told this and never needs to be.** It presses the key it was offered;
    /// the node turns that into the moves that get from wherever the cursor is *now* to the option
    /// asked for, and "now" is a fresh read at the moment of the press rather than whatever the
    /// last frame said.
    pub cursor: Option<usize>,
}

/// **The cursor is deliberately not part of this.** `send_pending` sends a frame when the reading
/// has moved, and the frame it sends carries no cursor — so comparing on one means an operator
/// moving `❯` at the desk emits a frame byte-identical to the last, once per press, to every
/// watching socket.
impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        self.question == other.question
            && self.options == other.options
            && self.header == other.header
            && self.multi == other.multi
    }
}

/// Reads the question off the screen.
///
/// Claude does not write a pending tool request to its transcript until *after* it has been
/// answered — the file froze for 4m20s at the prompt and only then jumped to carry both the
/// `tool_use` and its result (probe #42). So the wording has to come from the pane, and `source`
/// on the wire says so. Codex does publish an unmatched call while it waits (probe #43), but the
/// wire shape is identical either way, so the screen path is the one implementation.
pub async fn read(herdr: &Herdr, pane_id: &str, agent: Option<&str>) -> Option<Pending> {
    let reply: Value = herdr
        .call(
            "pane.read",
            serde_json::json!({
                "pane_id": pane_id, "source": "visible", "format": "text", "strip_ansi": true
            }),
        )
        .await
        .ok()?;
    detect_for(agent, reply["read"]["text"].as_str()?)
}

/// The detector this harness's screen is read with.
///
/// **One or the other, never one and then the other.** omp draws no numbered dialog at all, and it
/// *does* draw a numbered list — the steering queue, `1.` and `2.` under a `Steering · 2` header
/// ([#489](#)) — so a numbered read of an omp screen finds a question nobody asked, with the
/// prompts the operator is waiting on offered as the answers to it.
pub fn detect_for(agent: Option<&str>, screen: &str) -> Option<Pending> {
    match cursor_dialogs(agent) {
        true => detect_marked(screen),
        false => detect(screen),
    }
}

/// The harnesses measured to ask with a cursor rather than with numbers.
///
/// **Per-harness on purpose, and `omp` alone.** A numbered run proves itself — a dialog that draws
/// `1.` `2.` `3.` is a menu and nothing else looks like one — but a cursor run is anchored on a
/// single glyph and a column, and turning that loose on a harness nobody has looked at is inviting
/// a false question onto a pane. omp draws no numbers at all: a digit sent into either of its
/// dialogs leaves them standing, measured against both ([#487](#)).
///
/// **`pi` is not on this list**, though the same adapter reads its transcripts. What [#490](#)
/// measured about it is the session *format*; its TUI is a different program — herdr's own `pi`
/// manifest matches `Working...` where omp paints `⎋ Working…` — and nobody has put a keystroke
/// into one of its dialogs. A `pi` pane keeps the numbered reading every unmeasured harness gets.
pub fn cursor_dialogs(agent: Option<&str>) -> bool {
    matches!(agent, Some(kampr_journal::omp::AGENT))
}

/// A dialog whose options are marked with a cursor.
///
/// **Bounded in both directions, and refused outright at column zero.** A blank row and a row
/// indented past the label column are both walked over rather than stopping the run — that is what
/// lets an option's description sit under it — so the only things that end one are a rule, a row
/// at or left of the label column, and this bound. An anchor whose label starts in column zero
/// would make "indented past the label" true of every line on the screen, and the walk would
/// gather arbitrary prose as options; omp indents its own by four, inside a box.
///
/// The run is anchored on the row carrying the cursor — taken from the bottom, like the numbered
/// one, because a pane that is waiting has the thing it is waiting on last — and gathered from the
/// rows whose label starts in the **same column**. That alignment is the whole of what separates a
/// sibling option from the description omp indents under one, and it is why nothing here needs to
/// know which of the two dialogs it is reading.
pub fn detect_marked(screen: &str) -> Option<Pending> {
    let lines: Vec<&str> = screen.lines().collect();
    let inner: Vec<&str> = lines.iter().map(|l| inside(l)).collect();
    let cleaned: Vec<String> = lines.iter().map(|l| unbox(l)).collect();
    let focus = (0..inner.len())
        .rev()
        .find(|at| marked(inner[*at]).is_some_and(|m| m.focused))?;
    let anchor = marked(inner[focus]).filter(|m| m.at > 0)?;
    let aligned = |at: usize| marked(inner[at]).filter(|m| m.at == anchor.at).map(|m| (at, m));
    let mut rows = vec![(focus, anchor.clone())];
    for step in [true, false] {
        let mut at = focus;
        for _ in 0..MAX_LOOKBACK {
            at = match step {
                true => match at.checked_sub(1) {
                    Some(next) => next,
                    None => break,
                },
                false => at + 1,
            };
            if at >= inner.len() || is_rule(lines[at]) {
                break;
            }
            match aligned(at) {
                Some(found) => rows.push(found),
                // A description sits further in and a blank row separates the run from the
                // dialog's own footer; anything at or left of the label column ends it.
                None if inner[at].trim().is_empty() || indented_past(inner[at], anchor.at) => continue,
                None => break,
            }
        }
    }
    rows.sort_by_key(|(at, _)| *at);
    if rows.len() < 2 {
        return None;
    }
    // **The box the dialog is drawn in, not the rule inside it.** omp puts its question in the
    // head of the box and a rule between that and the options, so a floor at the nearest rule
    // finds nothing at all — and one that simply looks further up joins the harness's own chrome
    // onto the front of the question, which is what the first attempt published.
    let opened = box_top(&lines, rows[0].0);
    let question = question_above(
        &cleaned,
        rows[0].0,
        opened.map_or_else(|| rule_floor(&lines, rows[0].0), |at| at + 1),
    )?;
    let cursor = rows.iter().position(|(_, m)| m.focused);
    let at: Vec<usize> = rows.iter().map(|(at, _)| *at).collect();
    // **The checkbox is what says the question takes several answers**, and it says so while
    // nothing is ticked — which is the state every one of them opens in (#421). omp draws `☐`/`☑`
    // where a question that takes one answer gets `○` ([#494](#)).
    let multi = rows.iter().any(|(_, found)| found.ticked.is_some());
    let options = rows
        .iter()
        .enumerate()
        .map(|(index, (row, found))| PendingOption {
            key: (index + 1).to_string(),
            label: found.label.clone(),
            // `inner` rather than `lines`: the indentation test that separates a description
            // from the screen below the dialog is done on the row's own text, and omp's rows
            // start with the box border rather than with the spaces that test looks for.
            detail: detail_under(&inner, &cleaned, *row, at.get(index + 1).copied()),
            chosen: found.ticked.unwrap_or(false),
        })
        .collect();
    Some(Pending {
        question,
        header: box_title(&lines, rows[0].0),
        multi,
        options,
        cursor,
    })
}

/// A line with its box borders taken off and **its indentation kept**, which `unbox` does not do:
/// alignment is the only thing that separates one of these options from the description under it.
fn inside(line: &str) -> &str {
    let line = line.trim_end();
    let line = line.strip_suffix('│').unwrap_or(line);
    line.strip_prefix('│').unwrap_or(line)
}

fn indented_past(line: &str, at: usize) -> bool {
    line.chars().take_while(|c| *c == ' ').count() > at
}

/// Where the dialog's own box opens.
fn box_top(lines: &[&str], options_at: usize) -> Option<usize> {
    let floor = options_at.saturating_sub(MAX_LOOKBACK);
    (floor..options_at)
        .rev()
        .find(|at| lines[*at].starts_with(['╭', '┌']))
}

/// How far above the options the question may be, for a dialog that is not drawn in a box: never
/// past the rule that opens it (probe #36's link label sat four lines below the real question).
fn rule_floor(lines: &[&str], options_at: usize) -> usize {
    lines[..options_at]
        .iter()
        .rposition(|l| is_rule(l))
        .map_or(options_at.saturating_sub(MAX_LOOKBACK), |at| at + 1)
}

/// The title omp draws into the top border of the box — `Allow tool: bash`, `Ask`.
fn box_title(lines: &[&str], options_at: usize) -> Option<String> {
    let at = box_top(lines, options_at)?;
    let title = lines[at].trim_start_matches(['╭', '┌', '─', ' ']);
    let title = title.trim_end_matches(['╮', '┐', '─', ' ']).trim();
    (!title.is_empty()).then(|| title.to_string())
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
        if let Some(question) = question_above(&cleaned, chosen[0].0, rule_floor(&lines, chosen[0].0)) {
            let rows: Vec<usize> = chosen.iter().map(|(at, _)| *at).collect();
            let mut options: Vec<PendingOption> = chosen.into_iter().map(|(_, o)| o).collect();
            // **Noticed while stripping, not afterwards.** A checkbox is what says the question
            // takes several answers, and by the time the labels have had theirs taken out there is
            // nothing left to notice — which read a dialog with *nothing ticked yet* as an ordinary
            // one, and that is the state every one of them opens in.
            let mut boxed = false;
            for (index, option) in options.iter_mut().enumerate() {
                option.detail = detail_under(&lines, &cleaned, rows[index], rows.get(index + 1).copied());
                let (label, ticked) = untick(&option.label);
                if let Some(ticked) = ticked {
                    boxed = true;
                    option.label = label;
                    option.chosen = ticked;
                }
            }
            let multi = boxed;
            return Some(Pending {
                question,
                header: header_above(&cleaned, rows[0]),
                multi,
                options,
                cursor: None,
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

/// What the dialog says an option *means*: the lines between it and the next option that are not
/// options themselves.
///
/// **Not chosen by indentation.** The obvious rule is "more indented than the option", and it is
/// wrong on the harness's own output: a single-answer dialog indents a description five columns
/// under a two-column option, and a multi-answer one draws both at two (#421). What separates them
/// is that a description is not a numbered row, which is the same thing the option run is found by.
///
/// A blank line ends it. `blanks` already allows one blank *inside* a menu, so a description that
/// runs to a second paragraph would otherwise swallow the gap before the next option.
fn detail_under(
    lines: &[&str],
    cleaned: &[String],
    option_at: usize,
    next_at: Option<usize>,
) -> Option<String> {
    let end = next_at.unwrap_or(cleaned.len()).min(cleaned.len());
    let mut said: Vec<&str> = Vec::new();
    for at in (option_at + 1)..end {
        let text = cleaned.get(at)?.trim();
        if text.is_empty() {
            break;
        }
        if numbered(text).is_some() || is_rule(text) {
            break;
        }
        // **A description is indented and the screen under the dialog is not.** The last option
        // has no next option to stop at, so without this it runs to the foot of the screen and
        // takes the shell prompt with it — seen on a live pane, `"Run the browser test suite.
        // [11:23:16 dbrain@comingclean tmp]$"`. How *much* indentation is not a rule (a
        // single-answer dialog uses five columns and a multiple-answer one two, #421); having any
        // at all is what both have and what a prompt at column zero does not.
        if !lines.get(at)?.starts_with([' ', '\t']) {
            break;
        }
        said.push(text);
    }
    let joined = said.join(" ");
    let trimmed = joined.trim();
    // The one line that is the dialog's own control rather than anything about the option: a
    // multi-answer dialog draws `Submit` under its last row, and publishing it as a description
    // would put the word on a card that is not it.
    if trimmed.is_empty() || trimmed == SUBMIT_LABEL {
        return None;
    }
    Some(trimmed.to_string())
}

/// The label of the submit control a multi-answer dialog draws, which is not a description.
const SUBMIT_LABEL: &str = "Submit";

/// `[ ] unit` and `[✔] unit` — the checkbox a question that takes several answers draws against
/// every option, and the only thing on the screen that says it is one.
fn untick(label: &str) -> (String, Option<bool>) {
    let trimmed = label.trim_start();
    let Some(rest) = trimmed.strip_prefix('[') else {
        return (label.to_string(), None);
    };
    let Some((mark, tail)) = rest.split_once(']') else {
        return (label.to_string(), None);
    };
    let mark = mark.trim();
    let ticked = match mark {
        "" => false,
        "\u{2714}" | "x" | "X" | "*" | "\u{2713}" => true,
        _ => return (label.to_string(), None),
    };
    (tail.trim().to_string(), Some(ticked))
}

/// The dialog's own title, drawn above the question against a box glyph.
///
/// A multi-answer dialog draws it in a row of controls — `\u{2190}  \u{2610} Test suites  \u{2714} Submit  \u{2192}` — so
/// what is taken is the run between the box and the next control, which is separated by more than
/// one space. A single-answer dialog draws the box and the title alone.
fn header_above(cleaned: &[String], options_at: usize) -> Option<String> {
    let floor = options_at.saturating_sub(MAX_LOOKBACK);
    let at = (floor..options_at)
        .rev()
        .find(|&at| cleaned[at].contains(['\u{2610}', '\u{2612}', '\u{2611}']))?;
    let line = &cleaned[at];
    let start = line.find(['\u{2610}', '\u{2612}', '\u{2611}'])? + '\u{2610}'.len_utf8();
    let rest = line[start..].trim_start();
    let title = rest.split("  ").next()?.trim();
    (!title.is_empty() && title != SUBMIT_LABEL).then(|| title.to_string())
}

/// The question, chosen from inside the dialog rather than from whatever line happens to be
/// nearest.
///
/// "Nearest non-empty line above" published `"Security guide"` on Claude's trust prompt — an OSC 8
/// link label that `pane.read` flattens to bare text (probe #36), sitting four non-blank lines
/// below the real question. So: never look above the rule or border that opens the dialog, prefer
/// a line that actually asks something, and cut it at the question mark rather than at the width
/// the harness happened to wrap at.
fn question_above(cleaned: &[String], options_at: usize, floor: usize) -> Option<String> {
    let prose = |at: usize| {
        let line = &cleaned[at];
        !line.is_empty()
            && line.chars().count() > 2
            && numbered(line).is_none()
            && !is_reference(line)
            && !is_control_row(line)
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

/// The row of controls a dialog draws above a question that takes several answers — Claude's
/// `←  ☐ Test suites  ✔ Submit  →` and omp's `suites    branch    Submit` ([#494](#)).
///
/// It is chrome, and it is prose to every test that looks for prose: without this the wrap-joining
/// that reassembles a question split across rows walks straight into it and publishes
/// `"suites    branch    Submit Which test suites should I run?"` as the question.
fn is_control_row(line: &str) -> bool {
    line.split("  ")
        .map(str::trim)
        .any(|chip| chip.trim_start_matches(['\u{2714}', '\u{2610}', ' ']).trim() == SUBMIT_LABEL)
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
                label: "Yes".into(),
                detail: None,
                chosen: false,
            }
        );
        assert_eq!(p.options[1].key, "2");
        assert!(p.options[1].label.starts_with("Yes, and don't ask again"));
    }

    /// Captured verbatim from a real `claude` 2.1.258 `AskUserQuestion` dialog through
    /// `pane.read visible strip_ansi` in a throwaway herdr session
    /// (`research/probe/ask-question/on-disk.py`). Kampr published the five labels and nothing
    /// else, which is the report: *"we get options to select from with no context around them and
    /// the context is the most important part"*.
    #[test]
    fn a_question_the_harness_asked_carries_its_title_and_what_each_answer_means() {
        let p = detect(&fixture("claude-single")).expect("a dialog");

        assert_eq!(p.header.as_deref(), Some("Indentation"));
        assert_eq!(p.question, "Which indentation do you prefer?");
        assert!(!p.multi, "a single-answer dialog draws no checkboxes");
        assert_eq!(
            p.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(),
            [
                "Tabs",
                "Two spaces",
                "Four spaces",
                "Type something.",
                "Chat about this"
            ],
        );
        assert_eq!(
            p.options[0].detail.as_deref(),
            Some("Indent with tab characters.")
        );
        assert_eq!(
            p.options[2].detail.as_deref(),
            Some("Indent with four spaces per level.")
        );
        assert_eq!(
            p.options[3].detail, None,
            "the dialog's own escape hatches describe nothing and must not borrow a description",
        );
        assert!(p.options.iter().all(|o| !o.chosen));
    }

    /// The same harness, a question that takes several answers, captured after two digits were
    /// sent to it. **A digit toggles here and answers there** — measured, #421 — so what the
    /// screen says about which are ticked is the only thing a client has to go on.
    #[test]
    fn a_question_that_takes_several_answers_says_so_and_says_which_are_ticked() {
        let p = detect(&fixture("claude-multi")).expect("a dialog");

        assert!(
            p.multi,
            "the checkboxes are the only thing on the screen that says so"
        );
        assert_eq!(p.header.as_deref(), Some("Test suites"));
        assert_eq!(p.question, "Which test suites should I run?");
        assert_eq!(
            p.options
                .iter()
                .map(|o| (o.label.as_str(), o.chosen))
                .collect::<Vec<_>>(),
            [
                ("unit", true),
                ("integration", false),
                ("browser", true),
                ("Type something", false),
                ("Chat about this", false),
            ],
            "the two digits sent were 1 and 3",
        );
        assert_eq!(
            p.options[1].detail.as_deref(),
            Some("Run the integration test suite.")
        );
        assert_eq!(
            p.options[3].detail, None,
            "the dialog draws its own Submit control under the last row, which is not a description",
        );
    }

    /// A permission prompt draws no title, no checkboxes and no descriptions, and must not grow
    /// any: everything above is `AskUserQuestion`'s and this is the other half of what a pane
    /// blocks on.
    #[test]
    fn a_permission_prompt_still_has_no_title_no_ticks_and_no_descriptions() {
        let p = detect(CLAUDE).expect("a dialog");
        assert_eq!(p.header, None);
        assert!(!p.multi);
        assert!(p.options.iter().all(|o| o.detail.is_none() && !o.chosen));

        let trust = detect(CLAUDE_TRUST).expect("a dialog");
        assert_eq!(trust.header, None);
        assert!(!trust.multi);
        assert!(trust.options.iter().all(|o| o.detail.is_none()));
    }

    /// Seen on a live pane while writing the test above: the last option has no next option to
    /// stop at, so its description ran to the foot of the screen and took the shell prompt with it.
    #[test]
    fn the_last_options_description_stops_at_the_dialog_rather_than_at_the_foot_of_the_screen() {
        let screen = concat!(
            " \u{2610} Test suites\n",
            "\n",
            "Which test suites should I run?\n",
            "\n",
            "1. [ ] unit\n",
            "  Run the unit test suite.\n",
            "2. [ ] browser\n",
            "  Run the browser test suite.\n",
            "[11:23:16 dbrain@comingclean tmp]$ \n",
        );
        let p = detect(screen).expect("a dialog");
        assert_eq!(
            p.options[1].detail.as_deref(),
            Some("Run the browser test suite.")
        );
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("tests/fixtures/dialogs/{name}.txt")).expect("a captured dialog")
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

    /// **omp draws no numbers**, and a digit sent into either of its dialogs leaves it standing —
    /// measured against both, with the arrow keys moving the `❯` and Enter committing it
    /// ([#487](#)). So the options are found by the cursor and the column their labels start in,
    /// and the key on the wire is one this node synthesised: the client presses what it was
    /// offered, and the node turns that into the moves that reach it.
    #[test]
    fn an_approval_omp_draws_no_numbers_on_is_still_a_question_with_answers() {
        let p = detect_marked(&fixture("omp-approval")).expect("a dialog");
        assert_eq!(p.header.as_deref(), Some("Allow tool: bash"));
        assert_eq!(p.question, "Command: sleep 20; echo slow-done");
        assert_eq!(
            p.options.iter().map(|o| (&*o.key, &*o.label)).collect::<Vec<_>>(),
            [("1", "Approve"), ("2", "Deny")]
        );
        assert_eq!(p.cursor, Some(0));
        assert!(!p.multi);
        // The numbered detector is what runs first for every harness, and it finds nothing here —
        // which is why the marked one exists rather than replacing it.
        assert_eq!(detect(&fixture("omp-approval")), None);
    }

    /// The same dialog after one press of `↓`. Nothing about the options changed; the cursor did,
    /// and it is the cursor that decides how far an answer has to travel.
    #[test]
    fn the_cursor_is_read_wherever_the_operator_left_it() {
        let p = detect_marked(&fixture("omp-approval-moved")).expect("a dialog");
        assert_eq!(p.cursor, Some(1));
        assert_eq!(p.options.len(), 2);
    }

    /// omp's `ask` puts the question in the head of its box with a rule under it, where Claude
    /// puts it *below* the rule that opens the dialog — so the search has to be allowed past one,
    /// and the descriptions indented under each option must not be read as options themselves.
    #[test]
    fn the_ask_tools_question_sits_above_the_rule_and_its_options_below_it() {
        let p = detect_marked(&fixture("omp-ask")).expect("a dialog");
        assert_eq!(p.question, "Which branch should this land on?");
        assert_eq!(p.header.as_deref(), Some("Ask"));
        assert_eq!(
            p.options
                .iter()
                .map(|o| (&*o.label, o.detail.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("main", Some("Straight onto the default branch.")),
                ("a topic branch", Some("Open a PR instead.")),
                ("Other (type your own)", None),
            ]
        );
        assert_eq!(p.cursor, Some(0));
    }

    /// The detector is offered only to the harnesses whose dialogs have been measured. A cursor
    /// run is anchored on one glyph and a column, and turning it loose on a harness nobody has
    /// looked at is how a phone gets offered a question that was never asked.
    #[test]
    fn a_cursor_dialog_is_read_only_for_the_harnesses_it_was_measured_on() {
        assert!(cursor_dialogs(Some("omp")));
        // **`pi` is not on the list**, though the same adapter reads its transcripts: [#490](#)
        // measured the session format they share, and nobody has put a keystroke into one of its
        // dialogs. It keeps the numbered reading every unmeasured harness gets.
        assert!(!cursor_dialogs(Some("pi")));
        assert!(!cursor_dialogs(Some("claude")));
        assert!(!cursor_dialogs(None));
    }

    /// Claude's own dialogs must read exactly as they did: the marked detector is a fallback for
    /// the harnesses that need it, never a second opinion about a screen the first one read.
    #[test]
    fn a_numbered_dialog_is_still_read_by_the_numbers() {
        let p = detect(&fixture("claude-single")).expect("a dialog");
        assert_eq!(p.cursor, None);
        assert!(p.options.iter().all(|o| o.key.parse::<u8>().is_ok()));
    }

    /// **The steering queue is a numbered list on a pane that is often blocked**, and it is not a
    /// question: `Steering · 2` over `1.` and `2.` is exactly the shape the numbered detector was
    /// built to find. Reading an omp screen for numbers would offer the operator their own waiting
    /// prompts as answers to a question nobody asked — which is why the two detectors are chosen
    /// between rather than tried in turn.
    #[test]
    fn the_prompts_an_operator_has_queued_are_not_a_question_they_are_being_asked() {
        let screen = fixture("omp-queue");
        assert!(
            detect(&screen).is_some(),
            "the numbered detector does find it, which is the hazard"
        );
        // Through the routing rather than past it: this is the whole of what stops the hazard.
        assert_eq!(detect_for(Some("omp"), &screen), None);
        assert!(
            detect_for(Some("claude"), &screen).is_some(),
            "and every other harness is read for numbers exactly as before"
        );
    }

    /// An option list is found by a cursor glyph and a column, which is a looser thing to find than
    /// a numbered run — so the walk is bounded, and an anchor with nothing in front of it is
    /// refused rather than allowed to gather the screen.
    #[test]
    fn a_cursor_in_column_zero_is_not_a_dialog() {
        let screen = "\u{276f}Approve\nDeny\nSomething the agent printed earlier\n";
        assert_eq!(detect_marked(screen), None);
    }

    /// **A checkbox means a press is a tick, not an answer** (#421), and omp says so the same way
    /// Claude does — with the glyph in front of the label, while nothing is ticked yet
    /// ([#494](#)). It also draws a control row above the question, `suites  branch  Submit`,
    /// which is the harness talking about the dialog rather than about any option in it.
    #[test]
    fn a_question_omp_takes_several_answers_to_says_so_before_anything_is_ticked() {
        // **The dialog as it opens**, which is the state that matters: nothing is ticked, so the
        // *presence* of the checkbox is the only thing saying what kind of question this is.
        let p = detect_marked(&fixture("omp-ask-multi")).expect("a dialog");
        assert!(
            p.multi,
            "nothing is ticked and it is still a multiple-answer question: {p:?}"
        );
        assert_eq!(p.question, "Which test suites should I run?");
        assert_eq!(
            p.options
                .iter()
                .map(|o| (&*o.label, o.chosen))
                .collect::<Vec<_>>(),
            [
                ("unit", false),
                ("integration", false),
                ("browser", false),
                ("Other (type your own)", false),
            ],
            "{p:?}"
        );
        assert_eq!(p.cursor, Some(0));

        // And the same dialog after one `Space`, which is what a press on it does.
        let ticked = detect_marked(&fixture("omp-ask-multi-ticked")).expect("a dialog");
        assert!(ticked.multi);
        assert_eq!(
            ticked.options.iter().map(|o| o.chosen).collect::<Vec<_>>(),
            [true, false, false, false],
            "{ticked:?}"
        );
    }

    /// The second question of the same call, drawn in place of the first once it was answered —
    /// the shape [#492](#) is about. It takes one answer where the first took several, and the
    /// strip has to say so or the press it offers means the wrong thing.
    #[test]
    fn the_next_question_of_one_call_is_read_as_the_question_it_is() {
        let p = detect_marked(&fixture("omp-ask-second")).expect("a dialog");
        assert!(!p.multi, "a radio, not a checkbox: {p:?}");
        assert_eq!(p.question, "Which branch should this land on?");
        assert!(p.options.iter().all(|o| !o.chosen), "{p:?}");
    }
}
