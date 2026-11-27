use crate::dirs::Dirs;
use anyhow::Result;
use kampr_keep::SessionDir;
use kampr_native::{ResumeMode, resume_offered, resume_start};
use kampr_node::{Config, ConfigError};
use std::io::{BufRead as _, IsTerminal as _, Write as _};

/// A session the machine lost whose program registered how to bring it back (docs/25): started
/// again under `auto`, offered under `ask` to a person at the pane, never under `off`.
pub(super) fn offer(dirs: &Dirs, query: &str) -> Result<Option<SessionDir>> {
    let (settings, state) = match Config::load(&dirs.config()) {
        Ok(config) => {
            let state = config.resolve_state_dir(dirs.state_override());
            (config.native, state)
        }
        Err(ConfigError::Missing(_)) => (kampr_node::config::Native::default(), dirs.state()),
        Err(e) => return Err(e.into()),
    };
    let mut native = settings.resolve()?;
    let resume = settings.resume_in(&state)?;
    native.resume = Some(resume.clone());
    let Some(record) = resume_offered(&resume, &native.root, query).into_iter().next() else {
        return Ok(None);
    };
    if resume.mode == ResumeMode::Ask {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return Ok(None);
        }
        let cwd = record.command.cwd.as_deref().unwrap_or("its home directory");
        eprint!(
            "kampr: session {query} ended with the machine. It can be resumed by running\r\n  {:?}\r\nin {cwd:?}. Resume it? [y/N] ",
            record.command.argv
        );
        std::io::stderr().flush()?;
        let mut answer = String::new();
        stdin.lock().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            return Ok(None);
        }
    }
    resume_start(&native, &record.id)
}
