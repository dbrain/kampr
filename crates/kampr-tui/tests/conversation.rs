//! W8 — the conversation an agent pane opens on, and the prompt that answers a blocked one.
//!
//! The node here is scripted rather than real: every rule these tests hold down is a rule
//! `04-wire-protocol.md` states, and none of them is a rule herdr knows about.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use futures_util::{SinkExt, StreamExt};
use kampr_client::{Client, Event, Policy, Role, Session, Via};
use kampr_core::Backoff;
use kampr_tui::app::{App, Options};
use kampr_tui::convo::Convo;
use kampr_tui::image::Images;
use kampr_tui::mouse::Click;
use kampr_tui::theme::PHOSPHOR;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const BEAT: Duration = Duration::from_secs(2);
/// Long enough for a frame the client wrote to reach the scripted node over a loopback socket,
/// and the window a "nothing was sent" assertion waits out.
const SETTLE: Duration = Duration::from_millis(250);

// ---- the frames a node sends -----------------------------------------------------------------

fn turn(id: &str, role: &str, at: Option<&str>, blocks: Value) -> Value {
    let mut turn = json!({ "id": id, "role": role, "blocks": blocks });
    if let Some(at) = at {
        turn["at"] = json!(at);
    }
    turn
}

fn md(text: &str) -> Value {
    json!([{ "b": "md", "text": text }])
}

fn page(pane: &str, fresh: bool, cursor: Option<&str>, more: bool, turns: Value) -> Event {
    Event::Convo(
        serde_json::from_value(json!({
            "pane": pane, "fresh": fresh, "cursor": cursor, "more": more, "turns": turns
        }))
        .expect("a convo page"),
    )
}

fn revision(pane: &str, turns: Value) -> Event {
    Event::ConvoTurn {
        pane: pane.to_string(),
        turns: serde_json::from_value(turns).expect("turns"),
    }
}

fn desk(pane: &str, text: Option<&str>) -> Event {
    Event::ConvoComposer {
        pane: pane.to_string(),
        text: text.map(str::to_string),
        clear: Some("\u{3}".into()),
    }
}

fn facets(pane: &str, queued: Value) -> Event {
    Event::ConvoFacets {
        pane: pane.to_string(),
        facets: serde_json::from_value(json!({ "queued": queued })).expect("facets"),
    }
}

fn pending(pane: &str, question: Option<&str>, options: Value) -> Event {
    Event::Pending(
        serde_json::from_value(json!({
            "pane": pane, "question": question, "options": options, "source": "screen"
        }))
        .expect("a pending"),
    )
}

// ---- drawing ---------------------------------------------------------------------------------

const PANE: &str = "01JNODE/w1:p1";

fn drawn(convo: &mut Convo, width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let area = buf.area;
    let mut images = Images::default();
    convo.render(&mut buf, area, PANE, &PHOSPHOR, &mut images, Role::Full);
    buf
}

fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn screen(convo: &mut Convo, width: u16, height: u16) -> String {
    rows(&drawn(convo, width, height)).join("\n")
}

fn row_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on the screen:\n{text}"))
}

fn ink(buf: &Buffer, needle: &str) -> Color {
    let painted = rows(buf);
    let y = painted
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on the screen"));
    let x = painted[y].find(needle).expect("the column") as u16;
    buf[(x, y as u16)].fg
}

// ---- the merge ---------------------------------------------------------------------------

fn three_turns() -> Convo {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        Some("t_a"),
        false,
        json!([
            turn("t_a", "assistant", None, md("alpha")),
            turn("t_b", "assistant", None, md("bravo")),
            turn("t_c", "assistant", None, md("charlie")),
        ]),
    ));
    convo
}

#[test]
fn a_merging_page_puts_a_turn_it_does_not_hold_where_the_page_puts_it() {
    let mut convo = three_turns();
    // The page and the client share `t_b` and `t_c`; `t_x` sits between them and belongs there.
    // Prepending was the older rule and it puts the missing turn above `alpha`, at the top of a
    // view that is scrolled to the bottom, where it is never seen.
    convo.absorb(&page(
        PANE,
        false,
        Some("t_b"),
        true,
        json!([
            turn("t_b", "assistant", None, md("bravo")),
            turn("t_x", "assistant", None, md("xray")),
            turn("t_c", "assistant", None, md("charlie")),
        ]),
    ));

    let screen = screen(&mut convo, 60, 30);
    let (alpha, bravo, xray, charlie) = (
        row_of(&screen, "alpha"),
        row_of(&screen, "bravo"),
        row_of(&screen, "xray"),
        row_of(&screen, "charlie"),
    );
    assert!(
        alpha < bravo && bravo < xray && xray < charlie,
        "the page's own order is where the new turn goes:\n{screen}"
    );
}

