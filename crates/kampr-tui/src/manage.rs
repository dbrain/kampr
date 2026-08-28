//! W4 — the `manage` surface: the prompt, the confirmation and the notice around every op the
//! node already accepts.
//!
//! Three rules shape it and none of them are negotiable. **What a node does not claim is absent**
//! rather than disabled, which is what [`Manage::begin`] returning `None` is. **Nothing here
//! touches the herd** — the op goes out, the `herd.patch` comes back, and the node stays
//! authoritative in between. And a geometry-changing op **says what it will do to other people
//! before it does it**: a pane's size changing under somebody attached at the desk is announced
//! nowhere, which is what #298 measured with a cropped line and no error at either end.

use crate::keymap::Action;
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_client::{Caps, Event, Herd, Managed, NodeCaps, Role};
use kampr_core::wire::{NodeEntry, PaneEntry};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// How long the strip under the panes keeps saying what an op did. The same five-second-ish
/// window the status line's own notes use, stretched because a refusal is worth reading twice.
const NOTICE: Duration = Duration::from_secs(8);
const WIDTH: u16 = 74;

/// What one bind opened. `op` is the body of a `manage` message — `{"op": "...", "at": "..."}` —
/// with the `t` and the `rid` left to the client; `None` is a prompt still collecting a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub title: String,
    pub op: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Nothing is open and this key is not ours.
    Idle,
    Consumed,
    Send(Value),
    Cancelled,
}

/// What an empty answer means, which is not the same thing three times. A creation **omits** the
/// key, because omitting is what "let herdr name it" is: the schema types every one of these as
/// `["string","null"]` and not required, so a `null` would be accepted too — it would just be this
/// client asserting a name of nothing rather than declining to name one. A pane rename **nulls**
/// it, because `null` is the only way to clear a label. A tab or workspace rename **refuses** it
/// here rather than spending a round trip learning that herdr wants a string.
///
/// An earlier version of this comment claimed herdr 0.8.2 refuses a `null` where it wants a value.
/// **There is no probe row behind that and the schema contradicts it**; `kampr-node` has always
/// emitted `null` for these and #46 created a workspace through that path. Nothing depends on
/// which is true — omitting is correct either way — but the claim is gone rather than left to be
/// cited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Empty {
    Omit,
    Null,
    Refuse,
}

#[derive(Debug, Clone)]
struct Row {
    key: Option<char>,
    label: String,
    note: String,
    next: Next,
}

#[derive(Debug, Clone)]
enum Next {
    Send(Value),
    Confirm { lines: Vec<String>, op: Value },
    Ask(Ask),
    Pick { title: String, rows: Vec<Row> },
}

#[derive(Debug, Clone)]
struct Ask {
    prompt: String,
    hint: String,
    op: Value,
    field: &'static str,
    empty: Empty,
}

#[derive(Debug, Clone)]
enum Stage {
    Pick { rows: Vec<Row>, at: usize },
    Ask { ask: Ask, buf: String },
    Confirm { lines: Vec<String>, op: Value },
}

#[derive(Debug, Clone)]
struct Modal {
    title: String,
    stage: Stage,
}

enum Taken {
    Stay,
    Cancel,
    Next(Next),
}

#[derive(Debug, Default)]
pub struct Manage {
    open: Option<Modal>,
    /// What the node said it can be *asked to make*, as opposed to what it can do. The kinds and
    /// the sessions both come from here rather than from a list compiled into this client.
    caps: NodeCaps,
    inflight: Option<(String, Instant)>,
    outcome: Option<(String, Instant)>,
    /// herdr's own split tree, held opaque between a `layout.export` ack and a `layout.apply`.
    exported: Option<Value>,
}

impl Manage {
    pub fn new() -> Self {
        Self::default()
    }

