//! herdr's structure ops on Kampr sessions: workspaces, tabs, agents, worktrees and layouts kept by
//! the node, so herdr is the optional extra rather than the only way to have any (docs/17).

use crate::manage::{ManageError, ManageOp, Manager, native_argv, native_geometry};
use kampr_core::tile::{Split, WHOLE, ratio};
use kampr_native::{LayoutNode, NativeProvider, NewSession, PREFIX, TAB_PREFIX, WORKSPACE_PREFIX};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const OPS: &[&str] = &[
    "workspace.create",
    "tab.create",
    "agent.start",
    "worktree.create",
    "worktree.open",
    "layout.export",
    "layout.apply",
];

/// The harnesses Kampr reads the conversations of, by the binary each is started as.
const AGENTS: &[&str] = &[
    kampr_journal::claude::AGENT,
    kampr_journal::codex::AGENT,
    kampr_journal::omp::AGENT,
    kampr_journal::agy::AGENT,
];

/// The agents `agent.start` can run here, by where each resolves on the node's PATH.
pub fn native_agents(config: &crate::Config) -> Vec<(String, PathBuf)> {
    let configured = Some(config.fleet.path.clone()).filter(|p| !p.is_empty());
    let Some(path) = kampr_fleet::fleet_path(configured) else {
        return Vec::new();
    };
    AGENTS
        .iter()
        .filter_map(|kind| Some((kind.to_string(), resolve(&path.value, kind)?)))
        .collect()
}

