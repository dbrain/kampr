use kampr_core::naming::{DEFAULT_TEMPLATE, Fields, Template, TemplateError};
use kampr_core::provider::AgentStatus;
use kampr_core::wire::PaneEntry;
use serde_json::{Value, json};

const FIXTURE: &str = "tests/fixtures/naming-cases.json";

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
        let entry = entry(&case["fields"]);
        assert_eq!(
            template.render(&Fields::from_entry(&entry)),
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
        "{label|workspace|cwd|pane}[ ({argv|cmd})] · {agent|'bash'}"
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

#[test]
fn a_malformed_template_says_which_way_it_is_malformed() {
    assert_eq!(Template::parse("{workspace"), Err(TemplateError::UnclosedSlot));
    assert_eq!(Template::parse("[{workspace}"), Err(TemplateError::UnclosedGroup));
    assert_eq!(Template::parse("{workspace}]"), Err(TemplateError::UnopenedGroup));
    assert_eq!(Template::parse("{}"), Err(TemplateError::EmptyChoice));
    assert_eq!(Template::parse("{'oops}"), Err(TemplateError::UnclosedLiteral));
}
