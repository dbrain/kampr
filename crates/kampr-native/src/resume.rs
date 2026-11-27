//! `OSC 7577` in a native session (docs/25): the token each session's first process is given, the
//! reports read against it, the record store they land in and what a restart does with it.

mod intake;
mod restore;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use intake::Intake;
pub use restore::{Resumable, offered, start};
pub(crate) use restore::{forget, launch_argv, listed, take};

use kampr_core::resume::{TOKEN_ENV, Token};
use kampr_keep::Spec;
use std::path::{Path, PathBuf};

pub const TOKEN_FILE: &str = "resume.token";

/// `[native] resume`. `Off` stores nothing and gives no session a token, so no program is told it
/// may report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Ask,
    Auto,
    Off,
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, String> {
        match word {
            "ask" => Ok(Self::Ask),
            "auto" => Ok(Self::Auto),
            "off" => Ok(Self::Off),
            other => Err(format!(
                "resume must be \"ask\", \"auto\" or \"off\", not {other:?}"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resume {
    pub mode: Mode,
    /// Not the session root, which is tmpfs: a record is worth something only if it outlives a boot.
    pub dir: PathBuf,
}

impl Resume {
    pub(crate) fn active(&self) -> bool {
        self.mode != Mode::Off && stores_for(rustix::process::geteuid())
    }
}

/// Nothing is stored for root: a resumed command would run with every privilege the machine
/// has, from a file any process that reached the token could have written.
fn stores_for(uid: rustix::process::Uid) -> bool {
    !uid.is_root()
}

/// Gives a new session its token, in the child's environment now and in `resume.token` once the
/// keeper has made the directory. A Docker shell gets none: the command it would resume ran in a
/// container and would be run on the host.
pub(crate) fn issue(resume: Option<&Resume>, spec: &mut Spec) -> Option<Token> {
    resume.filter(|r| r.active())?;
    if spec.container.is_some() {
        return None;
    }
    let token = Token::generate().ok()?;
    spec.env.retain(|(have, _)| have != TOKEN_ENV);
    spec.env.push((TOKEN_ENV.to_string(), token.as_str().to_string()));
    Some(token)
}

pub(crate) fn write_token(session: &Path, token: &Token) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(session.join(TOKEN_FILE))?
        .write_all(token.as_str().as_bytes())
}

pub(crate) fn read_token(session: &Path) -> Option<Token> {
    Token::parse(std::fs::read_to_string(session.join(TOKEN_FILE)).ok()?.trim())
}
