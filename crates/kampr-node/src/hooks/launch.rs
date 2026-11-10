use super::install::quoted;
use kampr_proc::Table;
use std::path::Path;

const FLAG: &str = "--settings";

pub fn args(kind: &str, args: Vec<String>, settings: Option<&str>) -> Vec<String> {
    let mut args = args;
    if let Some(settings) = settings.filter(|_| kind == "claude") {
        args.extend([FLAG.to_string(), settings.to_string()]);
    }
    args
}

pub fn argv(argv: Vec<String>, settings: Option<&str>) -> Vec<String> {
    let kind = argv.first().map(|program| name(program)).unwrap_or_default();
    args(&kind, argv, settings)
}

// Only a line that starts by running claude is given them, and right after that word, where
// they cannot be read as part of whatever the rest of the line goes on to do.
pub fn line(line: &str, settings: Option<&str>) -> String {
    let trimmed = line.trim_start();
    let (program, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
    match settings {
        Some(settings) if name(program) == "claude" => {
            format!("{program} {FLAG} {} {rest}", quoted(settings))
                .trim_end()
                .to_string()
        }
        _ => line.to_string(),
    }
}

pub fn started_with(pid: u32, settings: &str) -> bool {
    kampr_proc::here()
        .argv(pid)
        .is_some_and(|args| args.windows(2).any(|pair| pair[0] == FLAG && pair[1] == settings))
}

fn name(program: &str) -> String {
    Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_claude_is_given_the_settings_and_only_when_there_are_any() {
        let s = Some(r#"{"hooks":{"Stop":"it's"}}"#);
        let own = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            args("claude", own(&["--resume"]), s),
            own(&["--resume", "--settings", r#"{"hooks":{"Stop":"it's"}}"#])
        );
        assert_eq!(args("codex", own(&["--resume"]), s), own(&["--resume"]));
        assert_eq!(args("claude", own(&["--resume"]), None), own(&["--resume"]));
        assert_eq!(
            argv(own(&["/opt/bin/claude", "-c"]), s),
            own(&[
                "/opt/bin/claude",
                "-c",
                "--settings",
                r#"{"hooks":{"Stop":"it's"}}"#
            ])
        );
        assert_eq!(argv(own(&["env", "claude"]), s), own(&["env", "claude"]));
    }

    #[test]
    fn a_command_line_gets_them_quoted_right_after_the_word_that_runs_claude() {
        let s = Some(r#"{"a":"it's; $HOME"}"#);
        assert_eq!(
            line("claude --resume; echo done", s),
            r#"claude --settings '{"a":"it'\''s; $HOME"}' --resume; echo done"#
        );
        assert_eq!(
            line(" /x/claude", s),
            r#"/x/claude --settings '{"a":"it'\''s; $HOME"}'"#
        );
        assert_eq!(line("claudette run", s), "claudette run");
        assert_eq!(line("echo claude", s), "echo claude");
        assert_eq!(line("claude -c", None), "claude -c");
    }
}
