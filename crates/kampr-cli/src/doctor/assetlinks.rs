use super::{Check, Status};
use kampr_auth::Tier;
use kampr_auth::android::canonical_fingerprint;
use kampr_node::{Config, assetlinks};
use std::collections::BTreeSet;
use std::time::Duration;

const PATH: &str = "/.well-known/assetlinks.json";
const RELATION: &str = "delegate_permission/common.get_login_creds";
const API: &str = "https://digitalassetlinks.googleapis.com/v1/statements:list";

/// Whether **this node's half** of the Android association is right.
///
/// It is a half and not the whole. Credential Manager checks the agreement in both directions, and
/// the app's direction is a `asset_statements` entry compiled into its APK naming the sites it may
/// hold passkeys for (#288). No node can see that, and the published APK declares none — so a green
/// here says the node is not what is wrong, and never that a passkey can be created.
///
/// Credential Manager runs no ceremony for a native app until *Google's* Digital Asset Links
/// validator has fetched this file, server-side and from the public internet, and found the app's
/// package and signing certificate in it. The phone never reads it. So asking the node's own
/// origin — which is what this check used to do — answers a question nobody is asking: a hostname
/// that resolves publicly to an RFC1918 address serves the file perfectly to everything on the LAN
/// and is refused on the phone anyway (#170). The deciding party is asked directly.
///
/// It is asked about the **RP ID**, not the origin's host. WebAuthn allows an RP ID that is a
/// registrable suffix of the origin, and Google validates the RP ID's well-known location — so a
/// node at `https://kampr.example.com` with `[auth] rp_id = "example.com"` is decided by a file on
/// `example.com` and by nothing the node itself serves.
pub async fn check(config: &Config) -> Check {
    let origin = config.origin();
    // Below tier 1 there is no ceremony to fail: Credential Manager needs an RP ID, an RP ID is a
    // registrable domain, and the tier check has already said so at length. Quiet rather than a
    // second paragraph about a file nothing will read.
    let Some(rp_id) = relying_party(config) else {
        return Check::ok(
            "assetlinks",
            format!("not in play: {origin} cannot do passkeys at all, so nothing reads {PATH}"),
        );
    };
    let Some(document) = assetlinks::document(&config.android) else {
        return Check::warn(
            "assetlinks",
            "[android] names no package and no usable certificate, so this node delegates to no \
             app and Kampr on Android cannot enrol a passkey here",
        )
        .fix("put the package and its SHA-256 signing certificate in [android] in config.toml — Kampr prints its own when a ceremony fails");
    };
    let asked = ask(&rp_id).await;
    let verdict = judge(&rp_id, &document, &asked, None);
    if verdict.status == Status::Ok {
        return verdict;
    }
    // The same URL from a second vantage point, and only when the first one went badly: served
    // correctly from here and unfetchable by Google is a public DNS record pointing somewhere
    // private (#170), while unserved from here too is a proxy eating /.well-known (#122). Two
    // different fixes that are indistinguishable from Google's answer alone.
    let here = fetch(&format!("https://{rp_id}{PATH}")).await;
    judge(&rp_id, &document, &asked, Some(&here))
}

fn relying_party(config: &Config) -> Option<String> {
    let tier = Tier::detect(&config.origin()).ok()?;
    match config.auth.rp_id.trim() {
        "" => tier.rp_id,
        overridden => tier.with_rp_id(overridden).rp_id,
    }
}

/// Google's answer, or the reason there is not one. "I could not ask" is not a verdict.
enum Asked {
    Unaskable(String),
    Answered { status: u16, body: String },
}

/// What this machine sees at the same URL Google was asked about.
enum Local {
    Unreachable,
    Served { status: u16, body: String },
}

async fn ask(rp_id: &str) -> Asked {
    let client = match http() {
        Some(client) => client,
        None => return Asked::Unaskable("no HTTP client to ask with".into()),
    };
    let site = format!("https://{rp_id}");
    let Ok(url) = url::Url::parse_with_params(
        &endpoint(),
        &[("source.web.site", site.as_str()), ("relation", RELATION)],
    ) else {
        return Asked::Unaskable(format!("{} is not a URL to ask", endpoint()));
    };
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            Asked::Answered {
                status,
                body: response.text().await.unwrap_or_default(),
            }
        }
        Err(e) => Asked::Unaskable(chain(&e)),
    }
}

async fn fetch(url: &str) -> Local {
    let Some(client) = http() else {
        return Local::Unreachable;
    };
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            Local::Served {
                status,
                body: response.text().await.unwrap_or_default(),
            }
        }
        Err(_) => Local::Unreachable,
    }
}

