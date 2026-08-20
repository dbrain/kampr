use kampr_core::provider::{AgentStatus, PaneInfo};
use kampr_core::wire::{
    Caps, ClientMsg, ErrorCode, Hello, NodeEntry, PROTOCOL, PaneEntry, PendingOption, PendingSource, Role,
    ServerMsg,
};
use kampr_journal::{Block, Page, Role as Role_, ToolState, Turn};

#[test]
fn hello_matches_the_documented_shape() {
    let v = serde_json::to_value(ServerMsg::Hello(Hello {
        protocol: PROTOCOL,
        node_id: "01J".into(),
        node_name: "comingclean".into(),
        build: "0.1.0+abc1234".into(),
        role: Role::Readonly,
        caps: Caps {
            push: true,
            scrollback: true,
            conversation: false,
        },
    }))
    .unwrap();
    assert_eq!(v["t"], "hello");
    assert_eq!(v["protocol"], 1);
    assert_eq!(v["role"], "readonly");
    assert_eq!(v["caps"]["scrollback"], true);
}

#[test]
fn a_pane_entry_carries_the_node_qualified_id() {
    let info = PaneInfo {
        pane_id: "w3:p2".into(),
        workspace: Some("kampr".into()),
        tab: Some("1".into()),
        cwd: Some("/home/dbrain/dev/kampr".into()),
        agent: Some("claude".into()),
        agent_status: AgentStatus::Blocked,
        cols: 74,
        rows: 30,
        scrollback_rows: 0,
        ..PaneInfo::default()
    };
    let v = serde_json::to_value(ServerMsg::Herd {
        nodes: vec![NodeEntry {
            id: "01J".into(),
            name: "comingclean".into(),
            kind: "local".into(),
            online: true,
            rtt_ms: Some(0.4),
            herdr_version: Some("0.8.2".into()),
            build: None,
            detail: None,
        }],
        panes: vec![PaneEntry::new("01J", &info, true)],
    })
    .unwrap();
    assert_eq!(v["t"], "herd");
    assert_eq!(v["panes"][0]["id"], "01J/w3:p2");
    assert_eq!(v["panes"][0]["node_id"], "01J");
    assert_eq!(v["panes"][0]["agent_status"], "blocked");
    assert_eq!(v["panes"][0]["cols"], 74);
    assert_eq!(v["panes"][0]["has_conversation"], true);
}

#[test]
fn error_codes_use_the_documented_spelling() {
    let v = serde_json::to_value(ServerMsg::Error {
        code: ErrorCode::NotWriter,
        message: "this device is read-only".into(),
        pane: None,
    })
    .unwrap();
    assert_eq!(v["code"], "not_writer");
    assert!(v["pane"].is_null());
}

