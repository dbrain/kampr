use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use kampr_core::program::{BlockKind, Event, MAX_RECORDS, ProgramRecords, ProgramState, WIRE_ROWS, parse};
use kampr_core::provider::{AgentStatus, NativePane, PaneInfo};
use kampr_core::wire::PaneEntry;

fn b64(text: &str) -> String {
    STANDARD.encode(text)
}

fn run(bodies: &[&str]) -> ProgramRecords {
    let mut records = ProgramRecords::default();
    for body in bodies {
        if let Some(event) = parse(body.as_bytes()) {
            records.apply(event);
        }
    }
    records
}

fn show(records: &ProgramRecords) -> Vec<String> {
    let mut rows: Vec<String> = records
        .saved()
        .iter()
        .map(|r| {
            let mut row = format!("{} {}", if r.id.is_empty() { "/" } else { &r.id }, r.state.word());
            let mut part = |name: &str, value: Option<String>| {
                if let Some(value) = value {
                    row.push_str(&format!(" {name}={value}"));
                }
            };
            part("kind", r.kind.map(|k| k.word().to_string()));
            part("progress", r.progress.map(|p| p.to_string()));
            part("app", records.app_of(&r.id).map(str::to_string));
            part("title", r.title.clone());
            part("msg", r.msg.clone());
            row
        })
        .collect();
    rows.sort();
    rows
}

fn msg(text: &str) -> String {
    format!("state=working:msg={}", b64(text))
}

#[test]
fn the_spec_examples_leave_the_records_the_spec_describes() {
    let brew =
        |state: &str, extra: &str, text: &str| format!("state={state}:{extra}app=brew:msg={}", b64(text));
    let one_program = [
        brew("working", "", "Installing updates"),
        brew("blocked", "kind=auth:", "Password required to install updates"),
        brew("working", "", "Installing updates"),
        brew("done", "", "Upgraded 12 packages"),
    ];
    let mut records = ProgramRecords::default();
    let mut seen = Vec::new();
    for body in &one_program {
        records.apply(parse(body.as_bytes()).expect("a report"));
        seen.push(show(&records));
    }
    assert_eq!(
        seen,
        vec![
            vec!["/ working app=brew msg=Installing updates"],
            vec!["/ blocked kind=auth app=brew msg=Password required to install updates"],
            vec!["/ working app=brew msg=Installing updates"],
            vec!["/ done app=brew msg=Upgraded 12 packages"],
        ]
    );

    let several = [
        format!("state=working:app=deploy:msg={}", b64("Deploying v2.4.1")),
        format!(
            "state=working:id=us-east:title={}:progress=40:msg={}",
            b64("US East"),
            b64("Pushing image")
        ),
        format!(
            "state=blocked:kind=permission:id=eu-west:title={}:msg={}",
            b64("EU West"),
            b64("Approve deploy")
        ),
        format!(
            "state=done:id=us-east:title={}:msg={}",
            b64("US East"),
            b64("Healthy")
        ),
    ];
    let bodies: Vec<&str> = several.iter().map(String::as_str).collect();
    assert_eq!(
        show(&run(&bodies)),
        [
            "/ working app=deploy msg=Deploying v2.4.1",
            "eu-west blocked kind=permission app=deploy title=EU West msg=Approve deploy",
            "us-east done app=deploy title=US East msg=Healthy",
        ]
    );

    let mut after = several.to_vec();
    after.push("state=clear:id=us-east".into());
    after.push("state=clear:id=eu-west".into());
    after.push(format!(
        "state=done:app=deploy:msg={}",
        b64("Deployed to 3 regions")
    ));
    let bodies: Vec<&str> = after.iter().map(String::as_str).collect();
    assert_eq!(
        show(&run(&bodies)),
        ["/ done app=deploy msg=Deployed to 3 regions"]
    );
}

#[test]
fn clear_takes_a_record_and_what_is_below_it_and_nothing_beside() {
    let set = |id: &str| format!("state=done:id={id}");
    let ids = ["a", "a/b", "a/b/c", "ab", "a.b", "x/a"];
    let bodies: Vec<String> = ids.iter().map(|id| set(id)).collect();
    let mut all: Vec<&str> = bodies.iter().map(String::as_str).collect();
    all.push("state=clear:id=a");
    let left: Vec<String> = run(&all).saved().into_iter().map(|r| r.id).collect();
    let mut left = left;
    left.sort();
    assert_eq!(left, ["a.b", "ab", "x/a"]);

    let mut every: Vec<&str> = bodies.iter().map(String::as_str).collect();
    every.extend(["state=done", "state=clear"]);
    assert!(run(&every).is_empty());
}