    /// A bind asked for an op. **`None` hides rather than disables**: a node that does not claim
    /// `manage`, a read-only device, or an op whose `tab_id` the pane entry does not carry.
    pub fn begin(
        &mut self,
        action: Action,
        herd: &Herd,
        pane: Option<&str>,
        caps: &Caps,
        role: Role,
    ) -> Option<Prompt> {
        // The live role, not the greeting's: a demotion lands mid-connection as `role` and every
        // op a `readonly` device sends is refused with `not_writer`.
        if !caps.manage || !role.writes() {
            return None;
        }
        let entry = pane.and_then(|id| herd.pane(id));
        let next = self.plan(action, herd, entry)?;
        let title = title_of(action);
        match next {
            Next::Send(op) => Some(Prompt { title, op: Some(op) }),
            other => {
                self.open = Some(Modal {
                    title: title.clone(),
                    stage: stage_of(other),
                });
                Some(Prompt { title, op: None })
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Progress {
        if self.open.is_none() {
            return Progress::Idle;
        }
        if key.code == KeyCode::Esc {
            self.open = None;
            return Progress::Cancelled;
        }
        match self.take(key) {
            Taken::Stay => Progress::Consumed,
            Taken::Cancel => {
                self.open = None;
                Progress::Cancelled
            }
            Taken::Next(Next::Send(op)) => self.fire(op),
            Taken::Next(next) => {
                let title = match &next {
                    Next::Pick { title, .. } => title.clone(),
                    _ => self.open.as_ref().map(|m| m.title.clone()).unwrap_or_default(),
                };
                self.open = Some(Modal {
                    title,
                    stage: stage_of(next),
                });
                Progress::Consumed
            }
        }
    }

    pub fn active(&self) -> bool {
        self.open.is_some()
    }

    /// Shut a modal that is collecting an op this device may no longer send. A demotion lands
    /// mid-connection, and a prompt left open is a prompt whose op comes back `not_writer`.
    pub fn close(&mut self) {
        self.open = None;
    }

    pub fn footer(&self) -> Option<String> {
        let modal = self.open.as_ref()?;
        let head = modal.title.to_uppercase();
        Some(match &modal.stage {
            // The keys are down the popup's own left column; repeating nine of them here is a
            // footer that runs off the end of the terminal and says less.
            Stage::Pick { .. } => {
                format!("{head}  up/down choose · a key jumps · enter do it · esc cancel")
            }
            Stage::Ask { .. } => {
                format!("{head}  type it · enter send · backspace erase · esc cancel")
            }
            Stage::Confirm { .. } => format!("{head}  enter do it · n / esc leave it alone"),
        })
    }

    pub fn render(&self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        if area.width < 8 || area.height < 3 {
            return;
        }
        match &self.open {
            Some(modal) => self.popup(modal, buf, area, theme),
            None => self.strip(buf, area, theme),
        }
    }

    /// Every `managed` ack nothing was awaiting, and every `error` frame. **`ok` on the ack, not
    /// its arrival**, is what a refusal is read from.
    pub fn observe(&mut self, event: &Event) {
        match event {
            Event::Caps(caps) => self.caps = caps.clone(),
            Event::Managed(ack) => self.ack(ack),
            // A refusal is acked *and* followed by an ordinary `error`, and the error is the half
            // that reaches here when the ack went to the op's own waiter. `stream_unavailable` is
            // the one code that is never a refusal — it is a pane's screen, not an op — so it must
            // not clear an op that is still outstanding.
            Event::Error(failure) if failure.code != "stream_unavailable" => {
                let Some((op, _)) = self.inflight.take() else {
                    return;
                };
                let why = match failure.message.is_empty() {
                    true => failure.code.clone(),
                    false => failure.message.clone(),
                };
                self.note(format!("{op} was refused · {why}"));
            }
            _ => {}
        }
    }

    fn plan(&self, action: Action, herd: &Herd, entry: Option<&PaneEntry>) -> Option<Next> {
        use Action::*;
        let pane = entry.map(|e| e.id.as_str());
        // A pane id carries its workspace and **never its tab**, so a client without `tab_id`
        // cannot address `tab.rename`, `tab.close` or `tab.focus` at all.
        let tab = entry.and_then(|e| e.tab_id.as_deref());
        let workspace = entry.and_then(|e| e.workspace_id.as_deref());
        match action {
            NewWorkspace => Some(self.menu(herd, entry)),
            NewTab => workspace.map(|at| Next::Send(json!({ "op": "tab.create", "at": at }))),
            RenameTab => tab.map(|at| rename(at, "tab", entry.and_then(|e| e.tab.clone()), Empty::Refuse)),
            CloseTab => tab.map(|at| close(at, "tab")),
            RenameWorkspace => workspace.map(|at| {
                rename(
                    at,
                    "workspace",
                    entry.and_then(|e| e.workspace.clone()),
                    Empty::Refuse,
                )
            }),
            CloseWorkspace => workspace.map(|at| close(at, "workspace")),
            SplitVertical => pane.map(|at| split(at, "right")),
            SplitHorizontal => pane.map(|at| split(at, "down")),
            ClosePane => pane.map(|at| close(at, "pane")),
            RenamePane => pane.map(|at| rename(at, "pane", entry.and_then(|e| e.label.clone()), Empty::Null)),
            NewWorktree => self.worktrees(herd),
            _ => None,
        }
    }

    /// Everything the wire carries, in one list, because the keymap has eleven binds and the
    /// node accepts fourteen ops. The rows a node has not answered for are absent.
    fn menu(&self, herd: &Herd, entry: Option<&PaneEntry>) -> Next {
        let mut rows = Vec::new();
        let nodes = reachable(herd);
        if !nodes.is_empty() {
            rows.push(Row {
                key: Some('w'),
                label: "workspace".into(),
                note: "a new workspace on a machine".into(),
                next: on_a_node(&nodes, "new workspace on", |node| {
                    Next::Ask(Ask {
                        prompt: format!("What is the workspace on {node} called?"),
                        hint: "enter with nothing typed lets herdr name it".into(),
                        op: json!({ "op": "workspace.create", "node": node }),
                        field: "label",
                        empty: Empty::Omit,
                    })
                }),
            });
        }
        if let Some(at) = entry.and_then(|e| e.workspace_id.as_deref()) {
            rows.push(Row {
                key: Some('t'),
                label: "tab".into(),
                note: "a new tab in this workspace".into(),
                next: Next::Ask(Ask {
                    prompt: "What is the tab called?".into(),
                    hint: "enter with nothing typed lets herdr name it".into(),
                    // `at` for tab.create is a WORKSPACE id; nodes take a tab id and derive it.
                    op: json!({ "op": "tab.create", "at": at }),
                    field: "label",
                    empty: Empty::Omit,
                }),
            });
        }
        if let Some(at) = entry.map(|e| e.id.as_str()) {
            rows.push(Row {
                key: Some('s'),
                label: "split".into(),
                note: "divide this pane at the desk".into(),
                next: Next::Pick {
                    title: "split".into(),
                    rows: vec![
                        Row {
                            key: Some('r'),
                            label: "right".into(),
                            note: "side by side".into(),
                            next: split(at, "right"),
                        },
                        Row {
                            key: Some('d'),
                            label: "down".into(),
                            note: "one above the other".into(),
                            next: split(at, "down"),
                        },
                    ],
                },
            });
        }
        if let Some(row) = self.agents(entry) {
            rows.push(row);
        }
        if let Some(next) = self.worktrees(herd) {
            rows.push(Row {
                key: Some('g'),
                label: "worktree".into(),
                note: "a git worktree, made or opened".into(),
                next,
            });
        }
        if let Some(row) = self.sessions() {
            rows.push(row);
        }
        if let Some(row) = self.layouts(entry) {
            rows.push(row);
        }
        if let Some(at) = entry.map(|e| e.id.as_str()) {
            rows.push(Row {
                key: Some('f'),
                label: "focus".into(),
                note: "put this pane in front at the desk".into(),
                next: Next::Send(json!({ "op": "focus", "at": at })),
            });
            rows.push(Row {
                key: Some('z'),
                label: "zoom".into(),
                note: "give this pane the tab, at the desk".into(),
                next: Next::Confirm {
                    lines: vec![
                        format!("Zoom {at} at the desk?"),
                        "herdr gives the pane the whole tab and hands it back on the next \
                         toggle. It is not this client's own zoom: #265 measured the PTY going \
                         84 to 171 columns under an attached client, so a program in it is \
                         redrawn at a size it did not ask for."
                            .into(),
                    ],
                    op: json!({ "op": "pane.zoom", "at": at, "mode": "toggle" }),
                },
            });
            rows.push(Row {
                key: Some('r'),
                label: "size".into(),
                note: "give this pane a real width, if it was born tiny".into(),
                next: size_menu(at),
            });
        }
        Next::Pick {
            title: "manage".into(),
            rows,
        }
    }

    /// **The kinds are the node's**, and a peer's harnesses are its own — `caps` answers for one
    /// node, so offering its list against another node's pane would be a guess wearing an
    /// answer's clothes.
    fn agents(&self, entry: Option<&PaneEntry>) -> Option<Row> {
        let entry = entry?;
        if self.caps.agent_kinds.is_empty() || entry.node_id != self.caps.node {
            return None;
        }
        let at = entry.id.as_str();
        let rows = self
            .caps
            .agent_kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| Row {
                key: digit(i),
                label: kind.clone(),
                note: String::new(),
                next: Next::Send(json!({ "op": "agent.start", "at": at, "kind": kind, "args": [] })),
            })
            .collect();
        Some(Row {
            key: Some('a'),
            label: "agent".into(),
            note: format!(
                "{} kinds this node named, in this pane",
                self.caps.agent_kinds.len()
            ),
            next: Next::Pick {
                title: "agent".into(),
                rows,
            },
        })
    }

    /// A **stopped** session can be started and a running one stopped. Neither is offered as
    /// somewhere to put a workspace or a pane: a session that is running and not `served` never
    /// joins this herd, so anything made in it would be invisible here for ever.
    fn sessions(&self) -> Option<Row> {
        if self.caps.sessions.is_empty() || self.caps.node.is_empty() {
            return None;
        }
        let node = self.caps.node.as_str();
        let mut rows: Vec<Row> = self
            .caps
            .sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let name = session.name.as_str();
                let note = match (session.running, session.served) {
                    (true, true) => "running · served here".into(),
                    (true, false) => "running · not served here — never joins this herd".into(),
                    (false, _) => "stopped".to_string(),
                };
                let next = match session.running {
                    true => Next::Confirm {
                        lines: vec![
                            format!("Stop the named session {name}?"),
                            "A named session is its own herdr server: every pane in it goes with \
                             it, and it leaves the herd. The ack waits for the host to agree \
                             before it answers (#241), so what it says has already happened."
                                .into(),
                        ],
                        op: json!({ "op": "session.stop", "node": node, "name": name }),
                    },
                    false => Next::Send(json!({ "op": "session.create", "node": node, "name": name })),
                };
                Row {
                    key: digit(i),
                    label: name.to_string(),
                    note,
                    next,
                }
            })
            .collect();
        rows.push(Row {
            key: Some('+'),
            label: "new…".into(),
            note: "start a session this node has never had".into(),
            next: Next::Ask(Ask {
                prompt: "What is the new session called?".into(),
                hint: "letters, digits, dash and underscore — it reaches a command line".into(),
                op: json!({ "op": "session.create", "node": node }),
                field: "name",
                empty: Empty::Refuse,
            }),
        });
        Some(Row {
            key: Some('n'),
            label: "session".into(),
            note: format!("{} named on this node", self.caps.sessions.len()),
            next: Next::Pick {
                title: "session".into(),
                rows,
            },
        })
    }

