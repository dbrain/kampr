//! Recording which herdr the node will run, at the two moments a login shell is there to ask.
//!
//! `kampr init` and `kampr service install` both run from the operator's own shell, where
//! `~/.local/bin` is on `PATH`; the node they set up runs from a service manager, where it is not.
//! The unit has always pinned `HERDR_SOCKET_PATH` for that reason, and this is the same fact about
//! the other half. It goes in `config.toml` rather than in the unit because config is what every
//! entry point reads — the node, this command, and the plugin dispatcher — and because there is
//! one config and two supervisors.

use anyhow::Result;
use kampr_herdr::locate::{Found, NotFound, Origin, Search, locate};
use kampr_node::Config;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Pin {
    Recorded(PathBuf),
    /// The config already names a path. It is the operator's answer and is never second-guessed.
    Kept(String),
    Missing(NotFound),
}

/// Resolve and write it down. Every path that installs a service goes through here, because the
/// shell that installs one is the only place the answer is available.
pub fn record(config: &mut Config, config_dir: &Path) -> Result<Pin> {
    let pin = resolve(config);
    if let Pin::Recorded(_) = pin {
        config.save(config_dir)?;
    }
    Ok(pin)
}

pub fn resolve(config: &mut Config) -> Pin {
    let found = locate(&config.herdr.binary, &Search::from_env());
    decide(&mut config.herdr.binary, found)
}

fn decide(binary: &mut String, found: Result<Found, NotFound>) -> Pin {
    match found {
        Err(error) => Pin::Missing(error),
        Ok(found) if found.origin == Origin::Configured => Pin::Kept(binary.clone()),
        Ok(found) => {
            *binary = found.path.display().to_string();
            Pin::Recorded(found.path)
        }
    }
}

impl Pin {
    /// One line for the operator, and it says what was written rather than only what was found:
    /// a node that shows a blank grid in every pane is the alternative.
    pub fn note(&self) -> String {
        match self {
            Self::Recorded(path) => format!(
                "{} — recorded in config.toml, because a service manager's PATH is not your shell's",
                path.display()
            ),
            Self::Kept(binary) => format!("{binary} — as config.toml already names it"),
            Self::Missing(error) => format!(
                "{error}. The node will serve the herd and accept input, and every pane will show \
                 a blank grid until it can run one"
            ),
        }
    }

    pub fn found(&self) -> bool {
        !matches!(self, Self::Missing(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(origin: Origin, path: &str) -> Result<Found, NotFound> {
        Ok(Found {
            path: PathBuf::from(path),
            origin,
        })
    }

    #[test]
    fn anything_resolved_from_the_environment_is_written_down_and_a_configured_path_is_left_alone() {
        let cases = [
            (
                "herdr",
                found(Origin::Path, "/home/x/.local/bin/herdr"),
                Pin::Recorded(PathBuf::from("/home/x/.local/bin/herdr")),
                "/home/x/.local/bin/herdr",
            ),
            (
                "herdr",
                found(Origin::BesideKampr, "/opt/kampr/herdr"),
                Pin::Recorded(PathBuf::from("/opt/kampr/herdr")),
                "/opt/kampr/herdr",
            ),
            (
                "/opt/herdr",
                found(Origin::Configured, "/opt/herdr"),
                Pin::Kept("/opt/herdr".into()),
                "/opt/herdr",
            ),
        ];
        for (configured, found, expected, after) in cases {
            let mut binary = configured.to_string();
            assert_eq!(decide(&mut binary, found), expected, "{configured}");
            assert_eq!(binary, after, "{configured}");
        }
    }

    #[test]
    fn a_binary_nobody_can_find_leaves_the_config_alone_and_says_what_it_costs() {
        let mut binary = "herdr".to_string();
        let pin = decide(
            &mut binary,
            Err(NotFound {
                binary: "herdr".into(),
                tried: vec![PathBuf::from("/usr/bin/herdr")],
                explicit: false,
            }),
        );
        assert!(!pin.found());
        assert_eq!(binary, "herdr", "guessing a path that is not there helps nobody");
        assert!(pin.note().contains("blank grid"), "{}", pin.note());
    }
}
