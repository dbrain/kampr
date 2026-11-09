//! Containers from the keyboard and the mouse: a host's list switch, the ops on a container, its
//! detail panel, the host menu, the switcher, and the shell a `docker.shell` ack names.

use crate::docker_rows::{container, host, native, term};
use crate::shell::{Conn, Fake, painted, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::{Client, Event, Role};
use kampr_tui::app::{App, Options};
use kampr_tui::docker::Detail;
use kampr_tui::image::Images;
use kampr_tui::prefs::Prefs;
use kampr_tui::sidebar::Area;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u16 = 120;
const H: u16 = 40;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn desk_nodes() -> Value {
    let mut web = container("web", "running");
    web["started_at"] = json!(now() - 3 * 3600);
    web["image"] = json!("nginx:1.29");
    web["project"] = json!("shop");
    web["service"] = json!("frontdoor");
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

/// The navigator on the host, showing its containers.
fn containers(app: &mut App) {
    navigate(app);
    walk_to(app, "comingclean");
    ch(app, ']');
}

async fn ops(conn: &mut Conn) -> Vec<Value> {
    conn.heard()
        .await
        .into_iter()
        .filter(|f| f["t"] == "manage")
        .collect()
}

fn click(app: &mut App, column: u16, row: u16) {
    let click = app.mouse.hit(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        &app.layout,
        Role::Full,
    );
    app.clicked(click);
}

/// Where `needle` is drawn on the first line that has it, within the sidebar's columns.
fn spot(screen: &str, line_has: &str, needle: &str) -> (u16, u16) {
    let (y, line) = screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.chars().take(40).collect::<String>().contains(line_has))
        .unwrap_or_else(|| panic!("no line with {line_has}:\n{screen}"));
    let head: String = line.chars().take(40).collect();
    let at = head
        .rfind(needle)
        .unwrap_or_else(|| panic!("no {needle} on {line}"));
    (head[..at].chars().count() as u16, y as u16)
}

#[tokio::test]
async fn brackets_switch_a_hosts_list_and_a_relaunch_remembers_it() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let before = painted(&mut app, W, H);
    assert!(before.contains("›_") && before.contains("▣ 1"), "{before}");
    assert!(!before.contains(" web "), "terminals by default:\n{before}");

    containers(&mut app);
    let after = painted(&mut app, W, H);
    assert!(after.contains("web") && after.contains("up 3h"), "{after}");
    assert!(after.contains("+1 stopped"), "{after}");
    assert!(
        !after.contains(" db "),
        "a clean exit waits behind +1 stopped:\n{after}"
    );
    assert!(
        picked_line(&app, &after).is_some_and(|l| l.contains("comingclean")),
        "the navigator stays on the host:\n{after}"
    );

    walk_to(&mut app, "+1 stopped");
    key(&mut app, KeyCode::Enter);
    let all = painted(&mut app, W, H);
    assert!(all.contains("db") && all.contains("hide 1 stopped"), "{all}");

    ch(&mut app, '[');
    let back = painted(&mut app, W, H);
    assert!(!back.contains(" web "), "[ steps back to the terminals:\n{back}");
    ch(&mut app, ']');
    drop(app);

    let kept = Prefs::load(dir.path());
    assert_eq!(kept.hosts.area("01JNODE"), Area::Containers, "{kept:?}");
    assert!(kept.hosts.stopped("01JNODE"), "{kept:?}");
    let mut again = launched(&client, dir.path());
    let screen = painted(&mut again, W, H);
    assert!(screen.contains("web") && screen.contains("db"), "{screen}");
}

#[test]
fn a_tui_toml_with_the_old_section_folds_still_loads() {
    let dir = tempfile::tempdir().expect("a config dir");
    std::fs::write(
        Prefs::path(dir.path()),
        "order = \"priority\"\n\n[folds.01JNODE]\nmuffit = false\ncontainers = true\nstopped = true\n",
    )
    .expect("written");
    let prefs = Prefs::load(dir.path());
    assert_eq!(prefs.order, kampr_tui::sidebar::Order::Priority);
    assert_eq!(prefs.hosts.area("01JNODE"), Area::Terminals);
}

#[tokio::test]
async fn a_click_on_a_hosts_tab_shows_that_list() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let screen = painted(&mut app, W, H);
    let (x, y) = spot(&screen, "comingclean", "▣");
    click(&mut app, x, y);
    let after = painted(&mut app, W, H);
    assert!(after.contains("web") && after.contains("up 3h"), "{after}");

    let (x, y) = spot(&after, "comingclean", "›_");
    click(&mut app, x, y);
    let back = painted(&mut app, W, H);
    assert!(!back.contains("up 3h"), "{back}");
}

