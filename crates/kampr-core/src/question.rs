//! What the command is asking — the *second* layer, and never the one that decides a host is
//! waiting.
//!
//! `kampr_fleet::waiting` answers "has it stopped for somebody" from the kernel (probe #334). This answers
//! "and what did it say", which is a guess about English and about whatever the author of a CLI
//! felt like writing. So the ladder only ever *improves* the answer box: recognising the wording
//! turns a text field into two buttons, and failing to recognise it turns two buttons back into a
//! text field. There is no rung on which failing to match makes a waiting host look like a
//! working one.
//!
//! The strongest signal here is not a pattern at all. A prompt is text the command wrote and did
//! **not** terminate with a newline, and a supervisor that owns the pty knows exactly which bytes
//! those are — no CLI has to cooperate for that to be true.

use crate::prompt::{is_consecutive_from_one, numbered_run, strip_ansi};
use crate::wire::PendingOption;
use serde::{Deserialize, Serialize};

/// How far above the prompt to look for a menu it is asking about.
const MENU_LOOKBACK: usize = 12;

/// The longest token inside a bracket that may still be a yes/no word. `[Y/n]`, `(yes/no)`,
/// `[y,n,a]` — anything wordier is prose that happens to contain a slash.
const MAX_CHOICE: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum Shape {
    /// The prompt declared its own choices, and sometimes its own default by capitalising one.
    Confirm {
        options: Vec<PendingOption>,
        default_key: Option<String>,
    },
    /// A menu above the prompt, keys `1..n`.
    Numbered { options: Vec<PendingOption> },
    /// The terminal stopped echoing while still in canonical mode (probe #340). Never render it,
    /// never keep it, never log it.
    Secret,
    /// The command has taken the whole screen — `vim`, `less`, anything in raw mode. Its
    /// unterminated tail is not a prompt and must not be shown as one; the pane is the interface.
    Screen,
    /// Nothing matched. The operator reads the tail and types a reply — always available, and the
    /// reason no pattern here is load-bearing.
    Free,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// The unterminated text the command left on the last line. Empty is legitimate: a command
    /// can wait having said nothing since its last newline.
    pub prompt: String,
    /// The completed lines above it, oldest first, for a reader who needs the context.
    pub context: Vec<String>,
    pub shape: Shape,
    /// **The kernel did not say this host was waiting — the screen did.**
    ///
    /// Set only where `/proc` is closed to the node, which is every command that changes user
    /// (probe #332). The evidence is then: the output has stopped, the last line was never
    /// terminated, and it parses as a question. That is enough to be worth showing and not enough
    /// to be silent about, so it travels rather than being flattened into the measured case.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inferred: bool,
}

/// What the pty's line discipline says about what the command is doing with it.
///
/// **Both bits, never one.** `vim` and `less` turn ECHO off exactly as a password prompt does; what
/// separates them is that a prompt is still line-based and a full-screen program is not (probe
/// #340). Reading ECHO alone marks every full-screen program a fleet run starts as a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub echo: bool,
    pub canonical: bool,
}

impl Default for Mode {
    /// What an ordinary pty looks like, and what to assume when the bits cannot be read at all.
    fn default() -> Self {
        Self {
            echo: true,
            canonical: true,
        }
    }
}

impl Mode {
    pub fn asking_for_a_secret(&self) -> bool {
        !self.echo && self.canonical
    }

    pub fn owns_the_screen(&self) -> bool {
        !self.canonical
    }
}

impl Question {
    pub fn options(&self) -> &[PendingOption] {
        match &self.shape {
            Shape::Confirm { options, .. } | Shape::Numbered { options } => options,
            Shape::Secret | Shape::Screen | Shape::Free => &[],
        }
    }

    /// Whether the answer must never be echoed back or retained.
    pub fn secret(&self) -> bool {
        matches!(self.shape, Shape::Secret)
    }

