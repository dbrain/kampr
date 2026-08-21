use super::Check;
use kampr_auth::Tier;
use kampr_auth::android::canonical_fingerprint;
use kampr_node::{Config, assetlinks};
use std::collections::BTreeSet;
use std::time::Duration;

const PATH: &str = "/.well-known/assetlinks.json";

/// Whether the Android app can hold a passkey for this node.
///
/// Credential Manager runs no ceremony for a native app until it has read this file *from the
/// origin the app is talking to* and found the app's package and signing certificate in it. The
/// node builds the document from `[android]` and serves it unauthenticated — so what breaks is
/// never the document, it is the path to it: a proxy that answers `/.well-known/*` itself, a CDN
/// holding a copy from before the fingerprint changed, or a hostname pointing somewhere else
/// entirely. Which is why this asks the origin rather than the config.
pub async fn check(config: &Config) -> Check {
    let origin = config.origin();
    // Below tier 1 there is no ceremony to fail: Credential Manager needs an RP ID, an RP ID is a
    // registrable domain, and the tier check has already said so at length. Quiet rather than a
    // second paragraph about a file nothing will read.
    if !Tier::detect(&origin).is_ok_and(|tier| tier.passkeys) {
        return Check::ok(
            "assetlinks",
            format!("not in play: {origin} cannot do passkeys at all, so nothing reads {PATH}"),
        );
    }
    let Some(document) = assetlinks::document(&config.android) else {
        return Check::warn(
            "assetlinks",
            "[android] names no package and no usable certificate, so this node delegates to no \
             app and Kampr on Android cannot enrol a passkey here",
        )
        .fix("put the package and its SHA-256 signing certificate in [android] in config.toml — Kampr prints its own when a ceremony fails");
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("kampr-doctor")
        .build()
    {
        Ok(client) => client,
        Err(e) => return Check::warn("assetlinks", format!("no HTTP client to test {origin} with: {e}")),
    };
    let response = match client.get(format!("{origin}{PATH}")).send().await {
        Ok(response) => response,
        // The same reasoning as the origin check: a node that is stopped, or a public hostname
        // this machine's own NAT will not hairpin, is not a broken asset-links file.
        Err(_) => {
            return Check::warn(
                "assetlinks",
                format!("nothing answers at {origin}{PATH} — the origin check says why"),
            )
            .fix("start the node and re-run this");
        }
    };
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    judge(&origin, &document, status, &body)
}

/// Everything that is decided from what came back, separated from getting it.
fn judge(origin: &str, wanted: &str, status: u16, body: &str) -> Check {
    if status != 200 {
        return Check::fail(
            "assetlinks",
            format!(
                "{origin}{PATH} answers {status} — Android reads this before it will let the app \
                 hold a passkey, and a refusal is the end of the ceremony"
            ),
        )
        .fix(format!(
            "stop the proxy answering /.well-known itself and let {PATH} through to the node"
        ));
    }
    let served = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(value) => value,
        Err(_) => {
            return Check::fail(
                "assetlinks",
                format!(
                    "{origin}{PATH} answers 200 with something that is not JSON — whatever is on \
                     that path is not this node"
                ),
            )
            .fix(format!(
                "stop the proxy answering /.well-known itself and let {PATH} through to the node"
            ));
        }
    };
    let (package, fingerprints) = statement(&served);
    if fingerprints.is_empty() {
        return Check::fail(
            "assetlinks",
            format!(
                "{origin}{PATH} names no Android signing certificate, so no build of the app can \
                 be recognised as the one this node delegates to"
            ),
        )
        .fix("set [android] fingerprints in config.toml and restart the node");
    }
    let (_, mine) = statement(&serde_json::from_str(wanted).unwrap_or(serde_json::Value::Null));
    if fingerprints != mine {
        return Check::fail(
            "assetlinks",
            format!(
                "{origin}{PATH} names {}, none of which this node's [android] lists — something \
                 in front of it is serving a copy from before that changed",
                certificates(fingerprints.len())
            ),
        )
        .fix("clear the cache on whatever proxies this hostname, or point it at this node");
    }
    Check::ok(
        "assetlinks",
        format!(
            "{origin}{PATH} delegates {package} to {}, so Kampr on Android can enrol a passkey \
             here",
            certificates(fingerprints.len())
        ),
    )
}

