//! A node's Docker containers (docs/19): the manage ops, and the read a detail sheet polls.

use crate::manage::{ManageError, ManageOp, Manager, native_geometry};
use crate::state::Node;
use kampr_core::wire::ErrorCode;
use kampr_docker::Directory;
use kampr_native::NewSession;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn is_docker_op(op: &str) -> bool {
    op.starts_with("docker.")
}

fn container_id(op: &ManageOp) -> Result<&str, ManageError> {
    op.id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ManageError::BadRequest(format!("{} needs `id`, the container", op.op)))
}

pub(crate) fn failed(id: &str) -> impl Fn(anyhow::Error) -> ManageError + '_ {
    move |e| match kampr_docker::is_not_found(&e) {
        true => ManageError::UnknownTarget(id.to_string()),
        false => ManageError::BadRequest(format!("{e:#}")),
    }
}

fn source(docker: Option<&Arc<Directory>>) -> Result<&Arc<Directory>, ManageError> {
    docker.ok_or_else(|| ManageError::BadRequest("this node does not follow a docker daemon".into()))
}

impl Manager<'_> {
    pub(crate) async fn docker_op(&self, op: &ManageOp) -> Result<Value, ManageError> {
        let docker = source(self.docker)?;
        let id = container_id(op)?;
        let client = docker.client();
        match op.op.as_str() {
            "docker.start" => client.start(id).await.map_err(failed(id))?,
            "docker.stop" => client.stop(id).await.map_err(failed(id))?,
            "docker.restart" => client.restart(id).await.map_err(failed(id))?,
            "docker.kill" => client.kill(id).await.map_err(failed(id))?,
            "docker.remove" => client.remove(id).await.map_err(failed(id))?,
            other => return Err(ManageError::Unsupported(other.to_string())),
        }
        Ok(json!({ "ok": true }))
    }
}

/// Served off the dispatch loop: a stats read takes a second and a sheet polls every three, an
/// adoption copies tens of megabytes and waits for a node to start, and a shell may pull an image.
pub fn off_the_loop(op: &str) -> bool {
    matches!(
        op,
        "docker.detail" | "docker.shell" | "docker.adopt" | "docker.release"
    )
}

/// The `managed` ack for an [`off_the_loop`] op, served here or on the peer that has the container.
pub async fn served(node: &Arc<Node>, op: &ManageOp, mut raw: Value) -> Result<Value, (ErrorCode, String)> {
    let target = op.node.as_deref().or(op.at.as_deref());
    if let Some(target) = target.filter(|t| node.route(t).is_none()) {
        if let Some(object) = raw.as_object_mut() {
            object.remove("rid");
        }
        return match node.peers.manage(target, raw).await {
            Ok(reply) => Ok(reply),
            Err(kampr_mesh::RelayError::Unknown(_)) => Err((
                ErrorCode::UnknownPane,
                format!("{target} is not on a node this herd serves"),
            )),
            Err(e) => Err((e.code(), e.to_string())),
        };
    }
    let refused = |e: ManageError| (e.code(), e.to_string());
    let id = container_id(op).map_err(refused)?;
    let mut ack = json!({ "t": "managed", "op": op.op, "ok": true });
    match op.op.as_str() {
        "docker.detail" => {
            let primary = node.primary();
            let docker = source(primary.docker.as_ref()).map_err(refused)?;
            ack["detail"] = kampr_docker::detail(docker, id)
                .await
                .map_err(failed(id))
                .map_err(refused)?;
        }
        "docker.shell" => {
            let (pane, shell) = shell(node, op, id).await.map_err(refused)?;
            ack["id"] = json!(pane);
            ack["shell"] = json!(shell);
        }
        "docker.adopt" => ack["node"] = json!(crate::adopt::adopt(node, id).await.map_err(refused)?),
        "docker.release" => crate::adopt::release(node, id).await.map_err(refused)?,
        other => return Err(refused(ManageError::Unsupported(other.to_string()))),
    }
    Ok(ack)
}

async fn shell(node: &Node, op: &ManageOp, id: &str) -> Result<(String, &'static str), ManageError> {
    let primary = node.primary();
    let docker = source(primary.docker.as_ref())?;
    let native = primary
        .native
        .as_ref()
        .ok_or_else(|| ManageError::BadRequest("this node does not keep native sessions".into()))?;
    let (cols, rows) = native_geometry(op)?;
    let client = docker.client();
    let inspect = client.inspect(id).await.map_err(failed(id))?;
    let shell = kampr_docker::shell::choose(client, &inspect, node.config.docker.debug_image())
        .await
        .map_err(failed(id))?;
    let name = inspect.short_name();
    let pane = native
        .create(NewSession {
            argv: shell.argv("docker", &inspect),
            label: Some(shell.label(name)),
            cols,
            rows,
            env: vec![(
                "DOCKER_HOST".into(),
                format!("unix://{}", client.socket().display()),
            )],
            container: Some(name.to_string()),
            ..NewSession::default()
        })
        .await
        .map_err(|e| ManageError::BadRequest(format!("{e:#}")))?;
    Ok((primary.global_pane(&pane), shell.mode()))
}
