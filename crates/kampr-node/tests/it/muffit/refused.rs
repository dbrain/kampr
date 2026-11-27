//! A muffit daemon that answers but will not take this node's token — the case where a second
//! daemon holds the port a node dials — is said aloud, once, instead of listing nothing.

use super::warnings::Warnings;
use super::{Fake, Harness, SOON, TOKEN};
use kampr_muffit::{Client, Reach};
use std::time::Duration;

async fn attempts(fake: &Fake, at_least: usize) {
    let deadline = tokio::time::Instant::now() + SOON;
    while fake
        .seen()
        .iter()
        .filter(|s| s.path == "/sessions/stream")
        .count()
        < at_least
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the node never retried the daemon"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_daemon_that_refuses_the_token_is_said_once_not_once_per_retry() {
    let (warnings, _guard) = Warnings::capture();
    let fake = Fake::start(Vec::new()).await;
    let _node = Harness::beside(&fake.url, "the-other-daemons-token").await;
    attempts(&fake, 3).await;

    let said = warnings.naming(&fake.url);
    assert_eq!(said.len(), 1, "{said:#?}");
    assert!(said[0].contains("401"), "{said:#?}");
}

#[tokio::test]
async fn reaching_a_daemon_tells_a_refusal_from_an_absence() {
    let fake = Fake::start(Vec::new()).await;
    assert_eq!(
        Client::new(&fake.url, TOKEN).unwrap().reach().await,
        Reach::Answers
    );

    let Reach::Refused(refusal) = Client::new(&fake.url, "wrong").unwrap().reach().await else {
        panic!("a wrong token is a refusal");
    };
    assert_eq!(refusal.status, 401);

    let gone = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    assert!(matches!(
        Client::new(&gone, TOKEN).unwrap().reach().await,
        Reach::Away(_)
    ));
}
