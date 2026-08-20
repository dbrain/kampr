use crate::assets;
use crate::session;
use crate::state::{BUILD, Node};
use anyhow::{Context, Result};
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, COOKIE, HOST, HeaderMap, HeaderName, ORIGIN,
    REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kampr_auth::{AuthError, Device, Role};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
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
const CSP: &str = "default-src 'self'; \
     script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'sha256-+bHRyQ0Z1/Lb6dgSILtTESBRCIFl8jkBb/dPQA4Pdnw='; \
     img-src 'self' data: blob:; \
     font-src 'self' data:; \
     connect-src 'self' ws: wss:; \
     worker-src 'self' blob:; \
     frame-ancestors 'none'; base-uri 'none'; object-src 'none'; form-action 'none'";

const TOKEN_PROTOCOL: &str = "kampr.token.";
const SESSION_COOKIE: &str = "kampr_session";

pub fn router(node: Arc<Node>) -> Router {
    let secured = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
    };
    Router::new()
        .route("/ws", get(websocket))
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/node", get(node_info))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}/revoke", post(revoke_device))
        .route("/api/devices/{id}/role", post(set_role))
        .route("/api/devices/{id}/renew", post(renew_device))
        .route("/api/pair", post(create_pairing))
        .route("/auth/pair", post(redeem_pairing))
        .route("/auth/webauthn/register/start", post(register_start))
        .route("/auth/webauthn/register/finish", post(register_finish))
        .route("/auth/webauthn/authenticate/start", post(authenticate_start))
        .route("/auth/webauthn/authenticate/finish", post(authenticate_finish))
        .fallback(get(static_asset))
        .layer(secured(CONTENT_SECURITY_POLICY, CSP))
        .layer(secured(X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(secured(X_FRAME_OPTIONS, "DENY"))
        .layer(secured(REFERRER_POLICY, "no-referrer"))
        .with_state(node)
}

pub async fn serve(node: Arc<Node>) -> Result<()> {
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
    fn refused(&self) -> Option<Response> {
        (!self.device.role.writes()).then(|| refuse(StatusCode::FORBIDDEN, "this device is read-only"))
    }
}

fn auth_rejection(error: AuthError) -> Response {
    match error {
        AuthError::RateLimited => refuse(StatusCode::TOO_MANY_REQUESTS, "too many attempts"),
        _ => refuse(StatusCode::UNAUTHORIZED, "this device is not authorised"),
    }
}

fn refuse(status: StatusCode, message: &str) -> Response {
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
        && let Some(first) = forwarded.split(',').next()
    {
        return first.trim().to_string();
    }
    connect.map_or_else(|| "unknown".to_string(), |c| c.0.ip().to_string())
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok())
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        return Some(token.trim().to_string());
    }
    if let Some(token) = subprotocol_token(headers) {
        return Some(token);
    }
    headers
        .get(COOKIE)
        .and_then(|v| v.to_str().ok())?
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_string())
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
/// it to match is what stops a page on another origin from driving this node. Its absence means a
/// non-browser client, which cannot be tricked into making the request in the first place.
fn same_origin(node: &Node, headers: &HeaderMap, method: &Method, uri: &Uri) -> bool {
    let is_upgrade = uri.path() == "/ws";
    if !is_upgrade && matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    allowed_origins(node, headers).iter().any(|a| a == origin)
}

fn allowed_origins(node: &Node, headers: &HeaderMap) -> Vec<String> {
    let mut allowed = vec![node.origin.clone()];
    allowed.extend(node.config.server.extra_origins.iter().cloned());
    // A Tier 0 node is reached at whatever address the phone typed, and there may be several — a
    // LAN IP, a `.local` name, loopback. Matching the request's own Host keeps that working
    // without asking the operator to enumerate them.
    if let Some(host) = headers.get(HOST).and_then(|v| v.to_str().ok()) {
        for scheme in ["http", "https"] {
            allowed.push(format!("{scheme}://{host}"));
        }
    }
    allowed
}

async fn websocket(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Authenticated { device, peer } = auth;
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
    upgrade.on_upgrade(move |socket| session::run(socket, node, device, peer))
}

/// What a client needs before it has a token: the node's name, and what this origin can and
/// cannot do. Deliberately says nothing about who is enrolled.
async fn node_info(State(node): State<Arc<Node>>) -> Json<Value> {
    let tier = node.auth.tier();
    let enrolled = node
        .auth
        .devices()
        .await
        .map(|d| d.iter().filter(|d| d.active(kampr_auth::now())).count());
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
            "unlocks": tier.unlocks,
        },
        "enrolled": enrolled.unwrap_or(0) > 0,
    }))
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
        Ok(e) => Json(json!({ "token": e.token, "device": e.device })).into_response(),
        Err(AuthError::RateLimited) => refuse(StatusCode::TOO_MANY_REQUESTS, "too many attempts"),
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
    if let Some(response) = auth.refused() {
        return response;
    }
    let role = body.map_or(Role::Full, |b| b.0.role);
    match node.auth.create_pairing(role).await {
        Ok(code) => Json(json!({
            "code": code,
            "role": role,
            "expires_in": node.auth.policy().pairing_ttl.as_secs(),
        }))
        .into_response(),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn devices(State(node): State<Arc<Node>>, _auth: Authenticated) -> Response {
    match node.auth.devices().await {
        Ok(devices) => Json(json!({ "devices": devices })).into_response(),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn revoke_device(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = auth.refused() {
        return response;
    }
    match node.auth.revoke(&id, &auth.device).await {
        Ok(true) => Json(json!({ "revoked": id })).into_response(),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn set_role(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
    Json(body): Json<RoleRequest>,
) -> Response {
    if let Some(response) = auth.refused() {
        return response;
    }
    match node.auth.set_role(&id, body.role, &auth.device).await {
        Ok(true) => Json(json!({ "device": id, "role": body.role })).into_response(),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn renew_device(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = auth.refused() {
        return response;
    }
    match node.auth.renew(&id, &auth.device).await {
        Ok(true) => Json(json!({ "renewed": id })).into_response(),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct RegisterStart {
    #[serde(default)]
    device_name: Option<String>,
}

async fn register_start(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Json(body): Json<RegisterStart>,
) -> Response {
    if let Some(response) = auth.refused() {
        return response;
    }
    let name = body.device_name.unwrap_or_else(|| auth.device.name.clone());
    match node.auth.start_passkey_registration(&name).await {
        Ok((challenge_id, options)) => {
            Json(json!({ "challenge_id": challenge_id, "options": options })).into_response()
        }
        Err(e) => passkey_rejection(e),
    }
}

#[derive(Debug, Deserialize)]
struct RegisterFinish {
    challenge_id: String,
    credential: Value,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    role: Option<Role>,
}

async fn register_finish(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    headers: HeaderMap,
    Json(body): Json<RegisterFinish>,
) -> Response {
    if let Some(response) = auth.refused() {
        return response;
    }
    let Ok(credential) = serde_json::from_value(body.credential) else {
        return refuse(StatusCode::BAD_REQUEST, "unreadable credential");
    };
    let name = body.device_name.unwrap_or_else(|| auth.device.name.clone());
    let agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    match node
        .auth
        .finish_passkey_registration(
            &body.challenge_id,
            &credential,
            &name,
            agent,
            body.role.unwrap_or(Role::Full),
        )
        .await
    {
        Ok(e) => Json(json!({ "token": e.token, "device": e.device })).into_response(),
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
        Ok(e) => Json(json!({ "token": e.token, "device": e.device })).into_response(),
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

async fn static_asset(uri: Uri) -> Response {
    assets::serve(uri.path())
}
