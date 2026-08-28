use kampr_core::wire::ErrorCode;
use kampr_herdr::Herdr;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::debug;

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
    #[serde(default)]
    pub cols: Option<u32>,
    #[serde(default)]
    pub rows: Option<u32>,
}

/// The smallest pane `pane.size` will produce.
///
/// A headless resize *persists* after the controller lets go (#219), so a client that fits a pane
/// to its own viewport locks that pane at that size until something else moves it. On a phone that
/// viewport is far narrower than anything a shell is usable at, and the escape hatch would become
/// the thing it exists to undo. 80x24 is the floor every terminal has agreed on for forty years.
pub const MIN_COLS: u32 = 80;
pub const MIN_ROWS: u32 = 24;

/// Well past any real desk — a guard against a typo rather than a considered limit.
pub const MAX_COLS: u32 = 1000;
pub const MAX_ROWS: u32 = 500;

/// The floor and the ceiling, apart from the op so they can be tested without a herdr.
///
/// The floor is the load-bearing one and it is there because of the operator this feature is for:
/// a resize on a headless pane persists after the controller goes (#219), so fitting a pane to a
/// phone's viewport would lock it at phone width for every other client, permanently. Refusing is
/// the whole answer — there is nothing to undo it with except another resize.
pub fn checked_size(cols: u32, rows: u32) -> Result<(u32, u32), ManageError> {
    if cols < MIN_COLS || rows < MIN_ROWS {
        return Err(ManageError::BadRequest(format!(
            "{cols}x{rows} is smaller than {MIN_COLS}x{MIN_ROWS}, and a pane keeps the size it is \
             given — fitting one to a small screen leaves it that narrow for everything else"
        )));
    }
    if cols > MAX_COLS || rows > MAX_ROWS {
        return Err(ManageError::BadRequest(format!(
            "{cols}x{rows} is larger than {MAX_COLS}x{MAX_ROWS}"
        )));
    }
    Ok((cols, rows))
}

/// Sixteen times the worst stop measured in #241, and a thousand times the worst create in #240.
const SESSION_SETTLE: Duration = Duration::from_secs(5);
const SESSION_POLL: Duration = Duration::from_millis(20);
const SESSION_POLL_MAX: Duration = Duration::from_millis(320);

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
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Unsupported(_) => ErrorCode::Unsupported,
            Self::BadRequest(_) => ErrorCode::BadRequest,
            Self::UnknownTarget(_) => ErrorCode::UnknownPane,
            Self::Herdr(_) => ErrorCode::HerdrUnavailable,
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

pub struct Manager<'a> {
    pub herdr: &'a Herdr,
    pub node_id: &'a str,
    pub binary: &'a str,
    pub holds: &'a crate::holds::PaneHolds,
}

/// What a manage op produced, and — for a session op — the wait that has to finish before its
/// ack means anything. `session.rs` is what spawns that wait rather than serving it inline.
pub struct Managed {
    pub reply: Value,
    pub settle: Option<Settle>,
}

/// The wait for the host to agree that a named session is, or is not, running.
///
/// Both session ops finish before the state they changed is visible: `herdr server --session` is
/// listed running 3-4 ms after the spawn, and `server.stop` answers `ok` in under a millisecond
/// while `session list` goes on reporting `running: true` for another 52-303 ms (#240, #241).
/// The `managed` ack is the cue for the client to re-ask `caps`, so acking before the host has
/// caught up hands back the state the operator was trying to change — which is what "the session
/// doesn't close when done" was.
///
/// Owned rather than borrowed because it is awaited off the dispatch loop.
pub struct Settle {
    binary: String,
    node_id: String,
    name: String,
    running: bool,
    failure: String,
}

impl Settle {
    pub async fn wait(self) -> Result<(), ManageError> {
        let agreed = settled(&self.binary, &self.name, self.running).await;
        crate::caps::sessions_changed(&self.node_id);
        if agreed {
            return Ok(());
        }
        Err(ManageError::Herdr(self.failure))
    }
}

