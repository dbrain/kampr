use crate::wire::{SessionInfo, TranscriptPage};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
pub struct Client {
    base: Url,
    token: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    Taken,
    NoLongerWaiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Allow,
    Always,
    Deny,
}

impl Answer {
    pub fn word(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Always => "always",
            Self::Deny => "deny",
        }
    }
}

impl Client {
    pub fn new(url: &str, token: &str) -> Result<Self> {
        let base = Url::parse(url).with_context(|| format!("muffit url {url:?}"))?;
        // muffit refuses any request carrying `Origin` (kampr.md §1); reqwest sends none unless told.
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("an http client for muffit")?;
        Ok(Self {
            base,
            token: token.to_string(),
            http,
        })
    }

    fn url(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        Ok(request.bearer_auth(&self.token).send().await?)
    }

    async fn ok_json(&self, request: reqwest::RequestBuilder, what: &str) -> Result<Value> {
        let response = self.send(request).await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("{what}: muffit answered {status}: {}", error_of(&body));
        }
        Ok(body)
    }

    pub async fn sessions(&self) -> Result<Vec<SessionInfo>> {
        let body = self
            .ok_json(self.http.get(self.url("/sessions")), "listing sessions")
            .await?;
        Ok(serde_json::from_value(body["sessions"].clone())?)
    }

    pub async fn stored(&self) -> Result<Vec<crate::wire::StoredSession>> {
        let mut url = self.url("/sessions");
        url.query_pairs_mut().append_pair("all", "true");
        let body = self
            .ok_json(self.http.get(url), "listing stored sessions")
            .await?;
        Ok(serde_json::from_value(body["sessions"].clone())?)
    }

    pub async fn transcript(
        &self,
        id: &str,
        before_seq: Option<u64>,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<TranscriptPage> {
        let mut url = self.url(&format!("/sessions/{}/transcript", segment(id)));
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &limit.to_string());
            if let Some(before) = before_seq {
                query.append_pair("before_seq", &before.to_string());
            }
            if let Some(after) = after_seq {
                query.append_pair("after_seq", &after.to_string());
            }
        }
        let body = self.ok_json(self.http.get(url), "reading a transcript").await?;
        Ok(serde_json::from_value(body)?)
    }

    pub async fn create(&self, cwd: &str, prompt: Option<&str>) -> Result<String> {
        let mut body = json!({ "cwd": cwd });
        if let Some(prompt) = prompt.filter(|p| !p.is_empty()) {
            body["prompt"] = json!(prompt);
        }
        let reply = self
            .ok_json(
                self.http.post(self.url("/sessions")).json(&body),
                "creating a session",
            )
            .await?;
        reply["id"]
            .as_str()
            .map(str::to_string)
            .context("muffit created a session and named no id")
    }

    pub async fn input(&self, id: &str, text: &str) -> Result<()> {
        self.control(id, "input", Some(json!({ "text": text }))).await
    }

    pub async fn interrupt(&self, id: &str) -> Result<()> {
        self.control(id, "interrupt", None).await
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        self.control(id, "stop", None).await
    }

    async fn control(&self, id: &str, verb: &str, body: Option<Value>) -> Result<()> {
        let mut request = self
            .http
            .post(self.url(&format!("/sessions/{}/{verb}", segment(id))));
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.ok_json(request, verb).await.map(|_| ())
    }

    pub async fn answer(&self, id: &str, prompt: u64, answer: Answer) -> Result<Answered> {
        let request = self
            .http
            .post(self.url(&format!("/sessions/{}/permission", segment(id))))
            .json(&json!({ "id": prompt, "answer": answer.word() }));
        let response = self.send(request).await?;
        let status = response.status();
        if status == reqwest::StatusCode::CONFLICT {
            return Ok(Answered::NoLongerWaiting);
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!(
                "answering a prompt: muffit answered {status}: {}",
                error_of(&body)
            );
        }
        Ok(Answered::Taken)
    }

    /// A websocket to `path`, said hello to. Built from a bare request so it carries no `Origin`.
    pub async fn socket(&self, path: &str, capabilities: &[&str]) -> Result<Socket> {
        let mut url = self.url(path);
        let scheme = match url.scheme() {
            "https" => "wss",
            _ => "ws",
        };
        url.set_scheme(scheme)
            .map_err(|()| anyhow::anyhow!("muffit url has no websocket form"))?;
        let mut request = url.as_str().into_client_request()?;
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {}", self.token).parse()?);
        let (mut socket, _) =
            tokio::time::timeout(REQUEST_TIMEOUT, tokio_tungstenite::connect_async(request))
                .await
                .context("muffit did not answer the websocket in time")??;
        socket
            .send(Message::Text(crate::wire::hello(capabilities).to_string().into()))
            .await?;
        Ok(socket)
    }
}

pub async fn send(socket: &mut Socket, frame: &Value) -> Result<()> {
    socket.send(Message::Text(frame.to_string().into())).await?;
    Ok(())
}

/// The next frame, `None` once the socket is closed. A frame nothing here can read is skipped.
pub async fn next(socket: &mut Socket) -> Option<crate::wire::ServerFrame> {
    loop {
        match socket.next().await? {
            Ok(Message::Text(text)) => match serde_json::from_str(&text) {
                Ok(frame) => return Some(frame),
                Err(e) => tracing::debug!(error = %e, "skipping a muffit frame"),
            },
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

fn error_of(body: &Value) -> String {
    body["error"]
        .as_str()
        .map_or_else(|| body.to_string(), str::to_string)
}

fn segment(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect()
}
