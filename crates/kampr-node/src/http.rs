use crate::assetlinks;
use crate::assets;
use crate::attach;
use crate::session;
use crate::state::{BUILD, Node};
use anyhow::{Context, Result};
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HeaderMap, HeaderName,
    IF_NONE_MATCH, ORIGIN, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kampr_auth::{AuthError, Delivery, Device, Role};
use kampr_core::wire::ErrorCode;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;

/// `'wasm-unsafe-eval'` is what a Compose Multiplatform wasm bundle needs and is strictly weaker
/// than `'unsafe-eval'`: it permits WebAssembly compilation and nothing else.
///
/// The `style-src` hash covers the single `<style>` CMP injects into its shadow root:
/// `:host { -webkit-touch-callout: none; -webkit-user-select: none; user-select: none;
/// position: relative }`. Dropping it costs an OS long-press callout over the terminal and the
/// positioning ancestor the offscreen input anchors to. A hash rather than `'unsafe-inline'`
/// because pane output is the most attacker-influenced surface here — and note a browser ignores
/// `'unsafe-inline'` entirely once any hash is present, so the two cannot be combined as a
/// belt-and-braces. If a CMP upgrade changes that rule, the console names the expected hash.
///
/// `connect-src 'self'` and nothing else. A bare `ws:` / `wss:` matches *any* host, which hands
/// anything that gets a script onto this page a clean channel out; CSP3 resolves `'self'` to the
/// same-origin WebSocket, so the schemes bought nothing.
const CSP: &str = "default-src 'self'; \
     script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'sha256-+bHRyQ0Z1/Lb6dgSILtTESBRCIFl8jkBb/dPQA4Pdnw='; \
     img-src 'self' data: blob:; \
     font-src 'self' data:; \
     connect-src 'self'; \
     worker-src 'self' blob:; \
     frame-ancestors 'none'; base-uri 'none'; object-src 'none'; form-action 'none'";

const TOKEN_PROTOCOL: &str = "kampr.token.";

/// The largest client message the node will read.
///
/// Every message but one is small — a `prefs` blob is capped at 2 KiB by the store — and
/// tungstenite's own default of 64 MiB is a `serde_json::Value` a stranger can ask this node to
/// build. The exception is `paste`, which carries a file the operator chose, and it is the reason
/// this may not simply be a round number: the client refuses at [`kampr_journal::attach::MAX_BYTES`]
/// and *says so on screen*, so everything under that must fit through here. A 1 MiB ceiling meant
/// every attachment between the two was encoded, sent, and killed the socket mid-write — with the
/// composer already reading "sent", because a paste has no acknowledgement to withdraw.
///
/// Base64 is four bytes for every three, and the envelope around it is a pane id and a filename.
const MAX_MESSAGE_BYTES: usize = (kampr_journal::attach::MAX_BYTES as usize).div_ceil(3) * 4 + 64 * 1024;

/// The same bound for `/mesh`, which cannot be the same number.
///
/// A mesh link is the client protocol backwards, so the hub *reads* a peer's **server** frames —
/// including a scrollback document, sized at roughly 4 MB for a pane deep enough to fill the ring
/// (`kampr_core::scrollback::DEFAULT_MAX_ROWS`). 16 MiB is what tungstenite already enforced per
/// *frame*, so it is the real ceiling on anything a peer can send today; applying it as the
/// message ceiling too cuts that from 64 MiB and changes nothing that works. What bounds the
/// anonymous half of this endpoint is the handshake semaphore, not this.
const MAX_MESH_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub fn router(node: Arc<Node>) -> Router {
    let secured = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
    };
    // Built here rather than per request: the one endpoint that has to answer a stranger is the
    // one that must not do work for one.
    let asset_links = assetlinks::document(&node.config.android);
    let hsts = node
        .config
        .server
        .tls
        .enabled
        .then(|| secured(STRICT_TRANSPORT_SECURITY, "max-age=31536000; includeSubDomains"));
    Router::new()
        .route("/ws", get(websocket))
        // Unauthenticated by necessity: Android reads this to decide whether the app asking for a
        // passkey is the app this node delegates to, which is a question that arises before any
        // credential exists.
        .route(
            "/.well-known/assetlinks.json",
            get(move || std::future::ready(asset_links_response(asset_links.clone()))),
        )
        .route("/mesh", get(mesh_socket))
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/node", get(node_info))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}/revoke", post(revoke_device))
        .route("/api/devices/{id}/role", post(set_role))
        .route("/api/devices/{id}/renew", post(renew_device))
        .route("/api/pair", post(create_pairing))
        .route("/api/mesh", get(mesh_state))
        .route("/api/mesh/invite", post(create_mesh_invite))
        .route("/api/mesh/{id}/revoke", post(revoke_mesh_node))
        .route("/api/warm", get(warm))
        .route("/api/attachment/{*locator}", get(attachment))
        .route("/api/push", get(push_state))
        .route("/api/push/subscribe", post(push_subscribe))
        .route("/api/push/unsubscribe", post(push_unsubscribe))
        .route("/api/push/rules", post(push_rule))
        .route("/auth/pair", post(redeem_pairing))
        .route("/auth/webauthn/register/start", post(register_start))
        .route("/auth/webauthn/register/finish", post(register_finish))
        .route("/auth/webauthn/authenticate/start", post(authenticate_start))
        .route("/auth/webauthn/authenticate/finish", post(authenticate_finish))
        .fallback(get(static_asset))
        // The wasm bundle is ~12 MB of the ~13 MB first load, it is content-hashed so every
        // release is a fresh URL, and `application/wasm` is in no reverse proxy's default
        // `gzip_types` — so nothing downstream compresses it if this does not. Brotli and gzip
        // only: `zstd` buys a few per cent over brotli on a bundle no browser asks for it on.
        // The layer is outside the WebSocket routes' concern by construction — a 101 has no body.
        .layer(CompressionLayer::new().br(true).gzip(true))
        .layer(secured(CONTENT_SECURITY_POLICY, CSP))
        .layer(secured(X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(secured(X_FRAME_OPTIONS, "DENY"))
        .layer(secured(REFERRER_POLICY, "no-referrer"))
        .layer(tower::util::option_layer(hsts))
        .with_state(node)
}

pub async fn serve(node: Arc<Node>) -> Result<()> {
    let served = std::pin::pin!(listen(node.clone()));
    let outcome = tokio::select! {
        outcome = served => outcome,
        signal = interrupted() => {
            tracing::info!(%signal, "shutting down");
            Ok(())
        }
    };
    node.restore_desks().await;
    outcome
}

async fn listen(node: Arc<Node>) -> Result<()> {
    let addr = node.config.bind_addr().context("server.bind")?;
    let app = router(node.clone());
    if node.config.server.tls.enabled {
        return serve_tls(node, addr, app).await;
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    serve_on(listener, app).await
}

/// **Nothing is drained.** The signal is caught for the one thing that has to happen off this
/// process — putting back an agents view this node set at somebody's desk, which nothing else
/// knows about — and then the node goes, exactly as fast as it did when the signal killed it.
async fn interrupted() -> &'static str {
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(term) => term,
            Err(_) => return std::future::pending().await,
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT",
            _ = term.recv() => "SIGTERM",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    }
}