#[test]
fn a_page_whose_new_turns_follow_the_last_shared_id_lands_them_there() {
    let mut convo = three_turns();
    convo.absorb(&page(
        PANE,
        false,
        Some("t_b"),
        true,
        json!([
            turn("t_b", "assistant", None, md("bravo")),
            turn("t_c", "assistant", None, md("charlie")),
            turn("t_y", "assistant", None, md("yankee")),
        ]),
    ));

    let screen = screen(&mut convo, 60, 30);
    assert!(
        row_of(&screen, "charlie") < row_of(&screen, "yankee"),
        "after the last id the two have in common, not above the first:\n{screen}"
    );
    assert!(row_of(&screen, "alpha") < row_of(&screen, "yankee"), "{screen}");
}

#[test]
fn a_page_that_shares_nothing_is_prepended_whole() {
    let mut convo = three_turns();
    // The one case where position really is a guess, and the only one the older rule was right
    // about: an older slice of the same transcript, which is what `convo.load` answers with.
    convo.absorb(&page(
        PANE,
        false,
        Some("t_q"),
        false,
        json!([turn("t_q", "assistant", None, md("quebec"))]),
    ));

    let screen = screen(&mut convo, 60, 30);
    assert!(
        row_of(&screen, "quebec") < row_of(&screen, "alpha"),
        "an older page goes above what is already held:\n{screen}"
    );
}

#[test]
fn a_fresh_page_drops_every_turn_held_for_the_pane_before_it_applies() {
    let mut convo = three_turns();
    convo.absorb(&page(
        PANE,
        true,
        Some("t_z"),
        false,
        json!([turn("t_z", "assistant", None, md("zulu"))]),
    ));

    let screen = screen(&mut convo, 60, 30);
    assert!(screen.contains("zulu"), "{screen}");
    for gone in ["alpha", "bravo", "charlie"] {
        assert!(
            !screen.contains(gone),
            "`fresh` replaces rather than merges, and {gone} is another transcript:\n{screen}"
        );
    }
}

#[test]
fn a_turn_with_no_blocks_is_withdrawn_rather_than_drawn_as_a_blank_card() {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn(
            "live",
            "assistant",
            None,
            md("the parser is a state machine")
        )]),
    ));
    assert!(convo.has(PANE));

    convo.absorb(&revision(
        PANE,
        json!([turn("live", "assistant", None, json!([]))]),
    ));

    let screen = screen(&mut convo, 60, 20);
    assert!(!screen.contains("state machine"), "{screen}");
    assert!(
        !screen.contains("agent"),
        "a withdrawn turn leaves no card behind, not even its header:\n{screen}"
    );
    assert!(!convo.has(PANE), "and the pane has nothing to show");
}

#[test]
fn a_tool_turn_revised_by_id_is_replaced_rather_than_appended() {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn(
            "t_9",
            "assistant",
            None,
            json!([{ "b": "tool", "name": "Bash", "summary": "probe key grammar", "state": "running" }])
        )]),
    ));
    convo.absorb(&revision(
        PANE,
        json!([turn(
            "t_9",
            "assistant",
            None,
            json!([{ "b": "tool", "name": "Bash", "summary": "probe key grammar", "lines": 48, "state": "done" }])
        )]),
    ));

    let screen = screen(&mut convo, 70, 20);
    assert_eq!(
        screen.matches("Bash").count(),
        1,
        "a tool that renders twice is the whole reason this is a revision:\n{screen}"
    );
    assert!(screen.contains("done") && !screen.contains("running"), "{screen}");
    assert!(screen.contains("48 lines"), "{screen}");
}

#[test]
fn a_resumed_session_keeps_the_nodes_order_however_its_stamps_read() {
    let mut convo = Convo::new();
    // The repo's own fixture has a final record stamped three weeks before the ones above it.
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([
            turn(
                "t_1",
                "assistant",
                Some("2026-08-20T13:00:00Z"),
                md("newest stamp")
            ),
            turn(
                "t_2",
                "assistant",
                Some("2026-08-01T09:00:00Z"),
                md("middle stamp")
            ),
            turn(
                "t_3",
                "assistant",
                Some("2026-07-15T08:00:00Z"),
                md("oldest stamp")
            ),
        ]),
    ));

    let screen = screen(&mut convo, 60, 30);
    assert!(
        row_of(&screen, "newest stamp") < row_of(&screen, "middle stamp")
            && row_of(&screen, "middle stamp") < row_of(&screen, "oldest stamp"),
        "sorting on `at` shuffles a resumed session:\n{screen}"
    );
}

// ---- the blocks --------------------------------------------------------------------------

