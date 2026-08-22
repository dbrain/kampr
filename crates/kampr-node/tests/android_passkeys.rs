//! Everything this node has to do before a passkey can exist on a phone.
//!
//! Android's Credential Manager will not run a WebAuthn ceremony for a native app unless the
//! relying party — this node, at the operator's own domain — publishes a Digital Asset Links
//! document naming the app's package and the SHA-256 of the certificate it is signed with. The
//! node is the relying party, so the node serves it, accepts the origin that app signs into its
//! client data, and states the ceremony in the form Android can actually satisfy.
//!
//! Everything here drives the socket rather than the builder: what a phone can read is what came
//! back over TCP, not what a function returned.

use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tokio::net::TcpStream;

const WELL_KNOWN: &str = "/.well-known/assetlinks.json";

/// The relation Credential Manager looks for. `common.handle_all_urls` — the app-links relation —
/// is deliberately absent: an app link declares its hosts at build time and every operator's node
/// is at a different one, so Kampr does not claim to open any URL.
const LOGIN_CREDS: &str = "delegate_permission/common.get_login_creds";

struct Harness {
    node: Arc<Node>,
    /// Where the socket actually is. Not the origin: a Tier 1 node is reached at a hostname a
    /// proxy owns, and these tests still have to dial the port it bound.
    address: (String, u16),
    #[allow(dead_code)]
    home: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.shutdown();
        self.server.abort();
    }
}

impl Harness {
    async fn start(tweak: impl FnOnce(&mut Config)) -> Self {
        let home = tempfile::tempdir().expect("a home");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("assetlinks");
        // Nothing in this suite reaches the internet: the release check is the one thing in a
        // node that would, and a test that phoned GitHub would be one with a rate limit.
        config.update.check = false;
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        tweak(&mut config);
        config.save(&config_dir).expect("a config");

        let node = Node::start(config, &state_dir).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        Self {
            node,
            address: ("127.0.0.1".to_string(), port),
            home,
            server,
        }
    }

    async fn token(&self) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(Role::Full, kampr_auth::Delivery::Console)
            .await
            .expect("a pairing");
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.expect("armed"));
        }
        let body = serde_json::json!({ "code": pairing.code, "device_name": "phone" });
        self.post("/auth/pair", &body.to_string(), None).await.json()["token"]
            .as_str()
            .expect("a token")
            .to_string()
    }

    async fn post(&self, path: &str, body: &str, token: Option<&str>) -> Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (host, port) = (self.address.0.as_str(), self.address.1);
        let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
             {auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut stream = TcpStream::connect((host, port)).await.expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.expect("read");
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        Response {
            status: head.lines().next().unwrap_or_default().to_string(),
            head: head.to_lowercase(),
            body: body.to_string(),
        }
    }

    /// No `Authorization`, no cookie, no `Origin` — a phone that has never paired.
    async fn anonymous(&self, method: &str, path: &str) -> Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (host, port) = (self.address.0.as_str(), self.address.1);
        let request = format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
        let mut stream = TcpStream::connect((host, port)).await.expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.expect("read");
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        Response {
            status: head.lines().next().unwrap_or_default().to_string(),
            head: head.to_lowercase(),
            body: body.to_string(),
        }
    }
}

struct Response {
    status: String,
    head: String,
    body: String,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_str(self.body.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{}", self.body))
    }

