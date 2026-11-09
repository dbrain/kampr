//! The sidebar's per-host lists, as rows: docs/19 "Client", mirrored from the Compose client's
//! `Docker.kt`, `Folds.kt` and `HerdGroup.kt`.

use kampr_client::Herd;
use kampr_core::wire::{NodeEntry, PaneEntry};
use kampr_tui::sidebar::{self, Area, Hosts, Order, Row, Tab};
use ratatui::style::Color;
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

fn drawn(herd: &Herd, hosts: &Hosts) -> Vec<String> {
    sidebar::rows(herd, Order::Grouped, None, hosts, NOW, |_| None)
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
        Row::Stopped { open, count, .. } => {
            format!("stopped {} {count}", if *open { "open" } else { "shut" })
        }
        Row::Project(project) => format!("project {project}"),
        Row::Container { entry, .. } => format!("container {}", entry.name),
        Row::Note(note) => format!("note {note}"),
        Row::Nested(inner) => format!("│{}", text(inner)),
        Row::Blank => String::new(),
        other => format!("{other:?}"),
    }
}

fn tabs(herd: &Herd, hosts: &Hosts) -> (Vec<Tab>, Area) {
    sidebar::rows(herd, Order::Grouped, None, hosts, NOW, |_| None)
        .into_iter()
        .find_map(|r| match r {
            Row::Node { tabs, area, .. } => Some((tabs, area)),
            _ => None,
        })
        .expect("a host row")
}

fn buffer(rows: &[Row], width: u16) -> ratatui::buffer::Buffer {
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
    buf
}