#[test]
fn a_markdown_table_arrives_as_a_table() {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn(
            "t_1",
            "assistant",
            None,
            md("| Key | Sends |\n| --- | --- |\n| ctrl+b | prefix |\n")
        )]),
    ));

    let screen = screen(&mut convo, 60, 20);
    // The node passes markdown through verbatim precisely so that this is still possible here.
    assert!(screen.contains('┼'), "a table has a frame:\n{screen}");
    assert!(screen.contains("│ Key"), "{screen}");
    assert!(screen.contains("│ ctrl+b"), "{screen}");
    assert!(
        !screen.contains("| Key |"),
        "the pipes are the source, not the rendering:\n{screen}"
    );
}

#[test]
fn a_diff_with_no_unified_headers_is_still_a_diff() {
    let mut convo = Convo::new();
    // `agy` sends the unified diff its edit tool puts in the tool result: hunk headers and all,
    // but no `---`/`+++`. The `+`/`-` prefixes are the only classifier there is.
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn(
            "t_1",
            "assistant",
            None,
            json!([{ "b": "diff", "path": "src/lib.rs",
                     "text": "@@ -1,3 +1,4 @@\n context\n-gone\n+added\n+more\n" }])
        )]),
    ));

    let buf = drawn(&mut convo, 60, 20);
    let screen = rows(&buf).join("\n");
    assert!(screen.contains("+2 -1"), "counted off the prefixes:\n{screen}");
    assert!(screen.contains("@@ -1,3 +1,4 @@"), "{screen}");
    assert_eq!(ink(&buf, "+added"), PHOSPHOR.done, "an addition reads as one");
    assert_eq!(ink(&buf, "-gone"), PHOSPHOR.blocked, "and a removal as one");
}

#[test]
fn a_live_turn_is_marked_rather_than_drawn_as_a_recorded_one() {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([
            turn("t_1", "assistant", Some("2026-08-20T13:00:00Z"), md("recorded")),
            turn(
                "live",
                "assistant",
                None,
                md("the parser is a state machine over")
            ),
        ]),
    ));

    let screen = screen(&mut convo, 60, 20);
    assert!(
        screen.contains("still writing"),
        "the wording may still change:\n{screen}"
    );
    assert_eq!(
        screen.matches("still writing").count(),
        1,
        "and only the live turn wears it:\n{screen}"
    );
}

// ---- the compaction summary ----------------------------------------------------------------

const SUMMARY: &str = "This session is being continued from a previous conversation that ran out \
                       of context.\nThe width inference was the subject.\nThe rect is not the PTY.";

fn summary_turn(id: &str) -> Value {
    let mut turn = turn(id, "user", None, md(SUMMARY));
    turn["kind"] = json!("compact");
    turn
}

fn compacted() -> Convo {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([
            turn("t_1", "user", None, md("carry on where you left off")),
            summary_turn("t_2"),
            turn("t_3", "assistant", None, md("Picking it back up.")),
        ]),
    ));
    convo
}

// The harness files its own summary under a **user** record (#259), so the transcript said the
// operator wrote three paragraphs they never typed. It is still theirs to read — it is drawn shut,
// under its own name, and it opens.
#[test]
fn a_compaction_summary_is_drawn_shut_and_under_its_own_name_rather_than_the_operators() {
    let mut convo = compacted();
    let screen = screen(&mut convo, 60, 24);

    assert!(screen.contains("compacted"), "the summary is named:\n{screen}");
    assert!(
        !screen.contains("ran out of context"),
        "and it is shut, not spelled out:\n{screen}"
    );
    assert_eq!(
        screen.matches("  you").count(),
        1,
        "the one thing the operator actually typed is the only turn in their voice:\n{screen}"
    );
    assert!(
        screen.contains("carry on where you left off") && screen.contains("Picking it back up."),
        "and the turns either side of it are untouched:\n{screen}"
    );
}

// The transcript has no other control on it, so the two keys it takes are taken *only* where there
// is a summary to move — anywhere else they are the agent's own, and a key this surface swallows
// never reaches the PTY.
#[test]
fn an_arrow_opens_a_summary_and_is_handed_back_where_there_is_none_to_open() {
    let right = || KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    let left = || KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);

    let mut convo = compacted();
    assert!(convo.key(PANE, right()), "the summary is shut, so this opens it");
    let opened = screen(&mut convo, 60, 24);
    assert!(opened.contains("ran out of context"), "{opened}");
    assert!(
        !convo.key(PANE, right()),
        "and a second one has nothing left to open, so the agent gets it"
    );

    assert!(convo.key(PANE, left()), "and it puts it away again");
    let shut = screen(&mut convo, 60, 24);
    assert!(!shut.contains("ran out of context"), "{shut}");
    assert!(!convo.key(PANE, left()), "with nothing left to shut");

    let mut plain = three_turns();
    assert!(
        !plain.key(PANE, right()) && !plain.key(PANE, left()),
        "a transcript that was never compacted keeps its arrow keys for the agent"
    );
}

// ---- paging ------------------------------------------------------------------------------

