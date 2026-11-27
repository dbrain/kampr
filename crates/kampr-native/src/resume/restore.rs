use super::Resume;
use super::store::{Owner, Store};
use kampr_core::resume::Command;
use std::collections::HashSet;
use std::path::Path;

/// A record whose session is gone, as the herd lists it (docs/25): enough to tell what would be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resumable {
    /// `<session id>/<pid>`, which a `resume` names.
    pub id: String,
    pub label: Option<String>,
    pub command: Command,
}

/// Records whose session no longer exists and that no resume was tried on, their `cwd` still there.
pub(crate) fn listed(resume: &Resume, live: &HashSet<String>) -> Vec<Resumable> {
    if !resume.active() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (session, file) in Store::new(resume.dir.clone()).all() {
        if live.contains(&session) {
            continue;
        }
        for owner in file.owners.iter().filter(|o| !o.tried && cwd_exists(&o.command)) {
            out.push(Resumable {
                id: format!("{session}/{}", owner.pid),
                label: file.label.clone(),
                command: owner.command.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn cwd_exists(command: &Command) -> bool {
    command.cwd.as_deref().is_none_or(|cwd| Path::new(cwd).is_dir())
}

/// The record `id` names, marked tried before it is handed back and gone from the store once the
/// caller has it: a resume that fails is not offered again.
pub(crate) fn take(resume: &Resume, live: &HashSet<String>, id: &str) -> Option<(Option<String>, Owner)> {
    if !resume.active() {
        return None;
    }
    let (session, pid) = id.split_once('/')?;
    let pid: u32 = pid.parse().ok()?;
    if live.contains(session) {
        return None;
    }
    let store = Store::new(resume.dir.clone());
    let taken = store
        .update(session, |file| {
            let owner = file
                .owners
                .iter_mut()
                .find(|o| o.pid == pid && !o.tried && cwd_exists(&o.command))?;
            owner.tried = true;
            let owner = owner.clone();
            Some((file.label.clone(), owner))
        })
        .ok()??;
    // The session is gone, so its replay offset means nothing once its last owner is spent.
    let spent = store.update(session, |file| {
        file.owners.retain(|o| o.pid != pid);
        file.owners.is_empty()
    });
    if spent.unwrap_or(false) {
        store.remove(session);
    }
    Some(taken)
}

pub(crate) fn forget(resume: &Resume, session: &str) {
    Store::new(resume.dir.clone()).remove(session);
}

/// A login shell only gives the program the environment its user would have had; every element of
/// `argv` reaches it as one argument, never as shell text.
pub(crate) fn launch_argv(command: &Command) -> Vec<String> {
    let mut argv = vec![
        "sh".to_string(),
        "-lc".into(),
        "exec \"$@\"".into(),
        "kampr-resume".into(),
    ];
    argv.extend(command.argv.iter().cloned());
    argv
}

fn live_ids(root: &Path) -> HashSet<String> {
    kampr_keep::list(root)
        .map(|sessions| sessions.iter().map(|s| s.id().to_string()).collect())
        .unwrap_or_default()
}

/// Every record `kampr attach` could offer for a session id or a prefix of one, whose keeper
/// under `root` is gone.
pub fn offered(resume: &Resume, root: &Path, session: &str) -> Vec<Resumable> {
    listed(resume, &live_ids(root))
        .into_iter()
        .filter(|r| {
            r.id.split_once('/')
                .is_some_and(|(id, _)| id.starts_with(session))
        })
        .collect()
}

/// Starts record `id` as a new session under `config`, with a token of its own, for a caller
/// with no provider (`kampr attach`). The record is spent from here on whatever becomes of it.
pub fn start(config: &crate::NativeConfig, id: &str) -> anyhow::Result<Option<kampr_keep::SessionDir>> {
    let Some(resume) = config.resume.as_ref() else {
        return Ok(None);
    };
    let Some((label, owner)) = take(resume, &live_ids(&config.root), id) else {
        return Ok(None);
    };
    let mut spec = kampr_keep::Spec::new(launch_argv(&owner.command));
    spec.cwd = owner.command.cwd;
    spec.label = label;
    let token = super::issue(Some(resume), &mut spec);
    let launch = config.launch.unwrap_or_else(kampr_keep::Launch::auto);
    let session = kampr_keep::spawn(&config.root, spec, launch, &config.keeper)?;
    if let Some(token) = token {
        super::write_token(session.path(), &token)?;
    }
    Ok(Some(session))
}
