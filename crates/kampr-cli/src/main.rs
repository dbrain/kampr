mod dirs;
mod doctor;
mod init;
mod mesh;
mod pairing;
mod recovery;
mod report;
mod service;
mod setup;
mod update;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use dirs::Dirs;
use kampr_node::{Config, Node, http};
use report::Local;

#[derive(Debug, Parser)]
#[command(name = "kampr", version, about = "Remote access to your herd")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Write config, generate this node's identity, and print a URL and a pairing code
    Init {
        #[command(flatten)]
        dirs: Dirs,
        #[arg(long)]
        name: Option<String>,
        /// `host:port`. Loopback by default; anything else is reachable from the network, and
        /// anything that pairs there can type into every terminal on this host.
        #[arg(long)]
        bind: Option<String>,
        /// The one URL clients will use. Set this before enrolling a passkey — a passkey is
        /// bound to an origin and does not survive a change of one.
        #[arg(long)]
        origin: Option<String>,
        /// Rewrite config.toml from defaults, carrying the identity, the bind, the origin, the
        /// proxy settings and `[mesh]` forward. It says what it keeps and what it resets.
        #[arg(long)]
        force: bool,
        /// Mint a new node identity as well. Every enrolled passkey and every mesh peer pinned to
        /// the old one stops working, so it is never implied by --force.
        #[arg(long)]
        new_identity: bool,
    },
    /// Run the node
    Serve {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// Pair devices, revoke them, and install the service; and report the tier ladder
    Setup {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// What this node is and whether it is reachable
    Status {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// Print the node's URL
    Url {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// Pair a device from the command line
    Pair {
        #[command(flatten)]
        dirs: Dirs,
        #[arg(long)]
        readonly: bool,
    },
    /// Get back in with the recovery code, when no paired device is left
    Recover {
        #[command(flatten)]
        dirs: Dirs,
        /// Issue a fresh code and retire the current one. For a paper record that was lost, or a
        /// node that predates recovery codes.
        #[arg(long)]
        new: bool,
        #[arg(long, default_value = "recovered")]
        name: String,
    },
    /// Check everything that has to be true for this node to work, and say what is not
    Doctor {
        #[command(flatten)]
        dirs: Dirs,
        #[arg(long)]
        json: bool,
    },
    /// Join hosts into one herd
    Mesh {
        #[command(subcommand)]
        action: MeshAction,
    },
    /// Replace this binary with the latest release, verifying it first
    Update {
        #[command(flatten)]
        dirs: Dirs,
        /// Say what is available and install nothing
        #[arg(long)]
        check: bool,
        /// A release tag rather than the latest — for going back to one that worked
        #[arg(long)]
        version: Option<String>,
    },
    /// Install or remove the user service that keeps the node running
    Service {
        action: ServiceAction,
        #[command(flatten)]
        dirs: Dirs,
    },
}

#[derive(Debug, Subcommand)]
enum MeshAction {
    /// Print a single-use join code for another node, and this node's fingerprint
    Invite {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// Join a hub. Runs on the node that dials out, which is the one that need not be reachable
    Join {
        #[command(flatten)]
        dirs: Dirs,
        /// The hub's URL, as `kampr mesh invite` printed it
        #[arg(long)]
        hub: String,
        #[arg(long)]
        code: String,
        /// The hub fingerprint to confirm. Without it the key is pinned on trust at first sight
        #[arg(long)]
        fingerprint: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Who may connect to this node, and which hubs it dials
    List {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// Cut a node off. Takes a name, node id, fingerprint or key
    Revoke {
        #[command(flatten)]
        dirs: Dirs,
        node: String,
    },
    /// Forget a node entirely, so it is not even listed as revoked
    Forget {
        #[command(flatten)]
        dirs: Dirs,
        node: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ServiceAction {
    Install,
    Uninstall,
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Init {
            dirs,
            name,
            bind,
            origin,
            force,
            new_identity,
        } => {
            init::run(
                &dirs.config(),
                &dirs.state(),
                init::Init {
                    node_name: name,
                    bind,
                    origin,
                    force,
                    new_identity,
                },
            )
            .await
        }
        Command::Serve { dirs } => serve(&dirs).await,
        Command::Setup { dirs } => setup::run(&dirs.config(), dirs.state_override()).await,
        Command::Status { dirs } => status(&dirs).await,
        Command::Url { dirs } => {
            println!("{}", Config::load(&dirs.config())?.origin());
            Ok(())
        }
        Command::Pair { dirs, readonly } => {
            let local = Local::open(&dirs.config(), dirs.state_override()).await?;
            let role = if readonly {
                kampr_auth::Role::Readonly
            } else {
                kampr_auth::Role::Full
            };
            let pairing = pairing::create(&local, role).await?;
            println!("{}", pairing.code);
            pairing::arm(&local, &pairing).await
        }
        Command::Recover { dirs, new, name } => {
            let local = Local::open(&dirs.config(), dirs.state_override()).await?;
            if new {
                recovery::issue(&local).await
            } else {
                recovery::redeem(&local, &name).await
            }
        }
        Command::Doctor { dirs, json } => doctor::run(&dirs, json).await,
        Command::Mesh { action } => match action {
            MeshAction::Invite { dirs } => mesh::invite(&dirs.config(), dirs.state_override()).await,
            MeshAction::Join {
                dirs,
                hub,
                code,
                fingerprint,
                name,
            } => {
                mesh::join(
                    &dirs.config(),
                    dirs.state_override(),
                    mesh::Join {
                        hub,
                        code,
                        expect: fingerprint,
                        name,
                    },
                )
                .await
            }
            MeshAction::List { dirs } => mesh::list(&dirs.config(), dirs.state_override()).await,
            MeshAction::Revoke { dirs, node } => {
                mesh::revoke(&dirs.config(), dirs.state_override(), &node).await
            }
            MeshAction::Forget { dirs, node } => {
                mesh::leave(&dirs.config(), dirs.state_override(), &node).await
            }
        },
        Command::Update { dirs, check, version } => {
            update::run(
                &dirs.config(),
                dirs.state_override(),
                update::Update { check, version },
            )
            .await
        }
        Command::Service { action, dirs } => match action {
            ServiceAction::Install => {
                // Loaded rather than guessed: `serve` resolves the state directory from the
                // config, so a unit that guessed the XDG default would supervise a node that
                // came up on an empty database and a fresh VAPID key.
                let config = Config::load(&dirs.config())?;
                let installed = service::install(
                    &std::env::current_exe()?,
                    &dirs.config(),
                    &config.resolve_state_dir(dirs.state_override()),
                    std::env::var("HERDR_SOCKET_PATH").ok().as_deref(),
                )?;
                println!("installed {}", installed.path.display());
                println!("start it with: {}", service::start_hint());
                if let Some(note) = installed.reboot.note() {
                    println!();
                    println!("{note}");
                }
                Ok(())
            }
            ServiceAction::Uninstall => {
                let state = Config::load(&dirs.config())
                    .map(|c| c.resolve_state_dir(dirs.state_override()))
                    .unwrap_or_else(|_| dirs.state());
                service::uninstall()?;
                println!("removed. Config and devices are kept at:");
                println!("  {}", dirs.config().display());
                println!("  {}", state.display());
                println!("Delete both to reset — every paired device loses access.");
                Ok(())
            }
            ServiceAction::Status => {
                println!("{}", service::status());
                Ok(())
            }
        },
    }
}

async fn serve(dirs: &Dirs) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("KAMPR_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let config = Config::load(&dirs.config())?;
    let origin = config.origin();
    let state_dir = config.resolve_state_dir(dirs.state_override());
    let node = Node::start(config, &state_dir).await?;
    tracing::info!(
        node = %node.config.node_name,
        id = %node.config.node_id,
        %origin,
        tier = node.auth.tier().tier,
        "kampr node listening"
    );
    http::serve(node).await
}

async fn status(dirs: &Dirs) -> Result<()> {
    let local = Local::open(&dirs.config(), dirs.state_override()).await?;
    let url = local.config.origin();
    let devices = local.auth.devices().await?;
    let active = devices.iter().filter(|d| d.active(kampr_auth::now())).count();
    // From the cache the node's own once-a-day check writes, never a request: status has to
    // answer on a machine with no route out, and in the time it takes to read a file.
    let waiting = kampr_node::update::available(
        &local.config,
        &local.config.resolve_state_dir(dirs.state_override()),
    );
    match waiting {
        Some(available) => println!(
            "kampr {} — {available} available, take it with `kampr update`",
            kampr_node::BUILD
        ),
        None => println!("kampr {}", kampr_node::BUILD),
    }
    println!(
        "  node      {} ({})",
        local.config.node_name, local.config.node_id
    );
    println!("  url       {url}");
    println!("{}", report::bind_summary(&local.config));
    println!(
        "  reachable {}",
        if report::reachable(&url).await {
            "yes"
        } else {
            "no"
        }
    );
    println!("  service   {}", service::status());
    if let Some(identity) = &local.identity {
        println!("  identity  {}", identity.fingerprint());
    }
    println!("  devices   {active} paired, {} total", devices.len());
    println!("{}", report::tier_summary(local.auth.tier()));
    Ok(())
}
