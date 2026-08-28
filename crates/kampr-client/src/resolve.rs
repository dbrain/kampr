use crate::profile::{ClientConfig, LocalDevice};
use kampr_auth::store::{Role, Store};
use kampr_auth::{AuditLog, Entry};
use kampr_node::Config;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the node on this machine gets to answer `/healthz` before the ladder moves on. It is
/// a loopback request against a process that is either running or not.
const HEALTH_TIMEOUT: Duration = Duration::from_millis(1500);

/// How this herd was found. It is what the CLI prints, and it is the difference between "your own
/// machine" and "the one you paired with".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    /// A node on this machine, reached with a device this CLI minted for itself.
    LocalNode { node_name: String, device: String },
    /// A token from a previous pair, out of the client config.
    Profile { name: String },
}

/// A herd this client can open, and the credential for it.
#[derive(Debug, Clone)]
pub struct Session {
    pub origin: String,
    pub token: String,
    pub via: Via,
}

impl Session {
    /// What `kampr` prints before it opens: which herd, and how it got in.
    pub fn describe(&self) -> String {
        match &self.via {
            Via::LocalNode { node_name, device } => {
                format!("{node_name} ({}) as {device}", self.origin)
            }
            Via::Profile { name } => format!("{name} ({})", self.origin),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// Neither rung landed. The message is the whole of what a person needs to do next, and this
    /// is never a prompt: a command that blocks on a question cannot be run from a script.
    #[error(
        "no herd to open.\n\n\
         There is no kampr node running on this machine, and no herd saved from a previous \
         pair.\n\n\
         To open this machine's own herd:  kampr init, then kampr serve\n\
         To open another machine's:        pair a device from its `kampr setup`, then\n\
         \x20                                   kampr connect <url> --code <code>"
    )]
    NoHerd,
    #[error("the device store at {0}: {1}")]
    Store(PathBuf, kampr_auth::StoreError),
    #[error(transparent)]
    Config(#[from] crate::profile::ConfigError),
    #[error("{0}")]
    Audit(String),
}

/// **Resolution order, first hit wins.**
///
/// 1. A node on this machine, if its config resolves and it answers `/healthz`. The CLI runs as
///    that node's own user with **write** access to its state, so it mints itself a device and
///    connects to the local origin.
/// 2. A saved client profile — a token from a previous pair. This is the ordinary remote case,
///    and it is how a laptop drives the herd.
/// 3. Neither, which prints how to pair and exits non-zero. It never prompts.
///
/// A hub shows its peers because the herd already contains them; there is no second code path
/// for "remote".
pub async fn resolve(config_dir: &Path, state_dir: Option<&Path>) -> Result<Session, ResolveError> {
    if let Ok(config) = Config::load(config_dir) {
        let origin = config.origin();
        if answers_healthz(&origin).await {
            return local_session(&config, config_dir, state_dir, origin).await;
        }
    }
    let client = ClientConfig::load(config_dir)?;
    if let Some((name, profile)) = client.chosen() {
        return Ok(Session {
            origin: profile.origin.clone(),
            token: profile.token.clone(),
            via: Via::Profile { name: name.clone() },
        });
    }
    Err(ResolveError::NoHerd)
}

/// Whether the node this config describes is actually up.
///
/// A TCP connection is not the question — something else may hold the port — so this asks for the
/// one route that answers without a credential and reads the answer.
pub async fn answers_healthz(origin: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(HEALTH_TIMEOUT).build() else {
        return false;
    };
    let url = format!("{}/healthz", origin.trim_end_matches('/'));
    match client.get(url).send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

/// **The self-minted device is a real device**: named `cli@<hostname>`, listed by `kampr setup`,
/// revoked like any other, and written to the audit log at creation. There is no code path here
/// that authenticates without a token.
///
/// It grants nothing that was not already granted. Minting requires write access to the node's
/// state database, and anything that can write that database can already enrol itself a device of
/// any role by other means — the boundary this crosses was crossed by having the file open.
/// `08-threat-model.md` §5, "The CLI mints itself a device", carries the whole argument.
async fn local_session(
    config: &Config,
    config_dir: &Path,
    state_dir: Option<&Path>,
    origin: String,
) -> Result<Session, ResolveError> {
    let state_dir = config.resolve_state_dir(state_dir);
    let db = Config::state_db(&state_dir);
    let store = Store::open(&db)
        .await
        .map_err(|e| ResolveError::Store(db.clone(), e))?;
    let mut client = ClientConfig::load(config_dir)?;
    let name = device_name();

    if let Some(local) = &client.local
        && local.node_id == config.node_id
        && let Ok(Some(device)) = store.device_for_token(&local.token, kampr_auth::now()).await
        && device.id == local.device_id
    {
        return Ok(Session {
            origin,
            token: local.token.clone(),
            via: Via::LocalNode {
                node_name: config.node_name.clone(),
                device: device.name,
            },
        });
    }

    let now = kampr_auth::now();
    // The node's own term, not `None`. A token that never expires is a plaintext bearer credential
    // living on every machine that runs `kampr`, surviving backups and dotfile syncs and exempt
    // from the one forcing function Tier 0 has. The passkey exemption does not apply to it: that
    // exists because a passkey is a strong credential, and this is a string in a file.
    let expires_at = Some(now + config.auth.token_days as i64 * 86_400);
    let device = store
        .create_device(
            &name,
            Role::Full,
            now,
            expires_at,
            Some("kampr-cli"),
            Some(&origin),
        )
        .await
        .map_err(|e| ResolveError::Store(db.clone(), e))?;
    let token = store
        .mint_token(&device.id, now, expires_at)
        .await
        .map_err(|e| ResolveError::Store(db.clone(), e))?;
    audit(config, &state_dir, &device);
    client.local = Some(LocalDevice {
        node_id: config.node_id.clone(),
        device_id: device.id.clone(),
        token: token.clone(),
    });
    client.save(config_dir)?;
    Ok(Session {
        origin,
        token,
        via: Via::LocalNode {
            node_name: config.node_name.clone(),
            device: device.name,
        },
    })
}

/// A device that appeared in the list with nothing in the log saying where it came from is
/// exactly the shape an operator should not have to reason about.
///
/// **Best effort, and the threat model says so**: auditing off, or a log that will not open,
/// mints the device anyway rather than refusing to open a herd.
fn audit(config: &Config, state_dir: &Path, device: &kampr_auth::Device) {
    if !config.auth.audit {
        return;
    }
    let Ok(log) = AuditLog::open(&Config::audit_path(state_dir)) else {
        return;
    };
    log.record(
        &Entry::new("device.minted")
            .device(&device.id, &device.name, device.role.as_str())
            .peer("cli")
            .detail(serde_json::json!({ "reason": "the kampr CLI on this host" })),
    );
}

pub fn device_name() -> String {
    format!("cli@{}", hostname())
}

pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "kampr".into())
}
