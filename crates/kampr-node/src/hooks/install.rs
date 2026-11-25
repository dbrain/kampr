use super::ordered::Json;
use kampr_core::wire::{HooksScope, HooksState};
use std::path::{Path, PathBuf};

const TOOL: Option<&str> = Some("*");

const EVENTS: &[(&str, Option<&str>, Option<u64>)] = &[
    ("PermissionRequest", TOOL, Some(3600)),
    ("PostToolUse", TOOL, None),
    ("PostToolUseFailure", TOOL, None),
    ("PermissionDenied", TOOL, None),
    ("UserPromptSubmit", None, Some(5)),
    ("MessageDisplay", None, Some(5)),
    ("Elicitation", None, Some(5)),
    ("ElicitationResult", None, Some(5)),
    ("Stop", None, Some(10)),
    ("StopFailure", None, Some(10)),
    ("SessionEnd", None, Some(10)),
];

pub struct Files {
    pub user: PathBuf,
    pub project: Option<PathBuf>,
    mine: Mine,
}

impl Files {
    pub fn new(home: &Path, project: Option<&Path>) -> Self {
        Self {
            user: home.join(".claude").join("settings.json"),
            project: project.map(|p| p.join(".claude").join("settings.local.json")),
            mine: Mine { command: command() },
        }
    }

    pub fn status(&self) -> (HooksState, Option<HooksScope>) {
        let found: Vec<(HooksScope, Vec<Entry>)> = [
            (HooksScope::User, Some(&self.user)),
            (HooksScope::Project, self.project.as_ref()),
        ]
        .into_iter()
        .filter_map(|(scope, path)| {
            let doc = read(path?).ok().flatten()?;
            Some((scope, entries(&doc)))
        })
        .collect();
        // A Kampr entry in any node's form is current: comparing the exact command is what made
        // two nodes sharing one settings.json report each other out of date.
        match found.iter().find(|(_, e)| !e.is_empty()) {
            Some((scope, _)) => (HooksState::Current, Some(*scope)),
            None => (HooksState::Absent, None),
        }
    }

    pub fn install(&self, scope: HooksScope) -> Result<(), String> {
        let (target, other) = match scope {
            HooksScope::User => (Some(&self.user), self.project.as_ref()),
            HooksScope::Project => (self.project.as_ref(), Some(&self.user)),
        };
        let target = target.ok_or("this pane has no working directory to install the hooks under")?;
        let mut doc = read(target)?.unwrap_or(Json::Object(Vec::new()));
        if !matches!(doc, Json::Object(_)) {
            return Err(format!(
                "{} does not hold a JSON object, so Kampr left it as it is",
                target.display()
            ));
        }
        if doc.get("hooks").is_some_and(|h| !matches!(h, Json::Object(_))) {
            return Err(format!(
                "{}'s hooks are not a JSON object, so Kampr left it as it is",
                target.display()
            ));
        }
        strip(&mut doc, false);
        if doc.get("hooks").is_none()
            && let Json::Object(entries) = &mut doc
        {
            entries.push(("hooks".into(), Json::Object(Vec::new())));
        }
        let Some(Json::Object(hooks)) = doc.get_mut("hooks") else {
            return Err("unreachable: hooks was just made an object".into());
        };
        hooks.retain(|(event, groups)| {
            EVENTS.iter().any(|(e, ..)| e == event) || !matches!(groups, Json::Array(g) if g.is_empty())
        });
        for (event, matcher, timeout) in EVENTS {
            let group = group(&self.mine.command, *matcher, *timeout);
            match hooks.iter_mut().find(|(e, _)| e == event) {
                Some((_, Json::Array(groups))) => groups.push(group),
                Some((_, other)) => *other = Json::Array(vec![group]),
                None => hooks.push((event.to_string(), Json::Array(vec![group]))),
            }
        }
        write(target, &doc)?;
        if let Some(other) = other
            && let Ok(Some(mut doc)) = read(other)
            && strip(&mut doc, true)
        {
            write(other, &doc)?;
        }
        Ok(())
    }
}

// Byte for byte the commands an install writes, so a Claude started with these that also has them
// installed runs each hook once (#641).
pub fn settings(command: &str) -> String {
    let hooks = EVENTS
        .iter()
        .map(|(event, matcher, timeout)| {
            (
                event.to_string(),
                Json::Array(vec![group(command, *matcher, *timeout)]),
            )
        })
        .collect();
    let doc = Json::Object(vec![("hooks".into(), Json::Object(hooks))]);
    serde_json::to_string(&doc).unwrap_or_default()
}