#[test]
fn a_cursor_that_is_absent_is_not_the_same_as_more_being_false() {
    let asked = |cursor: Option<&str>, more: bool| {
        let mut convo = Convo::new();
        convo.absorb(&page(
            PANE,
            true,
            cursor,
            more,
            json!([turn("t_1", "assistant", None, md("only turn"))]),
        ));
        let before = convo.load_more(PANE);
        (before, screen(&mut convo, 60, 12))
    };

    let (before, screen) = asked(Some("t_1"), true);
    assert_eq!(before.as_deref(), Some("t_1"));
    assert!(screen.contains("pgup for earlier turns"), "{screen}");

    // `more` says there is history and no cursor names it: there is nothing this client can ask
    // for, and it must not offer a `convo.load` it cannot send.
    let (before, screen) = asked(None, true);
    assert_eq!(before, None);
    assert!(screen.contains("not pageable"), "{screen}");

    let (before, screen) = asked(Some("t_1"), false);
    assert_eq!(before, None);
    assert!(screen.contains("the start of this transcript"), "{screen}");
}

#[test]
fn paging_past_the_top_of_what_is_held_hands_the_key_back_for_a_convo_load() {
    let mut convo = Convo::new();
    let turns: Vec<Value> = (0..12)
        .map(|n| turn(&format!("t_{n}"), "assistant", None, md(&format!("line {n}"))))
        .collect();
    convo.absorb(&page(PANE, true, Some("t_0"), true, json!(turns)));
    let _ = screen(&mut convo, 60, 8);

    let pgup = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
    let mut handed_back = false;
    for _ in 0..20 {
        if !convo.key(PANE, pgup) {
            handed_back = true;
            break;
        }
    }
    assert!(
        handed_back,
        "the top of what is held is where the node takes over"
    );
    assert_eq!(
        convo.load_more(PANE).as_deref(),
        Some("t_0"),
        "and the cursor is what it is asked with"
    );
    assert!(!convo.key("01JNODE/w1:p9", pgup), "a pane with nothing held");
}

#[test]
fn an_attachment_whose_bytes_are_not_here_shows_the_marker_the_wire_already_carries() {
    let mut convo = Convo::new();
    // The `text` beside an `att` is what a client that has never heard of the field renders, so
    // it is what goes on the screen until the bytes land — and a `404` is expected rather than
    // an error state, because an id names a record in a transcript.
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn(
            "t_1",
            "assistant",
            None,
            json!([{ "b": "md", "text": "[image · png]",
                     "att": { "id": "att_1", "kind": "image", "mime": "image/png", "bytes": 52831 } }])
        )]),
    ));

    let screen = screen(&mut convo, 60, 14);
    assert!(screen.contains("[image · png]"), "{screen}");
}

// ---- the prompt --------------------------------------------------------------------------

#[test]
fn only_a_key_the_prompt_offered_is_ever_answered() {
    let mut convo = Convo::new();
    convo.absorb(&pending(
        PANE,
        Some("Do you want to make this edit?"),
        json!([{ "key": "1", "label": "Yes" }, { "key": "2", "label": "Yes, and don't ask again" }]),
    ));

    assert_eq!(convo.answer(PANE, '1').as_deref(), Some("1"));
    assert_eq!(convo.answer(PANE, '2').as_deref(), Some("2"));
    assert_eq!(convo.answer(PANE, '3'), None, "3 was not offered");
    // The node decides whether a submit key follows, per harness: claude takes the bare digit,
    // codex needs an Enter (#43). A client that synthesises one is answering for the node.
    assert_eq!(convo.answer(PANE, '\r'), None, "an Enter is never synthesised");
    assert_eq!(convo.answer(PANE, '\n'), None);
    assert_eq!(
        convo.answer("01JNODE/w1:p9", '1'),
        None,
        "and not for another pane"
    );
}

#[test]
fn a_null_question_takes_the_strip_down() {
    let mut convo = Convo::new();
    convo.absorb(&pending(
        PANE,
        Some("Do you want to make this edit?"),
        json!([{ "key": "1", "label": "Yes" }]),
    ));
    let up = screen(&mut convo, 60, 12);
    assert!(up.contains("Do you want to make this edit?"), "{up}");
    assert!(up.contains(" 1 ") && up.contains("Yes"), "{up}");

    // There is no resolved event: the same message with a null question is the clearing.
    convo.absorb(&pending(PANE, None, json!([])));

    let down = screen(&mut convo, 60, 12);
    assert!(!down.contains("Do you want to make this edit?"), "{down}");
    assert!(convo.pending(PANE).is_none());
    assert_eq!(convo.answer(PANE, '1'), None);
}

// ---- the scripted node ---------------------------------------------------------------------

struct Conn {
    to_client: mpsc::UnboundedSender<Message>,
    from_client: mpsc::UnboundedReceiver<Value>,
}