pub async fn serve_on(listener: tokio::net::TcpListener, app: Router) -> Result<()> {
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("serving http")
}

/// Terminating TLS in-process is the alternative to a reverse proxy, not a replacement for one:
/// a certificate for a hostname is what moves a node off Tier 0, and this is only the half of
/// that a node can do for itself.
async fn serve_tls(node: Arc<Node>, addr: SocketAddr, app: Router) -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tls = &node.config.server.tls;
    let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key)
        .await
        .with_context(|| format!("loading {} and {}", tls.cert, tls.key))?;
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("serving https")
}

pub struct Authenticated {
    pub device: Device,
    pub peer: String,
}

/// The address a rate limit is keyed on, resolved once and the same way for every endpoint.
pub struct Peer(pub String);

impl FromRequestParts<Arc<Node>> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, node: &Arc<Node>) -> Result<Self, Self::Rejection> {
        Ok(Self(peer_of(
            node,
            &parts.headers,
            parts.extensions.get::<ConnectInfo<SocketAddr>>(),
        )))
    }
}

impl FromRequestParts<Arc<Node>> for Authenticated {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, node: &Arc<Node>) -> Result<Self, Response> {
        let peer = peer_of(
            node,
            &parts.headers,
            parts.extensions.get::<ConnectInfo<SocketAddr>>(),
        );
        if !same_origin(node, &parts.headers, &parts.method, &parts.uri) {
            return Err(refuse(StatusCode::FORBIDDEN, "cross-origin request refused"));
        }
        let token =
            bearer(&parts.headers).ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "no device token"))?;
        node.auth
            .authenticate(&token, &peer)
            .await
            .map(|device| Self { device, peer })
            .map_err(auth_rejection)
    }
}

impl Authenticated {
    /// `Some` is the refusal to return; `None` means the device may write.
    ///
    /// Recorded, because this is the same gap the socket had (probe #125): the device inventory
    /// and the pairing surface are exactly what a half-trusted device probes, and a 403 that
    /// leaves no line behind is a probe nobody can see. `verb` names the surface, and it is what
    /// the loop rule counts.
    fn refused(&self, node: &Node, verb: &str) -> Option<Response> {
        if self.device.role.writes() {
            return None;
        }
        node.auth.record_refusal(
            &self.device,
            &self.peer,
            verb,
            None,
            json!({ "code": ErrorCode::NotWriter }),
        );
        Some(refuse(StatusCode::FORBIDDEN, "this device is read-only"))
    }
}

fn auth_rejection(error: AuthError) -> Response {
    match error {
        AuthError::RateLimited => refuse(StatusCode::TOO_MANY_REQUESTS, "too many attempts"),
        _ => refuse(StatusCode::UNAUTHORIZED, "this device is not authorised"),
    }
}

