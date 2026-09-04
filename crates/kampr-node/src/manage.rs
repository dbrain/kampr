use crate::caps::SessionEntry;
use kampr_core::wire::ErrorCode;
use kampr_herdr::Herdr;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
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
    /// A fleet run written the way the operator would type it into their own shell, with `&&`,
    /// `|`, quotes and globs meaning what they mean there.
    ///
    /// Additive beside `args`, never instead of it: a client that has never heard of this field
    /// goes on sending an argv and that argv goes on being `exec`ed with nothing in front of it.
    /// When both arrive this one wins, because only a client that knows about it sends it.
    #[serde(default)]
    pub command: Option<String>,
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
    /// Groups the panes one fan-out produced. Assigned by the client, because a run spans hosts
    /// and no single node can name it.
    #[serde(default)]
    pub cohort: Option<String>,
    /// A fleet book entry. A field of its own rather than `at`, which is routed: a book entry id
    /// names no host, and putting one in `at` would send `fleet.drop` down a mesh link looking for
    /// the node that owns it.
    #[serde(default)]
    pub entry: Option<String>,
    /// Names the hold a `pane.size` release is letting go of, so that a viewer which has already
    /// been displaced by a newer one cannot take the newer one's hold down with it. Absent on
    /// every release an operator makes by hand, which lets go of whatever is standing.
    #[serde(default)]
    pub lease: Option<u64>,
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

/// The geometry a fleet run gets.
///
/// **This is the one place Kampr chooses a pane's size, and rule 3 permits it precisely because
/// the pane is Kampr's own**: a pty this node forked, with no desk attached and no operator
/// geometry to lose. It is not `pane.size`, it never reaches herdr, and no view switch, fit,
/// reconnect or layout can call it — the size is fixed when the run starts and the only other
/// caller is the operator asking for a different one on a new run.
/// What a `fleet.run` asks to have run, in whichever of the two shapes the client sent.
///
/// **`command` first, and it is not a reinterpretation of `args`.** The two mean different things
/// and always have: `args` is an argv and is `exec`ed with nothing in front of it, `command` is a
/// line the operator's own shell parses. A client sends one or the other, and one that sends both
/// is a new client whose `args` is only there for something older to render.
pub fn fleet_job(op: &ManageOp) -> Option<kampr_fleet::Job> {
    if let Some(line) = op.command.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        return Some(kampr_fleet::Job::Shell(line.to_string()));
    }
    let argv = op.args.clone().filter(|a| !a.is_empty())?;
    Some(kampr_fleet::Job::Argv(argv))
}

fn fleet_geometry(op: &ManageOp) -> Result<kampr_fleet::Geometry, ManageError> {
    let default = kampr_fleet::Geometry::default();
    let (cols, rows) = match (op.cols, op.rows) {
        (Some(cols), Some(rows)) => checked_size(cols, rows)?,
        (None, None) => (default.cols as u32, default.rows as u32),
        _ => {
            return Err(ManageError::BadRequest(
                "a fleet run takes both `cols` and `rows` or neither".into(),
            ));
        }
    };
    Ok(kampr_fleet::Geometry {
        cols: cols as u16,
        rows: rows as u16,
    })
}

/// Sixteen times the worst stop measured in #241, and a thousand times the worst create in #240.
const SESSION_SETTLE: Duration = Duration::from_secs(5);
const SESSION_POLL: Duration = Duration::from_millis(20);
const SESSION_POLL_MAX: Duration = Duration::from_millis(320);

/// How long a herdr started by a manage op has to answer before the op gives up on it. Probe #326
/// measured 50 ms on an idle machine with nothing to restore; this is four hundred times that,
/// because a host nobody has visited for a month is reading its workspaces back while an operator
/// waits on a phone.
const WAKE_DEADLINE: Duration = Duration::from_secs(20);