impl Conn {
    fn send(&self, frame: Value) {
        self.to_client
            .send(Message::text(frame.to_string()))
            .expect("the scripted node still has a client");
    }

    fn greet(&self, panes: Value) {
        self.send(json!({
            "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "comingclean",
            "build": "0.1.29", "role": "full",
            "caps": { "push": false, "scrollback": true, "conversation": true, "manage": true }
        }));
        self.send(json!({
            "t": "herd",
            "nodes": [{ "id": "01JNODE", "name": "comingclean", "kind": "local", "online": true }],
            "panes": panes
        }));
        self.send(json!({ "t": "prefs", "panes": {} }));
    }

    /// The node closing the socket, which is one of the two ways a connection ends and the one a
    /// test can drive.
    fn close(&self) {
        self.to_client
            .send(Message::Close(None))
            .expect("the scripted node still has a client");
    }

    /// Everything the client wrote in the window a loopback socket needs, so "nothing was sent"
    /// is an assertion about frames rather than about timing.
    async fn heard(&mut self) -> Vec<Value> {
        let mut frames = Vec::new();
        let deadline = tokio::time::Instant::now() + SETTLE;
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, self.from_client.recv()).await {
            frames.push(frame);
        }
        frames
    }

    async fn sent(&mut self, t: &str) -> Value {
        for _ in 0..64 {
            let frame = tokio::time::timeout(BEAT, self.from_client.recv())
                .await
                .unwrap_or_else(|_| panic!("the client never sent a {t}"))
                .expect("the client hung up");
            if frame.get("t").and_then(Value::as_str) == Some(t) {
                return frame;
            }
        }
        panic!("the client never sent a {t}");
    }
}

struct Fake {
    origin: String,
    conns: mpsc::UnboundedReceiver<Conn>,
}

impl Fake {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let origin = format!("http://{}", listener.local_addr().expect("an address"));
        let (conns, incoming) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let conns = conns.clone();
                tokio::spawn(async move {
                    let accepted = tokio_tungstenite::accept_hdr_async(
                        stream,
                        #[allow(clippy::result_large_err)]
                        |request: &Request, mut response: Response| {
                            if let Some(protocol) = request.headers().get("sec-websocket-protocol") {
                                response
                                    .headers_mut()
                                    .insert("sec-websocket-protocol", protocol.clone());
                            }
                            Ok(response)
                        },
                    )
                    .await;
                    let Ok(socket) = accepted else { return };
                    let (mut sink, mut source) = socket.split();
                    let (to_client, mut outbox) = mpsc::unbounded_channel::<Message>();
                    let (inbox, from_client) = mpsc::unbounded_channel::<Value>();
                    if conns
                        .send(Conn {
                            to_client,
                            from_client,
                        })
                        .is_err()
                    {
                        return;
                    }
                    tokio::spawn(async move {
                        while let Some(message) = outbox.recv().await {
                            if sink.send(message).await.is_err() {
                                return;
                            }
                        }
                    });
                    while let Some(Ok(message)) = source.next().await {
                        if let Ok(text) = message.into_text()
                            && let Ok(frame) = serde_json::from_str::<Value>(&text)
                            && inbox.send(frame).is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            origin,
            conns: incoming,
        }
    }

    async fn accept(&mut self) -> Conn {
        tokio::time::timeout(BEAT, self.conns.recv())
            .await
            .expect("no client dialled")
            .expect("the listener died")
    }

    fn client(&self) -> Client {
        Client::with_policy(
            Session {
                origin: self.origin.clone(),
                token: "scripted-token".into(),
                via: Via::Profile {
                    name: "scripted".into(),
                },
            },
            Policy {
                backoff: Backoff {
                    initial: Duration::from_millis(10),
                    max: Duration::from_millis(50),
                },
                connect_timeout: Duration::from_secs(2),
                manage_timeout: Duration::from_millis(500),
                event_capacity: 256,
            },
        )
    }
}

/// Drains the event stream through the shipped router until `want` has been seen.
async fn pump(
    app: &mut App,
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(&Event) -> bool,
) {
    for _ in 0..64 {
        let event = tokio::time::timeout(BEAT, events.recv())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        let done = want(&event);
        app.absorb(&event);
        if done {
            return;
        }
    }
    panic!("the event never arrived");
}

async fn until<T>(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(Event) -> Option<T>,
) -> T {
    for _ in 0..64 {
        let event = tokio::time::timeout(BEAT, events.recv())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        if let Some(found) = want(event) {
            return found;
        }
    }
    panic!("the event never arrived");
}

fn entry(id: &str, agent: Option<&str>, status: &str, conversation: bool) -> Value {
    json!({
        "id": id, "node_id": "01JNODE",
        "workspace_id": "01JNODE/w1", "tab_id": "01JNODE/w1:t1",
        "workspace": "herdr", "tab": "1", "cwd": "/home/dbrain/dev/kampr",
        "agent": agent, "agent_status": status, "rows": 4, "cols": 12,
        "has_conversation": conversation
    })
}

fn down(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

fn painted(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    rows(&buffer).join("\n")
}

fn app(client: &Arc<Client>) -> App {
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();
    app
}

#[tokio::test]
async fn an_agent_pane_opens_on_its_conversation_and_a_shell_pane_opens_on_the_terminal() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "working", true)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    // ADR 0005: a CLI that only ever draws grids opens every agent pane on the wrong view.
    let before = painted(&mut app, 110, 24);
    assert!(before.contains("waiting for the first frame"), "{before}");