/// Anything carrying a token or a device list. `no-store` because a shared cache between the
/// phone and the node must not keep either.
fn private_json(value: Value) -> Response {
    (StatusCode::OK, [(CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

/// A `StoreError` names the database path. The operator gets it in the log; the client gets the
/// status code.
fn store_failure(context: &str, error: &dyn std::fmt::Display) -> Response {
    tracing::error!(%context, error = %error, "device store call failed");
    refuse(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the device store is unavailable",
    )
}

pub fn refuse(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(CACHE_CONTROL, "no-store")],
        Json(json!({ "error": message })),
    )
        .into_response()
}

/// The address a rate limit is keyed on.
///
/// `X-Forwarded-For` is believed only when `trust_proxy` is set, because anyone who can reach the
/// node directly can forge it — and a forged one would hand an attacker a fresh rate-limit bucket
/// per guess.
fn peer_of(node: &Node, headers: &HeaderMap, connect: Option<&ConnectInfo<SocketAddr>>) -> String {
    if node.config.server.trust_proxy
        && let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        // The list grows left to right and only the last entry was written by the proxy in front
        // of us; everything to its left is whatever the client chose to send. Reading the head
        // gives a rotating header a fresh bucket per request.
        && let Some(nearest) = forwarded.rsplit(',').next()
        && !nearest.trim().is_empty()
    {
        return nearest.trim().to_string();
    }
    connect.map_or_else(|| "unknown".to_string(), |c| c.0.ip().to_string())
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok())
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        return Some(token.trim().to_string());
    }
    subprotocol_token(headers)
}

/// A browser cannot set headers on a WebSocket handshake, so the token rides in the subprotocol —
/// and the node has to echo the exact value back or the handshake fails.
fn subprotocol_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())?
        .split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix(TOKEN_PROTOCOL))
        .map(str::to_string)
}

/// A browser sends `Origin` on every WebSocket upgrade and every cross-origin write, so requiring
/// it to match a list the *node* owns is what stops a page on another origin from driving it.
///
/// The list comes from the bind address and `extra_origins`, never from the request's own `Host`:
/// a DNS-rebinding attacker who points a domain at this node's address controls both headers, so
/// reflecting `Host` lets them satisfy the gate with their own claim.
///
/// `GET` outside `/ws` is deliberately exempt, and that is only safe because **no credential this
/// node accepts is ambient**: a bearer header and a WebSocket subprotocol both have to be set by
/// the caller, and a cross-origin page cannot set either. Reintroducing a cookie credential would
/// turn every un-gated `GET` into a CSRF read, so it would have to gate them too.
fn same_origin(node: &Node, headers: &HeaderMap, method: &Method, uri: &Uri) -> bool {
    let guarded = uri.path() == "/ws" || !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if !guarded {
        return true;
    }
    match headers.get(ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(origin) => node.allowed_origins.iter().any(|a| a == origin),
        // No `Origin` is a non-browser client, which cannot be tricked into making the request.
        None => true,
    }
}

async fn websocket(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Authenticated { device, peer } = auth;
    // One token opens as many sockets as its holder likes and each is a session with its own
    // queue and its own pane pumps, so the bound is on live sessions rather than on the rate.
    let Ok(permit) = node.sockets.clone().try_acquire_owned() else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this node is serving all the sessions it will",
        );
    };
    let upgrade = match headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(',')
                .map(str::trim)
                .find(|p| p.starts_with(TOKEN_PROTOCOL))
        }) {
        Some(protocol) => upgrade.protocols([protocol.to_string()]),
        None => upgrade,
    };
    bounded(upgrade, MAX_MESSAGE_BYTES).on_upgrade(move |socket| async move {
        let _permit = permit;
        session::run(socket, node, device, peer).await;
    })
}

/// Both halves of the size bound. `max_message_size` alone leaves a single 16 MiB frame readable,
/// and a frame is buffered whole before the message it belongs to is assembled.
fn bounded(upgrade: WebSocketUpgrade, max: usize) -> WebSocketUpgrade {
    upgrade.max_message_size(max).max_frame_size(max)
}

/// A peer node dialling in. **No device token and no `Origin` check**, deliberately: mesh
/// authentication is a mutual ed25519 handshake carried inside the socket, and it is a different
/// credential space from anything a browser holds. A page that opens this socket gets as far as
/// being asked to sign a challenge with a key it does not have.
async fn mesh_socket(State(node): State<Arc<Node>>, Peer(peer): Peer, upgrade: WebSocketUpgrade) -> Response {
    if !node.config.mesh.accept {
        return refuse(StatusCode::NOT_FOUND, "this node does not accept mesh links");
    }
    // Both gates run *before* the upgrade, for the reason `/auth/pair` runs its limiter before
    // `claim_pairing`: past this point an anonymous caller has bought an argon2id pass at 19 MiB
    // and an attempt charged against every outstanding invite. The limiter bounds one address;
    // the semaphore bounds the memory when addresses rotate.
    if !node.auth.check_mesh(&peer) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "too many attempts");
    }
    let Ok(permit) = node.handshakes.clone().try_acquire_owned() else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this node is busy; try again shortly",
        );
    };
    bounded(upgrade, MAX_MESH_MESSAGE_BYTES)
        .on_upgrade(move |socket| crate::mesh::accept(socket, node, peer, permit))
}