fn http() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .user_agent("kampr-doctor")
        .build()
        .ok()
}

/// The test seam. `doctor`'s suite must not depend on Google being up, or on there being a
/// network at all, so the one address this reaches out to is overridable.
fn endpoint() -> String {
    std::env::var("KAMPR_ASSETLINKS_API").unwrap_or_else(|_| API.to_string())
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

/// Everything that is decided from what came back, separated from getting it.
///
/// Google caches a `statements:list` answer for its own `maxAge` — 600 s at the short end and a
/// little under 3000 s at the long one — so every fix below says to wait rather than to re-run,
/// and a green here is at worst about ten minutes stale.
fn judge(rp_id: &str, wanted: &str, asked: &Asked, here: Option<&Local>) -> Check {
    let file = format!("https://{rp_id}{PATH}");
    let (asked_status, body) = match asked {
        Asked::Unaskable(detail) => {
            return Check::warn(
                "assetlinks",
                format!(
                    "could not ask Google whether it can read {file}: {detail} — Android decides \
                     an RP ID by that answer and by nothing this node says, so whether a passkey \
                     can be created here is unestablished, not broken"
                ),
            )
            .fix("re-run this from a machine that can reach digitalassetlinks.googleapis.com");
        }
        Asked::Answered { status, body } => (*status, body),
    };
    if asked_status != 200 {
        return Check::warn(
            "assetlinks",
            format!(
                "digitalassetlinks.googleapis.com answered {asked_status} instead of a verdict on \
                 {file}, so whether Android will accept a passkey here is unestablished"
            ),
        )
        .fix("re-run this in a few minutes; the check is about Google's fetcher, not this node");
    }
    let answer = parsed(body);
    let (package, fingerprints) = statement(&parsed(wanted));
    let (theirs, published) = delegated(&answer);
    let codes = error_codes(&answer);
    if published.is_empty() {
        let refused = format!(
            "Google cannot read {file}{} — Android never fetches that file itself, so Credential \
             Manager refuses every passkey here with \"RP ID cannot be validated\"{}",
            if codes.is_empty() {
                " and returns no statement for it".to_string()
            } else {
                format!(" ({})", codes.join(", "))
            },
            vantage(here, &file),
        );
        let document_is_right = matches!(here, Some(Local::Served { status: 200, body }) if statement(&parsed(body)).1 == fingerprints);
        return Check::fail("assetlinks", refused).fix(if document_is_right {
            format!(
                "the document is right, so this is DNS: make {rp_id} resolve from the public \
                 internet to an address Google can reach — a public record pointing at an RFC1918 \
                 address is the usual cause. Google caches its answer for ten minutes or more, so \
                 re-run this after that and not before"
            )
        } else {
            format!(
                "serve {PATH} from {rp_id} over the public internet: stop whatever answers \
                 /.well-known there from answering it itself, and let it through to this node"
            )
        });
    }
    if !theirs.contains(&package) || published != fingerprints {
        return Check::fail(
            "assetlinks",
            format!(
                "{file} and this node's [android] have drifted: Google reads {} signed by {}, and \
                 this node names {package} signed by {}. Google's copy is the one Android obeys",
                or_nothing(&theirs),
                or_nothing(&published),
                or_nothing(&fingerprints),
            ),
        )
        .fix(format!(
            "make the copy served from {rp_id} match [android] in config.toml, then re-run this \
             ten minutes later — Google caches what it read"
        ));
    }
    Check::ok(
        "assetlinks",
        format!(
            "Google reads {file} and finds {package} delegated to {} — this node's half of the \
             association is right. The app's half is compiled into its APK, so whether a given \
             build may enrol here is not something this node can see",
            certificates(published.len())
        ),
    )
}

/// What separates "the file is wrong" from "Google cannot reach the right file": the same URL,
/// fetched from this machine, which is on the operator's own network.
fn vantage(here: Option<&Local>, file: &str) -> String {
    match here {
        None => String::new(),
        Some(Local::Unreachable) => ". This machine cannot fetch it either".into(),
        Some(Local::Served { status: 200, .. }) => format!(
            ". This machine reads {file} fine, so the document is not what is broken — the \
             hostname is unreachable from the internet"
        ),
        Some(Local::Served { status, .. }) => {
            format!(". This machine gets {status} from it too")
        }
    }
}

fn or_nothing(values: &BTreeSet<String>) -> String {
    match values.is_empty() {
        true => "nothing".into(),
        false => values.iter().cloned().collect::<Vec<_>>().join(", "),
    }
}

fn parsed(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
}

/// `errorCode` is a repeated enum, so it arrives as an array — but a single string is the shape
/// half the documentation shows and neither is worth a surprise.
fn error_codes(answer: &serde_json::Value) -> Vec<String> {
    match &answer["errorCode"] {
        serde_json::Value::Array(list) => list
            .iter()
            .filter_map(|c| c.as_str())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(one) => vec![one.clone()],
        _ => Vec::new(),
    }
}

/// What Google says the RP ID delegates, as packages and certificates. Its own JSON, which is
/// camel-cased and one certificate per statement rather than the node's array.
fn delegated(answer: &serde_json::Value) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut packages = BTreeSet::new();
    let mut fingerprints = BTreeSet::new();
    for statement in answer["statements"].as_array().into_iter().flatten() {
        if !relates(&statement["relation"]) {
            continue;
        }
        let app = &statement["target"]["androidApp"];
        let Some(package) = app["packageName"].as_str() else {
            continue;
        };
        let Some(fingerprint) = app["certificate"]["sha256Fingerprint"]
            .as_str()
            .and_then(canonical_fingerprint)
        else {
            continue;
        };
        packages.insert(package.to_string());
        fingerprints.insert(fingerprint);
    }
    (packages, fingerprints)
}