    fn target(&self) -> Value {
        let statements = self.json();
        let first = statements
            .as_array()
            .and_then(|s| s.first().cloned())
            .expect("one statement");
        assert!(
            first["relation"]
                .as_array()
                .expect("relations")
                .iter()
                .any(|r| r == LOGIN_CREDS),
            "without {LOGIN_CREDS} Credential Manager will not run a ceremony: {first}"
        );
        first["target"].clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_can_read_it_before_any_credential_exists() {
    let h = Harness::start(|_| {}).await;
    let response = h.anonymous("GET", WELL_KNOWN).await;
    assert!(
        response.status.contains("200"),
        "the file has to be readable before there is anything to authenticate with: {}",
        response.status
    );
    assert!(
        response.head.contains("content-type: application/json"),
        "Digital Asset Links is only read as application/json: {}",
        response.head
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn it_names_the_app_this_node_will_accept_a_passkey_from() {
    let h = Harness::start(|_| {}).await;
    let target = h.anonymous("GET", WELL_KNOWN).await.target();
    assert_eq!(target["namespace"], "android_app");
    assert_eq!(target["package_name"], "dev.kampr.app");
    assert_eq!(
        target["sha256_cert_fingerprints"]
            .as_array()
            .expect("fingerprints"),
        &vec![Value::from(kampr_node::assetlinks::RELEASE_FINGERPRINT)],
        "the default has to be the key the APK a user installs is signed with"
    );
}

/// A fingerprint is copied out of `keytool`, out of `apksigner`, or off a web page, and those
/// three disagree about case and colons. Serving it as typed is a file that parses and never
/// matches.
#[tokio::test(flavor = "multi_thread")]
async fn a_fingerprint_is_canonicalised_however_it_was_typed() {
    let lower = kampr_node::assetlinks::RELEASE_FINGERPRINT.to_lowercase();
    let bare = lower.replace(':', "");
    let h = Harness::start(move |c| {
        c.android.package_name = "dev.kampr.app.debug".into();
        c.android.fingerprints = vec![bare, format!("  {lower}  ")];
    })
    .await;
    let target = h.anonymous("GET", WELL_KNOWN).await.target();
    assert_eq!(target["package_name"], "dev.kampr.app.debug");
    assert_eq!(
        target["sha256_cert_fingerprints"]
            .as_array()
            .expect("fingerprints"),
        &vec![Value::from(kampr_node::assetlinks::RELEASE_FINGERPRINT)],
        "the same certificate written two ways is one fingerprint, upper-case and colon-separated"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn something_that_is_not_a_certificate_digest_is_never_served() {
    let h = Harness::start(|c| {
        c.android.fingerprints = vec![
            "not-a-fingerprint".into(),
            "AA:BB".into(),
            "ZZ:8A:21:84:46:AA:2B:99:08:5C:67:0B:5A:9B:70:32:5E:05:F9:27:CC:DD:12:17:E7:94:63:13:C7:7F:C6:18"
                .into(),
        ];
    })
    .await;
    let response = h.anonymous("GET", WELL_KNOWN).await;
    assert!(
        response.status.contains("404"),
        "a file naming no usable certificate is a promise nothing keeps: {} {}",
        response.status,
        response.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_that_delegates_to_nothing_says_so() {
    let h = Harness::start(|c| c.android.fingerprints.clear()).await;
    let response = h.anonymous("GET", WELL_KNOWN).await;
    assert!(
        response.status.contains("404"),
        "an operator who cleared the list meant it: {}",
        response.status
    );
}

/// The endpoint is unauthenticated by necessity, so it must not be a way to make this node do
/// work. The answer is built once at startup and is the same bytes whatever the request says.
#[tokio::test(flavor = "multi_thread")]
async fn it_is_a_static_bounded_answer_rather_than_a_new_unauthenticated_surface() {
    let h = Harness::start(|_| {}).await;
    let plain = h.anonymous("GET", WELL_KNOWN).await;
    let noisy = h
        .anonymous("GET", &format!("{WELL_KNOWN}?{}", "x".repeat(2048)))
        .await;
    assert_eq!(plain.body, noisy.body, "the request cannot change the answer");
    assert!(
        plain.body.len() < 1024,
        "a bounded answer, not a document that grows with the device list: {} bytes",
        plain.body.len()
    );
    let written = h.anonymous("POST", WELL_KNOWN).await;
    assert!(written.status.contains("405"), "read-only: {}", written.status);
}

/// The file and the ceremony are two halves of one promise. Publishing an asset link for an app
/// whose Credential Manager origin the WebAuthn engine then refuses is the worst of both: the
/// phone offers the passkey, the owner approves it, and the node rejects it at `finish`.
#[tokio::test(flavor = "multi_thread")]
async fn every_app_the_file_names_may_actually_finish_a_ceremony() {
    let h = Harness::start(|c| {
        // Tier 1: a hostname with a certificate, which is the only place passkeys exist at all.
        c.server.origin = "https://kampr.example.com".into();
    })
    .await;
    let target = h.anonymous("GET", WELL_KNOWN).await.target();
    let published = target["sha256_cert_fingerprints"]
        .as_array()
        .expect("fingerprints")
        .iter()
        .map(|f| f.as_str().expect("a string").to_string())
        .collect::<Vec<_>>();
    assert!(!published.is_empty());

    let engine = h.node.auth.passkeys().expect("a passkey engine at tier 1");
    let origins = engine.allowed_origins();
    for fingerprint in published {
        let origin = kampr_auth::android::credential_manager_origin(&fingerprint).expect("an app origin");
        assert!(
            origins.contains(&origin),
            "the file delegates to {fingerprint} but the engine would refuse {origin}: {origins:?}"
        );
    }
}

/// The half a correct `assetlinks.json` does not buy.
///
/// `webauthn-rs`'s generic passkey options ask for a non-discoverable credential with no
/// attachment and a `credProtect` extension, and its own documentation says Android cannot satisfy
/// that: GMS does not perform authenticator selection, so the relying party has to pre-select. A
/// phone sent the browser's option set is offered a security key it does not have.
#[tokio::test(flavor = "multi_thread")]
async fn a_phone_is_asked_for_the_ceremony_a_phone_can_perform() {
    let h = Harness::start(|c| c.server.origin = "https://kampr.example.com".into()).await;
    let token = h.token().await;
    let android = h
        .post(
            "/auth/webauthn/register/start",
            &json!({ "device_name": "Pixel", "platform": "android" }).to_string(),
            Some(&token),
        )
        .await
        .json();
    let key = &android["options"]["publicKey"];
    assert_eq!(
        key["authenticatorSelection"]["residentKey"], "required",
        "a passkey Android will make is a discoverable one: {key}"
    );
    assert_eq!(key["authenticatorSelection"]["requireResidentKey"], true);
    assert_eq!(
        key["authenticatorSelection"]["authenticatorAttachment"], "platform",
        "the screen lock, not a security key: {key}"
    );
    assert!(
        key["extensions"].get("credentialProtectionPolicy").is_none(),
        "credProtect is the extension Android has no answer for: {key}"
    );
    assert!(!key["challenge"].as_str().unwrap_or_default().is_empty());
}

/// The browser path was verified end to end against a virtual authenticator. A phone asking for
/// its own option set must not change what a browser is handed.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_still_gets_the_option_set_it_was_verified_with() {
    let h = Harness::start(|c| c.server.origin = "https://kampr.example.com".into()).await;
    let token = h.token().await;
    let browser = h
        .post(
            "/auth/webauthn/register/start",
            &json!({ "device_name": "Firefox" }).to_string(),
            Some(&token),
        )
        .await
        .json();
    let key = &browser["options"]["publicKey"];
    assert_eq!(key["authenticatorSelection"]["residentKey"], "discouraged");
    assert_eq!(
        key["extensions"]["credentialProtectionPolicy"], "userVerificationRequired",
        "{key}"
    );
}

/// The keystore and the APK are the only authorities on what Kampr is signed with. Asserting the
/// constant against itself would prove nothing, so this asks the tools that read the artefact.
///
/// Absent tooling is a loud skip rather than a silent pass; `KAMPR_ANDROID_CERT=1` makes it a
/// failure, which is what CI sets.
#[test]
fn the_default_is_the_certificate_the_release_apk_is_actually_signed_with() {
    let required = std::env::var_os("KAMPR_ANDROID_CERT").is_some();
    let found = fingerprint_of_apk().or_else(fingerprint_of_keystore);
    let Some((source, actual)) = found else {
        let why = "assetlinks cert check SKIPPED — no release APK and no keystore to read";
        assert!(!required, "{why}, and KAMPR_ANDROID_CERT demanded it");
        eprintln!("\n{}\n  {why}\n{}\n", "!".repeat(78), "!".repeat(78));
        return;
    };
    assert_eq!(
        actual,
        kampr_node::assetlinks::RELEASE_FINGERPRINT,
        "the default in assetlinks.rs no longer matches {source} — every installed \
         Kampr would be refused a passkey"
    );
}

fn fingerprint_of_apk() -> Option<(String, String)> {
    let apk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../client/androidApp/build/outputs/apk/release/androidApp-release.apk");
    if !apk.exists() {
        return None;
    }
    let output = Command::new("apksigner")
        .args(["verify", "--print-certs"])
        .arg(&apk)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let digest = text
        .lines()
        .find_map(|line| line.split_once("certificate SHA-256 digest: ").map(|(_, d)| d))?;
    Some((format!("{}", apk.display()), colonise(digest.trim())))
}

fn fingerprint_of_keystore() -> Option<(String, String)> {
    let store = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)?
        .join(".android-keystores/kampr-release.jks");
    if !store.exists() {
        return None;
    }
    let password = gradle_property("kamprReleaseStorePassword")?;
    let output = Command::new("keytool")
        .arg("-list")
        .arg("-v")
        .arg("-keystore")
        .arg(&store)
        .arg("-storepass")
        .arg(password)
        .arg("-alias")
        .arg(gradle_property("kamprReleaseKeyAlias").unwrap_or_else(|| "kampr".into()))
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let digest = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("SHA256: "))?;
    Some((format!("{}", store.display()), digest.trim().to_uppercase()))
}

fn gradle_property(name: &str) -> Option<String> {
    let path = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)?
        .join(".gradle/gradle.properties");
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{name}=")).map(str::to_string))
}

fn colonise(hex: &str) -> String {
    if hex.contains(':') {
        return hex.to_uppercase();
    }
    hex.to_uppercase()
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_string())
        .collect::<Vec<_>>()
        .join(":")
}
