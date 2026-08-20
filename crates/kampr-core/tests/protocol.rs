use kampr_core::provider::{AgentStatus, PaneInfo};
use kampr_core::wire::{Caps, ClientMsg, ErrorCode, Hello, NodeEntry, PROTOCOL, PaneEntry, Role, ServerMsg};

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
