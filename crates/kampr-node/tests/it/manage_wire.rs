use kampr_node::manage::{ManageOp, Target, parse_target};
use serde_json::Value;

/// The same file `client/shared/src/jvmTest/.../ManageWireTest.kt` asserts the Kotlin client
/// emits. Both sides agreeing with each other and neither agreeing with the wire is how every
/// seam defect in this project started.
fn fixture() -> Value {
    let raw = include_str!("../fixtures/manage-ops.json");
    serde_json::from_str(raw).expect("the manage fixture is JSON")
}

fn op(case: &str) -> ManageOp {
    let value = fixture()[case].clone();
    assert!(!value.is_null(), "no fixture case {case}");
    serde_json::from_value(value).unwrap_or_else(|e| panic!("{case} is not a ManageOp: {e}"))
}

#[test]
fn every_fixture_case_decodes_into_a_manage_op() {
    let all = fixture();
    let cases = all.as_object().expect("the fixture is an object");
    assert!(cases.len() >= 22, "the fixture must cover every op");
    for (case, value) in cases {
        assert_eq!(value["t"], "manage", "{case} is not a manage message");
        let parsed: ManageOp =
            serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{case}: {e}"));
        assert_eq!(parsed.op, value["op"].as_str().unwrap());
    }
}

/// `ratio`, `env`, `args` and `layout` are the four the client's old `Map<String, String?>`
/// could not express at all.
#[test]
fn the_four_non_string_fields_survive_the_wire() {
    assert_eq!(op("pane.split").ratio, Some(0.35));
    assert_eq!(op("workspace.create").env.unwrap()["RUST_LOG"], "debug");
    assert_eq!(
        op("agent.start").args.unwrap(),
        vec!["--model".to_string(), "opus".to_string()]
    );
    assert_eq!(op("layout.apply").layout.unwrap()["root"]["direction"], "right");
}

/// The two book ops carry `entry` and never `at`. `at` is what routing reads, and a book entry
/// names no host — putting one there would send `fleet.drop` down a mesh link looking for the node
/// that owns an id no node owns.
#[test]
fn a_book_op_addresses_an_entry_and_never_a_routed_target() {
    for case in ["fleet.save.entry", "fleet.drop"] {
        let op = op(case);
        assert_eq!(op.entry.as_deref(), Some("01JBOOK"), "{case}");
        assert_eq!(op.at, None, "{case} put a book entry where routing would read it");
        assert_eq!(op.node, None, "{case}");
    }
    assert_eq!(
        op("fleet.save").args.unwrap(),
        vec!["kampr".to_string(), "update".to_string()]
    );
    assert_eq!(op("fleet.save").label.as_deref(), Some("update everything"));
}

#[test]
fn a_cleared_pane_label_is_a_null_and_not_an_absent_key() {
    let cleared = fixture()["rename.clear"].clone();
    assert!(cleared.get("label").is_some_and(Value::is_null));
    assert_eq!(op("rename.clear").label, None);
    assert_eq!(op("rename").label.as_deref(), Some("build"));
}

#[test]
fn each_target_lands_on_the_kind_its_op_requires() {
    let at = |case: &str| op(case).at.expect("case has an `at`");
    let node = "01JNODE";
    assert!(matches!(
        parse_target(node, &at("pane.split")).unwrap(),
        Target::Pane(_)
    ));
    assert!(matches!(
        parse_target(node, &at("agent.start")).unwrap(),
        Target::Pane(_)
    ));
    assert!(matches!(
        parse_target(node, &at("tab.create")).unwrap(),
        Target::Workspace(_)
    ));
    assert!(matches!(
        parse_target(node, &at("layout.export")).unwrap(),
        Target::Tab(_)
    ));
    assert!(matches!(
        parse_target(node, &at("layout.apply")).unwrap(),
        Target::Tab(_)
    ));
    assert!(matches!(
        parse_target(node, &at("close")).unwrap(),
        Target::Tab(_)
    ));
    assert!(matches!(
        parse_target(node, &at("focus")).unwrap(),
        Target::Workspace(_)
    ));
    for case in [
        "pane.size",
        "pane.size.hold",
        "pane.size.match",
        "pane.size.release",
    ] {
        assert!(
            matches!(parse_target(node, &at(case)).unwrap(), Target::Pane(_)),
            "{case} reshapes one pane's PTY and has to be addressed at one",
        );
    }
}

/// The two numbers `pane.size` is entirely about, and the mode that carries neither. They are
/// `u32` rather than strings because a column count that arrives as `"200"` is a column count
/// nothing can clamp.
#[test]
fn a_size_carries_its_two_numbers_and_a_release_carries_none() {
    assert_eq!(
        (op("pane.size").cols, op("pane.size").rows),
        (Some(200), Some(50))
    );
    assert_eq!(op("pane.size").mode, None, "the safe mode is the default one");
    assert_eq!(op("pane.size.hold").mode.as_deref(), Some("hold"));
    // A match carries both numbers, because it is a hold that names a size — the mode is what
    // makes it the *view's* hold rather than the panel's, not a different kind of resize.
    assert_eq!(op("pane.size.match").mode.as_deref(), Some("match"));
    assert_eq!(
        (op("pane.size.match").cols, op("pane.size.match").rows),
        (Some(200), Some(50))
    );
    assert_eq!(
        (op("pane.size.release").cols, op("pane.size.release").rows),
        (None, None),
        "letting go names no size",
    );
}

#[test]
fn the_ops_addressed_at_a_node_carry_one() {
    for case in [
        "workspace.create",
        "worktree.create",
        "worktree.open",
        "session.create",
        "session.stop",
    ] {
        assert_eq!(op(case).node.as_deref(), Some("01JNODE"), "{case}");
        assert!(op(case).at.is_none(), "{case} must not carry an `at`");
    }
}
