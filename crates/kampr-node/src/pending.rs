use kampr_herdr::Herdr;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Option_ {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pending {
    pub question: String,
    pub options: Vec<Option_>,
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
    let options: Vec<(usize, Option_)> = cleaned
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

    let question = cleaned[..chosen[0].0]
        .iter()
        .rev()
        .find(|l| !l.is_empty() && l.chars().count() > 2 && numbered(l).is_none())?
        .clone();
    Some(Pending {
        question,
        options: chosen.into_iter().map(|(_, o)| o).collect(),
    })
}

/// Strips the box a TUI draws around a prompt, so the text inside can be read like any other
/// line. Herdr's `strip_ansi` removes the colour, not the border glyphs.
fn unbox(line: &str) -> String {
    line.trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '│' | '┃' | '║' | '|' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘'
            ) || matches!(c, '─' | '━' | '═' | '-' | '·' | '⎯')
        })
        .trim()
        .to_string()
}

fn numbered(line: &str) -> Option<Option_> {
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
    Some(Option_ {
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

    #[test]
    fn a_claude_permission_prompt_is_read_off_the_screen() {
        let p = detect(CLAUDE).unwrap();
        assert_eq!(p.question, "Do you want to make this edit to config.toml?");
        assert_eq!(p.options.len(), 3);
        assert_eq!(
            p.options[0],
            Option_ {
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
