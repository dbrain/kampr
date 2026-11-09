//! The sidebar's per-host sections, as rows: docs/19 "Client", mirrored from the Compose client's
//! `Docker.kt`, `Folds.kt` and `Herd.kt`.

use kampr_client::Herd;
use kampr_core::wire::{NodeEntry, PaneEntry};
use kampr_tui::sidebar::{self, Fold, Folds, Order, Row};
use serde_json::{Value, json};

pub(crate) const NOW: i64 = 1_800_000_000;

pub(crate) fn host(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "kind": "local", "online": true, "reachable": true })
}

pub(crate) fn container(name: &str, state: &str) -> Value {
    json!({
        "id": format!("{name}-id"), "name": name, "image": "postgres:18.6", "state": state,
        "started_at": NOW - 3 * 3600,
    })
}

pub(crate) fn term(id: &str) -> Value {
    json!({
        "id": id, "node_id": id.split('/').next().unwrap(),
        "workspace_id": format!("{}/w1", id.split('/').next().unwrap()),
        "tab_id": format!("{}/w1:t1", id.split('/').next().unwrap()),
        "workspace": "kampr", "tab": "1", "cwd": "/home/dbrain/dev/kampr",
        "agent_status": "unknown", "rows": 4, "cols": 12
    })
}

pub(crate) fn native(id: &str, provider: &str, container: Option<&str>) -> Value {
    json!({
        "id": id, "node_id": id.split('/').next().unwrap(), "provider": provider,
        "cwd": "/srv", "label": id.rsplit('/').next().unwrap(), "container": container,
        "agent_status": "unknown", "rows": 4, "cols": 12
    })
}

fn herd(nodes: Value, panes: Value) -> Herd {
    Herd {
        nodes: serde_json::from_value::<Vec<NodeEntry>>(nodes).expect("nodes"),
        panes: serde_json::from_value::<Vec<PaneEntry>>(panes).expect("panes"),
        ..Herd::default()
    }
}

fn drawn(herd: &Herd, folds: &Folds) -> Vec<String> {
    sidebar::rows(herd, Order::Grouped, None, folds, NOW, |_| None)
        .iter()
        .skip(1)
        .map(text)
        .collect()
}

fn text(row: &Row) -> String {
    match row {
        Row::Node { name, .. } => format!("host {name}"),
        Row::Workspace { name, .. } => format!("space {name}"),
        Row::Pane { pane, .. } => format!("pane {pane}"),
        Row::Fold {
            fold, open, count, ..
        } => {
            format!("{fold:?} {} {count}", if *open { "open" } else { "shut" })
        }
        Row::Project(project) => format!("project {project}"),
        Row::Container { entry, .. } => format!("container {}", entry.name),
        Row::Note(note) => format!("note {note}"),
        Row::Nested(inner) => format!("│{}", text(inner)),
        Row::Blank => String::new(),
        other => format!("{other:?}"),
    }
}

