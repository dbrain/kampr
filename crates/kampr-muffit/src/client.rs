use crate::reach::Reach;
use crate::wire::{Image, SessionInfo, TranscriptPage};
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
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

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

    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Whether the daemon takes this node's token, asked of its session list.
    pub async fn reach(&self) -> Reach {
        let request = self.http.get(self.url("/sessions")).timeout(PROBE_TIMEOUT);
        match self.send(request).await {
            Err(e) => Reach::Away(format!("{e:#}")),
            Ok(response) => {
                let status = response.status().as_u16();
                let body: Value = response.json().await.unwrap_or(Value::Null);
                Reach::from_status(status, &body)
            }
        }
    }

    pub async fn sessions(&self) -> Result<Vec<SessionInfo>> {
        let body = self
            .ok_json(self.http.get(self.url("/sessions")), "listing sessions")
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

    pub async fn input(&self, id: &str, text: &str, images: &[Image], msg_id: Option<&str>) -> Result<()> {
        let mut body = json!({ "text": text });
        if !images.is_empty() {
            body["images"] = json!(images);
        }
        if let Some(msg_id) = msg_id {
            body["msg_id"] = json!(msg_id);
        }
        self.control(id, "input", Some(body)).await
    }

    pub async fn interrupt(&self, id: &str) -> Result<()> {
        self.control(id, "interrupt", None).await
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        self.control(id, "stop", None).await
    }

    async fn control(&self, id: &str, verb: &str, body: Option<Value>) -> Result<()> {
        self.post_session(id, verb, body).await.map(|_| ())
    }

    /// A POST to one of the session's REST routes, with the daemon's answer body.
    pub async fn post_session(&self, id: &str, route: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .http
            .post(self.url(&format!("/sessions/{}/{route}", segment(id))));
        if let Some(body) = &body {
            request = request.json(body);
        }
        self.ok_json(request, route).await
    }

    pub async fn answer(&self, id: &str, prompt: u64, answer: Answer) -> Result<Answered> {
        self.post_answer(id, "permission", json!({ "id": prompt, "answer": answer.word() }))
            .await
    }

    /// The model's question (wire 3.4): `answer` is an option's index, or words.
    pub async fn answer_question(&self, id: &str, question: u64, answer: Value) -> Result<Answered> {
        self.post_answer(id, "question", json!({ "id": question, "answer": answer }))
            .await
    }

    async fn post_answer(&self, id: &str, what: &str, body: Value) -> Result<Answered> {
        let request = self
            .http
            .post(self.url(&format!("/sessions/{}/{what}", segment(id))))
            .json(&body);
        let response = self.send(request).await?;
        let status = response.status();
        if status == reqwest::StatusCode::CONFLICT {
            return Ok(Answered::NoLongerWaiting);
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!(
                "answering a {what}: muffit answered {status}: {}",
                error_of(&body)
            );
        }
        Ok(Answered::Taken)
    }

    /// A websocket to `path`, said hello to. Built from a bare request so it carries no `Origin`.
    pub async fn socket(&self, path: &str, query: Option<&str>, capabilities: &[&str]) -> Result<Socket> {
        let mut url = self.url(path);
        url.set_query(query);
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
                Err(e) => unreadable(&text, &e),
            },
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// A status frame skipped freezes the pane's status, so it is said aloud, once each time the
/// failure changes rather than on every push.
fn unreadable(text: &str, error: &serde_json::Error) {
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    if serde_json::from_str::<Value>(text).map_or(true, |frame| frame["type"] != "status") {
        tracing::debug!(error = %error, "skipping a muffit frame");
        return;
    }
    let said = error.to_string();
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if last.as_deref() != Some(said.as_str()) {
        tracing::warn!(error = %said, "muffit sent a status frame this node cannot read; the pane's status stands still");
        *last = Some(said);
    }
}

pub(crate) fn error_of(body: &Value) -> String {
    body["error"]
        .as_str()
        .map_or_else(|| body.to_string(), str::to_string)
}

fn segment(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect()
}