    app.convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn("t_1", "assistant", None, md("six, and they are"))]),
    ));

    let after = painted(&mut app, 110, 24);
    assert!(
        after.contains("six, and they are"),
        "the pane opened on the conversation with no prompting:\n{after}"
    );
    assert!(!after.contains("waiting for the first frame"), "{after}");
}

#[tokio::test]
async fn a_shell_pane_has_no_conversation_to_open_on() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([entry(PANE, None, "unknown", false)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    assert!(!app.convo.has(PANE));
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("waiting for the first frame"), "{screen}");
}

#[tokio::test]
async fn a_blocked_agent_is_one_keystroke_from_its_question_and_its_answer() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "blocked", true)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    conn.send(json!({
        "t": "pending", "pane": PANE, "question": "Do you want to make this edit?",
        "options": [{ "key": "1", "label": "Yes" }, { "key": "2", "label": "Yes, and don't ask again" }],
        "source": "screen"
    }));
    let arrived = until(&mut events, |e| match e {
        Event::Pending(p) => Some(Event::Pending(p)),
        _ => None,
    })
    .await;
    app.convo.absorb(&arrived);

    // Answering a blocked agent on another host from the sidebar is the one thing this client can
    // do that a herdr at the desk cannot, so the question is read without opening anything.
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains('⚑'), "the triage list flags it:\n{screen}");
    assert!(
        screen.contains("Do you want to make this"),
        "and the sidebar carries the question itself:\n{screen}"
    );
    assert!(screen.contains(" 1 ") && screen.contains("Yes"), "{screen}");

    app.key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));

    let answer = conn.sent("answer").await;
    assert_eq!(answer["pane"], json!(PANE));
    assert_eq!(answer["key"], json!("1"));
}

// ---- what the shell and the transcript must not share ----------------------------------------

fn grid_reset(pane: &str) -> Value {
    json!({
        "t": "grid.reset", "pane": pane, "cols": 12, "rows": 4,
        "rows_data": [{ "s": 0, "row": 0, "runs": [{ "s": 0, "x": "hello" }] }],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": []
    })
}

#[tokio::test]
async fn a_key_the_conversation_consumed_reaches_neither_the_pty_nor_the_ring() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "working", true)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    conn.send(grid_reset(PANE));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);
    app.convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!(
            (0..40)
                .map(|n| turn(&format!("t_{n}"), "assistant", None, md(&format!("line {n}"))))
                .collect::<Vec<_>>()
        ),
    ));
    let opened = painted(&mut app, 110, 24);
    assert!(opened.contains("line 39"), "on its conversation:\n{opened}");
    let _ = conn.heard().await;

    // The transcript scrolls; the pane's own ring is a different surface and must not move with
    // it, and the agent's PTY must never see the key at all.
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let after = painted(&mut app, 110, 24);
    assert!(!after.contains("back"), "the pane's ring did not move:\n{after}");
    let frames = conn.heard().await;
    assert!(
        !frames.iter().any(|f| f["t"] == json!("input")),
        "an arrow key the conversation took is not typed into the agent: {frames:?}"
    );
}

#[tokio::test]
async fn a_question_does_not_survive_the_socket_that_carried_it() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut app = app(&client);
    conn.greet(json!([entry(PANE, Some("claude"), "blocked", true)]));
    pump(&mut app, &mut events, |e| matches!(e, Event::Prefs { .. })).await;
    conn.send(json!({
        "t": "pending", "pane": PANE, "question": "Do you want to make this edit?",
        "options": [{ "key": "1", "label": "Yes" }], "source": "screen"
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Pending(_))).await;
    let asked = painted(&mut app, 110, 24);
    assert!(asked.contains("Do you want to make this"), "{asked}");

    conn.close();
    pump(&mut app, &mut events, |e| matches!(e, Event::Disconnected { .. })).await;
    assert!(
        app.convo.pending(PANE).is_none(),
        "nothing carried across a dropped socket is trustworthy, and a question least of all"
    );

    // `pending` is published on a blocked-state **edge**, so the node's first attempt at a pane
    // that is still blocked carries nothing at all. A client that kept the old question would
    // answer a key into a pane with nothing matching to answer it.
    let mut conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "blocked", true)]));
    pump(&mut app, &mut events, |e| matches!(e, Event::Prefs { .. })).await;
    let _ = conn.heard().await;

    let again = painted(&mut app, 110, 24);
    assert!(!again.contains("Do you want to make this"), "{again}");
    app.key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    let frames = conn.heard().await;
    assert!(
        !frames.iter().any(|f| f["t"] == json!("answer")),
        "a key is answered only against a question this connection offered: {frames:?}"
    );
}