/// The herd's own membership list: who may join, who is joined, and how far away they are.
async fn mesh_state(State(node): State<Arc<Node>>, auth: Authenticated) -> Response {
    if let Some(response) = auth.refused(&node, "api.mesh") {
        return response;
    }
    let mesh = node.auth.store().mesh();
    let (peers, hubs) = match (
        mesh.nodes(kampr_auth::MeshRole::Peer).await,
        mesh.nodes(kampr_auth::MeshRole::Hub).await,
    ) {
        (Ok(peers), Ok(hubs)) => (peers, hubs),
        (Err(e), _) | (_, Err(e)) => return store_failure("mesh", &e),
    };
    let live = node.peers.links();
    let described = |node_row: &kampr_auth::MeshNode| {
        let link = live.iter().find(|l| l.pubkey == node_row.pubkey);
        json!({
            "node_id": node_row.node_id,
            "name": node_row.name,
            "fingerprint": node_row.fingerprint(),
            "url": node_row.url,
            "created_at": node_row.created_at,
            "last_seen_at": node_row.last_seen_at,
            "revoked_at": node_row.revoked_at,
            "online": link.is_some(),
            "rtt_ms": link.and_then(|l| l.rtt_ms()),
            "build": link.map(|l| l.build.clone()),
        })
    };
    let identity = node.identity().ok();
    private_json(json!({
        "node_id": node.config.node_id,
        "fingerprint": identity.map(|i| i.fingerprint()),
        "accepts": node.config.mesh.accept,
        "peers": peers.iter().map(described).collect::<Vec<_>>(),
        "hubs": hubs.iter().map(described).collect::<Vec<_>>(),
    }))
}

/// A single-use join code. Separate from a device pairing code on purpose: one enrols a browser,
/// the other enrols a node, and neither is redeemable for the other.
async fn create_mesh_invite(State(node): State<Arc<Node>>, auth: Authenticated) -> Response {
    if let Some(response) = auth.refused(&node, "api.mesh.invite") {
        return response;
    }
    if !node.config.mesh.accept {
        return refuse(
            StatusCode::CONFLICT,
            "this node is not a hub: set `accept = true` under `[mesh]` in config.toml and restart it",
        );
    }
    let now = kampr_auth::now();
    let ttl = node.auth.policy().pairing_ttl.as_secs() as i64;
    let mesh = node.auth.store().mesh();
    let _ = mesh.expire_invites(now).await;
    let identity = match node.identity() {
        Ok(identity) => identity,
        Err(e) => return store_failure("mesh identity", &e),
    };
    match mesh.invite(now, now + ttl).await {
        Ok(code) => private_json(json!({
            "code": code,
            "expires_in": ttl,
            "fingerprint": identity.fingerprint(),
            "url": node.origin,
        })),
        Err(e) => store_failure("mesh invite", &e),
    }
}

/// Revocation has to bite on the connection that is already open, not at the next handshake.
async fn revoke_mesh_node(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.mesh.revoke") {
        return response;
    }
    match node.auth.store().mesh().revoke(&id, kampr_auth::now()).await {
        Ok(Some(revoked)) => {
            node.peers.disconnect(&revoked.pubkey);
            node.peers.disconnect(&revoked.node_id);
            node.auth.audit().record(
                &kampr_auth::Entry::new("mesh.revoked")
                    .device(&auth.device.id, &auth.device.name, auth.device.role.as_str())
                    .peer(&auth.peer)
                    .detail(json!({ "node": revoked.node_id, "fingerprint": revoked.fingerprint() })),
            );
            private_json(json!({ "revoked": revoked.node_id }))
        }
        Ok(None) => refuse(StatusCode::NOT_FOUND, "no such node in this herd"),
        Err(e) => store_failure("mesh revoke", &e),
    }
}

/// What a client needs before it has a token: the node's name, and what this origin can and
/// cannot do. Deliberately says nothing about who is enrolled.
async fn node_info(State(node): State<Arc<Node>>) -> Response {
    let tier = node.auth.tier();
    // Whether the node is claimed is the one thing on this page a first run branches on, and an
    // unreadable store cannot answer it. Saying "nobody is enrolled" would invite a stranger to
    // claim a node that is already somebody's.
    let enrolled = match node.auth.devices().await {
        Ok(devices) => devices.iter().any(|d| d.active(kampr_auth::now())),
        Err(e) => return store_failure("node_info", &e),
    };
    Json(json!({
        "node_id": node.config.node_id,
        "node_name": node.config.node_name,
        "build": BUILD,
        "protocol": kampr_core::wire::PROTOCOL,
        "bundle": assets::has_bundle(),
        "security": {
            "tier": tier.tier,
            "origin": tier.origin,
            "encrypted": tier.secure_context,
            "passkeys": tier.passkeys,
            "push": tier.push,
            "installable": tier.installable,
            "unlocks": tier.locked(),
        },
        "enrolled": enrolled,
    }))
    .into_response()
}

