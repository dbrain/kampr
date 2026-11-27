use super::App;
use crate::host::{HostMenu, Offers, Took};
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
        let Some(mut menu) = self.host.take() else {
            return;
        };
        let node = menu.node.clone();
        let records = self.resumes.get(&node).cloned().unwrap_or_default();
        let offers = Offers {
            claude: self.offers_claude(&node),
            records: &records,
        };
        match menu.key(key, &offers) {
            Took::Stay => self.host = Some(menu),
            Took::Close => {}
            Took::New => {
                self.router.leave();
                self.act(Action::NewWorkspace);
            }
            Took::Claude => {
                self.router.leave();
                self.new_claude = Some(crate::headless::NewClaude::new(&node));
            }
            Took::Resume(id) => {
                self.router.leave();
                self.spend_record(&node, &id, "native.resume");
            }
            Took::Dismiss(id) => {
                self.spend_record(&node, &id, "native.resume.dismiss");
                self.host = Some(menu);
            }
        }
    }

    pub(super) fn draw_host(&mut self, area: Rect, buf: &mut Buffer) {
        let Some(menu) = self.host.as_ref() else {
            return;
        };
        let offers = Offers {
            claude: self.offers_claude(&menu.node),
            records: self
                .resumes
                .get(&menu.node)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        };
        let state = self.client.state();
        menu.render(
            state.herd.node(&menu.node),
            &offers,
            area,
            buf,
            &self.options.theme,
        );
    }
}