// ---- attachments, and the pixels that outlive a view -----------------------------------------

/// A node that serves one attachment, so an image can actually land and be drawn.
async fn serving(body: Vec<u8>, mime: &'static str) -> Session {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let origin = format!("http://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte).await {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                let mut out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                out.extend_from_slice(&body);
                let _ = stream.write_all(&out).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Session {
        origin,
        token: "scripted-token".into(),
        via: Via::Profile {
            name: "scripted".into(),
        },
    }
}

fn png_bytes() -> Vec<u8> {
    let bitmap = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        24,
        24,
        image::Rgba([12, 200, 90, 255]),
    ));
    let mut out = std::io::Cursor::new(Vec::new());
    bitmap.write_to(&mut out, image::ImageFormat::Png).expect("a png");
    out.into_inner()
}

fn kitty(session: &Session) -> Images {
    Images::with(
        session,
        Some("kitty(0.48.2)"),
        kampr_tui::image::Caps {
            kitty_graphics: true,
            sixel: false,
        },
        Some((8, 16)),
    )
}

fn shot(id: &str) -> Value {
    json!([{ "b": "md", "text": "[image · png]",
             "att": { "id": id, "kind": "image", "mime": "image/png", "name": "shot.png" } }])
}

/// Draw until the fetch has landed and the picture is actually on the terminal.
async fn until_drawn(app: &mut App) {
    for _ in 0..200 {
        painted(app, 110, 30);
        if app.images.drawn() > 0 {
            return;
        }
        app.images.collect();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the picture never reached the terminal");
}

const OTHER: &str = "01JNODE/w1:p2";

#[tokio::test]
async fn an_image_does_not_outlive_the_view_that_drew_it() {
    let session = serving(png_bytes(), "image/png").await;
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([
        entry(PANE, Some("claude"), "working", true),
        entry(OTHER, Some("codex"), "idle", true)
    ]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), kitty(&session));
    app.refocus();
    app.convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn("t_1", "assistant", None, shot("att_1"))]),
    ));
    until_drawn(&mut app).await;

    // A drawn image is **not in the buffer** — its cells are `Skip`, so ratatui's diff cannot
    // repaint them and the pixels stay on the terminal until something takes them down.
    app.clicked(Click::Focus(OTHER.to_string()));
    assert_eq!(app.images.drawn(), 0, "the escape is taken back");
    assert!(app.wiping(), "and the terminal is wiped before the next frame");
}

#[tokio::test]
async fn swapping_a_pane_to_its_terminal_takes_its_pictures_with_it() {
    let session = serving(png_bytes(), "image/png").await;
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "working", true)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), kitty(&session));
    app.refocus();
    app.convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn("t_1", "assistant", None, shot("att_1"))]),
    ));
    until_drawn(&mut app).await;

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT));
    assert_eq!(
        app.images.drawn(),
        0,
        "the conversation is gone and so is its picture"
    );
    assert!(app.wiping());
}

#[tokio::test]
async fn an_attachment_this_terminal_will_not_draw_says_how_big_it_is_and_offers_to_save_it() {
    let saved = std::env::temp_dir().join(format!("kampr-tui-save-{}", std::process::id()));
    std::fs::create_dir_all(&saved).expect("a directory to save into");
    // The one process-wide knob `App::save` reads. Nothing else in this binary looks at it.
    unsafe { std::env::set_var("XDG_DOWNLOAD_DIR", &saved) };

    let session = serving(b"%PDF-1.7\n".to_vec(), "application/octet-stream").await;
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "working", true)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), kitty(&session));
    app.refocus();
    app.convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn(
            "t_1",
            "assistant",
            None,
            json!([{ "b": "md", "text": "[report]",
                 "att": { "id": "att_9", "kind": "file", "mime": "application/pdf",
                          "name": "report.pdf" } }])
        )]),
    ));

    let mut screen = String::new();
    for _ in 0..200 {
        screen = painted(&mut app, 110, 30);
        if screen.contains("click to save") {
            break;
        }
        app.images.collect();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // `kind` is an open string: a client that does not recognise one treats it as a file rather
    // than dropping the block, and the download is the affordance it gets instead of a picture.
    assert!(screen.contains("click to save"), "{screen}");
    assert!(screen.contains("9 B"), "and how much there is of it:\n{screen}");

    let (pane, id, rect) = app.layout.attachments[0].clone();
    assert_eq!((pane.as_str(), id.as_str()), (PANE, "att_9"));
    let click = app
        .mouse
        .hit(down(rect.x + 2, rect.y), &app.layout, kampr_client::Role::Full);
    app.clicked(click);
    let after = painted(&mut app, 110, 30);
    assert!(after.contains("saved"), "{after}");
    assert_eq!(
        std::fs::read(saved.join("report.pdf")).expect("the bytes went to disk"),
        b"%PDF-1.7\n"
    );
    let _ = std::fs::remove_dir_all(&saved);
}

