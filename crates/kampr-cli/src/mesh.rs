use crate::report::Local;
use anyhow::{Context, Result, bail};
use kampr_auth::identity::fingerprint_of;
use kampr_auth::{MeshRole, NodeIdentity};
use kampr_mesh::dial::{Hub, mesh_url};
use kampr_mesh::{Outgoing, Presence};
use kampr_node::{BUILD, Config};
use std::path::Path;
use std::time::Duration;

/// Mints a join code and prints the two things the operator has to carry to the other machine:
/// the URL and the code. The fingerprint is printed alongside so the *peer* end can be told what
/// it should expect to see, which is what turns a first connection from trust-on-first-use into a
/// confirmed one.
pub async fn invite(config_dir: &Path, state: Option<&Path>) -> Result<()> {
    let local = Local::open(config_dir, state).await?;
    let identity = NodeIdentity::load_or_create(&Config::node_key_path(config_dir))?;
    let now = kampr_auth::now();
    let ttl = local.auth.policy().pairing_ttl.as_secs() as i64;
    let mesh = local.auth.store().mesh();
    mesh.expire_invites(now).await?;
    let code = mesh.invite(now, now + ttl).await?;
    let url = local.config.origin();

    println!("Join code for a node, valid {} minutes, one node:", ttl / 60);
    println!();
    println!("  kampr mesh join --hub {url} --code {code} \\");
    println!("      --fingerprint {}", identity.fingerprint());
    println!();
    println!(
        "  hub          {} ({})",
        local.config.node_name, local.config.node_id
    );
    println!("  fingerprint  {}", identity.fingerprint());
    println!();
    println!("A node that cannot reach {url} cannot join. Nothing else has to be reachable:");
    println!("peers dial out, so only this host needs an address.");
    Ok(())
}

pub struct Join {
    pub hub: String,
    pub code: String,
    pub expect: Option<String>,
    pub name: Option<String>,
}

/// Dials the hub once, in the foreground, so the operator sees the answer rather than a log line.
///
/// The hub's key is pinned on success and every later connection is refused if it changes. When
/// `--fingerprint` is given the pin is confirmed *before* this node signs anything.
pub async fn join(config_dir: &Path, state: Option<&Path>, options: Join) -> Result<()> {
    let local = Local::open(config_dir, state).await?;
    let identity = NodeIdentity::load_or_create(&Config::node_key_path(config_dir))?;
    let url = mesh_url(&options.hub);
    println!("dialling {url} …");

    let hub = Hub {
        url: options.hub.clone(),
        name: options.name.clone().unwrap_or_else(|| "hub".into()),
        key: None,
        join: Some(options.code),
    };
    let presence = Presence {
        node_id: local.config.node_id.clone(),
        node_name: local.config.node_name.clone(),
        build: BUILD.to_string(),
    };
    let (hub_identity, mut out, _incoming) =
        kampr_mesh::dial(&hub, &identity, &presence, Duration::from_secs(15))
            .await
            .context("joining the hub")?;
    if let Some(expected) = &options.expect
        && !matches(expected, &hub_identity.key)
    {
        bail!(
            "the node at {url} has fingerprint {} — expected {expected}. Nothing was enrolled.",
            hub_identity.fingerprint()
        );
    }
    // The link closes here on purpose: `kampr serve` is what keeps one up, and it dials from the
    // row this writes rather than from anything held in this process.
    out.close().await;

    local
        .auth
        .store()
        .mesh()
        .enrol(
            &hub_identity.key,
            &hub_identity.node_id,
            &options.name.unwrap_or(hub_identity.node_name.clone()),
            MeshRole::Hub,
            Some(&url),
            kampr_auth::now(),
        )
        .await?;

    println!();
    println!("joined {} ({})", hub_identity.node_name, hub_identity.node_id);
    println!("  url          {url}");
    println!("  fingerprint  {}", hub_identity.fingerprint());
    if options.expect.is_none() {
        println!();
        println!("  Confirm that fingerprint on the hub — `kampr status` prints it there.");
        println!("  It is pinned now: a different node answering at this address is refused.");
    }
    println!();
    println!("`kampr serve` keeps the link up from here, and reconnects on its own.");
    Ok(())
}

pub async fn list(config_dir: &Path, state: Option<&Path>) -> Result<()> {
    let local = Local::open(config_dir, state).await?;
    let mesh = local.auth.store().mesh();
    let identity = NodeIdentity::load(&Config::node_key_path(config_dir))?;
    println!("kampr {} — {}", BUILD, local.config.node_name);
    if let Some(identity) = identity {
        println!("  this node    {}", identity.fingerprint());
    }
    for (label, role) in [("peers", MeshRole::Peer), ("hubs", MeshRole::Hub)] {
        let nodes = mesh.nodes(role).await?;
        println!();
        println!("  {label}");
        if nodes.is_empty() {
            println!("    none");
        }
        for node in nodes {
            let state = match node.revoked_at {
                Some(_) => "revoked",
                None => "enrolled",
            };
            println!(
                "    {:<20} {:<20} {state:<9} {}",
                node.name,
                node.fingerprint(),
                node.url.unwrap_or_default()
            );
        }
    }
    println!();
    println!("A live link shows as an online node in the herd; this list is who *may* connect.");
    Ok(())
}

pub async fn revoke(config_dir: &Path, state: Option<&Path>, needle: &str) -> Result<()> {
    let local = Local::open(config_dir, state).await?;
    match local
        .auth
        .store()
        .mesh()
        .revoke(needle, kampr_auth::now())
        .await?
    {
        Some(node) => {
            println!("revoked {} ({})", node.name, node.fingerprint());
            println!("A running node drops the link within seconds; a stopped one refuses it next time.");
            Ok(())
        }
        None => bail!("no node in this herd matches {needle:?}"),
    }
}

pub async fn leave(config_dir: &Path, state: Option<&Path>, needle: &str) -> Result<()> {
    let local = Local::open(config_dir, state).await?;
    match local.auth.store().mesh().forget(needle).await? {
        Some(node) => {
            println!("forgot {} ({})", node.name, node.fingerprint());
            Ok(())
        }
        None => bail!("no node in this herd matches {needle:?}"),
    }
}

/// A fingerprint an operator read off another screen, compared however they wrote it down.
fn matches(expected: &str, key: &str) -> bool {
    let normalise = |s: &str| s.trim().to_lowercase().replace('-', "");
    normalise(expected) == normalise(&fingerprint_of(key)) || normalise(expected) == normalise(key)
}
