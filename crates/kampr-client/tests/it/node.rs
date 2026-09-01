//! The real node, in process, with no herdr behind it.
//!
//! Nothing in a node waits on herdr — it binds its port and serves its own "the herd is not
//! running" state — so the handshake, the greeting, the role frame and the whole resolution
//! ladder can be driven honestly on a machine with no herdr installed, and without touching the
//! one herdr a machine has.

use kampr_auth::{Role, Store};
use kampr_client::profile::{ClientConfig, Profile};
use kampr_client::{Client, Event, Policy, Via, resolve};
use kampr_core::Backoff;
use kampr_node::{Config, Node, http};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const BEAT: Duration = Duration::from_secs(5);

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a home"),
        }
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }
}

struct Running {
    node: Arc<Node>,
    server: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn start(home: &Home) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let mut config = Config::bootstrap("scripted");
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.config_dir = home.config().display().to_string();
        config.state_dir = home.state().display().to_string();
        // No herdr, and no discovery of the one this machine may be running: the properties here
        // are about the socket, not about panes.
        config.herdr.socket = home.dir.path().join("no-herdr.sock").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        config.update.check = false;
        std::fs::create_dir_all(home.state()).expect("a state dir");
        config.save(&home.config()).expect("a config");
        let node = Node::start(config, &home.state()).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        Self { node, server }
    }

    fn stop(self) {
        self.node.shutdown();
        self.server.abort();
    }
}

async fn store(home: &Home) -> Store {
    Store::open(&Config::state_db(&home.state()))
        .await
        .expect("the device store")
}

fn policy() -> Policy {
    Policy {
        backoff: Backoff {
            initial: Duration::from_millis(20),
            max: Duration::from_millis(100),
        },
        connect_timeout: Duration::from_secs(2),
        manage_timeout: Duration::from_secs(2),
        event_capacity: 256,
    }
}

async fn until<T>(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(Event) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + BEAT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        if let Some(found) = want(event) {
            return found;
        }
    }
}

fn audit(home: &Home) -> String {
    std::fs::read_to_string(Config::audit_path(&home.state())).unwrap_or_default()
}

#[tokio::test]
async fn a_bare_kampr_finds_the_node_on_this_machine_and_mints_itself_a_device() {
    let home = Home::new();
    let running = Running::start(&home).await;

    let session = resolve(&home.config(), None).await.expect("a herd");
    let Via::LocalNode { device, .. } = &session.via else {
        panic!("a running node on this machine is the first rung");
    };
    assert_eq!(device, &kampr_client::resolve::device_name());

    // A real device: named, listed, revocable, and in the audit log at creation.
    let devices = store(&home).await.devices().await.expect("devices");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, kampr_client::resolve::device_name());
    assert_eq!(devices[0].role, Role::Full);
    assert!(
        audit(&home).contains("device.minted"),
        "a device nothing in the log accounts for is the shape an operator should never see"
    );

    // **It carries the node's own term, like every other Tier-0 enrolment.** A plaintext bearer
    // token in a file on every machine that runs `kampr`, with no expiry, outlives backups and
    // dotfile syncs and is exempt from the 30-day term whose whole job is to force a decision.
    let expires = devices[0]
        .expires_at
        .expect("the CLI device expires like any other");
    let term = kampr_auth::now() + 30 * 86_400;
    assert!(
        (expires - term).abs() < 120,
        "expected roughly the node's token_days term, got {expires} against {term}"
    );

    let client = Client::with_policy(session, policy());
    let mut events = client.events();
    let hello = until(&mut events, |e| match e {
        Event::Connected(hello) => Some(hello),
        _ => None,
    })
    .await;
    assert_eq!(hello.protocol, 1);
    assert_eq!(hello.node_name, "scripted");
    assert_eq!(hello.device.name, kampr_client::resolve::device_name());
    assert!(hello.role.writes());
    // The greeting is three frames and the third arrives unasked.
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    until(&mut events, |e| match e {
        Event::Prefs { greeting } => Some(greeting),
        _ => None,
    })
    .await
    .then_some(())
    .expect("the first prefs on a socket is the greeting's");

    // A node with no herdr is a herd that is offline, not an error — and it is a state a client
    // renders rather than an empty screen.
    let state = client.state();
    assert!(state.herd.known);
    assert!(
        state.herd.nodes.iter().all(|n| !n.online),
        "no herdr means an offline node, with a detail saying so"
    );
    drop(state);
    drop(client);
    running.stop();
}