    /// The tree is opaque to this client: an export is held exactly as it arrived, and
    /// `layout.apply` takes the whole reply or just its `root` — the node accepts either.
    fn layouts(&self, entry: Option<&PaneEntry>) -> Option<Row> {
        let at = entry?.tab_id.as_deref()?;
        let mut rows = vec![Row {
            key: Some('e'),
            label: "export".into(),
            note: "hold this tab's split tree".into(),
            next: Next::Send(json!({ "op": "layout.export", "at": at })),
        }];
        if let Some(layout) = &self.exported {
            rows.push(Row {
                key: Some('a'),
                label: "apply".into(),
                note: "put the held tree on this tab".into(),
                next: Next::Confirm {
                    lines: vec![
                        format!("Lay {at} out as the held tree?"),
                        "Every pane in the tab is moved and resized, here and at the desk.".into(),
                    ],
                    op: json!({ "op": "layout.apply", "at": at, "layout": layout }),
                },
            });
        }
        Some(Row {
            key: Some('l'),
            label: "layout".into(),
            note: match self.exported.is_some() {
                true => "export, or apply the one held".into(),
                false => "export this tab's split tree".into(),
            },
            next: Next::Pick {
                title: "layout".into(),
                rows,
            },
        })
    }

    fn worktrees(&self, herd: &Herd) -> Option<Next> {
        let nodes = reachable(herd);
        if nodes.is_empty() {
            return None;
        }
        Some(on_a_node(&nodes, "worktree on", |node| Next::Pick {
            title: "worktree".into(),
            rows: vec![
                Row {
                    key: Some('c'),
                    label: "create".into(),
                    note: "a new branch and a workspace on it".into(),
                    next: Next::Ask(Ask {
                        prompt: format!("Which branch is the worktree on {node} for?"),
                        hint: "herdr branches it off the repository's default base".into(),
                        op: json!({ "op": "worktree.create", "node": node }),
                        field: "branch",
                        empty: Empty::Refuse,
                    }),
                },
                Row {
                    key: Some('o'),
                    label: "open".into(),
                    note: "a worktree that is already on disk".into(),
                    next: Next::Ask(Ask {
                        prompt: format!("Which path on {node}?"),
                        hint: "the worktree's own directory, not the repository's".into(),
                        op: json!({ "op": "worktree.open", "node": node }),
                        field: "path",
                        empty: Empty::Refuse,
                    }),
                },
            ],
        }))
    }

