use crate::config::Config;
use crate::herd::HerdModel;
use anyhow::{Context, Result};
use kampr_auth::{AuditLog, Auth, Store, Tier};
use kampr_core::registry::RegistryConfig;
use kampr_core::wire::{NodeEntry, PaneEntry};
use kampr_core::{HerdrConfig, HerdrProvider, PaneRegistry};
use kampr_herdr::Herdr;
use kampr_journal::{Registry as Journals, SessionRef};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub const BUILD: &str = match option_env!("KAMPR_BUILD") {
    Some(b) => b,
    None => env!("CARGO_PKG_VERSION"),
};

/// Herdr fires no event when the attached client resizes (probe #52), so the herd model is
/// re-derived on a timer as well as on every structural event. This poll is the only thing that
/// notices a desk resize.
const HERD_POLL: Duration = Duration::from_secs(3);

pub struct Node {
    pub config: Config,
    pub origin: String,
    pub herdr: Herdr,
    pub provider: Arc<HerdrProvider>,
    pub registry: Arc<PaneRegistry>,
    pub auth: Arc<Auth>,
    herd: watch::Sender<Arc<HerdModel>>,
    refresher: JoinHandle<()>,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.refresher.abort();
    }
}

impl Node {
    pub async fn start(config: Config, state_dir: &Path) -> Result<Arc<Self>> {
        let herdr = match config.herdr.socket.as_str() {
            "" => Herdr::discover()?,
            path => Herdr::new(path),
        };
        let provider = Arc::new(
            kampr_core::herdr_provider::connect_with_retry(
                herdr.clone(),
                HerdrConfig {
                    binary: config.herdr.binary.clone(),
                    ..HerdrConfig::default()
                },
            )
            .await
            .context("connecting to herdr")?,
        );
        let registry = PaneRegistry::with_config(
            provider.clone(),
            RegistryConfig {
                scrollback_max_rows: config.limits.scrollback_max_rows,
                ..RegistryConfig::default()
            },
        );
        let auth = Arc::new(build_auth(&config, state_dir).await?);

        let (herd, _) = watch::channel(Arc::new(HerdModel::default()));
        let refresher = tokio::spawn(refresh_herd(Refresh {
            node_id: config.node_id.clone(),
            node_name: config.node_name.clone(),
            herdr: herdr.clone(),
            provider: provider.clone(),
            registry: registry.clone(),
            herd: herd.clone(),
        }));

        Ok(Arc::new(Self {
            origin: config.origin(),
            config,
            herdr,
            provider,
            registry,
            auth,
            herd,
            refresher,
        }))
    }

    pub fn herd(&self) -> Arc<HerdModel> {
        self.herd.borrow().clone()
    }

    pub fn subscribe_herd(&self) -> watch::Receiver<Arc<HerdModel>> {
        self.herd.subscribe()
    }

    pub fn node_id(&self) -> &str {
        &self.config.node_id
    }

    /// Strips the `<node_id>/` prefix. A pane addressed on another node's id is not ours to
    /// serve, which is what keeps ids unambiguous once the herd is meshed.
    pub fn local_pane(&self, global: &str) -> Option<String> {
        global
            .strip_prefix(&self.config.node_id)
            .and_then(|rest| rest.strip_prefix('/'))
            .filter(|rest| !rest.is_empty())
            .map(str::to_string)
    }

    pub fn global_pane(&self, local: &str) -> String {
        format!("{}/{local}", self.config.node_id)
    }
}

async fn build_auth(config: &Config, state_dir: &Path) -> Result<Auth> {
    let store = Store::open(&Config::state_db(state_dir))
        .await
        .context("opening the device store")?;
    let mut tier = Tier::detect(&config.origin()).with_context(|| format!("origin {:?}", config.origin()))?;
    if !config.auth.rp_id.is_empty() {
        tier = tier.with_rp_id(&config.auth.rp_id);
    }
    let audit = if config.auth.audit {
        AuditLog::open(&Config::audit_path(state_dir)).context("opening the audit log")?
    } else {
        AuditLog::disabled()
    };
    let policy = kampr_auth::Policy {
        pairing_ttl: Duration::from_secs(config.auth.pairing_ttl_secs),
        tier0_token_ttl: (config.auth.token_days > 0)
            .then(|| Duration::from_secs(config.auth.token_days * 86_400)),
        ..kampr_auth::Policy::default()
    };
    Ok(Auth::new(store, tier, audit, policy)?)
}

struct Refresh {
    node_id: String,
    node_name: String,
    herdr: Herdr,
    provider: Arc<HerdrProvider>,
    registry: Arc<PaneRegistry>,
    herd: watch::Sender<Arc<HerdModel>>,
}

async fn refresh_herd(ctx: Refresh) {
    let mut topology = ctx.registry.topology();
    let mut journals = kampr_journal::registry_from_home(&home());
    let mut previous = Arc::new(HerdModel::default());
    loop {
        let mut model = build_model(&ctx, &journals).await;
        model.stamp(&previous);
        let model = Arc::new(model);
        previous = model.clone();
        ctx.herd.send_replace(model);

        tokio::select! {
            changed = topology.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = tokio::time::sleep(HERD_POLL) => {}
        }
        // A harness installed after the node started should not need a restart to be seen.
        journals = kampr_journal::registry_from_home(&home());
    }
}

async fn build_model(ctx: &Refresh, journals: &Journals) -> HerdModel {
    let snapshot = ctx.provider.snapshot();
    let panes = ctx
        .registry
        .list_panes()
        .await
        .unwrap_or_default()
        .iter()
        .map(|info| {
            let session = snapshot
                .pane(&info.pane_id)
                .and_then(|p| p.agent_session.as_ref())
                .map(|s| SessionRef {
                    agent: s.agent.clone(),
                    kind: match s.kind.as_str() {
                        "path" => kampr_journal::SessionKind::Path,
                        _ => kampr_journal::SessionKind::Id,
                    },
                    value: s.value.clone(),
                });
            let has_conversation = journals.has_conversation(info.agent.as_deref(), session.as_ref());
            PaneEntry::new(&ctx.node_id, info, has_conversation)
        })
        .collect();
    HerdModel {
        nodes: vec![NodeEntry {
            id: ctx.node_id.clone(),
            name: ctx.node_name.clone(),
            kind: "local".into(),
            online: true,
            rtt_ms: ping(&ctx.herdr).await,
            herdr_version: Some(ctx.provider.herdr_version()),
        }],
        panes,
    }
}

async fn ping(herdr: &Herdr) -> Option<f64> {
    let at = Instant::now();
    herdr
        .call::<serde_json::Value>("ping", serde_json::json!({}))
        .await
        .ok()
        .map(|_| at.elapsed().as_secs_f64() * 1000.0)
}

fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}
