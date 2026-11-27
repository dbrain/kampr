use super::App;
use crate::headless::{Stored, Went};
use crossterm::event::KeyEvent;
use kampr_client::Managed;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;

/// Ops whose ack names the pane they made, which this client then opens.
const OPENS: [&str; 5] = [
    "claude.create",
    "claude.resume",
    "claude.to_terminal",
    "claude.from_terminal",
    "native.resume",
];

impl App {
    pub(super) fn offers_claude(&self, node: &str) -> bool {
        let state = self.client.state();
        state.role.writes()
            && state.caps().manage
            && state.herd.node(node).is_some_and(|n| n.claude == Some(true))
    }

    /// Once per socket for each node that keeps sessions, and again when one leaves the herd and
    /// comes back (docs/04 § native.resumable).
    pub(super) fn ask_resumes(&mut self) {
        let (ask, gone): (Vec<String>, Vec<String>) = {
            let state = self.client.state();
            if !state.caps().manage || !state.role.writes() {
                return;
            }
            let keeps: Vec<String> = state
                .herd
                .nodes
                .iter()
                .filter(|n| n.resume == Some(true) && n.is_serving())
                .map(|n| n.id.clone())
                .collect();
            (
                keeps
                    .iter()
                    .filter(|n| !self.asked_resumes.contains(*n))
                    .cloned()
                    .collect(),
                self.asked_resumes
                    .iter()
                    .filter(|n| !keeps.contains(n))
                    .cloned()
                    .collect(),
            )
        };
        for node in gone {
            self.asked_resumes.remove(&node);
            self.resumes.remove(&node);
        }
        for node in ask {
            self.asked_resumes.insert(node.clone());
            self.send_manage_about(json!({ "op": "native.resumable", "node": node }), node);
        }
    }

    pub(super) fn spend_record(&mut self, node: &str, id: &str, op: &str) {
        if let Some(records) = self.resumes.get_mut(node) {
            records.retain(|r| r.id != id);
        }
        self.send_manage(json!({ "op": op, "node": node, "id": id }));
    }

    /// `true` for an ack this surface answers and the manage strip does not.
    pub(super) fn headless_ack(&mut self, ack: &Managed, about: Option<String>) -> bool {
        match ack.op.as_str() {
            "native.resumable" => {
                if let Some(node) = about {
                    match ack.ok {
                        true => self
                            .resumes
                            .insert(node, crate::resume::records(ack.resumable.as_ref())),
                        false => self.resumes.remove(&node),
                    };
                }
                true
            }
            "claude.resumable" => {
                if let Some(modal) = self.new_claude.as_mut() {
                    modal.found(match ack.ok {
                        true => Ok(ack
                            .resumable
                            .clone()
                            .and_then(|v| serde_json::from_value::<Vec<Stored>>(v).ok())
                            .unwrap_or_default()),
                        false => Err(ack
                            .message
                            .clone()
                            .or_else(|| ack.code.clone())
                            .unwrap_or_default()),
                    });
                }
                true
            }
            op if ack.ok && OPENS.contains(&op) => {
                self.opening = ack.id.clone();
                false
            }
            _ => false,
        }
    }

    pub(super) fn new_claude_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.new_claude.as_mut() else {
            return;
        };
        match modal.key(key) {
            Went::Stay => {}
            Went::Close => self.new_claude = None,
            Went::Ask(op) => self.send_manage(op),
            Went::Send(op) => {
                self.new_claude = None;
                self.manage.sent(op.clone());
                self.send_manage(op);
            }
        }
    }

    pub(super) fn draw_new_claude(&self, area: Rect, buf: &mut Buffer) {
        if let Some(modal) = self.new_claude.as_ref() {
            modal.render(area, buf, &self.options.theme);
        }
    }

    pub(super) fn resume_lines(&self, node: &str) -> Vec<String> {
        let Some(records) = self.resumes.get(node).filter(|r| !r.is_empty()) else {
            return Vec::new();
        };
        let mut lines = vec!["  ↻ resume after reboot · w then . on the host".to_string()];
        lines.extend(records.iter().map(|r| format!("    {}", r.line())));
        lines
    }
}