    fn take(&mut self, key: KeyEvent) -> Taken {
        let Some(modal) = self.open.as_mut() else {
            return Taken::Stay;
        };
        match &mut modal.stage {
            Stage::Pick { rows, at } => {
                if rows.is_empty() {
                    return Taken::Cancel;
                }
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        *at = at.checked_sub(1).unwrap_or(rows.len() - 1);
                        Taken::Stay
                    }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                        *at = (*at + 1) % rows.len();
                        Taken::Stay
                    }
                    KeyCode::Enter => Taken::Next(rows[*at].next.clone()),
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        match rows.iter().position(|r| r.key == Some(c)) {
                            Some(i) => {
                                *at = i;
                                Taken::Next(rows[i].next.clone())
                            }
                            None => Taken::Stay,
                        }
                    }
                    _ => Taken::Stay,
                }
            }
            Stage::Ask { ask, buf } => match key.code {
                KeyCode::Backspace => {
                    buf.pop();
                    Taken::Stay
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    buf.push(c);
                    Taken::Stay
                }
                KeyCode::Enter => {
                    let text = buf.trim().to_string();
                    let mut op = ask.op.clone();
                    match (text.is_empty(), ask.empty) {
                        (true, Empty::Refuse) => return Taken::Stay,
                        (true, Empty::Omit) => {}
                        (true, Empty::Null) => op[ask.field] = Value::Null,
                        (false, _) => op[ask.field] = json!(text),
                    }
                    Taken::Next(Next::Send(op))
                }
                _ => Taken::Stay,
            },
            Stage::Confirm { op, .. } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => Taken::Next(Next::Send(op.clone())),
                KeyCode::Char('n') => Taken::Cancel,
                _ => Taken::Stay,
            },
        }
    }

    /// The op leaves and **nothing here changes**: the herd is the node's to move, and this
    /// client's own copy of it does not learn a thing until the `herd.patch` arrives.
    fn fire(&mut self, op: Value) -> Progress {
        self.open = None;
        let name = op["op"].as_str().unwrap_or_default().to_string();
        self.outcome = None;
        self.inflight = Some((name, Instant::now()));
        Progress::Send(op)
    }

    fn ack(&mut self, ack: &Managed) {
        if self.inflight.as_ref().is_some_and(|(op, _)| *op == ack.op) {
            self.inflight = None;
        }
        if !ack.ok {
            let why = ack
                .message
                .clone()
                .filter(|m| !m.is_empty())
                .or_else(|| ack.code.clone())
                .unwrap_or_else(|| "refused, with nothing said".into());
            self.note(format!("{} was refused · {why}", ack.op));
            return;
        }
        if ack.op == "layout.export" {
            match &ack.layout {
                Some(layout) => {
                    self.exported = Some(layout.clone());
                    self.note("layout.export · this tab's tree is held for layout.apply".into());
                }
                None => self.note("layout.export · the node acked without a layout".into()),
            }
            return;
        }
        // **A session's `id` is its bare name, not a node-qualified container id.** It names a
        // herdr server that joins the herd as a node of its own; treating it as a pane id
        // produced something shaped exactly like one that nothing can watch.
        let told = match (ack.op.starts_with("session."), &ack.id) {
            (true, Some(name)) => {
                self.session_settled(name, ack.op.as_str());
                format!("{} · the session {name} — it joins the herd as a node", ack.op)
            }
            (false, Some(id)) => format!("{} · {id} — the herd patch brings it here", ack.op),
            (_, None) => format!("{} · done", ack.op),
        };
        self.note(told);
    }

    /// #241: a session ack is a promise the **host** already agrees — the node polls
    /// `herdr session list` before answering — so the cached answer may be moved on it. Nothing
    /// else here is ever moved ahead of the node.
    fn session_settled(&mut self, name: &str, op: &str) {
        let running = op == "session.create";
        match self.caps.sessions.iter_mut().find(|s| s.name == name) {
            Some(session) => session.running = running,
            None => self.caps.sessions.push(kampr_client::SessionCaps {
                name: name.to_string(),
                running,
                served: false,
            }),
        }
    }

    fn note(&mut self, text: String) {
        self.outcome = Some((text, Instant::now()));
    }

    /// **Both halves, when both are true.** An op that is still outstanding is a fact about now,
    /// and letting the last outcome stand in front of it is how a client comes to look settled
    /// while it is still waiting on something.
    fn notice(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some((text, at)) = &self.outcome
            && at.elapsed() < NOTICE
        {
            parts.push(text.clone());
        }
        if let Some((op, at)) = &self.inflight
            && at.elapsed() < NOTICE
        {
            parts.push(format!(
                "{op} sent · waiting for the node — nothing moves here until the herd patch does"
            ));
        }
        match parts.is_empty() {
            true => None,
            false => Some(parts.join(" · ")),
        }
    }

    fn strip(&self, buf: &mut Buffer, area: Rect, t: &Theme) {
        let Some(text) = self.notice() else {
            return;
        };
        let row = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        Clear.render(row, buf);
        Paragraph::new(Line::from(Span::styled(
            format!(" manage · {text}"),
            Style::default().fg(t.accent).bg(t.accent_soft),
        )))
        .style(Style::default().bg(t.accent_soft))
        .render(row, buf);
    }

    fn popup(&self, modal: &Modal, buf: &mut Buffer, area: Rect, t: &Theme) {
        let width = WIDTH.min(area.width);
        let lines = body(modal, width.saturating_sub(4) as usize, t);
        let height = (lines.len() as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        Clear.render(rect, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        format!(" {} ", modal.title),
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    )),
            )
            .style(Style::default().bg(t.surface))
            .render(rect, buf);
    }
}

