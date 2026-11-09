use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Everything a child needs from its host, in one variable so a `docker exec -e NAME` can carry it
/// from the host's environment without the key ever appearing in an argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStdio {
    pub node_id: String,
    pub node_name: String,
    /// Hex of the child's ed25519 secret, minted and kept by the host so the child's identity
    /// survives the container being recreated.
    pub key: String,
    pub hub_key: String,
}

impl PeerStdio {
    pub const ENV: &str = "KAMPR_PEER_STDIO";

    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("strings serialise")
    }

    pub fn from_env() -> Result<Self> {
        let raw = std::env::var(Self::ENV).with_context(|| format!("{} is not set", Self::ENV))?;
        let spec: Self =
            serde_json::from_str(&raw).with_context(|| format!("{} is not readable", Self::ENV))?;
        if !kampr_mesh::valid_node_id(&spec.node_id) {
            bail!("{} names a node id a node cannot have", Self::ENV);
        }
        Ok(spec)
    }
}