fn painted(rows: &[Row], width: u16) -> String {
    let area = ratatui::layout::Rect::new(0, 0, width, rows.len() as u16);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    ratatui::widgets::Widget::render(
        sidebar::Sidebar {
            rows,
            theme: &kampr_tui::theme::PHOSPHOR,
            selected: None,
            focused: None,
            top: 0,
        },
        area,
        &mut buf,
    );
    (0..area.height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

fn opened(folds: &[(&str, Fold)]) -> Folds {
    let mut set = Folds::default();
    for (node, fold) in folds {
        set.flip(node, *fold);
    }
    set
}

#[test]
fn a_host_lists_its_terminals_then_muffit_open_then_its_containers_folded() {
    let mut n = host("01JA", "front");
    n["docker"] = json!(true);
    n["containers"] = json!([container("web", "running"), container("db", "running")]);
    let herd = herd(
        json!([n]),
        json!([term("01JA/w1:p1"), native("01JA/m1", "muffit", None)]),
    );

    assert_eq!(
        drawn(&herd, &Folds::default()),
        [
            "host front",
            "space kampr",
            "pane 01JA/w1:p1",
            "Muffit open 1",
            "pane 01JA/m1",
            "Containers shut 2",
        ]
    );
}

#[test]
fn open_containers_show_the_live_ones_by_project_and_hide_the_stopped_behind_one_row() {
    let mut old_crash = container("batch", "exited");
    old_crash["exit_code"] = json!(1);
    old_crash["last_crash"] = json!({ "at": NOW - 2 * 86_400, "exit_code": 1 });
    let mut fresh_crash = container("worker", "exited");
    fresh_crash["exit_code"] = json!(137);
    fresh_crash["oom"] = json!(true);
    fresh_crash["finished_at"] = json!(NOW - 180);
    fresh_crash["last_crash"] = json!({ "at": NOW - 180, "exit_code": 137, "oom": true });
    let mut sick = container("cache", "exited");
    sick["health"] = json!("unhealthy");
    let mut db = container("postgres", "running");
    db["project"] = json!("kobbler");
    let mut api = container("api", "restarting");
    api["project"] = json!("kobbler");
    let mut n = host("01JA", "front");
    n["docker"] = json!(true);
    n["containers"] = json!([
        db,
        container("loose", "paused"),
        api,
        old_crash,
        fresh_crash,
        sick,
        container("done", "exited"),
        container("gone", "dead"),
        container("made", "created"),
    ]);
    let herd = herd(json!([n]), json!([]));

    let open = opened(&[("01JA", Fold::Containers)]);
    assert_eq!(
        drawn(&herd, &open),
        [
            "host front",
            "Containers open 9",
            "container cache",
            "note exited",
            "container loose",
            "note paused",
            "container worker",
            "note exited 137 · OOM 3m ago",
            "project kobbler",
            "container api",
            "note restarting",
            "container postgres",
            "note up 3h",
            "Stopped shut 4",
        ]
    );

    let all = opened(&[("01JA", Fold::Containers), ("01JA", Fold::Stopped)]);
    let rows = drawn(&herd, &all);
    for name in ["batch", "done", "gone", "made"] {
        assert!(rows.contains(&format!("container {name}")), "{rows:#?}");
    }
    assert_eq!(rows.last().map(String::as_str), Some("Stopped open 4"));
}

#[test]
fn a_shell_sits_under_its_container_and_keeps_it_listed_while_folded_and_stopped() {
    let mut n = host("01JA", "front");
    n["containers"] = json!([container("web", "exited"), container("db", "running")]);
    let herd = herd(
        json!([n]),
        json!([term("01JA/w1:p1"), native("01JA/n1", "native", Some("web"))]),
    );

    assert_eq!(
        drawn(&herd, &Folds::default()),
        [
            "host front",
            "space kampr",
            "pane 01JA/w1:p1",
            "Containers shut 2",
            "container web",
            "note exited",
            "│pane 01JA/n1",
        ]
    );
}

#[test]
fn an_adopted_node_hangs_under_its_container_and_not_at_the_top() {
    let mut web = container("web", "running");
    web["child"] = json!("01JA-c-web");
    let mut n = host("01JA", "front");
    n["containers"] = json!([web]);
    let mut child = host("01JA-c-web", "web-node");
    child["parent"] = json!("01JA");
    child["kind"] = json!("peer");
    let mut stray = host("01JA-c-old", "old-node");
    stray["parent"] = json!("01JA");
    stray["kind"] = json!("peer");
    let herd = herd(
        json!([n, child, stray]),
        json!([term("01JA/w1:p1"), term("01JA-c-web/w1:p1")]),
    );

    assert_eq!(
        drawn(&herd, &Folds::default()),
        [
            "host front",
            "space kampr",
            "pane 01JA/w1:p1",
            "Containers shut 1",
            "container web",
            "note up 3h",
            "│host web-node",
            "│space kampr",
            "│pane 01JA-c-web/w1:p1",
            "│host old-node",
        ]
    );
}

#[test]
fn a_parent_loop_or_a_missing_parent_still_draws_every_node_once() {
    let mut a = host("01JA", "alpha");
    a["parent"] = json!("01JB");
    let mut b = host("01JB", "beta");
    b["parent"] = json!("01JA");
    let mut c = host("01JC", "gamma");
    c["parent"] = json!("01JGONE");
    let mut d = host("01JD", "delta");
    d["parent"] = json!("01JD");
    let herd = herd(json!([a, b, c, d]), json!([]));

    let rows = drawn(&herd, &Folds::default());
    for name in ["alpha", "beta", "gamma", "delta"] {
        assert_eq!(
            rows.iter()
                .filter(|r| r.ends_with(&format!("host {name}")))
                .count(),
            1,
            "{rows:#?}"
        );
    }
    assert!(rows.contains(&"host gamma".to_string()), "{rows:#?}");
    assert!(rows.contains(&"host delta".to_string()), "{rows:#?}");
}

#[test]
fn a_docker_that_does_not_answer_is_said_and_a_host_without_docker_draws_no_section() {
    let mut down = host("01JA", "front");
    down["docker"] = json!(false);
    let herd_down = herd(json!([down]), json!([]));
    assert_eq!(
        drawn(&herd_down, &opened(&[("01JA", Fold::Containers)])),
        [
            "host front",
            "Containers open 0",
            "note docker is not answering here"
        ]
    );

    let mut idle = host("01JA", "front");
    idle["docker"] = json!(true);
    assert_eq!(
        drawn(&herd(json!([idle]), json!([])), &Folds::default()),
        ["host front"]
    );
}

#[test]
fn the_muffit_section_folds_and_says_how_many_it_holds() {
    let herd = herd(
        json!([host("01JA", "front")]),
        json!([
            native("01JA/m1", "muffit", None),
            native("01JA/m2", "muffit", None)
        ]),
    );
    assert_eq!(
        drawn(&herd, &opened(&[("01JA", Fold::Muffit)])),
        ["host front", "Muffit shut 2"]
    );
}

#[test]
fn a_container_row_carries_its_restarts_and_health_and_the_fold_rows_are_selectable() {
    let mut web = container("web", "running");
    web["restarts"] = json!(4);
    web["health"] = json!("unhealthy");
    let mut n = host("01JA", "front");
    n["containers"] = json!([web]);
    let herd = herd(json!([n]), json!([]));
    let rows = sidebar::rows(
        &herd,
        Order::Grouped,
        None,
        &opened(&[("01JA", Fold::Containers)]),
        NOW,
        |_| None,
    );
    let row = rows
        .iter()
        .find(|r| matches!(r, Row::Container { .. }))
        .expect("a container row");
    assert!(row.selectable());
    assert!(
        rows.iter()
            .any(|r| matches!(r, Row::Fold { .. }) && r.selectable())
    );

    let screen = painted(&rows, 30);
    let line = screen.lines().find(|l| l.contains("web")).unwrap_or_default();
    assert!(line.contains("↻4") && line.contains("unhealthy"), "{screen}");
    assert!(screen.contains("▾ containers · docker"), "{screen}");
}

#[test]
fn a_failing_adoption_is_listed_whatever_its_state_and_says_so() {
    let mut stuck = container("stuck", "exited");
    stuck["adopt_error"] = json!("stuck is not running");
    let mut gone = container("gone", "missing");
    gone["adopt_error"] = json!("gone no longer exists");
    let mut n = host("01JA", "front");
    n["containers"] = json!([stuck, gone]);
    let herd = herd(json!([n]), json!([]));
    let open = opened(&[("01JA", Fold::Containers)]);
    assert_eq!(
        drawn(&herd, &open),
        [
            "host front",
            "Containers open 2",
            "container gone",
            "note gone",
            "container stuck",
            "note exited",
        ]
    );

    let rows = sidebar::rows(&herd, Order::Grouped, None, &open, NOW, |_| None);
    let screen = painted(&rows, 40);
    assert_eq!(screen.matches("adoption failing").count(), 2, "{screen}");
}