/// The sizes `pane.size` offers, and the only place in Kampr that reshapes a pane.
///
/// A menu rather than a typed number because these three are the ones worth having and every one
/// of them clears the node's floor — a pane keeps the size it is given, so an operator who can type
/// `40` into a prompt is an operator who can lock a pane at 40 columns for everybody.
///
/// The complement to `zoom` above rather than a replacement for it: `pane.zoom` moves the PTY only
/// when a client is attached and does nothing at all headless (#265), and this is the other half.
fn size_menu(at: &str) -> Next {
    let rows = [(80, 24), (120, 40), (200, 50)]
        .into_iter()
        .map(|(cols, rows)| Row {
            key: None,
            label: format!("{cols}x{rows}"),
            note: String::new(),
            next: Next::Confirm {
                lines: vec![
                    format!("Resize {at} to {cols}x{rows}?"),
                    "Kampr claims the PTY to do it and hands it straight back. On a headless \
                     pane the size stays (#219); on one somebody has open at their desk it \
                     reverts the moment the claim is released (#19) and their screen is wrong \
                     while it is held (#298). The reply says which happened rather than assuming."
                        .into(),
                ],
                op: json!({ "op": "pane.size", "at": at, "cols": cols, "rows": rows }),
            },
        })
        .collect();
    Next::Pick {
        title: "pane size".into(),
        rows,
    }
}

