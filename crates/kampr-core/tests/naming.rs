use kampr_core::naming::{DEFAULT_TEMPLATE, Fields, Template, TemplateError};
use kampr_core::provider::AgentStatus;
use kampr_core::wire::PaneEntry;
use serde_json::{Value, json};

const FIXTURE: &str = "tests/fixtures/naming-cases.json";

/// `title` is the session's own — the harness writes it, the wire does not carry it yet — so it
/// is overlaid onto the fields rather than read off a `PaneEntry`.
fn titled(fields: &Value) -> (PaneEntry, Option<String>) {
    (
        entry(fields),
        fields.get("title").and_then(Value::as_str).map(str::to_string),
    )
}

fn fields<'a>(entry: &'a PaneEntry, title: Option<&'a String>) -> Fields<'a> {
    Fields {
        title: title.map(String::as_str),
        ..Fields::from_entry(entry)
    }
}

fn entry(fields: &Value) -> PaneEntry {
    let s = |key: &str| fields.get(key).and_then(Value::as_str).map(str::to_string);
    let mut entry: PaneEntry = serde_json::from_value(json!({
        "id": s("pane").expect("every case names its pane"),
        "node_id": "01JNODE",
        "rows": 30,
    }))
    .expect("a pane entry with only its required fields");
    entry.workspace = s("workspace");
    entry.tab = s("tab");
    entry.cwd = s("cwd");
    entry.label = s("label");
    entry.agent = s("agent");
    entry.cmd = s("cmd");
    entry.argv = s("argv");
    entry.agent_status = match fields.get("status").and_then(Value::as_str) {
        Some("idle") => AgentStatus::Idle,
        Some("working") => AgentStatus::Working,
        Some("blocked") => AgentStatus::Blocked,
        Some("done") => AgentStatus::Done,
        _ => AgentStatus::Unknown,
    };
    entry
}

/// The Kotlin half of this is `NamingParityTest`, over the same file. Neither side owns it.
#[test]
fn every_shipped_case_renders_the_string_the_fixture_pins() {
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("the fixture is readable"))
            .expect("the fixture is JSON");
    let cases = cases.as_object().expect("the fixture is an object of cases");
    let mut ran = 0;
    for (name, case) in cases {
        if name == "_" {
            continue;
        }
        let template = Template::parse(case["template"].as_str().expect("a template")).expect(name);
        let (entry, title) = titled(&case["fields"]);
        assert_eq!(
            template.render(&fields(&entry, title.as_ref())),
            case["expect"].as_str().expect("an expectation"),
            "{name}"
        );
        ran += 1;
    }
    assert!(
        ran > 10,
        "the fixture is meant to carry the cases, and it carried {ran}"
    );
}

#[test]
fn a_command_ble_sh_hid_degrades_to_the_workspace_rather_than_to_empty_parens() {
    let template = Template::default();
    let mut entry = entry(&json!({ "pane": "w3:p2", "workspace": "kampr", "cwd": "/home/dbrain/dev/kampr" }));
    entry.cmd = Some("cargo".into());
    entry.argv = Some("cargo test".into());
    assert_eq!(
        template.render(&Fields::from_entry(&entry)),
        "kampr (cargo test) · bash"
    );
    entry.cmd = None;
    entry.argv = None;
    assert_eq!(template.render(&Fields::from_entry(&entry)), "kampr · bash");
}

#[test]
fn a_template_that_resolves_to_nothing_still_names_the_pane() {
    let template = Template::parse("[{cmd}]").expect("parses");
    let entry = entry(&json!({ "pane": "01JNODE/w3:p2" }));
    assert_eq!(template.render(&Fields::from_entry(&entry)), "w3:p2");
}

#[test]
fn the_default_template_is_the_one_the_fixture_and_the_clients_use() {
    // Spelled out on both sides — `NamingParityTest` asserts the same literal — so a change to it
    // here cannot land without the Kotlin one moving with it.
    assert_eq!(
        DEFAULT_TEMPLATE,
        "{label|title|workspace|cwd|pane}[ ({argv|cmd})] · {agent|'bash'}"
    );
    assert_eq!(Template::default(), Template::parse(DEFAULT_TEMPLATE).unwrap());
}

/// `{last_cmd}` and `{branch}` have no source (11-cli-briefs W9), so a config that asks for one is
/// a typo to say out loud rather than a section that silently renders nothing for ever.
#[test]
fn a_token_with_no_source_behind_it_is_refused_by_name() {
    assert_eq!(
        Template::parse("{workspace} {last_cmd}"),
        Err(TemplateError::UnknownToken("last_cmd".into()))
    );
    assert_eq!(
        Template::parse("{branch}"),
        Err(TemplateError::UnknownToken("branch".into()))
    );
}

/// The operator's rule: automatic only where nothing manual exists. `label` is what they typed on
/// the pane and `title` is what the harness called the conversation, so a template that puts the
/// generated one first is the defect this names.
#[test]
fn a_name_the_operator_set_by_hand_beats_a_title_the_session_generated() {
    let template = Template::default();
    let mut entry = entry(&json!({ "pane": "w3:p2", "workspace": "kampr", "agent": "claude" }));
    let title = "the width inference rewrite".to_string();

    assert_eq!(
        template.render(&fields(&entry, Some(&title))),
        "the width inference rewrite · claude",
        "a title stands in for a workspace on a pane herdr was never asked to label"
    );
    entry.label = Some("build".into());
    assert_eq!(template.render(&fields(&entry, Some(&title))), "build · claude");
    assert_eq!(
        template.render(&fields(&entry, None)),
        "build · claude",
        "and a session with no title of its own changes nothing"
    );
}

#[test]
fn a_malformed_template_says_which_way_it_is_malformed() {
    assert_eq!(Template::parse("{workspace"), Err(TemplateError::UnclosedSlot));
    assert_eq!(Template::parse("[{workspace}"), Err(TemplateError::UnclosedGroup));
    assert_eq!(Template::parse("{workspace}]"), Err(TemplateError::UnopenedGroup));
    assert_eq!(Template::parse("{}"), Err(TemplateError::EmptyChoice));
    assert_eq!(Template::parse("{'oops}"), Err(TemplateError::UnclosedLiteral));
}

/// **The token was in the template and the field was hard-wired to `None`**, so every pane in a
/// repository went on being named after the same working directory. Herdr's own label still wins:
/// automatic naming is for panes nobody has named.
#[test]
fn a_pane_the_harness_named_is_called_that_rather_than_after_its_directory() {
    let mut entry = entry(&json!({ "pane": "01JNODE/w1:p1", "agent": "claude" }));
    entry.cwd = Some("/home/u/dev/kampr".into());
    entry.title = Some("width-inference".into());

    assert_eq!(
        Template::parse(DEFAULT_TEMPLATE)
            .unwrap()
            .render(&Fields::from_entry(&entry)),
        "width-inference · claude"
    );

    entry.label = Some("the one I named".into());
    assert_eq!(
        Template::parse(DEFAULT_TEMPLATE)
            .unwrap()
            .render(&Fields::from_entry(&entry)),
        "the one I named · claude",
        "a generated name displaced the one the operator typed"
    );
}