// The command every install writes, named by the environment the node puts in a pane it launches
// rather than by a path, so a ~/.claude shared between nodes holds one command for all of them and
// stops reading each other out of date. A shell runs it, and a Claude with no hook environment —
// one not launched by a node — must say nothing on the operator's screen (#630). Not `exec`: an
// older kampr that has no `hook` exits 2, which Claude reads as a decision, and only a shell still
// standing can turn that into `exit 0`.
pub fn command() -> String {
    concat!(
        r#"[ -n "${KAMPR_BIN:-}" ] && [ -x "${KAMPR_BIN}" ] && "#,
        r#"[ -n "${KAMPR_HOOK_SOCKET:-}" ] && "#,
        r#""${KAMPR_BIN}" hook --socket "${KAMPR_HOOK_SOCKET}" 2>/dev/null; exit 0"#,
    )
    .to_string()
}

pub(super) fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

// A Kampr hook entry, in any node's form: the current one names the hook by the environment it
// reads, an older one names it by a path. Every form runs `kampr hook --socket`, and nothing else
// on the settings file does — so that is what marks an entry as ours, and any of them reads as
// current rather than out of date.
fn ours(command: &str) -> bool {
    command.contains(" hook --socket ")
}

struct Mine {
    command: String,
}

fn group(command: &str, matcher: Option<&str>, timeout: Option<u64>) -> Json {
    let mut hook = vec![
        ("type".to_string(), Json::String("command".into())),
        ("command".to_string(), Json::String(command.into())),
    ];
    if let Some(timeout) = timeout {
        hook.push(("timeout".into(), Json::Number(timeout.into())));
    }
    let mut group = Vec::new();
    if let Some(matcher) = matcher {
        group.push(("matcher".to_string(), Json::String(matcher.into())));
    }
    group.push(("hooks".into(), Json::Array(vec![Json::Object(hook)])));
    Json::Object(group)
}

type Entry = (String, Option<String>, String, Option<String>);

fn entries(doc: &Json) -> Vec<Entry> {
    let mut out = Vec::new();
    let Some(Json::Object(hooks)) = doc.get("hooks") else {
        return out;
    };
    for (event, groups) in hooks {
        let Json::Array(groups) = groups else { continue };
        for group in groups {
            let matcher = group.get("matcher").and_then(Json::as_str).map(str::to_string);
            let Some(Json::Array(hooks)) = group.get("hooks") else {
                continue;
            };
            for hook in hooks {
                let Some(command) = hook.get("command").and_then(Json::as_str) else {
                    continue;
                };
                if ours(command) {
                    let timeout = match hook.get("timeout") {
                        Some(Json::Number(n)) => Some(n.to_string()),
                        _ => None,
                    };
                    out.push((event.clone(), matcher.clone(), command.to_string(), timeout));
                }
            }
        }
    }
    out.sort();
    out
}

fn strip(doc: &mut Json, prune: bool) -> bool {
    let mut changed = false;
    let Some(Json::Object(hooks)) = doc.get_mut("hooks") else {
        return false;
    };
    for (_, groups) in hooks.iter_mut() {
        let Json::Array(groups) = groups else { continue };
        groups.retain_mut(|group| {
            let Some(Json::Array(inner)) = group.get_mut("hooks") else {
                return true;
            };
            let before = inner.len();
            inner.retain(|hook| !hook.get("command").and_then(Json::as_str).is_some_and(ours));
            changed |= inner.len() != before;
            !(inner.is_empty() && before > 0)
        });
    }
    if prune && changed {
        hooks.retain(|(_, groups)| !matches!(groups, Json::Array(g) if g.is_empty()));
        if hooks.is_empty() {
            doc.remove("hooks");
        }
    }
    changed
}

fn read(path: &Path) -> Result<Option<Json>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} could not be read: {e}", path.display())),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}), so Kampr left it as it is",
            path.display()
        )
    })
}

fn write(path: &Path, doc: &Json) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("{} could not be written: {e}", path.display());
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no directory", path.display()))?;
    std::fs::create_dir_all(dir).map_err(failed)?;
    let mut text = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    text.push('\n');
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let staged = dir.join(format!(".{name}.kampr"));
    std::fs::write(&staged, text).map_err(failed)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&staged, meta.permissions());
    }
    std::fs::rename(&staged, path).map_err(failed)
}
