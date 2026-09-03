//! Whether a command line looks like it is carrying a credential.
//!
//! **This is a blast-radius reduction, not a filter.** It exists because remembering a fleet run
//! writes its argv to disk on the node and renders it in a list on every device the operator owns,
//! and nobody pressed anything to ask for that — the history is automatic. So the automatic half
//! declines the shapes it can recognise, and the operator can delete anything in the book by hand,
//! which is the part that actually holds.
//!
//! What it cannot see is written down and tested rather than claimed:
//! `crates/kampr-node/tests/fixtures/secretish.json` has a `missed` section, and a positional
//! secret (`./deploy hunter2`) is in it. Do not present this as a guarantee to the operator.
//!
//! `dev.kampr.shared.model.secretish` is the Kotlin twin, and it reads the same fixture, because
//! the client's warning and the node's refusal disagreeing is worse than either alone.

/// Names that make the thing beside them a secret. Substring matches on an upper-cased name, so
/// `AWS_SECRET_ACCESS_KEY` is caught by `SECRET` and `GITHUB_TOKEN` by `TOKEN`.
///
/// `KEY` and `AUTH` are deliberately absent as bare words: they would catch `SORT_KEY` and
/// `AUTHOR`, and a rule that fires on a commit author is one the operator learns to tap through.
/// `ACCESS_KEY` is absent for the same reason — `AWS_ACCESS_KEY_ID` is an identifier that ships in
/// config files, and the secret half of that pair says `SECRET` on it.
const WORDS: [&str; 12] = [
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASSPHRASE",
    "APIKEY",
    "API_KEY",
    "CREDENTIAL",
    "PRIVATE_KEY",
    "PRIVKEY",
    "BEARER",
    "AUTH_TOKEN",
];

/// `--password-file prod.pass` names a path, and a path is the *safe* way to pass a secret. Firing
/// on it would make the rule loudest exactly where the operator did the right thing.
const PATHISH: [&str; 2] = ["_FILE", "_PATH"];

fn normalised(name: &str) -> String {
    name.replace('-', "_").to_ascii_uppercase()
}

fn names_a_secret(name: &str) -> bool {
    let upper = normalised(name);
    !PATHISH.iter().any(|tail| upper.ends_with(tail)) && WORDS.iter().any(|word| upper.contains(word))
}

/// A value the shell will substitute is a reference, not a secret: `TOKEN=$CI_TOKEN` writes down
/// the name of an environment variable and nothing else.
fn is_a_secret_value(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('$')
}

fn assignment(token: &str) -> Option<String> {
    let (name, value) = token.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    (is_a_secret_value(value) && names_a_secret(name)).then(|| name.to_string())
}

fn flag_with_value(token: &str) -> Option<String> {
    if !token.starts_with('-') {
        return None;
    }
    let (flag, value) = token.split_once('=')?;
    let name = flag.trim_start_matches('-');
    (is_a_secret_value(value) && names_a_secret(name)).then(|| flag.to_string())
}

/// A long flag whose value is the next word. Long only: `-p` is a port at least as often as a
/// password, and a rule that fires on `ssh -p 2222` is one nobody believes.
fn flag_before_value(token: &str, next: Option<&str>) -> Option<String> {
    let next = next?;
    if !token.starts_with("--") || token.contains('=') || next.starts_with('-') {
        return None;
    }
    let name = token.trim_start_matches('-');
    (is_a_secret_value(next) && names_a_secret(name)).then(|| token.to_string())
}

/// `curl -H "Authorization: Bearer …"`, the one very common shape that carries no `=` at all.
/// Read off the whole argument rather than its words, because the header arrives inside one
/// quoted string and splitting it puts the scheme and the credential in different tokens.
fn header(argument: &str) -> Option<&'static str> {
    const MARKERS: [(&str, &str); 2] = [("authorization:", "Authorization:"), ("bearer ", "Bearer")];
    let lower = argument.to_ascii_lowercase();
    for (needle, said) in MARKERS {
        let Some(at) = lower.find(needle) else {
            continue;
        };
        let rest =
            argument[at + needle.len()..].trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'');
        // A header whose credential is `$TOKEN` names a variable; the secret is in the environment
        // and never in what would be written down.
        if !rest.is_empty() && !rest.contains('$') {
            return Some(said);
        }
    }
    None
}

/// The word that made this command look like it carries a secret, or `None`.
///
/// Words rather than a boolean so a message can name what it saw: "this looks like it carries
/// `TOKEN`" is something an operator can act on, and "this looks secret" is something they argue
/// with.
pub fn secretish(args: &[String]) -> Option<String> {
    for argument in args {
        if let Some(said) = header(argument) {
            return Some(said.to_string());
        }
    }
    // Flattened, so an `sh -c 'TOKEN=abc ./deploy'` — which is the shape this project's own docs
    // tell the operator to use for a pipeline — is read rather than treated as one opaque word.
    let words: Vec<&str> = args.iter().flat_map(|a| a.split_whitespace()).collect();
    for (index, token) in words.iter().enumerate() {
        let found = assignment(token)
            .or_else(|| flag_with_value(token))
            .or_else(|| flag_before_value(token, words.get(index + 1).copied()));
        if found.is_some() {
            return found;
        }
    }
    None
}