/// What this device may do about notifications, and what it has already asked for.
///
/// **`available` is the only thing a client should branch on.** It is false whenever the origin is
/// not a secure context, whenever the operator turned push off, and whenever no VAPID key could be
/// loaded — three different reasons a subscribe button must be absent rather than present and
/// failing at the last step.
async fn push_state(State(node): State<Arc<Node>>, auth: Authenticated) -> Response {
    let tier = node.auth.tier();
    let subscriptions = node
        .auth
        .store()
        .push_subscriptions_for(&auth.device.id)
        .await
        .unwrap_or_default();
    let rules = node
        .auth
        .store()
        .push_rules(&auth.device.id)
        .await
        .unwrap_or_default();
    private_json(json!({
        "available": node.push.available(),
        "key": node.push.public_key(),
        "secure_context": tier.secure_context,
        "unlocks": tier.locked(),
        "subscribed": !subscriptions.is_empty(),
        "endpoints": subscriptions.iter().map(|s| json!({
            "kind": s.kind, "endpoint": s.endpoint, "v": s.payload_version
        })).collect::<Vec<_>>(),
        "rules": rules,
    }))
}

/// `GET /api/attachment/{pane}/{id}` — the bytes an `att` header stands for.
///
/// **The websocket never carries them.** It is also carrying live terminal frames, and the
/// biggest attachment measured is 2.22 MB in one record (probe #247): pushing that down the same
/// socket head-of-lines every pane on the connection for as long as a phone link takes to drain
/// it. So the header goes on the wire, the operator decides, and the bytes come over HTTP once.
///
/// A pane id is `<node_id>/<local>` and the client sends that slash literally, so the path is
/// **three** segments and not two. Exactly three: anything else is refused rather than guessed at,
/// because a wrong guess about which part is the pane is a check anchored on the wrong pane.
///
/// **A read-only device may fetch a record's attachment.** Looking at a screenshot somebody pasted
/// into an agent session is reading, and it is the whole point of a device you half-trust with a
/// screen.
///
/// **A file id is a different question and carries a different gate.** It names any path on this
/// machine and nothing minted it, so it is answered only for a device that may send input — that
/// device can already `cat` the file through the terminal it is typing into, and a read-only one
/// cannot. The refusal is the ordinary read-only `403` with an audit line, decided from the id's
/// own shape before anything on disk is looked at, so it says nothing about the path.
async fn attachment(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(locator): Path<String>,
) -> Response {
    let missing = || refuse(StatusCode::NOT_FOUND, "no such attachment");
    let mut parts = locator.split('/');
    let (Some(node_id), Some(local), Some(id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return missing();
    };
    if node_id.is_empty() || local.is_empty() || id.is_empty() {
        return missing();
    }
    let pane = format!("{node_id}/{local}");
    let id = id.to_string();
    // Both path forms read a file this node can see, so both are gated as typing is, and both are
    // decided here rather than after the mesh hop — a hub must refuse them on its own device row.
    let path_form = match kampr_journal::Source::decode(&id) {
        Ok(kampr_journal::Source::File(file)) => Some((file, false)),
        Ok(kampr_journal::Source::Diff(file)) => Some((file, true)),
        _ => None,
    };
    // **Deliberately not a path form.** A `paste` id names a file inside this node's own pastes
    // directory or it names nothing at all, and `serve_paste` proves that before it reads a byte —
    // so it carries the *record* gate rather than the typing one, and the device that may only
    // look can look at the screenshot somebody pasted.
    let pasted = match kampr_journal::Source::decode(&id) {
        Ok(kampr_journal::Source::Paste(file)) => Some(file),
        _ => None,
    };
    if path_form.is_some()
        && let Some(response) = auth.refused(&node, "api.attachment.path")
    {
        return response;
    }
    // A pane this node does not own is served by the node that does, over the mesh link that node
    // dialled — it has no inbound path for this hub to fetch from (ADR 0007), so the bytes come
    // back the way the frames do.
    if node.resolve(&pane).is_none() {
        return attach::relay(&node.peers, &pane, &id).await;
    }
    // Both halves are file IO and the *first* is the expensive one: a miss walks every project
    // directory and reads both ends of up to 64 transcripts. Leaving it on the executor put tens
    // of megabytes of synchronous reads on a tokio worker for a request any device may make.
    let served = tokio::task::spawn_blocking(move || {
        if let Some(file) = pasted {
            let dir = node.state_dir.join("pastes");
            return Some(attach::serve_paste(&file, &dir));
        }
        if let Some((file, diff)) = path_form {
            let home = node.config.journal_home();
            return Some(match diff {
                true => attach::serve_diff(&file, &home),
                false => attach::serve_file(&file, &home),
            });
        }
        let transcript = transcript_of(&node, &pane)?;
        Some(attach::serve(&node.journals(), &transcript, &id))
    })
    .await;
    match served {
        Ok(Some(response)) => response,
        Ok(None) => missing(),
        Err(_) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the attachment could not be read",
        ),
    }
}

