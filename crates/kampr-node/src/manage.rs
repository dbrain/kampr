use kampr_herdr::Herdr;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;

/// Everything you would do at the keyboard. Every field is optional at this layer so an unknown
/// or malformed op is a `bad_request` to one client rather than a decode failure that kills the
/// connection.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ManageOp {
    pub op: String,
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Option<Value>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub ratio: Option<f64>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub layout: Option<Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManageError {
    #[error("unknown op {0}")]
    Unsupported(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0} is not a pane, tab or workspace on this node")]
    UnknownTarget(String),
    #[error("herdr: {0}")]
    Herdr(String),
}

impl ManageError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "unsupported",
            Self::BadRequest(_) => "bad_request",
            Self::UnknownTarget(_) => "unknown_pane",
            Self::Herdr(_) => "herdr_unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Workspace(String),
    Tab(String),
    Pane(String),
}

/// A global id is `<node_id>/<herdr id>`, and herdr's own ids carry their kind: `w3`, `w3:t1`,
/// `w3:p2`. That is what lets one `close` verb reach a pane, a tab or a workspace.
pub fn parse_target(node_id: &str, at: &str) -> Result<Target, ManageError> {
    let local = at
        .strip_prefix(node_id)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| ManageError::UnknownTarget(at.to_string()))?;
    if local.is_empty() {
        return Err(ManageError::UnknownTarget(at.to_string()));
    }
    match local.split_once(':') {
        Some((_, kind)) if kind.starts_with('p') => Ok(Target::Pane(local.to_string())),
        Some((_, kind)) if kind.starts_with('t') => Ok(Target::Tab(local.to_string())),
        Some(_) => Err(ManageError::UnknownTarget(at.to_string())),
        None => Ok(Target::Workspace(local.to_string())),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Managed {
    pub id: Option<String>,
    pub layout: Option<String>,
}

pub struct Manager<'a> {
    pub herdr: &'a Herdr,
    pub node_id: &'a str,
    pub binary: &'a str,
}