#[test]
fn a_report_replaces_its_record_whole_and_app_comes_from_the_nearest_ancestor() {
    let records = run(&[
        "state=working:app=ci:title=VGl0bGU=:msg=aGk=",
        "state=working:id=job",
        "state=working:id=job/step:app=lint",
        "state=working:id=job/step/deep",
        "state=working:id=other/deep",
    ]);
    assert_eq!(records.app_of("job"), Some("ci"));
    assert_eq!(records.app_of("job/step/deep"), Some("lint"));
    assert_eq!(records.app_of("other/deep"), Some("ci"));
    let again = run(&["state=working:app=ci:title=VGl0bGU=:msg=aGk=", "state=idle"]);
    assert_eq!(show(&again), ["/ idle"]);
}

#[test]
fn a_report_that_breaks_a_rule_does_nothing() {
    let over = |n: usize| "A".repeat(n);
    let id_of = |segments: usize| vec!["s"; segments].join("/");
    let cases: Vec<(&str, String)> = vec![
        ("a control character in msg", msg("one\ntwo")),
        ("a tab in title", format!("state=idle:title={}", b64("a\tb"))),
        ("a C1 control", msg("a\u{85}b")),
        ("a DEL", msg("a\u{7f}b")),
        ("bad base64", "state=working:msg=!!!!".into()),
        ("padding that is the wrong amount", "state=working:msg=YQ=".into()),
        (
            "bytes that are not UTF-8",
            format!("state=working:msg={}", STANDARD.encode([0xff, 0xfe])),
        ),
        ("msg decoded over 2048", msg(&"x".repeat(2049))),
        (
            "msg encoded over 2732",
            format!("state=working:msg={}", over(2733)),
        ),
        (
            "title decoded over 192",
            format!("state=idle:title={}", b64(&"x".repeat(193))),
        ),
        (
            "title encoded over 256",
            format!("state=idle:title={}", over(257)),
        ),
        ("app over 32", format!("state=idle:app={}", "a".repeat(33))),
        ("app outside the grammar", "state=idle:app=a/b".into()),
        (
            "id over 128",
            format!("state=idle:id={}", vec!["a".repeat(32); 4].join("/")),
        ),
        ("a segment over 32", format!("state=idle:id={}", "a".repeat(33))),
        ("nine levels", format!("state=idle:id={}", id_of(9))),
        ("an empty segment", "state=idle:id=a//b".into()),
        ("a leading slash", "state=idle:id=/a".into()),
        ("a trailing slash", "state=idle:id=a/".into()),
        ("an id with a space", "state=idle:id=a b".into()),
        ("an empty id", "state=idle:id=".into()),
        ("a state nobody defined", "state=compacting:app=x".into()),
        ("no state", "app=x".into()),
        (
            "a body over the sequence limit",
            format!("state=idle:foo={}", over(4090)),
        ),
    ];
    for (why, body) in cases {
        assert_eq!(parse(body.as_bytes()), None, "{why}");
    }
    assert_eq!(parse(&[b's', b't', 0xff]), None);
    assert_eq!(parse(b"?"), None);
}

#[test]
fn what_the_limits_allow_is_read() {
    let eight = ["s"; 8].join("/");
    let widest = ["a".repeat(32), "b".repeat(32), "c".repeat(32), "d".repeat(29)].join("/");
    assert_eq!(widest.len(), 128);
    for (why, body) in [
        ("msg of exactly 2048", msg(&"x".repeat(2048))),
        (
            "title of exactly 192",
            format!("state=idle:title={}", b64(&"x".repeat(192))),
        ),
        ("eight levels", format!("state=idle:id={eight}")),
        ("an id of 128 bytes", format!("state=idle:id={widest}")),
        ("an app of 32", format!("state=idle:app={}", "a".repeat(32))),
        ("unpadded base64", "state=working:msg=YQ".into()),
        ("padded base64", "state=working:msg=YQ==".into()),
    ] {
        assert!(matches!(parse(body.as_bytes()), Some(Event::Set(_))), "{why}");
    }
}