fn certificates(n: usize) -> String {
    if n == 1 {
        "1 certificate".into()
    } else {
        format!("{n} certificates")
    }
}

/// The `android_app` statement, as a package and a set of certificates. A set because the order
/// two tools print the same two certificates in is not a difference worth reporting.
fn statement(document: &serde_json::Value) -> (String, BTreeSet<String>) {
    let target = document
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| &entry["target"])
        .find(|target| target["namespace"] == "android_app");
    let Some(target) = target else {
        return (String::new(), BTreeSet::new());
    };
    (
        target["package_name"].as_str().unwrap_or_default().to_string(),
        target["sha256_cert_fingerprints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str())
            .filter_map(canonical_fingerprint)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::Status;
    use super::*;

    const ORIGIN: &str = "https://kampr.example.com";

    fn served() -> String {
        assetlinks::document(&Config::bootstrap("x").android).expect("the shipped default")
    }

    #[tokio::test]
    async fn a_node_that_cannot_do_passkeys_says_so_once_and_stops() {
        let mut config = Config::bootstrap("x");
        config.server.origin = "http://192.168.1.24:8790".into();
        let check = check(&config).await;
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("cannot do passkeys"), "{}", check.detail);
        assert!(check.fix.is_none(), "there is nothing to fix at tier 0");
    }

    #[test]
    fn the_document_this_node_builds_is_the_one_it_wants_back() {
        let check = judge(ORIGIN, &served(), 200, &served());
        assert_eq!(check.status, Status::Ok, "{}", check.detail);
        assert!(check.detail.contains("dev.kampr.app"), "{}", check.detail);
    }

    #[test]
    fn a_proxy_answering_the_well_known_path_itself_is_a_failure_that_names_it() {
        for (status, body) in [(404, "not found"), (200, "<html>Congratulations!</html>")] {
            let check = judge(ORIGIN, &served(), status, body);
            assert_eq!(check.status, Status::Fail, "{status} {body}");
            assert!(
                check.fix.as_deref().unwrap_or_default().contains("/.well-known"),
                "the fix has to name the path the proxy is eating: {check:?}",
            );
        }
    }

    #[test]
    fn a_copy_naming_a_certificate_this_node_does_not_is_a_failure() {
        let stale = serde_json::json!([{
            "relation": ["delegate_permission/common.get_login_creds"],
            "target": {
                "namespace": "android_app",
                "package_name": "dev.kampr.app",
                "sha256_cert_fingerprints": ["AB".repeat(32)],
            },
        }])
        .to_string();
        let check = judge(ORIGIN, &served(), 200, &stale);
        assert_eq!(check.status, Status::Fail, "{}", check.detail);
        assert!(check.detail.contains("before that changed"), "{}", check.detail);
    }

    #[test]
    fn a_statement_with_no_certificate_at_all_is_a_failure_about_the_certificate() {
        let empty = serde_json::json!([{
            "relation": ["delegate_permission/common.get_login_creds"],
            "target": { "namespace": "android_app", "package_name": "dev.kampr.app", "sha256_cert_fingerprints": [] },
        }])
        .to_string();
        let check = judge(ORIGIN, &served(), 200, &empty);
        assert_eq!(check.status, Status::Fail);
        assert!(
            check.fix.as_deref().unwrap_or_default().contains("fingerprints"),
            "{check:?}"
        );
    }

    // Blanking [android] is a decision, not a broken node — but Android passkeys stop working and
    // nothing else in this report would say so.
    #[tokio::test]
    async fn a_node_that_delegates_to_no_app_warns_rather_than_fails() {
        let mut config = Config::bootstrap("x");
        config.server.origin = ORIGIN.into();
        config.android.fingerprints.clear();
        let check = check(&config).await;
        assert_eq!(check.status, Status::Warn, "{}", check.detail);
        assert!(
            check.fix.as_deref().unwrap_or_default().contains("[android]"),
            "{check:?}"
        );
    }
}