#[test]
fn client_messages_parse_and_unknown_fields_are_ignored() {
    let watch: ClientMsg =
        serde_json::from_str(r#"{"t":"watch","pane":"01J/w3:p2","scrollback":true,"future":9}"#).unwrap();
    assert!(matches!(
        watch,
        ClientMsg::Watch {
            scrollback: true,
            conversation: false,
            ..
        }
    ));

    let keys: ClientMsg = serde_json::from_str(r#"{"t":"input","pane":"p","keys":["ctrl+c"]}"#).unwrap();
    match keys {
        ClientMsg::Input {
            keys: Some(k),
            text: None,
            b64: None,
            ..
        } => assert_eq!(k, ["ctrl+c"]),
        other => panic!("{other:?}"),
    }

    let ping: ClientMsg = serde_json::from_str(r#"{"t":"ping","n":7}"#).unwrap();
    assert!(matches!(ping, ClientMsg::Ping { n: 7 }));
    assert_eq!(
        serde_json::to_value(ServerMsg::Pong { n: 7 }).unwrap()["t"],
        "pong"
    );

    assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"resync"}"#).is_ok());
    assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"from_the_future"}"#).is_err());
}

/// The client's Kotlin decoder reads `convo` as `pane` / `cursor` / `more` / `turns`, and each
/// block by its `b` tag — so these field names are the contract, not an implementation detail.
#[test]
fn convo_matches_the_shape_the_client_decodes() {
    let page = Page {
        turns: vec![Turn {
            id: "t_812".into(),
            role: Role_::Assistant,
            at: Some("2026-08-20T13:41:55Z".into()),
            blocks: vec![
                Block::Md {
                    text: "Six, and they are…".into(),
                },
                Block::Tool {
                    name: "Bash".into(),
                    summary: Some("probe key grammar".into()),
                    lines: Some(48),
                    state: ToolState::Done,
                },
                Block::Code {
                    lang: Some("ts".into()),
                    text: "send(pane)".into(),
                },
                Block::Diff {
                    path: Some("/tmp/x".into()),
                    text: "@@ -1 +1 @@\n-a\n+b\n".into(),
                },
            ],
        }],
        cursor: Some("t_812".into()),
        more: true,
    };
    let v = serde_json::to_value(ServerMsg::convo("01J/w3:p2", page)).unwrap();
    assert_eq!(v["t"], "convo");
    assert_eq!(v["pane"], "01J/w3:p2");
    assert_eq!(v["cursor"], "t_812");
    assert_eq!(v["more"], true);

    let turn = &v["turns"][0];
    assert_eq!(turn["id"], "t_812");
    assert_eq!(turn["role"], "assistant");
    assert_eq!(turn["at"], "2026-08-20T13:41:55Z");

    let blocks = turn["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["b"], "md");
    assert_eq!(blocks[0]["text"], "Six, and they are…");
    assert_eq!(blocks[1]["b"], "tool");
    assert_eq!(blocks[1]["name"], "Bash");
    assert_eq!(blocks[1]["summary"], "probe key grammar");
    assert_eq!(blocks[1]["lines"], 48);
    assert_eq!(blocks[1]["state"], "done");
    assert_eq!(blocks[2]["b"], "code");
    assert_eq!(blocks[2]["lang"], "ts");
    assert_eq!(blocks[3]["b"], "diff");
    assert_eq!(blocks[3]["path"], "/tmp/x");
    assert!(blocks[3]["text"].as_str().unwrap().starts_with("@@"));
}

#[test]
fn a_revised_turn_travels_as_convo_turn() {
    let v = serde_json::to_value(ServerMsg::ConvoTurn {
        pane: "01J/w3:p2".into(),
        turns: vec![Turn {
            id: "t_812".into(),
            role: Role_::Assistant,
            at: None,
            blocks: vec![Block::Tool {
                name: "Bash".into(),
                summary: None,
                lines: None,
                state: ToolState::Running,
            }],
        }],
    })
    .unwrap();
    assert_eq!(v["t"], "convo.turn");
    assert_eq!(v["pane"], "01J/w3:p2");
    assert_eq!(v["turns"][0]["id"], "t_812");
    assert_eq!(v["turns"][0]["blocks"][0]["state"], "running");
    assert!(
        v["turns"][0].get("at").is_none(),
        "an absent timestamp is omitted, not null"
    );
    assert!(v["turns"][0]["blocks"][0].get("summary").is_none());
}

/// A prompt is cleared by the same message with `question: null`, so the key must be present and
/// null rather than omitted.
#[test]
fn pending_carries_its_source_and_clears_with_a_null_question() {
    let asked = serde_json::to_value(ServerMsg::Pending {
        pane: "01J/w3:p2".into(),
        question: Some("Do you want to make this edit?".into()),
        options: vec![PendingOption {
            key: "1".into(),
            label: "Yes".into(),
        }],
        source: PendingSource::Screen,
    })
    .unwrap();
    assert_eq!(asked["t"], "pending");
    assert_eq!(asked["source"], "screen");
    assert_eq!(asked["options"][0]["key"], "1");
    assert_eq!(asked["options"][0]["label"], "Yes");

    let cleared = serde_json::to_value(ServerMsg::Pending {
        pane: "01J/w3:p2".into(),
        question: None,
        options: vec![],
        source: PendingSource::Transcript,
    })
    .unwrap();
    assert!(cleared["question"].is_null());
    assert!(cleared.as_object().unwrap().contains_key("question"));
    assert_eq!(cleared["source"], "transcript");
}
