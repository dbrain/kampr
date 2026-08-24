//! The observe supervisor against a herdr binary that behaves badly, and a socket that behaves.
//!
//! **The two halves of a node fail independently** (probe #233): the socket serves every surface
//! a person can look at, and the spawned `herdr terminal session observe` serves the only thing
//! they came for. So the fake here answers the socket perfectly and the fake binary is what is
//! made to misbehave, which is the shape every real report of this had.

use kampr_core::provider::Provider;
use kampr_core::registry::{PaneRegistry, PaneUpdate, RegistryConfig};
use kampr_core::{HerdrConfig, HerdrProvider};
use kampr_herdr::Herdr;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

struct Fake {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    bin: PathBuf,
}

/// A herdr whose socket answers everything the supervisor asks, and whose binary is `script`.
fn fake(script: &str) -> Fake {
    let dir = tempfile::tempdir().expect("a dir");
    let socket = dir.path().join("herdr.sock");
    let bin = dir.path().join("herdr");
    std::fs::write(&bin, script).expect("the fake binary");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let listener = UnixListener::bind(&socket).expect("bind");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(stream));
        }
    });
    Fake {
        _dir: dir,
        socket,
        bin,
    }
}

async fn serve(stream: UnixStream) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let Ok(request) = serde_json::from_str::<Value>(&line) else {
        return;
    };
    let mut stream = reader.into_inner();
    let result = match request["method"].as_str().unwrap_or_default() {
        "session.snapshot" => json!({ "snapshot": snapshot() }),
        "pane.read" => json!({ "read": { "text": "", "truncated": false } }),
        // Acknowledged and then held open, the way a real subscription is.
        "events.subscribe" => {
            let ack = json!({ "id": "kampr-events", "result": { "type": "subscription_started" } });
            if write_line(&mut stream, &ack).await.is_ok() {
                std::future::pending::<()>().await;
            }
            return;
        }
        other => json!({ "ok": other }),
    };
    let _ = write_line(&mut stream, &json!({ "id": "kampr", "result": result })).await;
}

async fn write_line(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    stream.write_all(value.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

fn snapshot() -> Value {
    json!({
        "version": "0.8.2",
        "protocol": 20,
        "focused_pane_id": "w1:p1",
        "workspaces": [{ "workspace_id": "w1", "number": 1, "label": "kampr" }],
        "tabs": [{ "tab_id": "w1:t1", "workspace_id": "w1", "label": "1" }],
        "panes": [{
            "pane_id": "w1:p1",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": "/tmp",
            "label": null,
            "agent": null,
            "agent_status": "unknown",
            "agent_session": null,
            "scroll": { "offset_from_bottom": 0, "max_offset_from_bottom": 0, "viewport_rows": 4 },
        }],
        "layouts": [{
            "tab_id": "w1:t1",
            "area": { "x": 0, "y": 0, "width": 20, "height": 4 },
            "panes": [{ "pane_id": "w1:p1", "rect": { "x": 0, "y": 0, "width": 20, "height": 4 } }],
        }],
    })
}

fn config(bin: &std::path::Path) -> HerdrConfig {
    HerdrConfig {
        binary: bin.display().to_string(),
        sweep: Duration::from_millis(600),
        sweep_watched: Duration::from_millis(600),
        settle: Duration::from_millis(20),
        // Nothing here is testing the width probe, and a re-measure restarts the stream.
        width_poll: Duration::from_secs(600),
        ..HerdrConfig::default()
    }
}

async fn online(provider: &HerdrProvider) {
    for _ in 0..200 {
        if provider.health().online {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the provider never reached the fake herdr");
}

fn text_of(u: &PaneUpdate) -> Vec<String> {
    u.rows()
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

const FRAME: &str = r#"{"type":"terminal.frame","seq":SEQ,"full":FULL,"width":20,"height":4,"encoding":"ansi","bytes":"BYTES"}"#;

fn frame(seq: u64, full: bool, bytes: &str) -> String {
    FRAME
        .replace("SEQ", &seq.to_string())
        .replace("FULL", if full { "true" } else { "false" })
        .replace("BYTES", bytes)
}

/// **`Observer::spawn` returning `Ok` means `fork` and `exec` worked and nothing more.** Clearing
/// the stream fault there called the binary half of the node healthy before a frame had ever
/// arrived, so a herdr that starts and immediately exits — one too old for the subcommand, one
/// that cannot read the socket — promised every client a grid it would then leave blank for ever,
/// while every socket-served surface went on reporting the node healthy. That is exactly the
/// shape probe #233 exists to prevent, with the guard keyed on the wrong event.
#[tokio::test(flavor = "multi_thread")]
async fn a_herdr_that_starts_and_sends_nothing_reports_a_fault_instead_of_promising_a_grid() {
    let fake = fake("#!/bin/sh\nexit 0\n");
    let provider = Arc::new(HerdrProvider::spawn(Herdr::new(&fake.socket), config(&fake.bin)));
    online(&provider).await;
    let registry = PaneRegistry::with_config(
        provider.clone(),
        RegistryConfig {
            first_grid_wait: Duration::from_millis(50),
            ..RegistryConfig::default()
        },
    );
    let _watcher = registry.watch("w1:p1").await.expect("watch");

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let panes = provider.list_panes().await.expect("list");
        if let Some(detail) = panes[0].detail.as_deref() {
            assert!(
                detail.contains("No pane on this node can show a screen"),
                "{detail}"
            );
            return;
        }
    }
    panic!("a stream that started and sent nothing reported no fault at all");
}

/// Probe #53: only the first frame of a stream is `full`, so every later one is a cursor-addressed
/// partial repaint. A lost or undecodable frame therefore desynchronises the emulator from the
/// pane for the life of the stream, in cells herdr believes it has already delivered — and the
/// NDJSON reader `continue`s straight past a line it cannot decode. Only a fresh stream's `full`
/// frame can repair it, so a gap has to restart the stream rather than publish the stale grid.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_frame_restarts_the_stream_rather_than_patching_over_the_hole() {
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' '{}'\nprintf '%s\\n' '{}'\nsleep 0.2\nprintf '%s\\n' '{}'\nsleep 30\n",
        frame(1, true, "G1syShtbMTsxSG9uZQ=="),
        frame(2, false, "G1syOzFIdHdv"),
        frame(4, false, "G1szOzFIZm91cg=="),
    );
    let fake = fake(&script);
    let provider = Arc::new(HerdrProvider::spawn(Herdr::new(&fake.socket), config(&fake.bin)));
    online(&provider).await;
    let registry = PaneRegistry::new(provider.clone());
    let mut watcher = registry.watch("w1:p1").await.expect("watch");

    let mut seen: Vec<PaneUpdate> = Vec::new();
    if watcher.is_ready() {
        seen.push(watcher.initial().clone());
    }
    while seen.len() < 3 {
        let next = tokio::time::timeout(Duration::from_secs(5), watcher.recv())
            .await
            .expect("the supervisor never published a third update")
            .expect("the watcher closed");
        seen.push(next);
    }
    assert!(seen[0].is_reset(), "the first frame is a full repaint");
    assert_eq!(text_of(&seen[0])[0], "one");
    assert_eq!(text_of(&seen[1]), ["two"], "seq 2 is an ordinary patch");
    assert!(
        seen[2].is_reset(),
        "seq 4 arrived after seq 2, so the stream is out of step and only a fresh one can \
         resynchronise it — instead it published {:?}",
        text_of(&seen[2])
    );
    assert_eq!(
        text_of(&seen[2])[0],
        "one",
        "the restarted stream repaints from the top"
    );
}