fn certificates(n: usize) -> String {
    if n == 1 {
        "1 certificate".into()
    } else {
        format!("{n} certificates")
    }
}

/// The `android_app` statement of the document *this node builds*, as a package and a set of
/// certificates. A set because the order two tools print the same two certificates in is not a
/// difference worth reporting.
fn statement(document: &serde_json::Value) -> (String, BTreeSet<String>) {
    let target = document
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| relates(&entry["relation"]))
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

fn relates(relation: &serde_json::Value) -> bool {
    match relation {
        serde_json::Value::Array(list) => list.iter().any(|r| r == RELATION),
        other => other == RELATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RP: &str = "kampr.example.com";

    fn served() -> String {
        assetlinks::document(&Config::bootstrap("x").android).expect("the shipped default")
    }

    fn google(statements: &str) -> Asked {
        Asked::Answered {
            status: 200,
            body: statements.into(),
        }
    }

    fn confirming(package: &str, fingerprint: &str) -> Asked {
        google(
            &serde_json::json!({
                "statements": [{
                    "source": { "web": { "site": format!("https://{RP}.") } },
                    "relation": RELATION,
                    "target": { "androidApp": {
                        "packageName": package,
                        "certificate": { "sha256Fingerprint": fingerprint },
                    }},
                }],
                "maxAge": "599.9s",
            })
            .to_string(),
        )
    }

    fn release() -> Asked {
        confirming("dev.kampr.app", assetlinks::RELEASE_FINGERPRINT)
    }

    #[test]
    fn google_agreeing_with_this_node_is_the_only_green_there_is() {
        let check = judge(RP, &served(), &release(), None);
        assert_eq!(check.status, Status::Ok, "{}", check.detail);
        assert!(
            check.detail.contains(RP),
            "the verified host has to be named: {}",
            check.detail
        );
        assert!(check.detail.contains("dev.kampr.app"), "{}", check.detail);
    }

    /// The green used to end "so Kampr on Android can enrol a passkey here", and that sentence was
    /// false for every build of the app anyone has ever installed: the app's half of the
    /// association is compiled into its APK and the published one declares none (#288). An
    /// operator whose ceremony had just failed was sent here and told the node was fine — which it
    /// was, and which was not what they asked.
    #[test]
    fn a_green_says_the_node_is_right_and_never_that_a_passkey_can_be_created() {
        let detail = judge(RP, &served(), &release(), None).detail;
        assert!(
            !detail.contains("can enrol"),
            "a check that cannot see the app must not promise what the app will do: {detail}"
        );
        assert!(
            detail.contains("half"),
            "and it has to say which half it answered: {detail}"
        );
    }

    #[test]
    fn a_file_google_cannot_fetch_is_a_failure_that_blames_googles_fetch_and_not_the_node() {
        let refused = google(r#"{"errorCode":["ERROR_CODE_FETCH_ERROR"],"maxAge":"600s"}"#);
        let here = Local::Served {
            status: 200,
            body: served(),
        };
        let check = judge(RP, &served(), &refused, Some(&here));
        assert_eq!(check.status, Status::Fail, "{}", check.detail);
        assert!(check.detail.contains("Google"), "{}", check.detail);
        assert!(
            check.detail.contains("refuse") || check.detail.contains("refused"),
            "it has to say the phone will be refused: {}",
            check.detail
        );
        assert!(
            check.detail.to_lowercase().contains("this machine reads"),
            "the second vantage point is what separates this from a broken file: {}",
            check.detail
        );
        assert!(
            check.fix.as_deref().unwrap_or_default().contains("resolve"),
            "and the fix is DNS, not the document: {check:?}"
        );
    }

    #[test]
    fn a_certificate_google_reads_that_this_node_does_not_name_is_drift_that_names_both() {
        let theirs = "AB".repeat(32).chunked();
        let check = judge(RP, &served(), &confirming("dev.kampr.app", &theirs), None);
        assert_eq!(check.status, Status::Fail, "{}", check.detail);
        assert!(
            check.detail.contains(&theirs),
            "the copy Google reads: {}",
            check.detail
        );
        assert!(
            check.detail.contains(assetlinks::RELEASE_FINGERPRINT),
            "and the one this node names: {}",
            check.detail
        );
    }

    #[test]
    fn a_package_google_reads_that_this_node_does_not_name_is_drift_that_names_both() {
        let check = judge(
            RP,
            &served(),
            &confirming("com.example.other", assetlinks::RELEASE_FINGERPRINT),
            None,
        );
        assert_eq!(check.status, Status::Fail, "{}", check.detail);
        assert!(check.detail.contains("com.example.other"), "{}", check.detail);
        assert!(check.detail.contains("dev.kampr.app"), "{}", check.detail);
    }

    // The distinction the whole check turns on: an unanswered question is not a verdict. A node
    // on a machine with no route to Google is not a node whose passkeys are broken.
    #[test]
    fn a_question_this_machine_could_not_ask_is_a_warning_and_never_a_failure() {
        let offline = Asked::Unaskable("dns error: failed to lookup address information".into());
        let check = judge(RP, &served(), &offline, Some(&Local::Unreachable));
        assert_eq!(check.status, Status::Warn, "{}", check.detail);
        assert!(
            check.detail.contains("could not") || check.detail.contains("unestablished"),
            "it has to say what could not be established: {}",
            check.detail
        );

        let broken = Asked::Answered {
            status: 503,
            body: "upstream unavailable".into(),
        };
        assert_eq!(judge(RP, &served(), &broken, None).status, Status::Warn);
    }

    #[test]
    fn google_answering_with_no_statement_at_all_is_the_same_refusal_as_a_fetch_error() {
        let empty = google(r#"{"maxAge":"600s"}"#);
        let check = judge(RP, &served(), &empty, Some(&Local::Unreachable));
        assert_eq!(check.status, Status::Fail, "{}", check.detail);
    }

    // A statement under some other relation delegates a passkey nothing at all.
    #[test]
    fn only_the_login_relation_counts_on_either_side() {
        let urls_only = google(
            &serde_json::json!({
                "statements": [{
                    "relation": "delegate_permission/common.handle_all_urls",
                    "target": { "androidApp": {
                        "packageName": "dev.kampr.app",
                        "certificate": { "sha256Fingerprint": assetlinks::RELEASE_FINGERPRINT },
                    }},
                }],
            })
            .to_string(),
        );
        assert_eq!(judge(RP, &served(), &urls_only, None).status, Status::Fail);
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

    // Blanking [android] is a decision, not a broken node — but Android passkeys stop working and
    // nothing else in this report would say so.
    #[tokio::test]
    async fn a_node_that_delegates_to_no_app_warns_rather_than_fails() {
        let mut config = Config::bootstrap("x");
        config.server.origin = format!("https://{RP}");
        config.android.fingerprints.clear();
        let check = check(&config).await;
        assert_eq!(check.status, Status::Warn, "{}", check.detail);
        assert!(
            check.fix.as_deref().unwrap_or_default().contains("[android]"),
            "{check:?}"
        );
    }

    // The apex case the operator is about to create: the node stays where it is and the file that
    // decides moves to the registrable domain above it. A check aimed at the origin's host would
    // be interrogating a hostname that no longer decides anything.
    #[test]
    fn the_host_asked_about_is_the_rp_id_and_not_the_origins_own() {
        let mut config = Config::bootstrap("x");
        config.server.origin = "https://kampr.example.net".into();
        assert_eq!(relying_party(&config).as_deref(), Some("kampr.example.net"));
        config.auth.rp_id = "example.net".into();
        assert_eq!(relying_party(&config).as_deref(), Some("example.net"));

        // An override never resurrects a passkey the origin cannot carry.
        config.server.origin = "http://192.168.1.24:8790".into();
        assert_eq!(relying_party(&config), None);
    }

    trait Chunked {
        fn chunked(&self) -> String;
    }

    impl Chunked for String {
        fn chunked(&self) -> String {
            self.as_bytes()
                .chunks(2)
                .map(|c| String::from_utf8_lossy(c).into_owned())
                .collect::<Vec<_>>()
                .join(":")
        }
    }
}