impl Manager<'_> {
    pub async fn run(&self, op: &ManageOp) -> Result<Managed, ManageError> {
        match op.op.as_str() {
            "session.create" => self.create_session(op).await,
            "session.stop" => self.stop_session(op).await,
            _ => Ok(Managed {
                reply: self.structural(op).await?,
                settle: None,
            }),
        }
    }

    async fn structural(&self, op: &ManageOp) -> Result<Value, ManageError> {
        match op.op.as_str() {
            "workspace.create" => {
                let mut params = json!({ "label": op.label, "cwd": op.cwd, "focus": false });
                if let Some(env) = env_map(op.env.as_ref())? {
                    params["env"] = env;
                }
                self.call("workspace.create", params).await
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
            "pane.size" => self.size_pane(op).await,
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

    /// The session outlives the node on purpose: an operator's agents are inside it, and a node
    /// restart is not a reason to end them.
    ///
    /// `herdr server` does not daemonise, so without this the new session joins the node's own
    /// process group and dies with it — a Ctrl-C on a foreground `kampr serve` signals the whole
    /// group, and systemd's default `KillMode` signals the whole cgroup. The unit answers the
    /// cgroup half with `KillMode=process`; this answers the group half.
    async fn create_session(&self, op: &ManageOp) -> Result<Managed, ManageError> {
        let name = session_name(op)?;
        let mut command = Command::new(kampr_herdr::locate::program(self.binary));
        command
            .args(["server", "--session", &name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        command.process_group(0);
        command.spawn().map_err(|e| ManageError::Herdr(e.to_string()))?;
        let failure = format!("{name} was started but never appeared in the session list");
        Ok(Managed {
            reply: json!({ "session": name }),
            settle: Some(self.settle(name, true, failure)),
        })
    }

    /// The one op that reshapes a pane, and the only place in Kampr that claims a PTY.
    ///
    /// It exists because a pane can be born unusable and nothing else can reach it: a headless
    /// session's PTY is whatever created it, `observe` never touches it (#14), and no method on
    /// herdr's socket API reports or sets a column count at all (#221). The already-shipped
    /// `pane.zoom` is the complement rather than a substitute — it moves the PTY only when a client
    /// is attached and does nothing at all headless (#265), which is exactly the case this serves.
    ///
    /// Three modes, and the default is the safe one:
    /// - `once` claims, resizes, releases, then *measures* — because on an attached pane the desk
    ///   takes its geometry straight back (#19) and a reply that assumed otherwise would be a
    ///   plausible-looking success, which is the failure this project has paid for before (#233).
    /// - `hold` keeps the claim so the size survives on an attached pane, at the cost of that desk
    ///   rendering wrong while it is held (#298). Never implicit; the operator ticks it.
    /// - `release` lets a hold go.
    async fn size_pane(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let Target::Pane(pane) = self.target(op)? else {
            return Err(ManageError::BadRequest("pane.size needs a pane".into()));
        };
        let mode = op.mode.as_deref().unwrap_or("once");

        if mode == "release" {
            let held = self.holds.release(&pane);
            let rows = self.viewport_rows(&pane).await;
            return Ok(json!({ "pane_id": pane, "held": false, "was_held": held, "rows": rows }));
        }

        // Checked before the numbers, so an unknown mode is reported as an unknown mode rather
        // than as the missing `cols` it also happens to have.
        if !matches!(mode, "hold" | "once") {
            return Err(ManageError::BadRequest(format!("unknown size mode {mode}")));
        }
        let (cols, rows) = match (op.cols, op.rows) {
            (Some(c), Some(r)) => (c, r),
            _ => return Err(ManageError::BadRequest("pane.size needs cols and rows".into())),
        };
        checked_size(cols, rows)?;

        // Let go *before* claiming, never after. Herdr allows one controller at a time and refuses
        // the second with `already has an attached client` (#21), so a re-size while holding has to
        // release first or it fails outright — and the release is asynchronous, so this waits for
        // the pane to actually be free rather than racing it.
        if self.holds.release(&pane) {
            self.holds.wait_until_free(&pane).await;
        }

        let socket = self.herdr.socket().to_path_buf();
        let controller = kampr_herdr::Controller::claim(self.binary, &socket, &pane, cols, rows)
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))?;

        match mode {
            "hold" => {
                self.holds.park(&pane, controller);
                Ok(json!({ "pane_id": pane, "cols": cols, "rows": rows, "held": true }))
            }
            _ => {
                controller
                    .release()
                    .await
                    .map_err(|e| ManageError::Herdr(e.to_string()))?;
                // What actually stuck. Rows are the honest half — `viewport_rows` is the PTY's and
                // not the rect's (#84) — and columns are reported by nothing anywhere (#221), so
                // the reply says what it measured rather than echoing what was asked for.
                let measured = self.viewport_rows(&pane).await;
                let kept = measured == Some(u64::from(rows));
                Ok(json!({
                    "pane_id": pane, "cols": cols, "rows": rows,
                    "held": false, "kept": kept, "measured_rows": measured,
                }))
            }
        }
    }

    /// The PTY's rows, or `None` if herdr will not say. Never an error: this is the *check* after
    /// a resize, and a check that cannot be made must not turn a resize that happened into a
    /// failure that did not.
    async fn viewport_rows(&self, pane: &str) -> Option<u64> {
        let reply: kampr_herdr::model::PaneReply = self
            .herdr
            .call("pane.get", json!({ "pane_id": pane }))
            .await
            .ok()?;
        reply.pane.scroll.map(|s| s.viewport_rows)
    }

    /// A named session has its own socket, so stopping one is an ordinary `server.stop` addressed
    /// somewhere other than this node's own herdr.
    async fn stop_session(&self, op: &ManageOp) -> Result<Managed, ManageError> {
        let name = session_name(op)?;
        let socket = crate::caps::sessions(self.binary)
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))?
            .into_iter()
            .find(|s| s.name == name)
            .and_then(|s| s.socket_path)
            .ok_or_else(|| ManageError::UnknownTarget(name.clone()))?;
        Herdr::new(socket)
            .call::<Value>("server.stop", json!({}))
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))?;
        let failure = format!("server.stop was accepted but {name} is still running");
        Ok(Managed {
            reply: json!({ "session": name }),
            settle: Some(self.settle(name, false, failure)),
        })
    }

    fn settle(&self, name: String, running: bool, failure: String) -> Settle {
        Settle {
            binary: self.binary.to_string(),
            node_id: self.node_id.to_string(),
            name,
            running,
            failure,
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, ManageError> {
        self.herdr
            .call::<Value>(method, params)
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))
    }
}