#[test]
fn a_bad_pair_is_skipped_and_the_rest_of_the_report_is_read() {
    let body = format!(
        "state=working:novalue:=x:Key=1:toolongakeyname1234=1:odd=a b:app=one:app=two:progress=250:kind=auth:msg={}",
        b64("hi")
    );
    let Some(Event::Set(record)) = parse(body.as_bytes()) else {
        panic!("a report")
    };
    assert_eq!(record.app.as_deref(), Some("two"));
    assert_eq!(record.msg.as_deref(), Some("hi"));
    assert_eq!(record.progress, None);
    assert_eq!(record.kind, None);
    assert_eq!(record.id, "");
}

#[test]
fn kind_and_progress_belong_to_the_states_that_have_them() {
    let read = |body: &str| match parse(body.as_bytes()) {
        Some(Event::Set(r)) => (r.kind, r.progress),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        read("state=blocked:kind=permission:progress=7"),
        (Some(BlockKind::Permission), Some(7))
    );
    assert_eq!(
        read("state=blocked:kind=question"),
        (Some(BlockKind::Question), None)
    );
    assert_eq!(
        read("state=working:kind=permission:progress=100"),
        (None, Some(100))
    );
    assert_eq!(read("state=working:progress=0"), (None, Some(0)));
    assert_eq!(read("state=blocked:kind=nonsense"), (None, None));
    assert_eq!(read("state=done:kind=auth:progress=50"), (None, None));
    assert_eq!(read("state=working:progress=101"), (None, None));
    assert_eq!(read("state=working:progress=-3"), (None, None));
    assert_eq!(read("state=working:progress=4x"), (None, None));
}

#[test]
fn the_oldest_record_makes_way_at_the_cap() {
    let mut records = ProgramRecords::default();
    for n in 0..MAX_RECORDS {
        records.apply(parse(format!("state=done:id=r{n}").as_bytes()).unwrap());
    }
    records.apply(parse(b"state=done:id=r0").unwrap());
    records.apply(parse(b"state=done:id=extra").unwrap());
    assert_eq!(records.len(), MAX_RECORDS);
    assert!(records.get("r0").is_some(), "a rewritten record is the newest");
    assert!(records.get("r1").is_none(), "r1 was the oldest");
    assert!(records.get("extra").is_some());
}

#[test]
fn the_session_is_what_its_records_fold_to() {
    use ProgramState::*;
    type Case<'a> = (&'a str, &'a [&'a str], Option<(ProgramState, Option<&'a str>)>);
    let table: &[Case] = &[
        ("nothing", &[], None),
        ("a root alone", &["state=working"], Some((Working, None))),
        (
            "a worker blocked beats a working root",
            &["state=working", "state=blocked:id=w:kind=permission:msg=aGk="],
            Some((Blocked, Some("hi"))),
        ),
        (
            "a worker working beats an idle root",
            &["state=idle", "state=working:id=w"],
            Some((Working, None)),
        ),
        (
            "a worker finishing is not the session finishing",
            &["state=working", "state=done:id=w"],
            Some((Working, None)),
        ),
        (
            "done workers under an idle root leave idle",
            &["state=idle", "state=done:id=w", "state=done:id=v"],
            Some((Idle, None)),
        ),
        (
            "workers alone with nothing active say nothing",
            &["state=done:id=w"],
            None,
        ),
        (
            "an errored worker shows when nothing is working",
            &["state=idle", "state=error:id=w:msg=Ym9vbQ=="],
            Some((Error, Some("boom"))),
        ),
        (
            "the root's error",
            &["state=error:msg=Ym9vbQ=="],
            Some((Error, Some("boom"))),
        ),
        (
            "blocked and error: blocked",
            &["state=error:id=a", "state=blocked:id=b"],
            Some((Blocked, None)),
        ),
        (
            "the root speaks for its state",
            &["state=blocked:msg=cm9vdA==", "state=blocked:id=w:msg=d29ya2Vy"],
            Some((Blocked, Some("root"))),
        ),
        (
            "else the newest speaks",
            &["state=blocked:id=a:msg=b2xk", "state=blocked:id=b:msg=bmV3"],
            Some((Blocked, Some("new"))),
        ),
    ];
    for (why, bodies, want) in table {
        let got = run(bodies).fold().map(|s| (s.state, s.message));
        let want = want.map(|(state, message)| (state, message.map(str::to_string)));
        assert_eq!(got, want, "{why}");
    }
}