impl Manager<'_> {
    pub async fn run(&self, op: &ManageOp) -> Result<Value, ManageError> {
        match op.op.as_str() {
            "workspace.create" => {
                self.call(
                    "workspace.create",
                    json!({ "label": op.label, "cwd": op.cwd, "env": op.env, "focus": false }),
                )
                .await
            }
            "tab.create" => {
                let workspace = self.workspace_of(op).await?;
                self.call(
                    "tab.create",
                    json!({ "workspace_id": workspace, "label": op.label, "cwd": op.cwd, "focus": false }),
                )
                .await
            }
            "pane.split" => {
                let Target::Pane(pane) = self.target(op)? else {
                    return Err(ManageError::BadRequest("pane.split needs a pane".into()));
                };
                let direction = match op.direction.as_deref() {
                    // Herdr's split grammar is exactly two directions (probe #46/#47); a client
                    // asking for "left" or "up" is asking for something that does not exist.
                    Some(d @ ("right" | "down")) => d,
                    other => {
                        return Err(ManageError::BadRequest(format!(
                            "direction must be right or down, not {other:?}"
                        )));
                    }
                };
                self.call(
                    "pane.split",
                    json!({ "target_pane_id": pane, "direction": direction,
                            "ratio": op.ratio, "cwd": op.cwd, "focus": false }),
                )
                .await
            }
            "pane.zoom" => {
                let Target::Pane(pane) = self.target(op)? else {
                    return Err(ManageError::BadRequest("pane.zoom needs a pane".into()));
                };
                let mode = op.mode.as_deref().unwrap_or("toggle");
                if !matches!(mode, "toggle" | "on" | "off") {
                    return Err(ManageError::BadRequest(format!("unknown zoom mode {mode}")));
                }
                self.call("pane.zoom", json!({ "pane_id": pane, "mode": mode }))
                    .await
            }
            "rename" => match self.target(op)? {
                // Only a pane's label is nullable: herdr's tab and workspace rename take a
                // required string, so there is nothing to clear them to.
                Target::Pane(id) => {
                    self.call("pane.rename", json!({ "pane_id": id, "label": op.label }))
                        .await
                }
                Target::Tab(id) => {
                    let label = self.required_label(op)?;
                    self.call("tab.rename", json!({ "tab_id": id, "label": label }))
                        .await
                }
                Target::Workspace(id) => {
                    let label = self.required_label(op)?;
                    self.call("workspace.rename", json!({ "workspace_id": id, "label": label }))
                        .await
                }
            },
            "close" => match self.target(op)? {
                Target::Pane(id) => self.call("pane.close", json!({ "pane_id": id })).await,
                Target::Tab(id) => self.call("tab.close", json!({ "tab_id": id })).await,
                Target::Workspace(id) => self.call("workspace.close", json!({ "workspace_id": id })).await,
            },
            "focus" => match self.target(op)? {
                Target::Pane(id) => self.call("pane.focus", json!({ "pane_id": id })).await,
                Target::Tab(id) => self.call("tab.focus", json!({ "tab_id": id })).await,
                Target::Workspace(id) => self.call("workspace.focus", json!({ "workspace_id": id })).await,
            },
            "agent.start" => {
                let Target::Pane(pane) = self.target(op)? else {
                    return Err(ManageError::BadRequest("agent.start needs a pane".into()));
                };
                let kind = op
                    .kind
                    .as_deref()
                    .ok_or_else(|| ManageError::BadRequest("agent.start needs a kind".into()))?;
                let name = op.name.clone().unwrap_or_else(|| kind.to_string());
                self.call(
                    "agent.start",
                    json!({ "pane_id": pane, "kind": kind, "name": name,
                            "args": op.args.clone().unwrap_or_default() }),
                )
                .await
            }
            "worktree.create" => {
                self.call(
                    "worktree.create",
                    json!({ "cwd": op.cwd, "branch": op.branch, "base": op.base,
                            "label": op.label, "focus": false }),
                )
                .await
            }
            "worktree.open" => {
                self.call(
                    "worktree.open",
                    json!({ "path": op.path, "cwd": op.cwd, "label": op.label, "focus": false }),
                )
                .await
            }
            "layout.export" => {
                let Target::Tab(tab) = self.target(op)? else {
                    return Err(ManageError::BadRequest("layout.export needs a tab".into()));
                };
                self.call("layout.export", json!({ "tab_id": tab })).await
            }
            "layout.apply" => {
                let Target::Tab(tab) = self.target(op)? else {
                    return Err(ManageError::BadRequest("layout.apply needs a tab".into()));
                };
                let root = layout_root(op.layout.as_ref())?;
                self.call(
                    "layout.apply",
                    json!({ "tab_id": tab, "root": root, "focus": false }),
                )
                .await
            }
            "session.create" => self.create_session(op).await,
            "session.stop" => self.stop_session(op).await,
            other => Err(ManageError::Unsupported(other.to_string())),
        }
    }

    fn target(&self, op: &ManageOp) -> Result<Target, ManageError> {
        let at = op
            .at
            .as_deref()
            .ok_or_else(|| ManageError::BadRequest(format!("{} needs an `at`", op.op)))?;
        parse_target(self.node_id, at)
    }

    fn required_label(&self, op: &ManageOp) -> Result<String, ManageError> {
        op.label
            .clone()
            .ok_or_else(|| ManageError::BadRequest("only a pane's label can be cleared".into()))
    }

    /// `tab.create` takes a workspace, and the wire's `at` may name either the workspace or a
    /// tab inside it — a client that has a tab id in hand should not have to strip it.
    async fn workspace_of(&self, op: &ManageOp) -> Result<String, ManageError> {
        match self.target(op)? {
            Target::Workspace(id) => Ok(id),
            Target::Tab(id) | Target::Pane(id) => {
                Ok(id.split_once(':').map(|(w, _)| w.to_string()).unwrap_or(id))
            }
        }
    }

    async fn create_session(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let name = session_name(op)?;
        Command::new(self.binary)
            .args(["server", "--session", &name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| ManageError::Herdr(e.to_string()))?;
        Ok(json!({ "session": name }))
    }

    /// A named session has its own socket, so stopping one is an ordinary `server.stop` addressed
    /// somewhere other than this node's own herdr.
    async fn stop_session(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let name = session_name(op)?;
        let socket = crate::caps::sessions(self.binary)
            .await
            .into_iter()
            .find(|s| s.name == name)
            .and_then(|s| s.socket_path)
            .ok_or_else(|| ManageError::UnknownTarget(name.clone()))?;
        Herdr::new(socket)
            .call::<Value>("server.stop", json!({}))
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))?;
        Ok(json!({ "session": name }))
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, ManageError> {
        self.herdr
            .call::<Value>(method, params)
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))
    }
}