fn painted(rows: &[Row], width: u16) -> String {
    let buf = buffer(rows, width);
    (0..buf.area.height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

fn showing(picks: &[(&str, Area)]) -> Hosts {
    let mut hosts = Hosts::default();
    for (node, area) in picks {
        hosts.show(node, *area);
    }
    hosts
}

fn front_with_everything() -> Herd {
    let mut n = host("01JA", "front");
    n["docker"] = json!(true);
    n["containers"] = json!([
        container("web", "running"),
        container("db", "running"),
        container("old", "exited")
    ]);
    herd(
        json!([n]),
        json!([term("01JA/w1:p1"), native("01JA/m1", "muffit", None)]),
    )
}

#[test]
fn a_host_lists_its_terminals_by_default_and_offers_a_switch_for_the_lists_it_has() {
    let herd = front_with_everything();

    assert_eq!(
        drawn(&herd, &Hosts::default()),
        ["host front", "space kampr", "pane 01JA/w1:p1"]
    );
    assert_eq!(
        tabs(&herd, &Hosts::default()),
        (
            vec![
                Tab {
                    area: Area::Terminals,
                    count: 1
                },
                Tab {
                    area: Area::Containers,
                    count: 2
                },
                Tab {
                    area: Area::Muffit,
                    count: 1
                },
            ],
            Area::Terminals
        ),
        "the containers count is the running ones"
    );
    assert_eq!(
        drawn(&herd, &showing(&[("01JA", Area::Muffit)])),
        ["host front", "pane 01JA/m1"]
    );
}

#[test]
fn a_host_with_only_terminals_has_no_switch_and_a_pick_whose_list_went_falls_back() {
    let herd = herd(json!([host("01JA", "front")]), json!([term("01JA/w1:p1")]));
    let gone = showing(&[("01JA", Area::Containers)]);

    assert_eq!(tabs(&herd, &gone), (vec![], Area::Terminals));
    assert_eq!(
        drawn(&herd, &gone),
        ["host front", "space kampr", "pane 01JA/w1:p1"]
    );
}

#[test]
fn the_containers_list_shows_the_live_ones_by_project_and_hides_the_stopped_behind_one_row() {
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
    let herd = herd(json!([n]), json!([term("01JA/w1:p1")]));

    let mut hosts = showing(&[("01JA", Area::Containers)]);
    assert_eq!(
        drawn(&herd, &hosts),
        [
            "host front",
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
            "stopped shut 4",
        ]
    );

    hosts.flip_stopped("01JA");
    let rows = drawn(&herd, &hosts);
    for name in ["batch", "done", "gone", "made"] {
        assert!(rows.contains(&format!("container {name}")), "{rows:#?}");
    }
    assert_eq!(rows.last().map(String::as_str), Some("stopped open 4"));
}

#[test]
fn a_shell_sits_under_its_container_in_the_containers_list_and_not_among_the_terminals() {
    let mut n = host("01JA", "front");
    n["containers"] = json!([container("web", "exited"), container("db", "running")]);
    let herd = herd(
        json!([n]),
        json!([term("01JA/w1:p1"), native("01JA/n1", "native", Some("web"))]),
    );

    assert_eq!(
        drawn(&herd, &Hosts::default()),
        ["host front", "space kampr", "pane 01JA/w1:p1"]
    );
    assert_eq!(
        drawn(&herd, &showing(&[("01JA", Area::Containers)])),
        [
            "host front",
            "container db",
            "note up 3h",
            "container web",
            "note exited",
            "│pane 01JA/n1",
        ]
    );
}

#[test]
fn an_adopted_node_hangs_under_its_container_and_a_stray_shows_whichever_list_is_up() {
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
        drawn(&herd, &Hosts::default()),
        ["host front", "space kampr", "pane 01JA/w1:p1", "│host old-node",]
    );
    assert_eq!(
        drawn(&herd, &showing(&[("01JA", Area::Containers)])),
        [
            "host front",
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
fn a_row_belongs_to_the_host_above_it_and_not_to_a_child_drawn_under_an_earlier_container() {
    let mut adopted = container("a-web", "running");
    adopted["child"] = json!("01JA-c-web");
    let mut n = host("01JA", "front");
    n["containers"] = json!([adopted, container("b-db", "running")]);
    let mut child = host("01JA-c-web", "web-node");
    child["parent"] = json!("01JA");
    child["kind"] = json!("peer");
    let herd = herd(
        json!([n, child]),
        json!([
            term("01JA-c-web/w1:p1"),
            native("01JA/n1", "native", Some("b-db"))
        ]),
    );
    let rows = sidebar::rows(
        &herd,
        Order::Grouped,
        None,
        &showing(&[("01JA", Area::Containers)]),
        NOW,
        |_| None,
    );
    let at = |want: &str| {
        rows.iter()
            .position(|r| text(r) == want)
            .unwrap_or_else(|| panic!("no {want}: {rows:#?}"))
    };
    let owner = |i: usize| sidebar::host_of(&rows, i).map(|(node, ..)| node);

    assert_eq!(owner(at("│pane 01JA/n1")).as_deref(), Some("01JA"));
    assert_eq!(owner(at("│pane 01JA-c-web/w1:p1")).as_deref(), Some("01JA-c-web"));
    assert_eq!(owner(at("container b-db")).as_deref(), Some("01JA"));
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

    let rows = drawn(&herd, &Hosts::default());
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
fn a_docker_that_does_not_answer_is_offered_and_said_and_a_host_without_docker_has_no_switch() {
    let mut down = host("01JA", "front");
    down["docker"] = json!(false);
    let herd_down = herd(json!([down]), json!([]));
    let picked = showing(&[("01JA", Area::Containers)]);
    assert_eq!(
        tabs(&herd_down, &picked),
        (
            vec![
                Tab {
                    area: Area::Terminals,
                    count: 0
                },
                Tab {
                    area: Area::Containers,
                    count: 0
                },
            ],
            Area::Containers
        )
    );
    assert_eq!(
        drawn(&herd_down, &picked),
        ["host front", "note docker is not answering here"]
    );
    assert_eq!(
        drawn(&herd_down, &Hosts::default()),
        ["host front", "note no terminals here"]
    );

    let mut idle = host("01JA", "front");
    idle["docker"] = json!(true);
    let herd_idle = herd(json!([idle]), json!([]));
    assert_eq!(tabs(&herd_idle, &picked), (vec![], Area::Terminals));
}

#[test]
fn the_switch_is_drawn_on_the_host_line_with_its_counts_capped_at_nine() {
    let mut n = host("01JA", "front");
    n["docker"] = json!(true);
    n["containers"] = json!(
        (0..12)
            .map(|i| container(&format!("c{i:02}"), "running"))
            .collect::<Vec<_>>()
    );
    let herd = herd(
        json!([n]),
        json!([term("01JA/w1:p1"), native("01JA/m1", "muffit", None)]),
    );
    let rows = sidebar::rows(&herd, Order::Grouped, None, &Hosts::default(), NOW, |_| None);
    let screen = painted(&rows, 40);
    let line = screen.lines().find(|l| l.contains("front")).unwrap_or_default();
    assert!(
        line.contains("›_") && line.contains("▣ 9+") && line.contains("◉ 1"),
        "{screen}"
    );
    assert!(
        !line.contains("ms") && !line.contains("local"),
        "no latency on the row: {line}"
    );

    let buf = buffer(&rows, 40);
    let y = screen.lines().position(|l| l.contains("front")).unwrap() as u16;
    let x = line.find("›_").map(|b| line[..b].chars().count()).unwrap() as u16;
    assert!(
        buf[(x, y)].modifier.contains(ratatui::style::Modifier::REVERSED),
        "the list showing is in reverse video"
    );
    let name_x = line.find("front").map(|b| line[..b].chars().count()).unwrap() as u16;
    assert_eq!(
        buf[(name_x, y)].fg,
        kampr_tui::theme::PHOSPHOR.accent,
        "this machine's name is in the accent colour"
    );
}

#[test]
fn a_host_that_is_not_this_machine_keeps_its_name_in_the_plain_colour() {
    let mut n = host("01JA", "far");
    n["kind"] = json!("peer");
    n["rtt_ms"] = json!(12.0);
    let herd = herd(json!([n]), json!([term("01JA/w1:p1")]));
    let rows = sidebar::rows(&herd, Order::Grouped, None, &Hosts::default(), NOW, |_| None);
    let screen = painted(&rows, 30);
    let y = screen.lines().position(|l| l.contains("far")).unwrap();
    let line = screen.lines().nth(y).unwrap();
    assert!(!line.contains("12ms"), "{line}");
    let x = line.find("far").map(|b| line[..b].chars().count()).unwrap() as u16;
    assert_ne!(
        buffer(&rows, 30)[(x, y as u16)].fg,
        kampr_tui::theme::PHOSPHOR.accent
    );
    assert_ne!(buffer(&rows, 30)[(x, y as u16)].fg, Color::Reset);
}

#[test]
fn a_container_row_carries_its_restarts_health_and_a_plus_while_it_runs() {
    let mut web = container("web", "running");
    web["restarts"] = json!(4);
    web["health"] = json!("unhealthy");
    let mut n = host("01JA", "front");
    n["containers"] = json!([web, container("idle", "exited")]);
    let herd = herd(json!([n]), json!([]));
    let mut hosts = showing(&[("01JA", Area::Containers)]);
    hosts.flip_stopped("01JA");
    let rows = sidebar::rows(&herd, Order::Grouped, None, &hosts, NOW, |_| None);
    let row = rows
        .iter()
        .find(|r| matches!(r, Row::Container { .. }))
        .expect("a container row");
    assert!(row.selectable());
    assert!(
        rows.iter()
            .any(|r| matches!(r, Row::Stopped { .. }) && r.selectable())
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r, Row::Node { .. }) && r.selectable()),
        "a host row is a stop for the navigator: its switch and menu are keys on it"
    );

    let screen = painted(&rows, 30);
    let line = screen.lines().find(|l| l.contains("web")).unwrap_or_default();
    assert!(line.contains("↻4") && line.contains("unhealthy"), "{screen}");
    assert!(line.trim_end().ends_with('+'), "{screen}");
    let idle = screen.lines().find(|l| l.contains("idle")).unwrap_or_default();
    assert!(!idle.contains('+'), "{screen}");
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
    let open = showing(&[("01JA", Area::Containers)]);
    assert_eq!(
        drawn(&herd, &open),
        [
            "host front",
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