#[tokio::test]
async fn the_plus_on_a_running_container_shells_into_it() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    containers(&mut app);
    key(&mut app, KeyCode::Esc);
    let screen = painted(&mut app, W, H);
    ops(&mut conn).await;
    let (x, y) = spot(&screen, "web", "+");
    click(&mut app, x, y);
    let sent = ops(&mut conn).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["op"], "docker.shell");
    assert_eq!(sent[0]["id"], "web-id");
}

#[tokio::test]
async fn the_keys_on_a_container_send_its_ops_with_no_confirmation() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    containers(&mut app);
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
    containers(&mut app);
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
    containers(&mut app);
    walk_to(&mut app, "web");
    ops(&mut conn).await;
    ch(&mut app, 'r');
    assert!(ops(&mut conn).await.is_empty());
    let screen = painted(&mut app, W, H);
    assert!(screen.contains("read-only"), "{screen}");
}

fn answer(conn: &Conn, rid: &Value, logs: Value) {
    let at = now();
    conn.send(json!({
        "t": "managed", "rid": rid, "op": "docker.detail", "ok": true,
        "detail": {
            "id": "web-id", "name": "web", "image": "nginx:1.29", "state": "running",
            "started_at": at - 3 * 3600, "restarts": 2, "health": "unhealthy", "oom": true,
            "project": "shop", "service": "frontdoor",
            "command": "nginx -g daemon off;", "restart_policy": "unless-stopped",
            "networks": ["shop_default"],
            "ports": ["8080→80/tcp"],
            "env": ["PATH=/usr/bin"],
            "mounts": [{ "src": "/srv/www", "dst": "/usr/share/nginx", "mode": "ro" }],
            "limits": { "memory": 536_870_912u64, "nano_cpus": 0 },
            "stats": {
                "cpu_pct": 12.5, "mem_used": 500_000_000u64, "mem_limit": 536_870_912u64,
                "net_rx": 2_097_152u64, "net_tx": 1024u64, "blk_read": 3_145_728u64,
                "blk_write": 0u64, "pids": 7
            },
            "health_log": [{ "start": "x", "end": "y", "exit_code": 1, "output": "curl: refused" }],
            "logs": logs,
            "events": [{ "at": at - 600, "action": "die", "exit_code": 137 }]
        }
    }));
}

async fn opened(conn: &mut Conn, app: &mut App) -> Value {
    containers(app);
    walk_to(app, "web");
    ops(conn).await;
    key(app, KeyCode::Enter);
    painted(app, W, H);
    let sent = ops(conn).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["op"], "docker.detail");
    assert_eq!(sent[0]["id"], "web-id");
    sent[0]["rid"].clone()
}

/// The panel's top-left corner: it holds one size whatever it shows.
fn corner(screen: &str) -> (usize, usize) {
    screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains('┌') && l.contains(" web "))
        .and_then(|(y, l)| l.chars().position(|c| c == '┌').map(|x| (x, y)))
        .unwrap_or_else(|| panic!("no panel:\n{screen}"))
}

#[tokio::test]
async fn the_panel_spins_until_the_node_answers_then_holds_its_size_across_tabs() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let rid = opened(&mut conn, &mut app).await;
    let waiting = painted(&mut app, W, H);
    assert!(
        waiting.chars().any(|c| SPINNER.contains(&c)),
        "a spinner stands in for the first answer:\n{waiting}"
    );
    let size = corner(&waiting);

    answer(
        &conn,
        &rid,
        json!([{ "at": now() - 60, "stream": "stderr", "line": "worker 3 started" }]),
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    let info = painted(&mut app, W, H);
    assert!(!info.chars().any(|c| SPINNER.contains(&c)), "{info}");
    assert_eq!(corner(&info), size, "{info}");
    for want in [
        "running",
        "unhealthy",
        "OOM",
        "↻2",
        "s shell",
        "r restart",
        "x stop",
        "K kill",
        "a adopt",
        "Info",
        "Logs",
        "Stats",
        "Env",
        "Mounts",
        "nginx:1.29",
        "nginx -g daemon off;",
        "unless-stopped",
        "8080→80/tcp",
        "shop_default",
        "shop · frontdoor",
        "curl: refused",
        "die",
        "exit 137",
    ] {
        assert!(info.contains(want), "{want}:\n{info}");
    }
    assert!(!info.contains("D remove"), "remove waits until it stops:\n{info}");
    assert!(
        !info.contains("worker 3 started"),
        "logs are their own tab:\n{info}"
    );
    assert!(
        !info.contains("docker.detail ·"),
        "a refresh is not an op the operator typed:\n{info}"
    );

    let tabs = [
        ('2', vec!["worker 3 started"]),
        (
            '3',
            vec![
                "cpu",
                "12.5%",
                "memory",
                "476 MB / 512 MB",
                "processes",
                "7",
                "↓ 2 MB",
                "read 3 MB",
            ],
        ),
        ('4', vec!["PATH=/usr/bin"]),
        ('5', vec!["/srv/www → /usr/share/nginx (ro)"]),
    ];
    for (c, wants) in tabs {
        ch(&mut app, c);
        let screen = painted(&mut app, W, H);
        assert_eq!(corner(&screen), size, "{c}:\n{screen}");
        for want in wants {
            assert!(screen.contains(want), "{c} {want}:\n{screen}");
        }
    }
    ch(&mut app, 'h');
    assert!(
        painted(&mut app, W, H).contains("PATH=/usr/bin"),
        "h steps a tab back"
    );
    ch(&mut app, 'l');
    ch(&mut app, 'l');
    assert!(
        painted(&mut app, W, H).contains("nginx -g daemon off;"),
        "l wraps to Info"
    );

    ch(&mut app, 'r');
    let sent = ops(&mut conn).await;
    assert_eq!(sent[0]["op"], "docker.restart", "the panel's keys act on it");

    key(&mut app, KeyCode::Esc);
    let closed = painted(&mut app, W, H);
    assert!(!closed.contains("nginx:1.29"), "{closed}");
}