/// The transcript this pane is on *now*, derived by the node from the same three handles
/// `convo` uses. **The request has no say in it**, which is what makes it usable as the proof
/// that an id belongs to the pane it was asked for.
pub(crate) fn transcript_of(node: &Node, pane: &str) -> Option<std::path::PathBuf> {
    let (session, local) = node.resolve(pane)?;
    let herd = node.herd();
    let entry = herd.pane(pane)?;
    let journals = node.journals();
    let identity = crate::convo::identity(&journals, &session.provider, &local);
    journals
        .locate(
            entry.agent.as_deref(),
            identity.announced.as_ref(),
            entry.cwd.as_deref().map(std::path::Path::new),
            &identity.harness,
        )
        .ok()
        .flatten()
}

#[derive(Debug, Deserialize)]
struct WarmQuery {
    #[serde(default)]
    pane: Option<String>,
}

/// Warm resume, in one request.
///
/// The service worker fetches this while a push notification is still being read, so the tap that
/// follows opens onto data rather than onto a load (findings §3.11). It is the herd plus, when a
/// pane is named, that pane's outstanding question — a few kilobytes, and the same shapes the
/// socket sends, so a client seeds its store from it with no second code path.
///
/// It is **not** the grid. A full grid is ~4 KB but reproducing the wire's style interning outside
/// a live connection would be a second encoder, and the socket delivers the real one within a
/// second of the tap. What this removes is the empty herd and the unanswered question.
async fn warm(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    axum::extract::Query(query): axum::extract::Query<WarmQuery>,
) -> Response {
    let herd = node.herd();
    let mut body = json!({
        "t": "herd",
        "nodes": herd.nodes,
        "panes": herd.panes,
        "role": auth.device.role,
    });
    // **Only a pane the herd calls blocked.** A dialog is read off the screen (probe #42) and a
    // screen holds whatever the agent last printed, so a pane that is not waiting on anybody can
    // still carry something shaped like a question — a numbered list in build output, a cursor
    // beside two aligned lines. The socket path has always gated on `blocked`; this one did not,
    // and it is the same read.
    if let Some(pane) = query.pane.as_deref()
        && herd.pane(pane).map(|p| p.agent_status) == Some(kampr_core::provider::AgentStatus::Blocked)
        && let Some((session, local)) = node.resolve(pane)
        && let Some(found) = crate::pending::read(
            &session.herdr,
            &local,
            herd.pane(pane).and_then(|p| p.agent.as_deref()),
        )
        .await
    {
        body["pending"] = json!({
            "t": "pending",
            "pane": pane,
            "question": found.question,
            "options": found.options,
            "source": "screen",
        });
    }
    // `no-store` from the node; the service worker keeps its own copy deliberately and knows how
    // stale it is. A shared cache between the phone and the node must not.
    private_json(body)
}

#[derive(Debug, Deserialize)]
struct PushSubscribe {
    endpoint: String,
    /// `webpush` from a browser, `unifiedpush` from a distributor. A label for the device list,
    /// never a branch: UnifiedPush 3.0 is RFC 8291, so both are delivered to identically.
    #[serde(default)]
    kind: Option<String>,
    /// The highest payload version this client can read, so the node never sends it a
    /// notification kind it would render as a different one. Absent means a client written before
    /// there was more than one kind, which is assumed rather than guessed upwards
    /// (`kampr_auth::ASSUMED_PAYLOAD_VERSION`).
    #[serde(default)]
    v: Option<i64>,
    keys: PushKeys,
}

#[derive(Debug, Deserialize)]
struct PushKeys {
    p256dh: String,
    auth: String,
}

/// A read-only device may subscribe. Being told an agent is blocked is *reading*, and it is the
/// whole point of a device you half-trust with a screen.
async fn push_subscribe(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Json(body): Json<PushSubscribe>,
) -> Response {
    if !node.push.available() {
        return refuse(StatusCode::CONFLICT, "this node cannot send notifications");
    }
    if !body.endpoint.starts_with("https://") {
        return refuse(StatusCode::BAD_REQUEST, "a push endpoint must be https");
    }
    if body.endpoint.len() > 2048 || body.keys.p256dh.len() > 256 || body.keys.auth.len() > 64 {
        return refuse(StatusCode::BAD_REQUEST, "push subscription is too large");
    }
    let kind = match body.kind.as_deref() {
        Some("unifiedpush") => "unifiedpush",
        _ => "webpush",
    };
    // Clamped rather than trusted: this is a number a device declares about itself, and the
    // only thing it may do is *withhold* a kind from a client too old for it.
    let payload_version = body.v.unwrap_or(kampr_auth::ASSUMED_PAYLOAD_VERSION).clamp(
        kampr_auth::ASSUMED_PAYLOAD_VERSION,
        kampr_push::note::VERSION as i64,
    );
    match node
        .auth
        .store()
        .save_push_subscription(
            &auth.device.id,
            &kampr_auth::NewPushSubscription {
                kind,
                endpoint: &body.endpoint,
                p256dh: &body.keys.p256dh,
                auth: &body.keys.auth,
                payload_version,
            },
            kampr_auth::now(),
        )
        .await
    {
        Ok(_) => {
            node.auth.audit().record(
                &kampr_auth::Entry::new("push.subscribed")
                    .device(&auth.device.id, &auth.device.name, auth.device.role.as_str())
                    .peer(&auth.peer)
                    .detail(json!({ "kind": kind, "v": payload_version })),
            );
            private_json(json!({ "subscribed": true }))
        }
        Err(e) => store_failure("push_subscribe", &e),
    }
}