/// The readiness probe is a **call**, not a connection or a file test. A starting herdr binds its
/// socket ~1 ms in and answers ~50 ms in, holding the early connection open rather than refusing
/// it (#326) — so the call is both the honest question and the same wait the op was going to do.
/// The window a wake must not act inside is the ~1 ms before the socket exists at all.
const WAKE_PROBE: Duration = Duration::from_secs(2);

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
    pub fleet: &'a std::sync::Arc<kampr_fleet::FleetProvider>,
    /// Told what `pane.size` put on a pane, so the streams stop reporting the width they inferred
    /// before it. Nothing here reads a size back out of it and nothing here resizes through it.
    pub provider: &'a std::sync::Arc<kampr_core::HerdrProvider>,
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
            // Both session ops shell out and neither needs this node's socket: `session.create`
            // is a wake with a name on it, and waking a herdr in order to stop it is absurd.
            "session.create" => self.create_session(op).await,
            "session.stop" => self.stop_session(op).await,
            // Fleet ops never touch herdr, so they never wake one. A host the operator has not
            // opened a terminal on is still a host they can run a command across.
            "fleet.run" | "fleet.stop" | "fleet.forget" => Ok(Managed {
                reply: self.fleet_op(op)?,
                settle: None,
            }),
            _ => {
                self.wake().await?;
                Ok(Managed {
                    reply: self.structural(op).await?,
                    settle: None,
                })
            }
        }
    }

    /// The three fleet ops, which are deliberately the whole surface.
    ///
    /// There is no `fleet.answer`: an answer is `input` to the pane, the same message that reaches
    /// every other pane in the herd. A second way to type into a terminal is a second thing to get
    /// wrong, and the first one already carries a phone's reply across the mesh.
    fn fleet_op(&self, op: &ManageOp) -> Result<Value, ManageError> {
        match op.op.as_str() {
            "fleet.run" => {
                let job = fleet_job(op).ok_or_else(|| {
                    ManageError::BadRequest(
                        "fleet.run needs `command`, the line to run, or `args`, an argv".into(),
                    )
                })?;
                let cohort = op.cohort.clone().ok_or_else(|| {
                    ManageError::BadRequest(
                        "fleet.run needs a `cohort` so its panes can be grouped with the rest of                          the run"
                            .into(),
                    )
                })?;
                let geometry = fleet_geometry(op)?;
                let pane = self
                    .fleet
                    .start(&cohort, &job, op.cwd.as_deref(), geometry)
                    .map_err(|e| ManageError::BadRequest(e.to_string()))?;
                Ok(json!({ "pane_id": pane, "cohort": cohort }))
            }
            "fleet.stop" => {
                let pane = self.fleet_target(op)?;
                self.fleet
                    .stop(&pane)
                    .map_err(|e| ManageError::BadRequest(e.to_string()))?;
                Ok(json!({ "ok": true }))
            }
            "fleet.forget" => {
                let pane = self.fleet_target(op)?;
                self.fleet
                    .forget(&pane)
                    .map_err(|e| ManageError::BadRequest(e.to_string()))?;
                Ok(json!({ "ok": true }))
            }
            other => Err(ManageError::BadRequest(format!("{other} is not a fleet op"))),
        }
    }

    fn fleet_target(&self, op: &ManageOp) -> Result<String, ManageError> {
        let at = op
            .at
            .as_deref()
            .ok_or_else(|| ManageError::BadRequest(format!("{} needs `at`", op.op)))?;
        Ok(at.rsplit_once('/').map_or(at, |(_, local)| local).to_string())
    }

    /// Starts the herdr this node's socket belongs to, if a manage op finds it stopped.
    ///
    /// **Only a manage op may do this.** Watching, polling, reconnecting and the herd sweep all
    /// find the same stopped herdr and all leave it stopped: an operator who is not using herdr on
    /// a host is not asking for one, and a node that started one anyway would be resurrecting the
    /// server they had just shut down. A manage op is the operator saying they want the machine.
    ///
    /// Racing is safe rather than negotiated — a second server for a session already running exits
    /// 1 and changes nothing (#243 for a named one, #325 for the default) — so two clients tapping
    /// at once, or a wake that crosses the operator's own `herdr` at the keyboard, costs one dead
    /// child process and nothing else.
    async fn wake(&self) -> Result<(), ManageError> {
        if self.answers().await {
            return Ok(());
        }
        let name = self.session_at_this_socket().await?;
        spawn_server(self.binary, &name)?;
        let deadline = Instant::now() + WAKE_DEADLINE;
        let mut wait = SESSION_POLL;
        loop {
            if self.answers().await {
                // The set of running sessions just changed, and a client refreshing on its own ack
                // must not be told for another ten seconds that this one is stopped (#241).
                crate::caps::sessions_changed(self.node_id);
                debug!(session = %name, socket = %self.herdr.socket().display(), "started herdr for a manage op");
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ManageError::Herdr(format!(
                    "herdr was started for {name} but never answered on {} within {WAKE_DEADLINE:?}",
                    self.herdr.socket().display()
                )));
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(SESSION_POLL_MAX);
        }
    }

    async fn answers(&self) -> bool {
        self.herdr
            .clone()
            .with_timeout(WAKE_PROBE)
            .snapshot()
            .await
            .is_ok()
    }

    /// Which session to start, asked of herdr rather than derived from the socket path.
    ///
    /// The node's socket is configuration and may be anything; only herdr knows which of its
    /// sessions owns one, and it lists them all whether or not they are running (#326). A socket
    /// no session claims is not something to guess a name for — `--session` would start a
    /// *different* server somewhere else and the op would then fail against a socket still dead.
    async fn session_at_this_socket(&self) -> Result<String, ManageError> {
        let listed = crate::caps::sessions(self.binary)
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))?;
        session_at(&listed, self.herdr.socket()).ok_or_else(|| {
            ManageError::Herdr(format!(
                "herdr is not running, and none of the {} sessions it lists owns {} — so there is \
                 nothing to start",
                listed.len(),
                self.herdr.socket().display()
            ))
        })
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

    /// Starting a session by name. `default` is one of the names: probe #324 measured
    /// `herdr server --session default` binding the default socket rather than making a namesake
    /// beside it, which is why [`Self::wake`] and this share one spawn.
    async fn create_session(&self, op: &ManageOp) -> Result<Managed, ManageError> {
        let name = session_name(op)?;
        spawn_server(self.binary, &name)?;
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
    /// - `match` is `hold` with an owner and an undo: the hold belongs to the websocket session
    ///   that asked for it, so it ends when that view does however the view ends, and it records
    ///   the geometry it found so that letting go puts the pane back. See
    ///   [ADR 0013](../../../docs/adr/0013-a-standing-intent-to-match-the-view.md).
    /// - `release` lets a hold go.
    async fn size_pane(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let Target::Pane(pane) = self.target(op)? else {
            return Err(ManageError::BadRequest("pane.size needs a pane".into()));
        };
        let mode = op.mode.as_deref().unwrap_or("once");

        if mode == "release" {
            let held = match op.lease {
                Some(token) => self.holds.let_go(&pane, token),
                None => self.holds.release(&pane),
            };
            let rows = self.viewport_rows(&pane).await;
            return Ok(json!({ "pane_id": pane, "held": false, "was_held": held, "rows": rows }));
        }

        // Checked before the numbers, so an unknown mode is reported as an unknown mode rather
        // than as the missing `cols` it also happens to have.
        if !matches!(mode, "hold" | "once" | "match") {
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
        //
        // A match carries the displaced hold's restore forward instead of dropping it, which is
        // what stops a window drag — or a handover to a second viewer — from turning the size
        // Kampr set into the size Kampr puts back.
        let carried = match mode {
            "match" => {
                let carried = self.holds.carry_for_match(&pane);
                self.holds.wait_until_free(&pane).await;
                carried
            }
            _ => {
                if self.holds.release(&pane) {
                    self.holds.wait_until_free(&pane).await;
                }
                None
            }
        };
        // Read before the claim, because after it the answer is the claim's own (#18).
        let found = match mode {
            "match" => match carried {
                Some(found) => Some(found),
                None => self.found_geometry(&pane).await,
            },
            _ => None,
        };

        let socket = self.herdr.socket().to_path_buf();
        let controller = kampr_herdr::Controller::claim(self.binary, &socket, &pane, cols, rows)
            .await
            .map_err(|e| ManageError::Herdr(e.to_string()))?;

        match mode {
            "hold" => {
                self.holds.park(
                    &pane,
                    controller,
                    crate::holds::PANEL_LIMIT,
                    None,
                    self.provider.clone(),
                );
                // A held controller *is* the pane's geometry until it lets go (#18), so there is
                // nothing left to check: this is the width.
                self.provider.resized(&pane, cols as u16, true);
                Ok(json!({ "pane_id": pane, "cols": cols, "rows": rows, "held": true }))
            }
            "match" => {
                let restore = found.map(|found| crate::restore::Restore {
                    binary: self.binary.to_string(),
                    herdr: self.herdr.clone(),
                    provider: self.provider.clone(),
                    found,
                    applied_rows: rows,
                });
                // No deadline. A matched hold's ceiling is the websocket session that owns it,
                // and that session releases it on every path out including a cancellation — so a
                // clock would only ever fire on an operator who was still looking at the pane.
                let token = self
                    .holds
                    .park(&pane, controller, None, restore, self.provider.clone());
                self.provider.resized(&pane, cols as u16, true);
                Ok(json!({
                    "pane_id": pane, "cols": cols, "rows": rows,
                    "held": true, "matched": true, "lease": token,
                    "found_cols": found.map(|(c, _)| c), "found_rows": found.map(|(_, r)| r),
                }))
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
                // Only what stuck. Rows are the half herdr reports honestly, so they are also the
                // only evidence that the columns went with them: on an attached pane the desk
                // takes the geometry back inside a second (#19), and adopting a width the PTY
                // does not have would crop every client instead of the one that asked.
                if kept {
                    self.provider.resized(&pane, cols as u16, false);
                }
                Ok(json!({
                    "pane_id": pane, "cols": cols, "rows": rows,
                    "held": false, "kept": kept, "measured_rows": measured,
                }))
            }
        }
    }

    /// The pane's own geometry before a matched hold claims it, and `None` unless **both** halves
    /// are honest.
    ///
    /// Rows come from `viewport_rows`, which is the PTY's and not the rect's (#84, #207). Columns
    /// come from a wrap the node has actually measured — the rect is fiction (#68) and nothing on
    /// the socket API reports a column count anywhere (#221) — so a pane that has never wrapped
    /// has no width worth putting back, and putting the rect back would be a resize to a number no
    /// row was ever laid out at. Nothing is better than a guess here: the pane keeps the viewer's
    /// size until something deliberate moves it, which is what `pane.size` is for.
    async fn found_geometry(&self, pane: &str) -> Option<(u16, u16)> {
        let cols = self.provider.measured_cols(pane)?;
        let rows = u16::try_from(self.viewport_rows(pane).await?).ok()?;
        (rows > 0).then_some((cols, rows))
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

/// The session outlives the node on purpose: an operator's agents are inside it, and a node
/// restart is not a reason to end them.
///
/// `herdr server` does not daemonise, so without the process group the new session joins the
/// node's own and dies with it — a Ctrl-C on a foreground `kampr serve` signals the whole group,
/// and systemd's default `KillMode` signals the whole cgroup. The unit answers the cgroup half
/// with `KillMode=process`; this answers the group half.
fn spawn_server(binary: &str, name: &str) -> Result<(), ManageError> {
    let mut command = Command::new(kampr_herdr::locate::program(binary));
    command
        .args(["server", "--session", name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    command.spawn().map_err(|e| ManageError::Herdr(e.to_string()))?;
    Ok(())
}

/// The name of the session herdr says owns `socket`, if any.
///
/// A path compare, then the same compare with the *directories* canonicalised — the socket file
/// itself does not exist while the server is down, so a node configured through a symlinked home
/// would otherwise match nothing and be told there is nothing to start.
fn session_at(sessions: &[SessionEntry], socket: &Path) -> Option<String> {
    let same = |listed: &Path| {
        listed == socket
            || (listed.file_name() == socket.file_name()
                && match (listed.parent(), socket.parent()) {
                    (Some(a), Some(b)) => match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
                        (Ok(a), Ok(b)) => a == b,
                        _ => false,
                    },
                    _ => false,
                })
    };
    sessions
        .iter()
        .find(|s| s.socket_path.as_deref().is_some_and(same))
        .map(|s| s.name.clone())
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
///
/// **A fleet run is the one creating op herdr never sees**, so its pane id is this node's own and
/// sits at the top level rather than inside a record. It is keyed on the op rather than found by
/// widening the search, because `pane.size` answers a top-level `pane_id` too and it creates
/// nothing — an `id` there would tell a client to wait for a herd patch that is never coming.
pub fn created_id(op: &str, reply: &Value) -> Option<String> {
    if op == "fleet.run" {
        return reply["pane_id"].as_str().map(str::to_string);
    }
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

/// What an op *measured*, as against what it echoed back — the half of a reply that is news.
///
/// `id` and `layout` are the two the ack already carried, and both are facts about a thing that
/// was created. This is the other kind: a fact about whether the op did what it was asked to.
///
/// **`pane.size` is the whole reason it exists.** It is the one op ADR 0012 lets reshape a pane,
/// it claims the PTY and then *checks*, and on an attached pane the desk takes the geometry
/// straight back inside a second (#19) — so `kept: false` is a routine answer and it was being
/// dropped on the floor, leaving the client told `ok: true` about a resize that did not happen.
/// That is [#233](#)'s shape on the one op that exists to be deliberate.
///
/// Additive, by rule: a client that has never heard of these fields is left exactly where it was.
pub fn measured(op: &str, reply: &Value) -> Vec<(&'static str, Value)> {
    const SIZE: &[&str] = &[
        "cols",
        "rows",
        "held",
        "kept",
        "measured_rows",
        "was_held",
        "matched",
        "lease",
        "found_cols",
        "found_rows",
    ];
    let fields: &[&'static str] = match op {
        "pane.size" => SIZE,
        _ => &[],
    };
    fields
        .iter()
        // A field herdr would not answer for is absent rather than null: `measured_rows` is
        // `None` when `pane.get` did not answer, and a client cannot read a null as a row count.
        .filter(|field| !reply[*field].is_null())
        .map(|field| (*field, reply[*field].clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn listed(entries: &[(&str, Option<&str>)]) -> Vec<SessionEntry> {
        entries
            .iter()
            .map(|(name, socket)| SessionEntry {
                name: (*name).to_string(),
                running: false,
                socket_path: socket.map(PathBuf::from),
            })
            .collect()
    }

    /// The socket a node is configured with is the whole question — the *name* of the session to
    /// start is herdr's answer about it, never a guess from the path. `default` is a name like any
    /// other (#324), which is what lets one spawn serve both kinds of server.
    #[test]
    fn the_session_to_start_is_the_one_herdr_says_owns_this_nodes_socket() {
        let sessions = listed(&[
            ("default", Some("/c/herdr/herdr.sock")),
            ("agents", Some("/c/herdr/sessions/agents/herdr.sock")),
        ]);
        assert_eq!(
            session_at(&sessions, Path::new("/c/herdr/herdr.sock")).as_deref(),
            Some("default")
        );
        assert_eq!(
            session_at(&sessions, Path::new("/c/herdr/sessions/agents/herdr.sock")).as_deref(),
            Some("agents")
        );
    }

    /// A socket no session claims has no name to start, and inventing one would start a different
    /// server somewhere else while the op went on failing against a socket still dead.
    #[test]
    fn a_socket_no_session_owns_is_nothing_to_start() {
        let sessions = listed(&[("default", Some("/c/herdr/herdr.sock"))]);
        assert_eq!(
            session_at(&sessions, Path::new("/run/user/1000/herdr.sock")),
            None
        );
        assert_eq!(session_at(&[], Path::new("/c/herdr/herdr.sock")), None);
        // An entry with no socket path is one this node cannot address, not a match for anything.
        assert_eq!(
            session_at(&listed(&[("nameless", None)]), Path::new("/c/herdr/herdr.sock")),
            None
        );
    }

    /// The socket file does not exist while the server is down, so only its directory can be
    /// canonicalised — and a node configured through a symlink (`/home/x` against `/var/home/x`)
    /// must not be told there is nothing to start.
    #[test]
    fn a_socket_reached_through_a_symlink_is_the_same_socket() {
        let dir = tempfile::tempdir().expect("a dir");
        let real = dir.path().join("herdr");
        std::fs::create_dir_all(&real).expect("the real dir");
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).expect("a symlink");
        let sessions = listed(&[("default", Some(&real.join("herdr.sock").display().to_string()))]);
        assert!(
            !link.join("herdr.sock").exists(),
            "the server is down: no socket file"
        );
        assert_eq!(
            session_at(&sessions, &link.join("herdr.sock")).as_deref(),
            Some("default"),
            "a symlinked config root is the same session"
        );
        assert_eq!(
            session_at(&sessions, &link.join("other.sock")),
            None,
            "the same directory is not the same socket"
        );
    }

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
                "workspace.create",
                &json!({"type":"workspace_created","workspace":{"workspace_id":"w1"},
                               "tab":{"tab_id":"w1:t1"}})
            ),
            Some("w1".into())
        );
        assert_eq!(
            created_id("tab.create", &json!({"tab":{"tab_id":"w1:t2"}})),
            Some("w1:t2".into())
        );
        assert_eq!(
            created_id("pane.split", &json!({"pane":{"pane_id":"w1:p3"}})),
            Some("w1:p3".into())
        );
        assert_eq!(
            created_id("session.create", &json!({"session":"agents"})),
            None,
            "a session name is not a pane id"
        );
        assert_eq!(created_id("close", &json!({"type":"ok"})), None);
    }

    /// A fleet run creates a pane herdr never hears about, and its id is the node's own — so it
    /// sits at the top level and the record walk above cannot see it. Without this the one op
    /// that makes a pane out of nothing acked with no `id` at all, while the wire promises one
    /// for every op that creates something.
    #[test]
    fn a_fleet_run_acks_the_pane_it_made_and_a_resize_acks_no_id_at_all() {
        assert_eq!(
            created_id("fleet.run", &json!({"pane_id":"fleet:abc","cohort":"c1"})),
            Some("fleet:abc".into())
        );
        // The reason it is keyed on the op: this reply has a top-level `pane_id` too, and it
        // created nothing. An `id` here tells a client to wait for a herd patch that never comes.
        assert_eq!(
            created_id("pane.size", &json!({"pane_id":"w1:p1","cols":100,"rows":30})),
            None
        );
    }

    /// A matched hold has to say so, and has to say what letting go will put back — a client that
    /// is told `held: true` and nothing else cannot tell the panel's hold from its own view's, and
    /// cannot say what the release will do. `found_*` is absent rather than null when the node has
    /// nothing honest to put back, because a rect is not a width (#68, #221).
    #[test]
    fn a_matched_hold_says_it_is_one_and_says_what_it_will_put_back() {
        let fields = |reply: &Value| -> Vec<&'static str> {
            measured("pane.size", reply).into_iter().map(|(f, _)| f).collect()
        };
        let carried = fields(&json!({"pane_id":"w1:p1","cols":200,"rows":50,"held":true,
                                     "matched":true,"lease":7,"found_cols":93,"found_rows":40}));
        assert!(
            carried.contains(&"matched") && carried.contains(&"lease"),
            "{carried:?}"
        );
        assert!(
            carried.contains(&"found_cols") && carried.contains(&"found_rows"),
            "{carried:?}"
        );
        // `lease` travels because a hub is a client of a peer: the hold lives on the peer, and the
        // hub is the only thing that will ever be in a position to let go of *that* hold rather
        // than whatever is standing. What must never travel is a `found_*` that is `null`, which
        // is a column count that is not a column count.
        let unproved = fields(&json!({"pane_id":"w1:p1","cols":200,"rows":50,"held":true,
                                      "matched":true,"found_cols":null,"found_rows":null}));
        assert!(!unproved.contains(&"found_cols"), "{unproved:?}");
        assert!(!unproved.contains(&"found_rows"), "{unproved:?}");
    }

    /// The measurement `pane.size` makes and the ack used to throw away. `kept: false` is the
    /// routine answer on an attached pane (#19) and it is the difference between a resize that
    /// happened and one that did not.
    #[test]
    fn a_resize_ack_carries_what_it_measured_and_omits_what_it_could_not() {
        let fields = |op, reply: &Value| -> Vec<(String, Value)> {
            measured(op, reply)
                .into_iter()
                .map(|(f, v)| (f.to_string(), v))
                .collect()
        };

        let took = fields(
            "pane.size",
            &json!({"pane_id":"w1:p1","cols":100,"rows":30,"held":false,
                    "kept":true,"measured_rows":30}),
        );
        assert_eq!(
            took,
            vec![
                ("cols".into(), json!(100)),
                ("rows".into(), json!(30)),
                ("held".into(), json!(false)),
                ("kept".into(), json!(true)),
                ("measured_rows".into(), json!(30)),
            ]
        );

        // The failure this whole thing is about: herdr took the claim and gave the geometry
        // straight back, so the numbers asked for are not the numbers the pane has.
        let refused = fields(
            "pane.size",
            &json!({"pane_id":"w1:p1","cols":100,"rows":30,"held":false,
                    "kept":false,"measured_rows":24}),
        );
        assert!(
            refused.contains(&("kept".to_string(), json!(false))),
            "{refused:?}"
        );

        // `pane.get` would not answer, so there is no measurement — and a null row count is not
        // a row count. It goes absent rather than travelling as `null`.
        let unmeasured = fields(
            "pane.size",
            &json!({"pane_id":"w1:p1","cols":100,"rows":30,"held":false,
                    "kept":false,"measured_rows":null}),
        );
        assert!(
            !unmeasured.iter().any(|(f, _)| f == "measured_rows"),
            "{unmeasured:?}"
        );
        assert!(unmeasured.contains(&("kept".to_string(), json!(false))));

        // A hold says it is held; a release says whether there was anything to let go of.
        assert!(
            fields(
                "pane.size",
                &json!({"pane_id":"w1:p1","cols":100,"rows":30,"held":true})
            )
            .contains(&("held".to_string(), json!(true)))
        );
        assert!(
            fields(
                "pane.size",
                &json!({"pane_id":"w1:p1","held":false,"was_held":true,"rows":30})
            )
            .contains(&("was_held".to_string(), json!(true)))
        );

        // Every other op measures nothing, and an ack that grew fields for them would be a wire
        // promise nothing keeps.
        assert!(fields("pane.split", &json!({"pane":{"pane_id":"w1:p3"},"kept":true})).is_empty());
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
