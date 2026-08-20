mod dirs;
mod init;
mod pairing;
mod report;
mod service;
mod setup;

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
        #[arg(long)]
        force: bool,
    },
    /// Run the node
    Serve {
        #[command(flatten)]
        dirs: Dirs,
    },
    /// The setup ladder
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
    /// Install or remove the user service that keeps the node running
    Service {
        action: ServiceAction,
        #[command(flatten)]
        dirs: Dirs,
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
        } => {
            init::run(
                &dirs.config(),
                &dirs.state(),
                init::Init {
                    node_name: name,
                    bind,
                    origin,
                    force,
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
        Command::Service { action, dirs } => match action {
            ServiceAction::Install => {
                let path = service::install(
                    &std::env::current_exe()?,
                    &dirs.config(),
                    &dirs.state(),
                    std::env::var("HERDR_SOCKET_PATH").ok().as_deref(),
                )?;
                println!("installed {}", path.display());
                println!("start it with: systemctl --user start kampr.service");
                Ok(())
            }
            ServiceAction::Uninstall => {
                service::uninstall()?;
                println!("removed. Config and devices are kept — delete them by hand to reset.");
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
    println!("kampr {}", kampr_node::BUILD);
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
