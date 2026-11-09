use super::{App, Screen, unix_now};
use crate::docker::{Detail, Op, Took, ask};
use crate::sidebar::{Fold, Hit};
use crossterm::event::KeyEvent;
use kampr_client::Managed;
use kampr_core::wire::ContainerEntry;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::time::Instant;

impl App {
    pub(super) fn hit(&mut self, hit: Hit) {
        match hit {
            Hit::Fold { node, fold } => self.fold(&node, fold),
            Hit::Container { node, id } => self.detail = Some(Detail::new(&node, &id)),
        }
    }

    fn fold(&mut self, node: &str, fold: Fold) {
        self.prefs.folds.flip(node, fold);
        self.keep_prefs();
    }

    fn container(&self, node: &str, id: &str) -> Option<ContainerEntry> {
        let state = self.client.state();
        state
            .herd
            .node(node)
            .and_then(|n| n.containers.iter().find(|c| c.id == id))
            .cloned()
    }

    pub(super) fn docker_key(&mut self, key: KeyEvent) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        match detail.key(key) {
            Took::Stay => {}
            Took::Close => self.detail = None,
            Took::Act(op) => {
                let (node, id) = (detail.node.clone(), detail.id.clone());
                let herd = self.container(&node, &id);
                let entry = self.detail.as_ref().and_then(|d| d.entry(herd.as_ref()));
                self.docker_op(op, &node, entry);
            }
        }
    }

    pub(super) fn docker_on_pick(&mut self, op: Op) {
        match self.rows().get(self.pick).and_then(|r| r.hit()) {
            Some(Hit::Container { node, id }) => {
                let entry = self.container(&node, &id);
                self.docker_op(op, &node, entry);
            }
            _ => self.note("s, r, x and a act on a container row"),
        }
    }

    fn docker_op(&mut self, op: Op, node: &str, entry: Option<ContainerEntry>) {
        let Some(entry) = entry else {
            self.note("that container has gone");
            return;
        };
        let (writes, manage) = {
            let state = self.client.state();
            (state.role.writes(), state.caps().manage)
        };
        if !writes {
            self.note("this device is read-only");
            return;
        }
        if !manage {
            self.note("this node does not claim manage");
            return;
        }
        let body = op.body(node, &entry);
        self.manage.sent(body.clone());
        self.send_manage(body);
    }

    /// `docker.detail` is this panel's own refresh, never an op the operator typed, so its acks stop
    /// here; a shell's ack names the pane the herd will bring.
    pub(super) fn docker_ack(&mut self, ack: &Managed) -> bool {
        if ack.op == "docker.shell" && ack.ok {
            self.opening = ack.id.clone();
        }
        if ack.op != "docker.detail" {
            return false;
        }
        if let Some(detail) = self.detail.as_mut() {
            match (&ack.detail, ack.ok) {
                (Some(data), true) if data["id"].as_str().is_none_or(|id| id == detail.id) => {
                    detail.answered(data, Instant::now())
                }
                (_, true) => detail.answered(&serde_json::Value::Null, Instant::now()),
                (_, false) => detail.refused(
                    ack.message
                        .clone()
                        .or_else(|| ack.code.clone())
                        .unwrap_or_default(),
                    Instant::now(),
                ),
            }
        }
        true
    }

    pub(super) fn tick_docker(&mut self) {
        if let Some(id) = self.opening.clone() {
            let found = {
                let state = self.client.state();
                state
                    .herd
                    .panes
                    .iter()
                    .find(|p| p.id == id || p.tab_id.as_deref() == Some(id.as_str()))
                    .map(|p| p.id.clone())
            };
            if let Some(pane) = found {
                self.opening = None;
                self.detail = None;
                self.router.leave();
                self.focus(pane);
                self.open(Screen::Panes);
            }
        }
        let now = Instant::now();
        if let Some(detail) = self.detail.as_mut().filter(|d| d.due(now)) {
            detail.asked(now);
            let body = ask(&detail.node, &detail.id);
            self.send_manage(body);
        }
    }

    pub(super) fn draw_docker(&mut self, area: Rect, buf: &mut Buffer) {
        let Some((node, id)) = self.detail.as_ref().map(|d| (d.node.clone(), d.id.clone())) else {
            return;
        };
        let herd = self.container(&node, &id);
        let theme = self.options.theme;
        if let Some(detail) = self.detail.as_mut() {
            detail.render(herd.as_ref(), unix_now(), area, buf, &theme);
        }
    }
}
