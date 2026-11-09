//! Containers from the keyboard and the mouse: folding a host's sections, the ops on a container,
//! its detail panel, and the shell a `docker.shell` ack names.

use crate::docker_rows::{container, host, native, term};
use crate::shell::{Conn, Fake, painted, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::{Client, Event, Role};
use kampr_tui::app::{App, Options};
use kampr_tui::docker::Detail;
use kampr_tui::image::Images;
use kampr_tui::prefs::Prefs;
use kampr_tui::sidebar::Fold;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u16 = 120;
const H: u16 = 40;

fn desk_nodes() -> Value {
    let mut web = container("web", "running");
    web["started_at"] = json!(now() - 3 * 3600);
    let mut db = container("db", "exited");
    db["exit_code"] = json!(0);
    let mut n = host("01JNODE", "comingclean");
    n["docker"] = json!(true);
    n["containers"] = json!([web, db]);
    json!([n])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs() as i64
}

async fn desk(role: &str) -> (Arc<Client>, Conn) {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(desk_nodes(), json!([term("01JNODE/w1:p1")]), role);
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    (client, conn)
}

fn launched(client: &Arc<Client>, dir: &std::path::Path) -> App {
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.remember_in(dir.to_path_buf());
    app.refocus();
    app
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn ch(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}

fn navigate(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    ch(app, 'w');
}

/// Walks the navigator down until the row it is on draws `text`.
fn walk_to(app: &mut App, text: &str) {
    for _ in 0..20 {
        let screen = painted(app, W, H);
        if picked_line(app, &screen).is_some_and(|l| l.contains(text)) {
            return;
        }
        key(app, KeyCode::Down);
    }
    panic!("the navigator never reached {text}:\n{}", painted(app, W, H));
}

fn picked_line(app: &App, screen: &str) -> Option<String> {
    let (_, rect) = app.layout.rows.get(app.picked_row()?)?;
    screen
        .lines()
        .nth(rect.y as usize)
        .map(|l| l.chars().take(rect.width as usize).collect())
}

async fn ops(conn: &mut Conn) -> Vec<Value> {
    conn.heard()
        .await
        .into_iter()
        .filter(|f| f["t"] == "manage")
        .collect()
}

#[tokio::test]
async fn enter_unfolds_the_containers_and_a_relaunch_remembers_it() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let before = painted(&mut app, W, H);
    assert!(before.contains("▸ containers · docker"), "{before}");
    assert!(!before.contains(" web "), "folded by default:\n{before}");

    navigate(&mut app);
    walk_to(&mut app, "containers · docker");
    key(&mut app, KeyCode::Enter);
    let after = painted(&mut app, W, H);
    assert!(after.contains("▾ containers · docker"), "{after}");
    assert!(after.contains("web") && after.contains("up 3h"), "{after}");
    assert!(after.contains("+1 stopped"), "{after}");
    assert!(
        !after.contains(" db "),
        "a clean exit waits behind +1 stopped:\n{after}"
    );

    walk_to(&mut app, "+1 stopped");
    key(&mut app, KeyCode::Enter);
    let all = painted(&mut app, W, H);
    assert!(all.contains("db") && all.contains("hide 1 stopped"), "{all}");
    drop(app);

    let kept = Prefs::load(dir.path());
    assert!(kept.folds.open("01JNODE", Fold::Containers), "{kept:?}");
    assert!(kept.folds.open("01JNODE", Fold::Stopped), "{kept:?}");
    let mut again = launched(&client, dir.path());
    let screen = painted(&mut again, W, H);
    assert!(
        screen.contains("▾ containers · docker") && screen.contains("db"),
        "{screen}"
    );
}

#[tokio::test]
async fn a_click_on_a_section_header_folds_it() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let screen = painted(&mut app, W, H);
    let y = screen
        .lines()
        .position(|l| l.contains("containers · docker"))
        .expect("a header");
    let click = app.mouse.hit(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: y as u16,
            modifiers: KeyModifiers::NONE,
        },
        &app.layout,
        Role::Full,
    );
    app.clicked(click);
    let after = painted(&mut app, W, H);
    assert!(
        after.contains("▾ containers · docker") && after.contains("web"),
        "{after}"
    );
}

#[tokio::test]
async fn the_keys_on_a_container_send_its_ops_with_no_confirmation() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    navigate(&mut app);
    walk_to(&mut app, "containers · docker");
    key(&mut app, KeyCode::Enter);
    walk_to(&mut app, "web");
    ops(&mut conn).await;

    for (c, op) in [
        ('r', "docker.restart"),
        ('x', "docker.stop"),
        ('a', "docker.adopt"),
        ('s', "docker.shell"),
    ] {
        ch(&mut app, c);
        let sent = ops(&mut conn).await;
        assert_eq!(sent.len(), 1, "{c}: {sent:?}");
        assert_eq!(sent[0]["op"], op);
        assert_eq!(sent[0]["node"], "01JNODE");
        assert_eq!(sent[0]["id"], "web-id");
    }

    walk_to(&mut app, "+1 stopped");
    key(&mut app, KeyCode::Enter);
    walk_to(&mut app, "db");
    ch(&mut app, 'x');
    let sent = ops(&mut conn).await;
    assert_eq!(sent[0]["op"], "docker.start", "{sent:?}");
}