fn stage_of(next: Next) -> Stage {
    match next {
        Next::Pick { rows, .. } => Stage::Pick { rows, at: 0 },
        // The buffer starts **empty** rather than seeded with the current name, because an empty
        // answer means something here: it clears a pane's label, and there is no other way back.
        Next::Ask(ask) => Stage::Ask {
            ask,
            buf: String::new(),
        },
        Next::Confirm { lines, op } => Stage::Confirm { lines, op },
        // Both callers take `Send` before they reach this: an op with nothing left to collect is
        // fired rather than staged.
        Next::Send(op) => unreachable!("a bare send never becomes a stage: {op}"),
    }
}

fn body<'a>(modal: &Modal, width: usize, t: &Theme) -> Vec<Line<'a>> {
    match &modal.stage {
        Stage::Pick { rows, at } => rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let on = i == *at;
                let mark = match on {
                    true => "\u{25b8}",
                    false => " ",
                };
                let key = row.key.map(|k| k.to_string()).unwrap_or_else(|| " ".into());
                Line::from(vec![
                    Span::styled(
                        format!(" {mark} {key}  "),
                        Style::default().fg(match on {
                            true => t.accent,
                            false => t.mute,
                        }),
                    ),
                    Span::styled(
                        format!("{:<12}", row.label),
                        Style::default()
                            .fg(match on {
                                true => t.accent_hi,
                                false => t.text,
                            })
                            .add_modifier(match on {
                                true => Modifier::BOLD,
                                false => Modifier::empty(),
                            }),
                    ),
                    Span::styled(row.note.clone(), Style::default().fg(t.dim)),
                ])
            })
            .collect(),
        Stage::Ask { ask, buf } => {
            let mut lines = vec![Line::from(Span::styled(
                format!(" {}", ask.prompt),
                Style::default().fg(t.text),
            ))];
            lines.push(Line::from(Span::styled(
                format!(" \u{276f} {buf}\u{2588}"),
                Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
            )));
            for line in wrap(&ask.hint, width) {
                lines.push(Line::from(Span::styled(
                    format!(" {line}"),
                    Style::default().fg(t.dim),
                )));
            }
            lines
        }
        Stage::Confirm { lines, .. } => {
            let mut out = Vec::new();
            for (i, para) in lines.iter().enumerate() {
                if i > 0 {
                    out.push(Line::from(""));
                }
                let style = match i {
                    0 => Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(t.dim),
                };
                for line in wrap(para, width) {
                    out.push(Line::from(Span::styled(format!(" {line}"), style)));
                }
            }
            out
        }
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn digit(i: usize) -> Option<char> {
    (i < 9).then(|| char::from(b'1' + i as u8))
}

