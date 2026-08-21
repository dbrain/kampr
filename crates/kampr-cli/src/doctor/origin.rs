use super::Check;
use kampr_node::Config;
use std::time::Duration;

/// A TCP connect to the origin proves a port is open, which is the one thing that is true of
/// every broken reverse proxy: a Proxy Host forwarding to the wrong port, to a Docker-bridge
/// address that is not this machine, or to nothing at all still leaves NPM's own 443 listening.
/// `/api/node` is unauthenticated by design and names the node, so one request settles it.
pub async fn check(config: &Config) -> Check {
    let origin = config.origin();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("kampr-doctor")
        .build()
    {
        Ok(client) => client,
        Err(e) => return Check::warn("origin", format!("no HTTP client to test {origin} with: {e}")),
    };
    let response = match client.get(format!("{origin}/api/node")).send().await {
        Ok(response) => response,
        Err(e) => return unreachable(&origin, &chain(&e)),
    };
    let status = response.status();
    let identity = response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| body["node_id"].as_str().map(str::to_string));
    match identity {
        Some(id) if id == config.node_id => Check::ok(
            "origin",
            format!("{origin} reaches this node — the whole path answers, proxy included"),
        ),
        Some(other) => Check::fail(
            "origin",
            format!(
                "{origin} answers, but it is node {other}, not this one ({}) — that hostname is \
                 proxied somewhere else",
                config.node_id
            ),
        )
        .fix(format!(
            "point the proxy host's Forward Hostname / Port at {}",
            config.server.bind
        )),
        None => Check::fail(
            "origin",
            format!(
                "{origin} answers {status}, but not with this node's identity — whatever is on \
                 that hostname is not Kampr"
            ),
        )
        .fix(format!(
            "point the proxy host's Forward Hostname / Port at {}, and turn Websockets Support on",
            config.server.bind
        )),
    }
}

/// reqwest's own `Display` is "error sending request for url (…)" whatever went wrong; the
/// interesting sentence is always further down the chain.
fn chain(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(next) = source {
        text.push_str("; ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

/// Never a failure. A node that is not running is not a broken proxy — the `service` check
/// already says that — and a hub whose public hostname resolves to a WAN address its own NAT will
/// not hairpin back is a healthy deployment that simply cannot test itself from here.
fn unreachable(origin: &str, detail: &str) -> Check {
    let text = detail.to_lowercase();
    if text.contains("certificate") || text.contains("tls") || text.contains("handshake") {
        return Check::warn("origin", format!("{origin} refused the TLS handshake: {detail}"))
            .fix("renew or replace the certificate the proxy serves for this hostname");
    }
    if text.contains("dns")
        || text.contains("failed to lookup")
        || text.contains("name or service not known")
        || text.contains("nodename nor servname")
    {
        return Check::warn("origin", format!("{origin} does not resolve from this machine")).fix(
            "point the hostname at this machine — or, if only your phone's DNS knows it, test it from there",
        );
    }
    Check::warn(
        "origin",
        format!(
            "nothing answers at {origin} — the node is stopped, nothing is proxying to it, or \
             this machine cannot reach its own public hostname because the NAT in front of it \
             does not hairpin"
        ),
    )
    .fix("start the node and re-run this; if it still says nothing answers, check the proxy host")
}

#[cfg(test)]
mod tests {
    use super::super::Status;
    use super::*;

    #[test]
    fn an_unreachable_origin_is_never_a_failure_and_names_which_kind_it_is() {
        let dns = unreachable(
            "https://kampr.example.com",
            "error sending request for url; client error; dns error: failed to lookup address \
             information: Name or service not known",
        );
        assert_eq!(dns.status, Status::Warn);
        assert!(dns.detail.contains("does not resolve"), "{}", dns.detail);

        let tls = unreachable(
            "https://kampr.example.com",
            "error sending request; invalid peer certificate: Expired",
        );
        assert_eq!(tls.status, Status::Warn);
        assert!(tls.fix.unwrap().contains("certificate"));

        let refused = unreachable("http://127.0.0.1:8790", "tcp connect error: Connection refused");
        assert_eq!(refused.status, Status::Warn);
        assert!(refused.detail.contains("hairpin"), "{}", refused.detail);
    }
}
