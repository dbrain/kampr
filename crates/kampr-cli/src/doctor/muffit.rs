use super::Check;
use kampr_node::Config;
use kampr_node::muffit::standing::{Verdict, standing};

const ID: &str = "muffit";

/// A daemon that is down is a supported state; one that answers and refuses this node's token is
/// a node dialling the wrong daemon (a container's holds 7891 on some hosts), and lists nothing.
pub async fn check(config: &Config) -> Check {
    let Some(standing) = standing(config).await else {
        return Check::ok(ID, "off: no muffit token was found, or [muffit] enabled = false");
    };
    let at = format!("{} (from {})", standing.url, standing.from);
    match standing.verdict {
        Verdict::Answers => Check::ok(ID, format!("the daemon at {at} takes this node's token")),
        Verdict::Away(_) => Check::ok(
            ID,
            format!("no daemon answers at {at}; its sessions are listed once one does"),
        ),
        Verdict::Refused { status, why } => Check::fail(
            ID,
            format!(
                "the daemon at {at} answered {status} ({why}), so no muffit session is listed; \
                 it is not the daemon whose token this node read"
            ),
        )
        .fix(
            "find the daemon this user runs with `ss -ltnp | grep muffit-server` and set \
             [muffit] url = \"http://127.0.0.1:<its port>\" in config.toml (the port is [server] \
             port in muffit's config.toml)",
        ),
    }
}
