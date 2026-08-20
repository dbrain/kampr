use crate::provider::{AgentStatus, PaneInfo};
use crate::registry::PaneUpdate;
use crate::scrollback::ScrollbackDoc;
use kampr_term::{Cell, CellAttrs, Color, RowDiff};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PROTOCOL: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    #[serde(flatten)]
    pub attrs: CellAttrs,
}

#[derive(Debug, Clone, Serialize)]
pub struct Styles {
    pub from: u32,
    pub styles: Vec<Style>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Run {
    pub s: u32,
    pub x: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowRuns {
    pub row: u32,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Caps {
    pub push: bool,
    pub scrollback: bool,
    pub conversation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Full,
    Readonly,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hello {
    pub protocol: u32,
    pub node_id: String,
    pub node_name: String,
    pub build: String,
    pub role: Role,
    pub caps: Caps,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub herdr_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaneEntry {
    pub id: String,
    pub node_id: String,
    pub workspace: Option<String>,
    pub tab: Option<String>,
    pub cwd: Option<String>,
    pub label: Option<String>,
    pub agent: Option<String>,
    pub agent_status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    pub scrollback_rows: u32,
    pub has_conversation: bool,
    /// Node-stamped; herdr's snapshot carries no timestamp, so whoever assembles the herd message
    /// owns this clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl PaneEntry {
    pub fn new(node_id: &str, p: &PaneInfo, has_conversation: bool) -> Self {
        Self {
            id: format!("{node_id}/{}", p.pane_id),
            node_id: node_id.to_string(),
            workspace: p.workspace.clone(),
            tab: p.tab.clone(),
            cwd: p.cwd.clone(),
            label: p.label.clone(),
            agent: p.agent.clone(),
            agent_status: p.agent_status,
            cols: p.cols,
            rows: p.rows,
            scrollback_rows: p.scrollback_rows,
            has_conversation,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t")]
pub enum ServerMsg {
    #[serde(rename = "hello")]
    Hello(Hello),
    #[serde(rename = "herd")]
    Herd {
        nodes: Vec<NodeEntry>,
        panes: Vec<PaneEntry>,
    },
    #[serde(rename = "herd.patch")]
    HerdPatch {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        added: Vec<PaneEntry>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        changed: Vec<PaneEntry>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        removed_ids: Vec<String>,
    },
    #[serde(rename = "styles")]
    Styles(Styles),
    #[serde(rename = "grid.reset")]
    GridReset {
        pane: String,
        cols: u16,
        rows: u16,
        rows_data: Vec<RowRuns>,
        cursor: Cursor,
        links: Vec<String>,
    },
    #[serde(rename = "grid.patch")]
    GridPatch {
        pane: String,
        rows: Vec<RowRuns>,
        cursor: Cursor,
        /// Absent unless the pane's link table grew; the protocol document only shows `links` on
        /// `grid.reset`, but a hyperlink can first appear inside a patch.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        links: Vec<String>,
    },
    #[serde(rename = "scrollback")]
    Scrollback {
        pane: String,
        from_top: u32,
        rows: Vec<RowRuns>,
        total_rows: u32,
        complete: bool,
        capped: bool,
    },
    #[serde(rename = "error")]
    Error {
        code: ErrorCode,
        message: String,
        pane: Option<String>,
    },
    #[serde(rename = "pong")]
    Pong { n: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotWriter,
    UnknownPane,
    NodeOffline,
    HerdrUnavailable,
    RateLimited,
    BadRequest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "t")]
pub enum ClientMsg {
    #[serde(rename = "watch")]
    Watch {
        pane: String,
        #[serde(default)]
        scrollback: bool,
        #[serde(default)]
        conversation: bool,
    },
    #[serde(rename = "unwatch")]
    Unwatch { pane: String },
    #[serde(rename = "input")]
    Input {
        pane: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        b64: Option<String>,
        #[serde(default)]
        keys: Option<Vec<String>>,
    },
    #[serde(rename = "answer")]
    Answer { pane: String, key: String },
    #[serde(rename = "convo.load")]
    ConvoLoad { pane: String, before: Option<String> },
    #[serde(rename = "resync")]
    Resync,
    #[serde(rename = "ping")]
    Ping { n: u64 },
}

/// Per-connection style interning. Ids are stable for the life of a connection, and style 0 is
/// always the default pen, so a client can render before the first `styles` message arrives.
#[derive(Debug, Default)]
pub struct Encoder {
    entries: Vec<Style>,
    index: HashMap<Style, u32>,
    sent: u32,
}

impl Encoder {
    pub fn new() -> Self {
        let default = Style::default();
        Self {
            entries: vec![default],
            index: HashMap::from([(default, 0)]),
            sent: 1,
        }
    }

    fn intern(&mut self, style: Style) -> u32 {
        if let Some(id) = self.index.get(&style) {
            return *id;
        }
        let id = self.entries.len() as u32;
        self.entries.push(style);
        self.index.insert(style, id);
        id
    }

    pub fn take_styles(&mut self) -> Option<Styles> {
        if self.sent as usize == self.entries.len() {
            return None;
        }
        let from = self.sent;
        self.sent = self.entries.len() as u32;
        Some(Styles {
            from,
            styles: self.entries[from as usize..].to_vec(),
        })
    }

    pub fn rows(&mut self, diffs: &[RowDiff]) -> Vec<RowRuns> {
        diffs
            .iter()
            .map(|d| RowRuns {
                row: d.row,
                runs: self.runs(&d.cells),
            })
            .collect()
    }

    fn runs(&mut self, cells: &[Cell]) -> Vec<Run> {
        let blank = Cell::default();
        let end = cells.iter().rposition(|c| *c != blank).map_or(0, |i| i + 1);
        let mut runs: Vec<Run> = Vec::new();
        for cell in &cells[..end] {
            let s = self.intern(Style {
                fg: cell.fg,
                bg: cell.bg,
                attrs: cell.attrs,
            });
            match runs.last_mut() {
                Some(r) if r.s == s && r.l == cell.link => r.x.push(cell.ch),
                _ => runs.push(Run {
                    s,
                    x: cell.ch.to_string(),
                    l: cell.link,
                }),
            }
        }
        runs
    }

    /// Emits the `styles` message first when the update introduced new pens, so a client never
    /// sees a run referencing a style id it has not been told about.
    pub fn encode(&mut self, pane: &str, update: &PaneUpdate) -> Vec<ServerMsg> {
        let rows = self.rows(update.rows());
        let mut out = Vec::with_capacity(2);
        if let Some(s) = self.take_styles() {
            out.push(ServerMsg::Styles(s));
        }
        out.push(match update {
            PaneUpdate::Reset {
                cols,
                rows: n,
                cursor,
                links,
                ..
            } => ServerMsg::GridReset {
                pane: pane.to_string(),
                cols: *cols,
                rows: *n,
                rows_data: rows,
                cursor: *cursor,
                links: links.as_ref().clone(),
            },
            PaneUpdate::Patch {
                cursor, new_links, ..
            } => ServerMsg::GridPatch {
                pane: pane.to_string(),
                rows,
                cursor: *cursor,
                links: new_links.as_ref().clone(),
            },
        });
        out
    }

    pub fn encode_scrollback(&mut self, pane: &str, doc: &ScrollbackDoc) -> Vec<ServerMsg> {
        let rows = self.rows(&doc.rows);
        let mut out = Vec::with_capacity(2);
        if let Some(s) = self.take_styles() {
            out.push(ServerMsg::Styles(s));
        }
        out.push(ServerMsg::Scrollback {
            pane: pane.to_string(),
            from_top: doc.from_top,
            rows,
            total_rows: doc.total_rows,
            complete: doc.complete,
            capped: doc.capped,
        });
        out
    }
}