#[tokio::test]
async fn memory_near_its_limit_is_drawn_red() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let rid = opened(&mut conn, &mut app).await;
    answer(&conn, &rid, json!([]));
    tokio::time::sleep(Duration::from_millis(250)).await;
    ch(&mut app, '3');
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(W, H)).expect("a terminal");
    terminal.draw(|f| app.draw(f)).expect("a frame");
    let buf = terminal.backend().buffer().clone();
    let tone = |label: &str| {
        let y = (0..H)
            .find(|&y| {
                (0..W)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .contains(label)
            })
            .unwrap_or_else(|| panic!("no {label}"));
        (0..W)
            .find(|&x| buf[(x, y)].symbol() == "█")
            .map(|x| buf[(x, y)].fg)
            .unwrap_or_else(|| panic!("no bar on {label}"))
    };
    let t = kampr_tui::theme::PHOSPHOR;
    assert_eq!(tone("memory"), t.blocked, "90% of the limit is red");
    assert_eq!(tone("cpu"), t.done, "12.5% is calm");
}

#[tokio::test]
async fn the_logs_follow_the_tail_until_scrolled_and_pan_sideways() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let rid = opened(&mut conn, &mut app).await;
    let mut logs: Vec<Value> = (0..80)
        .map(|i| json!({ "at": now(), "stream": "stdout", "line": format!("line {i:03}") }))
        .collect();
    logs.push(json!({ "at": now(), "stream": "stdout", "line": format!("{}TAILEND", "x".repeat(200)) }));
    answer(&conn, &rid, json!(logs));
    tokio::time::sleep(Duration::from_millis(250)).await;
    ch(&mut app, '2');
    let tail = painted(&mut app, W, H);
    assert!(tail.contains("line 079"), "follows the tail:\n{tail}");
    assert!(!tail.contains("line 000"), "{tail}");
    assert!(!tail.contains("TAILEND"), "no wrap:\n{tail}");

    for _ in 0..30 {
        key(&mut app, KeyCode::Right);
    }
    let panned = painted(&mut app, W, H);
    assert!(
        panned.contains("TAILEND"),
        "→ pans sideways in the logs:\n{panned}"
    );
    key(&mut app, KeyCode::Home);

    key(&mut app, KeyCode::PageUp);
    let up = painted(&mut app, W, H);
    assert!(!up.contains("line 079"), "{up}");
    key(&mut app, KeyCode::End);
    let again = painted(&mut app, W, H);
    assert!(again.contains("line 079"), "end follows again:\n{again}");
}