    /// Whether the command has the whole screen — its tail is not a prompt and must not be drawn
    /// as one.
    pub fn owns_the_screen(&self) -> bool {
        matches!(self.shape, Shape::Screen)
    }

    /// Whether the *text alone* is enough to call this a question.
    ///
    /// `Confirm` and `Numbered` only. `Free` is deliberately excluded: it matches any program that
    /// pauses mid-line, which is every progress line ever printed, and inferring a question from
    /// one would put half a fleet on the board asking nothing.
    pub fn reads_as_a_question(&self) -> bool {
        matches!(self.shape, Shape::Confirm { .. } | Shape::Numbered { .. })
    }

    pub fn inferred(mut self) -> Self {
        self.inferred = true;
        self
    }
}

/// Reads the question, given what the supervisor already knows for certain.
///
/// `tail` is the unterminated text since the last newline; `completed` the whole lines above it,
/// oldest first. `mode` comes from the supervisor's own pty and is only meaningful there — on a
/// pane with a shell on it ECHO is already off before anything asks for a secret (probe #333).
pub fn read(tail: &str, completed: &[String], mode: Mode, context_rows: usize) -> Question {
    let prompt = strip_ansi(tail).trim_end_matches(['\r', '\u{0}']).to_string();
    let context: Vec<String> = completed
        .iter()
        .rev()
        .take(context_rows)
        .rev()
        .map(|l| strip_ansi(l).trim_end().to_string())
        .collect();

    let shape = if mode.asking_for_a_secret() {
        Shape::Secret
    } else if mode.owns_the_screen() {
        Shape::Screen
    } else if let Some(shape) = confirm(&prompt) {
        shape
    } else if let Some(shape) = menu_above(completed) {
        shape
    } else {
        Shape::Free
    };

    Question {
        prompt: prompt.trim_end().to_string(),
        context,
        shape,
        inferred: false,
    }
}

/// `:: Proceed with installation? [Y/n]` → two options, defaulting to yes.
///
/// Anchored at the **end** of the prompt on purpose. `Downloading [1/5] linux-firmware` has a
/// bracket with a slash in it too, and the thing that separates it from a question is that a
/// question has nothing after its brackets but whitespace and a colon.
fn confirm(prompt: &str) -> Option<Shape> {
    // A question mark after the brackets is ordinary — `Are you sure (yes/no)?` — and so is the
    // colon a CLI uses instead of one. What may *not* follow is a word.
    let trimmed = prompt
        .trim_end()
        .trim_end_matches([':', '>', '?', '.', ' '])
        .trim_end();
    let close = trimmed.chars().last().and_then(|c| match c {
        ']' => Some('['),
        ')' => Some('('),
        _ => None,
    })?;
    let open = trimmed.rfind(close)?;
    let inside = &trimmed[open + 1..trimmed.len() - 1];

    let parts: Vec<&str> = inside
        .split(['/', ','])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 2 || parts.len() > 4 {
        return None;
    }
    if !parts
        .iter()
        .all(|p| p.len() <= MAX_CHOICE && !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return None;
    }

    let mut options = Vec::new();
    let mut default_key = None;
    for part in &parts {
        let key = part.to_ascii_lowercase().chars().next()?.to_string();
        if part.chars().all(|c| c.is_ascii_uppercase())
            && parts
                .iter()
                .any(|o| o != part && o.chars().any(|c| c.is_ascii_lowercase()))
        {
            // Exactly the `[Y/n]` convention: one choice shouted, the others not.
            if default_key.is_some() {
                default_key = None;
                break;
            }
            default_key = Some(key.clone());
        }
        if options.iter().any(|o: &PendingOption| o.key == key) {
            return None;
        }
        options.push(PendingOption {
            key,
            label: part.to_string(),
        });
    }

    Some(Shape::Confirm { options, default_key })
}