/// The machines a `node`-scoped op may name are the herd's, **never `caps.sessions`**. A session
/// that is running and not served is not a node of this herd and never will be, so a workspace
/// made in one would be a workspace nothing here can reach. An offline node is left out for the
/// same reason it is not a button: the op cannot land.
fn reachable(herd: &Herd) -> Vec<&NodeEntry> {
    herd.nodes.iter().filter(|n| n.online).collect()
}

fn on_a_node(nodes: &[&NodeEntry], what: &str, then: impl Fn(&str) -> Next) -> Next {
    if let [only] = nodes {
        return then(&only.id);
    }
    Next::Pick {
        title: what.to_string(),
        rows: nodes
            .iter()
            .enumerate()
            .map(|(i, node)| Row {
                key: digit(i),
                label: node.name.clone(),
                note: node.kind.clone(),
                next: then(&node.id),
            })
            .collect(),
    }
}

/// `rename` routes by the target's own type, and only a **pane's** label is nullable — herdr's
/// tab and workspace renames take a required string, so there is nothing to clear them to.
fn rename(at: &str, kind: &str, current: Option<String>, empty: Empty) -> Next {
    Next::Ask(Ask {
        prompt: match current {
            Some(now) => format!("This {kind} is called \"{now}\". Call it what?"),
            None => format!("This {kind} has no name. Call it what?"),
        },
        hint: match empty {
            Empty::Null => "enter with nothing typed clears the label".into(),
            _ => format!("a {kind} needs a name — enter with nothing typed does nothing"),
        },
        op: json!({ "op": "rename", "at": at }),
        field: "label",
        empty,
    })
}

