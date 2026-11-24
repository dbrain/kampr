//! A named key against a herdr without `pane.send_keys`: the bytes a terminal would have sent go
//! out as text instead, so an installed herdr that predates the method keeps taking the chord.

use kampr_core::provider::{Input, Provider};
use kampr_core::{HerdrConfig, HerdrProvider};
use kampr_herdr::Herdr;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;

const PANE: &str = "w1:p1";

struct FakeHerdr {
    texts: Arc<Mutex<Vec<String>>>,
    keys_calls: Arc<Mutex<usize>>,
    supports_keys: bool,
    _dir: tempfile::TempDir,
    socket: PathBuf,
}

impl FakeHerdr {
    fn start(supports_keys: bool) -> Arc<Self> {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let texts = Arc::new(Mutex::new(Vec::new()));
        let keys_calls = Arc::new(Mutex::new(0));
        let fake = Arc::new(Self {
            texts: texts.clone(),
            keys_calls: keys_calls.clone(),
            supports_keys,
            _dir: dir,
            socket: socket.clone(),
        });
        tokio::spawn({
            let texts = texts.clone();
            let keys_calls = keys_calls.clone();
            let supports_keys = fake.supports_keys;
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let texts = texts.clone();
                    let keys_calls = keys_calls.clone();
                    tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt as _;
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let request: Value = serde_json::from_str(&line).unwrap_or_default();
                        let method = request["method"].as_str().unwrap_or_default().to_string();
                        let params = &request["params"];
                        let reply = match method.as_str() {
                            "pane.send_keys" => {
                                *keys_calls.lock().unwrap() += 1;
                                if supports_keys {
                                    json!({ "result": { "type": "ok" } })
                                } else {
                                    json!({ "error": {
                                        "code": "unknown_method",
                                        "message": "no such method: pane.send_keys" } })
                                }
                            }
                            "pane.send_text" => {
                                texts
                                    .lock()
                                    .unwrap()
                                    .push(params["text"].as_str().unwrap_or_default().to_string());
                                json!({ "result": { "type": "ok" } })
                            }
                            other => json!({ "result": { "type": "ok", "method": other } }),
                        };
                        let mut reply = reply;
                        reply["id"] = json!("kampr");
                        let mut stream = reader.into_inner();
                        let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
                    });
                }
            }
        });
        fake
    }

    fn texts(&self) -> Vec<String> {
        self.texts.lock().unwrap().clone()
    }

    fn keys_calls(&self) -> usize {
        *self.keys_calls.lock().unwrap()
    }
}

fn provider(fake: &FakeHerdr) -> HerdrProvider {
    HerdrProvider::spawn(
        Herdr::new(&fake.socket),
        HerdrConfig {
            // No tasks dialing the fake: the write is the whole of what is under test.
            enabled: false,
            ..HerdrConfig::default()
        },
    )
}

#[tokio::test]
async fn a_named_key_against_a_herdr_without_send_keys_goes_out_as_bytes() {
    let fake = FakeHerdr::start(false);
    let provider = provider(&fake);
    provider
        .write_pane(PANE, Input::Keys(vec!["shift+tab".into()]))
        .await
        .expect("the fallback write must succeed");
    assert_eq!(
        fake.texts(),
        vec!["\u{1b}[Z".to_string()],
        "shift+tab is a byte a legacy terminal sends, and it is what the pane got"
    );
}

#[tokio::test]
async fn a_shift_enter_against_a_herdr_without_send_keys_degrades_to_the_byte_it_has() {
    let fake = FakeHerdr::start(false);
    let provider = provider(&fake);
    provider
        .write_pane(PANE, Input::Keys(vec!["shift+enter".into()]))
        .await
        .expect("the fallback write must succeed");
    assert_eq!(
        fake.texts(),
        vec!["\r".to_string()],
        "a legacy pane has no distinct shift+enter; the honest degradation is the plain enter"
    );
}

#[tokio::test]
async fn a_key_no_terminal_can_send_is_refused_rather_than_dropped() {
    let fake = FakeHerdr::start(false);
    let provider = provider(&fake);
    provider
        .write_pane(PANE, Input::Keys(vec!["nosuchkey".into()]))
        .await
        .expect_err("nothing to send, so the refusal must surface");
    assert_eq!(
        fake.texts(),
        Vec::<String>::new(),
        "a write of nothing must not happen"
    );
}

#[tokio::test]
async fn a_herdr_with_send_keys_never_hears_the_fallback() {
    let fake = FakeHerdr::start(true);
    let provider = provider(&fake);
    provider
        .write_pane(PANE, Input::Keys(vec!["shift+enter".into()]))
        .await
        .expect("herdr with the method takes the key itself");
    assert_eq!(fake.keys_calls(), 1);
    assert_eq!(fake.texts(), Vec::<String>::new(), "no text write rides along");
}