#[test]
fn running_records_end_with_their_process_and_finished_ones_stay() {
    let bodies = [
        "state=working",
        "state=blocked:id=a",
        "state=done:id=b",
        "state=error:id=c",
        "state=idle:id=d",
    ];
    let mut records = run(&bodies);
    assert!(records.apply(Event::Prompt));
    assert_eq!(show(&records), ["b done", "c error", "d idle"]);
    assert!(!records.apply(Event::Prompt), "nothing left to end");

    let mut ended = run(&bodies);
    assert!(ended.process_ended());
    assert_eq!(ended, records);

    let mut reset = run(&bodies);
    assert!(reset.apply(Event::Reset));
    assert!(reset.is_empty());
}

#[test]
fn the_wire_rows_are_urgent_first_capped_and_absent_for_a_lone_root() {
    assert!(run(&["state=working"]).rows().is_empty());
    assert!(ProgramRecords::default().rows().is_empty());
    let bodies: Vec<String> = std::iter::once("state=idle".to_string())
        .chain((0..40).map(|n| format!("state=done:id=d{n}")))
        .chain(["state=blocked:id=late".to_string()])
        .collect();
    let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
    let rows = run(&refs)
        .rows()
        .into_iter()
        .map(|r| r.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), WIRE_ROWS);
    assert_eq!(rows[0], "late");
    assert_eq!(rows[1], "d39");
}

#[test]
fn a_saved_set_comes_back_the_same_and_a_forged_one_does_not() {
    let records = run(&[
        "state=working:app=ci",
        "state=blocked:id=a/b:kind=auth:progress=3",
        "state=done:id=c",
    ]);
    let back = ProgramRecords::restore(records.saved()).expect("a set");
    assert_eq!(back.saved(), records.saved());
    assert_eq!(back.rows(), records.rows());
    let mut twice = records.saved();
    twice.push(twice[0].clone());
    assert_eq!(ProgramRecords::restore(twice), None);
}

fn entry(bodies: &[&str]) -> serde_json::Value {
    let pane = PaneInfo {
        pane_id: "n:01k".into(),
        agent_status: AgentStatus::Idle,
        native: Some(NativePane {
            session_id: "01k".into(),
            program: Some(run(bodies)),
            ..NativePane::default()
        }),
        ..PaneInfo::default()
    };
    serde_json::to_value(PaneEntry::new("01J", &pane, false)).unwrap()
}

#[test]
fn an_error_is_said_beside_an_idle_status_and_the_records_only_when_there_is_more_than_a_root() {
    let failed = entry(&["state=error:app=ci:msg=Ym9vbQ=="]);
    assert_eq!(failed["agent_status"], "idle");
    assert_eq!(failed["program_error"], true);
    assert_eq!(failed["program_msg"], "boom");
    assert!(failed.get("program").is_none(), "{failed}");

    let busy = entry(&[
        "state=working:app=ci",
        "state=error:id=lint/slow:progress=3:msg=Ym9vbQ==",
        "state=working:id=test:progress=40",
    ]);
    assert!(
        busy.get("program_error").is_none(),
        "an error under a working root is not the session's: {busy}"
    );
    let rows = busy["program"].as_array().expect("rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        serde_json::json!({ "id": "test", "state": "working", "progress": 40, "app": "ci" })
    );
    assert_eq!(rows[2]["id"], "lint/slow");
    assert_eq!(rows[2]["app"], "ci");

    let plain = entry(&["state=working"]);
    assert!(
        plain.get("program_error").is_none() && plain.get("program").is_none(),
        "{plain}"
    );
}

#[test]
fn a_working_fold_carries_its_progress_the_roots_included() {
    assert_eq!(entry(&["state=working:progress=40"])["program_progress"], 40);
    assert_eq!(
        entry(&["state=idle", "state=working:id=test:progress=7"])["program_progress"],
        7
    );
    for bodies in [
        &["state=working"][..],
        &[
            "state=working:progress=40",
            "state=blocked:id=a:kind=auth:progress=3",
        ],
        &["state=done:progress=100"],
    ] {
        let pane = entry(bodies);
        assert!(pane.get("program_progress").is_none(), "{bodies:?}: {pane}");
    }
}
