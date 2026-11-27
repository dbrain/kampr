//! The actions a muffit pane offers: the daemon's `commands.list` (wire 3.8) where it has one,
//! else the verbs every 3.x daemon takes.

use kampr_core::conversation::{Action, ArgSpec, Suggestion};
use serde_json::{Value, json};

/// Verbs the pane drives some other way: controls, answers, steers and the catalog itself.
const NOT_ACTIONS: &[&str] = &[
    "models.list",
    "model.swap",
    "thinking.set",
    "mode.set",
    "permissions.set",
    "permission.answer",
    "question.answer",
    "question.withdraw",
    "queue.unqueue",
    "commands.list",
];

/// The verbs a 3.3 daemon takes as actions, under the ids panes have always been sent.
const FALLBACK: &[&str] = &[
    "compact",
    "cost",
    "revert",
    "changesets",
    "rewind",
    "btw",
    "context",
];

/// An action id's verb: the fallback's short ids name their verbs, a catalog id is its verb.
pub fn verb_of(id: &str) -> &str {
    match id {
        "compact" => "context.compact",
        other => other,
    }
}

fn id_of(verb: &str) -> &str {
    match verb {
        "context.compact" => "compact",
        other => other,
    }
}

fn arg(name: &str, label: &str, required: bool) -> ArgSpec {
    ArgSpec {
        name: name.into(),
        label: label.into(),
        required,
        options: Vec::new(),
    }
}

/// The arguments `handle_control` in muffit's `ws.rs` reads for a verb; the catalog names none.
fn args_of(verb: &str) -> Vec<ArgSpec> {
    match verb {
        "rewind" => vec![
            arg("changeset", "Changeset", false),
            arg("to_seq", "Message", false),
        ],
        "fork" => vec![
            arg("changeset", "Changeset", false),
            arg("to_seq", "Message", false),
        ],
        "btw" => vec![arg("question", "Question", true)],
        "context" => vec![arg("strategy", "Strategy", false)],
        "rename" => vec![arg("title", "Title", true)],
        "job.stop" => vec![arg("id", "Job", true)],
        "model.add" => vec![
            arg("name", "Name", true),
            arg("kind", "Kind", true),
            arg("model", "Model", true),
            arg("base_url", "Base URL", false),
            arg("api_key", "API key", false),
        ],
        "mcp.add" | "mcp.remove" | "mcp.enable" | "mcp.disable" | "mcp.approve" => {
            vec![arg("name", "Server", true)]
        }
        _ => Vec::new(),
    }
}

fn label_of(verb: &str) -> Option<&'static str> {
    Some(match verb {
        "context.compact" => "Compact the context",
        "cost" => "Show the cost",
        "revert" => "Revert the last change",
        "changesets" => "List the changesets",
        "rewind" => "Rewind",
        "btw" => "Ask a side question",
        "context" => "Explain the context",
        _ => return None,
    })
}

fn action(verb: &str, summary: Option<&str>) -> Action {
    Action {
        id: id_of(verb).into(),
        label: label_of(verb)
            .map(str::to_string)
            .or_else(|| summary.map(str::to_string))
            .unwrap_or_else(|| verb.to_string()),
        detail: match verb {
            "rewind" => Some("to a changeset or a message".into()),
            _ => None,
        },
        args: args_of(verb),
        group: None,
        confirm: match verb {
            "rewind" => Some("The work after it is moved to a backup.".into()),
            _ => None,
        },
    }
}

pub fn fallback() -> Vec<Action> {
    FALLBACK.iter().map(|id| action(verb_of(id), None)).collect()
}

/// The actions of a `commands.list` payload, `None` when it lists nothing readable.
pub fn from_catalog(payload: &Value) -> Option<Vec<Action>> {
    let actions: Vec<Action> = payload["commands"]
        .as_array()?
        .iter()
        .filter_map(|c| Some((c["verb"].as_str()?, c["summary"].as_str())))
        .filter(|(verb, _)| !NOT_ACTIONS.contains(verb))
        .map(|(verb, summary)| action(verb, summary))
        .collect();
    (!actions.is_empty()).then_some(actions)
}

pub fn suggestions(actions: &[Action], query: &str) -> Vec<Suggestion> {
    actions
        .iter()
        .filter(|a| a.id.starts_with(query))
        .map(|a| Suggestion {
            id: a.id.clone(),
            label: format!("/{}", a.id),
            detail: Some(a.label.clone()),
        })
        .collect()
}

/// A composer line that is one of the actions, as `/<id> <words>`: its id and arguments, the
/// words going to its first argument.
pub fn slash(actions: &[Action], text: &str) -> Option<(String, Value)> {
    let line = text.trim().strip_prefix('/')?;
    let (id, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let action = actions.iter().find(|a| a.id == id)?;
    let rest = rest.trim();
    let args = match action.args.first() {
        Some(first) if !rest.is_empty() => json!({ first.name.clone(): rest }),
        _ => json!({}),
    };
    Some((action.id.clone(), args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slash_line_names_an_action_and_fills_its_first_argument() {
        let actions = from_catalog(&json!({ "commands": [
            { "verb": "rename", "summary": "rename the session" },
            { "verb": "context.compact", "summary": "compact" },
            { "verb": "model.swap", "summary": "switch" }
        ] }))
        .unwrap();
        assert_eq!(
            actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["rename", "compact"]
        );
        assert_eq!(
            slash(&actions, "/rename a title"),
            Some(("rename".into(), json!({ "title": "a title" })))
        );
        assert_eq!(slash(&actions, "/compact"), Some(("compact".into(), json!({}))));
        assert_eq!(slash(&actions, "/model.swap x"), None);
        assert_eq!(slash(&actions, "rename it"), None);
    }
}