fn session_name(op: &ManageOp) -> Result<String, ManageError> {
    let name = op
        .name
        .as_deref()
        .ok_or_else(|| ManageError::BadRequest("a session op needs a name".into()))?;
    // The name becomes a directory under herdr's config root and reaches a command line, so it is
    // validated here rather than trusted.
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ManageError::BadRequest(format!("unusable session name {name:?}")));
    }
    Ok(name.to_string())
}

/// `layout.export` hands back `{workspace_id, tab_id, root}` and `layout.apply` wants the `root`,
/// so a client that round-trips an export unchanged is accepted as-is.
fn layout_root(layout: Option<&Value>) -> Result<Value, ManageError> {
    let layout = layout.ok_or_else(|| ManageError::BadRequest("layout.apply needs a layout".into()))?;
    let root = layout.get("root").unwrap_or(layout);
    if root.get("type").is_none() {
        return Err(ManageError::BadRequest("a layout needs a typed root node".into()));
    }
    Ok(root.clone())
}

/// The id a creating op produced, dug out of whichever record herdr chose to echo.
pub fn created_id(reply: &Value) -> Option<String> {
    for (record, field) in [
        ("workspace", "workspace_id"),
        ("tab", "tab_id"),
        ("pane", "pane_id"),
        ("root_pane", "pane_id"),
    ] {
        if let Some(id) = reply[record][field].as_str() {
            return Some(id.to_string());
        }
    }
    reply["session"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_carries_its_kind_in_the_id() {
        assert_eq!(
            parse_target("01J", "01J/w3").unwrap(),
            Target::Workspace("w3".into())
        );
        assert_eq!(
            parse_target("01J", "01J/w3:t1").unwrap(),
            Target::Tab("w3:t1".into())
        );
        assert_eq!(
            parse_target("01J", "01J/w3:p2").unwrap(),
            Target::Pane("w3:p2".into())
        );
    }

    #[test]
    fn a_target_on_another_node_is_not_ours_to_act_on() {
        assert!(parse_target("01J", "01K/w3:p2").is_err());
        assert!(parse_target("01J", "w3:p2").is_err());
        assert!(parse_target("01J", "01J/").is_err());
        assert!(parse_target("01J", "01J/w3:x9").is_err());
    }

    #[test]
    fn a_created_id_is_found_wherever_herdr_put_it() {
        assert_eq!(
            created_id(
                &json!({"type":"workspace_created","workspace":{"workspace_id":"w1"},
                               "tab":{"tab_id":"w1:t1"}})
            ),
            Some("w1".into())
        );
        assert_eq!(
            created_id(&json!({"tab":{"tab_id":"w1:t2"}})),
            Some("w1:t2".into())
        );
        assert_eq!(
            created_id(&json!({"pane":{"pane_id":"w1:p3"}})),
            Some("w1:p3".into())
        );
        assert_eq!(created_id(&json!({"session":"agents"})), Some("agents".into()));
        assert_eq!(created_id(&json!({"type":"ok"})), None);
    }

    #[test]
    fn a_layout_is_accepted_as_an_export_or_as_a_bare_root() {
        let export = json!({"workspace_id":"w1","tab_id":"w1:t1","root":{"type":"pane"}});
        assert_eq!(layout_root(Some(&export)).unwrap(), json!({"type":"pane"}));
        assert_eq!(
            layout_root(Some(&json!({"type":"split","direction":"right"}))).unwrap()["type"],
            "split"
        );
        assert!(layout_root(None).is_err());
        assert!(layout_root(Some(&json!({"nope": 1}))).is_err());
    }

    #[test]
    fn a_session_name_that_reaches_a_command_line_is_validated() {
        let named = |n: &str| ManageOp {
            name: Some(n.to_string()),
            ..ManageOp::default()
        };
        assert_eq!(session_name(&named("agents")).unwrap(), "agents");
        assert_eq!(session_name(&named("kp-2_x")).unwrap(), "kp-2_x");
        for bad in ["", "../escape", "a b", "-;rm -rf", &"x".repeat(65)] {
            assert!(session_name(&named(bad)).is_err(), "{bad:?} must be refused");
        }
        assert!(session_name(&ManageOp::default()).is_err());
    }

    #[test]
    fn error_codes_map_onto_the_wire_vocabulary() {
        assert_eq!(ManageError::Unsupported("x".into()).code(), "unsupported");
        assert_eq!(ManageError::BadRequest("x".into()).code(), "bad_request");
        assert_eq!(ManageError::UnknownTarget("x".into()).code(), "unknown_pane");
        assert_eq!(ManageError::Herdr("x".into()).code(), "herdr_unavailable");
    }
}
