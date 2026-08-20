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
    let start = options.iter().position(|(_, o)| o.key == "1")?;
    let mut chosen = vec![options[start].clone()];
    for pair in options[start + 1..].iter() {
        let expected = (chosen.len() + 1).to_string();
        if pair.1.key != expected || pair.0 > chosen.last().expect("non-empty").0 + 2 {
            break;
        }
        chosen.push(pair.clone());
    }
    if chosen.len() < 2 {
        return None;
    }

    let question = question_above(&lines, &cleaned, chosen[0].0)?;
    Some(Pending {
        question,
        options: chosen.into_iter().map(|(_, o)| o).collect(),
    })
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
    let candidates: Vec<&String> = cleaned[floor..options_at]
        .iter()
        .filter(|l| !l.is_empty() && l.chars().count() > 2 && numbered(l).is_none())
        .collect();
    let asked = candidates
        .iter()
        .rev()
        .find_map(|l| l.rfind('?').map(|at| l[..=at].to_string()));
    asked.or_else(|| {
        candidates
            .iter()
            .rev()
            .find(|l| !is_reference(l))
            .map(|l| (*l).clone())
    })
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

/// Strips the box a TUI draws around a prompt, so the text inside can be read like any other
/// line. Herdr's `strip_ansi` removes the colour, not the border glyphs.
fn unbox(line: &str) -> String {
    line.trim().trim_matches(is_chrome).trim().to_string()
}

fn is_chrome(c: char) -> bool {
    matches!(
        c,
        '│' | '┃' | '║' | '|' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘'
    ) || matches!(c, '─' | '━' | '═' | '-' | '·' | '⎯')
}

fn numbered(line: &str) -> Option<PendingOption> {
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
    })
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

    #[test]
    fn the_question_is_the_nearest_real_line_above_the_options() {
        let screen =
            "old output\n\n│ Run `rm -rf /tmp/x`? │\n│                     │\n│ 1. Yes │\n│ 2. No  │\n";
        assert_eq!(detect(screen).unwrap().question, "Run `rm -rf /tmp/x`?");
    }
}