#[tokio::test]
async fn a_path_in_a_tool_call_is_offered_to_a_writer_and_never_to_a_readonly_device() {
    let session = serving(png_bytes(), "image/png").await;
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([entry(PANE, Some("claude"), "working", true)]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), kitty(&session));
    app.refocus();
    let tool = json!([{ "b": "tool", "name": "Read", "state": "done",
                        "summary": "wrote /tmp/hero-mock.png" }]);
    app.convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn("t_1", "assistant", None, tool)]),
    ));

    let writing = painted(&mut app, 110, 30);
    assert!(
        writing.contains("[/tmp/hero-mock.png]"),
        "a picture a tool call named is offered:\n{writing}"
    );

    // **Only a device that may send input may ask for one** — the node answers a read-only
    // device `403`, so the affordance is absent rather than a fetch that comes back refused.
    conn.send(json!({ "t": "role", "role": "readonly" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Role(_))).await;
    let reading = painted(&mut app, 110, 30);
    assert!(
        !reading.contains("[/tmp/hero-mock.png]"),
        "and it moves with the live role:\n{reading}"
    );
}

/// **What is waiting is the harness's own record, not a guess.** A prompt sent while the agent is
/// working is queued rather than answered, and the transcript gains a `queue-operation` long
/// before it gains a turn — so a conversation that draws only turns shows nothing at all until the
/// agent gets round to it, while the terminal beside it has shown the prompt the whole time.
#[test]
fn a_prompt_the_harness_has_queued_is_drawn_as_waiting_rather_than_not_at_all() {
    let mut convo = Convo::new();
    convo.absorb(&page(
        PANE,
        true,
        None,
        false,
        json!([turn("t1", "assistant", None, md("the answer before it"))]),
    ));
    convo.absorb(&facets(
        PANE,
        json!([{ "text": "and now do the other half", "at": "2026-08-28T21:04:00Z" }]),
    ));

    let screen = screen(&mut convo, 60, 20);
    assert!(
        screen.contains("and now do the other half"),
        "the queued prompt is on the conversation before any record arrives:\n{screen}"
    );
    assert!(
        screen.contains("queued"),
        "and it is named as waiting rather than drawn as a recorded turn:\n{screen}"
    );
    assert!(
        !screen.contains("  you"),
        "the queue is the pane's, so a prompt in it may not be this operator's:\n{screen}"
    );
}

/// The queue is republished whole whenever it moves, so the newest one replaces what is held —
/// a prompt the agent has taken leaves rather than lingering.
#[test]
fn a_prompt_the_agent_has_taken_leaves_the_queue_rather_than_standing_in_it() {
    let mut convo = Convo::new();
    convo.absorb(&page(PANE, true, None, false, json!([])));
    convo.absorb(&facets(PANE, json!([{ "text": "first" }, { "text": "second" }])));
    assert!(screen(&mut convo, 60, 20).contains("first"));

    convo.absorb(&facets(PANE, json!([{ "text": "second" }])));
    let screen = screen(&mut convo, 60, 20);
    assert!(!screen.contains("first"), "{screen}");
    assert!(screen.contains("second"), "{screen}");
}

/// **The pane's own half-typed line, which the terminal client was being sent and dropping.**
///
/// `input` is `pane.send_text` and appends to whatever is already on the line, so a reply sent from
/// here joins onto it. The node measures and publishes that line; a client that ignores the frame
/// leaves the operator to find out by sending.
#[test]
fn what_the_operator_left_at_the_desk_is_shown_before_a_reply_joins_onto_it() {
    let mut convo = Convo::new();
    convo.absorb(&page(PANE, true, None, false, json!([])));
    convo.absorb(&desk(PANE, Some("half a sentence")));

    let shown = screen(&mut convo, 60, 20);
    assert!(
        shown.contains("half a sentence"),
        "the line waiting at the pane is on screen:\n{shown}"
    );

    // Empty is `text: null` rather than an absent frame, and it is what takes the strip down —
    // the same rule `pending` follows, because there is no resolved event for either.
    convo.absorb(&desk(PANE, None));
    let cleared = screen(&mut convo, 60, 20);
    assert!(
        !cleared.contains("half a sentence"),
        "and it comes down when the pane's line is emptied:\n{cleared}"
    );
}