fn close(at: &str, kind: &str) -> Next {
    Next::Confirm {
        lines: vec![
            format!("Close this {kind}?"),
            format!("{at} — the shells in it end, and so does whatever they are running."),
        ],
        op: json!({ "op": "close", "at": at }),
    }
}

/// **Say what a split will do before doing it.** It is not a violation of ADR 0002 — that
/// invariant is about the side effects of *viewing* — but a pane's geometry moving under somebody
/// else is announced nowhere, and #298 is what that costs: a desk drew a 50-column box around a
/// 70-column PTY and cropped a line mid-word, with no error at either end.
fn split(at: &str, direction: &str) -> Next {
    Next::Confirm {
        lines: vec![
            format!("Split {at} {direction}?"),
            "herdr re-lays out the whole tab, so every pane in it changes size — on this screen \
             and on anyone attached at the desk."
                .into(),
            "A program that has already drawn its box is not told. #298 measured that: a pane \
             reshaped under an attached client went on drawing the old one, and the line that \
             came back whole from the API appeared at the desk cropped."
                .into(),
        ],
        op: json!({ "op": "pane.split", "at": at, "direction": direction, "ratio": 0.5 }),
    }
}

fn title_of(action: Action) -> String {
    use Action::*;
    match action {
        NewWorkspace => "manage",
        NewTab => "tab",
        RenameTab => "rename tab",
        CloseTab => "close tab",
        RenameWorkspace => "rename workspace",
        CloseWorkspace => "close workspace",
        SplitVertical | SplitHorizontal => "split",
        ClosePane => "close pane",
        RenamePane => "rename pane",
        NewWorktree => "worktree",
        _ => "manage",
    }
    .to_string()
}
