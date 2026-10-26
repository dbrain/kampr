//! The hub and the machine behind it: the hub's herd while it answers, this one while it does
//! not, and the hub again the moment it is back.

use crate::shell::painted;
use futures_util::{SinkExt, StreamExt};
use kampr_client::{Client, Policy, Session, Via};
use kampr_core::Backoff;
use kampr_tui::app::{App, Options};
use kampr_tui::image::Images;
use kampr_tui::route::Route;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

const BEAT: Duration = Duration::from_secs(5);

/// A node on `listener` that greets every socket and then listens, or turns every token away.
fn serve(listener: TcpListener, refuse: bool) {
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(socket) = tokio_tungstenite::accept_hdr_async(
                    stream,
                    #[allow(clippy::result_large_err)]
                    move |request: &Request, mut response: Response| {
                        if refuse {
                            let mut no = ErrorResponse::new(Some("this device is not authorised".into()));
                            *no.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED;
                            return Err(no);
                        }
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
                            "build": "0.1.29", "role": "full", "caps": {} }),
                    json!({ "t": "herd", "nodes": [{ "id": "01JNODE", "name": "comingclean", "kind": "local", "online": true }],
                            "panes": [] }),
                    json!({ "t": "prefs", "panes": {} }),
                ] {
                    let _ = sink.send(Message::text(frame.to_string())).await;
                }
                while source.next().await.is_some() {}
            });
        }
    });
}

async fn listening() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    (listener, origin)
}

fn session(origin: &str, via: Via) -> Session {
    Session {
        origin: origin.to_string(),
        token: "a-token".into(),
        via,
    }
}

fn hub(origin: &str) -> Session {
    session(
        origin,
        Via::Profile {
            name: "giftofthemagi2".into(),
        },
    )
}

fn local(origin: &str) -> Session {
    session(
        origin,
        Via::LocalNode {
            node_name: "comingclean".into(),
            device: "cli@comingclean".into(),
        },
    )
}

fn opened(on: &Session, route: Route) -> App {
    let client = Arc::new(Client::with_policy(
        on.clone(),
        Policy {
            backoff: Backoff {
                initial: Duration::from_millis(10),
                max: Duration::from_millis(50),
            },
            connect_timeout: Duration::from_secs(1),
            manage_timeout: Duration::from_millis(500),
            event_capacity: 256,
        },
    ));
    let mut app = App::new(client, Options::default(), Images::default());
    app.route(Some(route));
    app
}

/// What the run loop does: absorb, draw, and see whether this herd's loop is over.
async fn run(app: &mut App) -> (Session, Route) {
    let mut events = app.client().events();
    tokio::time::timeout(BEAT, async {
        loop {
            while let Ok(event) = events.try_recv() {
                app.absorb(&event);
            }
            painted(app, 100, 20);
            if let Some((next, route)) = app.switching() {
                return (next, route.expect("the route goes on to the next herd"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the herd on screen changed")
}

#[tokio::test]
async fn a_hub_that_stops_answering_hands_over_to_this_machine_and_takes_back_over_when_it_returns() {
    let (here, here_origin) = listening().await;
    serve(here, false);
    let (gone, hub_origin) = listening().await;
    let address = gone.local_addr().expect("an address");
    drop(gone);
    let mut route = Route::new(hub(&hub_origin), local(&here_origin), std::env::temp_dir());
    route.grace = Duration::from_millis(300);
    route.every = Duration::from_millis(100);

    let mut on_hub = opened(&hub(&hub_origin), route);
    let (next, route) = run(&mut on_hub).await;
    assert_eq!(next.origin, here_origin, "this machine's herd stands in");
    assert!(!route.on_hub());

    let mut standing_in = opened(&next, route);
    let screen = painted(&mut standing_in, 100, 20);
    assert!(
        screen.contains("giftofthemagi2 unreachable") || screen.contains("magi2 unreachable"),
        "the sidebar says why this is not the hub's herd:\n{screen}"
    );
    serve(
        TcpListener::bind(address).await.expect("the hub's port again"),
        false,
    );
    let (back, route) = run(&mut standing_in).await;
    assert_eq!(back.origin, hub_origin, "the hub is back, so its herd is");
    assert!(route.on_hub());
}

#[tokio::test]
async fn a_hub_that_turns_this_device_away_falls_back_at_once_and_offers_to_pair_again() {
    let (here, here_origin) = listening().await;
    serve(here, false);
    let (refusing, hub_origin) = listening().await;
    serve(refusing, true);
    let route = Route::new(hub(&hub_origin), local(&here_origin), std::env::temp_dir());

    let mut on_hub = opened(&hub(&hub_origin), route);
    let (next, route) = run(&mut on_hub).await;
    assert_eq!(
        next.origin, here_origin,
        "with no ten-second wait: the token will not get better"
    );
    assert!(route.refused);

    let mut standing_in = opened(&next, route);
    let screen = painted(&mut standing_in, 100, 20);
    assert!(screen.contains("pair with giftofthemagi2"), "{screen}");
    assert!(
        !screen.contains("unreachable"),
        "it is reachable, it just does not know this device:\n{screen}"
    );
}