#[tokio::test]
async fn a_second_run_reuses_the_device_the_first_one_minted() {
    let home = Home::new();
    let running = Running::start(&home).await;

    let first = resolve(&home.config(), None).await.expect("a herd");
    let second = resolve(&home.config(), None).await.expect("a herd");
    assert_eq!(first.token, second.token);
    assert_eq!(
        store(&home).await.devices().await.expect("devices").len(),
        1,
        "one row per invocation would fill the device list with this CLI"
    );

    // Revoked like any other device — and the next run mints a fresh one rather than presenting
    // a credential the operator took away.
    let devices = store(&home).await.devices().await.expect("devices");
    assert!(
        store(&home)
            .await
            .revoke_device(&devices[0].id, kampr_auth::now())
            .await
            .expect("revoked")
    );
    let third = resolve(&home.config(), None).await.expect("a herd");
    assert_ne!(third.token, first.token);
    assert_eq!(store(&home).await.devices().await.expect("devices").len(), 2);
    running.stop();
}

#[tokio::test]
async fn a_saved_profile_is_the_second_rung_and_a_dead_node_falls_through_to_it() {
    let home = Home::new();
    // A config exists and no node answers `/healthz`, which is the laptop case exactly.
    let mut config = Config::bootstrap("absent");
    config.server.bind = "127.0.0.1:1".into();
    config.server.origin = "http://127.0.0.1:1".into();
    // Named explicitly, and it matters: a config that leaves it empty resolves to the XDG
    // default, so a test that took the first rung by mistake would mint a device into the
    // operator's own database rather than into this temporary one.
    config.state_dir = home.state().display().to_string();
    config.config_dir = home.config().display().to_string();
    config.save(&home.config()).expect("a config");

    let mut client_config = ClientConfig::default();
    client_config.profiles.insert(
        "front".into(),
        Profile {
            origin: "https://kampr.example.com".into(),
            token: "from-a-previous-pair".into(),
        },
    );
    client_config.save(&home.config()).expect("client.toml");

    let session = resolve(&home.config(), None).await.expect("a herd");
    assert_eq!(session.origin, "https://kampr.example.com");
    assert_eq!(session.token, "from-a-previous-pair");
    assert_eq!(session.via, Via::Profile { name: "front".into() });
    assert!(
        store_missing(&home),
        "nothing was minted against a node that is not there"
    );
}

fn store_missing(home: &Home) -> bool {
    !Config::state_db(&home.state()).exists()
}

#[tokio::test]
async fn neither_rung_says_how_to_pair_rather_than_prompting() {
    let home = Home::new();
    std::fs::create_dir_all(home.config()).expect("a config dir");
    let refusal = resolve(&home.config(), None)
        .await
        .expect_err("there is no herd here");
    let said = refusal.to_string();
    assert!(said.contains("kampr init"), "{said}");
    assert!(said.contains("kampr connect"), "{said}");
}

#[tokio::test]
async fn a_socket_that_spells_its_token_any_other_way_is_refused() {
    let home = Home::new();
    let running = Running::start(&home).await;
    let origin = running.node.config.origin();

    let refused = kampr_client::dial::connect(&origin, "not-a-token", Duration::from_secs(2)).await;
    assert!(
        refused.is_err(),
        "there is no code path that authenticates without a token"
    );
    running.stop();
}

#[tokio::test]
async fn a_demotion_lands_on_the_socket_that_is_already_open() {
    let home = Home::new();
    let running = Running::start(&home).await;
    let session = resolve(&home.config(), None).await.expect("a herd");
    let client = Client::with_policy(session, policy());
    let mut events = client.events();
    until(&mut events, |e| matches!(e, Event::Connected(_)).then_some(())).await;
    assert!(client.state().role.writes());

    let devices = running.node.auth.devices().await.expect("devices");
    let device = devices.into_iter().next().expect("the minted device");
    running
        .node
        .auth
        .set_role(&device.id, Role::Readonly, &device)
        .await
        .expect("demoted");

    let role = until(&mut events, |e| match e {
        Event::Role(role) => Some(role),
        _ => None,
    })
    .await;
    assert!(!role.writes());
    let state = client.state();
    assert!(!state.role.writes());
    assert!(state.herd.known, "a demotion is not a second greeting");
    drop(state);
    drop(client);
    running.stop();
}
