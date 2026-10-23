use crate::dirs::Dirs;
use anyhow::{Context, Result, bail};
use kampr_keep::{Ending, Launch, SessionDir, Spec};
use kampr_native::{AttachOptions, Ended, NativeConfig};
use kampr_node::{Config, ConfigError};
use std::path::PathBuf;

pub struct New {
    pub detached: bool,
    pub cwd: Option<PathBuf>,
    pub label: Option<String>,
    pub argv: Vec<String>,
}

/// A machine with no `kampr init` still keeps sessions, where `kampr-keep` puts them by default.
fn configured(dirs: &Dirs) -> Result<NativeConfig> {
    match Config::load(&dirs.config()) {
        Ok(config) => config.native.resolve(),
        Err(ConfigError::Missing(_)) => kampr_node::config::Native::default().resolve(),
        Err(e) => Err(e.into()),
    }
}

pub fn new(dirs: &Dirs, new: New) -> Result<()> {
    let native = configured(dirs)?;
    let argv = if new.argv.is_empty() {
        vec![kampr_fleet::env::login_shell()]
    } else {
        new.argv
    };
    let mut spec = Spec::new(argv);
    let cwd = match new.cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()?,
    };
    spec.cwd = Some(cwd.to_string_lossy().into_owned());
    spec.label = new.label;
    if let Ok(ws) = rustix::termios::tcgetwinsize(std::io::stdout())
        && ws.ws_col > 0
        && ws.ws_row > 0
    {
        (spec.cols, spec.rows) = (ws.ws_col, ws.ws_row);
    }
    let launch = native.launch.unwrap_or_else(Launch::auto);
    let session = kampr_keep::spawn(&native.root, spec, launch, &native.keeper)?;
    println!("{}", session.id());
    if new.detached {
        return Ok(());
    }
    attach_to(&session)
}

pub fn attach(dirs: &Dirs, query: &str) -> Result<()> {
    attach_to(&resolve(&configured(dirs)?.root, query)?)
}

fn attach_to(session: &SessionDir) -> Result<()> {
    let ended = kampr_native::attach(session, &AttachOptions::from_env())
        .with_context(|| format!("attaching to {}", session.id()))?;
    let Ended::Session(exit) = ended else {
        return Ok(());
    };
    println!("kampr: {} {}", session.id(), kampr_native::exit_status(&exit));
    let code = match (exit.code, exit.signal) {
        (_, Some(signal)) => 128 + signal,
        (Some(code), None) => code,
        (None, None) => 1,
    };
    std::process::exit(code)
}

fn resolve(root: &std::path::Path, query: &str) -> Result<SessionDir> {
    let sessions = kampr_keep::list(root)?;
    if let Some(exact) = sessions.iter().find(|s| s.id() == query) {
        return Ok(exact.clone());
    }
    let matching: Vec<&SessionDir> = sessions.iter().filter(|s| s.id().starts_with(query)).collect();
    match matching.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no session matches {query:?}"),
        many => bail!(
            "{query:?} matches {} sessions: {}",
            many.len(),
            many.iter().map(|s| s.id()).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub fn ls(dirs: &Dirs) -> Result<()> {
    let sessions = kampr_keep::list(&configured(dirs)?.root)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let rows: Vec<[String; 6]> = sessions
        .iter()
        .filter_map(|s| Some((s, s.spec().ok()?)))
        .map(|(s, spec)| row(s, &spec, now))
        .collect();
    let header = ["ID", "STATE", "SIZE", "HOSTS", "AGE", "COMMAND"].map(String::from);
    let mut widths = header.clone().map(|h| h.len());
    for r in &rows {
        for (w, cell) in widths.iter_mut().zip(r) {
            *w = (*w).max(cell.chars().count());
        }
    }
    for r in std::iter::once(&header).chain(&rows) {
        let mut line = String::new();
        for (i, cell) in r.iter().enumerate() {
            if i + 1 == r.len() {
                line.push_str(cell);
            } else {
                line.push_str(&format!("{cell:<w$}  ", w = widths[i]));
            }
        }
        println!("{}", line.trim_end());
    }
    Ok(())
}

fn row(session: &SessionDir, spec: &Spec, now: u64) -> [String; 6] {
    let state = match session.ending() {
        Some(Ending::Exited(exit)) => kampr_native::exit_status(&exit),
        None if session.alive() => "live".to_string(),
        _ => "gone".to_string(),
    };
    let size = session.size().map_or_else(
        || format!("{}x{}", spec.cols, spec.rows),
        |s| format!("{}x{} {}", s.cols, s.rows, s.holder),
    );
    let hosts = session.hosts();
    let hosts = if hosts.is_empty() {
        "-".to_string()
    } else {
        hosts
            .iter()
            .map(|h| h.herdr_pane.clone().unwrap_or_else(|| format!("pid {}", h.pid)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let command = spec.label.clone().unwrap_or_else(|| spec.argv.join(" "));
    [
        session.id().to_string(),
        state,
        size,
        hosts,
        age(now.saturating_sub(spec.created_unix)),
        command,
    ]
}

fn age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}