fn panel(detail: &mut Detail) -> String {
    let area = ratatui::layout::Rect::new(0, 0, W, H);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    detail.render(None, now(), area, &mut buf, &kampr_tui::theme::PHOSPHOR);
    (0..H)
        .map(|y| {
            (0..W)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

#[test]
fn a_new_log_line_lands_only_while_the_tail_is_followed() {
    let lines = |n: usize, last: &str| {
        let mut logs: Vec<Value> = (0..n)
            .map(|i| json!({ "at": 0, "stream": "stdout", "line": format!("line {i:03}") }))
            .collect();
        logs.push(json!({ "at": 0, "stream": "stdout", "line": last }));
        json!({ "id": "web-id", "name": "web", "state": "running", "logs": logs })
    };
    let mut detail = Detail::new("01JNODE", "web-id");
    detail.answered(&lines(80, "first tail"), Instant::now());
    detail.key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    assert!(panel(&mut detail).contains("first tail"));

    detail.answered(&lines(80, "second tail"), Instant::now());
    assert!(panel(&mut detail).contains("second tail"), "following");

    detail.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    detail.answered(&lines(90, "third tail"), Instant::now());
    let held = panel(&mut detail);
    assert!(!held.contains("third tail"), "scrolled up, it stays put:\n{held}");

    detail.key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
    assert!(panel(&mut detail).contains("third tail"), "G follows again");
}

#[tokio::test]
async fn kill_while_alive_and_remove_once_stopped() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    let rid = opened(&mut conn, &mut app).await;
    answer(&conn, &rid, json!([]));
    tokio::time::sleep(Duration::from_millis(250)).await;
    painted(&mut app, W, H);
    ch(&mut app, 'D');
    assert!(ops(&mut conn).await.iter().all(|o| o["op"] != "docker.remove"));
    ch(&mut app, 'K');
    let sent: Vec<Value> = ops(&mut conn)
        .await
        .into_iter()
        .filter(|o| o["op"] != "docker.detail")
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["op"], "docker.kill");
    assert_eq!(sent[0]["id"], "web-id");
    key(&mut app, KeyCode::Esc);

    walk_to(&mut app, "+1 stopped");
    key(&mut app, KeyCode::Enter);
    walk_to(&mut app, "db");
    key(&mut app, KeyCode::Enter);
    let screen = painted(&mut app, W, H);
    assert!(
        screen.contains("D remove") && !screen.contains("K kill"),
        "{screen}"
    );
    ops(&mut conn).await;
    ch(&mut app, 'K');
    ch(&mut app, 'D');
    let sent: Vec<Value> = ops(&mut conn)
        .await
        .into_iter()
        .filter(|o| o["op"] != "docker.detail")
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["op"], "docker.remove");
    assert_eq!(sent[0]["id"], "db-id");
}

#[tokio::test]
async fn a_removed_containers_panel_closes() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut events = client.events();
    let mut app = launched(&client, dir.path());
    opened(&mut conn, &mut app).await;
    assert!(painted(&mut app, W, H).contains("Mounts"));

    let mut n = desk_nodes()[0].clone();
    n["containers"] = json!([container("db", "exited")]);
    conn.send(json!({ "t": "herd.patch", "added": { "nodes": [], "panes": [] },
        "changed": { "nodes": [n], "panes": [] }, "removed_ids": [] }));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    let screen = painted(&mut app, W, H);
    assert!(!screen.contains("Mounts"), "{screen}");
}

#[tokio::test]
async fn the_host_menu_says_its_reach_and_offers_a_new_session() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    navigate(&mut app);
    walk_to(&mut app, "comingclean");
    ch(&mut app, '.');
    let menu = painted(&mut app, W, H);
    assert!(menu.contains("this machine"), "{menu}");
    assert!(!menu.contains(" manage "), "{menu}");
    assert!(menu.contains("New…"), "{menu}");
    key(&mut app, KeyCode::Esc);
    assert!(!painted(&mut app, W, H).contains("New…"));

    ch(&mut app, 'm');
    assert!(painted(&mut app, W, H).contains("New…"), "m opens it too");
    ch(&mut app, 'n');
    let new = painted(&mut app, W, H);
    assert!(!new.contains("New…"), "{new}");
    assert!(new.contains(" manage "), "the new-session menu:\n{new}");
}

#[tokio::test]
async fn the_switcher_finds_a_container_and_opens_its_panel() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, mut conn) = desk("full").await;
    let mut app = launched(&client, dir.path());
    ops(&mut conn).await;
    for query in ["nginx", "shop", "frontdoor", "comingclean"] {
        app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        ch(&mut app, 'g');
        for c in query.chars() {
            ch(&mut app, c);
        }
        let screen = painted(&mut app, W, H);
        assert!(
            screen
                .lines()
                .any(|l| l.contains("web") && l.contains("nginx:1.29")),
            "{query}:\n{screen}"
        );
        key(&mut app, KeyCode::Esc);
    }

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    ch(&mut app, 'g');
    for c in "nginx".chars() {
        ch(&mut app, c);
    }
    key(&mut app, KeyCode::Enter);
    let screen = painted(&mut app, W, H);
    assert!(screen.contains("Mounts"), "{screen}");
    let sent = ops(&mut conn).await;
    assert_eq!(sent[0]["op"], "docker.detail", "{sent:?}");
    assert_eq!(sent[0]["id"], "web-id");
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