fn resolve(path: &str, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    path.split(':')
        .filter(|entry| !entry.is_empty())
        .map(|entry| Path::new(entry).join(name))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

fn failed(e: anyhow::Error) -> ManageError {
    ManageError::BadRequest(format!("{e:#}"))
}

impl Manager<'_> {
    /// Kampr first: an op naming nothing of herdr's is Kampr's even where herdr is on, and a
    /// client reaches herdr by saying so.
    pub(crate) fn kampr_structure(&self, op: &ManageOp) -> bool {
        if self.native.is_none() || !OPS.contains(&op.op.as_str()) {
            return false;
        }
        match op.provider.as_deref() {
            Some("herdr") => false,
            Some("native") => true,
            _ => op.at.is_none() || self.native_target(op).is_some(),
        }
    }

    pub(crate) async fn structure_op(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let native = self
            .native
            .ok_or_else(|| ManageError::BadRequest("this node does not keep native sessions".into()))?;
        match op.op.as_str() {
            "workspace.create" => {
                let name = op
                    .label
                    .clone()
                    .filter(|l| !l.trim().is_empty())
                    .or_else(|| op.cwd.as_deref().and_then(base_name))
                    .unwrap_or_else(|| "workspace".to_string());
                let workspace = native.new_workspace(&name).map_err(failed)?;
                let pane = self
                    .start(native, op, native_argv(op), op.cwd.clone(), Some(&workspace))
                    .await?;
                Ok(
                    json!({ "pane": { "pane_id": pane }, "workspace_id": format!("{WORKSPACE_PREFIX}{workspace}") }),
                )
            }
            "tab.create" => {
                let workspace = self
                    .workspace_at(native, op)
                    .ok_or_else(|| ManageError::BadRequest("tab.create needs a workspace".into()))?;
                let pane = self
                    .start(native, op, native_argv(op), op.cwd.clone(), Some(&workspace))
                    .await?;
                Ok(json!({ "pane": { "pane_id": pane }, "tab_id": kampr_native::tab_of(&pane) }))
            }
            "agent.start" => self.agent(native, op).await,
            "worktree.create" => self.worktree_create(native, op).await,
            "worktree.open" => {
                let path = op
                    .path
                    .as_deref()
                    .filter(|p| Path::new(p).is_dir())
                    .ok_or_else(|| {
                        ManageError::BadRequest("worktree.open needs the `path` of a worktree".into())
                    })?;
                let branch = git(Path::new(path), &["branch", "--show-current"]).ok();
                let name = branch
                    .map(|b| b.replace('/', "-"))
                    .filter(|b| !b.is_empty())
                    .or_else(|| base_name(path))
                    .unwrap_or_else(|| "worktree".to_string());
                self.workspace_on(native, op, &name, path).await
            }
            "layout.export" => {
                let tab = self.tab_at(native, op)?;
                let tree = native
                    .tab(&tab)
                    .ok_or_else(|| ManageError::UnknownTarget(op.at.clone().unwrap_or_default()))?;
                Ok(json!({ "layout": { "root": exported(native, &tree) } }))
            }
            "layout.apply" => self.apply(native, op).await,
            other => Err(ManageError::Unsupported(other.to_string())),
        }
    }

    /// `rename` and `close` on a workspace or a tab of Kampr sessions.
    pub(crate) fn container_op(
        &self,
        native: &NativeProvider,
        op: &ManageOp,
        local: &str,
    ) -> Result<Value, ManageError> {
        let members = match local.strip_prefix(WORKSPACE_PREFIX) {
            Some(id) => {
                if op.op == "rename" {
                    let label = op
                        .label
                        .as_deref()
                        .filter(|l| !l.trim().is_empty())
                        .ok_or_else(|| ManageError::BadRequest("a workspace needs a name".into()))?;
                    if !native.rename_workspace(id, label).map_err(failed)? {
                        return Err(ManageError::UnknownTarget(local.to_string()));
                    }
                    return Ok(json!({ "ok": true }));
                }
                native.workspace_members(id)
            }
            None => native.tab_members(local),
        };
        if op.op != "close" {
            return Err(ManageError::BadRequest(format!(
                "{} does not apply to {local}",
                op.op
            )));
        }
        if members.is_empty() {
            return Err(ManageError::UnknownTarget(local.to_string()));
        }
        for pane in members {
            native.close(&pane).map_err(failed)?;
        }
        Ok(json!({ "ok": true }))
    }

    async fn start(
        &self,
        native: &NativeProvider,
        op: &ManageOp,
        argv: Vec<String>,
        cwd: Option<String>,
        workspace: Option<&str>,
    ) -> Result<String, ManageError> {
        let (cols, rows) = native_geometry(op)?;
        native
            .create(NewSession {
                argv,
                cwd,
                cols,
                rows,
                workspace: workspace.map(str::to_string),
                ..NewSession::default()
            })
            .await
            .map_err(failed)
    }

    fn local_at(&self, op: &ManageOp) -> Option<String> {
        self.native_target(op)
    }

    /// The workspace an op is aimed at: `at` naming one, a tab or a session in one, or `workspace`.
    fn workspace_at(&self, native: &NativeProvider, op: &ManageOp) -> Option<String> {
        let from_at = self.local_at(op).and_then(|local| {
            if let Some(id) = local.strip_prefix(WORKSPACE_PREFIX) {
                return Some(id.to_string());
            }
            let pane = match local.starts_with(TAB_PREFIX) {
                true => native.tab_members(&local).into_iter().next()?,
                false => local,
            };
            native.workspace_of(&pane)
        });
        from_at.or_else(|| {
            op.workspace
                .as_deref()
                .and_then(|w| native.resolve_workspace(w).ok())
        })
    }

    fn tab_at(&self, native: &NativeProvider, op: &ManageOp) -> Result<String, ManageError> {
        let local = self
            .local_at(op)
            .ok_or_else(|| ManageError::BadRequest(format!("{} needs a tab", op.op)))?;
        Ok(match local.starts_with(PREFIX) {
            true => native.tab_of_pane(&local),
            false => local,
        })
    }

    async fn agent(&self, native: &NativeProvider, op: &ManageOp) -> Result<Value, ManageError> {
        let kind = op
            .kind
            .as_deref()
            .ok_or_else(|| ManageError::BadRequest("agent.start needs a kind".into()))?;
        let binary = native_agents(self.config)
            .into_iter()
            .find_map(|(k, path)| (k == kind).then_some(path))
            .ok_or_else(|| ManageError::BadRequest(format!("{kind} is not installed on this node")))?;
        let mut argv = vec![binary.display().to_string()];
        argv.extend(op.args.clone().unwrap_or_default());
        let beside = self.local_at(op).filter(|l| l.starts_with(PREFIX));
        let cwd = op.cwd.clone().or_else(|| {
            let pane = beside.as_deref()?;
            native.session(pane)?.spec().ok()?.cwd
        });
        let workspace = self.workspace_at(native, op);
        let pane = self.start(native, op, argv, cwd, workspace.as_deref()).await?;
        if let Some(at) = beside {
            native
                .split(&at, &pane, Split::Right, WHOLE / 2)
                .map_err(failed)?;
        }
        Ok(json!({ "pane": { "pane_id": pane } }))
    }

    async fn worktree_create(&self, native: &NativeProvider, op: &ManageOp) -> Result<Value, ManageError> {
        let branch = op
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .ok_or_else(|| ManageError::BadRequest("worktree.create needs a branch".into()))?;
        let cwd = op
            .cwd
            .clone()
            .or_else(|| {
                let pane = self.local_at(op).filter(|l| l.starts_with(PREFIX))?;
                native.session(&pane)?.spec().ok()?.cwd
            })
            .ok_or_else(|| {
                ManageError::BadRequest("worktree.create needs a `cwd` in a git repository".into())
            })?;
        let repo = PathBuf::from(git(Path::new(&cwd), &["rev-parse", "--show-toplevel"]).map_err(failed)?);
        let name = branch.replace('/', "-");
        let path = self
            .config
            .native
            .worktrees_dir()
            .join(base_name(&repo.display().to_string()).unwrap_or_else(|| "repo".into()))
            .join(&name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| failed(e.into()))?;
        }
        let at = path.display().to_string();
        let exists = git(
            &repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .is_ok();
        let mut args = vec!["worktree", "add"];
        match exists {
            true => args.extend([at.as_str(), branch]),
            false => {
                args.extend(["-b", branch, at.as_str()]);
                args.extend(op.base.as_deref());
            }
        }
        git(&repo, &args).map_err(failed)?;
        self.workspace_on(native, op, &name, &at).await
    }

    async fn workspace_on(
        &self,
        native: &NativeProvider,
        op: &ManageOp,
        name: &str,
        path: &str,
    ) -> Result<Value, ManageError> {
        let workspace = native.new_workspace(name).map_err(failed)?;
        let pane = self
            .start(
                native,
                op,
                native_argv(op),
                Some(path.to_string()),
                Some(&workspace),
            )
            .await?;
        Ok(json!({
            "pane": { "pane_id": pane },
            "workspace_id": format!("{WORKSPACE_PREFIX}{workspace}"),
            "path": path,
        }))
    }

    async fn apply(&self, native: &NativeProvider, op: &ManageOp) -> Result<Value, ManageError> {
        let layout = op
            .layout
            .as_ref()
            .ok_or_else(|| ManageError::BadRequest("layout.apply needs a layout".into()))?;
        let root = layout.get("root").unwrap_or(layout);
        let workspace = self.workspace_at(native, op);
        let mut made = Vec::new();
        let tree = Box::pin(self.build(native, op, root, workspace.as_deref(), &mut made)).await;
        let tree = match tree {
            Ok(tree) => tree,
            Err(e) => {
                for pane in &made {
                    let _ = native.close(pane);
                }
                return Err(e);
            }
        };
        let tab = native.place(tree).map_err(failed)?;
        Ok(json!({ "pane": { "pane_id": made[0] }, "tab_id": tab, "panes": made }))
    }

    async fn build(
        &self,
        native: &NativeProvider,
        op: &ManageOp,
        node: &Value,
        workspace: Option<&str>,
        made: &mut Vec<String>,
    ) -> Result<LayoutNode, ManageError> {
        match node["type"].as_str() {
            Some("pane") => {
                let argv = match node["command"].as_str().filter(|c| !c.trim().is_empty()) {
                    Some(line) => vec![kampr_native::login_shell(), "-lc".into(), line.to_string()],
                    None => vec![kampr_native::login_shell(), "-l".into()],
                };
                let cwd = node["cwd"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| op.cwd.clone());
                let pane = self.start(native, op, argv, cwd, workspace).await?;
                made.push(pane.clone());
                Ok(LayoutNode::Pane { pane })
            }
            Some("split") => {
                let split = node["direction"]
                    .as_str()
                    .and_then(Split::parse)
                    .ok_or_else(|| ManageError::BadRequest(format!("a split needs right or down: {node}")))?;
                let first = Box::pin(self.build(native, op, &node["first"], workspace, made)).await?;
                let second = Box::pin(self.build(native, op, &node["second"], workspace, made)).await?;
                Ok(LayoutNode::Split {
                    split,
                    ratio: ratio(node["ratio"].as_f64().unwrap_or(0.5)),
                    first: Box::new(first),
                    second: Box::new(second),
                })
            }
            _ => Err(ManageError::BadRequest(format!(
                "a layout node needs a type: {node}"
            ))),
        }
    }
}

/// herdr's `layout.export` shape (#47), with each leaf's directory and command so an apply can
/// start the same thing again.
fn exported(native: &NativeProvider, node: &LayoutNode) -> Value {
    match node {
        LayoutNode::Pane { pane } => {
            let spec = native.session(pane).and_then(|s| s.spec().ok());
            let mut leaf = json!({ "type": "pane", "pane_id": pane });
            if let Some(spec) = spec {
                leaf["cwd"] = json!(spec.cwd);
                leaf["command"] = json!(command_of(&spec.argv));
            }
            leaf
        }
        LayoutNode::Split {
            split,
            ratio,
            first,
            second,
        } => json!({
            "type": "split",
            "direction": split,
            "ratio": f64::from(*ratio) / f64::from(WHOLE),
            "first": exported(native, first),
            "second": exported(native, second),
        }),
    }
}

/// What a session was started with, as the line an apply hands a login shell; `None` for the
/// login shell itself.
fn command_of(argv: &[String]) -> Option<String> {
    match argv {
        [_, flag, line] if flag == "-lc" => Some(line.clone()),
        [_, flag] if flag == "-l" => None,
        argv => Some(argv.join(" ")),
    }
}

fn base_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