#[derive(Debug, Deserialize)]
struct PushUnsubscribe {
    endpoint: String,
}

async fn push_unsubscribe(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Json(body): Json<PushUnsubscribe>,
) -> Response {
    match node
        .auth
        .store()
        .delete_push_subscription(&auth.device.id, &body.endpoint)
        .await
    {
        Ok(removed) => {
            if removed {
                node.auth.audit().record(
                    &kampr_auth::Entry::new("push.unsubscribed")
                        .device(&auth.device.id, &auth.device.name, auth.device.role.as_str())
                        .peer(&auth.peer),
                );
            }
            private_json(json!({ "subscribed": false, "removed": removed }))
        }
        Err(e) => store_failure("push_unsubscribe", &e),
    }
}

/// Snooze and mute, per agent and per device. `pane: "*"` covers every agent on this device.
async fn push_rule(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Json(rule): Json<kampr_auth::PushRule>,
) -> Response {
    if rule.pane_id != kampr_auth::ALL_PANES && node.herd().pane(&rule.pane_id).is_none() {
        return refuse(StatusCode::NOT_FOUND, "no such pane on this node");
    }
    match node
        .auth
        .store()
        .set_push_rule(&auth.device.id, &rule, kampr_auth::now())
        .await
    {
        Ok(()) => match node.auth.store().push_rules(&auth.device.id).await {
            Ok(rules) => private_json(json!({ "rules": rules })),
            Err(e) => store_failure("push_rules", &e),
        },
        Err(e) => store_failure("push_rule", &e),
    }
}

#[derive(Debug, Deserialize)]
struct PairRequest {
    code: String,
    #[serde(default)]
    device_name: Option<String>,
}

async fn redeem_pairing(
    State(node): State<Arc<Node>>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(body): Json<PairRequest>,
) -> Response {
    if !same_origin(&node, &headers, &Method::POST, &Uri::from_static("/auth/pair")) {
        return refuse(StatusCode::FORBIDDEN, "cross-origin request refused");
    }
    let name = body
        .device_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "device".into());
    let agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    match node
        .auth
        .redeem_pairing(&body.code, name.trim(), agent, &peer)
        .await
    {
        Ok(e) => {
            // Probe #50 in the useful direction: the operator watching the console sees that a
            // device just paired, on the same screen the code was printed on. A pairing that
            // nobody expected is the one worth noticing, and this is the only channel that
            // reaches somebody who is not holding the phone.
            let name = e.device.name.clone();
            let role = e.device.role.as_str().to_string();
            let node = node.clone();
            tokio::spawn(async move {
                let session = node.primary();
                node.toaster
                    .show(
                        &session.herdr,
                        "pairing",
                        &format!("{name} paired"),
                        Some(&format!("{role} access · revoke it with kampr setup")),
                    )
                    .await;
            });
            private_json(json!({ "token": e.token, "device": e.device }))
        }
        Err(AuthError::RateLimited) => refuse(StatusCode::TOO_MANY_REQUESTS, "too many attempts"),
        // The same answer `/mesh` gives when its handshake semaphore is full, for the same
        // reason: the argon2id pass behind this route is bounded, and a caller told to come back
        // is not a caller told the code was wrong.
        Err(AuthError::Busy) => refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "this node is busy; try again shortly",
        ),
        Err(_) => refuse(StatusCode::UNAUTHORIZED, "that pairing code is not valid"),
    }
}

#[derive(Debug, Deserialize)]
struct RoleRequest {
    role: Role,
}

async fn create_pairing(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    body: Option<Json<RoleRequest>>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.pairing.create") {
        return response;
    }
    let role = body.map_or(Role::Full, |b| b.0.role);
    match node.auth.create_pairing(role, Delivery::Authenticated).await {
        Ok(pairing) => private_json(json!({
            "code": pairing.code,
            "role": role,
            "expires_in": node.auth.policy().pairing_ttl.as_secs(),
        })),
        Err(e) => store_failure("create_pairing", &e),
    }
}

/// The inventory names every enrolled device, when it was last seen and what it may do. A
/// read-only device is one you half-trust; that is not the list to hand it.
async fn devices(State(node): State<Arc<Node>>, auth: Authenticated) -> Response {
    if let Some(response) = auth.refused(&node, "api.devices") {
        return response;
    }
    match node.auth.devices().await {
        Ok(devices) => {
            let now = kampr_auth::now();
            // Additive, and the node's judgement rather than the client's: currently paired is
            // `revoked_at` *and* `expires_at`, and a client reading only the first showed a
            // device whose token ran out weeks ago as still connected. An older client ignores
            // the field and keeps its old reading.
            let devices: Vec<Value> = devices
                .iter()
                .map(|device| {
                    let mut row = json!(device);
                    row["active"] = json!(device.active(now));
                    row
                })
                .collect();
            private_json(json!({ "devices": devices }))
        }
        Err(e) => store_failure("devices", &e),
    }
}

