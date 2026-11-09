use super::App;
use crate::host::{HostMenu, Took};
use crate::keymap::Action;
use crate::sidebar::{self, Area};
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

impl App {
    pub(super) fn show_area(&mut self, node: &str, area: Area) {
        self.prefs.hosts.show(node, area);
        self.keep_prefs();
    }

    pub(super) fn cycle_area(&mut self, by: i8) {
        let rows = self.rows();
        let Some((node, tabs, area, at)) = sidebar::host_of(&rows, self.pick) else {
            self.note("[ and ] switch a host's list");
            return;
        };
        let Some(next) = sidebar::step(&tabs, area, by.into()) else {
            self.note("terminals are all this host has");
            return;
        };
        self.show_area(&node, next);
        self.pick = at;
    }

    pub(super) fn host_on_pick(&mut self) {
        let rows = self.rows();
        match sidebar::host_of(&rows, self.pick) {
            Some((node, ..)) => self.host = Some(HostMenu::new(&node)),
            None => self.note(". opens the menu of the host the navigator is on"),
        }
    }

    pub(super) fn host_key(&mut self, key: KeyEvent) {
        let Some(menu) = self.host.as_ref() else {
            return;
        };
        match menu.key(key) {
            Took::Stay => {}
            Took::Close => self.host = None,
            Took::New => {
                self.host = None;
                self.router.leave();
                self.act(Action::NewWorkspace);
            }
        }
    }

    pub(super) fn draw_host(&mut self, area: Rect, buf: &mut Buffer) {
        let Some(menu) = self.host.as_ref() else {
            return;
        };
        let state = self.client.state();
        menu.render(state.herd.node(&menu.node), area, buf, &self.options.theme);
    }
}
