use super::PeerStdio;
use crate::config::Config;
use crate::state::Node;
use anyhow::{Context, Result, anyhow};
use kampr_auth::MeshRole;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::info;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// The child's config and key, written from what its host handed it. The host is the authority on
/// both, so they are rewritten whenever they differ and everything else an operator set is kept.
pub fn prepare(config_dir: &Path, state_dir: Option<&Path>, spec: &PeerStdio) -> Result<(Config, PathBuf)> {
    let mut config = match Config::load(config_dir) {
        Ok(config) => config,
        Err(crate::ConfigError::Missing(_)) => {
            let mut fresh = Config::bootstrap(&spec.node_name);
            // Its host injects the binary, so a release check in here can only ever be noise.
            fresh.update.check = false;
            fresh
        }
        Err(e) => return Err(e.into()),
    };
    config.node_id = spec.node_id.clone();
    config.node_name = spec.node_name.clone();
    config.config_dir = config_dir.display().to_string();
    if let Some(state) = state_dir {
        config.state_dir = state.display().to_string();
    }
    config.save(config_dir)?;
    let state = config.resolve_state_dir(state_dir);
    std::fs::create_dir_all(&state).with_context(|| format!("creating {}", state.display()))?;
    write_key(&config.key_path(), &spec.key)?;
    Ok((config, state))
}

fn write_key(path: &Path, hex: &str) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|held| held.trim() == hex) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        kampr_auth::private_dir(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(hex.as_bytes())?;
    Ok(())
}

/// Proves the host is the one whose key it was handed, and serves it until the stream ends.
pub async fn serve_parent<R, W>(node: Arc<Node>, hub_key: &str, reader: R, writer: W) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let identity = node.identity()?;
    let mut link = kampr_mesh::stdio::link(reader, writer);
    let hub = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        kampr_mesh::greet(&mut link, &identity, &node.presence(), None, Some(hub_key)),
    )
    .await
    .map_err(|_| anyhow!("the host never finished the handshake"))??;
    node.auth
        .store()
        .mesh()
        .enrol(
            &hub.key,
            &hub.node_id,
            &hub.node_name,
            MeshRole::Hub,
            None,
            kampr_auth::now(),
        )
        .await?;
    info!(host = %hub.node_name, "serving the host over stdio");
    let heard = link.incoming.heard();
    let (out, incoming) = link.split();
    crate::mesh::serve_hub(node, hub, out, incoming, heard).await;
    Ok(())
}
