//! The one subscription that could not be made, made — against a real herdr.
//!
//! Runs in a throwaway named session the test creates and destroys. `default` is never touched,
//! and a machine with no `herdr` on PATH reports a skip rather than a failure.

use kampr_core::herdr_provider::subscriptions;
use kampr_herdr::{Herdr, Sub};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

struct Session {
    socket: PathBuf,
}

impl Session {
    async fn start(tag: &str) -> Option<Self> {
        which("herdr")?;
        let name = format!("kampr-ev-{tag}-{}", std::process::id());
        assert_ne!(name, "default");
        let socket = herdr_home().join("sessions").join(&name).join("herdr.sock");
        std::process::Command::new("herdr")
            .args(["server", "--session", &name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..100 {
            if socket.exists() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let session = Self { socket };
                session
                    .herdr()
                    .call::<Value>("workspace.create", json!({ "label": "ev", "cwd": "/tmp" }))
                    .await
                    .ok()?;
                return Some(session);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    fn herdr(&self) -> Herdr {
        Herdr::new(&self.socket)
    }
}

impl Drop for Session {
    /// **Never leave a herdr behind**, including when the machine is loaded enough that
    /// `server.stop` takes seconds to land. Removing the directory while the server is still
    /// writing to it leaves the session listed and the process running, so the socket going away
    /// is what is waited on rather than a fixed sleep.
    fn drop(&mut self) {
        let socket = self.socket.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("teardown runtime")
                .block_on(async {
                    let _ = Herdr::new(&socket).call::<Value>("server.stop", json!({})).await;
                });
        })
        .join()
        .ok();
        for _ in 0..50 {
            if !self.socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let Some(dir) = self.socket.parent() else { return };
        for _ in 0..10 {
            if std::fs::remove_dir_all(dir).is_ok() && !dir.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        eprintln!("could not remove the throwaway session at {}", dir.display());
    }
}

fn which(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

fn herdr_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").expect("HOME")).join(".config"))
        .join("herdr")
}

/// The load-bearing one. The status event is what the whole triage story rests on, and it was
/// unsubscribed because herdr refuses it without a `pane_id` and one bad entry rejects the entire
/// call (probe #54). This asserts both halves against the real socket: the list the provider
/// actually builds is accepted, and the same list with the `pane_id` dropped is not.
#[tokio::test]
async fn the_status_subscription_is_accepted_only_when_every_entry_names_its_pane() {
    let Some(session) = Session::start("accept").await else {
        eprintln!("skipping: herdr is not on PATH");
        return;
    };
    let herdr = session.herdr();
    let pane = herdr.snapshot().await.unwrap().panes[0].pane_id.clone();

    let good = subscriptions(std::slice::from_ref(&pane));
    assert!(
        good.iter().any(|s| s.pane_id.as_deref() == Some(pane.as_str())),
        "the plan must name the pane"
    );
    herdr
        .subscribe(&good)
        .await
        .expect("herdr accepts a status subscription that names its pane");

    let stripped: Vec<Sub> = good
        .iter()
        .map(|s| Sub {
            kind: s.kind,
            pane_id: None,
        })
        .collect();
    let refused = herdr.subscribe(&stripped).await;
    assert!(
        refused.is_err(),
        "probe #54: one entry without a pane_id must reject the whole call — if this passes, \
         herdr changed and the per-pane split is no longer needed"
    );
}

/// The race the resubscribe loop has to survive: the pane set comes from a snapshot, and a pane
/// can close before the subscribe lands. Herdr answers `pane_not_found` and closes the socket, so
/// a stale id is as fatal as a missing one — the caller must re-derive and retry, never treat it
/// as a permanent failure.
#[tokio::test]
async fn a_pane_that_closed_between_the_snapshot_and_the_subscribe_takes_the_whole_call() {
    let Some(session) = Session::start("ghost").await else {
        eprintln!("skipping: herdr is not on PATH");
        return;
    };
    let herdr = session.herdr();
    let live = herdr.snapshot().await.unwrap().panes[0].pane_id.clone();

    let mut plan = subscriptions(std::slice::from_ref(&live));
    plan.push(Sub::pane("pane.agent_status_changed", "w9:p9"));
    let refused = match herdr.subscribe(&plan).await {
        Err(e) => e,
        Ok(_) => panic!("a pane that no longer exists must take the whole call"),
    };
    assert!(
        refused.to_string().contains("pane_not_found"),
        "expected pane_not_found, got {refused}"
    );

    // And the same list without the ghost still works, which is what makes retrying from a fresh
    // snapshot the right response rather than giving up on the status event.
    herdr
        .subscribe(&subscriptions(std::slice::from_ref(&live)))
        .await
        .expect("re-deriving the pane set recovers");
}
