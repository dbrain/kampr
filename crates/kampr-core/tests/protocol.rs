use kampr_core::provider::{AgentStatus, PaneInfo};
use kampr_core::wire::{
    Caps, ClientMsg, ErrorCode, Hello, NodeEntry, PROTOCOL, PaneEntry, PendingOption, PendingSource, Role,
    ServerMsg,
};
use kampr_journal::{Attachment, Block, Page, Role as Role_, ToolState, Turn};

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

/// The one thing `role` must never become is a second greeting: a client that re-ran everything
/// `hello` means would throw away its herd and its preferences over a permission change.
#[test]
fn a_role_change_is_its_own_frame_and_carries_only_the_role() {
    let v = serde_json::to_value(ServerMsg::RoleChanged { role: Role::Readonly }).unwrap();
    assert_eq!(v["t"], "role");
    assert_eq!(v["role"], "readonly");
    assert_eq!(
        v.as_object().unwrap().len(),
        2,
        "a role frame that grew fields is one a client has to inspect: {v}"
    );
    assert_eq!(
        serde_json::to_value(ServerMsg::RoleChanged { role: Role::Full }).unwrap()["role"],
        "full"
    );
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
        cols: Some(74),
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
            update: None,
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
                    att: None,
                },
                Block::Md {
                    text: "[image · png]".into(),
                    att: Some(Attachment {
                        id: "opaque".into(),
                        kind: "image".into(),
                        mime: Some("image/png".into()),
                        bytes: Some(52831),
                        name: Some("shot.png".into()),
                    }),
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
    let v = serde_json::to_value(ServerMsg::convo("01J/w3:p2", page.clone(), false)).unwrap();
    assert_eq!(v["t"], "convo");
    assert_eq!(v["pane"], "01J/w3:p2");
    assert_eq!(v["cursor"], "t_812");
    assert_eq!(v["more"], true);
    // Additive: a page that merges is byte for byte the page every build before `fresh` sent, so
    // an installed phone sees nothing new. Only a page that *replaces* carries the field.
    assert!(v.get("fresh").is_none(), "{v}");
    let replacing = serde_json::to_value(ServerMsg::convo("01J/w3:p2", page, true)).unwrap();
    assert_eq!(replacing["fresh"], true, "{replacing}");

    let turn = &v["turns"][0];
    assert_eq!(turn["id"], "t_812");
    assert_eq!(turn["role"], "assistant");
    assert_eq!(turn["at"], "2026-08-20T13:41:55Z");

    let blocks = turn["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["b"], "md");
    assert_eq!(blocks[0]["text"], "Six, and they are…");
    assert!(
        blocks[0].get("att").is_none(),
        "an md block with nothing attached carries no `att` at all"
    );
    // The `att` header, which is the whole of what an attachment costs the socket. A client that
    // has never heard of it still has `text`, and that is what an installed phone renders.
    assert_eq!(blocks[1]["b"], "md");
    assert_eq!(blocks[1]["text"], "[image · png]");
    assert_eq!(blocks[1]["att"]["id"], "opaque");
    assert_eq!(blocks[1]["att"]["kind"], "image");
    assert_eq!(blocks[1]["att"]["mime"], "image/png");
    assert_eq!(blocks[1]["att"]["bytes"], 52831);
    assert_eq!(blocks[1]["att"]["name"], "shot.png");
    assert_eq!(blocks[2]["b"], "tool");
    assert_eq!(blocks[2]["name"], "Bash");
    assert_eq!(blocks[2]["summary"], "probe key grammar");
    assert_eq!(blocks[2]["lines"], 48);
    assert_eq!(blocks[2]["state"], "done");
    assert_eq!(blocks[3]["b"], "code");
    assert_eq!(blocks[3]["lang"], "ts");
    assert_eq!(blocks[4]["b"], "diff");
    assert_eq!(blocks[4]["path"], "/tmp/x");
    assert!(blocks[4]["text"].as_str().unwrap().starts_with("@@"));
}

/// A paste has no filename and no dimensions — the media type is all there is — so the fields the
/// source cannot answer are **absent**, not empty strings a client has to special-case.
#[test]
fn an_attachment_omits_what_its_source_never_carried() {
    let v = serde_json::to_value(Block::Md {
        text: "[image · png]".into(),
        att: Some(Attachment {
            id: "opaque".into(),
            kind: "image".into(),
            mime: Some("image/png".into()),
            bytes: Some(70),
            name: None,
        }),
    })
    .unwrap();

    assert_eq!(v["att"]["id"], "opaque");
    assert!(v["att"].get("name").is_none(), "{v}");
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

/// The mesh question is "which of my machines are stale", and it is only answerable if the answer
/// rides beside `build` on every node — including a peer's, which a hub re-publishes verbatim.
#[test]
fn a_node_names_the_release_that_supersedes_it_and_says_nothing_otherwise() {
    let entry = NodeEntry {
        id: "01J".into(),
        name: "front".into(),
        kind: "local".into(),
        online: true,
        rtt_ms: None,
        herdr_version: None,
        build: Some("0.1.0".into()),
        update: Some("0.1.2".into()),
        detail: None,
    };
    let v = serde_json::to_value(ServerMsg::Herd {
        nodes: vec![entry.clone()],
        panes: Vec::new(),
    })
    .unwrap();
    assert_eq!(v["nodes"][0]["build"], "0.1.0");
    assert_eq!(
        v["nodes"][0]["update"], "0.1.2",
        "the field has to carry the version; a bare boolean cannot say what is available"
    );

    let current = serde_json::to_value(NodeEntry {
        update: None,
        ..entry.clone()
    })
    .unwrap();
    assert!(
        current.as_object().unwrap().get("update").is_none(),
        "a current node still shipped an `update` key, so a client cannot tell quiet from stale: {current}"
    );

    // A hub re-publishes a peer's own entry, so the field has to survive a round trip through
    // the same struct the mesh deserialises into.
    let round: NodeEntry = serde_json::from_value(serde_json::to_value(&entry).unwrap()).unwrap();
    assert_eq!(round.update.as_deref(), Some("0.1.2"));
    let old: NodeEntry =
        serde_json::from_str(r#"{"id":"01J","name":"front","kind":"peer","online":true}"#).unwrap();
    assert_eq!(old.update, None);
}
