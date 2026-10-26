use kampr_client::{DialError, HubOffer, Session, Via};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// How long the hub may stay unreachable before this machine's own herd is shown instead. Long
/// enough to ride out a node restart, short enough that an outage is not a blank screen.
const GRACE: Duration = Duration::from_secs(10);

/// How often the hub is tried again while this machine's herd stands in for it.
const EVERY: Duration = Duration::from_secs(15);

const DIAL: Duration = Duration::from_secs(5);

/// A hub and the node on this machine that stands behind it: the hub's herd contains this one, so
/// it is the one to show, and this one is what is left while the hub cannot be reached.
#[derive(Debug, Clone)]
pub struct Route {
    pub hub: Session,
    pub local: Session,
    pub config_dir: PathBuf,
    pub grace: Duration,
    pub every: Duration,
    on_hub: bool,
    /// The hub turned this device away. Trying it again on a timer cannot fix that; pairing can.
    pub refused: bool,
}

pub enum Step {
    Stay,
    Go(Session),
}

impl Route {
    pub fn new(hub: Session, local: Session, config_dir: PathBuf) -> Self {
        Self {
            hub,
            local,
            config_dir,
            grace: GRACE,
            every: EVERY,
            on_hub: true,
            refused: false,
        }
    }

    pub fn on_hub(&self) -> bool {
        self.on_hub
    }

    pub fn hub_name(&self) -> String {
        match &self.hub.via {
            Via::Profile { name } => name.clone(),
            Via::LocalNode { node_name, .. } => node_name.clone(),
        }
    }

    pub fn offer(&self) -> HubOffer {
        HubOffer {
            name: self.hub_name(),
            origin: self.hub.origin.clone(),
            config_dir: self.config_dir.clone(),
        }
    }

    pub fn fall_back(&mut self) -> Session {
        self.on_hub = false;
        self.local.clone()
    }

    pub fn paired(&mut self, hub: Session) -> Session {
        self.hub = hub;
        self.refused = false;
        self.on_hub = true;
        self.hub.clone()
    }
}

/// The timers a [`Route`] runs on, which live with the herd on screen rather than with the route:
/// each herd is a fresh loop and starts its own clock.
pub struct Watch {
    since: Instant,
    down: Option<Instant>,
    tried: Option<Instant>,
    trying: Option<oneshot::Receiver<Result<(), DialError>>>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            since: Instant::now(),
            down: None,
            tried: None,
            trying: None,
        }
    }
}

impl Watch {
    /// Called every tick. On the hub, whether it has been gone long enough to stop waiting for it;
    /// off it, whether a dial just landed.
    pub fn step(&mut self, route: &mut Route, connected: bool) -> Step {
        let now = Instant::now();
        if route.on_hub {
            match connected {
                true => {
                    self.since = now;
                    self.down = None;
                }
                false => {
                    let down = *self.down.get_or_insert(self.since);
                    if now.duration_since(down) >= route.grace {
                        return Step::Go(route.fall_back());
                    }
                }
            }
            return Step::Stay;
        }
        if route.refused {
            return Step::Stay;
        }
        if let Some(trying) = self.trying.as_mut() {
            match trying.try_recv() {
                Ok(Ok(())) => {
                    self.trying = None;
                    route.on_hub = true;
                    return Step::Go(route.hub.clone());
                }
                Ok(Err(DialError::Refused(_))) => {
                    self.trying = None;
                    route.refused = true;
                }
                Ok(Err(_)) | Err(oneshot::error::TryRecvError::Closed) => self.trying = None,
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
            return Step::Stay;
        }
        if self.tried.is_none_or(|at| now.duration_since(at) >= route.every) {
            self.tried = Some(now);
            let (done, trying) = oneshot::channel();
            let hub = route.hub.clone();
            tokio::spawn(async move {
                let dialled = kampr_client::dial::connect(&hub.origin, &hub.token, DIAL)
                    .await
                    .map(drop);
                let _ = done.send(dialled);
            });
            self.trying = Some(trying);
        }
        Step::Stay
    }
}
