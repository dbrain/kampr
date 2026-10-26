//! Pairing with the hub from inside the TUI: the sidebar offers it, a code is all it asks for, and
//! the herd on screen becomes the hub's.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use futures_util::{SinkExt, StreamExt};
use kampr_client::{Client, Event, HubOffer, Policy, Role, Session, Via};
use kampr_core::Backoff;
use kampr_tui::app::{App, Options};
use kampr_tui::image::Images;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

const BEAT: Duration = Duration::from_secs(3);

/// A node that greets once and then says nothing, which is all the sidebar needs.
async fn node() -> Arc<Client> {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(socket) = tokio_tungstenite::accept_hdr_async(
                    stream,
                    #[allow(clippy::result_large_err)]
                    |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        if let Some(p) = request.headers().get("sec-websocket-protocol") {
                            response.headers_mut().insert("sec-websocket-protocol", p.clone());
                        }
                        Ok(response)
                    },
                )
                .await
                else {
                    return;
                };
                let (mut sink, mut source) = socket.split();
                for frame in [
                    json!({ "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "comingclean",
                            "build": "0.1.29", "role": "full",
                            "caps": { "scrollback": true, "conversation": true, "manage": true } }),
                    json!({ "t": "herd", "nodes": [{ "id": "01JNODE", "name": "comingclean", "kind": "local", "online": true }],
                            "panes": [{ "id": "01JNODE/w1:p1", "node_id": "01JNODE", "workspace_id": "01JNODE/w1",
                                        "tab_id": "01JNODE/w1:t1", "workspace": "herdr", "agent_status": "unknown" }] }),
                    json!({ "t": "prefs", "panes": {} }),
                ] {
                    let _ = sink.send(Message::text(frame.to_string())).await;
                }
                while source.next().await.is_some() {}
            });
        }
    });
    let client = Arc::new(Client::with_policy(
        Session {
            origin,
            token: "local-token".into(),
            via: Via::LocalNode {
                node_name: "comingclean".into(),
                device: "cli@comingclean".into(),
            },
        },
        Policy {
            backoff: Backoff {
                initial: Duration::from_millis(10),
                max: Duration::from_millis(50),
            },
            connect_timeout: Duration::from_secs(2),
            manage_timeout: Duration::from_millis(500),
            event_capacity: 256,
        },
    ));
    let mut events = client.events();
    tokio::time::timeout(BEAT, async {
        while !matches!(events.recv().await, Ok(Event::Prefs { .. })) {}
    })
    .await
    .expect("greeted");
    client
}

/// The hub's `/auth/pair`, answering one code with one token, and saying what it was asked.
async fn hub() -> (String, tokio::sync::oneshot::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    let (asked, heard) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().to_string())
                    })
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                if body.len() >= length {
                    let _ = asked.send(serde_json::from_str::<Value>(body).unwrap_or_default());
                    break;
                }
            }
        }
        let body = json!({ "token": "hub-token" }).to_string();
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
    });
    (origin, heard)
}

fn painted(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 24)).expect("a test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **"How do I pair to the hub?" had no answer on screen.** A peer's own herd is one machine, and
/// the way to the rest was a command in a doc. Now the sidebar says the hub is there, a click asks
/// for the code `kampr pair` printed on it, and the herd this client is showing becomes the hub's.
#[tokio::test]
async fn the_sidebar_offers_the_hub_and_a_code_is_all_pairing_asks_for() {
    let client = node().await;
    let (origin, heard) = hub().await;
    let home = std::env::temp_dir().join(format!("kampr-tui-pair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();
    app.offer_hub(HubOffer {
        name: "giftofthemagi2".into(),
        origin: origin.clone(),
        config_dir: home.clone(),
    });

    let offered = painted(&mut app);
    assert!(offered.contains("pair with giftofthemagi2"), "{offered}");

    let row = app.layout.pair.expect("the offer is a row a click can reach");
    let click = app.mouse.hit(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: row.x + 2,
            row: row.y,
            modifiers: KeyModifiers::NONE,
        },
        &app.layout,
        Role::Full,
    );
    app.clicked(click);
    let asking = painted(&mut app);
    assert!(
        asking.contains("kampr pair"),
        "the prompt says where the code comes from:\n{asking}"
    );

    for c in "ab12-cd34".chars() {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let asked = tokio::time::timeout(BEAT, heard)
        .await
        .expect("the hub was asked")
        .expect("a body");
    assert_eq!(asked["code"], "ab12-cd34");

    let (session, route) = tokio::time::timeout(BEAT, async {
        loop {
            painted(&mut app);
            if let Some(session) = app.switching() {
                return session;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the client moved to the hub");
    assert_eq!(session.origin, origin);
    assert_eq!(session.token, "hub-token");
    let route = route.expect("this machine's herd stays behind the hub it just paired with");
    assert!(route.on_hub());
    assert_eq!(route.local.token, "local-token");
    assert!(app.quitting(), "this herd's loop ends so the hub's can start");
    let saved = kampr_client::ClientConfig::load(&home).expect("saved");
    assert_eq!(saved.default.as_deref(), Some("giftofthemagi2"));
    let _ = std::fs::remove_dir_all(&home);
}

/// A node's `/api/pair`: answers one request with a code, and hands back the request's head.
async fn minting() -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    let (asked, heard) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut asked = Some(asked);
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            if !head.starts_with("POST /api/pair") {
                continue;
            }
            if let Some(asked) = asked.take() {
                let _ = asked.send(head);
            }
            let body = json!({ "code": "ab12-cd34", "role": "full", "expires_in": 600 }).to_string();
            let _ = stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });
    (origin, heard)
}

/// The other half of pairing: a device already signed in hands a new one a code, with no trip to
/// the hub's console — the app's "Pair a device", from the keyboard.
#[tokio::test]
async fn a_signed_in_terminal_mints_a_code_for_another_device() {
    let (origin, heard) = minting().await;
    let client = Arc::new(Client::start(Session {
        origin: origin.clone(),
        token: "desk-token".into(),
        via: Via::Profile {
            name: "giftofthemagi2".into(),
        },
    }));
    let mut app = App::new(client, Options::default(), Images::default());
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));

    let head = tokio::time::timeout(BEAT, heard)
        .await
        .expect("the node was asked")
        .expect("a request");
    assert!(
        head.to_ascii_lowercase()
            .contains("authorization: bearer desk-token"),
        "asked as this device:\n{head}"
    );
    let shown = tokio::time::timeout(BEAT, async {
        loop {
            let screen = painted(&mut app);
            if screen.contains("ab12-cd34") {
                return screen;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the code reached the screen");
    assert!(
        shown.contains(&format!("kampr connect {origin} --code ab12-cd34")),
        "and says what to type on the other machine:\n{shown}"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!painted(&mut app).contains("ab12-cd34"), "esc puts it away");
}