#[tokio::test]
async fn a_shell_ack_opens_the_pane_it_names_once_the_herd_brings_it() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut events = client.events();
    let mut app = launched(&client, dir.path());
    navigate(&mut app);
    walk_to(&mut app, "containers · docker");
    key(&mut app, KeyCode::Enter);
    walk_to(&mut app, "web");
    ch(&mut app, 's');
    let sent = ops(&mut conn).await;
    conn.send(json!({
        "t": "managed", "rid": sent[0]["rid"], "op": "docker.shell", "ok": true, "id": "01JNODE/n:web1"
    }));
    tokio::time::sleep(Duration::from_millis(250)).await;
    painted(&mut app, W, H);
    assert_eq!(app.focused(), Some("01JNODE/w1:p1"));

    conn.send(json!({ "t": "herd.patch", "added": {
        "nodes": [], "panes": [native("01JNODE/n:web1", "native", Some("web"))]
    }, "changed": { "nodes": [], "panes": [] }, "removed_ids": [] }));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    let screen = painted(&mut app, W, H);
    assert_eq!(app.focused(), Some("01JNODE/n:web1"), "{screen}");
}

#[tokio::test]
async fn a_read_only_device_sends_nothing_and_says_why() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("read").await;
    let mut app = launched(&client, dir.path());
    navigate(&mut app);
    walk_to(&mut app, "containers · docker");
    key(&mut app, KeyCode::Enter);
    walk_to(&mut app, "web");
    ops(&mut conn).await;
    ch(&mut app, 'r');
    assert!(ops(&mut conn).await.is_empty());
    let screen = painted(&mut app, W, H);
    assert!(screen.contains("read-only"), "{screen}");
}

#[tokio::test]
async fn enter_on_a_container_opens_its_detail_and_the_node_fills_it() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    navigate(&mut app);
    walk_to(&mut app, "containers · docker");
    key(&mut app, KeyCode::Enter);
    walk_to(&mut app, "web");
    ops(&mut conn).await;
    key(&mut app, KeyCode::Enter);
    painted(&mut app, W, H);
    let sent = ops(&mut conn).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["op"], "docker.detail");
    assert_eq!(sent[0]["id"], "web-id");

    let at = now();
    conn.send(json!({
        "t": "managed", "rid": sent[0]["rid"], "op": "docker.detail", "ok": true,
        "detail": {
            "id": "web-id", "name": "web", "image": "nginx:1.29", "state": "running",
            "started_at": at - 3 * 3600, "restarts": 2,
            "ports": ["8080→80/tcp"],
            "env": ["PATH=/usr/bin"],
            "mounts": [{ "src": "/srv/www", "dst": "/usr/share/nginx", "mode": "ro" }],
            "stats": { "cpu_pct": 12.5, "mem_used": 134_217_728u64, "mem_limit": 536_870_912u64, "pids": 7 },
            "health_log": [{ "start": "x", "end": "y", "exit_code": 1, "output": "curl: refused" }],
            "logs": [{ "at": at - 60, "stream": "stderr", "line": "worker 3 started" }],
            "events": [{ "at": at - 600, "action": "die", "exit_code": 137 }]
        }
    }));
    tokio::time::sleep(Duration::from_millis(250)).await;
    let screen = painted(&mut app, W, H);
    for want in [
        "nginx:1.29",
        "cpu 12.5%",
        "128 MB / 512 MB",
        "8080→80/tcp",
        "PATH=/usr/bin",
        "/srv/www → /usr/share/nginx",
        "curl: refused",
        "worker 3 started",
        "die",
        "exit 137",
    ] {
        assert!(screen.contains(want), "{want}:\n{screen}");
    }
    assert!(
        !screen.contains("docker.detail ·"),
        "a refresh is not an op the operator typed:\n{screen}"
    );

    ch(&mut app, 'r');
    let sent = ops(&mut conn).await;
    assert_eq!(sent[0]["op"], "docker.restart", "the panel's keys act on it");

    key(&mut app, KeyCode::Esc);
    let closed = painted(&mut app, W, H);
    assert!(!closed.contains("nginx:1.29"), "{closed}");
}

#[test]
fn the_detail_asks_again_every_three_seconds_and_never_twice_at_once() {
    let mut detail = Detail::new("01JNODE", "web-id");
    let t0 = Instant::now();
    assert!(detail.due(t0));
    detail.asked(t0);
    assert!(!detail.due(t0 + Duration::from_secs(5)), "one in flight");
    detail.answered(&json!({ "name": "web" }), t0 + Duration::from_millis(100));
    assert!(!detail.due(t0 + Duration::from_secs(3)));
    assert!(detail.due(t0 + Duration::from_millis(3100)));

    detail.asked(t0 + Duration::from_millis(3100));
    detail.refused("docker is not answering".into(), t0 + Duration::from_secs(4));
    assert!(
        detail.due(t0 + Duration::from_secs(7)),
        "a refusal ends the ask too"
    );
}
