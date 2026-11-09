use crate::api::{Event, Inspect, Summary};
use anyhow::{Context, Result, bail};
use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixStream;

/// `stats?stream=false` waits for a second sample to fill `precpu_stats`, about a second.
const CALL: Duration = Duration::from_secs(15);
const STOP_GRACE_SECS: u32 = 10;
/// A static binary is tens of megabytes, unpacked by the daemon into an overlay.
const INJECT: Duration = Duration::from_secs(120);
const PULL: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
#[error("docker answered {status}: {message}")]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

pub fn is_not_found(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>().is_some_and(|e| e.status == 404)
}

#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub async fn ping(&self) -> bool {
        self.call(Method::GET, "/_ping").await.is_ok()
    }

    pub async fn ids(&self) -> Result<Vec<String>> {
        let listed: Vec<Summary> = self.json("/containers/json?all=1").await?;
        Ok(listed.into_iter().map(|s| s.id).collect())
    }

    pub async fn inspect(&self, id: &str) -> Result<Inspect> {
        self.json(&format!("/containers/{}/json", escape(id))).await
    }

    pub async fn logs(&self, id: &str, tail: usize) -> Result<Vec<u8>> {
        let path = format!(
            "/containers/{}/logs?tail={tail}&timestamps=1&stdout=1&stderr=1",
            escape(id)
        );
        Ok(self.call(Method::GET, &path).await?.to_vec())
    }

    pub async fn stats(&self, id: &str) -> Result<serde_json::Value> {
        self.json(&format!("/containers/{}/stats?stream=false", escape(id)))
            .await
    }

    pub async fn start(&self, id: &str) -> Result<()> {
        self.post(&format!("/containers/{}/start", escape(id))).await
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        self.post(&format!("/containers/{}/stop?t={STOP_GRACE_SECS}", escape(id)))
            .await
    }

    pub async fn restart(&self, id: &str) -> Result<()> {
        self.post(&format!("/containers/{}/restart?t={STOP_GRACE_SECS}", escape(id)))
            .await
    }

    pub async fn kill(&self, id: &str) -> Result<()> {
        self.post(&format!("/containers/{}/kill", escape(id))).await
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        self.call(Method::DELETE, &format!("/containers/{}?force=true", escape(id)))
            .await
            .map(|_| ())
    }

    /// The image's `Architecture`, in Docker's spelling (`amd64`, `arm64`).
    pub async fn image_architecture(&self, image: &str) -> Result<String> {
        let id = image.trim_start_matches("sha256:");
        let image: serde_json::Value = self.json(&format!("/images/{}/json", escape(id))).await?;
        Ok(image["Architecture"].as_str().unwrap_or_default().to_string())
    }

    /// Unpacks a tar at `dir` in the container, owned by the container's own user.
    pub async fn put_archive(&self, id: &str, dir: &str, tar: Vec<u8>) -> Result<()> {
        let path = format!(
            "/containers/{}/archive?path={}&copyUIDGID=1",
            escape(id),
            escape_path(dir)
        );
        self.call_with(Method::PUT, &path, Bytes::from(tar), INJECT)
            .await
            .map(|_| ())
    }

    /// The file at `path` in the container, wrapped in a tar; `None` when there is none.
    pub async fn get_archive(&self, id: &str, path: &str) -> Result<Option<Bytes>> {
        let path = format!("/containers/{}/archive?path={}", escape(id), escape_path(path));
        match self.call(Method::GET, &path).await {
            Ok(body) => Ok(Some(body)),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// What is at `path`, the link itself when it is a symlink; `None` when nothing is.
    pub async fn stat_path(&self, id: &str, path: &str) -> Result<Option<PathStat>> {
        let uri = format!("/containers/{}/archive?path={}", escape(id), escape_path(path));
        let response = tokio::time::timeout(CALL, self.send(Method::HEAD, &uri, Bytes::new()))
            .await
            .with_context(|| format!("docker did not answer {uri}"))??;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!(refused(status, b""));
        }
        let header = response
            .headers()
            .get("X-Docker-Container-Path-Stat")
            .context("docker sent no path stat")?;
        let json = base64::engine::general_purpose::STANDARD.decode(header.as_bytes())?;
        Ok(Some(serde_json::from_slice(&json)?))
    }

    pub async fn has_image(&self, reference: &str) -> Result<bool> {
        match self
            .call(Method::GET, &format!("/images/{}/json", encode(reference)))
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `POST /images/create` read to its end; a pull that fails part way still answers 200 and
    /// says so in the progress lines, which are the caller's to read.
    pub async fn create_image(&self, from_image: &str, tag: &str) -> Result<Bytes> {
        let mut path = format!("/images/create?fromImage={}", encode(from_image));
        if !tag.is_empty() {
            path += &format!("&tag={}", encode(tag));
        }
        self.call_with(Method::POST, &path, Bytes::new(), PULL).await
    }

    /// From `since_nano` inclusive, then live; the journal drops what it already holds.
    pub async fn events(&self, since_nano: Option<i64>) -> Result<Events> {
        let mut path = "/events?filters=%7B%22type%22%3A%5B%22container%22%5D%7D".to_string();
        if let Some(nano) = since_nano {
            path += &format!("&since={}.{:09}", nano / 1_000_000_000, nano % 1_000_000_000);
        }
        let response = tokio::time::timeout(CALL, self.send(Method::GET, &path, Bytes::new())).await??;
        Ok(Events {
            body: response.into_body(),
            pending: Vec::new(),
        })
    }

    async fn post(&self, path: &str) -> Result<()> {
        self.call(Method::POST, path).await.map(|_| ())
    }

    async fn json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let body = self.call(Method::GET, path).await?;
        serde_json::from_slice(&body).with_context(|| format!("reading docker's answer to {path}"))
    }

    async fn call(&self, method: Method, path: &str) -> Result<Bytes> {
        self.call_with(method, path, Bytes::new(), CALL).await
    }

    async fn call_with(&self, method: Method, path: &str, body: Bytes, within: Duration) -> Result<Bytes> {
        let read = async {
            let response = self.send(method, path, body).await?;
            let status = response.status();
            let body = response.into_body().collect().await?.to_bytes();
            if status.is_client_error() || status.is_server_error() {
                bail!(refused(status, &body));
            }
            Ok(body)
        };
        tokio::time::timeout(within, read)
            .await
            .with_context(|| format!("docker did not answer {path}"))?
    }

    async fn send(&self, method: Method, path: &str, body: Bytes) -> Result<Response<Incoming>> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .with_context(|| format!("connecting to {}", self.socket.display()))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(hyper::header::HOST, "docker");
        if !body.is_empty() {
            request = request.header(hyper::header::CONTENT_TYPE, "application/x-tar");
        }
        let request = request.body(Full::new(body))?;
        Ok(sender.send_request(request).await?)
    }
}

fn refused(status: StatusCode, body: &[u8]) -> ApiError {
    let message = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
    ApiError {
        status: status.as_u16(),
        message,
    }
}

/// An id or a name; anything else would be a path of its own.
fn escape(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_.-".contains(*c))
        .collect()
}

fn escape_path(path: &str) -> String {
    path.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_.-/".contains(*c))
        .collect()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PathStat {
    pub mode: u32,
    /// Absolute: the daemon resolves a relative target (`sh -> dash` reads `/usr/bin/dash`).
    pub link_target: String,
}

impl PathStat {
    pub fn is_executable_file(&self) -> bool {
        const GO_DIR: u32 = 1 << 31;
        self.mode & GO_DIR == 0 && self.mode & 0o111 != 0
    }
}

pub struct Events {
    body: Incoming,
    pending: Vec<u8>,
}

impl Events {
    /// `None` when the stream ends; a line that is not an event is skipped.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                match serde_json::from_slice(&line) {
                    Ok(event) => return Some(event),
                    Err(_) => continue,
                }
            }
            let frame = self.body.frame().await?.ok()?;
            if let Ok(data) = frame.into_data() {
                self.pending.extend_from_slice(&data);
            }
        }
    }
}