async fn revoke_device(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.device.revoke") {
        return response;
    }
    match node.auth.revoke(&id, &auth.device).await {
        Ok(true) => private_json(json!({ "revoked": id })),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => store_failure("revoke", &e),
    }
}

async fn set_role(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
    Json(body): Json<RoleRequest>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.device.role") {
        return response;
    }
    match node.auth.set_role(&id, body.role, &auth.device).await {
        Ok(true) => private_json(json!({ "device": id, "role": body.role })),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => store_failure("set_role", &e),
    }
}

async fn renew_device(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.device.renew") {
        return response;
    }
    match node.auth.renew(&id, &auth.device).await {
        Ok(true) => private_json(json!({ "renewed": id })),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => store_failure("renew", &e),
    }
}

#[derive(Debug, Deserialize)]
struct RegisterStart {
    #[serde(default)]
    device_name: Option<String>,
    /// Which authenticator API is going to run this ceremony. Only ever chooses between two
    /// option sets the node states; it grants nothing and is verified no differently.
    #[serde(default)]
    platform: Option<String>,
}

async fn register_start(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Json(body): Json<RegisterStart>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.passkey.register.start") {
        return response;
    }
    let name = body.device_name.unwrap_or_else(|| auth.device.name.clone());
    let client = kampr_auth::Client::from_platform(body.platform.as_deref());
    match node.auth.start_passkey_registration(&name, client).await {
        Ok((challenge_id, options)) => {
            Json(json!({ "challenge_id": challenge_id, "options": options })).into_response()
        }
        Err(e) => passkey_rejection(e),
    }
}

/// `device_name` and `role` were fields here and are gone: the passkey binds to the device that
/// asked for it, which already has both. An older client still sends them and is still served —
/// unknown fields are ignored, which is the rule the wire is additive under.
#[derive(Debug, Deserialize)]
struct RegisterFinish {
    challenge_id: String,
    credential: Value,
}

async fn register_finish(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Json(body): Json<RegisterFinish>,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.passkey.register.finish") {
        return response;
    }
    let Ok(credential) = serde_json::from_value(body.credential) else {
        return refuse(StatusCode::BAD_REQUEST, "unreadable credential");
    };
    match node
        .auth
        .finish_passkey_registration(&body.challenge_id, &credential, &auth.device)
        .await
    {
        Ok(e) => private_json(json!({ "token": e.token, "device": e.device })),
        Err(e) => passkey_rejection(e),
    }
}

async fn authenticate_start(State(node): State<Arc<Node>>, Peer(peer): Peer) -> Response {
    match node.auth.start_passkey_authentication(&peer).await {
        Ok((challenge_id, options)) => {
            Json(json!({ "challenge_id": challenge_id, "options": options })).into_response()
        }
        Err(e) => passkey_rejection(e),
    }
}

#[derive(Debug, Deserialize)]
struct AuthenticateFinish {
    challenge_id: String,
    credential: Value,
}

async fn authenticate_finish(
    State(node): State<Arc<Node>>,
    Peer(peer): Peer,
    headers: HeaderMap,
    Json(body): Json<AuthenticateFinish>,
) -> Response {
    if !same_origin(
        &node,
        &headers,
        &Method::POST,
        &Uri::from_static("/auth/webauthn/authenticate/finish"),
    ) {
        return refuse(StatusCode::FORBIDDEN, "cross-origin request refused");
    }
    let Ok(credential) = serde_json::from_value(body.credential) else {
        return refuse(StatusCode::BAD_REQUEST, "unreadable credential");
    };
    match node
        .auth
        .finish_passkey_authentication(&body.challenge_id, &credential, &peer)
        .await
    {
        Ok(e) => private_json(json!({ "token": e.token, "device": e.device })),
        Err(e) => passkey_rejection(e),
    }
}

/// A node that cannot do passkeys says so plainly rather than failing somewhere inside the
/// ceremony — the client is expected to have hidden the control already, and this is the backstop.
fn passkey_rejection(error: AuthError) -> Response {
    match error {
        AuthError::Passkey(kampr_auth::PasskeyError::Unavailable) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "this origin cannot do passkeys",
                "reason": "a WebAuthn RP ID must be a registrable domain; an IP address is not one",
            })),
        )
            .into_response(),
        AuthError::RateLimited => refuse(StatusCode::TOO_MANY_REQUESTS, "too many attempts"),
        AuthError::UnknownCredential => refuse(StatusCode::NOT_FOUND, "no passkey is enrolled"),
        other => refuse(StatusCode::UNAUTHORIZED, &other.to_string()),
    }
}

fn asset_links_response(document: Option<String>) -> Response {
    match document {
        Some(body) => (
            [
                (CONTENT_TYPE, "application/json"),
                (CACHE_CONTROL, "public, max-age=300"),
            ],
            body,
        )
            .into_response(),
        None => refuse(StatusCode::NOT_FOUND, "this node delegates to no Android app"),
    }
}

async fn static_asset(uri: Uri, headers: HeaderMap) -> Response {
    let held = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok());
    assets::serve(uri.path(), held)
}