/// Waits until the host agrees that `name` is (or is not) running, and answers whether it ever did.
///
/// The wait backs off rather than running flat out. Those two numbers are three orders apart, so a
/// fixed 20 ms interval spent nearly all of its ~250 subprocesses on the stop's long tail;
/// doubling from [`SESSION_POLL`] answers the create just as quickly for a tenth of the processes.
async fn settled(binary: &str, name: &str, running: bool) -> bool {
    let deadline = Instant::now() + SESSION_SETTLE;
    let mut wait = SESSION_POLL;
    loop {
        match crate::caps::sessions(binary).await {
            // No answer is not "the op did not take": the host was not asked, so the deadline is
            // what settles it rather than a list nobody read.
            Err(e) => debug!(session = %name, error = %e, "could not read the session list"),
            Ok(found) => {
                if found.iter().any(|s| s.name == name && s.running) == running {
                    return true;
                }
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(SESSION_POLL_MAX);
    }
}

/// Herdr 0.8.2 types `env` as a map and refuses a `null` outright — and the client omits the key
/// whenever the operator typed no variables, which is nearly every new workspace. `Option::None`
/// serialising to `null` therefore failed the op every time, so an absent or empty map has to
/// leave the key off the params rather than name it as nothing.
fn env_map(env: Option<&Value>) -> Result<Option<Value>, ManageError> {
    match env {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) if map.is_empty() => Ok(None),
        Some(value @ Value::Object(_)) => Ok(Some(value.clone())),
        Some(other) => Err(ManageError::BadRequest(format!(
            "env must be a map of strings, not {other}"
        ))),
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

/// The herdr-local id a creating op produced, dug out of whichever record herdr chose to echo.
///
/// A session is deliberately not one of them: it is named, not addressed, and node-qualifying its
/// name produced an `id` shaped exactly like a pane id for something no client can watch.
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
    None
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
        assert_eq!(
            created_id(&json!({"session":"agents"})),
            None,
            "a session name is not a pane id"
        );
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
        let spelled = |e: ManageError| serde_json::to_value(e.code()).unwrap();
        assert_eq!(spelled(ManageError::Unsupported("x".into())), "unsupported");
        assert_eq!(spelled(ManageError::BadRequest("x".into())), "bad_request");
        assert_eq!(spelled(ManageError::UnknownTarget("x".into())), "unknown_pane");
        assert_eq!(spelled(ManageError::Herdr("x".into())), "herdr_unavailable");
    }

    /// The guard the operator asked for by name: a phone must not be able to lock a pane at phone
    /// width. A headless resize persists (#219), so there is no later event that undoes one.
    #[test]
    fn a_pane_cannot_be_resized_smaller_than_a_terminal_is_usable_at() {
        let refused = |c, r| match checked_size(c, r) {
            Err(ManageError::BadRequest(said)) => said,
            other => panic!("{c}x{r} was allowed: {other:?}"),
        };
        // A phone's own viewport, which is exactly what "fit this to my screen" would ask for.
        let said = refused(45, 20);
        assert!(
            said.contains("80x24"),
            "the refusal has to name the floor: {said}"
        );
        assert!(
            said.contains("keeps the size it is given"),
            "and say why it cannot be undone: {said}",
        );
        refused(MIN_COLS - 1, MIN_ROWS);
        refused(MIN_COLS, MIN_ROWS - 1);
        refused(MAX_COLS + 1, MIN_ROWS);
        refused(MIN_COLS, MAX_ROWS + 1);

        assert_eq!(checked_size(MIN_COLS, MIN_ROWS).unwrap(), (80, 24));
        assert_eq!(checked_size(200, 50).unwrap(), (200, 50));
    }
}