fn menu_above(completed: &[String]) -> Option<Shape> {
    let window: Vec<String> = completed
        .iter()
        .rev()
        .take(MENU_LOOKBACK)
        .rev()
        .map(|l| strip_ansi(l))
        .collect();
    let mut options: Vec<PendingOption> = Vec::new();
    for line in window.iter().rev() {
        let run = numbered_run(line);
        if run.is_empty() {
            if !options.is_empty() {
                break;
            }
            continue;
        }
        let mut merged = run;
        merged.append(&mut options);
        options = merged;
        if is_consecutive_from_one(&options) {
            return Some(Shape::Numbered { options });
        }
    }
    is_consecutive_from_one(&options).then_some(Shape::Numbered { options })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: Mode = Mode {
        echo: false,
        canonical: true,
    };
    const FULL_SCREEN: Mode = Mode {
        echo: false,
        canonical: false,
    };

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn pacmans_real_prompt_offers_yes_and_no_and_knows_yes_is_the_default() {
        // Byte for byte what probe #336 read off the pty, cursor-show escape and all.
        let q = read(
            ":: Proceed with installation? [Y/n] \u{1b}[?25h",
            &lines("Total Installed Size:  9.69 MiB\nNet Upgrade Size:      0.00 MiB"),
            Mode::default(),
            4,
        );
        assert_eq!(q.prompt, ":: Proceed with installation? [Y/n]");
        assert_eq!(
            q.shape,
            Shape::Confirm {
                options: vec![
                    PendingOption {
                        key: "y".into(),
                        label: "Y".into()
                    },
                    PendingOption {
                        key: "n".into(),
                        label: "n".into()
                    },
                ],
                default_key: Some("y".into()),
            }
        );
    }

    #[test]
    fn a_shouted_no_is_the_default_instead() {
        let q = read("Remove kdelibs4support-git? [y/N] ", &[], Mode::default(), 0);
        let Shape::Confirm { default_key, .. } = q.shape else {
            panic!("expected a confirm, got {:?}", q.shape);
        };
        assert_eq!(default_key.as_deref(), Some("n"));
    }

    #[test]
    fn all_lowercase_declares_options_but_no_default() {
        let q = read("Continue? [y/n] ", &[], Mode::default(), 0);
        let Shape::Confirm { options, default_key } = q.shape else {
            panic!("expected a confirm");
        };
        assert_eq!(options.len(), 2);
        assert_eq!(default_key, None, "the prompt did not name one");
    }

    #[test]
    fn a_word_pair_is_a_confirm_too() {
        let q = read("Are you sure (yes/no)? ", &[], Mode::default(), 0);
        assert!(matches!(q.shape, Shape::Confirm { .. }));
    }

    #[test]
    fn a_progress_counter_is_not_a_question() {
        // The false positive that would turn every download into a prompt. `[1/5]` has a bracket
        // and a slash and is not asking anything, and there is text after it.
        assert_eq!(
            read("Downloading [1/5] linux-firmware", &[], Mode::default(), 0).shape,
            Shape::Free
        );
        assert_eq!(
            read("(2/17) upgrading bash", &[], Mode::default(), 0).shape,
            Shape::Free
        );
    }

    #[test]
    fn a_path_with_a_slash_in_brackets_is_not_a_question() {
        assert_eq!(
            read(
                "error: could not open [/var/lib/pacman/db.lck]",
                &[],
                Mode::default(),
                0
            )
            .shape,
            Shape::Free
        );
    }

    #[test]
    fn two_shouted_choices_mean_the_prompt_named_no_default() {
        // `[Y/N]` is a CLI that shouted both. Guessing either one would be picking for the
        // operator, so it declares options and no default.
        let q = read("Overwrite? [Y/N] ", &[], Mode::default(), 0);
        let Shape::Confirm { options, default_key } = q.shape else {
            panic!("expected a confirm");
        };
        assert_eq!(options.len(), 2);
        assert_eq!(default_key, None);
    }

    #[test]
    fn a_password_prompt_is_secret_whatever_it_says() {
        // #337: ECHO off with the job parked in read(2). The wording is irrelevant and must not
        // be trusted — a prompt in another language is still a secret.
        let q = read("Contrasenya: ", &[], SECRET, 0);
        assert_eq!(q.shape, Shape::Secret);
        assert!(q.secret());
        assert!(q.options().is_empty());
    }

    #[test]
    fn echo_off_beats_a_confirm_that_would_otherwise_match() {
        let q = read("Unlock? [Y/n] ", &[], SECRET, 0);
        assert_eq!(q.shape, Shape::Secret, "never offer buttons for a secret");
    }

    #[test]
    fn only_a_declared_shape_reads_as_a_question_on_its_text_alone() {
        // The guard on the inferred rung. `Free` matches any program that pauses mid-line — every
        // progress line ever printed — so inferring a question from one would put half a fleet on
        // the board asking nothing.
        assert!(read(":: Proceed? [Y/n] ", &[], Mode::default(), 0).reads_as_a_question());
        assert!(!read("linking target/debug/kampr ", &[], Mode::default(), 0).reads_as_a_question());
        assert!(!read("Password: ", &[], SECRET, 0).reads_as_a_question());
        assert!(!read("~", &[], FULL_SCREEN, 0).reads_as_a_question());
    }

    #[test]
    fn an_inferred_question_says_so_and_an_ordinary_one_does_not() {
        let measured = read(":: Proceed? [Y/n] ", &[], Mode::default(), 0);
        assert!(!measured.inferred);
        assert!(measured.clone().inferred().inferred);
    }

    #[test]
    fn a_full_screen_program_is_not_a_password_prompt() {
        // Probe #340: `vim` and `less` turn ECHO off exactly as `getpass` does. Testing ECHO alone
        // called every one of them a secret — and hid a screen the operator needed to see.
        let q = read("~", &lines("\"file\" 3L, 20B"), FULL_SCREEN, 2);
        assert_eq!(q.shape, Shape::Screen);
        assert!(!q.secret());
        assert!(q.options().is_empty());
    }

    #[test]
    fn a_full_screen_program_does_not_get_buttons_from_text_that_looks_like_a_prompt() {
        // A pager showing a file that happens to contain `[Y/n]` is not asking anything.
        let q = read("Continue? [Y/n] ", &[], FULL_SCREEN, 0);
        assert_eq!(q.shape, Shape::Screen);
    }

    #[test]
    fn a_menu_on_one_line_above_the_prompt_is_read_as_the_options() {
        let q = read(
            "Enter a number (default=1): ",
            &lines(
                ":: There are 3 providers available for jre-openjdk:\n:: Repository extra\n   1) jdk-openjdk  2) jre-openjdk  3) jre21-openjdk",
            ),
            Mode::default(),
            4,
        );
        let Shape::Numbered { options } = q.shape else {
            panic!("expected a menu, got {:?}", q.shape);
        };
        assert_eq!(options.len(), 3);
        assert_eq!(options[1].label, "jre-openjdk");
    }

    #[test]
    fn a_prompt_with_nothing_recognisable_still_produces_an_answerable_question() {
        // The rung that has to always exist: the fallback is a text box, never a shrug.
        let q = read("what now> ", &lines("something happened"), Mode::default(), 3);
        assert_eq!(q.shape, Shape::Free);
        assert_eq!(q.prompt, "what now>");
        assert_eq!(q.context, vec!["something happened".to_string()]);
    }

    #[test]
    fn a_command_that_waits_having_said_nothing_is_still_a_question() {
        // `cat` with no prompt at all (#335). Empty is not an error and not a reason to hide it.
        let q = read("", &lines("reading from stdin"), Mode::default(), 3);
        assert_eq!(q.shape, Shape::Free);
        assert_eq!(q.prompt, "");
    }

    #[test]
    fn context_is_trimmed_to_the_rows_asked_for_and_stays_oldest_first() {
        let q = read("? ", &lines("one\ntwo\nthree\nfour"), Mode::default(), 2);
        assert_eq!(q.context, vec!["three".to_string(), "four".to_string()]);
    }
}
